# hellobhai - Agent Instructions

<!-- This is the single source of truth for all AI coding agents. AGENTS.md is a symlink to this file. -->

## Overview

Local-first macOS menu bar companion app. Lives entirely in the status bar (no dock icon, no main window). Push-to-talk (ctrl+option) captures voice, transcribes it on-device with Whisper (Hindi/English/Hinglish), and sends the transcript + screenshots to Claude via the local `claude -p` CLI. The reply is spoken with on-device neural TTS (Supertonic-3, one voice for English and Hindi) while a purple Dynamic-Island-style pill grows out of the notch. A purple cursor overlay can fly to and point at UI elements, and Claude can click, type, and press keys for the user.

No servers, API keys, analytics, or auto-updater. Network traffic is `claude -p` itself plus one-time model downloads from Hugging Face on first launch (~1GB total).

## Architecture

- **App Type**: Menu bar-only (`LSUIElement=true`), no dock icon or main window. Not sandboxed (needed to spawn `claude`).
- **Framework**: SwiftUI (macOS native) with AppKit bridging for menu bar panel and cursor overlay
- **Pattern**: MVVM with `@StateObject` / `@Published` state management
- **AI Chat**: `claude -p` subprocess (`ClaudeCLI.swift`). Model alias `sonnet` (default) or `opus`. Screenshots go in as base64 image blocks over `--input-format stream-json`; text deltas come back over `--output-format stream-json --include-partial-messages`. Runs with `--strict-mcp-config --disable-slash-commands --setting-sources project --no-session-persistence` from the temp dir so no user/project context leaks in. Voice turns pass `allowsMacTools: true`: `--tools Bash,Read` with an `--allowedTools` allowlist (`osascript`, `open`, `mdfind`, `shortcuts`, `ls`, `Read`) and `--add-dir ~`; anything else non-read-only is denied in print mode. The onboarding demo runs with `--tools ""`. Each voice request is an agent loop of up to 8 screenshot → claude → actions rounds.
- **Speech-to-Text**: WhisperKit (`large-v3-v20240930_626MB`) with auto language detection, batch-transcribed on hotkey release; clips detected as Urdu are re-transcribed as Hindi (they sound alike). The prompt tells Claude transcripts can mishear technical terms. Model stored in `~/Library/Application Support/hellobhai/Models`. Falls back to on-device Apple Speech (needs Dictation enabled) until the model is loaded.
- **Text-to-Speech**: FluidAudio Supertonic-3 (`Supertonic3Manager`, voice F1) — one model and voice for both languages; each sentence is tagged `hi` (contains Devanagari) or `en`. Synthesized one sentence at a time with the next sentence prepared during playback. Falls back to `AVSpeechSynthesizer` until models load or on failure. Claude is told to reply in natural Hinglish (Hindi grammar, everyday English words) written in Devanagari when the user speaks Hindi/Hinglish, so the voice reads it as Hindi.
- **Computer Control**: Claude appends `[CLICK:x,y:label:screenN]`, `[TYPE:text]`, `[KEY:cmd+s]` tags; `ComputerControl.swift` replays them as `CGEvent`s (Accessibility permission). The buddy flies to each click target 1.2s before clicking; pressing the hotkey cancels remaining actions. Only on explicit user request (enforced by prompt).
- **Visual Explanations & Live Notes**: Claude appends `[SHOW:what to explain]` (diagram) or `[NOTES:what to cover]` (summary/notes) and keeps its own reply to a one-line intro. A second `claude -p` call (`visualExplanationSystemPrompt` / `liveNotesSystemPrompt`; notes also get the screenshots) streams JSON lines: a title, then steps of `{"t":"say"}` narration followed by that step's lines. `NarratedStepQueue` plays steps in lockstep — a step's lines appear while its sentence is spoken (`LocalTTSClient.speakTextAndWaitUntilFinished`). Narration follows the user's language; panel text is always English; no emoji. **Teaching style (core product principle):** be direct and exact by default (formulas, theorems, definitions, simple facts get no analogy); only for genuinely complex/abstract topics, or when the user asks to simplify, explain through one everyday analogy from the user's own life (Indian daily life for Hindi/Hinglish speakers), carried all the way through, real term after it lands. Enforced in the voice, diagram and notes prompts; diagrams show analogies with `like` captions (space reserved in the layout), notes with an `analogy` card. **Math mode** for math/physics/graphs: exact board coordinates on an 800×500 area — `polygon`, `square_on` (exact square on a segment, optional unit `grid`), `line`, `path`, `angle`, `point`, `ring`, `pie`, `label`, `formula`, `latex` (KaTeX in a foreignObject); plus graphs in math coordinates inside the latest `axes` (grid, ticks): `plot` and `area` (expressions in x), `parametric` (x(t), y(t)), `tangent` (numeric derivative), `mark`, `vector`, and `numberline`. Expressions are compiled by a small recursive-descent parser in the page (`compileExpression`: + − × ÷ ^, implicit multiplication, pi/e, trig/log/sqrt/abs/…; never `eval`). Claude must never compute curve points itself. Math mode is an infinite canvas too (Claude is told to spread out, give second figures their own axes beside or below, and leave latex/formula unplaced so they go under the drawing). Free labels on the board avoid each other automatically (`moveLabelToFreeSpot` spirals to the nearest spot clear of placed labels, shifting x/y since CSS transforms would be overridden by the appear animation). `pulse` runs 3 cycles, not forever, and the camera stops updating the viewBox once settled (endless sub-pixel updates re-rendered glow filters every frame and looked like blinking). Plot labels go in a stacked legend; `highlight` on math shapes glows instead of drawing an ellipse. Math shapes take highlight/pulse/tick/cross by id (position on an outer group, animations on an inner group so CSS transforms don't clobber position). Title/captions live in their own layer, centered on the bounds of everything drawn. Panel calls run with `--effort low` so the first line streams in ~2s. The full streamed script is printed to the log for debugging. **Diagrams** use auto-layout: Claude declares `group`/`box`/`circle`/`icon`/`note`/`bars`/`arrow` with no coordinates and dagre lays out the graph after every addition (existing shapes glide; column gap compensates for dagre's cluster rank multiplier; arrow labels sit above the line, step badges opposite). Also `step`, `flow`, `highlight`, `cross`, `tick`, `shake`, `pulse`, `text`, `layout` (LR/TB). Infinite canvas camera (eases to fit, scroll/drag pan, pinch zoom, double-click fit, refits on resize). **Notes** render `title`, `heading`, `bullet` (==highlight==), `remember`, `todo`, `quote`, `code` (Prism), `table`, `chart` (rough.js bar/line); all model text is inserted as text nodes. `[CLOSE_DRAWING]` hides the panel; asking to save notes uses osascript into Apple Notes. Pressing the hotkey cancels `currentVisualTask`. Locked-down WKWebView (CSP blocks network, navigation refused, non-persistent storage, commands passed as JSON data).
- **Speaking Indicator**: `CompanionManager.isSpeakingAloud` drives `NotchSpeakingIndicatorView` in the overlay — a black pill hugging the notch (sized from `NSScreen.auxiliaryTopLeftArea`/`auxiliaryTopRightArea`) with a purple orb and voice bars; hangs from top center on displays without a notch.
- **Dependencies (SPM)**: `FluidAudio` (FluidInference/FluidAudio, 0.17.x), `WhisperKit` (argmaxinc/WhisperKit → `argmax-oss-swift`, 1.1.x).
- **Screen Capture**: ScreenCaptureKit (macOS 14.2+), multi-monitor support
- **Voice Input**: Push-to-talk via `AVAudioEngine`. System-wide keyboard shortcut via listen-only CGEvent tap.
- **Element Pointing**: Claude embeds `[POINT:x,y:label:screenN]` tags in responses. The overlay parses these, maps coordinates to the correct monitor, and animates the blue cursor along a bezier arc to the target.
- **Concurrency**: `SWIFT_DEFAULT_ACTOR_ISOLATION = MainActor`, async/await throughout

