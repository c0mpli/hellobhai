# Memory and personalization plan

Goal: bhai learns from day-to-day use who the user is, what they work on, and how they like to be answered (length, language, tone, voice), and uses that to answer better. Everything stays on the user's machine, in line with the rest of hellobhai: no servers, no new accounts, and the only model calls are the user's own `claude -p` / `codex exec`.

Status: plan only. Nothing here is built yet.

## What exists today

- `conversationHistory`: the last 20 exchanges, held in memory only. It is per chat tab and gone when the app quits.
- Each turn sends screenshots of every screen, so bhai already "sees" the current activity, but only at the moment the user asks.
- Voice turns already have read-only `Read`/`Glob`/`Grep` tools, so Claude can search local files when it is pointed at them.

## How others do it (Nov 2026)

- **ChatGPT and Claude apps.** They keep a short profile of facts and preferences that is injected into every prompt. A background pass rewrites it from recent chats, and the user can view and edit it. This works well and is cheap.
- **Letta (MemGPT).** Uses tiers: small "core" memory always in the prompt, searchable recall of past conversations, and an archival store. The model edits its own memory with tools.
- **Mem0 / Zep.** Memory layers that pull facts out of each exchange into a vector or graph store and retrieve the relevant ones per query. They are built for multi-user servers, which is more than a single local user needs.
- **Screen history (screenpipe, Microsoft Recall; Rewind is shut down).** These record the screen continuously with OCR and make it searchable. They are powerful but a large privacy risk: Recall's encrypted data was extracted by a researcher in 2026. screenpipe can expose its history as an MCP server.

The lesson for us is that a small curated profile plus a searchable local log, refreshed by a periodic reflection pass, gets most of the benefit. Continuous screen recording is the risky part, and we should make it opt-in or leave it to screenpipe.

## Design

All files go in `~/Library/Application Support/hellobhai/Memory/`. On Windows/Linux they go in the app data folder with the same layout, so the format is shared. They are plain Markdown and JSONL, so the user can open, read and edit them.

| File | What it holds | Who writes it |
|------|---------------|---------------|
| `profile.md` | Who the user is, their projects and tools, people they mention, and how they like answers: length, language and script, tone, analogies or not, voice and speed. Kept under about 1,500 tokens. | Reflection pass, explicit "remember" requests |
| `episodes/YYYY-MM-DD.jsonl` | One line per turn: time, question, a short reply summary, the frontmost app and window title, and signals (interrupted, corrected, asked to simplify, typed or spoken). | The app, after every turn |
| `days/YYYY-MM-DD.md` | A short summary of each day: what they worked on, what they asked, what went well or badly. | Reflection pass |
| `activity/YYYY-MM-DD.jsonl` (opt-in) | The frontmost app and window title every 60 s. No screenshots, no text. | The app |

### 1. Use it in every answer

- `profile.md` is added to the voice and chat system prompts under "what you know about the user". The board, notes and 3D prompts get only the style part.
- Voice turns get `--add-dir <Memory folder>` plus one prompt line: "past days are in days/ and episodes/; grep them when the user refers to earlier work ('what was I doing yesterday', 'that thing we discussed')". This reuses the existing read-only tools, so no vector database is needed at first.

### 2. Explicit memory

- New tags: `[REMEMBER:fact]` and `[FORGET:fact]`, parsed like the other tags. They are used when the user says "remember that...", "don't ever...", "I prefer...", "forget...". The app edits `profile.md` straight away and bhai confirms it in the reply.
- Settings → new Memory tab: on/off, "Open memory folder", "Forget everything" (with a confirm), activity tracking on/off, and excluded apps.

### 3. Learning from behaviour (the reflection pass)

- Once a day the app runs one `claude -p` call with the current `profile.md`, that day's episodes (and activity if enabled), and a fixed prompt. It runs at the first idle moment after midnight, or at launch if a day was missed. The call returns an updated profile and a `days/` summary.
- Signals it uses, collected for free per turn:
  - Interrupted during speech → replies too long.
  - "No, I meant..." right after a reply → misunderstood.
  - "Simplify" or "in Hindi" → style preference.
  - Typed vs spoken, time of day, and which apps the questions come from.
- The profile diff is kept in `profile.history/` so a bad rewrite can be undone.

### 4. Day-to-day activity (opt-in)

- Activity tracking samples only the frontmost app name and window title. Password managers, banking and private browser windows are excluded by default, and the user can add more.
- It is enough for "what was I working on Tuesday" and for the reflection pass to learn routines (for example, "codes in Xcode in the evenings, watches lectures in the morning").
- Full screen history: if screenpipe is installed, point Claude at its MCP server instead of building OCR recording ourselves.

## Privacy

- Nothing new leaves the machine except what already does: the profile and relevant snippets go to the user's own Claude/Codex CLI as part of the prompt, next to the screenshots that are already sent.
- Memory is on by default. Activity tracking is off by default. Everything can be wiped from settings.
- `docs/architecture.md` "What leaves your machine" gets updated in the same change.

## Phases

1. **Profile and explicit memory**: `profile.md`, the REMEMBER/FORGET tags, prompt injection and the Memory settings tab. This is the biggest win for the least work.
2. **Episodes and reflection**: the per-turn log with signals, the daily reflection pass, day summaries, and grep access for "earlier" questions.
3. **Activity tracking**: the opt-in app and window-title sampler with exclusions.
4. **Later, if needed**: screenpipe MCP, and embeddings search once grep stops being enough.

Each phase is built on the Mac first, then ported to `tauri/` (platform parity).
