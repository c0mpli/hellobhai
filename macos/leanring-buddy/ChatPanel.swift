//
//  ChatPanel.swift
//  leanring-buddy
//
//  Typed chat with hellobhai: a floating card that slides in from the
//  right (or left) edge of the screen, like Cursor's or Google Meet's side
//  chat. Typed messages go through the same pipeline as voice, and voice
//  turns show up here too, so it reads as one conversation.
//

import AppKit
import Combine
import SwiftUI

/// Non-activating, so the app the user is working in stays frontmost (its
/// screenshot, and where clicks and typing go), but still able to become
/// key so the text field takes typing.
private final class ChatKeyablePanel: NSPanel {
    override var canBecomeKey: Bool { true }

    var onEscapePressed: (() -> Void)?

    override func cancelOperation(_ sender: Any?) {
        onEscapePressed?()
    }
}

/// The bhai logo (the app icon: orb sending out a pointer), used where a
/// status dot used to be in the panel header and in the chat.
struct BhaiLogoView: View {
    let sideLength: CGFloat

    var body: some View {
        Image(nsImage: NSApplication.shared.applicationIconImage)
            .resizable()
            .interpolation(.high)
            .frame(width: sideLength, height: sideLength)
            .accessibilityLabel("bhai")
    }
}

enum ChatPanelSide: String {
    case left
    case right
}

@MainActor
final class ChatPanelManager: ObservableObject {
    private static let panelWidth: CGFloat = 380
    private static let gapFromScreenEdges: CGFloat = 12
    private static let minimizedTabSize = NSSize(width: 30, height: 112)

    /// Minimized, the chat shrinks to a small tab sticking out of the screen
    /// edge; clicking the tab brings the full chat back.
    @Published private(set) var isMinimized = false
    /// A reply arrived while minimized, so the tab shows a dot.
    @Published private(set) var hasUnreadReply = false

    private unowned let companionManager: CompanionManager
    private var panel: ChatKeyablePanel?
    private var currentScreen: NSScreen = NSScreen.main ?? NSScreen.screens[0]
    private var chatPanelSideCancellable: AnyCancellable?
    private var chatMessagesCancellable: AnyCancellable?
    /// Set while the close animation runs, so a show() during it cancels
    /// the pending orderOut.
    private var isClosingPanel = false

    /// Called with the chat's window number when it appears, and nil when
    /// it closes, so the orb can sit under it meanwhile.
    var onChatWindowShownOrHidden: ((Int?) -> Void)?

    private var panelSide: ChatPanelSide { companionManager.chatPanelSide }

