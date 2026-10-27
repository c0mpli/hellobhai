# hellobhai

A local-first AI buddy that lives next to your cursor on macOS. Hold **Control + Option**, ask a question, and it looks at your screen, talks back, and can fly over to point at the thing you're looking for.

Everything runs on your Mac:

| Piece | How |
|-------|-----|
| AI | `claude -p` (Claude Code CLI) as a subprocess — uses your existing Claude Code login, no API keys |
| Speech-to-text | Apple Speech, on-device (`requiresOnDeviceRecognition`) |
| Text-to-speech | `AVSpeechSynthesizer`, on-device |
| Screen capture | ScreenCaptureKit, only when you hold the hotkey |

No servers, no proxy, no analytics, no auto-updater.

## Setup

### Prerequisites

- macOS 14.2+
- Xcode 16+
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and logged in — run `claude` once in a terminal

### Build & run

```bash
open leanring-buddy.xcodeproj
```

1. Select the `leanring-buddy` scheme
2. Set your signing team under Signing & Capabilities
3. **Cmd + R**

The app shows up in the menu bar (no dock icon). Click it, grant permissions, hit Start.

### Better voice (recommended)

The default system voice is robotic. Download a good one:

**System Settings → Accessibility → Spoken Content → System Voice → Manage Voices** → pick any **Premium** or **Enhanced** English voice (e.g. Zoe, Ava, Evan). hellobhai automatically uses the highest-quality installed voice for your language.

### Permissions

- **Microphone** — push-to-talk
- **Speech Recognition** — on-device transcription
- **Accessibility** — global Control + Option hotkey
- **Screen Recording** — screenshot when you press the hotkey

## How it works

Menu bar app with two `NSPanel`s — the dropdown panel and a full-screen transparent cursor overlay. Push-to-talk streams mic audio to Apple's on-device speech recognizer. On release, the transcript + screenshots of every display are piped as stream-json into `claude -p --tools "" --model sonnet|opus`. The reply is spoken with `AVSpeechSynthesizer`. Claude can append `[POINT:x,y:label:screenN]` to make the blue cursor fly to a UI element on any monitor.

See `AGENTS.md` for the full architecture.

## Project structure

```
leanring-buddy/                  # Swift source
  CompanionManager.swift            # Central state machine
  CompanionPanelView.swift          # Menu bar panel UI
  ClaudeCLI.swift                   # `claude -p` subprocess client
  LocalTTSClient.swift              # On-device TTS
  AppleSpeechTranscriptionProvider.swift  # On-device STT
  BuddyDictationManager.swift       # Push-to-talk pipeline
  OverlayWindow.swift               # Blue cursor overlay
AGENTS.md                        # Architecture doc (CLAUDE.md symlinks here)
```

## License

MIT — see `LICENSE`.
