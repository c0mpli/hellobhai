// Per-monitor overlay. All positions from Rust are global physical pixels;
// convert with (x - originX) / scaleFactor into this page's CSS px.
// Event names and payloads are the Rust <-> page contract in the plan (section 7).
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const monitorIndex = Number(new URLSearchParams(window.location.search).get('monitor') ?? 0);
const overlayCanvas = document.getElementById('overlay-canvas');
let monitorConfig = null; // { originX, originY, width, height, scaleFactor }

function globalToPageX(globalX) { return (globalX - monitorConfig.originX) / monitorConfig.scaleFactor; }
function globalToPageY(globalY) { return (globalY - monitorConfig.originY) / monitorConfig.scaleFactor; }

async function startOverlay() {
  monitorConfig = await invoke('overlay_ready', { monitorIndex });

  // TODO(D): draw everything below on overlayCanvas.
  await listen('pointer-state', (event) => { /* { x, y, visible } */ });
  await listen('buddy-voice-state', (event) => { /* { state: idle | listening | processing | responding } */ });
  await listen('audio-level', (event) => { /* { level } 0..1 */ });
  await listen('point-at', (event) => { /* { x, y, bubbleText } */ });
  await listen('pen-position', (event) => { /* { x, y } or null */ });
  await listen('screen-circles', (event) => { /* { circles: [{ x, y, width, height, appearDelaySeconds }] } */ });
  await listen('fade-annotations', () => {});
  await listen('clear-annotations', () => {});
  await listen('drawing-enabled', (event) => { /* { enabled }; on pointerup: invoke('user_stroke_added', { points }) */ });
  await listen('speaking-changed', (event) => { /* { isSpeaking } */ });
  await listen('onboarding-text', (event) => { /* { text, visible } */ });
}

startOverlay();