    init(companionManager: CompanionManager) {
        self.companionManager = companionManager
        // The side can be switched from the chat header or the menu bar panel.
        chatPanelSideCancellable = companionManager.$chatPanelSide
            .dropFirst()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in self?.moveToCurrentSide() }
        chatMessagesCancellable = companionManager.$chatMessages
            .dropFirst()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] chatMessages in
                // The empty placeholder row added while a reply is on its way doesn't count.
                guard let self, self.isMinimized, let lastMessage = chatMessages.last,
                      !lastMessage.isFromUser, !lastMessage.text.isEmpty else { return }
                self.hasUnreadReply = true
            }
    }

    var isVisible: Bool { panel?.isVisible ?? false }

    var isOpenAndFocused: Bool {
        guard let panel else { return false }
        return panel.isVisible && panel.isKeyWindow && !isMinimized
    }

    /// Slides the panel in on the screen the mouse is on and focuses the text field.
    func show() {
        if panel == nil {
            createPanel()
        }
        guard let panel else { return }

        let mouseLocation = NSEvent.mouseLocation
        currentScreen = NSScreen.screens.first(where: { $0.frame.contains(mouseLocation) }) ?? NSScreen.main ?? currentScreen

        isMinimized = false
        hasUnreadReply = false
        isClosingPanel = false
        let targetFrame = onScreenFrame(on: currentScreen)
        if !panel.isVisible {
            panel.setFrame(offScreenFrame(for: targetFrame), display: false)
            panel.alphaValue = 0
        }
        panel.makeKeyAndOrderFront(nil)
        onChatWindowShownOrHidden?(panel.windowNumber)
        NSAnimationContext.runAnimationGroup { animationContext in
            animationContext.duration = 0.28
            animationContext.timingFunction = CAMediaTimingFunction(controlPoints: 0.2, 0.9, 0.3, 1)
            panel.animator().setFrame(targetFrame, display: true)
            panel.animator().alphaValue = 1
        }
    }

    /// Slides the panel back out past the screen edge.
    func hide() {
        guard let panel, panel.isVisible else { return }
        isClosingPanel = true
        let hiddenFrame = offScreenFrame(for: panel.frame)
        NSAnimationContext.runAnimationGroup({ animationContext in
            animationContext.duration = 0.22
            animationContext.timingFunction = CAMediaTimingFunction(name: .easeIn)
            panel.animator().setFrame(hiddenFrame, display: true)
            panel.animator().alphaValue = 0
        }, completionHandler: { [weak self] in
            // Reopened while it was sliding out: keep it.
            guard let self, self.isClosingPanel else { return }
            self.isClosingPanel = false
            panel.orderOut(nil)
            self.onChatWindowShownOrHidden?(nil)
        })
    }

    /// Shrinks the chat to a tab on the screen edge, centred on where the chat was.
    func minimize() {
        guard let panel, panel.isVisible, !isMinimized else { return }
        isMinimized = true
        refreshCurrentScreenIfDisconnected()
        // The tab has no text field, so the keyboard goes back to the user's app.
        giveKeyboardFocusBackToFrontmostApp()
        animate(panel, to: minimizedTabFrame(centeredOn: panel.frame.midY))
    }

    func expandFromMinimized() {
        guard let panel, isMinimized else { return }
        isMinimized = false
        hasUnreadReply = false
        refreshCurrentScreenIfDisconnected()
        panel.makeKeyAndOrderFront(nil)
        animate(panel, to: onScreenFrame(on: currentScreen))
    }

    /// On while Claude clicks for the user, so a click aimed at something
    /// behind the chat (screenshots leave the chat out) reaches it.
    func setLetsClicksPassThrough(_ letsClicksPassThrough: Bool) {
        panel?.ignoresMouseEvents = letsClicksPassThrough
    }

    /// After a display is unplugged, the remembered screen's frame points
    /// off screen; use the screen AppKit moved the panel to instead.
    private func refreshCurrentScreenIfDisconnected() {
        guard !NSScreen.screens.contains(currentScreen) else { return }
        currentScreen = panel?.screen ?? NSScreen.main ?? NSScreen.screens[0]
    }

    private func moveToCurrentSide() {
        guard let panel, panel.isVisible else { return }
        refreshCurrentScreenIfDisconnected()
        let targetFrame = isMinimized ? minimizedTabFrame(centeredOn: panel.frame.midY) : onScreenFrame(on: currentScreen)
        animate(panel, to: targetFrame)
    }

    private func animate(_ panel: NSPanel, to targetFrame: NSRect) {
        NSAnimationContext.runAnimationGroup { animationContext in
            animationContext.duration = 0.26
            animationContext.timingFunction = CAMediaTimingFunction(controlPoints: 0.2, 0.9, 0.3, 1)
            panel.animator().setFrame(targetFrame, display: true)
        }
    }

    /// Hands keyboard focus back to the app underneath, so [TYPE] and [KEY]
    /// actions land there instead of in the chat's own text field. Ordering
    /// the panel out and back in without making it key does that, because
    /// the panel never activated the app.
    func giveKeyboardFocusBackToFrontmostApp() {
        guard let panel, panel.isKeyWindow else { return }
        panel.orderOut(nil)
        panel.orderFrontRegardless()
    }

    // MARK: - Frames

    /// Full height of the visible area (below the menu bar, above the Dock),
    /// inset from the chosen edge so it reads as a floating card.
    private func onScreenFrame(on screen: NSScreen) -> NSRect {
        let visibleFrame = screen.visibleFrame
        let panelHeight = visibleFrame.height - Self.gapFromScreenEdges * 2
        let originX = panelSide == .right
            ? visibleFrame.maxX - Self.panelWidth - Self.gapFromScreenEdges
            : visibleFrame.minX + Self.gapFromScreenEdges
        return NSRect(x: originX, y: visibleFrame.minY + Self.gapFromScreenEdges, width: Self.panelWidth, height: panelHeight)
    }

    /// Flush with the screen edge so it reads as the chat tucked away there.
    private func minimizedTabFrame(centeredOn centerY: CGFloat) -> NSRect {
        let visibleFrame = currentScreen.visibleFrame
        let tabSize = Self.minimizedTabSize
        let originX = panelSide == .right ? visibleFrame.maxX - tabSize.width : visibleFrame.minX
        let originY = min(max(centerY - tabSize.height / 2, visibleFrame.minY), visibleFrame.maxY - tabSize.height)
        return NSRect(x: originX, y: originY, width: tabSize.width, height: tabSize.height)
    }

    private func offScreenFrame(for onScreenFrame: NSRect) -> NSRect {
        let slideDistance = Self.panelWidth * 0.35
        return onScreenFrame.offsetBy(dx: panelSide == .right ? slideDistance : -slideDistance, dy: 0)
    }

    private func createPanel() {
        let chatPanel = ChatKeyablePanel(
            contentRect: NSRect(x: 0, y: 0, width: Self.panelWidth, height: 600),
            styleMask: [.borderless, .nonactivatingPanel, .resizable],
            backing: .buffered,
            defer: false
        )
        chatPanel.isFloatingPanel = true
        chatPanel.level = .floating
        chatPanel.isOpaque = false
        chatPanel.backgroundColor = .clear
        chatPanel.hasShadow = true
        chatPanel.hidesOnDeactivate = false
        chatPanel.isExcludedFromWindowsMenu = true
        chatPanel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        chatPanel.onEscapePressed = { [weak self] in self?.minimize() }

        let hostingView = NSHostingView(rootView: ChatPanelView(companionManager: companionManager, chatPanelManager: self))
        hostingView.wantsLayer = true
        hostingView.layer?.backgroundColor = .clear
        chatPanel.contentView = hostingView
        panel = chatPanel
    }
}

