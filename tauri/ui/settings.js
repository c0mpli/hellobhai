// Tray settings panel. Commands and events are the Rust <-> page contract in the plan (section 7).
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

function renderSettings(settingsSnapshot) {
  // TODO(D): model picker (fable/opus/sonnet/haiku), voice picker F1-M5 with previews,
  // shortcut picker, launch at login, push-to-talk key capsules.
}

function renderStatus(statusSnapshot) {
  // TODO(D): status line, speech model + voice status, microphone status, version.
}

async function startSettingsPanel() {
  renderSettings(await invoke('get_settings'));
  renderStatus(await invoke('get_status'));
  await listen('status-changed', (event) => renderStatus(event.payload));
  await listen('buddy-voice-state', (event) => { /* { state } */ });
  await listen('speaking-changed', (event) => { /* { isSpeaking } */ });

  document.getElementById('open-microphone-settings-button').addEventListener('click', () => invoke('open_microphone_settings'));
  document.getElementById('replay-onboarding-button').addEventListener('click', () => invoke('replay_onboarding'));
  document.getElementById('quit-button').addEventListener('click', () => invoke('quit_app'));
  document.getElementById('launch-at-login-toggle').addEventListener('change', (event) =>
    invoke('set_launch_at_login', { enabled: event.target.checked }));
}

startSettingsPanel();
