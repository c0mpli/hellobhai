// Injected into generated/orb.html (the Mac orb page, exported unchanged).
// The page only draws; like the Mac's OrbMouseHandlingView, this script takes
// every mouse event on the orb and hands it to Rust (orb_window.rs), which
// decides drag versus click (3 px) and moves the window in physical pixels.
// Rust sets the page's opacity (asleep is dimmed, hidden for fullscreen video),
// so the fade lives here too.
(() => {
  const invoke = (command, args) => window.__TAURI_INTERNALS__.invoke(command, args).catch(() => {});
  let isPrimaryButtonDown = false;
  let isMoveCommandInFlight = false;
  let hasMoveWaiting = false;

  // One move command at a time; a move that arrives meanwhile is sent once the
  // previous one returns, so a fast drag never queues up a backlog.
  function sendPointerMoved() {
    if (isMoveCommandInFlight) {
      hasMoveWaiting = true;
      return;
    }
    isMoveCommandInFlight = true;
    invoke('orb_pointer_moved').then(() => {
      isMoveCommandInFlight = false;
      if (hasMoveWaiting && isPrimaryButtonDown) {
        hasMoveWaiting = false;
        sendPointerMoved();
      }
    });
  }

  function finishPress(event) {
    if (!isPrimaryButtonDown) return;
    isPrimaryButtonDown = false;
    hasMoveWaiting = false;
    // A cancelled press (the window lost the mouse) never counts as a click.
    invoke('orb_pointer_released', { wasCancelled: event.type === 'pointercancel' });
  }

  // Tells Rust the page is alive; silence means its web process died and it gets reloaded.
  setInterval(() => invoke('orb_page_heartbeat'), 2000);

  // "Memory paused": a small pause badge in the top-right corner (the Mac's
  // pause.circle.fill in white at 85%). It never takes a click.
  let isMemoryPausedMarkerVisible = false;
  let memoryPausedMarkerElement = null;
  function createMemoryPausedMarker() {
    const svgNamespace = 'http://www.w3.org/2000/svg';
    const markerElement = document.createElementNS(svgNamespace, 'svg');
    markerElement.setAttribute('viewBox', '0 0 14 14');
    markerElement.setAttribute('aria-label', 'Memory paused');
    markerElement.style.cssText = 'position:fixed;top:4px;right:4px;width:14px;height:14px;pointer-events:none;z-index:2;';
    const backgroundCircle = document.createElementNS(svgNamespace, 'circle');
    backgroundCircle.setAttribute('cx', '7');
    backgroundCircle.setAttribute('cy', '7');
    backgroundCircle.setAttribute('r', '7');
    backgroundCircle.setAttribute('fill', 'rgba(255,255,255,0.85)');
    markerElement.appendChild(backgroundCircle);
    for (const barX of ['4.6', '7.9']) {
      const pauseBar = document.createElementNS(svgNamespace, 'rect');
      pauseBar.setAttribute('x', barX);
      pauseBar.setAttribute('y', '4');
      pauseBar.setAttribute('width', '1.5');
      pauseBar.setAttribute('height', '6');
      pauseBar.setAttribute('rx', '0.5');
      pauseBar.setAttribute('fill', '#101211');
      markerElement.appendChild(pauseBar);
    }
    return markerElement;
  }
  window.hbSetMemoryPausedMarkerVisible = (shouldShowMarker) => {
    isMemoryPausedMarkerVisible = shouldShowMarker;
    if (!document.body) return;
    if (!memoryPausedMarkerElement) {
      memoryPausedMarkerElement = createMemoryPausedMarker();
      document.body.appendChild(memoryPausedMarkerElement);
    }
    memoryPausedMarkerElement.style.display = shouldShowMarker ? 'block' : 'none';
  };
  window.addEventListener('DOMContentLoaded', () => window.hbSetMemoryPausedMarkerVisible(isMemoryPausedMarkerVisible));

  window.addEventListener('DOMContentLoaded', () => {
    const fadeStyle = document.createElement('style');
    // The pointing hand says the orb can be clicked; the fade matches the Mac's 0.3 s window alpha animation.
    fadeStyle.textContent = 'html { cursor: pointer; transition: opacity 0.3s ease; } canvas { pointer-events: none; }';
    document.head.appendChild(fadeStyle);

    const rootElement = document.documentElement;
    rootElement.addEventListener('pointerdown', (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      // Keeps the moves coming while the window slides under the pointer.
      rootElement.setPointerCapture(event.pointerId);
      isPrimaryButtonDown = true;
      invoke('orb_pointer_pressed');
    });
    rootElement.addEventListener('pointermove', () => {
      if (isPrimaryButtonDown) sendPointerMoved();
    });
    rootElement.addEventListener('pointerup', finishPress);
    rootElement.addEventListener('pointercancel', finishPress);
    rootElement.addEventListener('pointerenter', () => invoke('orb_hover_changed', { isHovering: true }));
    rootElement.addEventListener('pointerleave', () => invoke('orb_hover_changed', { isHovering: false }));
    window.addEventListener('contextmenu', (event) => event.preventDefault());
  });
})();