// MARK: - View

struct ChatPanelView: View {
    @ObservedObject var companionManager: CompanionManager
    @ObservedObject var chatPanelManager: ChatPanelManager

    @State private var draftMessage = ""
    @FocusState private var isComposerFocused: Bool

    private static let suggestedPrompts = [
        "What's on my screen?",
        "Explain this to me",
        "Make notes of this page",
    ]

    var body: some View {
        if chatPanelManager.isMinimized {
            ChatMinimizedTab(
                panelSide: companionManager.chatPanelSide,
                hasUnreadReply: chatPanelManager.hasUnreadReply,
                action: { chatPanelManager.expandFromMinimized() }
            )
        } else {
            expandedChat
        }
    }

    private var expandedChat: some View {
        VStack(spacing: 0) {
            header
            // Divider() ignores .background, so the line is drawn by hand.
            Rectangle()
                .fill(DS.Colors.borderSubtle)
                .frame(height: 1)
            messageList
            composer
        }
        .background(
            RoundedRectangle(cornerRadius: DS.CornerRadius.extraLarge, style: .continuous)
                .fill(DS.Colors.background)
                .overlay(
                    RoundedRectangle(cornerRadius: DS.CornerRadius.extraLarge, style: .continuous)
                        .stroke(DS.Colors.borderSubtle, lineWidth: 1)
                )
        )
        .clipShape(RoundedRectangle(cornerRadius: DS.CornerRadius.extraLarge, style: .continuous))
        .onAppear { isComposerFocused = true }
        // The focused text field takes Esc before the panel's
        // cancelOperation would, so it is caught here too.
        .onExitCommand { chatPanelManager.minimize() }
    }