### Key Architecture Decisions

**claude binary lookup**: GUI apps launch with a minimal PATH, so `ClaudeCLI` checks `~/.local/bin`, `~/.claude/local`, `/opt/homebrew/bin`, `/usr/local/bin`, then falls back to `zsh -ilc 'command -v claude'`.

**Conversation history**: stream-json input takes one user message per run, so the last 20 exchanges (agent rounds included, with action summaries) are folded into the prompt text rather than sent as separate turns.

**Menu Bar Panel Pattern**: The companion panel uses `NSStatusItem` for the menu bar icon and a custom borderless `NSPanel` for the floating control panel. This gives full control over appearance (dark, rounded corners, custom shadow) and avoids the standard macOS menu/popover chrome. The panel is non-activating so it doesn't steal focus. A global event monitor auto-dismisses it on outside clicks.

**Cursor Overlay**: A full-screen transparent `NSPanel` hosts the blue cursor companion. It's non-activating, joins all Spaces, and never steals focus. The cursor position, response text, waveform, and pointing animations all render in this overlay via SwiftUI through `NSHostingView`.

**Global Push-To-Talk Shortcut**: Background push-to-talk uses a listen-only `CGEvent` tap instead of an AppKit global monitor so modifier-based shortcuts like `ctrl + option` are detected more reliably while the app is running in the background.

