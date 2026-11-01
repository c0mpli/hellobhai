# hellobhai for Windows and Linux (Tauri)

The Windows and Linux version of hellobhai, one crate built with Tauri 2 (Rust + WebView2 on Windows, WebKitGTK on Linux) and plain HTML/JS with no framework or bundler. Status: feature-complete port of the Mac app (orb and cursor voice styles, typed chat and bhai modes, Claude or Codex), compile-checked for both targets and unit-tested from a Mac, installers built in CI; **nothing here has run on real Windows or Linux hardware yet** (see "What is verified" below and the "Linux" section).

## Requirements

- Windows 10 or 11, x64.
- WebView2 runtime (built into Windows 11; the installer fetches it on Windows 10 if missing).
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and logged in: run `claude` once in a terminal.
- About 1.1 GB free for the first-launch downloads.
- To build: Rust stable (MSVC toolchain), Node 22+, Visual Studio Build Tools with the C++ workload.

## Using it

Hold **ctrl + win** and talk (Hindi, English or Hinglish); let go to send. Pressing it again interrupts whatever bhai is doing. While holding it you can drag on the screen to circle something; drawing without speaking asks "What is this?". In voice mode the orb sits at the right end of the taskbar (drag it anywhere; a click is tap to talk), or pick the cursor style to have the blue pointer follow the mouse. In chat mode the hotkey opens a typed chat that slides in from a screen edge. Click the tray icon for the settings panel: General (bhai mode, brain Claude or Codex with the Claude model or Codex effort, shortcut presets or a recorded combination, microphone, launch at login, replay onboarding), Voice (voice with preview), Chat (how spoken and typed questions are answered, chat side), Buddy (voice style, orb size and colour). ctrl + alt is not offered alone because Windows reports AltGr as ctrl + alt.

## First-launch downloads

Into `%LOCALAPPDATA%\ai.hellobhai.desktop\Models`, once, https only, each checked against a pinned size and SHA-256 before it is used (zips are extracted into a `.partial` folder that is renamed when complete):

| What | Size | From |
|------|------|------|
| whisper.cpp `whisper-server` b5454 (x64) | ~9 MB | GitHub releases |
| Whisper `ggml-large-v3-turbo-q5_0.bin` | ~574 MB | Hugging Face |
| ONNX Runtime 1.31.1 (x64) | ~83 MB | GitHub releases |
| Supertonic-3 model + voice F2 | ~398 MB | Hugging Face (pinned revision) |

Until Whisper is ready bhai says its ears are still downloading; until Supertonic is ready it speaks with the Windows system voice.

## What is verified

Verified from a Mac: `cargo check` on the host, `cargo check --target x86_64-pc-windows-msvc` (including tests), `cargo test` (tag parsing, stream-json, history folding, narrated step ordering, screen mapping, key combos, shortcut tracking incl. AltGr, sentence splitting, Supertonic text cleanup, downloads and zip safety, window placement), the overlay page against stubbed browser APIs in Node, and the settings page in Chrome. `tauri/generated/` matches the Mac sources.

Not verified (no Windows machine): anything at runtime on Windows, including the keyboard hook and the Start-menu suppression, WASAPI mic capture, the whisper-server and ONNX Runtime downloads, startup and inference, Supertonic and WinRT speech output, GDI capture (black windows, mixed DPI), `SendInput` clicks/typing/keys (Devanagari, elevated apps), the pointer-hidden check, finding `claude.exe`, Claude Code's Glob/Grep/Read with Windows paths, `--system-prompt-file` with Windows paths, `[OPEN]` through ShellExecute (Start menu shortcuts, app links), the per-child job objects (cancelling an npm `claude.cmd` must kill node.exe), whether whisper-server and onnxruntime.dll start on a clean PC with the app-local Visual C++ DLLs, the 150 ms chord confirmation that lets win+ctrl+arrow etc. pass through, WebView2 transparency + click-through + non-focusable windows, `content_protected` keeping our windows out of screenshots, tray toggle timing, the 3D window's hit regions and Esc, and the NSIS installer itself (built only in CI). The two Windows-only unit tests (`input.rs` key codes, `claude_tools.rs` arguments) run only in CI.

## Layout

| Path | What it is |
|------|------------|
| `src-tauri/` | The Rust crate (`hellobhai`, library `hellobhai_lib`), `tauri.conf.json`, capabilities, icons. |
| `src-tauri/src/platform/` | The only code that calls OS APIs. `windows/` and `linux/` are the real implementations; `unsupported/` holds stubs with the same function names so the crate builds and tests run on a Mac. |
| `ui/` | Our own pages: per-monitor overlay, typed chat, tray settings panel, `page-host-bridge.js` and `orb-host-bridge.js`. |
| `generated/` | **Generated, never edit by hand.** The Mac app's board, notes, 3D and orb pages, and `generated/prompts/*.txt` (system prompts with Windows wording). |
| `prompt-substitutions.json` | The Windows edits the exporter makes to the Mac prompts (each must match exactly once). |
| `scripts/check-windows-target.sh`, `check-linux-target.sh` | Type-check the Windows and Linux targets from a Mac. |

`tauri.conf.json` embeds `ui/` and the four generated pages as the app's frontend; the prompts are compiled in with `include_str!`.

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

