// Typed side chat, the counterpart of the Mac's ChatPanelView. Rust (chat_panel.rs)
// owns the rows and the window; this page draws the `chat-state` snapshot it pushes.
// All message text goes in as text nodes.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const SUGGESTED_PROMPTS = ["What's on my screen?", 'Explain this to me', 'Make notes of this page'];

const element = (elementId) => document.getElementById(elementId);
const messageListElement = element('message-list');
const composerInputElement = element('composer-input');
const sendButtonElement = element('send-button');

let latestChatState = null;
let lastRenderedMessagesJson = '';
let lastRenderedMessagesOnlyJson = '';
let lastScrolledToActiveChatTabId = null;

function createElement(tagName, className, textContent) {
  const createdElement = document.createElement(tagName);
  if (className) createdElement.className = className;
  if (textContent !== undefined) createdElement.textContent = textContent;
  return createdElement;
}

function createLogoImage(className) {
  const logoImage = createElement('img', `bhai-logo ${className}`);
  logoImage.src = 'logo.png';
  logoImage.alt = 'bhai';
  return logoImage;
}

function renderEmptyState() {
  const emptyState = createElement('div', 'empty-state');
  emptyState.append(
    createElement('div', 'empty-state-title', 'Ask about anything on your screen.'),
    createElement('div', 'empty-state-subtitle', 'Same as holding the hotkey and talking: it sees your screen, points, draws, and can do things for you.'),
  );
  const suggestionChips = createElement('div', 'suggestion-chips');
  for (const suggestedPrompt of SUGGESTED_PROMPTS) {
    const suggestionChip = createElement('button', 'suggestion-chip', suggestedPrompt);
    suggestionChip.type = 'button';
    suggestionChip.addEventListener('click', () => sendMessage(suggestedPrompt));
    suggestionChips.append(suggestionChip);
  }
  emptyState.append(suggestionChips);
  return emptyState;
}

/// While this reply is spoken: text already said stays normal, the sentence
/// being said is lit in the accent colour and the rest is dimmed until bhai
/// gets to it. Offsets are UTF-16, like JavaScript strings.
function createReplyTextElement(chatMessage, speechProgress) {
  const isBeingSpoken = speechProgress
    && speechProgress.messageId === chatMessage.id
    && speechProgress.sentenceEnd <= chatMessage.text.length;
  if (!isBeingSpoken) return createElement('div', 'reply-text', chatMessage.text);
  const replyText = createElement('div', 'reply-text');
  replyText.append(
    document.createTextNode(chatMessage.text.slice(0, speechProgress.sentenceStart)),
    createElement('span', 'spoken-sentence', chatMessage.text.slice(speechProgress.sentenceStart, speechProgress.sentenceEnd)),
    createElement('span', 'not-yet-spoken', chatMessage.text.slice(speechProgress.sentenceEnd)),
  );
  return replyText;
}

function renderMessageRow(chatMessage, previousMessage, speechProgress) {
  if (chatMessage.isFromUser) {
    const userRow = createElement('div', 'user-row');
    // Extra space above each new question, so a question and its answer read as one group.
    if (previousMessage && !previousMessage.isFromUser) userRow.classList.add('is-question-after-reply');
    userRow.append(createElement('div', 'user-bubble', chatMessage.text));
    return userRow;
  }
  const replyRow = createElement('div', 'reply-row');
  if (chatMessage.isStreaming && chatMessage.text === '') {
    // Before the first words arrive.
    replyRow.classList.add('is-thinking');
    const thinkingDots = createElement('div', 'thinking-dots');
    thinkingDots.append(createElement('span'), createElement('span'), createElement('span'));
    replyRow.append(createLogoImage('reply-logo'), thinkingDots);
    return replyRow;
  }
  const replyText = createReplyTextElement(chatMessage, speechProgress);
  // A thin caret trails the text while it is still streaming.
  if (chatMessage.isStreaming) replyText.append(createElement('span', 'streaming-caret', ' ▏'));
  replyRow.append(createLogoImage('reply-logo'), replyText);
  return replyRow;
}