**Onboarding**: Start → welcome text → Claude picks something on screen and the cursor points at it → "press control + option" prompt streams in.

## Key Files

| File | Lines | Purpose |
|------|-------|---------|
| `leanring_buddyApp.swift` | ~74 | Menu bar app entry point. `CompanionAppDelegate` creates `MenuBarPanelManager`, starts `CompanionManager`, registers as login item. |
| `CompanionManager.swift` | ~1248 | Central state machine. Owns dictation, shortcut monitoring, screen capture, `ClaudeCLI`, `LocalTTSClient`, and overlay management. Coordinates push-to-talk → screenshot → claude → TTS → pointing. |
| `ClaudeCLI.swift` | ~221 | Spawns `claude -p` with stream-json in/out, streams text deltas, terminates the subprocess on task cancellation. |
| `LocalTTSClient.swift` | ~166 | Supertonic-3 neural TTS (English + Hindi) with per-sentence pipelining and system-voice fallback. Reports speaking state for the edge glow. |
| `WhisperTranscriptionProvider.swift` | ~193 | WhisperKit push-to-talk transcription with auto language detection; Apple Speech fallback while the model downloads. |
| `VisualExplanationPanel.swift` | ~1749 | Floating panel with two live pages: an auto-layout (dagre) rough.js whiteboard with infinite canvas, and a notes page (code via Prism, tables, sketch charts). Streams JSON commands, CSP lockdown, cursor-relative placement. |
| `VisualToolkit/` | — | Bundled `rough.js` 4.6.6, `dagre.min.js` 1.1.4, `prism.min.js` 1.29.0 (core + 24 languages), `katex.min.js` + `katex-inline.css` 0.16.11 (fonts inlined), `icons.json` (139 Lucide shapes), `Virgil.woff2`. Licenses in `THIRD_PARTY_NOTICES.md`. |
| `ComputerControl.swift` | ~198 | Parses `[CLICK]`/`[TYPE]`/`[KEY]` tags and posts synthetic mouse/keyboard events. |
| `MenuBarPanelManager.swift` | ~243 | NSStatusItem + custom NSPanel lifecycle. Menu bar icon, floating panel show/hide/position, click-outside-to-dismiss. |
| `CompanionPanelView.swift` | ~607 | SwiftUI panel content: status, push-to-talk instructions, model picker (Sonnet/Opus), permissions UI, quit button. Uses `DS` design system. |
| `OverlayWindow.swift` | ~895 | Full-screen transparent overlay hosting the blue cursor, response text, waveform, and spinner. Cursor animation, bezier-arc pointing, multi-monitor mapping. |
| `CompanionResponseOverlay.swift` | ~217 | SwiftUI response text bubble and waveform next to the cursor. |
| `CompanionScreenCaptureUtility.swift` | ~132 | Multi-monitor JPEG screenshots via ScreenCaptureKit. |
| `BuddyDictationManager.swift` | ~860 | Push-to-talk voice pipeline: `AVAudioEngine` mic capture, permission checks, transcript finalization, shortcut parsing, keyterms, audio levels for the waveform. |
| `BuddyTranscriptionProvider.swift` | ~30 | Protocols for the transcription provider/session. |
| `AppleSpeechTranscriptionProvider.swift` | ~150 | On-device Apple Speech provider, used as the fallback while Whisper loads. |
| `GlobalPushToTalkShortcutMonitor.swift` | ~132 | Listen-only `CGEvent` tap publishing press/release transitions. |
| `DesignSystem.swift` | ~880 | Design tokens — `DS.Colors`, `DS.CornerRadius`, shared styles. |
| `WindowPositionManager.swift` | ~262 | Window placement, Screen Recording and Accessibility permission helpers. |

