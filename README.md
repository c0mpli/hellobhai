# hellobhai

A local-first AI buddy that lives next to your cursor on macOS. Hold **Control + Option**, talk in **Hindi, English or Hinglish**, and it looks at your screen, talks back, points at things, sketches live explanations, and can click and type to get things done for you.

## What it does

- **Talks Hinglish.** Speak however you normally do; it auto-detects Hindi or English and replies the same way, in natural Hinglish.
- **Sees your screen.** Every question comes with a screenshot of all your displays.
- **Points at things.** A purple cursor flies over to the button or menu it's talking about.
- **Does things for you.** "Zen mein naya tab kholo", "meri latest recording Rahul ko WhatsApp pe bhejo". It mixes Mac tools (AppleScript, `open`, Spotlight, Shortcuts) with on-screen clicks and typing, checking the screen after each step. Anything that sends, posts or deletes needs your spoken "haan".
- **Explains the simple way.** Every explanation starts from something you have already lived, like a chai tapri, a dabbawala or your phone's contact list, maps the idea onto it piece by piece, and only then gives the real term.
- **Explains like a teacher at a whiteboard.** When a picture helps, an Excalidraw-style board pops up beside your cursor. Claude explains step by step out loud and draws each part as it says it: groups, boxes, hand-drawn icons, labelled arrows, numbered steps, sticky notes, and things travelling along the arrows, all laid out automatically so nothing overlaps, on an infinite canvas you can pan and zoom. Narration follows your language; the board is always in English.
- **Writes live notes.** "Notes bana do" or "summarize this page" and it writes notes beside your cursor as it talks: headings, highlighted key terms, syntax-highlighted code, comparison tables, sketch charts, and to-dos. Say "save kar do" to put them in Apple Notes.
- **Shows when it's talking.** A Dynamic-Island-style pill grows out of the notch while it speaks.

## How it works

| Piece | How |
|-------|-----|
| AI | `claude -p` (Claude Code CLI) as a subprocess, using your existing Claude Code login. No API keys. |
| Speech-to-text | WhisperKit (Whisper large-v3 turbo) on-device, with Apple Speech as the fallback |
| Text-to-speech | Supertonic-3 on-device: one voice for English and Hindi, with the macOS voice as the fallback |
| Actions | `CGEvent` clicks and keystrokes, plus an allowlist of Mac commands (`osascript`, `open`, `mdfind`, `shortcuts`, `ls`) |
| Visuals | Claude streams narration plus JSON drawing commands; each step is drawn with rough.js, Lucide icons and Excalidraw's Virgil font while its sentence is spoken |

No servers, no analytics, no auto-updater. Network use is `claude -p` itself plus a one-time ~1GB model download from Hugging Face on first launch.

## Setup

### Prerequisites

- An Apple Silicon Mac on macOS 14.2+ (16GB RAM recommended; 8GB works but is tight)
- Xcode 16+
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and logged in. Run `claude` once in a terminal.

### Build & run

```bash
./scripts/create-signing-cert.sh   # one time: local signing certificate
./scripts/build-and-run.sh         # build, sign, launch (run again after every change)
```

`build-and-run.sh` signs every build with the same local certificate, so macOS remembers the permissions you grant instead of asking again after each rebuild. Logs go to `build/hellobhai.log`.

### First launch

1. A triangle appears in the menu bar and the setup panel opens.
2. Grant **Microphone**, **Accessibility** and **Screen Recording**. Screen Recording needs one app restart; just run the script again.
3. Click **Start**.
4. Give it a couple of minutes: Whisper and Supertonic download (~1GB), and macOS compiles Whisper for the Neural Engine once. Until then it uses Apple's speech recognition (this needs **Dictation** turned on in System Settings → Keyboard) and the system voice.

## Using it

| Say | It will |
|-----|---------|
| "what's on my screen?" | Describe it and point at what matters |
| "UPI kaise kaam karta hai?" | Explain in Hinglish while sketching the flow live |
| "Zen mein naya tab kholo" | Do it |
| "latest recording Rahul ko WhatsApp pe bhejo" | Find the file, open the chat, attach it, and **ask before sending** |

Pressing the hotkey again interrupts whatever it's doing. Model choice (Sonnet or Opus) is in the menu bar panel.

## Project structure

```
leanring-buddy/                          # Swift source (legacy folder name)
  CompanionManager.swift                    # Central state machine + agent loop + prompts
  ClaudeCLI.swift                           # `claude -p` subprocess client (stream-json in/out)
  WhisperTranscriptionProvider.swift        # On-device STT
  LocalTTSClient.swift                      # On-device TTS (Supertonic-3)
  ComputerControl.swift                     # [CLICK]/[TYPE]/[KEY] parsing + CGEvents
  VisualExplanationPanel.swift              # Live sketch whiteboard
  OverlayWindow.swift                       # Purple cursor + notch speaking pill
  BuddyDictationManager.swift               # Push-to-talk pipeline
scripts/
  create-signing-cert.sh                    # One-time local signing certificate
  build-and-run.sh                          # Build, sign, relaunch
CLAUDE.md                                 # Full architecture doc (AGENTS.md links here)
```

## License

MIT, see `LICENSE`.
