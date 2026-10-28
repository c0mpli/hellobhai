//
//  AvatarPanel.swift
//  leanring-buddy
//
//  A round video bubble in the bottom-right corner showing a real-looking
//  person. There is no live face rendering: the person is a handful of short
//  pre-made clips (idle, listening, thinking, several talking takes) that
//  crossfade by state, which looks natural at bubble size and stays local.
//
//  Clips live in ~/Library/Application Support/hellobhai/Avatars/<character>/
//  as idle, listening, thinking and talking1...talkingN (.mp4 or .mov).
//  Only idle is required; missing states fall back to idle.
//

import AppKit
import AVFoundation
import SwiftUI

enum AvatarActivity {
    case idle
    case listening
    case thinking
    case talking
}

@MainActor
final class AvatarPanelManager {
    private static let bubbleDiameter: CGFloat = 168
    private static let screenEdgeMargin: CGFloat = 24
    private static let crossfadeDuration: CFTimeInterval = 0.3

    static let avatarsDirectory = FileManager.default
        .urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("hellobhai/Avatars", isDirectory: true)

    private var panel: NSPanel?
    private var bubbleView: AvatarBubbleView?
    private var clipURLsByActivity: [AvatarActivity: [URL]] = [:]
    private var currentActivity: AvatarActivity = .idle
    private var lastPlayedTalkingClipURL: URL?

    /// Two players so the next clip can start underneath and fade in,
    /// hiding the seam between clips (and between loops of the same clip).
    private var visiblePlayerIndex = 0
    private let players = [AVPlayer(), AVPlayer()]
    private var clipEndObserver: NSObjectProtocol?

    func show(characterName: String) {
        clipURLsByActivity = Self.findClips(characterName: characterName)
        guard clipURLsByActivity[.idle] != nil else {
            print("🧑 Avatar: no clips for \(characterName). Put idle.mp4 (plus optional listening, thinking, talking1...) in \(Self.avatarsDirectory.appendingPathComponent(characterName).path)")
            hide()
            return
        }
        if panel == nil { createPanel() }
        panel?.orderFrontRegardless()
        playClip(for: currentActivity)
    }

    func hide() {
        players.forEach { $0.pause() }
        panel?.orderOut(nil)
    }

    func setActivity(_ activity: AvatarActivity) {
        guard activity != currentActivity else { return }
        currentActivity = activity
        guard panel?.isVisible == true else { return }
        playClip(for: activity)
    }

    // MARK: - Clips

    private static func findClips(characterName: String) -> [AvatarActivity: [URL]] {
        let characterDirectory = avatarsDirectory.appendingPathComponent(characterName, isDirectory: true)
        let fileURLs = (try? FileManager.default.contentsOfDirectory(at: characterDirectory, includingPropertiesForKeys: nil)) ?? []
        let videoURLs = fileURLs
            .filter { ["mp4", "mov", "m4v"].contains($0.pathExtension.lowercased()) }
            .sorted { $0.lastPathComponent < $1.lastPathComponent }

        var clipURLsByActivity: [AvatarActivity: [URL]] = [:]
        for videoURL in videoURLs {
            let clipName = videoURL.deletingPathExtension().lastPathComponent.lowercased()
            let activity: AvatarActivity?
            if clipName == "idle" { activity = .idle }
            else if clipName == "listening" { activity = .listening }
            else if clipName == "thinking" { activity = .thinking }
            else if clipName.hasPrefix("talking") { activity = .talking }
            else { activity = nil }
            if let activity { clipURLsByActivity[activity, default: []].append(videoURL) }
        }
        return clipURLsByActivity
    }

    private func nextClipURL(for activity: AvatarActivity) -> URL? {
        guard let clipURLs = clipURLsByActivity[activity] ?? clipURLsByActivity[.idle] else { return nil }
        guard activity == .talking, clipURLs.count > 1 else { return clipURLs.first }
        // Rotate through talking takes without repeating the same one twice in a row.
        let talkingClipURL = clipURLs.filter { $0 != lastPlayedTalkingClipURL }.randomElement()
        lastPlayedTalkingClipURL = talkingClipURL
        return talkingClipURL
    }

    private func playClip(for activity: AvatarActivity) {
        guard let clipURL = nextClipURL(for: activity), let bubbleView else { return }

        let incomingPlayerIndex = 1 - visiblePlayerIndex
        let incomingPlayer = players[incomingPlayerIndex]
        let outgoingPlayer = players[visiblePlayerIndex]
        let incomingItem = AVPlayerItem(url: clipURL)
        incomingPlayer.replaceCurrentItem(with: incomingItem)
        incomingPlayer.play()

        // Every clip, looping or not, hands over to the next one at its end:
        // the same state's clip again, or a different talking take.
        if let clipEndObserver { NotificationCenter.default.removeObserver(clipEndObserver) }
        clipEndObserver = NotificationCenter.default.addObserver(
            forName: AVPlayerItem.didPlayToEndTimeNotification,
            object: incomingItem,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.playClip(for: self.currentActivity)
            }
        }

