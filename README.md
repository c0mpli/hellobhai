# hellobhai

A local-first AI buddy, bhai, that lives in a small orb on your Mac's screen. Hold **Control + Option** and talk in **Hindi, English or Hinglish** (or type to it in a side chat), and it looks at your screen, talks back, points at things, sketches live explanations, and can click and type to get things done for you.

An open-source, local-first take on [HeyClicky](https://heyclicky.com), built on top of Farza's open-source [Clicky](https://github.com/farzaa/clicky).

## Demo

**"How do these buses actually work?"** A 3D model built right over the video, then a live whiteboard of the swivel joint.


https://github.com/user-attachments/assets/f3f0f3b9-85ad-42b9-aef5-4ef3e5641740


**"What is this device?"** A Raspberry Pi hotspot, modelled part by part over the YouTube video.



https://github.com/user-attachments/assets/305cff8a-7777-4671-906f-11a68852630f


## What it does

- **Talks Hinglish.** Speak however you normally do; it auto-detects Hindi or English and replies the same way, in natural Hinglish.
- **Sees your screen.** Every question comes with a screenshot of all your displays.
- **Lives in an orb.** A small glowing orb sits in the corner of your screen and shows what it's doing: listening, thinking, speaking (only while it's actually talking), or dozing. Drag it anywhere; click it to chat. Pick from 10 colors in the menu bar panel. Prefer the classic look? Switch the voice style to **Cursor** and a blue pointer stays next to your mouse instead, showing a waveform while it listens and a spinner while it thinks.
- **Chats too.** Click the orb (or set the hotkey to Chat mode) and a dark chat card slides in from the right or left edge. Typed questions go through the same pipeline as spoken ones: it sees your screen, points, draws and can act for you. Replies stream in as text, or text plus voice if you pick it; spoken questions always get a spoken answer, shown in the chat too. Minimize it to a small tab on the screen edge (Esc), which shows a dot when a reply arrives.
- **Points at things.** A blue pointer flies out of the orb to the button or menu it's talking about, then back in.
- **Does things for you.** "Zen mein naya tab kholo", "meri latest recording Rahul ko WhatsApp pe bhejo". It mixes Mac tools (AppleScript, `open`, Spotlight, Shortcuts) with on-screen clicks and typing, checking the screen after each step. Anything that sends, posts or deletes needs your "haan", spoken or typed.
- **Explains the simple way.** Every explanation starts from something you have already lived, like a chai tapri, a dabbawala or your phone's contact list, maps the idea onto it piece by piece, and only then gives the real term.
- **Explains like a teacher at a whiteboard.** When a picture helps, an Excalidraw-style board pops up beside your cursor. Claude explains step by step out loud and draws each part as it says it: groups, boxes, hand-drawn icons, labelled arrows, numbered steps, sticky notes, and things travelling along the arrows, all laid out automatically so nothing overlaps, on an infinite canvas you can pan and zoom. Narration follows your language; the board is always in English.
- **Builds 3D models.** Ask how a machine, gadget or device works and it builds a true-to-size 3D model that lines up with the real thing on your screen (same angle, same spot), part by part, with labels pointing at each piece and arrows for airflow or force. Drag to rotate.
- **Circles things.** It circles the exact text or values it's talking about on your screen, in gold. You can circle things too: hold Control + Option, drag around something and ask "what is this?".
- **Draws real math.** Exact geometry, plotted functions, shaded integrals, tangents, vectors, number lines and properly typeset equations, built up step by step as it explains.
- **Writes live notes.** "Notes bana do" or "summarize this page" and it writes notes beside your cursor as it talks: headings, highlighted key terms, syntax-highlighted code, comparison tables, sketch charts, and to-dos. Say "save kar do" to put them in Apple Notes.
- **Shows when it's talking.** A Dynamic-Island-style pill grows out of the notch while it speaks.

## How it works

Hold the hotkey and talk (or type in the chat). Whisper turns your voice into text on your machine, a screenshot of every display is taken, and both go to Claude through your own Claude Code login (`claude -p`). Claude replies with words to say plus small instructions like `[POINT:…]`, `[CIRCLE:…]`, `[SHOW:…]` or `[MODEL3D:…]`; the app speaks the words with an on-device voice and follows the instructions on your screen.

