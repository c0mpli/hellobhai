# hellobhai for Windows (Tauri) - Agent Instructions

Shared naming and clarity rules are in ../CLAUDE.md. Layout, regenerating `generated/`, checking from a Mac and building on Windows are in `README.md`. Nothing here has run on Windows yet; verification is `cargo check` (host), `scripts/check-windows-target.sh` and `cargo test`. The assistant is called "bhai" in all UI text; "hellobhai" is only the app name.

## Architecture

- **App type**: tray-only Tauri 2 app (Rust + WebView2), plain HTML/JS pages with no framework or bundler. Every window is created in Rust; `RunEvent::ExitRequested` is prevented so closing windows never quits.
- **OS boundary**: only `src-tauri/src/platform/` calls OS APIs. `platform/windows/` and `platform/unsupported/` expose the same free functions (no traits); Linux later = add `platform/linux/` with the same file names.
- **Brain** (`companion_brain.rs`, port of the Mac's `CompanionManager`): hotkey press cancels everything (aborts the response and visual tasks, which kills a running `claude` via `kill_on_drop`; stops speech; finishes and stops the board; closes the 3D window; emits `clear-annotations`), lets the overlays take mouse drags for user drawing and starts recording. Release stops recording, transcribes with Whisper, then `send_transcript` (or "What is this?" when the user only drew). `send_transcript` takes plain text so a typed chat panel can reuse it. Agent loop of up to 8 rounds: screenshot every monitor -> `claude -p` with the voice prompt and the platform tool allowlist -> `response_tags::parse_reply` (Mac order: actions, CLOSE_DRAWING, SHOW, NOTES, MODEL3D, CIRCLE, POINT) -> point, speak (after the previous speech ends), circle, act; stops on no actions or a repeated action list. Every round is appended to the last-20 history with an action summary. No lock is held across an await.
- **Claude** (`claude_cli.rs`): the Mac's flags in the Mac's order, but `--system-prompt-file` (prompt files are written to `app_local_data_dir/prompts` on every launch) instead of `--system-prompt`, because of the Windows command-line limit. Runs from the temp dir, stdin written and stderr drained on their own tasks, `CREATE_NO_WINDOW` and a kill-on-close job object (`platform/*/child_process.rs`). Voice turns add `platform::voice_turn_tool_arguments` (PowerShell allowlist) and `platform::voice_turn_environment`; side calls use `--tools ""`, and board/notes/3D calls `--effort low`.
- **Side calls**: `[SHOW]`/`[NOTES]`/`[MODEL3D]` start a second claude call whose JSON lines go `StreamedLineForwarder` -> `NarratedStepQueue` -> board or 3D window; the queue plays `{"t":"say"}` steps in lockstep with `SpeechPlayer::speak_text_and_wait_until_finished` while the script is still streaming (`tokio::join!`). Narrations (and notes as plain text) are appended to the last history entry. Esc or the 3D close button aborts the narration.
- **Prompts**: `generated/prompts/*.txt` are exported from the Mac app with Windows wording (`prompt-substitutions.json`) and compiled in with `include_str!`. Never edit them by hand.
- **Settings**: `app_config_dir/settings.json` (claude model alias, Supertonic voice, push-to-talk shortcut, onboarding done); missing or corrupt means defaults.

## Rust -> page events (emitted to every window)

`buddy-voice-state {state}`, `audio-level {level}`, `point-at {x, y, bubbleText}` (global physical pixels; bubble text is the click label or the onboarding comment, else null), `screen-circles {circles: [{x, y, width, height, appearDelaySeconds}]}`, `fade-annotations`, `clear-annotations`, `onboarding-text {text, visible}`. The window module adds `pointer-state`, `pen-position`, `drawing-enabled`, `speaking-changed`, `status-changed`.

## Key files (`src-tauri/src/`)

| File | Lines | Purpose |
|------|-------|---------|
| `lib.rs` | ~130 | Builder, plugins, command registration, startup wiring. |
| `companion_brain.rs` | ~820 | Central state machine: hotkey, agent loop, side calls, circles, onboarding. |
| `claude_cli.rs` | ~210 | Spawns `claude -p`, streams text deltas, maps errors like the Mac. |
| `stream_json.rs` | ~170 | stream-json output parsing, stdin message with base64 JPEGs, history folding. |
| `response_tags.rs` | ~380 | Tag parsers with the Swift grammar, plus tests. |
| `narrated_step_queue.rs` | ~360 | `StreamedLineForwarder` + `NarratedStepQueue` (lockstep narration). |
| `prompts.rs` | ~180 | Embedded system prompts, prompt files, per-request prompt texts. |
| `conversation_history.rs` | ~65 | Last 20 exchanges. |
| `settings.rs`, `app_paths.rs`, `app_state.rs` | small | Settings file, data folders, shared state for commands. |
| `audio_capture.rs`, `whisper_transcriber.rs`, `push_to_talk.rs`, `downloads.rs` | | Voice input: cpal mic, whisper-server sidecar, hotkey tracker, first-launch downloads. |
| `speech_player.rs`, `supertonic.rs`, `sentence_splitter.rs` | | Voice output: Supertonic-3 F2 over ONNX Runtime, sentence pipelining, system-voice fallback. |
| `screen_capture.rs`, `screen_mapping.rs`, `key_combo.rs`, `computer_control.rs` | | Screenshots, screenshot <-> global mapping, `[CLICK]/[TYPE]/[KEY]` input. |
| `overlay_windows.rs`, `board_panel.rs`, `model3d_window.rs`, `tray_and_settings.rs`, `ui_commands.rs`, `page_bridge.rs` | | Overlays, board/notes window, 3D window, tray + settings panel, page bridge. |
| `platform/` | | OS code (`windows/`, `unsupported/` stubs). |

CI: `.github/workflows/tauri.yml` runs `cargo test` and builds the NSIS installer on windows-latest. Never run `tauri build` on the Mac.

When something here changes (new or deleted files, architecture, build steps, line counts drifting by more than 50), update this file in the same change.
