//
//  OrbWindow.swift
//  leanring-buddy
//
//  hellobhai's home on screen: a small glowing orb (the bundled orb.riv,
//  drawn by Rive's web runtime in a transparent WKWebView) that sits in a
//  borderless, non-activating, always-on-top panel. The user can drag it
//  anywhere; a click opens the chat panel. Its animation shows what bhai is
//  doing (listening, thinking, speaking, asleep), and when Claude points,
//  clicks or draws, the blue pointer launches out of it and flies back in.
//
//  Locked down like the whiteboard: a CSP blocks all network loads,
//  navigation is refused, storage is non-persistent, and Swift only ever
//  calls the page with fixed state names and numbers.
//

import AppKit
import Combine
import WebKit

/// What the orb is showing. Hover is layered on top by the window itself.
enum OrbVisualState: String {
    case asleep
    case idle
    case listening
    case thinking
    case speaking
}

/// One of the orb.riv built-in color variants (its `color` number input).
struct OrbColorVariant: Identifiable {
    let colorInputValue: Int
    /// Two colors from the variant's swirl, for the settings swatch.
    let swatchGradientHexColors: [String]
    var id: Int { colorInputValue }

    static let defaultColorInputValue = 8

    /// orb.riv answers to color values 0 through 9; higher values look the
    /// same as 9. Swatch colors were picked from renders of each value.
    static let allVariants: [OrbColorVariant] = [
        OrbColorVariant(colorInputValue: 0, swatchGradientHexColors: ["#FFE03C", "#FF3B5C"]),
        OrbColorVariant(colorInputValue: 1, swatchGradientHexColors: ["#FF4FD8", "#3FD0F5"]),
        OrbColorVariant(colorInputValue: 2, swatchGradientHexColors: ["#FF8A80", "#E8322E"]),
        OrbColorVariant(colorInputValue: 3, swatchGradientHexColors: ["#FF7A8A", "#FFC93C"]),
        OrbColorVariant(colorInputValue: 4, swatchGradientHexColors: ["#FFE14D", "#F08A24"]),
        OrbColorVariant(colorInputValue: 5, swatchGradientHexColors: ["#7DF59A", "#1DB954"]),
        OrbColorVariant(colorInputValue: 6, swatchGradientHexColors: ["#7FFFF2", "#12BFAE"]),
        OrbColorVariant(colorInputValue: 7, swatchGradientHexColors: ["#4FC3F7", "#1D4ED8"]),
        OrbColorVariant(colorInputValue: 8, swatchGradientHexColors: ["#3FD5F5", "#7A4DFF"]),
        OrbColorVariant(colorInputValue: 9, swatchGradientHexColors: ["#E07BFF", "#6A4DFF"])
    ]
}

@MainActor
final class OrbWindowManager: NSObject, ObservableObject, WKNavigationDelegate {
    static let orbWindowSize = NSSize(width: 76, height: 76)
    /// Gap kept between the default position and the screen's corner.
    private static let defaultMarginFromScreenCorner: CGFloat = 24
    private static let savedOriginXUserDefaultsKey = "orbWindowOriginX"
    private static let savedOriginYUserDefaultsKey = "orbWindowOriginY"
    /// orb.riv's own "asleep" animation shrinks the orb to nothing, which
    /// would leave nothing to find, drag or click. Asleep is shown as the
    /// idle orb dimmed instead.
    private static let asleepWindowAlpha: CGFloat = 0.45

    /// Where the orb is on screen (global AppKit coordinates). The overlay
    /// launches the pointer from its center and places onboarding bubbles
    /// next to it.
    @Published private(set) var orbFrameInScreenCoordinates: CGRect = .zero

    /// Called on a click that wasn't a drag.
    var onOrbClicked: (() -> Void)?
    /// Called on any hover, click or drag, so a sleeping orb wakes up.
    var onUserInteractedWithOrb: (() -> Void)?

    private var panel: NSPanel?
    private var webView: WKWebView?
    private var orbPageNavigation: WKNavigation?
    private var isPageReady = false

    private var currentVisualState: OrbVisualState = .idle
    private var currentColorInputValue = OrbColorVariant.defaultColorInputValue
    private var isMouseHoveringOverOrb = false
    private var isHiddenBecauseSystemPointerIsHidden = false
    private var systemPointerVisibilityTimer: Timer?