    // MARK: Header

    private var header: some View {
        HStack(spacing: 6) {
            BhaiLogoView(sideLength: 22)
            Text("bhai")
                .font(.system(size: 14, weight: .semibold))
                .foregroundColor(DS.Colors.textPrimary)
            Spacer()
            ChatIconButton(
                systemImageName: "arrow.left.arrow.right",
                helpText: companionManager.chatPanelSide == .right ? "Move the chat to the left edge" : "Move the chat to the right edge"
            ) {
                companionManager.chatPanelSide = companionManager.chatPanelSide == .right ? .left : .right
            }
            ChatIconButton(
                systemImageName: companionManager.chatPanelSide == .right ? "chevron.right.2" : "chevron.left.2",
                helpText: "Minimize to the screen edge (Esc)"
            ) {
                chatPanelManager.minimize()
            }
            ChatIconButton(systemImageName: "xmark", helpText: "Close") {
                // Closing the chat (not minimizing it) goes back to voice mode,
                // which also slides the chat away.
                companionManager.isChatInputMode = false
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }

    // MARK: Messages

    private var messageList: some View {
        ScrollViewReader { scrollProxy in
            ScrollView(.vertical, showsIndicators: false) {
                VStack(alignment: .leading, spacing: 8) {
                    if companionManager.chatMessages.isEmpty {
                        emptyState
                    }
                    ForEach(Array(companionManager.chatMessages.enumerated()), id: \.element.id) { messageIndex, chatMessage in
                        // Extra space above each new question, so a question
                        // and its answer read as one group.
                        let isQuestionAfterReply = chatMessage.isFromUser
                            && messageIndex > 0
                            && !companionManager.chatMessages[messageIndex - 1].isFromUser
                        ChatMessageRow(chatMessage: chatMessage)
                            .padding(.top, isQuestionAfterReply ? 12 : 0)
                            .id(chatMessage.id)
                    }
                }
                .padding(14)
            }
            .onChange(of: companionManager.chatMessages) { _, newMessages in
                guard let lastMessage = newMessages.last else { return }
                withAnimation(.easeOut(duration: 0.2)) {
                    scrollProxy.scrollTo(lastMessage.id, anchor: .bottom)
                }
            }
        }
        .frame(maxHeight: .infinity)
    }

    private var emptyState: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Ask about anything on your screen.")
                .font(.system(size: 15, weight: .semibold))
                .foregroundColor(DS.Colors.textPrimary)
            Text("Same as holding the hotkey and talking: it sees your screen, points, draws, and can do things for you.")
                .font(.system(size: 12))
                .foregroundColor(DS.Colors.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            VStack(alignment: .leading, spacing: 6) {
                ForEach(Self.suggestedPrompts, id: \.self) { suggestedPrompt in
                    ChatSuggestionChip(text: suggestedPrompt) {
                        send(suggestedPrompt)
                    }
                }
            }
            .padding(.top, 4)
        }
        .padding(.top, 8)
    }

    // MARK: Composer

    private var composer: some View {
        VStack(spacing: 8) {
            HStack(alignment: .bottom, spacing: 8) {
                // Return sends; shift+return adds a line (vertical-axis TextField).
                TextField("Message bhai...", text: $draftMessage, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.system(size: 13))
                    .foregroundColor(DS.Colors.textPrimary)
                    .lineLimit(1...6)
                    // Lines the first row up with the middle of the send button.
                    .padding(.vertical, 3)
                    .focused($isComposerFocused)
                    .onSubmit { send(draftMessage) }

                ChatSendButton(isEnabled: !draftMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) {
                    send(draftMessage)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: DS.CornerRadius.large, style: .continuous)
                    .fill(DS.Colors.surface2)
                    .overlay(
                        RoundedRectangle(cornerRadius: DS.CornerRadius.large, style: .continuous)
                            .stroke(isComposerFocused ? DS.Colors.overlayCursorColor.opacity(0.6) : DS.Colors.borderSubtle, lineWidth: 1)
                    )
            )

            HStack(spacing: 8) {
                Text(companionManager.selectedBrainEngine == .codex ? "Codex" : companionManager.selectedModel.capitalized)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(DS.Colors.textTertiary)
                Spacer()
                Text("Replies")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(DS.Colors.textTertiary)
                ChatSegmentedToggle(
                    options: [(label: "Text", optionID: "text"), (label: "Text + voice", optionID: "voice")],
                    selectedOptionID: companionManager.speaksChatReplies ? "voice" : "text",
                    helpText: "How typed messages are answered. Spoken questions always get a spoken reply."
                ) { selectedOptionID in
                    companionManager.speaksChatReplies = selectedOptionID == "voice"
                }
            }
            .padding(.horizontal, 4)
        }
        .padding(12)
    }

    private func send(_ messageText: String) {
        let trimmedMessage = messageText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedMessage.isEmpty else { return }
        draftMessage = ""
        companionManager.sendTypedChatMessage(trimmedMessage)
    }
}

// MARK: - Pieces

private struct ChatMessageRow: View {
    let chatMessage: CompanionChatMessage