The full walkthrough, diagrams, the pieces and where they live, and exactly what leaves your machine are in [docs/architecture.md](docs/architecture.md).

## Platforms

| Platform | Status | Folder |
|----------|--------|--------|
| macOS (Apple Silicon, 14.2+) | Ready | [`macos/`](macos) |
| Windows 10/11 (x64) | Preview: built, not yet tested on a real PC | [`tauri/`](tauri) |

## Setup (macOS)

### Prerequisites

- An Apple Silicon Mac on macOS 14.2+ (16GB RAM recommended; 8GB works but is tight)
- Xcode 16+
- [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and logged in. Run `claude` once in a terminal.

### Use Codex instead of Claude

bhai can also think with OpenAI's [Codex CLI](https://github.com/openai/codex). Install it, run `codex login` once in a terminal, then pick **Brain → Codex** in the menu bar panel (General tab). It runs `codex exec` with your ChatGPT login and bhai's own instructions, with no shell, connectors or plugins, and without your Codex config, MCP servers or project AGENTS.md files (a global `~/.codex/AGENTS.md`, if you have one, is still read). Pointing, circles, whiteboard, notes, 3D models and on-screen clicks/typing all work. The Mac command tools (osascript, open, mdfind, Shortcuts, saving notes to Apple Notes) are Claude-only, because Codex has no safe command allowlist; with Codex, bhai does those tasks by clicking and typing instead.

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

## Setup (Windows, preview)

The Windows app does the same things as the Mac app (voice in Hindi/English/Hinglish, screenshots, pointing, circles, your own drawings, clicks and typing, whiteboard, notes, 3D models, on-device Whisper and Supertonic-3) except the orb, the typed chat and Codex. The blue pointer follows your mouse instead. It has only been compile-checked so far; expect rough edges.

1. Windows 10/11 x64 with [Claude Code](https://docs.anthropic.com/en/docs/claude-code) installed and logged in (run `claude` once).
2. Get the installer from the `hellobhai-windows-installer` artifact of the latest `tauri` GitHub Actions run, or build it: `cd tauri && npm install && npx tauri build --bundles nsis`.
3. Run it (it is unsigned, so SmartScreen warns). A tray icon appears; on first launch it downloads about 1.1 GB of speech models once.
4. Hold **ctrl + win** and talk. Click the tray icon for settings.

Details: [`tauri/README.md`](tauri/README.md).

## Using it

| Say | It will |
|-----|---------|
| "what's on my screen?" | Describe it and point at what matters |
| "UPI kaise kaam karta hai?" | Explain in Hinglish while sketching the flow live |
| "Zen mein naya tab kholo" | Do it |
| "latest recording Rahul ko WhatsApp pe bhejo" | Find the file, open the chat, attach it, and **ask before sending** |

Pressing the hotkey again interrupts whatever it's doing, in either mode.

### Menu bar panel

Click the orb glyph in the menu bar to open the panel:

- **Brain**: Claude (default) or Codex, and the Claude model: Fable, Opus, Sonnet (default) or Haiku.
- **Buddy**: voice style (**Orb** or **Cursor**), and the orb's size and color.
- **Chat**: hotkey mode (**Voice**: hold to talk; **Chat**: press to open the chat, press again to tuck it away), how typed questions are answered (text, or text + voice), and which screen edge the chat slides in from.
- **Voice**: engine and voice, with previews; speech model status.
- Shortcut, permissions, launch at login, replay onboarding, quit.

## Project structure

```
macos/                     Mac app (Swift; legacy Xcode name leanring-buddy)
  leanring-buddy/            Swift sources + VisualToolkit/ (whiteboard, notes, 3D pages)
  leanring-buddy.xcodeproj
  scripts/                   create-signing-cert.sh, build-and-run.sh
  CLAUDE.md                  Mac architecture for coding agents
tauri/                     Windows app (Tauri; Linux later), preview
  src-tauri/                 Rust crate; OS code only in src/platform/
  ui/                        overlay and tray settings pages
  generated/                 Mac pages and prompts, exported (never edit)
scripts/                   cross-platform scripts (Mac -> tauri/generated exporter)
docs/                      architecture.md, mobile-plan.md
CLAUDE.md                  repo map + shared conventions (AGENTS.md links here)
```

## License

MIT, see `LICENSE`.