## Build & Run

```bash
# One time per machine: create the local signing certificate
./scripts/create-signing-cert.sh

# Build, sign with the local "hellobhai Local Signing" cert, and relaunch.
# Logs: build/hellobhai.log
./scripts/build-and-run.sh

# Compile check only (unsigned, separate DerivedData, reuses resolved packages):
xcodebuild -project leanring-buddy.xcodeproj -scheme leanring-buddy -derivedDataPath /tmp/hb-dd \
  -clonedSourcePackagesDirPath build/DerivedData/SourcePackages CODE_SIGNING_ALLOWED=NO build

# Known non-blocking warnings: Swift 6 concurrency warnings,
# deprecated onChange warning in OverlayWindow.swift. Do NOT attempt to fix these.
```

**Signing and permissions**: macOS ties TCC grants (mic, accessibility, screen recording) to the code signature. `build-and-run.sh` always signs with the same self-signed "hellobhai Local Signing" certificate (in the login keychain), so grants survive rebuilds. Ad-hoc or differently-signed builds lose them. It builds with `ENABLE_DEBUG_DYLIB=NO` because re-signing doesn't cover Xcode's separate `hellobhai.debug.dylib` and dyld refuses to load it. On a new machine, run `./scripts/create-signing-cert.sh` once first (or set a real team in Xcode).

## Code Style & Conventions

### Variable and Method Naming

IMPORTANT: Follow these naming rules strictly. Clarity is the top priority.

- Be as clear and specific with variable and method names as possible
- **Optimize for clarity over concision.** A developer with zero context on the codebase should immediately understand what a variable or method does just from reading its name
- Use longer names when it improves clarity. Do NOT use single-character variable names
- Example: use `originalQuestionLastAnsweredDate` instead of `originalAnswered`
- When passing props or arguments to functions, keep the same names as the original variable. Do not shorten or abbreviate parameter names. If you have `currentCardData`, pass it as `currentCardData`, not `card` or `cardData`

### Code Clarity

- **Clear is better than clever.** Do not write functionality in fewer lines if it makes the code harder to understand
- Write more lines of code if additional lines improve readability and comprehension
- Make things so clear that someone with zero context would completely understand the variable names, method names, what things do, and why they exist
- When a variable or method name alone cannot fully explain something, add a comment explaining what is happening and why

### Swift/SwiftUI Conventions

- Use SwiftUI for all UI unless a feature is only supported in AppKit (e.g., `NSPanel` for floating windows)
- All UI state updates must be on `@MainActor`
- Use async/await for all asynchronous operations
- Comments should explain "why" not just "what", especially for non-obvious AppKit bridging
- AppKit `NSPanel`/`NSWindow` bridged into SwiftUI via `NSHostingView`
- All buttons must show a pointer cursor on hover
- For any interactive element, explicitly think through its hover behavior (cursor, visual feedback, and whether hover should communicate clickability)

### Do NOT

- Do not add features, refactor code, or make "improvements" beyond what was asked
- Do not add docstrings, comments, or type annotations to code you did not change
- Do not try to fix the known non-blocking warnings (Swift 6 concurrency, deprecated onChange)
- Do not rename the project directory or scheme (the "leanring" typo is legacy; renaming breaks the Xcode project)
- Build with `scripts/build-and-run.sh`, not ad-hoc signed `xcodebuild` — other signatures invalidate TCC permissions

## Git Workflow

- Branch naming: `feature/description` or `fix/description`
- Commit messages: imperative mood, concise, explain the "why" not the "what"
- Do not force-push to main

## Self-Update Instructions

<!-- AI agents: follow these instructions to keep this file accurate. -->

When you make changes to this project that affect the information in this file, update this file to reflect those changes. Specifically:

1. **New files**: Add new source files to the "Key Files" table with their purpose and approximate line count
2. **Deleted files**: Remove entries for files that no longer exist
3. **Architecture changes**: Update the architecture section if you introduce new patterns, frameworks, or significant structural changes
4. **Build changes**: Update build commands if the build process changes
5. **New conventions**: If the user establishes a new coding convention during a session, add it to the appropriate conventions section
6. **Line count drift**: If a file's line count changes significantly (>50 lines), update the approximate count in the Key Files table

Do NOT update this file for minor edits, bug fixes, or changes that don't affect the documented architecture or conventions.
