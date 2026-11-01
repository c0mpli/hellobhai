// Tray settings panel, the Windows/Linux counterpart of the Mac's CompanionPanelView.
// Every change goes through `change_setting {name, value}`; Rust saves it and sends
// `settings-changed` to every window, which re-renders this page too.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow, LogicalSize } = window.__TAURI__.window;

const SETTINGS_TABS = [
  { optionID: 'general', label: 'General' },
  { optionID: 'voice', label: 'Voice' },
  { optionID: 'chat', label: 'Chat' },
  { optionID: 'buddy', label: 'Buddy' },
];
const CLAUDE_MODEL_OPTIONS = [
  { optionID: 'fable', label: 'Fable' },
  { optionID: 'opus', label: 'Opus' },
  { optionID: 'sonnet', label: 'Sonnet' },
  { optionID: 'haiku', label: 'Haiku' },
];
// "" means the model's own default effort (no override passed).
const REASONING_EFFORT_OPTIONS = [
  { value: '', label: 'Default' },
  { value: 'low', label: 'Low' },
  { value: 'medium', label: 'Medium' },
  { value: 'high', label: 'High' },
];
const SUPERTONIC_VOICES = ['F1', 'F2', 'F3', 'F4', 'F5', 'M1', 'M2', 'M3', 'M4', 'M5'];
// Same names, order and swatch colours as the Mac's OrbColorVariant; the value is orb.riv's color input.
const ORB_COLOR_VARIANTS = [
  { colorInputValue: 0, displayName: 'Rainbow', swatchGradientHexColors: ['#FFE03C', '#FF3B5C'] },
  { colorInputValue: 1, displayName: 'Candy', swatchGradientHexColors: ['#FF4FD8', '#3FD0F5'] },
  { colorInputValue: 2, displayName: 'Red', swatchGradientHexColors: ['#FF8A80', '#E8322E'] },
  { colorInputValue: 3, displayName: 'Coral', swatchGradientHexColors: ['#FF7A8A', '#FFC93C'] },
  { colorInputValue: 4, displayName: 'Sunset', swatchGradientHexColors: ['#FFE14D', '#F08A24'] },
  { colorInputValue: 5, displayName: 'Green', swatchGradientHexColors: ['#7DF59A', '#1DB954'] },
  { colorInputValue: 6, displayName: 'Teal', swatchGradientHexColors: ['#7FFFF2', '#12BFAE'] },
  { colorInputValue: 7, displayName: 'Blue', swatchGradientHexColors: ['#4FC3F7', '#1D4ED8'] },
  { colorInputValue: 8, displayName: 'Aurora', swatchGradientHexColors: ['#3FD5F5', '#7A4DFF'] },
  { colorInputValue: 9, displayName: 'Violet', swatchGradientHexColors: ['#E07BFF', '#6A4DFF'] },
];
const VOICE_STATE_HEADER_TEXT = { idle: 'Active', listening: 'Listening', processing: 'Processing', responding: 'Responding' };
// The status snapshot has no push from every part yet, so it is refreshed while the panel shows.
const STATUS_REFRESH_INTERVAL_MILLISECONDS = 1000;
const SELECTED_TAB_STORAGE_KEY = 'selectedPanelSettingsTab';
const PLAY_ICON_SVG = '<svg viewBox="0 0 10 10" width="8" height="8" aria-hidden="true"><path d="M2 1 9 5 2 9Z" fill="currentColor"/></svg>';
const STOP_ICON_SVG = '<svg viewBox="0 0 10 10" width="8" height="8" aria-hidden="true"><rect x="2" y="2" width="6" height="6" rx="1" fill="currentColor"/></svg>';

const element = (elementId) => document.getElementById(elementId);

let latestSettingsSnapshot = null;
let latestStatusSnapshot = null;
let latestVoiceState = 'idle';
let isRecordingShortcut = false;
let shortcutRecorderHeldKeyLabels = [];
let shortcutRecorderHint = null;
// The recorder's Esc also reaches this page as a keydown; it must not hide the panel.
let lastShortcutRecordingStoppedAtMilliseconds = 0;

function changeSetting(settingName, settingValue) {
  return invoke('change_setting', { change: { name: settingName, value: settingValue } });
}

