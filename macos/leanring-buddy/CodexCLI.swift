//
//  CodexCLI.swift
//  leanring-buddy
//
//  Runs OpenAI's local `codex exec` CLI as an alternative brain to
//  `claude -p`, with the same call shape as ClaudeCLI so CompanionManager can
//  switch between them. Uses the CLI's existing `codex login`, so no API keys.
//  Screenshots are written to a per-call temp folder and attached with
//  --image; replies are read from the --json event stream on stdout.
//

import Foundation

/// Which local CLI answers the user. Persisted as `selectedBrainEngine`.
enum BrainEngine: String {
    case claude
    case codex
}

final class CodexCLI {
    /// "low", "medium" or "high", or nil to use the model's default effort.
    /// A per-call effort (the panels ask for "low") takes precedence.
    var reasoningEffort: String?

    /// Send screenshots + prompt to `codex exec` and stream the reply.
    /// Calls `onTextChunk` with each agent message as it completes (codex
    /// exec emits whole messages, not deltas). Only the last message is the
    /// answer, so a later message replaces an earlier one.
    /// Cancelling the calling task terminates the subprocess.
    /// `allowsMacTools` is ignored: Codex has no command allowlist, so it
    /// always runs in the read-only sandbox.
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

        // The lookup may start a login shell, so keep it off the main thread.
        guard let codexExecutableURL = await Task.detached(operation: { Self.codexExecutableURL }).value else {
            throw ClaudeCLIError(message: "couldn't find the codex CLI. install it, then run `codex login` once in Terminal.")
        }