    private static func bundledResourceData(named resourceName: String, withExtension fileExtension: String) -> Data? {
        guard let resourceURL = Bundle.main.url(forResource: resourceName, withExtension: fileExtension),
              let resourceData = try? Data(contentsOf: resourceURL) else {
            print("⚠️ Orb: missing bundled \(resourceName).\(fileExtension)")
            return nil
        }
        return resourceData
    }

    /// The page template with the Rive runtime (wasm inlined) and orb.riv
    /// filled in, so the web view needs no file or network access.
    private static let orbPageHTML: String = {
        let pageTemplate = bundledResourceData(named: "orb", withExtension: "html")
            .flatMap { String(data: $0, encoding: .utf8) } ?? ""
        let riveRuntimeSource = bundledResourceData(named: "rive", withExtension: "js")
            .flatMap { String(data: $0, encoding: .utf8) } ?? ""
        let orbFileBase64 = bundledResourceData(named: "orb", withExtension: "riv")?.base64EncodedString() ?? ""
        return pageTemplate
            .replacingOccurrences(of: "__ORB_RIV_BASE64__", with: orbFileBase64)
            .replacingOccurrences(of: "/*__RIVE_RUNTIME_SOURCE__*/", with: riveRuntimeSource)
    }()

    override init() {
        super.init()
        // Known before the panel exists, so the overlay can place the
        // welcome bubble next to the orb as soon as it appears.
        orbFrameInScreenCoordinates = NSRect(origin: savedOrDefaultOrigin(), size: Self.orbWindowSize)
    }

    var orbCenterInScreenCoordinates: CGPoint {
        CGPoint(x: orbFrameInScreenCoordinates.midX, y: orbFrameInScreenCoordinates.midY)
    }

    // MARK: - Showing

    func show() {
        if panel == nil {
            createPanel()
        }
        guard let panel else { return }
        panel.setFrameOrigin(savedOrDefaultOrigin())
        orbFrameInScreenCoordinates = panel.frame
        if !isPageReady && orbPageNavigation == nil {
            orbPageNavigation = webView?.loadHTMLString(Self.orbPageHTML, baseURL: nil)
        }
        panel.orderFrontRegardless()
        startWatchingSystemPointerVisibility()
    }

    func hide() {
        panel?.orderOut(nil)
        systemPointerVisibilityTimer?.invalidate()
        systemPointerVisibilityTimer = nil
    }

    // MARK: - State and Color

    func setVisualState(_ visualState: OrbVisualState) {
        guard visualState != currentVisualState else { return }
        currentVisualState = visualState
        applyStateToPageAndWindow()
    }

    func setColorInputValue(_ colorInputValue: Int) {
        currentColorInputValue = colorInputValue
        guard isPageReady else { return }
        // A plain integer, so nothing user-controlled is ever evaluated.
        webView?.evaluateJavaScript("hb.setOrbColor(\(colorInputValue))")
    }

    private func applyStateToPageAndWindow() {
        // Hover only shows over a resting orb; it never hides listening,
        // thinking or speaking.
        let isResting = currentVisualState == .idle || currentVisualState == .asleep
        let pageStateName: String
        if isMouseHoveringOverOrb && isResting {
            pageStateName = "hover"
        } else if currentVisualState == .asleep {
            pageStateName = OrbVisualState.idle.rawValue
        } else {
            pageStateName = currentVisualState.rawValue
        }
        if isPageReady {
            webView?.evaluateJavaScript("hb.setOrbState('\(pageStateName)')")
        }

        let targetAlpha: CGFloat
        if isHiddenBecauseSystemPointerIsHidden {
            targetAlpha = 0
        } else if currentVisualState == .asleep && !isMouseHoveringOverOrb {
            targetAlpha = Self.asleepWindowAlpha
        } else {
            targetAlpha = 1
        }
        guard let panel, panel.alphaValue != targetAlpha else { return }
        NSAnimationContext.runAnimationGroup { animationContext in
            animationContext.duration = 0.3
            panel.animator().alphaValue = targetAlpha
        }
    }

    // MARK: - Fullscreen Video