/// Remembered per viewer only; the panel works the same if storage is unavailable.
function readSelectedTab() {
  try { return localStorage.getItem(SELECTED_TAB_STORAGE_KEY) || 'general'; } catch (storageError) { return 'general'; }
}
let selectedTab = readSelectedTab();

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
  if (progressText.includes('failed')) return 'bad';
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

function keyCapsuleElements(keyLabels) {
  return keyLabels.map((keyLabel) => {
    const keyCapsuleElement = document.createElement('span');
    keyCapsuleElement.className = 'key-capsule';
    keyCapsuleElement.textContent = keyLabel;
    return keyCapsuleElement;
  });
}

/// A row of segment buttons; the selected one is blue, like PanelSegmentedControl.
function renderSegmentedControl(containerElement, options, selectedOptionID, onSelectOption) {
  containerElement.replaceChildren(...options.map(({ optionID, label }) => {
    const isSelected = optionID === selectedOptionID;
    const segmentButton = document.createElement('button');
    segmentButton.type = 'button';
    segmentButton.className = 'segment-button' + (isSelected ? ' is-selected' : '');
    segmentButton.textContent = label;
    segmentButton.setAttribute('role', 'radio');
    segmentButton.setAttribute('aria-checked', String(isSelected));
    segmentButton.addEventListener('click', () => onSelectOption(optionID));
    return segmentButton;
  }));
}

function renderTabs() {
  renderSegmentedControl(element('settings-tab-picker'), SETTINGS_TABS, selectedTab, (tabID) => {
    selectedTab = tabID;
    try { localStorage.setItem(SELECTED_TAB_STORAGE_KEY, tabID); } catch (storageError) { /* not remembered */ }
    renderTabs();
  });
  for (const tabPageElement of document.querySelectorAll('.tab-page')) {
    tabPageElement.hidden = tabPageElement.dataset.tab !== selectedTab;
  }
  fitWindowToPanel();
}

function renderIntro(settingsSnapshot) {
  const hasCompletedOnboarding = settingsSnapshot.hasCompletedOnboarding;
  element('push-to-talk-hint').hidden = !hasCompletedOnboarding;
  element('welcome-intro').hidden = hasCompletedOnboarding;
  element('hint-verb').textContent = settingsSnapshot.isChatInputMode ? 'Press' : 'Hold';
  element('hint-ending').textContent = settingsSnapshot.isChatInputMode ? 'to chat.' : 'to talk.';
  element('hint-key-capsules').replaceChildren(...keyCapsuleElements(settingsSnapshot.pushToTalkKeyCapsuleLabels));
}

function renderShortcutRecorder() {
  if (!latestSettingsSnapshot) return;
  const recorderButton = element('shortcut-recorder-button');
  recorderButton.classList.toggle('is-recording', isRecordingShortcut);
  if (isRecordingShortcut && shortcutRecorderHeldKeyLabels.length === 0) {
    const hintElement = document.createElement('span');
    hintElement.className = 'recorder-hint';
    hintElement.textContent = shortcutRecorderHint ?? 'Press keys...';
    recorderButton.replaceChildren(hintElement);
  } else {
    const keyLabels = isRecordingShortcut ? shortcutRecorderHeldKeyLabels : latestSettingsSnapshot.pushToTalkKeyCapsuleLabels;
    const keyCapsulesElement = document.createElement('span');
    keyCapsulesElement.className = 'key-capsules';
    keyCapsulesElement.replaceChildren(...keyCapsuleElements(keyLabels));
    recorderButton.replaceChildren(keyCapsulesElement);
  }
  const resetButton = element('reset-shortcut-button');
  resetButton.hidden = latestSettingsSnapshot.isDefaultPushToTalkShortcut || isRecordingShortcut;
  const resetTitle = `Back to ${latestSettingsSnapshot.defaultPushToTalkDisplayText}`;
  resetButton.title = resetTitle;
  resetButton.setAttribute('aria-label', resetTitle);
  fitWindowToPanel();
}