        bubbleView.crossfade(toPlayerLayerAt: incomingPlayerIndex, duration: Self.crossfadeDuration)
        visiblePlayerIndex = incomingPlayerIndex
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.crossfadeDuration) { [weak self] in
            // Skip if another clip swapped in meanwhile and the outgoing player is visible again.
            guard let self, self.visiblePlayerIndex == incomingPlayerIndex else { return }
            outgoingPlayer.pause()
        }
    }

    // MARK: - Panel

    private func createPanel() {
        players.forEach { $0.isMuted = true }

        let bubbleFrame = NSRect(x: 0, y: 0, width: Self.bubbleDiameter, height: Self.bubbleDiameter)
        let avatarPanel = NSPanel(
            contentRect: bubbleFrame,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        avatarPanel.isOpaque = false
        avatarPanel.backgroundColor = .clear
        avatarPanel.hasShadow = true
        avatarPanel.level = .floating
        avatarPanel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        avatarPanel.hidesOnDeactivate = false
        // Dragging anywhere on the bubble moves it.
        avatarPanel.isMovableByWindowBackground = true

        let bubbleView = AvatarBubbleView(frame: bubbleFrame, players: players)
        avatarPanel.contentView = bubbleView

        // AppKit remembers where the user dragged the bubble across launches;
        // the first time there is nothing saved, so start in the bottom-right.
        if !avatarPanel.setFrameUsingName("hellobhaiAvatarBubble"), let screen = NSScreen.main {
            let visibleFrame = screen.visibleFrame
            avatarPanel.setFrameOrigin(NSPoint(
                x: visibleFrame.maxX - Self.bubbleDiameter - Self.screenEdgeMargin,
                y: visibleFrame.minY + Self.screenEdgeMargin
            ))
        }
        avatarPanel.setFrameAutosaveName("hellobhaiAvatarBubble")

        panel = avatarPanel
        self.bubbleView = bubbleView
    }
}

/// Circular video view with a thin purple ring. Hosts both player layers
/// stacked; only one is opaque at a time.
private final class AvatarBubbleView: NSView {
    private let playerLayers: [AVPlayerLayer]

    init(frame: NSRect, players: [AVPlayer]) {
        playerLayers = players.map { AVPlayerLayer(player: $0) }
        super.init(frame: frame)
        wantsLayer = true

        let circleLayer = CALayer()
        circleLayer.frame = bounds
        circleLayer.cornerRadius = bounds.width / 2
        circleLayer.masksToBounds = true
        circleLayer.backgroundColor = NSColor.black.cgColor
        for (playerLayerIndex, playerLayer) in playerLayers.enumerated() {
            playerLayer.frame = bounds
            playerLayer.videoGravity = .resizeAspectFill
            playerLayer.opacity = playerLayerIndex == 0 ? 1 : 0
            circleLayer.addSublayer(playerLayer)
        }
        layer?.addSublayer(circleLayer)

        let ringLayer = CAShapeLayer()
        ringLayer.path = CGPath(ellipseIn: bounds.insetBy(dx: 1, dy: 1), transform: nil)
        ringLayer.fillColor = nil
        ringLayer.strokeColor = NSColor(DS.Colors.overlayCursorColor).withAlphaComponent(0.9).cgColor
        ringLayer.lineWidth = 2
        layer?.addSublayer(ringLayer)

        // The open hand tells the user the bubble can be dragged. activeAlways
        // because hellobhai is never the frontmost app.
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect], owner: self))
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override var mouseDownCanMoveWindow: Bool { true }
    override func mouseEntered(with event: NSEvent) { NSCursor.openHand.set() }
    override func mouseExited(with event: NSEvent) { NSCursor.arrow.set() }

    func crossfade(toPlayerLayerAt incomingPlayerLayerIndex: Int, duration: CFTimeInterval) {
        CATransaction.begin()
        CATransaction.setAnimationDuration(duration)
        for (playerLayerIndex, playerLayer) in playerLayers.enumerated() {
            playerLayer.opacity = playerLayerIndex == incomingPlayerLayerIndex ? 1 : 0
        }
        CATransaction.commit()
        window?.invalidateShadow()
    }
}
