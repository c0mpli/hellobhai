// Entry point of the exporter built by scripts/export-mac-web-assets-for-tauri.sh.
// Compiled together with copies of VisualExplanationPanel.swift,
// Model3DOverlay.swift and the prompts pulled out of CompanionManager.swift,
// it writes the real generated pages and prompts into tauri/generated/.
//
// Every substitution must match exactly once. If a Mac edit breaks one, the
// exporter fails instead of silently shipping Mac-only text to other platforms.
import Foundation

struct PromptSubstitution: Decodable {
    let prompt: String
    let find: String
    let replace: String
}

func exitWithError(_ message: String) -> Never {
    FileHandle.standardError.write(Data("error: \(message)\n".utf8))
    exit(1)
}

func numberOfOccurrences(of searchText: String, in text: String) -> Int {
    text.components(separatedBy: searchText).count - 1
}

func replacingExactlyOnce(_ searchText: String, with replacementText: String, in text: String, substitutionDescription: String) -> String {
    let matchCount = numberOfOccurrences(of: searchText, in: text)
    guard matchCount == 1 else {
        exitWithError("substitution \(substitutionDescription) matched \(matchCount) times, expected exactly 1: \(searchText.prefix(80))")
    }
    return text.replacingOccurrences(of: searchText, with: replacementText)
}

func writeTextFile(_ text: String, to filePath: String) {
    do {
        try text.write(toFile: filePath, atomically: true, encoding: .utf8)
    } catch {
        exitWithError("could not write \(filePath): \(error)")
    }
}

@MainActor func exportPagesAndPrompts() {
    guard CommandLine.arguments.count == 2 else { exitWithError("usage: export-mac-web-assets <repository root>") }
    let repositoryRoot = CommandLine.arguments[1]
    let generatedDirectory = repositoryRoot + "/tauri/generated"
    let generatedPromptsDirectory = generatedDirectory + "/prompts"
    try? FileManager.default.createDirectory(atPath: generatedPromptsDirectory, withIntermediateDirectories: true)

    // The resources are missing (and the pages silently empty) if VisualToolkit
    // was not copied next to the binary.
    guard !VisualExplanationPanelManager.availableIconNames.isEmpty else {
        exitWithError("VisualToolkit resources were not found next to the exporter binary")
    }

    // Tauri's IPC is a fetch to http://ipc.localhost on Windows; the pages'
    // own CSP has to allow it. Nothing else in the pages changes.
    let macContentSecurityPolicyEnd = "script-src 'unsafe-inline'; style-src 'unsafe-inline'; font-src data:\">"
    let tauriContentSecurityPolicyEnd = "script-src 'unsafe-inline'; style-src 'unsafe-inline'; font-src data:; connect-src ipc: http://ipc.localhost\">"
    let pagesByFileName: [(fileName: String, pageHTML: String)] = [
        ("board.html", VisualExplanationPanelManager.boardPageHTML),
        ("notes.html", VisualExplanationPanelManager.notesPageHTML),
        ("model3d.html", Model3DOverlayManager.overlayPageHTML),
    ]
    for (fileName, pageHTML) in pagesByFileName {
        for leftoverPlaceholder in ["__HANDWRITING_FONT_BASE64__", "__PEN_TRIANGLE_SVG_PATH__", "__PENCIL_SCRATCH_SOUND_SCRIPT__", "__THREE_JS_SOURCE__"] where pageHTML.contains(leftoverPlaceholder) {
            exitWithError("\(fileName) still contains \(leftoverPlaceholder)")
        }
        let tauriPageHTML = replacingExactlyOnce(
            macContentSecurityPolicyEnd, with: tauriContentSecurityPolicyEnd, in: pageHTML,
            substitutionDescription: "content security policy in \(fileName)")
        writeTextFile(tauriPageHTML, to: generatedDirectory + "/" + fileName)
    }

    var promptTextByName: [String: String] = [
        "voice_response": ExportedPrompts.companionVoiceResponseSystemPrompt,
        "agent_continuation": ExportedPrompts.agentContinuationPrompt,
        "visual_explanation": ExportedPrompts.visualExplanationSystemPrompt,
        "live_notes": ExportedPrompts.liveNotesSystemPrompt,
        "model3d": ExportedPrompts.model3DSystemPrompt,
        "onboarding_demo": ExportedPrompts.onboardingDemoSystemPrompt,
    ]

    let substitutionsFilePath = repositoryRoot + "/tauri/prompt-substitutions.json"
    guard let substitutionsData = FileManager.default.contents(atPath: substitutionsFilePath),
          let promptSubstitutions = try? JSONDecoder().decode([PromptSubstitution].self, from: substitutionsData) else {
        exitWithError("could not read \(substitutionsFilePath)")
    }
    for (substitutionIndex, promptSubstitution) in promptSubstitutions.enumerated() {
        guard let promptText = promptTextByName[promptSubstitution.prompt] else {
            exitWithError("substitution #\(substitutionIndex) names unknown prompt \(promptSubstitution.prompt)")
        }
        promptTextByName[promptSubstitution.prompt] = replacingExactlyOnce(
            promptSubstitution.find, with: promptSubstitution.replace, in: promptText,
            substitutionDescription: "#\(substitutionIndex) (\(promptSubstitution.prompt))")
    }

    for (promptName, promptText) in promptTextByName {
        writeTextFile(promptText, to: generatedPromptsDirectory + "/" + promptName + ".txt")
    }

    writeTextFile("""
    # Generated files: do not edit

    Everything in this folder is written by `scripts/export-mac-web-assets-for-tauri.sh` from the Mac app's sources
    (`macos/leanring-buddy/VisualExplanationPanel.swift`, `Model3DOverlay.swift`, `VisualToolkit/`, the prompts in
    `CompanionManager.swift`) plus `tauri/prompt-substitutions.json`. Edit those and run the script on a Mac instead.

    """, to: generatedDirectory + "/README.md")
}

MainActor.assumeIsolated { exportPagesAndPrompts() }