function renderModelSection(settingsSnapshot) {
  const isCodexBrain = settingsSnapshot.brainEngine === 'codex';
  renderSegmentedControl(
    element('brain-engine-picker'),
    [{ optionID: 'claude', label: 'Claude' }, { optionID: 'codex', label: 'Codex' }],
    settingsSnapshot.brainEngine,
    async (brainEngine) => { await changeSetting('brainEngine', brainEngine); refreshStatus(); },
  );
  element('claude-model-row').hidden = isCodexBrain;
  renderSegmentedControl(element('claude-model-picker'), CLAUDE_MODEL_OPTIONS, settingsSnapshot.claudeModel, (modelAlias) => changeSetting('claudeModel', modelAlias));
  fillSelect(element('effort-picker'), REASONING_EFFORT_OPTIONS, settingsSnapshot.reasoningEffort ?? '');
}

function renderBuddySection(settingsSnapshot) {
  const isOrbStyle = settingsSnapshot.voiceBuddyStyle === 'orb';
  renderSegmentedControl(
    element('voice-style-picker'),
    [{ optionID: 'orb', label: 'Orb' }, { optionID: 'cursor', label: 'Cursor' }],
    settingsSnapshot.voiceBuddyStyle,
    (voiceBuddyStyle) => changeSetting('voiceBuddyStyle', voiceBuddyStyle),
  );
  element('voice-style-subtitle').textContent = isOrbStyle ? 'Pointer flies out of the orb' : 'Pointer stays next to the mouse';

  const orbAppearanceRowsElement = element('orb-appearance-rows');
  orbAppearanceRowsElement.classList.toggle('is-disabled', !isOrbStyle);
  element('orb-size-slider').disabled = !isOrbStyle;
  element('orb-color-picker').disabled = !isOrbStyle;
  element('orb-style-footnote').hidden = isOrbStyle;

  // Don't fight the thumb while it is being dragged.
  if (document.activeElement !== element('orb-size-slider')) {
    element('orb-size-slider').value = String(settingsSnapshot.orbSideLength);
  }
  element('orb-size-value').textContent = `${Math.round(settingsSnapshot.orbSideLength)} px`;

  fillSelect(
    element('orb-color-picker'),
    ORB_COLOR_VARIANTS.map(({ colorInputValue, displayName }) => ({ value: String(colorInputValue), label: displayName })),
    String(settingsSnapshot.selectedOrbColor),
  );
  const selectedColorVariant = ORB_COLOR_VARIANTS.find((variant) => variant.colorInputValue === settingsSnapshot.selectedOrbColor) ?? ORB_COLOR_VARIANTS[8];
  element('orb-color-swatch').style.background = `linear-gradient(135deg, ${selectedColorVariant.swatchGradientHexColors.join(', ')})`;
}

function renderSettings(settingsSnapshot) {
  latestSettingsSnapshot = settingsSnapshot;
  renderIntro(settingsSnapshot);

  renderSegmentedControl(
    element('bhai-mode-picker'),
    [{ optionID: 'voice', label: 'Voice' }, { optionID: 'chat', label: 'Chat' }],
    settingsSnapshot.isChatInputMode ? 'chat' : 'voice',
    (bhaiMode) => changeSetting('isChatInputMode', bhaiMode === 'chat'),
  );
  element('bhai-mode-label').textContent = settingsSnapshot.isChatInputMode ? 'Chat' : 'Voice';
  element('bhai-mode-subtitle').textContent = settingsSnapshot.isChatInputMode ? 'Shortcut and orb open the chat' : 'Hold the shortcut and talk';

  renderModelSection(settingsSnapshot);
  renderShortcutRecorder();
  element('launch-at-login-toggle').checked = settingsSnapshot.launchAtLogin;
  element('replay-onboarding-button').hidden = !settingsSnapshot.hasCompletedOnboarding;

  fillSelect(
    element('voice-picker'),
    SUPERTONIC_VOICES.map((supertonicVoice) => ({ value: supertonicVoice, label: supertonicVoiceDisplayName(supertonicVoice) })),
    settingsSnapshot.supertonicVoice,
  );

  renderSegmentedControl(
    element('voice-reply-picker'),
    [{ optionID: 'voice', label: 'Voice + text' }, { optionID: 'text', label: 'Text' }],
    settingsSnapshot.speaksVoiceReplies ? 'voice' : 'text',
    (replyKind) => changeSetting('speaksVoiceReplies', replyKind === 'voice'),
  );
  renderSegmentedControl(
    element('chat-reply-picker'),
    [{ optionID: 'text', label: 'Text' }, { optionID: 'voice', label: 'Text + voice' }],
    settingsSnapshot.speaksChatReplies ? 'voice' : 'text',
    (replyKind) => changeSetting('speaksChatReplies', replyKind === 'voice'),
  );
  renderSegmentedControl(
    element('chat-side-picker'),
    [{ optionID: 'left', label: 'Left' }, { optionID: 'right', label: 'Right' }],
    settingsSnapshot.chatPanelSide,
    (chatPanelSide) => changeSetting('chatPanelSide', chatPanelSide),
  );

  renderBuddySection(settingsSnapshot);
  renderHeaderStatus();
  fitWindowToPanel();
}