    var body: some View {
        if chatMessage.isFromUser {
            HStack {
                Spacer(minLength: 40)
                Text(chatMessage.text)
                    .font(.system(size: 13))
                    .foregroundColor(.white)
                    .textSelection(.enabled)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .fill(DS.Colors.overlayCursorColor)
                    )
            }
        } else if chatMessage.isStreaming && chatMessage.text.isEmpty {
            // Before the first words arrive.
            HStack(alignment: .center, spacing: 8) {
                BhaiLogoView(sideLength: 20)
                ChatThinkingDots()
            }
        } else {
            HStack(alignment: .top, spacing: 8) {
                BhaiLogoView(sideLength: 20)
                    .padding(.top, -2)
                // A thin caret trails the text while it is still streaming.
                (Text(chatMessage.text) + Text(chatMessage.isStreaming ? "\u{2009}\u{258F}" : "").foregroundColor(DS.Colors.overlayCursorColor))
                    .font(.system(size: 13))
                    .foregroundColor(DS.Colors.textPrimary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .animation(.easeOut(duration: 0.12), value: chatMessage.text)
                Spacer(minLength: 16)
            }
        }
    }
}

private struct ChatThinkingDots: View {
    @State private var isAnimating = false

    var body: some View {
        HStack(spacing: 4) {
            ForEach(0..<3, id: \.self) { dotIndex in
                Circle()
                    .fill(DS.Colors.textTertiary)
                    .frame(width: 6, height: 6)
                    .opacity(isAnimating ? 1 : 0.3)
                    .animation(
                        .easeInOut(duration: 0.6).repeatForever().delay(Double(dotIndex) * 0.18),
                        value: isAnimating
                    )
            }
        }
        .onAppear { isAnimating = true }
    }
}

private struct ChatSuggestionChip: View {
    let text: String
    let action: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            Text(text)
                .font(.system(size: 12, weight: .medium))
                .foregroundColor(isHovered ? DS.Colors.textPrimary : DS.Colors.textSecondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(
                    Capsule().fill(Color.white.opacity(isHovered ? 0.1 : 0.05))
                )
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .pointerCursor()
    }
}

private struct ChatSendButton: View {
    let isEnabled: Bool
    let action: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            Image(systemName: "arrow.up")
                .font(.system(size: 12, weight: .bold))
                .foregroundColor(.white)
                .frame(width: 26, height: 26)
                .background(
                    Circle().fill(isEnabled ? DS.Colors.overlayCursorColor.opacity(isHovered ? 0.85 : 1) : Color.white.opacity(0.1))
                )
        }
        .buttonStyle(.plain)
        .disabled(!isEnabled)
        .onHover { isHovered = $0 }
        .pointerCursor(isEnabled: isEnabled)
    }
}

