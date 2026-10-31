# hellobhai for Windows (Tauri)

The Windows version of hellobhai, built with Tauri 2 (Rust + WebView2) and plain HTML/JS with no framework or bundler. Linux may follow from the same crate later. Status: feature-complete port, compile-checked and unit-tested from a Mac; **nothing here has run on Windows yet** (see "What is verified" below).

## Requirements

- Windows 10 or 11, x64.
- WebView2 runtime (built into Windows 11; the installer fetches it on Windows 10 if missing).
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and logged in: run `claude` once in a terminal.
- About 1.1 GB free for the first-launch downloads.
- To build: Rust stable (MSVC toolchain), Node 22+, Visual Studio Build Tools with the C++ workload.

## Using it

Hold **ctrl + win** and talk (Hindi, English or Hinglish); let go to send. Pressing it again interrupts whatever bhai is doing. While holding it you can drag on the screen to circle something; drawing without speaking asks "What is this?". Click the tray icon for the settings panel: Claude model (Fable, Opus, Sonnet or Haiku), voice with preview, shortcut (ctrl + win, ctrl + left alt, shift + ctrl, ctrl + alt + space, shift + ctrl + space), status of Claude, speech model, voice and microphone, launch at login, replay onboarding, quit. ctrl + alt is not offered alone because Windows reports AltGr as ctrl + alt.

## First-launch downloads

Into `%LOCALAPPDATA%\ai.hellobhai.desktop\Models`, once, each size-checked:

| What | Size | From |
|------|------|------|
| whisper.cpp `whisper-server` b5454 (x64) | ~9 MB | GitHub releases |
| Whisper `ggml-large-v3-turbo-q5_0.bin` | ~574 MB | Hugging Face |
| ONNX Runtime 1.31.1 (x64) | ~83 MB | GitHub releases |
| Supertonic-3 model + voice F2 | ~398 MB | Hugging Face (pinned revision) |

Until Whisper is ready bhai says its ears are still downloading; until Supertonic is ready it speaks with the Windows system voice.

## What is verified

Verified from a Mac: `cargo check` on the host, `cargo check --target x86_64-pc-windows-msvc` (including tests), `cargo test` (tag parsing, stream-json, history folding, narrated step ordering, screen mapping, key combos, shortcut tracking incl. AltGr, sentence splitting, Supertonic text cleanup, downloads and zip safety, window placement), the overlay page against stubbed browser APIs in Node, and the settings page in Chrome. `tauri/generated/` matches the Mac sources.

Not verified (no Windows machine): anything at runtime on Windows, including the keyboard hook and the Start-menu suppression, WASAPI mic capture, the whisper-server and ONNX Runtime downloads, startup and inference, Supertonic and WinRT speech output, GDI capture (black windows, mixed DPI), `SendInput` clicks/typing/keys (Devanagari, elevated apps), the pointer-hidden check, finding `claude.exe` and whether Claude Code honours the PowerShell allow/deny rules, `--system-prompt-file` with Windows paths, the job object, WebView2 transparency + click-through + non-focusable windows, `content_protected` keeping our windows out of screenshots, tray toggle timing, the 3D window's hit regions and Esc, and the NSIS installer itself (built only in CI). The two Windows-only unit tests (`input.rs` key codes, `claude_tools.rs` arguments) run only in CI.

## Layout

| Path | What it is |
|------|------------|
| `src-tauri/` | The Rust crate (`hellobhai`, library `hellobhai_lib`), `tauri.conf.json`, capabilities, icons. |
| `src-tauri/src/platform/` | The only code that calls OS APIs. `windows/` is the real implementation; `unsupported/` holds stubs with the same function names so the crate builds and tests run on a Mac. Linux later means adding `platform/linux/`. |
| `ui/` | Our own pages: per-monitor overlay, tray settings panel, and `page-host-bridge.js`. |
| `generated/` | **Generated, never edit by hand.** The Mac app's board, notes and 3D pages, and `generated/prompts/*.txt` (system prompts with Windows wording). |
| `prompt-substitutions.json` | The Windows edits the exporter makes to the Mac prompts (each must match exactly once). |
| `scripts/check-windows-target.sh` | Type-checks the Windows target from a Mac. |

`tauri.conf.json` embeds `ui/` and the three generated pages as the app's frontend; the prompts are compiled in with `include_str!`.

## Regenerating `generated/`

The Mac app is the single source of truth for the board, notes and 3D pages (built inside `macos/leanring-buddy/VisualExplanationPanel.swift`, `Model3DOverlay.swift` and `VisualToolkit/`) and for every system prompt (`CompanionManager.swift`). After changing any of those, or `prompt-substitutions.json`, run on a Mac with Xcode installed:

```bash
./scripts/export-mac-web-assets-for-tauri.sh   # from the repository root
```

and commit `tauri/generated/`. The script compiles copies of the two Swift page files with the prompt literals and `scripts/export-mac-web-assets/main.swift`, then writes the real generated HTML and prompts. It fails if a substitution no longer matches exactly once, so a Mac prompt edit never silently ships Mac instructions (osascript, cmd+…) to Windows. The pages get one change: their CSP also allows Tauri's IPC (`connect-src ipc: http://ipc.localhost`).

## Checking from a Mac

There is no Windows machine in the loop, so verification is compile-only plus unit tests of platform-independent logic:

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
cd tauri/src-tauri && cargo check && cargo test     # host (macOS): platform/unsupported stubs
cd tauri && ./scripts/check-windows-target.sh        # cargo check --target x86_64-pc-windows-msvc
```

`check-windows-target.sh` exists because tauri-build compiles the Windows resource file (icon, version info) with `llvm-rc`, which is not installed on a Mac; without it the build script panics with `NotAttempted("llvm-rc")`. The script puts `scripts/stub-llvm-rc` (writes an empty `.res`) on PATH for the check only. Real builds happen on Windows (CI: `cargo tauri build --bundles nsis`). `reqwest` uses `native-tls` because `rustls` pulls in `aws-lc-sys`, which needs a C toolchain with `windows.h` and cannot be cross-checked.

## Building on Windows

```powershell
cd tauri
npm install
npx tauri build --bundles nsis
```

For a dev run with logs in the terminal: `npx tauri dev`. The installer lands in `src-tauri/target/release/bundle/nsis/`; CI (`.github/workflows/tauri.yml`) builds the same one on every push to `tauri/` and uploads it as the `hellobhai-windows-installer` artifact. The installer is unsigned, so SmartScreen warns on first run.
