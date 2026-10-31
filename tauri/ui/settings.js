// Tray settings panel, the Windows counterpart of the Mac's CompanionPanelView.
// Commands and events are the Rust <-> page contract in the plan (section 7).
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow, LogicalSize } = window.__TAURI__.window;

const CLAUDE_MODEL_OPTIONS = [
  { alias: 'fable', label: 'Fable' },
  { alias: 'opus', label: 'Opus' },
  { alias: 'sonnet', label: 'Sonnet' },
  { alias: 'haiku', label: 'Haiku' },
];
const SUPERTONIC_VOICES = ['F1', 'F2', 'F3', 'F4', 'F5', 'M1', 'M2', 'M3', 'M4', 'M5'];
// Same order and wording as PushToTalkShortcutOption::display_text in push_to_talk.rs.
const PUSH_TO_TALK_SHORTCUT_OPTIONS = [
  { option: 'ctrlWin', label: 'ctrl + win' },
  { option: 'ctrlLeftAlt', label: 'ctrl + left alt' },
  { option: 'shiftCtrl', label: 'shift + ctrl' },
  { option: 'ctrlAltSpace', label: 'ctrl + alt + space' },
  { option: 'shiftCtrlSpace', label: 'shift + ctrl + space' },
];
const VOICE_STATE_HEADER_TEXT = { idle: 'Active', listening: 'Listening', processing: 'Processing', responding: 'Responding' };
// The status snapshot has no push from every part yet, so it is refreshed while the panel shows.
const STATUS_REFRESH_INTERVAL_MILLISECONDS = 1000;

const element = (elementId) => document.getElementById(elementId);

/// "F1" -> "Female 1", "M3" -> "Male 3", like the Mac panel.
function supertonicVoiceDisplayName(supertonicVoice) {
  return `${supertonicVoice.startsWith('M') ? 'Male' : 'Female'} ${supertonicVoice.slice(1)}`;
}

function setStatusLabel(statusLabelElement, statusText, statusKind) {
  statusLabelElement.textContent = statusText;
  statusLabelElement.classList.remove('is-good', 'is-busy', 'is-warning', 'is-bad');
  if (statusKind) statusLabelElement.classList.add(`is-${statusKind}`);
}

function statusKindForProgressText(progressText) {
  if (progressText === 'ready' || progressText === 'Supertonic-3') return 'good';
  if (progressText.startsWith('failed') || progressText.includes('failed')) return 'bad';
  if (progressText === 'system voice') return 'warning';
  return 'busy';
}

function fillSelect(selectElement, options, selectedValue) {
  selectElement.replaceChildren(...options.map(({ value, label }) => {
    const optionElement = document.createElement('option');
    optionElement.value = value;
    optionElement.textContent = label;
    return optionElement;
  }));
  selectElement.value = selectedValue;
}

function renderKeyCapsules(keyCapsuleLabels) {
  element('hint-key-capsules').replaceChildren(...keyCapsuleLabels.map((keyLabel) => {
    const keyCapsuleElement = document.createElement('span');
    keyCapsuleElement.className = 'key-capsule';
    keyCapsuleElement.textContent = keyLabel;
    return keyCapsuleElement;
  }));
}

function renderClaudeModelPicker(selectedClaudeModel) {
  element('claude-model-picker').replaceChildren(...CLAUDE_MODEL_OPTIONS.map(({ alias, label }) => {
    const segmentButton = document.createElement('button');
    segmentButton.type = 'button';
    segmentButton.className = 'segment-button' + (alias === selectedClaudeModel ? ' is-selected' : '');
    segmentButton.textContent = label;
    segmentButton.setAttribute('role', 'radio');
    segmentButton.setAttribute('aria-checked', String(alias === selectedClaudeModel));
    segmentButton.addEventListener('click', async () => {
      await invoke('set_claude_model', { model: alias });
      renderClaudeModelPicker(alias);
    });
    return segmentButton;
  }));
}

function renderSettings(settingsSnapshot) {
  renderKeyCapsules(settingsSnapshot.pushToTalkKeyCapsuleLabels);
  renderClaudeModelPicker(settingsSnapshot.claudeModel);
  fillSelect(
    element('voice-picker'),
    SUPERTONIC_VOICES.map((supertonicVoice) => ({ value: supertonicVoice, label: supertonicVoiceDisplayName(supertonicVoice) })),
    settingsSnapshot.supertonicVoice,
  );
  fillSelect(
    element('shortcut-picker'),
    PUSH_TO_TALK_SHORTCUT_OPTIONS.map(({ option, label }) => ({ value: option, label })),
    settingsSnapshot.pushToTalkShortcut,
  );
  element('launch-at-login-toggle').checked = settingsSnapshot.launchAtLogin;
}