function renderMessages(chatMessages, speechProgress) {
  const messagesJson = JSON.stringify([chatMessages, speechProgress]);
  if (messagesJson === lastRenderedMessagesJson) return;
  const onlySpeechProgressChanged = JSON.stringify(chatMessages) === lastRenderedMessagesOnlyJson;
  lastRenderedMessagesJson = messagesJson;
  lastRenderedMessagesOnlyJson = JSON.stringify(chatMessages);
  const previousScrollTop = messageListElement.scrollTop;
  messageListElement.replaceChildren();
  if (chatMessages.length === 0) {
    messageListElement.append(renderEmptyState());
    return;
  }
  chatMessages.forEach((chatMessage, messageIndex) => {
    messageListElement.append(renderMessageRow(chatMessage, chatMessages[messageIndex - 1], speechProgress));
  });
  // Following speech only recolours text, so the reader's scroll position stays.
  if (onlySpeechProgressChanged) {
    messageListElement.scrollTop = previousScrollTop;
  } else {
    messageListElement.scrollTo({ top: messageListElement.scrollHeight, behavior: 'smooth' });
  }
}

function renderTabs(chatTabs, activeChatTabId) {
  const tabStripElement = element('tab-strip');
  tabStripElement.replaceChildren(...chatTabs.map((chatTab) => {
    const isActive = chatTab.id === activeChatTabId;
    const tabElement = createElement('div', isActive ? 'chat-tab is-active' : 'chat-tab');
    tabElement.title = chatTab.title;
    tabElement.addEventListener('click', () => invoke('switch_to_chat_tab', { tabId: chatTab.id }));
    const closeButton = createElement('button', 'chat-tab-close');
    closeButton.type = 'button';
    closeButton.title = 'Close tab (Ctrl+W)';
    closeButton.setAttribute('aria-label', 'Close tab');
    closeButton.innerHTML = '<svg viewBox="0 0 10 10" width="7" height="7" aria-hidden="true"><path d="M1.5 1.5 8.5 8.5M8.5 1.5 1.5 8.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/></svg>';
    closeButton.addEventListener('click', (clickEvent) => {
      clickEvent.stopPropagation();
      invoke('close_chat_tab', { tabId: chatTab.id });
    });
    tabElement.append(createElement('span', 'chat-tab-title', chatTab.title), closeButton);
    return tabElement;
  }));
  const activeTabElement = tabStripElement.querySelector('.chat-tab.is-active');
  if (activeTabElement && activeChatTabId !== lastScrolledToActiveChatTabId) {
    lastScrolledToActiveChatTabId = activeChatTabId;
    activeTabElement.scrollIntoView({ behavior: 'smooth', inline: 'nearest', block: 'nearest' });
  }
}

function applyChatState(chatState) {
  const wasShownAndExpanded = latestChatState && latestChatState.isShown && !latestChatState.isMinimized;
  latestChatState = chatState;
  const isRightSide = chatState.chatPanelSide === 'right';
  document.body.classList.toggle('is-shown', chatState.isShown);
  document.body.classList.toggle('is-minimized', chatState.isMinimized);
  document.body.classList.toggle('has-unread-reply', chatState.hasUnreadReply);
  document.body.classList.toggle('side-left', !isRightSide);

  element('swap-side-button').title = isRightSide ? 'Move the chat to the left edge' : 'Move the chat to the right edge';
  // Chevrons point toward the edge the chat tucks into; the tab's points back out.
  element('minimize-chevrons').setAttribute('d', isRightSide ? 'M4 3l5 5-5 5M8 3l5 5-5 5' : 'M12 3 7 8l5 5M8 3 3 8l5 5');
  element('tab-chevron').setAttribute('d', isRightSide ? 'M7.5 2 3.5 6l4 4' : 'M4.5 2l4 4-4 4');
  // Codex has no model choice here, so it shows its name instead of the model menu.
  const isCodexBrain = chatState.brainEngine === 'codex';
  element('model-label').hidden = !isCodexBrain;
  element('model-label').textContent = chatState.modelLabel;
  element('model-picker').hidden = isCodexBrain;
  element('model-picker').value = chatState.claudeModel;
  element('effort-picker').value = chatState.reasoningEffort ?? '';
  element('replies-text-button').classList.toggle('is-selected', !chatState.speaksChatReplies);
  element('replies-voice-button').classList.toggle('is-selected', chatState.speaksChatReplies);

  const chatTextSize = `${chatState.chatTextPointSize}px`;
  if (document.documentElement.style.getPropertyValue('--chat-text-size') !== chatTextSize) {
    document.documentElement.style.setProperty('--chat-text-size', chatTextSize);
    resizeComposerToFitText();
  }
  renderTabs(chatState.tabs, chatState.activeChatTabId);
  renderMessages(chatState.messages, chatState.speechProgress);
  if (chatState.isShown && !chatState.isMinimized && !wasShownAndExpanded) composerInputElement.focus();
}