function renderHeaderStatus() {
  const needsSetup = (latestSettingsSnapshot && !latestSettingsSnapshot.hasCompletedOnboarding)
    || (latestStatusSnapshot && (!latestStatusSnapshot.brainCliFound || latestStatusSnapshot.microphoneWorking === false));
  element('status-line').textContent = needsSetup ? 'Setup' : (VOICE_STATE_HEADER_TEXT[latestVoiceState] ?? 'Active');
}

function brainCliStatus(statusSnapshot) {
  const isCodexBrain = latestSettingsSnapshot?.brainEngine === 'codex';
  if (statusSnapshot.brainCliFound) return { text: isCodexBrain ? 'Codex CLI found' : 'Claude Code found', kind: 'good' };
  return {
    text: isCodexBrain
      ? 'Codex CLI not found: install it, then sign in with codex login'
      : 'Claude CLI not found: install Claude Code, then run claude',
    kind: 'warning',
  };
}

function renderStatus(statusSnapshot) {
  latestStatusSnapshot = statusSnapshot;
  const cliStatus = brainCliStatus(statusSnapshot);
  setStatusLabel(element('brain-cli-status'), cliStatus.text, cliStatus.kind);
  setStatusLabel(element('voice-status'), statusSnapshot.voice, statusKindForProgressText(statusSnapshot.voice));
  const speechModelText = statusSnapshot.speechModel === 'ready' ? 'Whisper ready' : `Whisper ${statusSnapshot.speechModel}`;
  setStatusLabel(element('speech-model-status'), speechModelText, statusKindForProgressText(statusSnapshot.speechModel));
  if (statusSnapshot.microphoneWorking === null || statusSnapshot.microphoneWorking === undefined) {
    setStatusLabel(element('microphone-status'), 'Not tried yet', null);
  } else {
    setStatusLabel(element('microphone-status'), statusSnapshot.microphoneWorking ? 'Working' : 'Blocked', statusSnapshot.microphoneWorking ? 'good' : 'bad');
  }
  const previewButton = element('preview-voice-button');
  previewButton.innerHTML = statusSnapshot.isSpeaking ? STOP_ICON_SVG : PLAY_ICON_SVG;
  const previewTitle = statusSnapshot.isSpeaking ? 'Stop' : 'Preview voice';
  previewButton.title = previewTitle;
  previewButton.setAttribute('aria-label', previewTitle);
  const platformNoteElement = element('platform-note');
  platformNoteElement.hidden = !statusSnapshot.platformLimitationNote;
  platformNoteElement.textContent = statusSnapshot.platformLimitationNote ?? '';
  element('version-label').textContent = `v${statusSnapshot.version}`;
  renderHeaderStatus();
  fitWindowToPanel();
}

/// The window is created at a guessed height; size it to the panel so no
/// invisible part of it blocks clicks on the desktop.
let lastFittedPanelHeight = 0;
function fitWindowToPanel() {
  // Never taller than the usable screen (taskbar excluded) minus the corner margin.
  const panelMaximumHeight = Math.max(240, window.screen.availHeight - 24);
  document.documentElement.style.setProperty('--panel-max-height', panelMaximumHeight + 'px');
  const panelHeight = Math.ceil(element('settings-panel').getBoundingClientRect().height);
  if (panelHeight === lastFittedPanelHeight) return;
  lastFittedPanelHeight = panelHeight;
  getCurrentWindow().setSize(new LogicalSize(340, panelHeight)).catch(() => {});
}

async function refreshStatus() {
  try { renderStatus(await invoke('get_status')); } catch (statusError) { /* app is shutting down */ }
}

