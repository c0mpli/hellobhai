# hellobhai on mobile (future)

Notes for porting hellobhai to phones later. Written 2026-10-31.

## Verdict

Build **Android first**. Almost the whole Mac design (screenshot -> Claude -> tags -> overlay) maps one to one, and Android is the bigger market for Indian users. iPhone can only do a reduced "share your screen and ask" version.

## Android: what maps to what

| Mac today | Android equivalent |
|---|---|
| ScreenCaptureKit screenshots | `MediaProjection` (user grants screen capture once per session) |
| Transparent overlay window (buddy, gold circles, whiteboard, 3D labels) | `SYSTEM_ALERT_WINDOW` ("display over other apps") |
| User circles things with the mouse | User circles with a finger on the overlay |
| `[CLICK]` / `[TYPE]` / `[KEY]` via CGEvent | `AccessibilityService` (`dispatchGesture`, `performAction`, set text); it can also read the exact UI tree, which is better than guessing from pixels |
| ctrl+option push-to-talk | Floating bubble, or register as the default assistant (long-press power) |
| Whiteboard + 3D (rough.js, dagre, KaTeX, three.js in WKWebView) | Same HTML in an Android `WebView`, mostly as-is |
| Whisper (WhisperKit) | Small on-device Whisper-class model |
| Supertonic-3 / Kokoro TTS | Same class of small model on-device |
| `claude -p` CLI | No CLI on phones: Anthropic API behind our own backend |

**Risk:** Google Play strictly reviews Accessibility Service use for non-accessibility purposes. Either justify it in the listing/policy declaration or ship a version without auto-tapping (explain and point only).

## iPhone: the limits

- No drawing over other apps. Closest options: a picture-in-picture floating window (known workaround), Dynamic Island / Live Activities for the speaking indicator.
- Seeing the screen needs a ReplayKit Broadcast Upload Extension that the user starts manually; it has a ~50MB memory limit.
- No tapping or typing in other apps.
- Result: "share your screen, talk, get answers plus a whiteboard", not a buddy on top of everything.

## Order of work when we start

1. Backend + Anthropic API replacing `claude -p` (needed for mobile anyway).
2. Android: MediaProjection + overlay + push-to-talk + WebView whiteboard (explain and point).
3. Android: user finger circles + Claude gold circles.
4. Android: AccessibilityService actions, after checking Play policy.
5. iPhone reduced version.