    /// A video player hides the real pointer while it plays fullscreen; the
    /// orb steps out of the way with it, but only while bhai is resting.
    private func startWatchingSystemPointerVisibility() {
        guard systemPointerVisibilityTimer == nil else { return }
        systemPointerVisibilityTimer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                let isResting = self.currentVisualState == .idle || self.currentVisualState == .asleep
                let shouldHide = isResting && !SystemPointerVisibility.isSystemPointerVisible()
                guard shouldHide != self.isHiddenBecauseSystemPointerIsHidden else { return }
                self.isHiddenBecauseSystemPointerIsHidden = shouldHide
                // A hidden orb must not catch clicks meant for the video.
                self.panel?.ignoresMouseEvents = shouldHide
                self.applyStateToPageAndWindow()
            }
        }
    }

    // MARK: - Position

    private func savedOrDefaultOrigin() -> NSPoint {
        let userDefaults = UserDefaults.standard
        if userDefaults.object(forKey: Self.savedOriginXUserDefaultsKey) != nil {
            let savedOrigin = NSPoint(
                x: userDefaults.double(forKey: Self.savedOriginXUserDefaultsKey),
                y: userDefaults.double(forKey: Self.savedOriginYUserDefaultsKey)
            )
            // Only reuse it if that spot is still on a connected screen.
            let savedFrame = NSRect(origin: savedOrigin, size: Self.orbWindowSize)
            if NSScreen.screens.contains(where: { $0.visibleFrame.intersects(savedFrame) }) {
                return Self.originKeptOnScreen(savedOrigin)
            }
        }
        // Bottom-right of the main screen; visibleFrame already ends above the Dock.
        let mainVisibleFrame = (NSScreen.main ?? NSScreen.screens.first)?.visibleFrame ?? .zero
        return NSPoint(
            x: mainVisibleFrame.maxX - Self.orbWindowSize.width - Self.defaultMarginFromScreenCorner,
            y: mainVisibleFrame.minY + Self.defaultMarginFromScreenCorner
        )
    }

    /// Clamps a window origin so the whole orb stays inside the visible
    /// frame (no menu bar, no Dock) of the screen it is mostly on.
    private static func originKeptOnScreen(_ proposedOrigin: NSPoint) -> NSPoint {
        let proposedCenter = NSPoint(x: proposedOrigin.x + orbWindowSize.width / 2, y: proposedOrigin.y + orbWindowSize.height / 2)
        let screenUnderOrb = NSScreen.screens.first(where: { $0.frame.contains(proposedCenter) })
            ?? NSScreen.screens.min(by: { distanceSquared(from: proposedCenter, to: $0.frame) < distanceSquared(from: proposedCenter, to: $1.frame) })
        guard let visibleFrame = screenUnderOrb?.visibleFrame else { return proposedOrigin }
        return NSPoint(
            x: min(max(proposedOrigin.x, visibleFrame.minX), visibleFrame.maxX - orbWindowSize.width),
            y: min(max(proposedOrigin.y, visibleFrame.minY), visibleFrame.maxY - orbWindowSize.height)
        )
    }

    private static func distanceSquared(from point: NSPoint, to rect: NSRect) -> CGFloat {
        let nearestX = min(max(point.x, rect.minX), rect.maxX)
        let nearestY = min(max(point.y, rect.minY), rect.maxY)
        return (point.x - nearestX) * (point.x - nearestX) + (point.y - nearestY) * (point.y - nearestY)
    }

    fileprivate func moveOrb(toProposedOrigin proposedOrigin: NSPoint) {
        guard let panel else { return }
        panel.setFrameOrigin(Self.originKeptOnScreen(proposedOrigin))
        orbFrameInScreenCoordinates = panel.frame
    }

    fileprivate func finishDraggingOrb() {
        guard let panel else { return }
        UserDefaults.standard.set(Double(panel.frame.origin.x), forKey: Self.savedOriginXUserDefaultsKey)
        UserDefaults.standard.set(Double(panel.frame.origin.y), forKey: Self.savedOriginYUserDefaultsKey)
        onUserInteractedWithOrb?()
    }

    fileprivate var currentPanelOrigin: NSPoint {
        panel?.frame.origin ?? .zero
    }

    fileprivate func setMouseHovering(_ isHovering: Bool) {
        guard isHovering != isMouseHoveringOverOrb else { return }
        isMouseHoveringOverOrb = isHovering
        if isHovering {
            onUserInteractedWithOrb?()
        }
        applyStateToPageAndWindow()
    }

    fileprivate func handleOrbClicked() {
        onUserInteractedWithOrb?()
        onOrbClicked?()
    }

    // MARK: - Web View Delegate

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard navigation == orbPageNavigation else { return }
        isPageReady = true
        setColorInputValue(currentColorInputValue)
        applyStateToPageAndWindow()
    }

    /// Only the in-memory page load is allowed; nothing can navigate anywhere.
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        let isInMemoryPageLoad = navigationAction.request.url?.absoluteString == "about:blank"
        decisionHandler(isInMemoryPageLoad ? .allow : .cancel)
    }

    // MARK: - Panel

    private func createPanel() {
        let createdPanel = NSPanel(
            contentRect: NSRect(origin: .zero, size: Self.orbWindowSize),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        createdPanel.isOpaque = false
        createdPanel.backgroundColor = .clear
        createdPanel.hasShadow = false
        // Same level as the cursor overlay, ordered after it, so the orb
        // floats above everything including the overlay's pointer.
        createdPanel.level = .screenSaver
        createdPanel.hidesOnDeactivate = false
        createdPanel.becomesKeyOnlyIfNeeded = true
        createdPanel.isReleasedWhenClosed = false
        createdPanel.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary]

        let webViewConfiguration = WKWebViewConfiguration()
        // Nothing drawn here should ever be persisted to disk.
        webViewConfiguration.websiteDataStore = .nonPersistent()

        let contentFrame = NSRect(origin: .zero, size: Self.orbWindowSize)
        let createdWebView = WKWebView(frame: contentFrame, configuration: webViewConfiguration)
        createdWebView.autoresizingMask = [.width, .height]
        createdWebView.navigationDelegate = self
        // Transparent so only the orb shows over the desktop.
        createdWebView.setValue(false, forKey: "drawsBackground")

        // The page is display-only: a view on top takes every mouse event
        // for dragging, clicking and hover, so Rive never sees the mouse.
        let mouseHandlingView = OrbMouseHandlingView(frame: contentFrame, orbWindowManager: self)
        mouseHandlingView.autoresizingMask = [.width, .height]

        let containerView = NSView(frame: contentFrame)
        containerView.addSubview(createdWebView)
        containerView.addSubview(mouseHandlingView)
        createdPanel.contentView = containerView

        panel = createdPanel
        webView = createdWebView
    }
}

