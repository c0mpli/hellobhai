// Injected into the generated board, notes and 3D pages (exported from the Mac
// app). Those pages post to window.webkit.messageHandlers as on the Mac; this
// forwards each message to the Rust `page_message` command so they run unmodified.
(() => {
  const handlerForChannel = (channel) => ({
    postMessage: (message) => window.__TAURI_INTERNALS__.invoke('page_message', { channel, message }),
  });
  window.webkit = { messageHandlers: { pen: handlerForChannel('pen'), model3D: handlerForChannel('model3D') } };
})();
