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

enum ChatPanelSide: String {
    case left
    case right
}

@MainActor
final class ChatPanelManager: ObservableObject {
    static let chatPanelSideUserDefaultsKey = "chatPanelSide"
    private static let panelWidth: CGFloat = 380
    private static let gapFromScreenEdges: CGFloat = 12

    @Published var panelSide: ChatPanelSide = ChatPanelSide(
        rawValue: UserDefaults.standard.string(forKey: ChatPanelManager.chatPanelSideUserDefaultsKey) ?? ""
    ) ?? .right {
        didSet {
            UserDefaults.standard.set(panelSide.rawValue, forKey: Self.chatPanelSideUserDefaultsKey)
            if let panel, panel.isVisible {
                panel.setFrame(onScreenFrame(on: currentScreen), display: true, animate: true)
            }
        }
    }

    private unowned let companionManager: CompanionManager
    private var panel: ChatKeyablePanel?
    private var currentScreen: NSScreen = NSScreen.main ?? NSScreen.screens[0]

    init(companionManager: CompanionManager) {
        self.companionManager = companionManager
    }

    var isVisible: Bool { panel?.isVisible ?? false }

    /// Slides the panel in on the screen the mouse is on and focuses the text field.
    func show() {
        if panel == nil {
            createPanel()
        }
        guard let panel else { return }

        let mouseLocation = NSEvent.mouseLocation
        currentScreen = NSScreen.screens.first(where: { $0.frame.contains(mouseLocation) }) ?? NSScreen.main ?? currentScreen

        let targetFrame = onScreenFrame(on: currentScreen)
        if !panel.isVisible {
            panel.setFrame(offScreenFrame(for: targetFrame), display: false)
            panel.alphaValue = 0
        }
        panel.makeKeyAndOrderFront(nil)
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
        let hiddenFrame = offScreenFrame(for: panel.frame)
        NSAnimationContext.runAnimationGroup({ animationContext in
            animationContext.duration = 0.22
            animationContext.timingFunction = CAMediaTimingFunction(name: .easeIn)
            panel.animator().setFrame(hiddenFrame, display: true)
            panel.animator().alphaValue = 0
        }, completionHandler: {
            panel.orderOut(nil)
        })
    }

    func switchSide() {
        panelSide = panelSide == .right ? .left : .right
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
        chatPanel.onEscapePressed = { [weak self] in self?.hide() }

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

    /// Thinking dots show between sending and the first reply line.
    private var isWaitingForReply: Bool {
        companionManager.voiceState == .processing && (companionManager.chatMessages.last?.isFromUser ?? false)
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().background(DS.Colors.borderSubtle)
            messageList
            composer
        }
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(DS.Colors.background)
                .overlay(
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .stroke(DS.Colors.borderSubtle, lineWidth: 1)
                )
        )
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
        .onAppear { isComposerFocused = true }
    }

    // MARK: Header

    private var header: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(DS.Colors.overlayCursorColor)
                .frame(width: 8, height: 8)
                .shadow(color: DS.Colors.overlayCursorColor.opacity(0.6), radius: 4)
            Text("hellobhai")
                .font(.system(size: 14, weight: .semibold))
                .foregroundColor(DS.Colors.textPrimary)
            Spacer()
            ChatIconButton(
                systemImageName: chatPanelManager.panelSide == .right ? "rectangle.lefthalf.inset.filled" : "rectangle.righthalf.inset.filled",
                helpText: chatPanelManager.panelSide == .right ? "Move to the left" : "Move to the right"
            ) {
                chatPanelManager.switchSide()
            }
            ChatIconButton(systemImageName: "xmark", helpText: "Close (Esc)") {
                chatPanelManager.hide()
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }

    // MARK: Messages

    private var messageList: some View {
        ScrollViewReader { scrollProxy in
            ScrollView(.vertical, showsIndicators: false) {
                VStack(alignment: .leading, spacing: 12) {
                    if companionManager.chatMessages.isEmpty {
                        emptyState
                    }
                    ForEach(companionManager.chatMessages) { chatMessage in
                        ChatMessageRow(chatMessage: chatMessage)
                            .id(chatMessage.id)
                    }
                    if isWaitingForReply {
                        ChatThinkingDots()
                            .id("thinking")
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
                TextField("Message hellobhai...", text: $draftMessage, axis: .vertical)
                    .textFieldStyle(.plain)
                    .font(.system(size: 13))
                    .foregroundColor(DS.Colors.textPrimary)
                    .lineLimit(1...6)
                    .focused($isComposerFocused)
                    .onSubmit { send(draftMessage) }

                ChatSendButton(isEnabled: !draftMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) {
                    send(draftMessage)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .background(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(DS.Colors.surface2)
                    .overlay(
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .stroke(isComposerFocused ? DS.Colors.overlayCursorColor.opacity(0.6) : DS.Colors.borderSubtle, lineWidth: 1)
                    )
            )

            HStack(spacing: 8) {
                Text(companionManager.selectedModel.capitalized)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(DS.Colors.textTertiary)
                Spacer()
                Text("Speak replies")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(DS.Colors.textTertiary)
                Toggle("", isOn: $companionManager.speaksChatReplies)
                    .toggleStyle(.switch)
                    .tint(DS.Colors.overlayCursorColor)
                    .controlSize(.mini)
                    .labelsHidden()
                    .pointerCursor()
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
        } else {
            HStack(alignment: .top, spacing: 8) {
                Circle()
                    .fill(DS.Colors.overlayCursorColor.opacity(0.9))
                    .frame(width: 6, height: 6)
                    .padding(.top, 6)
                Text(chatMessage.text)
                    .font(.system(size: 13))
                    .foregroundColor(DS.Colors.textPrimary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
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
        .padding(.leading, 14)
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
