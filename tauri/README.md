# hellobhai for Windows (Tauri)

The Windows version of hellobhai, built with Tauri 2 (Rust + WebView2) and plain HTML/JS with no framework or bundler. Linux may follow from the same crate later. Status: being built; nothing here has run on Windows yet.

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

The installer is unsigned, so SmartScreen warns on first run. On first launch the app downloads about 1.06 GB (whisper-server, the Whisper model, ONNX Runtime, Supertonic-3) into `%LOCALAPPDATA%\ai.hellobhai.desktop\Models`.