function startShortcutRecording() {
  isRecordingShortcut = true;
  shortcutRecorderHeldKeyLabels = [];
  shortcutRecorderHint = null;
  invoke('set_shortcut_recording', { isRecording: true });
  renderShortcutRecorder();
}

function stopShortcutRecording() {
  if (!isRecordingShortcut) return;
  isRecordingShortcut = false;
  lastShortcutRecordingStoppedAtMilliseconds = Date.now();
  invoke('set_shortcut_recording', { isRecording: false });
  renderShortcutRecorder();
}

function handleShortcutRecorderEvent(recorderEvent) {
  if (!isRecordingShortcut) return;
  if (recorderEvent.state === 'holding') {
    shortcutRecorderHeldKeyLabels = recorderEvent.keyLabels;
  } else if (recorderEvent.state === 'needsTwoOrMoreKeys') {
    shortcutRecorderHeldKeyLabels = [];
    shortcutRecorderHint = 'Use 2+ keys';
  } else {
    // Recorded (settings-changed brings the new keys) or Esc.
    isRecordingShortcut = false;
    lastShortcutRecordingStoppedAtMilliseconds = Date.now();
  }
  renderShortcutRecorder();
}

async function startSettingsPanel() {
  renderTabs();
  renderSettings(await invoke('get_settings'));
  await refreshStatus();

  await listen('settings-changed', (event) => renderSettings(event.payload));
  await listen('status-changed', (event) => renderStatus(event.payload));
  await listen('buddy-voice-state', (event) => { latestVoiceState = event.payload.state; renderHeaderStatus(); });
  await listen('speaking-changed', () => refreshStatus());
  await listen('shortcut-recorder', (event) => handleShortcutRecorderEvent(event.payload));

  // Settings can change elsewhere (first-launch autostart), so re-read them each time the panel opens.
  document.addEventListener('visibilitychange', async () => {
    if (document.visibilityState === 'visible') {
      renderSettings(await invoke('get_settings'));
      refreshStatus();
    } else {
      stopShortcutRecording();
    }
  });
  setInterval(() => { if (document.visibilityState === 'visible') refreshStatus(); }, STATUS_REFRESH_INTERVAL_MILLISECONDS);

  element('effort-picker').addEventListener('change', (event) => changeSetting('reasoningEffort', event.target.value || null));
  element('voice-picker').addEventListener('change', (event) => changeSetting('supertonicVoice', event.target.value));
  element('preview-voice-button').addEventListener('click', async () => {
    await invoke('preview_voice', { voice: element('voice-picker').value });
    refreshStatus();
  });
  element('orb-size-slider').addEventListener('input', (event) => {
    element('orb-size-value').textContent = `${event.target.value} px`;
    changeSetting('orbSideLength', Number(event.target.value));
  });
  element('orb-color-picker').addEventListener('change', (event) => changeSetting('selectedOrbColor', Number(event.target.value)));
  element('shortcut-recorder-button').addEventListener('click', () => (isRecordingShortcut ? stopShortcutRecording() : startShortcutRecording()));
  element('reset-shortcut-button').addEventListener('click', () => changeSetting('pushToTalkShortcut', latestSettingsSnapshot.defaultPushToTalkShortcut));
  element('launch-at-login-toggle').addEventListener('change', async (event) => {
    await invoke('set_launch_at_login', { enabled: event.target.checked });
    renderSettings(await invoke('get_settings'));
  });
  element('open-microphone-settings-button').addEventListener('click', () => invoke('open_microphone_settings'));
  element('replay-onboarding-button').addEventListener('click', () => invoke('replay_onboarding'));
  element('start-button').addEventListener('click', () => invoke('replay_onboarding'));
  element('quit-button').addEventListener('click', () => invoke('quit_app'));
  element('close-button').addEventListener('click', () => { stopShortcutRecording(); getCurrentWindow().hide(); });
  document.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape') return;
    const wasEscapeForTheRecorder = isRecordingShortcut || Date.now() - lastShortcutRecordingStoppedAtMilliseconds < 400;
    if (wasEscapeForTheRecorder) {
      stopShortcutRecording();
      return;
    }
    getCurrentWindow().hide();
  });
}

startSettingsPanel();