CI also copies `msvcp140.dll`, `vcruntime140.dll` and `vcruntime140_1.dll` from Visual Studio into `src-tauri/vc-runtime/` and builds with `--config src-tauri/tauri.vc-runtime.conf.json`, so the installer carries the Visual C++ runtime that whisper-server and ONNX Runtime need. A local build without that step works on PCs that already have the VC++ redistributable.

For a dev run with logs in the terminal: `npx tauri dev`. The installer lands in `src-tauri/target/release/bundle/nsis/`; CI (`.github/workflows/tauri.yml`) builds the same one on every push to `tauri/` and uploads it as the `hellobhai-windows-installer` artifact. The installer is unsigned, so SmartScreen warns on first run.

## Linux

The same crate builds for Linux x64 (`src-tauri/src/platform/linux/`, same function names as `platform/windows/`). CI's `linux-test-and-build` job (ubuntu-22.04) runs `cargo test` and builds a `.deb` and an AppImage, uploaded as the `hellobhai-linux-packages` artifact. **Nothing has run on a Linux desktop yet.**

**Requirements**: an X11 session (Xorg) for everything to work, WebKitGTK 4.1 (Ubuntu 22.04+, Debian 12+, Fedora 39+), libXtst, libXi, libXfixes and an AppIndicator tray (GNOME needs the AppIndicator extension, which Ubuntu ships). Claude Code installed and logged in (`claude` once in a terminal); it is found in `~/.local/bin`, `~/.claude/local`, `~/.npm-global/bin`, `/usr/local/bin`, `/usr/bin`, then the login shell (`$SHELL -ilc 'command -v claude'`), and `codex` the same way. Optional: `espeak-ng` speaks until the neural voice has downloaded.

**How each piece works on X11**: screenshots with `XGetImage` on the root window; clicks, keys and typing with XTest (characters the layout lacks, like Devanagari, are typed by mapping a spare keycode for one press, as xdotool does); push-to-talk from XInput2 raw key events (listen-only, delivered even while the window manager grabs Super, injected XTEST events ignored), with the same tracker and 150 ms chord confirmation as Windows; the hidden-pointer check through XFixes (an all-transparent cursor image counts as hidden); fullscreen and focus hand-back through `_NET_ACTIVE_WINDOW` / `_NET_WM_STATE`; `[OPEN]` through `xdg-open`, or `gtk-launch` for an installed app's `.desktop` file (matched by file name or its `Name=` line under the XDG, Flatpak and Snap application folders); files with an execute bit or a script/launcher/installer extension are refused. Claude's tools on voice turns are `Read,Glob,Grep` with `--add-dir` for home and the application folders, like Windows. Each `claude`/`whisper-server` child runs in its own process group (killed as a group on cancel) with `PR_SET_PDEATHSIG`. The Windows-worded prompts get Linux wording when the prompt files are written (`platform/linux_prompt_wording.rs`, every substitution must match exactly once, checked by tests). Sound input settings open GNOME Settings, KDE System Settings or pavucontrol.

**Wayland (best effort)**: the app sets `GDK_BACKEND=x11` (unless the user set it) and runs through XWayland, because Wayland gives apps no global window positions, always-on-top or click-through. Then: screenshots come from the desktop's screenshot portal (`org.freedesktop.portal.Screenshot`, non-interactive; the desktop may ask once; one portal shot serves a whole capture round, and the saved PNG is deleted); clicks and typing only reach X11 apps; the pointer is only known over X11 windows; push-to-talk reads `/dev/input/event*` when the user is in the `input` group, otherwise it only hears keys typed into X11 windows. `platform::platform_limitation_note()` gives the settings panel a sentence explaining this. `WEBKIT_DISABLE_DMABUF_RENDERER=1` is set to avoid blank WebKitGTK windows on some GPU drivers.

**First-launch downloads** go to `~/.local/share/ai.hellobhai.desktop/Models`, pinned by size and SHA-256 like Windows (`.tar.gz` archives are extracted into a `.partial` folder; only sibling soname symlinks and rwx bits are kept):

| What | Size | From |
|------|------|------|
| whisper.cpp `whisper-bin-ubuntu-x64.tar.gz` b5454 (`whisper-server` + libwhisper/libggml) | ~10 MB | GitHub releases |
| ONNX Runtime 1.31.1 `onnxruntime-linux-x64-1.31.1.tgz` (`libonnxruntime.so.1.31.1`) | ~11 MB | GitHub releases |

The Whisper and Supertonic models are the same files as on Windows.

**Checking from a Mac**: `./scripts/check-linux-target.sh` (`cargo check --target x86_64-unknown-linux-gnu`, add `--tests` for the tests). It puts `scripts/stub-pkg-config` on PATH so the GTK/WebKitGTK/ALSA sys crates' build scripts pass without Linux libraries, and points openssl-sys at Homebrew's OpenSSL headers with clang aimed at Linux. Needs `rustup target add x86_64-unknown-linux-gnu` and `brew install openssl@3`. The Linux-only unit tests (keysym and input-event mapping, pointer, capture maths, portal paths, download filters, `.desktop` launch ids) run in CI.

**Not verified (no Linux machine)**: everything at runtime, including transparent click-through overlays on WebKitGTK, XInput2 raw events under GNOME/KDE, XTest typing of Devanagari through the spare keycode, XGetImage under compositors, the screenshot portal on GNOME and KDE, `/dev/input` reading on Wayland, the downloaded whisper-server and libonnxruntime starting on a clean system, `gtk-launch` of .desktop files, the tray on GNOME, espeak-ng output, and the .deb/AppImage themselves.