async function refreshChatState() {
  applyChatState(await invoke('get_chat_state'));
}

function resizeComposerToFitText() {
  composerInputElement.style.height = 'auto';
  composerInputElement.style.height = `${composerInputElement.scrollHeight}px`;
  sendButtonElement.disabled = composerInputElement.value.trim() === '';
}

function sendMessage(messageText) {
  const trimmedMessage = messageText.trim();
  if (!trimmedMessage) return;
  composerInputElement.value = '';
  resizeComposerToFitText();
  invoke('send_chat_message', { text: trimmedMessage });
}

// Return sends; shift+return adds a line. Not while an IME is composing.
composerInputElement.addEventListener('keydown', (keyboardEvent) => {
  if (keyboardEvent.key === 'Enter' && !keyboardEvent.shiftKey && !keyboardEvent.isComposing) {
    keyboardEvent.preventDefault();
    sendMessage(composerInputElement.value);
  }
});
composerInputElement.addEventListener('input', resizeComposerToFitText);
composerInputElement.addEventListener('focus', () => element('composer-box').classList.add('is-focused'));
composerInputElement.addEventListener('blur', () => element('composer-box').classList.remove('is-focused'));
sendButtonElement.addEventListener('click', () => sendMessage(composerInputElement.value));

// Esc minimizes, from anywhere in the page (the Mac catches it in the text field too).
// Ctrl+T opens a new chat (nothing on an empty one) and Ctrl+W closes the current tab.
document.addEventListener('keydown', (keyboardEvent) => {
  if (keyboardEvent.key === 'Escape') {
    keyboardEvent.preventDefault();
    invoke('minimize_chat');
    return;
  }
  const hasOnlyCtrl = keyboardEvent.ctrlKey && !keyboardEvent.altKey && !keyboardEvent.shiftKey && !keyboardEvent.metaKey;
  if (!hasOnlyCtrl) return;
  const pressedKey = keyboardEvent.key.toLowerCase();
  if (pressedKey === 't') {
    keyboardEvent.preventDefault();
    invoke('open_new_chat_tab');
  } else if (pressedKey === 'w' && latestChatState) {
    keyboardEvent.preventDefault();
    invoke('close_chat_tab', { tabId: latestChatState.activeChatTabId });
  }
});

element('new-chat-button').addEventListener('click', () => invoke('open_new_chat_tab'));
element('swap-side-button').addEventListener('click', () => invoke('swap_chat_side'));
element('minimize-button').addEventListener('click', () => invoke('minimize_chat'));
element('close-button').addEventListener('click', () => invoke('close_chat'));
element('minimized-tab').addEventListener('click', () => invoke('expand_chat'));
function changeSetting(settingName, settingValue) {
  return invoke('change_setting', { change: { name: settingName, value: settingValue } });
}
element('model-picker').addEventListener('change', (event) => changeSetting('claudeModel', event.target.value));
element('effort-picker').addEventListener('change', (event) => changeSetting('reasoningEffort', event.target.value || null));
element('replies-text-button').addEventListener('click', () => invoke('set_speaks_chat_replies', { speaksChatReplies: false }));
element('replies-voice-button').addEventListener('click', () => invoke('set_speaks_chat_replies', { speaksChatReplies: true }));

listen('chat-state', (chatStateEvent) => applyChatState(chatStateEvent.payload));
// The model can change in the settings panel; pick it up when the chat comes back.
window.addEventListener('focus', refreshChatState);

resizeComposerToFitText();
refreshChatState();