        // A fresh empty folder per call: holds the screenshots and is the
        // working directory, so no project AGENTS.md is nearby.
        let callFolderURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("hellobhai-codex-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: callFolderURL, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: callFolderURL) }

        // bhai's system prompt replaces Codex's built-in coding-agent
        // instructions (which ask for Markdown and file references) instead
        // of being added on top of them.
        let instructionsFileURL = callFolderURL.appendingPathComponent("instructions.md")
        try Data(systemPrompt.utf8).write(to: instructionsFileURL)

        var imageFilePaths: [String] = []
        var promptText = ""
        if !images.isEmpty {
            // Images can't carry a caption, so their labels go in the prompt in the same order.
            promptText += "attached images, in order:\n"
            for (imageIndex, image) in images.enumerated() {
                let imageFileURL = callFolderURL.appendingPathComponent("image-\(imageIndex + 1).jpg")
                try image.data.write(to: imageFileURL)
                imageFilePaths.append(imageFileURL.path)
                promptText += "\(imageIndex + 1). \(image.label)\n"
            }
            promptText += "\n"
        }
        promptText += ClaudeCLI.foldedPromptText(conversationHistory: conversationHistory, userPrompt: userPrompt)
        let stdinData = Data(promptText.utf8)

        let process = Process()
        process.executableURL = codexExecutableURL
        process.arguments = Self.codexExecArguments(
            imageFilePaths: imageFilePaths,
            workingDirectoryPath: callFolderURL.path,
            instructionsFilePath: instructionsFileURL.path,
            reasoningEffort: effort ?? reasoningEffort
        )
        process.currentDirectoryURL = callFolderURL
        // An npm install of codex is a `#!/usr/bin/env node` script, and GUI
        // apps get a minimal PATH, so put the usual node locations on it.
        var environment = ProcessInfo.processInfo.environment
        let codexDirectoryPath = codexExecutableURL.deletingLastPathComponent().path
        environment["PATH"] = "\(codexDirectoryPath):/opt/homebrew/bin:/usr/local/bin:" + (environment["PATH"] ?? "/usr/bin:/bin")
        process.environment = environment

        let stdinPipe = Pipe()
        let stdoutPipe = Pipe()
        let stderrPipe = Pipe()
        process.standardInput = stdinPipe
        process.standardOutput = stdoutPipe
        process.standardError = stderrPipe

        try process.run()
        print("🤖 codex exec: sent \(images.count) image(s), effort \(effort ?? reasoningEffort ?? "default")")

        // Write stdin and drain stderr off-thread so neither pipe can fill up
        // and block the subprocess while we read stdout.
        DispatchQueue.global().async {
            try? stdinPipe.fileHandleForWriting.write(contentsOf: stdinData)
            try? stdinPipe.fileHandleForWriting.close()
        }
        let stderrTask = Task.detached {
            String(data: stderrPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        }

        var eventStreamParser = CodexEventStreamParser()

        try await withTaskCancellationHandler {
            for try await line in stdoutPipe.fileHandleForReading.bytes.lines {
                if let latestAgentMessageText = eventStreamParser.handleJSONLine(line) {
                    await onTextChunk(latestAgentMessageText)
                }
            }
        } onCancel: {
            process.terminate()
        }

        process.waitUntilExit()
        try Task.checkCancellation()

        if let turnFailedMessage = eventStreamParser.turnFailedMessage {
            throw ClaudeCLIError(message: Self.userFacingErrorMessage(for: turnFailedMessage))
        }
        guard process.terminationStatus == 0 else {
            let stderrText = await stderrTask.value
            let failureDetail = eventStreamParser.lastErrorMessage ?? String(stderrText.suffix(500))
            throw ClaudeCLIError(message: Self.userFacingErrorMessage(for: "codex exec exited with \(process.terminationStatus): \(failureDetail)"))
        }

        // An interrupted turn, or one that ended on a tool call, exits 0
        // without any answer; say so instead of staying silent.
        guard let lastAgentMessageText = eventStreamParser.lastAgentMessageText else {
            throw ClaudeCLIError(message: "codex returned no reply. try again.")
        }

        let duration = Date().timeIntervalSince(startTime)
        return (text: lastAgentMessageText, duration: duration)
    }

    /// The full `codex exec` argument list. Kept separate so it can be
    /// checked without the binary.
    static func codexExecArguments(
        imageFilePaths: [String],
        workingDirectoryPath: String,
        instructionsFilePath: String,
        reasoningEffort: String?
    ) -> [String] {
        var arguments = [
            "exec",
            "--json",
            // Don't save this run as a resumable session.
            "--ephemeral",
            "--skip-git-repo-check",
            // Skips ~/.codex/config.toml (MCP servers, plugins, the user's
            // model) and the user's execpolicy rules, like ClaudeCLI's
            // --strict-mcp-config and --setting-sources project. Auth still
            // comes from ~/.codex.
            "--ignore-user-config",
            "--ignore-rules",
            "--sandbox", "read-only",
            "--cd", workingDirectoryPath,
            "--config", "approval_policy=\"never\"",
            // Don't read AGENTS.md files into the prompt.
            "--config", "project_doc_max_bytes=0",
            "--config", "web_search=\"disabled\"",
            "--config", "model_instructions_file=\(tomlBasicString(instructionsFilePath))",
            // No shell: the read-only sandbox still lets commands read the
            // whole disk (~/.ssh, ~/.codex/auth.json), and text on screen
            // could ask for that. Also saves the delay of exploring.
            "--config", "features.shell_tool=false",
            // ChatGPT connectors and installed plugins load from the login,
            // not config.toml, so --ignore-user-config doesn't drop them.
            "--config", "features.apps=false",
            "--config", "features.plugins=false"
        ]
        if let reasoningEffort {
            arguments += ["--config", "model_reasoning_effort=\(tomlBasicString(reasoningEffort))"]
        }
        if !imageFilePaths.isEmpty {
            // --image takes several comma-separated values, so the bare "-"
            // (read the prompt from stdin) goes after "--" or it would be
            // swallowed as another image.
            arguments.append("--image=\(imageFilePaths.joined(separator: ","))")
        }
        arguments += ["--", "-"]
        return arguments
    }

    /// --config values are parsed as TOML, so a free-form string must be a
    /// quoted TOML basic string with backslashes, quotes and control
    /// characters escaped.
    static func tomlBasicString(_ rawText: String) -> String {
        var escapedText = "\""
        for unicodeScalar in rawText.unicodeScalars {
            switch unicodeScalar {
            case "\\": escapedText += "\\\\"
            case "\"": escapedText += "\\\""
            case "\n": escapedText += "\\n"
            case "\r": escapedText += "\\r"
            case "\t": escapedText += "\\t"
            default:
                if unicodeScalar.value < 0x20 || unicodeScalar.value == 0x7F {
                    escapedText += String(format: "\\u%04X", unicodeScalar.value)
                } else {
                    escapedText.unicodeScalars.append(unicodeScalar)
                }
            }
        }
        return escapedText + "\""
    }

    /// Turns auth failures into a "sign in" hint; other errors pass through.
    static func userFacingErrorMessage(for codexErrorText: String) -> String {
        let lowercasedErrorText = codexErrorText.lowercased()
        let looksLikeSignInProblem = ["401", "unauthorized", "not logged in", "login", "log in", "sign in", "authentication"]
            .contains { lowercasedErrorText.contains($0) }
        if looksLikeSignInProblem {
            return "codex isn't signed in. run `codex login` in Terminal, then try again. (\(codexErrorText.prefix(300)))"
        }
        return String(codexErrorText.prefix(500))
    }

    /// GUI apps launch with a minimal PATH, so check the usual install
    /// locations each time (cheap, so installing codex shows up without a
    /// restart), then fall back to a login shell lookup done once. The
    /// login shell can be slow, so call this off the main thread.
    nonisolated static var codexExecutableURL: URL? {
        let homeDirectoryPath = FileManager.default.homeDirectoryForCurrentUser.path
        let candidatePaths = [
            "\(homeDirectoryPath)/.local/bin/codex",
            "/opt/homebrew/bin/codex",
            "/usr/local/bin/codex",
            "\(homeDirectoryPath)/.npm-global/bin/codex"
        ]
        if let installedPath = candidatePaths.first(where: { FileManager.default.isExecutableFile(atPath: $0) }) {
            return URL(fileURLWithPath: installedPath)
        }
        return codexExecutableURLFromLoginShell
    }

    // ponytail: resolved once per launch; a codex installed only on a custom
    // PATH (nvm, volta) needs an app restart to be found.
    private nonisolated static let codexExecutableURLFromLoginShell: URL? = ClaudeCLI.executableURLFromLoginShell(commandName: "codex")
}

/// Reads the `codex exec --json` JSONL stream (ThreadEvent in
/// codex-rs/exec/src/exec_events.rs). Agent text only arrives in
/// `item.completed` events with `item.type == "agent_message"`.
struct CodexEventStreamParser {
    /// The answer: codex treats the turn's last agent message as final.
    private(set) var lastAgentMessageText: String?
    /// Set by `turn.failed`; the run has no usable answer.
    private(set) var turnFailedMessage: String?
    /// The latest top-level `error` event. Codex also sends these for
    /// retries ("Reconnecting..."), so they only matter if the run fails.
    private(set) var lastErrorMessage: String?

    /// Returns the agent message text when this line completed one.
    mutating func handleJSONLine(_ jsonLine: String) -> String? {
        guard let lineData = jsonLine.data(using: .utf8),
              let threadEvent = try? JSONSerialization.jsonObject(with: lineData) as? [String: Any],
              let threadEventType = threadEvent["type"] as? String else {
            return nil
        }

        switch threadEventType {
        case "item.completed":
            guard let item = threadEvent["item"] as? [String: Any],
                  item["type"] as? String == "agent_message",
                  let agentMessageText = item["text"] as? String,
                  !agentMessageText.isEmpty else {
                return nil
            }
            lastAgentMessageText = agentMessageText
            return agentMessageText
        case "turn.failed":
            let failureError = threadEvent["error"] as? [String: Any]
            turnFailedMessage = failureError?["message"] as? String ?? lastErrorMessage ?? "codex turn failed"
            return nil
        case "error":
            lastErrorMessage = threadEvent["message"] as? String
            return nil
        default:
            return nil
        }
    }
}
