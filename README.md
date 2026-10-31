# hellobhai

A local-first AI buddy that lives next to your cursor on macOS. Hold **Control + Option**, talk in **Hindi, English or Hinglish**, and it looks at your screen, talks back, points at things, sketches live explanations, and can click and type to get things done for you.

An open-source, local-first take on [HeyClicky](https://heyclicky.com), built on top of Farza's open-source [Clicky](https://github.com/farzaa/clicky).

## Demo

**"How do these buses actually work?"** A 3D model built right over the video, then a live whiteboard of the swivel joint.


https://github.com/user-attachments/assets/f3f0f3b9-85ad-42b9-aef5-4ef3e5641740


**"What is this device?"** A Raspberry Pi hotspot, modelled part by part over the YouTube video.



https://github.com/user-attachments/assets/305cff8a-7777-4671-906f-11a68852630f


## What it does

- **Talks Hinglish.** Speak however you normally do; it auto-detects Hindi or English and replies the same way, in natural Hinglish.
- **Sees your screen.** Every question comes with a screenshot of all your displays.
- **Lives in an orb.** A small glowing orb sits in the corner of your screen and shows what it's doing: listening, thinking, speaking, or dozing. Drag it anywhere; click it to chat. Pick its color in the menu bar panel.
- **Points at things.** A blue pointer flies out of the orb to the button or menu it's talking about, then back in.
- **Does things for you.** "Zen mein naya tab kholo", "meri latest recording Rahul ko WhatsApp pe bhejo". It mixes Mac tools (AppleScript, `open`, Spotlight, Shortcuts) with on-screen clicks and typing, checking the screen after each step. Anything that sends, posts or deletes needs your spoken "haan".
- **Explains the simple way.** Every explanation starts from something you have already lived, like a chai tapri, a dabbawala or your phone's contact list, maps the idea onto it piece by piece, and only then gives the real term.
- **Explains like a teacher at a whiteboard.** When a picture helps, an Excalidraw-style board pops up beside your cursor. Claude explains step by step out loud and draws each part as it says it: groups, boxes, hand-drawn icons, labelled arrows, numbered steps, sticky notes, and things travelling along the arrows, all laid out automatically so nothing overlaps, on an infinite canvas you can pan and zoom. Narration follows your language; the board is always in English.
- **Builds 3D models.** Ask how a machine, gadget or device works and it builds a true-to-size 3D model that lines up with the real thing on your screen (same angle, same spot), part by part, with labels pointing at each piece and arrows for airflow or force. Drag to rotate.
- **Circles things.** It circles the exact text or values it's talking about on your screen, in gold. You can circle things too: hold Control + Option, drag around something and ask "what is this?".
- **Draws real math.** Exact geometry, plotted functions, shaded integrals, tangents, vectors, number lines and properly typeset equations, built up step by step as it explains.
- **Writes live notes.** "Notes bana do" or "summarize this page" and it writes notes beside your cursor as it talks: headings, highlighted key terms, syntax-highlighted code, comparison tables, sketch charts, and to-dos. Say "save kar do" to put them in Apple Notes.
- **Shows when it's talking.** A Dynamic-Island-style pill grows out of the notch while it speaks.

## How it works

Hold the hotkey and talk. Whisper turns your voice into text on your machine, a screenshot of every display is taken, and both go to Claude through your own Claude Code login (`claude -p`). Claude replies with words to say plus small instructions like `[POINT:…]`, `[CIRCLE:…]`, `[SHOW:…]` or `[MODEL3D:…]`; the app speaks the words with an on-device voice and follows the instructions on your screen.

The full walkthrough, diagrams, the pieces and where they live, and exactly what leaves your machine are in [docs/architecture.md](docs/architecture.md).

## Platforms

| Platform | Status | Folder |
|----------|--------|--------|
| macOS (Apple Silicon, 14.2+) | Ready | [`macos/`](macos) |
| Windows 10/11 | In progress | `windows/` |

## Setup (macOS)

### Prerequisites

- An Apple Silicon Mac on macOS 14.2+ (16GB RAM recommended; 8GB works but is tight)
- Xcode 16+
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and logged in. Run `claude` once in a terminal.

### Build & run

```bash
cd macos
./scripts/create-signing-cert.sh   # one time: local signing certificate
./scripts/build-and-run.sh         # build, sign, launch (run again after every change)
```

`build-and-run.sh` signs every build with the same local certificate, so macOS remembers the permissions you grant instead of asking again after each rebuild. Logs go to `macos/build/hellobhai.log`.

### First launch

1. An orb icon appears in the menu bar and the setup panel opens.
2. Grant **Microphone**, **Accessibility** and **Screen Recording**. Screen Recording needs one app restart; just run the script again.
3. Click **Start**.
4. Give it a couple of minutes: Whisper, Kokoro and Supertonic download (~1.2GB), and macOS compiles Whisper for the Neural Engine once. Until then it uses Apple's speech recognition (this needs **Dictation** turned on in System Settings → Keyboard) and the system voice.

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
macos/                     Mac app (Swift; legacy Xcode name leanring-buddy)
  leanring-buddy/            Swift sources + VisualToolkit/ (whiteboard, notes, 3D pages)
  leanring-buddy.xcodeproj
  scripts/                   create-signing-cert.sh, build-and-run.sh
  CLAUDE.md                  Mac architecture for coding agents
windows/                   Windows app (Tauri), in progress
scripts/                   cross-platform scripts
docs/                      architecture.md, mobile-plan.md
CLAUDE.md                  repo map + shared conventions (AGENTS.md links here)
```

## License

MIT, see `LICENSE`.
