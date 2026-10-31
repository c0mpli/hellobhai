//
//  Model3DOverlay.swift
//  leanring-buddy
//
//  A see-through 3D model drawn right over the thing on screen (a circuit
//  board in a product photo, a jet in a video), built live from primitives
//  while Claude narrates: parts pop in, a cutaway slices the casing open,
//  and handwritten callout tags point at each part. Rendering is three.js in
//  a transparent WKWebView inside a borderless, non-activating panel.
//
//  The panel is click-through everywhere except over the model and its
//  labels (so the user can drag to orbit it without the window blocking the
//  screen). The page reports those regions; a timer flips
//  ignoresMouseEvents as the mouse moves in and out of them.
//
//  Locked down like the whiteboard: a CSP blocks all network loads,
//  navigation is refused, storage is non-persistent, and commands are passed
//  in as JSON data (parsed with JSON.parse, never evaluated as code).
//

import AppKit
import WebKit

@MainActor
final class Model3DOverlayManager: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
    /// Largest overlay size; smaller screens get a panel that fits.
    private static let maximumOverlaySize = NSSize(width: 900, height: 660)

    /// The handwriting font for callout tags: the same Patrick Hand the
    /// whiteboard uses.
    private static let handwritingFontResourceName = "PatrickHand"

    private static let escapeKeyCode: UInt16 = 53

    private var panel: NSPanel?
    private var webView: WKWebView?

    /// Commands that arrive before the page has finished loading.
    private var pendingCommandLines: [String] = []
    private var isPageReady = false
    /// The overlay page's load, so the blank page loaded on close can't be
    /// mistaken for it finishing.
    private var overlayPageNavigation: WKNavigation?

    /// Where the model and its labels are, in page coordinates (top-left
    /// origin, points). The panel only takes the mouse inside these.
    private var interactiveRegionsInPage: [CGRect] = []
    private var isMouseOverInteractiveRegion = false
    private var mouseTrackingTimer: Timer?
    private var globalEscapeKeyMonitor: Any?
    private var localEscapeKeyMonitor: Any?

    var isVisible: Bool { panel?.isVisible ?? false }

    /// Called when the user dismisses the model (Esc or its close button),
    /// not when the app closes it, so the narration about it can be stopped.
    var onUserClosedOverlay: (() -> Void)?

    // MARK: - Page

    private static func bundledResourceData(named resourceName: String, withExtension fileExtension: String) -> Data? {
        guard let resourceURL = Bundle.main.url(forResource: resourceName, withExtension: fileExtension),
              let resourceData = try? Data(contentsOf: resourceURL) else {
            print("⚠️ 3D overlay: missing bundled \(resourceName).\(fileExtension)")
            return nil
        }
        return resourceData
    }

    /// The page template with three.js and the handwriting font inlined, so
    /// the web view never needs file or network access.
    private static let overlayPageHTML: String = {
        let pageTemplate = bundledResourceData(named: "model3d", withExtension: "html")
            .flatMap { String(data: $0, encoding: .utf8) } ?? ""
        let threeJSSource = bundledResourceData(named: "three.min", withExtension: "js")
            .flatMap { String(data: $0, encoding: .utf8) } ?? ""
        let handwritingFontBase64 = bundledResourceData(named: handwritingFontResourceName, withExtension: "woff2")?
            .base64EncodedString() ?? ""
        return pageTemplate
            .replacingOccurrences(of: "__HANDWRITING_FONT_BASE64__", with: handwritingFontBase64)
            .replacingOccurrences(of: "<!--__PEN_TRIANGLE_SVG_PATH__-->", with: VisualExplanationPanelManager.penTriangleSVGPath)
            .replacingOccurrences(of: "/*__PENCIL_SCRATCH_SOUND_SCRIPT__*/", with: VisualExplanationPanelManager.pencilScratchSoundScript)
            .replacingOccurrences(of: "/*__THREE_JS_SOURCE__*/", with: threeJSSource)
    }()

    // MARK: - Overlay API

    /// Opens an empty overlay centered on the given point (AppKit global
    /// coordinates), kept inside that display, ready for streamed commands.
    func begin(centeredOnAppKitGlobalPoint centerPoint: CGPoint, onDisplayFrame displayFrame: CGRect) {
        if panel == nil {
            createPanel()
        }
        guard let panel else { return }

        let visibleFrame = NSScreen.screens.first(where: { $0.frame.intersects(displayFrame) })?.visibleFrame ?? displayFrame
        let overlaySize = NSSize(
            width: min(Self.maximumOverlaySize.width, visibleFrame.width - 24),
            height: min(Self.maximumOverlaySize.height, visibleFrame.height - 24)
        )
        var overlayOrigin = NSPoint(x: centerPoint.x - overlaySize.width / 2, y: centerPoint.y - overlaySize.height / 2)
        overlayOrigin.x = min(max(overlayOrigin.x, visibleFrame.minX + 12), visibleFrame.maxX - overlaySize.width - 12)
        overlayOrigin.y = min(max(overlayOrigin.y, visibleFrame.minY + 12), visibleFrame.maxY - overlaySize.height - 12)
        panel.setFrame(NSRect(origin: overlayOrigin, size: overlaySize), display: false)

        isPageReady = false
        pendingCommandLines.removeAll()
        interactiveRegionsInPage.removeAll()
        isMouseOverInteractiveRegion = false
        panel.ignoresMouseEvents = true
        overlayPageNavigation = webView?.loadHTMLString(Self.overlayPageHTML, baseURL: nil)
        panel.orderFrontRegardless()
        startMouseTrackingAndEscapeMonitoring()
    }

    /// Builds one streamed command line. Lines that aren't JSON objects are
    /// ignored by the page.
    func addCommandLine(_ commandLine: String) {
        // Closed (by the user or a newer request): nothing left to build on.
        guard panel?.isVisible == true else { return }
        guard isPageReady else {
            pendingCommandLines.append(commandLine)
            return
        }
        guard let encodedLineData = try? JSONEncoder().encode(commandLine),
              let encodedLine = String(data: encodedLineData, encoding: .utf8) else { return }
        // The line goes in as a JSON string literal, so it can never break
        // out into executable JavaScript.
        webView?.evaluateJavaScript("hb.add(\(encodedLine))")
    }

    func close() {
        guard let panel, panel.isVisible else { return }
        panel.orderOut(nil)
        stopMouseTrackingAndEscapeMonitoring()
        pendingCommandLines.removeAll()
        isPageReady = false
        overlayPageNavigation = nil
        // Unload the page so WebGL rendering and the pen sound stop.
        webView?.loadHTMLString("", baseURL: nil)
    }

    // MARK: - Web View Delegates

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard navigation == overlayPageNavigation else { return }
        isPageReady = true
        let queuedCommandLines = pendingCommandLines
        pendingCommandLines.removeAll()
        queuedCommandLines.forEach(addCommandLine)
    }

    /// Only in-memory page loads are allowed; nothing can navigate anywhere.
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        let isInMemoryPageLoad = navigationAction.request.url?.absoluteString == "about:blank"
        decisionHandler(isInMemoryPageLoad ? .allow : .cancel)
    }

    /// The page sends two kinds of messages: where the model and labels are
    /// (hit regions), and a click on its close button.
    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        guard let messageBody = message.body as? [String: Any],
              let messageKind = messageBody["kind"] as? String else { return }
        switch messageKind {
        case "hitRegions":
            let rectangleValues = (messageBody["rects"] as? [[Double]]) ?? []
            interactiveRegionsInPage = rectangleValues.compactMap { rectangleValue in
                guard rectangleValue.count == 4 else { return nil }
                return CGRect(x: rectangleValue[0], y: rectangleValue[1], width: rectangleValue[2], height: rectangleValue[3])
            }
        case "close":
            closeBecauseUserAskedTo()
        default:
            break
        }
    }

    private func closeBecauseUserAskedTo() {
        guard isVisible else { return }
        close()
        onUserClosedOverlay?()
    }

    // MARK: - Mouse Pass-Through and Escape

    private func startMouseTrackingAndEscapeMonitoring() {
        mouseTrackingTimer?.invalidate()
        mouseTrackingTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 30.0, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated {
                self?.updateMousePassThrough()
            }
        }

        // The panel never becomes key, so Esc is watched globally (works with
        // the Accessibility permission the app already has) and locally.
        if globalEscapeKeyMonitor == nil {
            globalEscapeKeyMonitor = NSEvent.addGlobalMonitorForEvents(matching: .keyDown) { [weak self] keyEvent in
                guard keyEvent.keyCode == Self.escapeKeyCode else { return }
                Task { @MainActor in self?.closeBecauseUserAskedTo() }
            }
        }
        if localEscapeKeyMonitor == nil {
            localEscapeKeyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] keyEvent in
                guard let self, keyEvent.keyCode == Self.escapeKeyCode, self.isVisible else { return keyEvent }
                // Esc still closes the model from this app's other windows
                // (menu bar panel, whiteboard), but only an Esc aimed at the
                // overlay itself is swallowed; the others keep working there.
                let isAimedAtOverlay = keyEvent.window === self.panel
                self.closeBecauseUserAskedTo()
                return isAimedAtOverlay ? nil : keyEvent
            }
        }
    }

    private func stopMouseTrackingAndEscapeMonitoring() {
        mouseTrackingTimer?.invalidate()
        mouseTrackingTimer = nil
        if let globalEscapeKeyMonitor {
            NSEvent.removeMonitor(globalEscapeKeyMonitor)
            self.globalEscapeKeyMonitor = nil
        }
        if let localEscapeKeyMonitor {
            NSEvent.removeMonitor(localEscapeKeyMonitor)
            self.localEscapeKeyMonitor = nil
        }
    }

    /// Takes the mouse only while it's over the model or a label, so the
    /// rest of the overlay never blocks clicks on the screen underneath.
    private func updateMousePassThrough() {
        guard let panel, panel.isVisible else { return }
        // Never switch mid-drag, or an orbit drag would be cut off when the
        // mouse slides past the model's edge.
        guard NSEvent.pressedMouseButtons == 0 else { return }

        let mouseLocation = NSEvent.mouseLocation
        let panelFrame = panel.frame
        // Page coordinates have a top-left origin; AppKit's are bottom-left.
        let mouseLocationInPage = CGPoint(x: mouseLocation.x - panelFrame.minX, y: panelFrame.maxY - mouseLocation.y)
        let isOverInteractiveRegion = interactiveRegionsInPage.contains { $0.contains(mouseLocationInPage) }
        guard isOverInteractiveRegion != isMouseOverInteractiveRegion else { return }

        isMouseOverInteractiveRegion = isOverInteractiveRegion
        panel.ignoresMouseEvents = !isOverInteractiveRegion
        // A non-key window gets no hover events, so the page is told when to
        // show its close button.
        webView?.evaluateJavaScript("hb.setHover(\(isOverInteractiveRegion ? "true" : "false"))")
    }

    // MARK: - Panel

    private func createPanel() {
        let createdPanel = NSPanel(
            contentRect: NSRect(origin: .zero, size: Self.maximumOverlaySize),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        createdPanel.isOpaque = false
        createdPanel.backgroundColor = .clear
        createdPanel.hasShadow = false
        createdPanel.isFloatingPanel = true
        createdPanel.level = .floating
        createdPanel.hidesOnDeactivate = false
        createdPanel.becomesKeyOnlyIfNeeded = true
        createdPanel.isReleasedWhenClosed = false
        createdPanel.ignoresMouseEvents = true
        createdPanel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]

        let webViewConfiguration = WKWebViewConfiguration()
        // Nothing drawn here should ever be persisted to disk.
        webViewConfiguration.websiteDataStore = .nonPersistent()
        // Lets the pen's scratch sound (WebAudio) start without a click.
        webViewConfiguration.mediaTypesRequiringUserActionForPlayback = []
        webViewConfiguration.userContentController.add(self, name: "model3D")

        let createdWebView = FirstMouseWebView(frame: NSRect(origin: .zero, size: Self.maximumOverlaySize), configuration: webViewConfiguration)
        createdWebView.autoresizingMask = [.width, .height]
        createdWebView.navigationDelegate = self
        // Transparent so only the model shows over the desktop.
        createdWebView.setValue(false, forKey: "drawsBackground")
        createdPanel.contentView = createdWebView

        panel = createdPanel
        webView = createdWebView
    }
}

/// The overlay panel is never key, so without this the first click (the
/// start of an orbit drag) would only be spent activating the window.
private final class FirstMouseWebView: WKWebView {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool {
        true
    }
}
