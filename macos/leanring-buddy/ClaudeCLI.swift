//
//  ClaudeCLI.swift
//  leanring-buddy
//
//  Runs the local `claude -p` (Claude Code) CLI as a subprocess instead of
//  calling the Anthropic API over HTTP. Uses the CLI's existing login, so no
//  API keys or proxy are needed. Screenshots are sent as base64 image blocks
//  over stream-json stdin, and text deltas are read from stream-json stdout.
//

import Foundation

struct ClaudeCLIError: LocalizedError {
    let message: String

    var errorDescription: String? {
        message
    }
}

final class ClaudeCLI {
    /// A Claude Code model alias like "sonnet" or "opus".
    var model: String

    init(model: String = "sonnet") {
        self.model = model
    }

    /// Send screenshots + prompt to `claude -p` and stream the reply.
    /// Calls `onTextChunk` with the accumulated text each time a delta arrives.
    /// Cancelling the calling task terminates the subprocess.
    func analyzeImageStreaming(
        images: [(data: Data, label: String)],
        systemPrompt: String,
        conversationHistory: [(userPlaceholder: String, assistantResponse: String)] = [],
        userPrompt: String,
        allowsMacTools: Bool = false,
        effort: String? = nil,
        onTextChunk: @MainActor @Sendable (String) -> Void
    ) async throws -> (text: String, duration: TimeInterval) {
        let startTime = Date()

        guard let claudeExecutableURL = Self.claudeExecutableURL else {
            throw ClaudeCLIError(message: "couldn't find the claude CLI. install claude code and run `claude` once to log in.")
        }

        let promptText = Self.foldedPromptText(conversationHistory: conversationHistory, userPrompt: userPrompt)

        var contentBlocks: [[String: Any]] = []
        for image in images {
            contentBlocks.append([
                "type": "image",
                "source": [
                    "type": "base64",
                    "media_type": "image/jpeg",
                    "data": image.data.base64EncodedString()
                ]
            ])
            contentBlocks.append(["type": "text", "text": image.label])
        }
        contentBlocks.append(["type": "text", "text": promptText])

        let stdinMessage: [String: Any] = [
            "type": "user",
            "message": ["role": "user", "content": contentBlocks]
        ]
        var stdinData = try JSONSerialization.data(withJSONObject: stdinMessage)
        stdinData.append(0x0A)

        let process = Process()
        process.executableURL = claudeExecutableURL
        process.arguments = [
            "-p",
            "--input-format", "stream-json",
            "--output-format", "stream-json",
            "--include-partial-messages",
            "--verbose",
            "--model", model,
            "--system-prompt", systemPrompt,
            // No MCP servers, slash commands, or user settings — skipping them
            // keeps startup fast and the prompt free of unrelated context.
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--setting-sources", "project",
            "--no-session-persistence"
        ] + (allowsMacTools ? Self.macToolArguments : ["--tools", ""])
          + (effort.map { ["--effort", $0] } ?? [])
        // Run from an empty temp dir so no project CLAUDE.md gets picked up.
        process.currentDirectoryURL = FileManager.default.temporaryDirectory

        let stdinPipe = Pipe()
        let stdoutPipe = Pipe()
        let stderrPipe = Pipe()
        process.standardInput = stdinPipe
        process.standardOutput = stdoutPipe
        process.standardError = stderrPipe

        try process.run()
        print("🤖 claude -p: sent \(String(format: "%.1f", Double(stdinData.count) / 1_048_576.0))MB, \(images.count) image(s), model \(model)")

        // Write stdin and drain stderr off-thread so neither pipe can fill up
        // and block the subprocess while we read stdout.
        DispatchQueue.global().async {
            // The throwing variant reports a dead pipe as an error; the old
            // write(_:) raises an Objective-C exception that crashes the app.
            try? stdinPipe.fileHandleForWriting.write(contentsOf: stdinData)
            try? stdinPipe.fileHandleForWriting.close()
        }
        let stderrTask = Task.detached {
            String(data: stderrPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        }

        var accumulatedResponseText = ""
        var finalResultText: String?
        var resultErrorText: String?

        try await withTaskCancellationHandler {
            for try await line in stdoutPipe.fileHandleForReading.bytes.lines {
                guard let lineData = line.data(using: .utf8),
                      let outputEvent = try? JSONSerialization.jsonObject(with: lineData) as? [String: Any],
                      let outputEventType = outputEvent["type"] as? String else {
                    continue
                }

                if outputEventType == "stream_event",
                   let streamEvent = outputEvent["event"] as? [String: Any],
                   streamEvent["type"] as? String == "content_block_delta",
                   let delta = streamEvent["delta"] as? [String: Any],
                   delta["type"] as? String == "text_delta",
                   let textChunk = delta["text"] as? String {
                    accumulatedResponseText += textChunk
                    let currentAccumulatedText = accumulatedResponseText
                    await onTextChunk(currentAccumulatedText)
                } else if outputEventType == "result" {
                    if outputEvent["is_error"] as? Bool == true {
                        resultErrorText = outputEvent["result"] as? String ?? "claude -p returned an error"
                    } else {
                        finalResultText = outputEvent["result"] as? String
                    }
                }
            }
        } onCancel: {
            process.terminate()
        }

        process.waitUntilExit()
        try Task.checkCancellation()

        if let resultErrorText {
            throw ClaudeCLIError(message: resultErrorText)
        }
        guard process.terminationStatus == 0 else {
            let stderrText = await stderrTask.value
            throw ClaudeCLIError(message: "claude -p exited with \(process.terminationStatus): \(stderrText.prefix(500))")
        }

        let duration = Date().timeIntervalSince(startTime)
        return (text: finalResultText ?? accumulatedResponseText, duration: duration)
    }

    /// The CLI's stream-json input takes one user message, so prior turns
    /// are folded into the prompt text instead of sent as separate messages.
    /// CodexCLI reuses this, since `codex exec` also takes one prompt.
    static func foldedPromptText(
        conversationHistory: [(userPlaceholder: String, assistantResponse: String)],
        userPrompt: String
    ) -> String {
        var promptText = ""
        if !conversationHistory.isEmpty {
            promptText += "conversation so far:\n"
            for (userPlaceholder, assistantResponse) in conversationHistory {
                promptText += "user: \(userPlaceholder)\nyou: \(assistantResponse)\n"
            }
            promptText += "\nnew message from the user:\n"
        }
        promptText += userPrompt
        return promptText
    }

    /// Lets Claude drive the Mac with a short allowlist of commands. In print
    /// mode anything not allowlisted (and not read-only) is denied outright,
    /// so `rm`, `mv`, `curl` and friends never run. `osascript` is the
    /// powerful one — it's what sends messages and controls apps — so the
    /// system prompt requires a spoken yes before anything irreversible.
    private static let macToolArguments: [String] = [
        "--tools", "Bash,Read",
        "--allowedTools",
        "Bash(osascript:*)",
        "Bash(open:*)",
        "Bash(mdfind:*)",
        "Bash(shortcuts:*)",
        "Bash(ls:*)",
        "Read",
        // Home folder access so it can find files like recent recordings.
        "--add-dir", FileManager.default.homeDirectoryForCurrentUser.path
    ]

    /// GUI apps launch with a minimal PATH, so check the usual install
    /// locations first, then ask a login shell as a last resort.
    static let claudeExecutableURL: URL? = {
        let homeDirectoryPath = FileManager.default.homeDirectoryForCurrentUser.path
        let candidatePaths = [
            "\(homeDirectoryPath)/.local/bin/claude",
            "\(homeDirectoryPath)/.claude/local/claude",
            "/opt/homebrew/bin/claude",
            "/usr/local/bin/claude"
        ]
        if let installedPath = candidatePaths.first(where: { FileManager.default.isExecutableFile(atPath: $0) }) {
            return URL(fileURLWithPath: installedPath)
        }

        let loginShellProcess = Process()
        let loginShellOutputPipe = Pipe()
        loginShellProcess.executableURL = URL(fileURLWithPath: "/bin/zsh")
        loginShellProcess.arguments = ["-ilc", "command -v claude"]
        loginShellProcess.standardOutput = loginShellOutputPipe
        loginShellProcess.standardError = FileHandle.nullDevice
        guard (try? loginShellProcess.run()) != nil else { return nil }
        loginShellProcess.waitUntilExit()
        let resolvedPath = String(
            data: loginShellOutputPipe.fileHandleForReading.readDataToEndOfFile(),
            encoding: .utf8
        )?
            .split(separator: "\n")
            .last
            .map { String($0).trimmingCharacters(in: .whitespaces) }
        guard let resolvedPath, FileManager.default.isExecutableFile(atPath: resolvedPath) else { return nil }
        return URL(fileURLWithPath: resolvedPath)
    }()
}