private struct ChatIconButton: View {
    let systemImageName: String
    let helpText: String
    let action: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            Image(systemName: systemImageName)
                .font(.system(size: 11, weight: .semibold))
                .foregroundColor(isHovered ? DS.Colors.textPrimary : DS.Colors.textTertiary)
                .frame(width: 24, height: 24)
                .background(Circle().fill(Color.white.opacity(isHovered ? 0.12 : 0.06)))
        }
        .buttonStyle(.plain)
        .help(helpText)
        .onHover { isHovered = $0 }
        .pointerCursor()
    }
}

/// Small two-or-more option toggle used under the composer.
private struct ChatSegmentedToggle: View {
    let options: [(label: String, optionID: String)]
    let selectedOptionID: String
    let helpText: String
    let onSelectOption: (String) -> Void

    var body: some View {
        HStack(spacing: 0) {
            ForEach(options, id: \.optionID) { option in
                ChatSegmentButton(
                    label: option.label,
                    isSelected: option.optionID == selectedOptionID,
                    action: { onSelectOption(option.optionID) }
                )
            }
        }
        .padding(2)
        .background(Capsule().fill(Color.white.opacity(0.06)))
        // strokeBorder keeps the ring inside the capsule so it isn't cut off at the ends.
        .overlay(Capsule().strokeBorder(DS.Colors.borderSubtle, lineWidth: 0.5))
        .help(helpText)
    }
}

private struct ChatSegmentButton: View {
    let label: String
    let isSelected: Bool
    let action: () -> Void
    @State private var isHovered = false

    var body: some View {
        Button(action: action) {
            Text(label)
                .font(.system(size: 10, weight: .semibold))
                .foregroundColor(isSelected ? .white : (isHovered ? DS.Colors.textSecondary : DS.Colors.textTertiary))
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(Capsule().fill(isSelected ? DS.Colors.overlayCursorColor : Color.clear))
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .onHover { isHovered = $0 }
        .pointerCursor()
    }
}

/// The minimized chat: a tab poking out of the screen edge, rounded only on
/// the inner side so it looks like the chat is tucked behind the edge.
private struct ChatMinimizedTab: View {
    let panelSide: ChatPanelSide
    let hasUnreadReply: Bool
    let action: () -> Void
    @State private var isHovered = false

    private var tabBackgroundColor: Color { isHovered ? DS.Colors.surface3 : DS.Colors.surface2 }

    var body: some View {
        let innerCornerRadius: CGFloat = 12
        let tabShape = UnevenRoundedRectangle(
            topLeadingRadius: panelSide == .right ? innerCornerRadius : 0,
            bottomLeadingRadius: panelSide == .right ? innerCornerRadius : 0,
            bottomTrailingRadius: panelSide == .left ? innerCornerRadius : 0,
            topTrailingRadius: panelSide == .left ? innerCornerRadius : 0,
            style: .continuous
        )
        Button(action: action) {
            VStack(spacing: 8) {
                Image(systemName: panelSide == .right ? "chevron.left" : "chevron.right")
                    .font(.system(size: 11, weight: .bold))
                Image(systemName: "bubble.left.fill")
                    .font(.system(size: 12))
                    // Unread badge on the bubble's corner, ringed in the tab's color.
                    .overlay(alignment: .topTrailing) {
                        if hasUnreadReply {
                            Circle()
                                .fill(DS.Colors.overlayCursorColor)
                                .frame(width: 7, height: 7)
                                .overlay(Circle().stroke(tabBackgroundColor, lineWidth: 1.5))
                                .offset(x: 4, y: -3)
                        }
                    }
            }
            .foregroundColor(isHovered ? .white : DS.Colors.textSecondary)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(tabShape.fill(tabBackgroundColor))
            // A lighter edge than the chat's, so the tab stays visible on dark wallpapers.
            .overlay(tabShape.stroke(Color.white.opacity(0.14), lineWidth: 1))
            .contentShape(tabShape)
        }
        .buttonStyle(.plain)
        .help("Open chat")
        .onHover { isHovered = $0 }
        .pointerCursor()
    }
}