let latestVoiceState = 'idle';
let latestStatusSnapshot = null;

function renderHeaderStatus() {
  const statusDotElement = element('status-dot');
  statusDotElement.classList.remove('is-busy', 'is-warning');
  const needsSetup = latestStatusSnapshot && (!latestStatusSnapshot.claudeFound || latestStatusSnapshot.microphoneWorking === false);
  if (needsSetup) {
    element('status-line').textContent = 'Setup';
    statusDotElement.classList.add('is-warning');
    return;
  }
  element('status-line').textContent = VOICE_STATE_HEADER_TEXT[latestVoiceState] ?? 'Active';
  if (latestVoiceState !== 'idle') statusDotElement.classList.add('is-busy');
}

function renderStatus(statusSnapshot) {
  latestStatusSnapshot = statusSnapshot;
  setStatusLabel(
    element('claude-status'),
    statusSnapshot.claudeFound ? 'Found' : 'Not found: install Claude Code',
    statusSnapshot.claudeFound ? 'good' : 'bad',
  );
  setStatusLabel(element('voice-status'), statusSnapshot.voice, statusKindForProgressText(statusSnapshot.voice));
  const speechModelText = statusSnapshot.speechModel === 'ready' ? 'Whisper ready' : `Whisper ${statusSnapshot.speechModel}`;
  setStatusLabel(element('speech-model-status'), speechModelText, statusKindForProgressText(statusSnapshot.speechModel));
  if (statusSnapshot.microphoneWorking === null || statusSnapshot.microphoneWorking === undefined) {
    setStatusLabel(element('microphone-status'), 'Not tried yet', null);
  } else {
    setStatusLabel(element('microphone-status'), statusSnapshot.microphoneWorking ? 'Working' : 'Blocked', statusSnapshot.microphoneWorking ? 'good' : 'bad');
  }
  element('version-label').textContent = `v${statusSnapshot.version}`;
  renderHeaderStatus();
  fitWindowToPanel();
}

/// The window is created at a guessed height; size it to the panel so no
/// invisible part of it blocks clicks on the desktop.
let lastFittedPanelHeight = 0;
function fitWindowToPanel() {
  const panelHeight = Math.ceil(element('settings-panel').getBoundingClientRect().height);
  if (panelHeight === lastFittedPanelHeight) return;
  lastFittedPanelHeight = panelHeight;
  getCurrentWindow().setSize(new LogicalSize(340, panelHeight)).catch(() => {});
}

async function refreshStatus() {
  try { renderStatus(await invoke('get_status')); } catch (statusError) { /* app is shutting down */ }
}

async function startSettingsPanel() {
  renderSettings(await invoke('get_settings'));
  await refreshStatus();

  await listen('status-changed', (event) => renderStatus(event.payload));
  await listen('buddy-voice-state', (event) => { latestVoiceState = event.payload.state; renderHeaderStatus(); });

  // Settings can change elsewhere (first-launch autostart), so re-read them each time the panel opens.
  document.addEventListener('visibilitychange', async () => {
    if (document.visibilityState === 'visible') {
      renderSettings(await invoke('get_settings'));
      refreshStatus();
    }
  });
  setInterval(() => { if (document.visibilityState === 'visible') refreshStatus(); }, STATUS_REFRESH_INTERVAL_MILLISECONDS);

  element('voice-picker').addEventListener('change', (event) => invoke('set_supertonic_voice', { voice: event.target.value }));
  element('preview-voice-button').addEventListener('click', () => invoke('preview_voice', { voice: element('voice-picker').value }));
  element('shortcut-picker').addEventListener('change', async (event) => {
    await invoke('set_push_to_talk_shortcut', { option: event.target.value });
    renderSettings(await invoke('get_settings'));
  });
  element('launch-at-login-toggle').addEventListener('change', async (event) => {
    await invoke('set_launch_at_login', { enabled: event.target.checked });
    renderSettings(await invoke('get_settings'));
  });
  element('open-microphone-settings-button').addEventListener('click', () => invoke('open_microphone_settings'));
  element('replay-onboarding-button').addEventListener('click', () => invoke('replay_onboarding'));
  element('quit-button').addEventListener('click', () => invoke('quit_app'));
  element('close-button').addEventListener('click', () => getCurrentWindow().hide());
  document.addEventListener('keydown', (event) => { if (event.key === 'Escape') getCurrentWindow().hide(); });
}

startSettingsPanel();
