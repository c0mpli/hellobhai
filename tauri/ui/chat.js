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

function renderMessageRow(chatMessage, previousMessage) {
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
  const replyText = createElement('div', 'reply-text', chatMessage.text);
  // A thin caret trails the text while it is still streaming.
  if (chatMessage.isStreaming) replyText.append(createElement('span', 'streaming-caret', ' ▏'));
  replyRow.append(createLogoImage('reply-logo'), replyText);
  return replyRow;
}

function renderMessages(chatMessages) {
  const messagesJson = JSON.stringify(chatMessages);
  if (messagesJson === lastRenderedMessagesJson) return;
  lastRenderedMessagesJson = messagesJson;
  messageListElement.replaceChildren();
  if (chatMessages.length === 0) {
    messageListElement.append(renderEmptyState());
    return;
  }
  chatMessages.forEach((chatMessage, messageIndex) => {
    messageListElement.append(renderMessageRow(chatMessage, chatMessages[messageIndex - 1]));
  });
  messageListElement.scrollTo({ top: messageListElement.scrollHeight, behavior: 'smooth' });
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
  element('model-label').textContent = chatState.modelLabel;
  element('replies-text-button').classList.toggle('is-selected', !chatState.speaksChatReplies);
  element('replies-voice-button').classList.toggle('is-selected', chatState.speaksChatReplies);

  renderMessages(chatState.messages);
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
document.addEventListener('keydown', (keyboardEvent) => {
  if (keyboardEvent.key === 'Escape') {
    keyboardEvent.preventDefault();
    invoke('minimize_chat');
  }
});

element('swap-side-button').addEventListener('click', () => invoke('swap_chat_side'));
element('minimize-button').addEventListener('click', () => invoke('minimize_chat'));
element('close-button').addEventListener('click', () => invoke('close_chat'));
element('minimized-tab').addEventListener('click', () => invoke('expand_chat'));
element('replies-text-button').addEventListener('click', () => invoke('set_speaks_chat_replies', { speaksChatReplies: false }));
element('replies-voice-button').addEventListener('click', () => invoke('set_speaks_chat_replies', { speaksChatReplies: true }));

listen('chat-state', (chatStateEvent) => applyChatState(chatStateEvent.payload));
// The model can change in the settings panel; pick it up when the chat comes back.
window.addEventListener('focus', refreshChatState);

resizeComposerToFitText();
refreshChatState();