/// Turns mouse input on the orb into drag-to-move, click-to-open-chat and
/// hover, and shows a pointing-hand cursor over it.
private final class OrbMouseHandlingView: NSView {
    /// A press that moves less than this is a click, not a drag.
    private static let dragStartDistance: CGFloat = 3

    private weak var orbWindowManager: OrbWindowManager?
    private var mouseDownLocationInScreen: NSPoint = .zero
    private var panelOriginAtMouseDown: NSPoint = .zero
    private var isDraggingOrb = false

    init(frame: NSRect, orbWindowManager: OrbWindowManager) {
        self.orbWindowManager = orbWindowManager
        super.init(frame: frame)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    // The panel never becomes key, so the first click must count.
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        // activeAlways: the app is almost never frontmost while the user
        // hovers the orb.
        addTrackingArea(NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .mouseMoved, .activeAlways, .inVisibleRect],
            owner: self,
            userInfo: nil
        ))
    }

    override func mouseEntered(with event: NSEvent) {
        NSCursor.pointingHand.set()
        orbWindowManager?.setMouseHovering(true)
    }

    // Cursor rects only work in the key window, so the cursor is set by hand.
    override func mouseMoved(with event: NSEvent) {
        NSCursor.pointingHand.set()
    }

    override func mouseExited(with event: NSEvent) {
        NSCursor.arrow.set()
        orbWindowManager?.setMouseHovering(false)
    }

    override func mouseDown(with event: NSEvent) {
        mouseDownLocationInScreen = NSEvent.mouseLocation
        panelOriginAtMouseDown = orbWindowManager?.currentPanelOrigin ?? .zero
        isDraggingOrb = false
    }

    override func mouseDragged(with event: NSEvent) {
        let currentMouseLocation = NSEvent.mouseLocation
        let dragOffsetX = currentMouseLocation.x - mouseDownLocationInScreen.x
        let dragOffsetY = currentMouseLocation.y - mouseDownLocationInScreen.y
        if !isDraggingOrb && hypot(dragOffsetX, dragOffsetY) < Self.dragStartDistance {
            return
        }
        isDraggingOrb = true
        orbWindowManager?.moveOrb(toProposedOrigin: NSPoint(
            x: panelOriginAtMouseDown.x + dragOffsetX,
            y: panelOriginAtMouseDown.y + dragOffsetY
        ))
    }

    override func mouseUp(with event: NSEvent) {
        if isDraggingOrb {
            orbWindowManager?.finishDraggingOrb()
        } else {
            orbWindowManager?.handleOrbClicked()
        }
        isDraggingOrb = false
    }
}
