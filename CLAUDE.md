# hellobhai - Agent Instructions

<!-- Single source of truth for AI coding agents. AGENTS.md is a symlink to this file. Platform details live in each platform folder's CLAUDE.md. -->

## Overview

hellobhai is a local-first AI buddy that lives in a small draggable orb on screen; the assistant is called "bhai" in the UI. Hold a hotkey and talk in Hindi, English or Hinglish (or type in its side chat), and it looks at your screen, talks back, points at and circles things, draws live whiteboards, notes and 3D models, and can click and type for you. Speech-to-text and text-to-speech run on-device; the only network traffic is the local `claude -p` CLI (the user's own Claude Code login) plus one-time model downloads. No servers, API keys, analytics or auto-updater.

## Repository layout

| Path | What it is |
|------|------------|
| `macos/` | The Mac app (Swift, SwiftUI + AppKit, Xcode project `leanring-buddy`). Details: `macos/CLAUDE.md`. |
| `tauri/` | The Windows app (Tauri 2: Rust + WebView2, plain HTML/JS; Linux may follow from the same crate). Ported feature for feature except the orb, typed chat and Codex; compile-checked and unit-tested from a Mac, not yet run on Windows. Details: `tauri/CLAUDE.md` and `tauri/README.md`. CI: `.github/workflows/tauri.yml`. |
| `scripts/` | Scripts that span platforms. `export-mac-web-assets-for-tauri.sh` (with `export-mac-web-assets/main.swift`) dumps the Mac app's generated board, notes and 3D pages and its system prompts into `tauri/generated/`; run it on a Mac after changing those and commit the result. Mac-only scripts are in `macos/scripts/`. |
| `docs/` | `architecture.md` (how it works, the tag protocol, what leaves your machine), `mobile-plan.md` (future phone port). |

**The Mac app is the source of truth** for the whiteboard, notes and 3D pages (built inside `macos/leanring-buddy/VisualExplanationPanel.swift` and `VisualToolkit/model3d.html`) and for every system prompt (`CompanionManager.swift`). Other platforms use generated copies; regenerate them after changing those, never edit the copies by hand.

## Product principles (all platforms)

- Narration follows the user's language (natural Hinglish in Devanagari for Hindi/Hinglish speakers); board and panel text is always English.
- No emoji anywhere in the UI or panels.
- Teaching style: direct and exact by default; an everyday analogy only for genuinely complex topics or when the user asks to simplify.
- Computer control only on explicit request; anything that sends, posts or deletes needs a spoken yes.

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

## Do NOT

- Do not add features, refactor code, or make "improvements" beyond what was asked
- Do not rename `macos/leanring-buddy` or the Xcode scheme (the "leanring" typo is legacy; renaming breaks the Xcode project)

## Git Workflow

- Branch naming: `feature/description` or `fix/description`
- Commit messages: imperative mood, concise, explain the "why" not the "what"
- Do not force-push to main

## Self-Update Instructions

<!-- AI agents: follow these instructions to keep this file accurate. -->

When you make changes to this project that affect the information in this file, update this file to reflect those changes. Specifically:

1. **New files**: Add new source files to the platform CLAUDE.md's "Key Files" table with their purpose and approximate line count
2. **Deleted files**: Remove entries for files that no longer exist
3. **Architecture changes**: Update the architecture section if you introduce new patterns, frameworks, or significant structural changes
4. **Build changes**: Update build commands if the build process changes
5. **New conventions**: If the user establishes a new coding convention during a session, add it to the appropriate conventions section
6. **Line count drift**: If a file's line count changes significantly (>50 lines), update the approximate count in the Key Files table

Do NOT update this file for minor edits, bug fixes, or changes that don't affect the documented architecture or conventions.

Platform details belong in that platform's CLAUDE.md; this file only holds the repo map and what is shared.
