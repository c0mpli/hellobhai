//
//  VisualExplanationPanel.swift
//  leanring-buddy
//
//  A floating panel near the cursor where Claude explains things live while
//  narrating: either a whiteboard it sketches on (drawings) or a notes page
//  it writes on (summaries, lists, key points). Both look like Clicky's
//  real-time graphics: cream grid paper, a warm title bar with the topic,
//  flat pastel shapes with one clean hand-drawn stroke, and the Patrick Hand
//  marker font. Claude streams one JSON command per line (headings, rows of
//  token boxes, boxes, arrows, force arrows, math shapes, notes…). Everything
//  is drawn live: strokes trace along their path, fills fade in after the
//  outline, and text writes left to right, while a small gold triangle pen
//  travels with the drawing head and a soft synthesized pencil scratch
//  plays. Icons are real vector drawings (Lucide), never emoji.
//
//  The web view is locked down: a CSP blocks all network loads, navigation
//  away from the page is refused, storage is non-persistent, and commands are
//  passed in as JSON data (parsed with JSON.parse, never evaluated as code).
//

import AppKit
import WebKit

@MainActor
final class VisualExplanationPanelManager: NSObject, WKNavigationDelegate {
    private static let panelContentSize = NSSize(width: 760, height: 580)

    /// Height of the warm title bar the page draws; the traffic lights sit in it.
    private static let titleBarHeight: CGFloat = 28

    private var panel: NSPanel?
    private var webView: WKWebView?

    /// Commands that arrive before the board page has finished loading.
    private var pendingDrawingCommandLines: [String] = []
    private var isBoardPageReady = false

    // MARK: - Bundled Assets

    /// Loaded once from the app bundle and inlined into the page so the panel
    /// never needs file or network access.
    private static func bundledResourceData(named resourceName: String, withExtension fileExtension: String) -> Data? {
        guard let resourceURL = Bundle.main.url(forResource: resourceName, withExtension: fileExtension),
              let resourceData = try? Data(contentsOf: resourceURL) else {
            print("⚠️ Visual panel: missing bundled \(resourceName).\(fileExtension)")
            return nil
        }
        return resourceData
    }

    private static let roughJSSource: String = bundledResourceData(named: "rough", withExtension: "js")
        .flatMap { String(data: $0, encoding: .utf8) } ?? ""

    /// Graph layout engine that positions diagram shapes so nothing overlaps.
    private static let dagreJSSource: String = bundledResourceData(named: "dagre.min", withExtension: "js")
        .flatMap { String(data: $0, encoding: .utf8) } ?? ""

    /// Math typesetting (fractions, integrals, matrices) for the board and notes.
    /// The CSS has all KaTeX fonts inlined as data URIs, so no network is needed.
    private static let katexJSSource: String = bundledResourceData(named: "katex.min", withExtension: "js")
        .flatMap { String(data: $0, encoding: .utf8) } ?? ""
    private static let katexCSSSource: String = bundledResourceData(named: "katex-inline", withExtension: "css")
        .flatMap { String(data: $0, encoding: .utf8) } ?? ""

    /// Syntax highlighter for code blocks in notes.
    private static let prismJSSource: String = bundledResourceData(named: "prism.min", withExtension: "js")
        .flatMap { String(data: $0, encoding: .utf8) } ?? ""

    /// Lucide icon shapes keyed by name, as compact arrays (see board renderer).
    private static let iconShapesJSON: String = bundledResourceData(named: "icons", withExtension: "json")
        .flatMap { String(data: $0, encoding: .utf8) } ?? "{}"

    /// Patrick Hand: the upright marker-style handwriting used on the board and notes.
    private static let handwritingFontBase64: String = bundledResourceData(named: "PatrickHand", withExtension: "woff2")?
        .base64EncodedString() ?? ""

    /// Icon names Claude may use, listed in its drawing instructions.
    static let availableIconNames: [String] = {
        guard let iconShapesData = iconShapesJSON.data(using: .utf8),
              let iconShapesByName = try? JSONSerialization.jsonObject(with: iconShapesData) as? [String: Any] else {
            return []
        }
        return iconShapesByName.keys.sorted()
    }()

    // MARK: - Drawing API

    /// What the panel is showing: a whiteboard sketch or a notes page.
    enum LiveContentKind {
        case drawing
        case notes
    }

    /// Opens a blank whiteboard or notes page beside the cursor, ready for
    /// streamed commands.
    func beginLiveContent(_ liveContentKind: LiveContentKind) {
        presentPanelNearCursor()
        isBoardPageReady = false
        pendingDrawingCommandLines.removeAll()
        let pageHTML = liveContentKind == .drawing ? Self.boardPageHTML : Self.notesPageHTML
        webView?.loadHTMLString(pageHTML, baseURL: nil)
    }

    /// Draws or writes one command line. Lines that aren't JSON objects, like
    /// stray prose or code fences, are ignored by the page.
    func addCommandLine(_ drawingCommandLine: String) {
        guard isBoardPageReady else {
            pendingDrawingCommandLines.append(drawingCommandLine)
            return
        }
        callBoardFunction("add", withStringArgument: drawingCommandLine)
    }

    /// Tells the page every step is shown, so the pen can rest out of view.
    func finishLiveContent() {
        guard isBoardPageReady else {
            pendingDrawingCommandLines.append(Self.finishedDrawingMarker)
            return
        }
        webView?.evaluateJavaScript("hb.done()")
    }

    /// Reveals whatever the pen hasn't drawn yet at once and stops the pen
    /// and its scratch sound (the hotkey was pressed, so the user is talking).
    func stopDrawing() {
        guard isBoardPageReady else { return }
        webView?.evaluateJavaScript("hb.stopDrawing()")
    }

    func close() {
        // Stop the pen first: once hidden, the page stops animating, and only
        // its animation loop would otherwise silence a scratch mid-stroke.
        stopDrawing()
        panel?.orderOut(nil)
    }

    private static let finishedDrawingMarker = "__hellobhai_done__"

    /// Passes the line as a JSON-encoded string literal so its contents can
    /// never break out into executable JavaScript.
    private func callBoardFunction(_ boardFunctionName: String, withStringArgument stringArgument: String) {
        guard let encodedArgumentData = try? JSONEncoder().encode(stringArgument),
              let encodedArgument = String(data: encodedArgumentData, encoding: .utf8) else { return }
        webView?.evaluateJavaScript("hb.\(boardFunctionName)(\(encodedArgument))")
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        isBoardPageReady = true
        let queuedDrawingCommandLines = pendingDrawingCommandLines
        pendingDrawingCommandLines.removeAll()
        for queuedLine in queuedDrawingCommandLines {
            if queuedLine == Self.finishedDrawingMarker {
                webView.evaluateJavaScript("hb.done()")
            } else {
                callBoardFunction("add", withStringArgument: queuedLine)
            }
        }
    }

    /// Only the initial in-memory page load is allowed; nothing on the board
    /// can navigate anywhere.
    func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        let isInitialPageLoad = navigationAction.request.url?.absoluteString == "about:blank"
        decisionHandler(isInitialPageLoad ? .allow : .cancel)
    }

    // MARK: - Panel

    private func presentPanelNearCursor() {
        if panel == nil {
            createPanel()
        }
        guard let panel else { return }
        // Re-place it beside the cursor for every new visual, since that's
        // where the user is looking right now.
        panel.setFrameOrigin(Self.panelOriginNearCursor(panelSize: panel.frame.size))
        panel.orderFrontRegardless()
    }

    private func createPanel() {
        let createdPanel = NSPanel(
            contentRect: NSRect(origin: .zero, size: Self.panelContentSize),
            styleMask: [.titled, .closable, .resizable, .nonactivatingPanel, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        createdPanel.title = "hellobhai"
        // The page draws its own title bar with the topic; only the red close
        // and green zoom lights are kept, and they show on hover.
        createdPanel.titlebarAppearsTransparent = true
        createdPanel.titleVisibility = .hidden
        createdPanel.standardWindowButton(.miniaturizeButton)?.isHidden = true
        setTrafficLightsVisible(false, in: createdPanel)
        createdPanel.isFloatingPanel = true
        createdPanel.level = .floating
        createdPanel.hidesOnDeactivate = false
        createdPanel.becomesKeyOnlyIfNeeded = true
        createdPanel.isReleasedWhenClosed = false
        // Resize freely; the canvas refits to any shape, but keep it usable.
        createdPanel.minSize = NSSize(width: 360, height: 260)
        createdPanel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        // Cream paper, so resizing never flashes a different color.
        createdPanel.backgroundColor = NSColor(red: 0.973, green: 0.961, blue: 0.925, alpha: 1)

        let webViewConfiguration = WKWebViewConfiguration()
        // Nothing drawn here should ever be persisted to disk.
        webViewConfiguration.websiteDataStore = .nonPersistent()
        // The pencil scratch is synthesized with WebAudio and has to start
        // without a click on the page.
        webViewConfiguration.mediaTypesRequiringUserActionForPlayback = []

        let containerView = PanelHoverTrackingView(frame: NSRect(origin: .zero, size: Self.panelContentSize))
        containerView.onHoverChanged = { [weak self, weak createdPanel] isHovering in
            guard let self, let createdPanel else { return }
            self.setTrafficLightsVisible(isHovering, in: createdPanel)
        }

        let createdWebView = WKWebView(frame: containerView.bounds, configuration: webViewConfiguration)
        createdWebView.autoresizingMask = [.width, .height]
        createdWebView.navigationDelegate = self
        createdWebView.setValue(false, forKey: "drawsBackground")
        containerView.addSubview(createdWebView)

        // The web view would swallow clicks on the page-drawn title bar, so a
        // transparent strip on top of it hands them to the window for dragging.
        let windowDragStrip = WindowDragStripView(frame: NSRect(
            x: 0,
            y: containerView.bounds.height - Self.titleBarHeight,
            width: containerView.bounds.width,
            height: Self.titleBarHeight
        ))
        windowDragStrip.autoresizingMask = [.width, .minYMargin]
        containerView.addSubview(windowDragStrip)

        createdPanel.contentView = containerView

        panel = createdPanel
        webView = createdWebView
    }

    private func setTrafficLightsVisible(_ isVisible: Bool, in targetPanel: NSPanel) {
        for windowButtonType in [NSWindow.ButtonType.closeButton, .zoomButton] {
            targetPanel.standardWindowButton(windowButtonType)?.animator().alphaValue = isVisible ? 1 : 0
        }
    }

    /// To the right of the cursor and vertically centered on it, flipping to
    /// the cursor's left only when there isn't room; kept on the cursor's screen.
    private static func panelOriginNearCursor(panelSize: NSSize) -> NSPoint {
        let mouseLocation = NSEvent.mouseLocation
        let cursorScreen = NSScreen.screens.first(where: { $0.frame.contains(mouseLocation) }) ?? NSScreen.main
        let visibleFrame = cursorScreen?.visibleFrame ?? NSRect(origin: .zero, size: panelSize)
        let gapFromCursor: CGFloat = 40

        let fitsRightOfCursor = mouseLocation.x + gapFromCursor + panelSize.width <= visibleFrame.maxX - 12
        var panelOrigin = NSPoint(
            x: fitsRightOfCursor ? mouseLocation.x + gapFromCursor : mouseLocation.x - gapFromCursor - panelSize.width,
            y: mouseLocation.y - panelSize.height / 2
        )
        panelOrigin.x = min(max(panelOrigin.x, visibleFrame.minX + 12), visibleFrame.maxX - panelSize.width - 12)
        panelOrigin.y = min(max(panelOrigin.y, visibleFrame.minY + 12), visibleFrame.maxY - panelSize.height - 12)
        return panelOrigin
    }

    // MARK: - Board Page

    private static var boardPageHTML: String {
        """
        <!doctype html>
        <html>
        <head>
        <meta charset="utf-8">
        <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; font-src data:">
        <style>
        @font-face { font-family: "Patrick Hand"; src: url(data:font/woff2;base64,\(handwritingFontBase64)) format("woff2"); }
        \(sharedPaperStylesheet)
        \(boardStylesheet)
        \(katexCSSSource)
        </style>
        <script>\(roughJSSource)</script>
        <script>\(dagreJSSource)</script>
        <script>\(katexJSSource)</script>
        <script>const ICON_SHAPES = \(iconShapesJSON);</script>
        </head>
        <body>
        <div id="titlebar"><span id="topic"></span></div>
        <svg id="board" viewBox="0 0 800 500" preserveAspectRatio="xMidYMid meet">
          <g id="zone-layer"></g>
          <g id="edge-layer"></g>
          <g id="main-layer"></g>
          <g id="title-layer"></g>
          <g id="motion-layer"></g>
        </svg>
        <div id="formula-layer"></div>
        \(penElementHTML)
        <script>\(penAndScratchSoundScript)</script>
        <script>\(boardRendererScript)</script>
        </body>
        </html>
        """
    }

    /// Cream grid paper, the warm title bar with the centered lowercase topic,
    /// the gold pen, and the drawing indicator: shared by the board and notes.
    private static let sharedPaperStylesheet = """
    /* Barely-visible square grid. The board moves it with its camera (see
       moveGridWithCamera) so the drawing never slides over a static grid. */
    html, body { margin: 0; height: 100%; overflow: hidden; background-color: #f8f5ec;
      background-image: linear-gradient(rgba(84,72,40,0.04) 1px, transparent 1px),
        linear-gradient(90deg, rgba(84,72,40,0.04) 1px, transparent 1px);
      background-size: 17px 17px; background-position: -1px 5px; }
    #titlebar { position: absolute; top: 0; left: 0; right: 0; height: 28px; z-index: 5;
      background: linear-gradient(#e9e6dc, #e1ded3); border-bottom: 1px solid #d6d2c5;
      display: flex; align-items: center; justify-content: center;
      font: 13px -apple-system, "SF Pro Text", system-ui, sans-serif; color: #8a8578; letter-spacing: 0.1px;
      -webkit-user-select: none; cursor: default; }
    #topic { max-width: 70%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    #pen { position: fixed; left: 0; top: 0; width: 17px; height: 17px; z-index: 20; pointer-events: none;
      opacity: 0; transition: opacity 0.5s; will-change: transform;
      filter: drop-shadow(0 1.5px 3px rgba(90,70,20,0.32)) drop-shadow(0 0 5px rgba(201,166,70,0.25)); }
    /* Shows at once when drawing starts; only resting fades it out. */
    #pen.visible { opacity: 1; transition: none; }
    """

    /// The gold rounded "play" triangle that travels with the drawing head.
    /// Shared with the 3D model overlay so both pens look the same.
    static let penTriangleSVGPath = """
    <path d="M4.4 2.2 Q2.8 1.5 2.8 3.3 V12.7 Q2.8 14.5 4.4 13.8 L13.4 9.1 Q14.9 8 13.4 6.9 Z" fill="#cfa83a"/>
    """

    private static let penElementHTML = """
    <svg id="pen" viewBox="0 0 16 16">\(penTriangleSVGPath)</svg>
    """

    private static let boardStylesheet = """
    #board { position: absolute; inset: 28px 0 0 0; width: 100%; height: calc(100% - 28px); cursor: grab; touch-action: none; }
    #board.panning { cursor: grabbing; }
    /* KaTeX formulas live in HTML over the board (see positionFormulaOverlays). */
    #formula-layer { position: absolute; inset: 28px 0 0 0; overflow: hidden; pointer-events: none; z-index: 1; }
    #formula-layer > div { position: absolute; left: 0; top: 0; transform-origin: 0 0; white-space: nowrap; }
    /* Free labels get a paper-colored halo so they stay readable over lines. */
    text.halo { stroke: #f8f5ec; stroke-width: 5px; stroke-linejoin: round; paint-order: stroke; }
    /* The heading is bold marker: Patrick Hand has one weight, so the ink is
       thickened with a same-colored outline, and letter-spacing undoes the
       crowding the outline adds. */
    text.heading { stroke: #2a2a2a; stroke-width: 1.5px; stroke-linejoin: round; paint-order: normal; letter-spacing: 0.6px; }
    .hb-pulse { transform-box: fill-box; transform-origin: center; animation: hb-pulse 1.4s 3 ease-in-out; }
    .hb-shake { animation: hb-shake 0.6s ease-in-out 2; }
    .hb-glow { filter: drop-shadow(0 0 5px #e9c84a) drop-shadow(0 0 2px #e9c84a); }
    @keyframes hb-pulse { 0%,100% { transform: scale(1); filter: none; } 50% { transform: scale(1.05); filter: drop-shadow(0 0 8px #e3c35a); } }
    @keyframes hb-shake { 0%,100% { transform: translateX(0); } 20% { transform: translateX(-6px); } 40% { transform: translateX(6px); } 60% { transform: translateX(-4px); } 80% { transform: translateX(4px); } }
    """

    /// Defines `scratchSound`, the soft pencil-on-paper scratch the pen makes.
    /// Shared with the 3D model overlay so both pens sound the same.
    static let pencilScratchSoundScript = """
    // Soft pencil-on-paper scratch: looping white noise through a band-pass
    // filter, with its loudness jittered every frame while a stroke draws and
    // short ticks while text writes. Silent while the pen glides or idles.
    const scratchSound = (() => {
      let audioContext = null, noiseGain = null, noiseFilter = null;
      function ensureAudio() {
        if (audioContext) { return true; }
        try {
          audioContext = new (window.AudioContext || window.webkitAudioContext)();
          const noiseBuffer = audioContext.createBuffer(1, audioContext.sampleRate * 2, audioContext.sampleRate);
          const noiseSamples = noiseBuffer.getChannelData(0);
          for (let sampleIndex = 0; sampleIndex < noiseSamples.length; sampleIndex += 1) { noiseSamples[sampleIndex] = Math.random() * 2 - 1; }
          const noiseSource = audioContext.createBufferSource();
          noiseSource.buffer = noiseBuffer;
          noiseSource.loop = true;
          noiseFilter = audioContext.createBiquadFilter();
          noiseFilter.type = 'bandpass';
          noiseFilter.frequency.value = 2600;
          noiseFilter.Q.value = 0.9;
          noiseGain = audioContext.createGain();
          noiseGain.gain.value = 0;
          noiseSource.connect(noiseFilter);
          noiseFilter.connect(noiseGain);
          noiseGain.connect(audioContext.destination);
          noiseSource.start();
          return true;
        } catch (audioError) { audioContext = null; return false; }
      }
      function wakeAudio() {
        if (!ensureAudio()) { return false; }
        if (audioContext.state === 'suspended') { audioContext.resume().catch(() => {}); }
        return true;
      }
      return {
        scratchWhileStroking() {
          if (!wakeAudio()) { return; }
          const now = audioContext.currentTime;
          noiseGain.gain.setTargetAtTime(0.05 * (0.5 + Math.random() * 0.5), now, 0.015);
          noiseFilter.frequency.setTargetAtTime(1900 + Math.random() * 1500, now, 0.03);
        },
        tickWhileWriting() {
          if (!wakeAudio()) { return; }
          const now = audioContext.currentTime;
          noiseGain.gain.cancelScheduledValues(now);
          noiseGain.gain.setValueAtTime(noiseGain.gain.value, now);
          noiseGain.gain.linearRampToValueAtTime(0.03 + Math.random() * 0.025, now + 0.008);
          noiseGain.gain.setTargetAtTime(0, now + 0.014, 0.012);
          noiseFilter.frequency.setTargetAtTime(2600 + Math.random() * 1400, now, 0.01);
        },
        silence() {
          if (!audioContext) { return; }
          noiseGain.gain.setTargetAtTime(0, audioContext.currentTime, 0.03);
        },
        // Cuts the sound at once and parks the audio clock. Needed when the
        // page stops animating (hidden panel): only the animation loop would
        // otherwise ramp the gain back down, so a hiss would keep playing.
        // The next scratch or tick wakes the context again.
        stopImmediately() {
          if (!audioContext) { return; }
          noiseGain.gain.cancelScheduledValues(0);
          noiseGain.gain.value = 0;
          audioContext.suspend().catch(() => {});
        }
      };
    })();
    document.addEventListener('visibilitychange', () => { if (document.hidden) { scratchSound.stopImmediately(); } });
    """

    /// The live-drawing engine shared by the board and notes. Elements handed
    /// to queuePenLeaves start hidden and are revealed one at a time, in
    /// order: strokes trace along their path (duration proportional to their
    /// length), fills fade in after their outline, and text writes left to
    /// right with a clip reveal. The gold triangle pen follows the drawing
    /// head and glides between elements, and a WebAudio pencil scratch plays
    /// only while the pen is actually drawing.
    private static let penAndScratchSoundScript = """
    const penElement = document.getElementById('pen');
    const pendingPenLeaves = [];
    const penPosition = { x: 0, y: 0, hasBeenPlaced: false };
    // Seconds since the previous animation frame; the pen program reads it.
    let penFrameDelta = 0;
    let penIdleSeconds = 0;
    // Set by the page once its stream has ended, so the pen can rest out of view.
    let isPenStreamFinished = false;

    \(pencilScratchSoundScript)

    // How a leaf is revealed: 'stroke' traces along its outline, 'fill'
    // fades in, 'write' unclips left to right.
    function penLeafKind(leafElement) {
      const tagName = leafElement.tagName.toLowerCase();
      if (['path', 'line', 'polyline', 'polygon', 'circle', 'ellipse', 'rect'].includes(tagName)) {
        const fillValue = leafElement.getAttribute('fill');
        const strokeValue = leafElement.getAttribute('stroke');
        const hasStroke = strokeValue && strokeValue !== 'none' && strokeValue !== 'transparent';
        const hasFill = fillValue && fillValue !== 'none' && fillValue !== 'transparent';
        if (hasStroke && !hasFill && typeof leafElement.getTotalLength === 'function') { return 'stroke'; }
        return 'fill';
      }
      return 'write';
    }

    // Where the text itself is on screen. A block element (a notes heading,
    // a table) is as wide as the page, but its text may end much sooner.
    function screenRectOfText(leafElement) {
      if (leafElement instanceof HTMLElement) {
        const textRange = document.createRange();
        textRange.selectNodeContents(leafElement);
        const textRect = textRange.getBoundingClientRect();
        if (textRect.width > 0) { return textRect; }
      }
      return leafElement.getBoundingClientRect();
    }

    function setWriteClip(leafElement, revealedFraction) {
      let hiddenRight = `${(1 - revealedFraction) * 100}%`;
      // Nothing written yet: hide all of it (bullet dots, card edges too).
      if (revealedFraction > 0 && revealedFraction < 1 && leafElement instanceof HTMLElement && leafElement.isConnected) {
        // In the element's own pixels (it may be scaled), up to the text's right edge.
        const leafRect = leafElement.getBoundingClientRect(), textRect = screenRectOfText(leafElement);
        const elementScale = leafElement.offsetWidth ? leafRect.width / leafElement.offsetWidth : 1;
        hiddenRight = `${Math.max(0, (leafRect.right - (textRect.left + textRect.width * revealedFraction)) / (elementScale || 1))}px`;
      }
      const clipValue = revealedFraction >= 1 ? '' : `inset(-30% ${hiddenRight} -30% -4%)`;
      leafElement.style.clipPath = clipValue;
      leafElement.style.webkitClipPath = clipValue;
    }

    // Hides the leaves right away and queues them for the pen. Within each
    // shape (leaves sharing a parent) fills wait until its outline is drawn.
    function queuePenLeaves(leafElements) {
      const orderedLeaves = [];
      let currentRun = [];
      const flushRun = () => {
        orderedLeaves.push(...currentRun.filter((leaf) => leaf.penKind !== 'fill'), ...currentRun.filter((leaf) => leaf.penKind === 'fill'));
        currentRun = [];
      };
      for (const leafElement of leafElements) {
        leafElement.setAttribute('data-pen', 'queued');
        leafElement.penKind = penLeafKind(leafElement);
        if (leafElement.penKind === 'stroke') {
          const strokeLength = leafElement.getTotalLength() || 1;
          leafElement.penStrokeLength = strokeLength;
          leafElement.style.strokeDasharray = `${strokeLength} ${strokeLength + 10}`;
          leafElement.style.strokeDashoffset = strokeLength;
        } else if (leafElement.penKind === 'fill') {
          leafElement.style.opacity = 0;
        } else {
          setWriteClip(leafElement, 0);
        }
        if (currentRun.length && currentRun[0].parentNode !== leafElement.parentNode) { flushRun(); }
        currentRun.push(leafElement);
      }
      flushRun();
      pendingPenLeaves.push(...orderedLeaves);
    }

    function revealLeafInstantly(leafElement) {
      leafElement.style.strokeDasharray = '';
      leafElement.style.strokeDashoffset = '';
      leafElement.style.opacity = '';
      setWriteClip(leafElement, 1);
      leafElement.setAttribute('data-pen', 'done');
    }

    // When a step falls behind its narration, the pen speeds up to catch up.
    function penSpeedMultiplier() {
      return Math.min(1 + Math.max(0, pendingPenLeaves.length - 8) * 0.07, 4);
    }

    function screenPointOnStroke(leafElement, lengthAlongStroke) {
      const localPoint = leafElement.getPointAtLength(lengthAlongStroke);
      const screenMatrix = leafElement.getScreenCTM();
      if (!screenMatrix) { return [penPosition.x, penPosition.y]; }
      return [screenMatrix.a * localPoint.x + screenMatrix.c * localPoint.y + screenMatrix.e,
        screenMatrix.b * localPoint.x + screenMatrix.d * localPoint.y + screenMatrix.f];
    }

    // The right edge of the part of a text that's written so far.
    function screenPointOfWritingHead(leafElement, revealedFraction) {
      const leafRect = screenRectOfText(leafElement);
      const lineHeight = Math.min(leafRect.height, 30);
      return [leafRect.left + leafRect.width * revealedFraction, leafRect.top + lineHeight * 0.62];
    }

    function placePen(screenX, screenY) {
      penPosition.x = screenX; penPosition.y = screenY; penPosition.hasBeenPlaced = true;
      // The triangle sits a little right of and below the drawing head, like a nib.
      penElement.style.transform = `translate(${screenX + 3}px, ${screenY - 1}px)`;
      penElement.classList.add('visible');
    }

    function* glidePenTo(targetPointProvider) {
      const [targetX, targetY] = targetPointProvider();
      if (!penPosition.hasBeenPlaced) { placePen(targetX, targetY); return; }
      const startX = penPosition.x, startY = penPosition.y;
      const glideDistance = Math.hypot(targetX - startX, targetY - startY);
      if (glideDistance < 3) { return; }
      const glideSeconds = Math.min(Math.max(glideDistance / 1500, 0.08), 0.32);
      scratchSound.silence();
      let elapsedSeconds = 0;
      while (elapsedSeconds < glideSeconds) {
        elapsedSeconds += penFrameDelta * penSpeedMultiplier();
        const progress = Math.min(elapsedSeconds / glideSeconds, 1);
        const easedProgress = progress < 0.5 ? 2 * progress * progress : 1 - Math.pow(-2 * progress + 2, 2) / 2;
        // Re-read the target every frame: the camera or layout may be moving.
        const [currentTargetX, currentTargetY] = targetPointProvider();
        placePen(startX + (currentTargetX - startX) * easedProgress, startY + (currentTargetY - startY) * easedProgress);
        yield;
      }
    }

    // Fills fading in after their outline. They fade on the pen's own clock
    // (not a CSS transition) while the pen moves on to the next stroke.
    const fadingFills = [];
    function advanceFadingFills() {
      for (let fadeIndex = fadingFills.length - 1; fadeIndex >= 0; fadeIndex -= 1) {
        const fade = fadingFills[fadeIndex];
        fade.elapsedSeconds += penFrameDelta;
        const fadeProgress = Math.min(fade.elapsedSeconds / 0.35, 1);
        fade.leafElement.style.opacity = fadeProgress;
        if (fadeProgress >= 1) { revealLeafInstantly(fade.leafElement); fadingFills.splice(fadeIndex, 1); }
      }
    }

    function* drawLeafWithPen(leafElement) {
      if (leafElement.penKind === 'fill') {
        fadingFills.push({ leafElement, elapsedSeconds: 0 });
        return;
      }
      if (leafElement.penKind === 'stroke') {
        const strokeLength = leafElement.penStrokeLength;
        yield* glidePenTo(() => screenPointOnStroke(leafElement, 0));
        const screenMatrix = leafElement.getScreenCTM();
        const screenScale = screenMatrix ? Math.hypot(screenMatrix.a, screenMatrix.b) : 1;
        const strokeSeconds = Math.min(Math.max(strokeLength * screenScale / 620, 0.12), 1.3);
        let elapsedSeconds = 0;
        while (elapsedSeconds < strokeSeconds) {
          if (!leafElement.isConnected) { return; }
          elapsedSeconds += penFrameDelta * penSpeedMultiplier();
          const progress = Math.min(elapsedSeconds / strokeSeconds, 1);
          leafElement.style.strokeDashoffset = strokeLength * (1 - progress);
          placePen(...screenPointOnStroke(leafElement, strokeLength * progress));
          scratchSound.scratchWhileStroking();
          yield;
        }
        revealLeafInstantly(leafElement);
        return;
      }
      yield* glidePenTo(() => screenPointOfWritingHead(leafElement, 0));
      const characterCount = Math.max(String(leafElement.textContent || '').trim().length, 1);
      const writeSeconds = Math.min(Math.max(characterCount * 0.05, 0.25), 1.8);
      let elapsedSeconds = 0, lastTickedCharacter = -1;
      scratchSound.silence();
      while (elapsedSeconds < writeSeconds) {
        if (!leafElement.isConnected) { return; }
        elapsedSeconds += penFrameDelta * penSpeedMultiplier();
        const progress = Math.min(elapsedSeconds / writeSeconds, 1);
        setWriteClip(leafElement, progress);
        placePen(...screenPointOfWritingHead(leafElement, progress));
        // One soft tick every couple of letters.
        const currentCharacter = Math.floor(progress * characterCount / 2);
        if (currentCharacter !== lastTickedCharacter) { lastTickedCharacter = currentCharacter; scratchSound.tickWhileWriting(); }
        yield;
      }
      revealLeafInstantly(leafElement);
    }

    // Per-frame work that runs on the pen's clock (the board's shape glides),
    // so it advances exactly when the pen does.
    const penFrameCallbacks = [];
    function runPenFrameCallbacks() { penFrameCallbacks.forEach((frameCallback) => frameCallback(penFrameDelta)); }
    // Set by a page that needs to know when the pen starts on a leaf (notes
    // scroll it into view).
    let onPenLeafStart = null;

    let currentPenLeaf = null, currentLeafAnimation = null;
    // Hotkey or close: everything still queued appears at once, and the
    // pen and its sound stop.
    function stopPenDrawing() {
      pendingPenLeaves.splice(0).forEach(revealLeafInstantly);
      fadingFills.splice(0).forEach((fade) => revealLeafInstantly(fade.leafElement));
      if (currentPenLeaf) { revealLeafInstantly(currentPenLeaf); }
      currentPenLeaf = null;
      currentLeafAnimation = null;
      scratchSound.stopImmediately();
      penElement.classList.remove('visible');
    }

    function* penProgram() {
      while (true) {
        runPenFrameCallbacks();
        if (!pendingPenLeaves.length && !currentLeafAnimation) {
          advanceFadingFills();
          scratchSound.silence();
          penIdleSeconds += penFrameDelta;
          // Fade out once the last element is done, so it never sits parked in empty space.
          if (penIdleSeconds > (isPenStreamFinished ? 0.6 : 1.4)) { penElement.classList.remove('visible'); }
          yield;
          continue;
        }
        penIdleSeconds = 0;
        if (!currentLeafAnimation) {
          // Shapes redrawn by a relayout replace their old elements, so
          // disconnected leaves are skipped.
          while (pendingPenLeaves.length && !pendingPenLeaves[0].isConnected) { pendingPenLeaves.shift(); }
          if (!pendingPenLeaves.length) { yield; continue; }
          currentPenLeaf = pendingPenLeaves.shift();
          if (onPenLeafStart) { onPenLeafStart(currentPenLeaf); }
          currentLeafAnimation = drawLeafWithPen(currentPenLeaf);
        }
        advanceFadingFills();
        let animationStep;
        try { animationStep = currentLeafAnimation.next(); } catch (leafError) { console.log(leafError); revealLeafInstantly(currentPenLeaf); animationStep = { done: true }; }
        if (animationStep.done) { currentLeafAnimation = null; currentPenLeaf = null; }
        yield;
      }
    }

    function isPenBusy() { return pendingPenLeaves.length > 0 || currentLeafAnimation !== null || penIdleSeconds === 0; }

    const penAnimation = penProgram();
    let previousPenFrameTime = null;
    function runPenFrame(frameTime) {
      penFrameDelta = previousPenFrameTime === null ? 0 : Math.min((frameTime - previousPenFrameTime) / 1000, 0.1);
      previousPenFrameTime = frameTime;
      try { penAnimation.next(); } catch (penError) { console.log(penError); }
      requestAnimationFrame(runPenFrame);
    }
    requestAnimationFrame(runPenFrame);

    function setPanelTopic(topicText) {
      document.getElementById('topic').textContent = String(topicText || '').toLowerCase();
    }
    """

    /// Draws handed-over commands one at a time with a short gap, so even a
    /// burst of commands still looks like it's being sketched live. Claude
    /// declares shapes and arrows without coordinates; dagre lays the graph
    /// out after every addition and existing shapes glide to their new spots.
    private static let boardRendererScript = """
    const SVG_NS = 'http://www.w3.org/2000/svg';
    const board = document.getElementById('board');
    const zoneLayer = document.getElementById('zone-layer');
    const edgeLayer = document.getElementById('edge-layer');
    const mainLayer = document.getElementById('main-layer');
    // Things that move forever (flows) live here so they never affect auto-zoom.
    const motionLayer = document.getElementById('motion-layer');
    // Title and captions sit here, positioned from the bounds of everything else.
    const titleLayer = document.getElementById('title-layer');
    const roughCanvas = rough.svg(board, { options: { roughness: 0.45, bowing: 0.5, disableMultiStroke: true, disableMultiStrokeFill: true, preserveVertices: true } });
    const HAND_FONT = '"Patrick Hand", "Chalkboard SE", system-ui, sans-serif';
    const INK = '#2a2a2a';
    const MUTED_INK = '#8a8578';
    const PAPER = '#f8f5ec';
    // Clicky's flat pastels: a light fill, a mid outline for filled shapes,
    // and a strong ink for lines, curves, force arrows, and colored text.
    const PALETTE = {
      blue:   { fill: '#d6e4f5', outline: '#6fa0e3', ink: '#2f5fc4' },
      // The saturated blue Clicky uses for the key parts of technical drawings.
      bluestrong: { fill: '#a9c8ef', outline: '#3c6fd0', ink: '#2f5fc4' },
      pink:   { fill: '#f6d5d0', outline: '#e39a90', ink: '#c9604f' },
      green:  { fill: '#d5efd5', outline: '#8fcf8f', ink: '#3f9a4a' },
      gray:   { fill: '#dcdcd8', outline: '#8d8d8d', ink: '#888888' },
      steel:  { fill: '#b4b4b1', outline: '#858583', ink: '#6a6a68' },
      purple: { fill: '#b9a6f5', outline: '#7c5ce6', ink: '#6a4bd8' },
      yellow: { fill: '#f8ecc0', outline: '#e3c35a', ink: '#c99a1e' },
      orange: { fill: '#fbe2c2', outline: '#e9a23b', ink: '#e9a23b' },
      red:    { fill: '#f6d0cf', outline: '#e07a7a', ink: '#d14b4b' },
      cyan:   { fill: '#d2eef0', outline: '#6cc4cc', ink: '#2a9aa6' }
    };
    const COLORS = { fg: INK, muted: MUTED_INK, gold: '#d4a72c' };
    for (const [paletteName, paletteEntry] of Object.entries(PALETTE)) { COLORS[paletteName] = paletteEntry.ink; }
    // One clean, very slightly wobbly stroke (no rough.js multi-stroke sketchiness).
    const CLEAN_STROKE = { roughness: 0.45, bowing: 0.5, disableMultiStroke: true, disableMultiStrokeFill: true, preserveVertices: true };
    const ARROW_INK = '#3a3a3a';
    const GLIDE_SECONDS = 0.45;

    const layoutGraph = new dagre.graphlib.Graph({ compound: true, multigraph: true });
    layoutGraph.setGraph({ rankdir: 'LR', nodesep: 44, ranksep: 90, edgesep: 24, marginx: 20, marginy: 20 });
    layoutGraph.setDefaultEdgeLabel(() => ({}));

    const nodesById = {};
    const groupsById = {};
    const edgesById = {};
    const edgeStepBadges = [];
    const captionElements = [];
    let titleElement = null;
    let edgeRedrawTimer = null;

    const commandQueue = [];
    let isDrawingCommand = false;
    let isStreamFinished = false;

    function colorOf(colorName, fallbackName) { return COLORS[colorName] || COLORS[fallbackName] || COLORS.fg; }
    function fillOf(colorName, fallbackName) { return (PALETTE[colorName] || PALETTE[fallbackName] || PALETTE.gray).fill; }
    function outlineOf(colorName, fallbackName) { return (PALETTE[colorName] || PALETTE[fallbackName] || PALETTE.gray).outline; }
    function translucent(hexColor, alpha) {
      const red = parseInt(hexColor.slice(1, 3), 16), green = parseInt(hexColor.slice(3, 5), 16), blue = parseInt(hexColor.slice(5, 7), 16);
      return `rgba(${red},${green},${blue},${alpha})`;
    }
    function svgElement(tagName, attributes) {
      const element = document.createElementNS(SVG_NS, tagName);
      for (const [attributeName, attributeValue] of Object.entries(attributes || {})) { element.setAttribute(attributeName, attributeValue); }
      return element;
    }
    // Same text → same rough.js seed, so a redrawn arrow keeps its exact wobble.
    function stableSeed(text) {
      let hash = 7;
      for (const character of String(text)) { hash = (hash * 31 + character.charCodeAt(0)) | 0; }
      return (Math.abs(hash) % 2147483000) + 1;
    }

    // Hands everything drawn since the last call to the pen, in drawing
    // order. Elements inside a container marked data-pen="done" (arrows and
    // groups redrawn by a relayout) appear instantly instead.
    const PEN_LEAF_SELECTOR = 'path, line, polyline, polygon, circle, ellipse, rect, text, foreignObject';
    function queueUnrevealedBoardElements() {
      const newLeaves = [];
      for (const layer of [zoneLayer, edgeLayer, mainLayer, titleLayer]) {
        layer.querySelectorAll(PEN_LEAF_SELECTOR).forEach((leafElement) => {
          if (leafElement.closest('[data-pen]')) { return; }
          const enclosingFormula = leafElement.parentElement && leafElement.parentElement.closest('foreignObject');
          if (enclosingFormula) { return; }
          newLeaves.push(leafElement);
        });
      }
      if (newLeaves.length) { queuePenLeaves(newLeaves); }
    }

    function wrapIntoLines(text, maxCharactersPerLine) {
      const words = String(text).split(/\\s+/);
      const lines = [];
      let currentLine = '';
      for (const word of words) {
        if (currentLine && (currentLine + ' ' + word).length > maxCharactersPerLine) { lines.push(currentLine); currentLine = word; }
        else { currentLine = currentLine ? currentLine + ' ' + word : word; }
      }
      if (currentLine) { lines.push(currentLine); }
      return lines.slice(0, 4);
    }

    function addText(parentElement, x, centerY, text, options) {
      const { size = 22, color = 'fg', anchor = 'middle', maxCharacters = 0, fill } = options || {};
      const lines = maxCharacters ? wrapIntoLines(text, maxCharacters) : [String(text)];
      const lineHeight = size * 1.15;
      const textElement = svgElement('text', { x, y: centerY - (lines.length - 1) * lineHeight / 2 + size * 0.15,
        'text-anchor': anchor, 'dominant-baseline': 'middle', 'font-size': size, 'font-family': HAND_FONT,
        fill: fill || colorOf(color, 'fg') });
      lines.forEach((lineText, lineIndex) => {
        const lineElement = svgElement('tspan', { x, dy: lineIndex === 0 ? 0 : lineHeight });
        lineElement.textContent = lineText;
        textElement.appendChild(lineElement);
      });
      // Labels floating on the board (not inside a shape) get a paper halo,
      // and keep a minimum size on screen when the camera zooms out.
      if (parentElement === mainLayer || parentElement === edgeLayer) {
        textElement.classList.add('halo');
        textElement.baseFontSize = size;
        applyMinimumLabelSize(textElement);
      }
      parentElement.appendChild(textElement);
      // Free-standing labels on the board never land on top of each other.
      if (parentElement === mainLayer) { moveLabelToFreeSpot(textElement); }
      return textElement;
    }

    // Bounding boxes of labels already placed on the board, so new labels can
    // find a free spot instead of overlapping them.
    const placedLabelBoxes = [];
    function boxesOverlap(firstBox, secondBox, margin) {
      return firstBox.x < secondBox.x + secondBox.width + margin && secondBox.x < firstBox.x + firstBox.width + margin
        && firstBox.y < secondBox.y + secondBox.height + margin && secondBox.y < firstBox.y + firstBox.height + margin;
    }
    // Tries nearby positions (spiralling out) until the label overlaps no
    // other label, then moves it there by shifting its x/y attributes (a
    // transform would be overridden by the appear animation).
    function moveLabelToFreeSpot(textElement) {
      const originalBox = textElement.getBBox();
      if (!originalBox.width) { return; }
      const candidateOffsets = [[0, 0]];
      for (let ringIndex = 1; ringIndex <= 7; ringIndex += 1) {
        const offsetDistance = ringIndex * 13;
        candidateOffsets.push([0, offsetDistance], [0, -offsetDistance], [offsetDistance, 0], [-offsetDistance, 0],
          [offsetDistance, offsetDistance], [-offsetDistance, offsetDistance], [offsetDistance, -offsetDistance], [-offsetDistance, -offsetDistance]);
      }
      for (const [offsetX, offsetY] of candidateOffsets) {
        const candidateBox = { x: originalBox.x + offsetX, y: originalBox.y + offsetY, width: originalBox.width, height: originalBox.height };
        if (placedLabelBoxes.some((placedBox) => boxesOverlap(candidateBox, placedBox, 12))) { continue; }
        if (offsetX || offsetY) {
          textElement.setAttribute('x', parseFloat(textElement.getAttribute('x')) + offsetX);
          textElement.setAttribute('y', parseFloat(textElement.getAttribute('y')) + offsetY);
          textElement.querySelectorAll('tspan').forEach((lineElement) => lineElement.setAttribute('x', parseFloat(lineElement.getAttribute('x')) + offsetX));
        }
        placedLabelBoxes.push(candidateBox);
        return;
      }
      placedLabelBoxes.push(originalBox);
    }

    function roundedRectanglePath(x, y, width, height, cornerRadius) {
      const radius = Math.min(cornerRadius, width / 2, height / 2);
      return `M${x + radius},${y} H${x + width - radius} Q${x + width},${y} ${x + width},${y + radius} V${y + height - radius} ` +
        `Q${x + width},${y + height} ${x + width - radius},${y + height} H${x + radius} Q${x},${y + height} ${x},${y + height - radius} ` +
        `V${y + radius} Q${x},${y} ${x + radius},${y} Z`;
    }

    function pointsFromString(pointsText) {
      const numbers = pointsText.trim().split(/[\\s,]+/).map(Number);
      const points = [];
      for (let index = 0; index + 1 < numbers.length; index += 2) { points.push([numbers[index], numbers[index + 1]]); }
      return points;
    }

    // Draws a Lucide icon (24x24 stroke shapes) sketchily at the given center.
    function drawIcon(parentElement, iconName, centerX, centerY, size, colorName) {
      const iconShapes = ICON_SHAPES[iconName];
      if (!iconShapes) { return null; }
      const scale = size / 24;
      const groupElement = svgElement('g', { transform: `translate(${centerX - size / 2},${centerY - size / 2}) scale(${scale})` });
      const strokeOptions = { ...CLEAN_STROKE, roughness: 0.3, stroke: colorOf(colorName, 'fg'), strokeWidth: 2 / scale, seed: stableSeed(iconName) };
      for (const shape of iconShapes) {
        let roughNode = null;
        if (shape[0] === 'p') { roughNode = roughCanvas.path(shape[1], strokeOptions); }
        else if (shape[0] === 'c') { roughNode = roughCanvas.circle(shape[1], shape[2], shape[3] * 2, strokeOptions); }
        else if (shape[0] === 'r') { roughNode = roughCanvas.rectangle(shape[1], shape[2], shape[3], shape[4], strokeOptions); }
        else if (shape[0] === 'l') { roughNode = roughCanvas.line(shape[1], shape[2], shape[3], shape[4], strokeOptions); }
        else if (shape[0] === 'pl') { roughNode = roughCanvas.linearPath(pointsFromString(shape[1]), strokeOptions); }
        else if (shape[0] === 'pg') { roughNode = roughCanvas.polygon(pointsFromString(shape[1]), strokeOptions); }
        else if (shape[0] === 'e') { roughNode = roughCanvas.ellipse(shape[1], shape[2], shape[3] * 2, shape[4] * 2, strokeOptions); }
        if (roughNode) { groupElement.appendChild(roughNode); }
      }
      parentElement.appendChild(groupElement);
      return groupElement;
    }

    // A flat pastel fill with one clean outline, like Clicky's boxes.
    function shapeOptions(colorName, seedText) {
      return { ...CLEAN_STROKE, stroke: outlineOf(colorName, 'blue'), strokeWidth: 2.1, seed: stableSeed(seedText),
        fill: fillOf(colorName, 'blue'), fillStyle: 'solid' };
    }

    // A straight arrow with a small open chevron head, drawn as two strokes
    // (shaft, then head) so the pen traces the shaft first.
    function drawStraightArrow(parentElement, fromX, fromY, toX, toY, options) {
      const { color = ARROW_INK, width = 1.5, headLength = 11, seedText = '' } = options || {};
      const arrowGroup = svgElement('g');
      const strokeOptions = { ...CLEAN_STROKE, roughness: 0.3, stroke: color, strokeWidth: width, seed: stableSeed('arrow' + seedText + fromX + fromY + toX + toY) };
      arrowGroup.appendChild(roughCanvas.line(fromX, fromY, toX, toY, strokeOptions));
      const headAngle = Math.atan2(toY - fromY, toX - fromX);
      arrowGroup.appendChild(roughCanvas.linearPath([
        [toX - headLength * Math.cos(headAngle - 0.5), toY - headLength * Math.sin(headAngle - 0.5)],
        [toX, toY],
        [toX - headLength * Math.cos(headAngle + 0.5), toY - headLength * Math.sin(headAngle + 0.5)]], { ...strokeOptions, roughness: 0.15 }));
      parentElement.appendChild(arrowGroup);
      return arrowGroup;
    }

    // Numbered step badge: a small pale-yellow circle with a thin dark ring.
    function drawStepBadge(parentElement, centerX, centerY, stepNumber) {
      const badgeGroup = svgElement('g');
      badgeGroup.appendChild(roughCanvas.circle(centerX, centerY, 20, { ...CLEAN_STROKE, roughness: 0.25, stroke: '#4a4a4a', strokeWidth: 1.2,
        fill: '#f9f1d2', fillStyle: 'solid', seed: stableSeed('step' + stepNumber) }));
      // Centered on the glyph box (not the baseline) so the digit sits optically in the middle.
      const digitElement = addText(badgeGroup, centerX, centerY, stepNumber, { size: 14 });
      digitElement.setAttribute('dominant-baseline', 'central');
      digitElement.setAttribute('y', centerY + 2);
      parentElement.appendChild(badgeGroup);
      return badgeGroup;
    }

    // Gold check mark, centered at the given point.
    function drawCheckMark(parentElement, centerX, centerY, size) {
      const checkSize = size || 22;
      const checkElement = roughCanvas.linearPath([[centerX - checkSize * 0.45, centerY - checkSize * 0.02],
        [centerX - checkSize * 0.12, centerY + checkSize * 0.3], [centerX + checkSize * 0.48, centerY - checkSize * 0.38]],
        { ...CLEAN_STROKE, roughness: 0.3, stroke: COLORS.gold, strokeWidth: 3.4, seed: stableSeed('check' + centerX + centerY) });
      checkElement.querySelectorAll('path').forEach((pathElement) => { pathElement.setAttribute('stroke-linecap', 'round'); pathElement.setAttribute('stroke-linejoin', 'round'); });
      parentElement.appendChild(checkElement);
      return checkElement;
    }

    // Text width in board units, measured with the handwriting font.
    const measuringContext = document.createElement('canvas').getContext('2d');
    function measureTextWidth(text, size) {
      measuringContext.font = `${size}px ${HAND_FONT}`;
      return measuringContext.measureText(String(text)).width;
    }

    // MARK: node sizes and drawing (all in local coordinates centered on 0,0)

    function nodeSize(command) {
      const labelLength = String(command.label || '').length;
      if (command.t === 'circle') { const diameter = (command.r || 54) * 2; return [diameter, diameter]; }
      if (command.t === 'icon') { return [Math.min(Math.max(labelLength * 9 + 24, 96), 180), 104]; }
      if (command.t === 'note') {
        const noteWidth = command.w || 210;
        return [noteWidth, 30 + wrapIntoLines(command.text, Math.floor(noteWidth / 9)).length * 23];
      }
      if (command.t === 'bars') { return [command.w || 270, command.h || 180]; }
      const boxWidth = Math.min(Math.max(labelLength * 9.6 + 44, 110), 240);
      const labelLineCount = wrapIntoLines(command.label || '', Math.floor(boxWidth / 9.6)).length;
      return [boxWidth, (command.icon ? 92 : 50) + (labelLineCount - 1) * 24];
    }

    function drawIconAndLabel(parentElement, width, height, command) {
      const hasIcon = command.icon && ICON_SHAPES[command.icon];
      const maxCharacters = Math.max(8, Math.floor(width / 9.6));
      // Icons inside a pastel shape use the shape's strong ink.
      const iconColorName = PALETTE[command.color] ? command.color : 'fg';
      if (hasIcon && command.label) {
        const iconSize = Math.min(32, height * 0.4);
        drawIcon(parentElement, command.icon, 0, -height / 2 + 10 + iconSize / 2, iconSize, iconColorName);
        addText(parentElement, 0, height / 2 - 12 - (wrapIntoLines(command.label, maxCharacters).length - 1) * 12 - 8, command.label,
          { size: 21, maxCharacters });
      } else if (hasIcon) {
        drawIcon(parentElement, command.icon, 0, 0, Math.min(44, height * 0.6), iconColorName);
      } else if (command.label) {
        addText(parentElement, 0, 1, command.label, { size: 22, maxCharacters });
      }
    }

    function drawNodeContents(innerElement, command, width, height) {
      if (command.t === 'circle') {
        innerElement.appendChild(roughCanvas.circle(0, 0, width, shapeOptions(command.color, command.id)));
        drawIconAndLabel(innerElement, width * 0.78, height * 0.82, command);
      } else if (command.t === 'icon') {
        drawIcon(innerElement, command.name || command.icon, 0, -14, 50, command.color || 'fg');
        if (command.label) { addText(innerElement, 0, 34, command.label, { size: 20, maxCharacters: 16 }); }
      } else if (command.t === 'note') {
        const noteGroup = svgElement('g', { transform: 'rotate(-1.5)' });
        noteGroup.appendChild(roughCanvas.rectangle(-width / 2, -height / 2, width, height,
          { ...CLEAN_STROKE, stroke: outlineOf(command.color, 'yellow'), strokeWidth: 1.6, fill: fillOf(command.color, 'yellow'), fillStyle: 'solid', seed: stableSeed(command.text) }));
        addText(noteGroup, 0, 0, command.text, { size: 20, maxCharacters: Math.floor(width / 9) });
        innerElement.appendChild(noteGroup);
      } else if (command.t === 'bars') {
        const values = command.values || [];
        const maximumValue = Math.max(...values, 1);
        const slotWidth = width / Math.max(values.length, 1);
        const barWidth = slotWidth * 0.58;
        values.forEach((value, barIndex) => {
          const barHeight = (height - 44) * value / maximumValue;
          const barX = -width / 2 + barIndex * slotWidth + (slotWidth - barWidth) / 2;
          const barY = height / 2 - 22 - barHeight;
          const barColorName = (command.colors && command.colors[barIndex]) || command.color || 'purple';
          const barGroup = svgElement('g');
          barGroup.appendChild(roughCanvas.rectangle(barX, barY, barWidth, barHeight, shapeOptions(barColorName, (command.id || 'bars') + barIndex)));
          innerElement.appendChild(barGroup);
          addText(innerElement, barX + barWidth / 2, barY - 13, (command.prefix || '') + value + (command.suffix || ''), { size: 19 });
          if (command.labels && command.labels[barIndex]) {
            addText(innerElement, barX + barWidth / 2, height / 2 - 6, command.labels[barIndex], { size: 18, color: 'muted' });
          }
        });
      } else {
        innerElement.appendChild(roughCanvas.path(roundedRectanglePath(-width / 2, -height / 2, width, height, 4), shapeOptions(command.color, command.id)));
        drawIconAndLabel(innerElement, width, height, command);
      }
    }

    // Compiles a math expression in x into a function, without eval. Supports
    // + - * / ^, unary minus, parentheses, implicit multiplication (3x,
    // 2sin(x), (x+1)(x-1)), the constants pi and e, and common functions.
    const EXPRESSION_FUNCTIONS = { sin: Math.sin, cos: Math.cos, tan: Math.tan, asin: Math.asin, acos: Math.acos, atan: Math.atan,
      sinh: Math.sinh, cosh: Math.cosh, tanh: Math.tanh, sqrt: Math.sqrt, abs: Math.abs, exp: Math.exp, log: Math.log10, ln: Math.log,
      floor: Math.floor, ceil: Math.ceil, sign: Math.sign };
    // x is the variable for plots, t for parametric curves (θ is read as t).
    const EXPRESSION_NAMES = Object.keys(EXPRESSION_FUNCTIONS).concat(['pi', 'e', 'x', 't']).sort((first, second) => second.length - first.length);
    function compileExpression(expressionText) {
      const normalizedText = String(expressionText).toLowerCase().replace(/\\s+/g, '').replace(/π/g, 'pi').replace(/×|·/g, '*')
        .replace(/÷/g, '/').replace(/−/g, '-').replace(/²/g, '^2').replace(/³/g, '^3').replace(/θ/g, 't');
      const tokens = [];
      let readIndex = 0;
      while (readIndex < normalizedText.length) {
        const remainingText = normalizedText.slice(readIndex);
        const numberMatch = remainingText.match(/^(\\d+\\.?\\d*|\\.\\d+)/);
        if (numberMatch) { tokens.push({ kind: 'number', value: parseFloat(numberMatch[0]) }); readIndex += numberMatch[0].length; continue; }
        const knownName = EXPRESSION_NAMES.find((name) => remainingText.startsWith(name));
        if (knownName) { tokens.push({ kind: 'name', value: knownName }); readIndex += knownName.length; continue; }
        if ('+-*/^()'.includes(remainingText[0])) { tokens.push({ kind: 'symbol', value: remainingText[0] }); readIndex += 1; continue; }
        throw new Error('unknown symbol in expression: ' + remainingText[0]);
      }
      let tokenIndex = 0;
      const peek = () => tokens[tokenIndex];
      const startsFactor = (token) => token && (token.kind === 'number' || token.kind === 'name' || token.value === '(');
      function parseSum() {
        let leftFunction = parseProduct();
        while (peek() && (peek().value === '+' || peek().value === '-')) {
          const operator = tokens[tokenIndex++].value, previousFunction = leftFunction, rightFunction = parseProduct();
          leftFunction = operator === '+' ? (x) => previousFunction(x) + rightFunction(x) : (x) => previousFunction(x) - rightFunction(x);
        }
        return leftFunction;
      }
      function parseProduct() {
        let leftFunction = parseUnary();
        while (peek() && (peek().value === '*' || peek().value === '/' || startsFactor(peek()))) {
          const operator = (peek().value === '*' || peek().value === '/') ? tokens[tokenIndex++].value : '*';
          const previousFunction = leftFunction, rightFunction = parseUnary();
          leftFunction = operator === '*' ? (x) => previousFunction(x) * rightFunction(x) : (x) => previousFunction(x) / rightFunction(x);
        }
        return leftFunction;
      }
      function parseUnary() {
        if (peek() && peek().value === '-') { tokenIndex += 1; const innerFunction = parseUnary(); return (x) => -innerFunction(x); }
        if (peek() && peek().value === '+') { tokenIndex += 1; return parseUnary(); }
        const baseFunction = parsePrimary();
        if (peek() && peek().value === '^') { tokenIndex += 1; const exponentFunction = parseUnary(); return (x) => Math.pow(baseFunction(x), exponentFunction(x)); }
        return baseFunction;
      }
      function parsePrimary() {
        const token = tokens[tokenIndex++];
        if (!token) { throw new Error('expression ended early'); }
        if (token.kind === 'number') { return () => token.value; }
        if (token.value === '(') { const innerFunction = parseSum(); tokenIndex += 1; return innerFunction; }
        if (token.value === 'x' || token.value === 't') { return (x) => x; }
        if (token.value === 'pi') { return () => Math.PI; }
        if (token.value === 'e') { return () => Math.E; }
        const mathFunction = EXPRESSION_FUNCTIONS[token.value];
        if (mathFunction) {
          // With parentheses the argument is exactly what's inside them, so
          // sin(x)^2 squares the sine rather than meaning sin(x^2).
          let argumentFunction;
          if (peek() && peek().value === '(') { tokenIndex += 1; argumentFunction = parseSum(); tokenIndex += 1; }
          else { argumentFunction = parseUnary(); }
          return (x) => mathFunction(argumentFunction(x));
        }
        throw new Error('unexpected token ' + token.value);
      }
      const compiledFunction = parseSum();
      if (tokenIndex < tokens.length) { throw new Error('unexpected trailing input'); }
      return compiledFunction;
    }

    // The math-to-board coordinate frame that plots draw into.
    let currentAxes = null;
    function axesFrame(command) {
      return { left: command.box ? command.box[0] : 80, top: command.box ? command.box[1] : 100,
        width: command.box ? command.box[2] : 640, height: command.box ? command.box[3] : 300,
        xMin: (command.x || [0, 4 * Math.PI])[0], xMax: (command.x || [0, 4 * Math.PI])[1],
        yMin: (command.y || [-1.5, 1.5])[0], yMax: (command.y || [-1.5, 1.5])[1] };
    }
    function frameToBoard(frame, mathX, mathY) {
      return [frame.left + (mathX - frame.xMin) / (frame.xMax - frame.xMin) * frame.width,
        frame.top + (frame.yMax - mathY) / (frame.yMax - frame.yMin) * frame.height];
    }

    // Lets highlight / pulse / shake / tick / cross target geometry shapes too.
    function registerGeometryShape(shapeId, shapePoints, groupElement) {
      if (!shapeId) { return; }
      const pointXs = shapePoints.map((point) => point[0]), pointYs = shapePoints.map((point) => point[1]);
      const minX = Math.min(...pointXs), maxX = Math.max(...pointXs), minY = Math.min(...pointYs), maxY = Math.max(...pointYs);
      // Position lives on the outer group's transform attribute; animations
      // (pulse, shake) go on the inner group. A CSS animation transform would
      // otherwise replace the position and snap the shape to the origin.
      const outerElement = svgElement('g', { transform: `translate(${(minX + maxX) / 2},${(minY + maxY) / 2})` });
      const innerElement = svgElement('g');
      outerElement.appendChild(innerElement);
      // The shape is drawn in board coordinates; shift it so marks drawn in
      // local coordinates (centered on 0,0) line up with it.
      groupElement.setAttribute('transform', `translate(${-(minX + maxX) / 2},${-(minY + maxY) / 2})`);
      groupElement.replaceWith(outerElement);
      innerElement.appendChild(groupElement);
      nodesById[shapeId] = { command: { t: 'shape' }, width: maxX - minX, height: maxY - minY, innerElement, outerElement,
        isPlaced: true, centerX: (minX + maxX) / 2, centerY: (minY + maxY) / 2, isGeometry: true };
    }

    // MARK: layout

    function ensureGroup(groupId) {
      if (!groupId || groupsById[groupId]) { return; }
      groupsById[groupId] = { command: { id: groupId, color: 'blue' }, element: null, isNew: true };
      layoutGraph.setNode(groupId, {});
    }

    function addNode(command) {
      const nodeId = command.id || ('node' + Object.keys(nodesById).length);
      if (nodesById[nodeId]) { return nodeId; }
      const [width, height] = nodeSize(command);
      layoutGraph.setNode(nodeId, { width, height });
      const groupId = command.in || command.group;
      if (groupId) { ensureGroup(groupId); layoutGraph.setParent(nodeId, groupId); }
      const outerElement = svgElement('g');
      const innerElement = svgElement('g');
      outerElement.appendChild(innerElement);
      outerElement.style.opacity = 0;
      mainLayer.appendChild(outerElement);
      nodesById[nodeId] = { command, width, height, outerElement, innerElement, isPlaced: false, centerX: 0, centerY: 0 };
      return nodeId;
    }

    function addEdge(command, edgeStyle) {
      if (!nodesById[command.from] || !nodesById[command.to]) { return; }
      const edgeId = command.id || (command.from + '>' + command.to);
      if (edgesById[edgeId]) { return; }
      // Labels don't reserve their own layout column (that made diagrams
      // very wide); instead the gap between columns fits the longest label.
      const labelLines = command.label ? wrapIntoLines(command.label, 18) : [];
      const labelWidth = labelLines.length ? Math.max(...labelLines.map((line) => line.length)) * 8.5 + 16 : 0;
      layoutGraph.setEdge(command.from, command.to, { minlen: 1 }, edgeId);
      edgesById[edgeId] = { command, edgeStyle, labelWidth, element: null, labelElement: null, geometry: null, isNew: true };
    }

    function polylineGeometry(points) {
      const cumulativeLengths = [0];
      for (let index = 1; index < points.length; index += 1) {
        cumulativeLengths.push(cumulativeLengths[index - 1] + Math.hypot(points[index].x - points[index - 1].x, points[index].y - points[index - 1].y));
      }
      const totalLength = cumulativeLengths[cumulativeLengths.length - 1] || 1;
      function segmentAt(progress) {
        const targetLength = Math.min(Math.max(progress, 0), 1) * totalLength;
        let segmentIndex = 1;
        while (segmentIndex < points.length - 1 && cumulativeLengths[segmentIndex] < targetLength) { segmentIndex += 1; }
        const segmentStart = points[segmentIndex - 1], segmentEnd = points[segmentIndex];
        const segmentLength = (cumulativeLengths[segmentIndex] - cumulativeLengths[segmentIndex - 1]) || 1;
        const segmentProgress = (targetLength - cumulativeLengths[segmentIndex - 1]) / segmentLength;
        return { segmentStart, segmentEnd, segmentProgress };
      }
      return {
        pointAt(progress) {
          const { segmentStart, segmentEnd, segmentProgress } = segmentAt(progress);
          return [segmentStart.x + (segmentEnd.x - segmentStart.x) * segmentProgress, segmentStart.y + (segmentEnd.y - segmentStart.y) * segmentProgress];
        },
        perpendicularAt(progress) {
          const { segmentStart, segmentEnd } = segmentAt(progress);
          const segmentLength = Math.hypot(segmentEnd.x - segmentStart.x, segmentEnd.y - segmentStart.y) || 1;
          return [-(segmentEnd.y - segmentStart.y) / segmentLength, (segmentEnd.x - segmentStart.x) / segmentLength];
        }
      };
    }

    function redrawEdge(edgeId) {
      const edge = edgesById[edgeId];
      const layoutEdge = layoutGraph.edge(edge.command.from, edge.command.to, edgeId);
      if (!layoutEdge || !layoutEdge.points) { return; }
      const points = layoutEdge.points;
      if (edge.element) { edge.element.remove(); }
      if (edge.labelElement) { edge.labelElement.remove(); }
      const isConnector = edge.edgeStyle === 'connector';
      // Thin dark-gray arrows like Clicky's; a color name tints them.
      const arrowColor = PALETTE[edge.command.color] ? colorOf(edge.command.color) : ARROW_INK;
      const strokeOptions = { ...CLEAN_STROKE, roughness: 0.35, stroke: isConnector ? outlineOf('yellow') : arrowColor,
        strokeWidth: isConnector ? 1.4 : 1.5, seed: stableSeed(edgeId),
        strokeLineDash: (isConnector || edge.command.style === 'dashed') ? [7, 6] : undefined };
      const groupElement = svgElement('g');
      // An arrow that already existed is only being moved by a relayout, so
      // it appears at once instead of being traced again.
      if (!edge.isNew) { groupElement.setAttribute('data-pen', 'done'); }
      const pointPairs = points.map((point) => [point.x, point.y]);
      groupElement.appendChild(pointPairs.length > 2 ? roughCanvas.curve(pointPairs, strokeOptions) : roughCanvas.line(...pointPairs[0], ...pointPairs[1], strokeOptions));
      if (!isConnector) {
        const lastPoint = points[points.length - 1], previousPoint = points[points.length - 2];
        const headAngle = Math.atan2(lastPoint.y - previousPoint.y, lastPoint.x - previousPoint.x);
        groupElement.appendChild(roughCanvas.linearPath([
          [lastPoint.x - 11 * Math.cos(headAngle - 0.5), lastPoint.y - 11 * Math.sin(headAngle - 0.5)],
          [lastPoint.x, lastPoint.y],
          [lastPoint.x - 11 * Math.cos(headAngle + 0.5), lastPoint.y - 11 * Math.sin(headAngle + 0.5)]],
          { ...strokeOptions, roughness: 0.15, strokeLineDash: undefined }));
      }
      edgeLayer.appendChild(groupElement);
      edge.element = groupElement;
      edge.geometry = polylineGeometry(points);
      if (edge.command.label) {
        // Just above the middle of the arrow, so the line never runs through it.
        const [middleX, middleY] = edge.geometry.pointAt(0.5);
        const [perpendicularX, perpendicularY] = edge.geometry.perpendicularAt(0.5);
        const offsetSide = perpendicularY > 0 ? -1 : 1;
        edge.labelElement = addText(edgeLayer, middleX + perpendicularX * 16 * offsetSide, middleY + perpendicularY * 16 * offsetSide,
          edge.command.label, { size: 18, color: 'muted', maxCharacters: 18 });
        if (!edge.isNew) { edge.labelElement.setAttribute('data-pen', 'done'); }
      }
      edge.isNew = false;
    }

    function redrawGroup(groupId) {
      const group = groupsById[groupId];
      const layoutGroup = layoutGraph.node(groupId);
      if (!layoutGroup || layoutGroup.width === undefined) { return; }
      if (group.element) { group.element.remove(); }
      const zoneX = layoutGroup.x - layoutGroup.width / 2 - 14, zoneY = layoutGroup.y - layoutGroup.height / 2 - 34;
      const zoneWidth = layoutGroup.width + 28, zoneHeight = layoutGroup.height + 48;
      const groupElement = svgElement('g');
      if (!group.isNew) { groupElement.setAttribute('data-pen', 'done'); }
      groupElement.appendChild(roughCanvas.path(roundedRectanglePath(zoneX, zoneY, zoneWidth, zoneHeight, 10),
        { ...CLEAN_STROKE, stroke: outlineOf(group.command.color, 'gray'), strokeWidth: 1.4, strokeLineDash: [8, 6],
          fill: translucent(fillOf(group.command.color, 'gray'), 0.35), fillStyle: 'solid', seed: stableSeed(groupId) }));
      if (group.command.label) {
        addText(groupElement, zoneX + 14, zoneY + 18, group.command.label, { size: 19, color: 'muted', anchor: 'start' });
      }
      zoneLayer.appendChild(groupElement);
      group.element = groupElement;
      group.isNew = false;
    }

    function positionStepBadges() {
      for (const badge of edgeStepBadges) {
        const edge = edgesById[badge.edgeId];
        if (!edge || !edge.geometry) { continue; }
        // Near the arrow's start, on the opposite side from its label.
        const [pointX, pointY] = edge.geometry.pointAt(0.18);
        const [perpendicularX, perpendicularY] = edge.geometry.perpendicularAt(0.18);
        const badgeSide = perpendicularY > 0 ? 1 : -1;
        badge.element.setAttribute('transform', `translate(${pointX + perpendicularX * 20 * badgeSide},${pointY + perpendicularY * 20 * badgeSide})`);
        badge.element.style.opacity = 1;
      }
    }

    // Bounds of everything drawn except the title and captions. The heading
    // sits top-left above them (like Clicky's), captions left-aligned below.
    function drawingBounds() {
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      for (const layer of [zoneLayer, edgeLayer, mainLayer]) {
        if (!layer.childElementCount) { continue; }
        const layerBounds = layer.getBBox();
        if (!layerBounds.width && !layerBounds.height) { continue; }
        minX = Math.min(minX, layerBounds.x); minY = Math.min(minY, layerBounds.y);
        maxX = Math.max(maxX, layerBounds.x + layerBounds.width); maxY = Math.max(maxY, layerBounds.y + layerBounds.height);
      }
      if (minX === Infinity) { return { minX: 0, maxX: 700, minY: 0, maxY: 0 }; }
      return { minX, maxX, minY, maxY };
    }

    // Board units per screen point at the current camera zoom.
    // Keeps each HTML formula exactly over its placeholder rect, at the camera's zoom.
    const formulaLayer = document.getElementById('formula-layer');
    const formulaOverlays = [];
    function positionFormulaOverlays() {
      const layerRect = formulaLayer.getBoundingClientRect();
      for (const { formulaElement, placeholderElement } of formulaOverlays) {
        const screenMatrix = placeholderElement.getScreenCTM();
        if (!screenMatrix) { continue; }
        const placeholderX = parseFloat(placeholderElement.getAttribute('x')), placeholderY = parseFloat(placeholderElement.getAttribute('y'));
        const screenX = screenMatrix.a * placeholderX + screenMatrix.c * placeholderY + screenMatrix.e - layerRect.left;
        const screenY = screenMatrix.b * placeholderX + screenMatrix.d * placeholderY + screenMatrix.f - layerRect.top;
        formulaElement.style.transform = `translate(${screenX}px, ${screenY}px) scale(${screenMatrix.a})`;
      }
    }

    function boardUnitsPerScreenPoint() {
      const boardWidth = board.getBoundingClientRect().width;
      return boardWidth ? camera.width / boardWidth : 1;
    }

    // Free labels never get smaller than about 15pt on screen, however far
    // the camera zooms out to fit a big drawing.
    const MINIMUM_LABEL_SCREEN_SIZE = 15;
    function applyMinimumLabelSize(textElement) {
      const neededSize = Math.max(textElement.baseFontSize, MINIMUM_LABEL_SCREEN_SIZE * boardUnitsPerScreenPoint());
      if (Math.abs(neededSize - parseFloat(textElement.getAttribute('font-size'))) > 0.3) { textElement.setAttribute('font-size', neededSize); }
    }

    // The title and captions are counter-scaled by the camera zoom, so the
    // heading stays one constant large size on screen like Clicky's. They
    // use a transform attribute (not a CSS transition), so the bounds the
    // camera reads are always the final position.
    function moveTitleLayerElement(element, x, y, scale) {
      element.setAttribute('transform', `translate(${x},${y}) scale(${scale})`);
    }

    function positionTitleAndCaptions() {
      const { minX, minY, maxY } = drawingBounds();
      const titleScale = boardUnitsPerScreenPoint();
      // Left-aligned with the board origin (where rows and captions start),
      // or further left if the drawing extends past it.
      // At the left edge of the frame that fits the drawing, like Clicky's
      // heading, even when the drawing itself is centered.
      const frameLeftX = frameFittingDrawing().x + FRAME_PADDING_LEFT;
      if (titleElement) { moveTitleLayerElement(titleElement, frameLeftX, minY - (titleElement.hasSubtitle ? 98 : 70) * titleScale, titleScale); }
      captionElements.forEach((captionElement, captionIndex) => {
        moveTitleLayerElement(captionElement, frameLeftX, maxY + (40 + captionIndex * 32) * titleScale, titleScale);
      });
    }

    // Shapes glide on the pen's clock instead of a CSS transition, so a
    // glide always lands on its target (a transition can stall while the
    // page isn't rendering) and later relayouts start from where it is.
    function placeNodeAt(node, x, y) {
      node.shownX = x; node.shownY = y;
      node.outerElement.setAttribute('transform', `translate(${x},${y})`);
    }
    function startNodeGlide(node, targetX, targetY) {
      if (Math.abs(node.shownX - targetX) < 0.5 && Math.abs(node.shownY - targetY) < 0.5) { node.glide = null; placeNodeAt(node, targetX, targetY); return; }
      node.glide = { fromX: node.shownX, fromY: node.shownY, toX: targetX, toY: targetY, elapsedSeconds: 0 };
    }
    penFrameCallbacks.push((frameDelta) => {
      for (const node of Object.values(nodesById)) {
        if (!node.glide) { continue; }
        node.glide.elapsedSeconds += frameDelta;
        const progress = Math.min(node.glide.elapsedSeconds / GLIDE_SECONDS, 1);
        const easedProgress = progress < 0.5 ? 2 * progress * progress : 1 - Math.pow(-2 * progress + 2, 2) / 2;
        placeNodeAt(node, node.glide.fromX + (node.glide.toX - node.glide.fromX) * easedProgress, node.glide.fromY + (node.glide.toY - node.glide.fromY) * easedProgress);
        if (progress >= 1) { node.glide = null; }
      }
    });
    // Snaps every gliding shape to its target (arrows are redrawn there).
    function finishNodeGlides() {
      for (const node of Object.values(nodesById)) {
        if (node.glide) { placeNodeAt(node, node.glide.toX, node.glide.toY); node.glide = null; }
      }
    }

    // Lays everything out again: shapes glide to their new spots, then arrows,
    // groups, and badges are redrawn where the shapes ended up.
    function relayout() {
      const longestLabelWidth = Math.max(0, ...Object.values(edgesById).map((edge) => edge.labelWidth || 0));
      // dagre multiplies the gap between columns by (2 × group nesting depth + 1)
      // when there are groups, so divide it back out to keep diagrams compact.
      const groupNestingDepth = (groupId) => {
        const parentGroupId = layoutGraph.parent(groupId);
        return 1 + (parentGroupId && groupsById[parentGroupId] ? groupNestingDepth(parentGroupId) : 0);
      };
      const deepestGroupNesting = Math.max(0, ...Object.keys(groupsById).map(groupNestingDepth));
      const desiredColumnGap = Math.min(Math.max(longestLabelWidth + 24, 90), 180);
      layoutGraph.graph().ranksep = desiredColumnGap / (2 * deepestGroupNesting + 1);
      dagre.layout(layoutGraph);
      for (const [nodeId, node] of Object.entries(nodesById)) {
        if (node.isGeometry) { continue; }
        const layoutNode = layoutGraph.node(nodeId);
        if (!layoutNode) { continue; }
        node.centerX = layoutNode.x; node.centerY = layoutNode.y;
        if (!node.isPlaced) {
          placeNodeAt(node, layoutNode.x, layoutNode.y);
          node.outerElement.style.opacity = 1;
          drawNodeContents(node.innerElement, node.command, node.width, node.height);
          node.isPlaced = true;
        } else {
          startNodeGlide(node, layoutNode.x, layoutNode.y);
        }
      }
      positionTitleAndCaptions();
      clearTimeout(edgeRedrawTimer);
      edgeRedrawTimer = setTimeout(() => {
        // Arrows and groups are drawn at the final layout, so the shapes must be there too.
        finishNodeGlides();
        Object.keys(groupsById).forEach(redrawGroup);
        Object.keys(edgesById).forEach(redrawEdge);
        positionStepBadges();
        positionTitleAndCaptions();
        queueUnrevealedBoardElements();
      }, GLIDE_SECONDS * 1000);
    }

    // MARK: rows and captions (left-aligned flow, like Clicky's numbered boards)
    // Rows stack top to bottom from the board origin: an optional step badge,
    // then items left to right. The last arrow in a row stretches so the item
    // after it starts in the shared result column, so results line up.
    const ROW_RESULT_COLUMN_X = 440;
    const ROW_ITEMS_START_WITH_BADGE_X = 41;
    const ROW_MINIMUM_SLOT_HEIGHT = 70;
    const ROW_VERTICAL_GAP = 40;
    let flowCursorY = 0;
    let previousFlowCaption = null;

    function rowItemSize(item) {
      if (item.box !== undefined) {
        return [item.w || Math.max(measureTextWidth(item.box, 24) + 44, 86), item.h || 42];
      }
      if (item.text !== undefined) { return [measureTextWidth(item.text, item.size || 24), 30]; }
      if (item.check) { return [26, 24]; }
      if (item.stack) { return [104, 70]; }
      if (item.network) { return [92, 64]; }
      if (item.dot) { return [26, 26]; }
      if (item.icon) { return [40, 40]; }
      return [0, 0];
    }

    function drawRowItem(parentElement, item, leftX, centerY, width, height) {
      if (item.box !== undefined) {
        const boxColorName = item.color || 'blue';
        parentElement.appendChild(roughCanvas.rectangle(leftX, centerY - height / 2, width, height, shapeOptions(boxColorName, 'rowbox' + item.box + leftX + centerY)));
        addText(parentElement, leftX + width / 2, centerY + 1, item.box, { size: item.size || 24 });
      } else if (item.text !== undefined) {
        addText(parentElement, leftX, centerY, item.text, { size: item.size || 24, color: item.color || 'fg', anchor: 'start' });
      } else if (item.check) {
        drawCheckMark(parentElement, leftX + width / 2, centerY, 24);
      } else if (item.stack) {
        // Three offset translucent cards: a pile of documents or data. The
        // back cards' outlines show through the front ones, like Clicky's.
        const cardWidth = 79, cardHeight = 54;
        const stackColorName = item.stack === true ? 'gray' : item.stack;
        for (let cardIndex = 0; cardIndex < 3; cardIndex += 1) {
          parentElement.appendChild(roughCanvas.rectangle(leftX + cardIndex * 12, centerY - height / 2 + cardIndex * 8, cardWidth, cardHeight,
            { stroke: outlineOf(stackColorName, 'gray'), strokeWidth: 1.8,
              fill: translucent(fillOf(stackColorName, 'gray'), 0.45), fillStyle: 'solid', seed: stableSeed('card' + cardIndex + leftX + centerY) }));
        }
      } else if (item.network) {
        drawNetworkIcon(parentElement, leftX + width / 2, centerY);
      } else if (item.dot) {
        parentElement.appendChild(roughCanvas.circle(leftX + width / 2, centerY, 24,
          { roughness: 0.3, stroke: outlineOf(item.dot === true ? 'purple' : item.dot, 'purple'), strokeWidth: 2,
            fill: fillOf(item.dot === true ? 'purple' : item.dot, 'purple'), fillStyle: 'solid', seed: stableSeed('dot' + leftX + centerY) }));
      } else if (item.icon) {
        drawIcon(parentElement, item.icon, leftX + width / 2, centerY, 36, item.color || 'fg');
      }
    }

    // A tiny neural network: four purple nodes (two layers of two), fully
    // connected, with a loop bracket under it (training repeats).
    function drawNetworkIcon(parentElement, centerX, centerY) {
      const nodeOffsetX = 33, nodeOffsetY = 22, nodeRadius = 10;
      const nodeCenters = [[-nodeOffsetX, -nodeOffsetY], [nodeOffsetX, -nodeOffsetY], [-nodeOffsetX, nodeOffsetY], [nodeOffsetX, nodeOffsetY]]
        .map(([offsetX, offsetY]) => [centerX + offsetX, centerY + offsetY]);
      const connectionOptions = { roughness: 0.25, stroke: '#8d8d8d', strokeWidth: 1.4 };
      for (const [firstIndex, secondIndex] of [[0, 1], [2, 3], [0, 3], [2, 1]]) {
        const [firstX, firstY] = nodeCenters[firstIndex], [secondX, secondY] = nodeCenters[secondIndex];
        const connectionLength = Math.hypot(secondX - firstX, secondY - firstY);
        const insetX = (secondX - firstX) / connectionLength * nodeRadius, insetY = (secondY - firstY) / connectionLength * nodeRadius;
        parentElement.appendChild(roughCanvas.line(firstX + insetX, firstY + insetY, secondX - insetX, secondY - insetY,
          { ...connectionOptions, seed: stableSeed('connection' + firstIndex + secondIndex + centerX) }));
      }
      nodeCenters.forEach(([nodeX, nodeY], nodeIndex) => {
        parentElement.appendChild(roughCanvas.circle(nodeX, nodeY, nodeRadius * 2,
          { roughness: 0.25, stroke: outlineOf('purple'), strokeWidth: 2, fill: fillOf('purple'), fillStyle: 'solid', seed: stableSeed('node' + nodeIndex + centerX) }));
      });
      parentElement.appendChild(roughCanvas.path(
        `M${centerX - 46},${centerY + 38} Q${centerX - 41},${centerY + 39} ${centerX - 42},${centerY + 46} Q${centerX - 42},${centerY + 56} ${centerX - 30},${centerY + 56} ` +
        `L${centerX + 30},${centerY + 56} Q${centerX + 42},${centerY + 56} ${centerX + 42},${centerY + 46} Q${centerX + 41},${centerY + 39} ${centerX + 46},${centerY + 38}`,
        { roughness: 0.3, stroke: '#3a3a3a', strokeWidth: 1.5, seed: stableSeed('loop' + centerX) }));
    }

    function drawRow(command) {
      const rowItems = Array.isArray(command.items) ? command.items : [];
      const itemSizes = rowItems.map(rowItemSize);
      const rowHeight = Math.max(42, ...itemSizes.map((itemSize) => itemSize[1]));
      const slotHeight = Math.max(rowHeight, ROW_MINIMUM_SLOT_HEIGHT);
      const rowCenterY = flowCursorY + slotHeight / 2;
      const rowGroup = svgElement('g');
      mainLayer.appendChild(rowGroup);
      let cursorX = 0;
      if (command.n !== undefined) {
        drawStepBadge(rowGroup, 11, rowCenterY, command.n);
        cursorX = ROW_ITEMS_START_WITH_BADGE_X;
      }
      const lastArrowIndex = rowItems.map((item) => Boolean(item && item.arrow)).lastIndexOf(true);
      let previousItemWasBox = false;
      rowItems.forEach((item, itemIndex) => {
        if (!item || typeof item !== 'object') { return; }
        if (item.arrow) {
          const arrowStartX = cursorX + (itemIndex === 0 ? 0 : 26);
          const arrowEndX = itemIndex === lastArrowIndex ? Math.max(arrowStartX + 70, ROW_RESULT_COLUMN_X - 22) : arrowStartX + 70;
          drawStraightArrow(rowGroup, arrowStartX, rowCenterY, arrowEndX, rowCenterY, { seedText: 'row' + itemIndex });
          if (typeof item.arrow === 'string') {
            addText(rowGroup, (arrowStartX + arrowEndX) / 2, rowCenterY - 18, item.arrow, { size: 18, color: 'muted' });
          }
          cursorX = arrowEndX + 22;
          previousItemWasBox = false;
          return;
        }
        const [itemWidth, itemHeight] = itemSizes[itemIndex];
        const isBox = item.box !== undefined;
        if (cursorX > 0 && itemIndex > 0) { cursorX += (isBox && previousItemWasBox) ? 10 : 22; }
        drawRowItem(rowGroup, item, cursorX, rowCenterY, itemWidth, itemHeight);
        cursorX += itemWidth;
        previousItemWasBox = isBox;
      });
      flowCursorY += slotHeight + ROW_VERTICAL_GAP;
      previousFlowCaption = null;
      positionTitleAndCaptions();
    }

    // A gray section caption in the flow ("Input tensor"). With
    // "column":"result" it sits in the result column on the same line as
    // the caption just before it.
    function drawFlowCaption(command) {
      const isResultColumn = command.column === 'result';
      if (isResultColumn && previousFlowCaption) {
        addText(mainLayer, ROW_RESULT_COLUMN_X, previousFlowCaption.centerY, command.text, { size: command.size || 22, color: command.color || 'muted', anchor: 'start' });
        return;
      }
      const captionCenterY = flowCursorY + 12;
      addText(mainLayer, isResultColumn ? ROW_RESULT_COLUMN_X : 0, captionCenterY, command.text, { size: command.size || 22, color: command.color || 'muted', anchor: 'start' });
      previousFlowCaption = { centerY: captionCenterY };
      flowCursorY += 46;
      positionTitleAndCaptions();
    }

    // MARK: commands

    function drawCommand(command) {
      const commandType = command.t;

      if (commandType === 'title') {
        // Big bold handwritten heading top-left, an optional gray subtitle
        // under it, and the short lowercase topic in the panel's title bar.
        if (titleElement) { titleElement.remove(); }
        titleElement = svgElement('g');
        const headingElement = addText(titleElement, 0, 0, command.text, { size: 28, anchor: 'start' });
        headingElement.classList.add('heading');
        if (command.sub) {
          addText(titleElement, 0, 42, command.sub, { size: 22, color: 'muted', anchor: 'start' });
          titleElement.hasSubtitle = true;
        }
        titleLayer.appendChild(titleElement);
        setPanelTopic(command.topic || command.text);
        positionTitleAndCaptions();

      } else if (commandType === 'row') {
        drawRow(command);

      } else if (commandType === 'caption') {
        drawFlowCaption(command);

      } else if (commandType === 'force') {
        // A thick colored force arrow in board coordinates (lift, weight,
        // thrust), with its label beside the middle or past the end.
        const [fromX, fromY] = command.from, [toX, toY] = command.to;
        const forceColor = colorOf(command.color, 'blue');
        const groupElement = svgElement('g');
        drawStraightArrow(groupElement, fromX, fromY, toX, toY, { color: forceColor, width: command.width || 3.2, headLength: 17, seedText: 'force' });
        mainLayer.appendChild(groupElement);
        if (command.label) {
          const forceLength = Math.hypot(toX - fromX, toY - fromY) || 1;
          const directionX = (toX - fromX) / forceLength, directionY = (toY - fromY) / forceLength;
          const labelColorName = command.labelColor || (command.color === 'gray' ? 'muted' : 'fg');
          if (command.labelAt === 'end') {
            addText(mainLayer, toX + directionX * 40, toY + directionY * 40, command.label, { size: command.size || 24, color: labelColorName });
          } else {
            // Beside the arrow: to its right for vertical arrows, above for horizontal ones.
            const isMostlyVertical = Math.abs(directionY) > Math.abs(directionX);
            const labelX = isMostlyVertical ? (fromX + toX) / 2 + 16 : (fromX + toX) / 2;
            const labelY = isMostlyVertical ? (fromY + toY) / 2 : (fromY + toY) / 2 - 22;
            addText(mainLayer, labelX, labelY, command.label, { size: command.size || 24, color: labelColorName, anchor: isMostlyVertical ? 'start' : 'middle' });
          }
        }

      } else if (commandType === 'layout') {
        layoutGraph.graph().rankdir = command.direction === 'TB' ? 'TB' : 'LR';
        relayout();

      } else if (commandType === 'group' || commandType === 'zone') {
        ensureGroup(command.id);
        groupsById[command.id].command = command;
        if (command.in) { layoutGraph.setParent(command.id, command.in); ensureGroup(command.in); }

      } else if (['box', 'circle', 'icon', 'note', 'bars'].includes(commandType)) {
        const nodeId = addNode(command);
        if (commandType === 'note' && command.to && nodesById[command.to]) {
          addEdge({ from: nodeId, to: command.to, id: nodeId + '~' + command.to }, 'connector');
        }
        relayout();

      } else if (commandType === 'arrow') {
        addEdge(command, 'arrow');
        relayout();

      } else if (commandType === 'text') {
        const captionElement = svgElement('g');
        addText(captionElement, 0, 0, command.text, { size: command.size || 22, color: command.color || 'muted', maxCharacters: 60, anchor: 'start' });
        titleLayer.appendChild(captionElement);
        captionElements.push(captionElement);
        positionTitleAndCaptions();

      } else if (commandType === 'step') {
        const badgeContainer = svgElement('g');
        drawStepBadge(badgeContainer, 0, 0, command.n);
        const targetNode = nodesById[command.on];
        if (targetNode) {
          badgeContainer.setAttribute('transform', `translate(${-targetNode.width / 2 - 2},${-targetNode.height / 2 - 2})`);
          targetNode.innerElement.appendChild(badgeContainer);
        } else if (edgesById[command.on]) {
          badgeContainer.style.opacity = 0;
          edgeLayer.appendChild(badgeContainer);
          edgeStepBadges.push({ edgeId: command.on, element: badgeContainer });
          positionStepBadges();
        }

      } else if (commandType === 'highlight' || commandType === 'cross' || commandType === 'tick') {
        const targetNode = nodesById[command.id];
        if (!targetNode) { return; }
        // On math shapes (curves, figures) a circle around them gets in the
        // way, so highlighting makes the shape itself glow instead.
        if (commandType === 'highlight' && targetNode.isGeometry) { targetNode.innerElement.classList.add('hb-glow'); return; }
        const markGroup = svgElement('g');
        const halfWidth = targetNode.width / 2, halfHeight = targetNode.height / 2;
        if (commandType === 'highlight') {
          markGroup.appendChild(roughCanvas.ellipse(0, 0, targetNode.width + 40, targetNode.height + 34,
            { ...CLEAN_STROKE, roughness: 1.1, bowing: 1.5, stroke: outlineOf(command.color, 'yellow'), strokeWidth: 2.6 }));
        } else if (commandType === 'cross') {
          const crossOptions = { ...CLEAN_STROKE, stroke: colorOf(command.color, 'red'), strokeWidth: 3.2 };
          markGroup.appendChild(roughCanvas.line(-halfWidth, -halfHeight, halfWidth, halfHeight, crossOptions));
          markGroup.appendChild(roughCanvas.line(halfWidth, -halfHeight, -halfWidth, halfHeight, crossOptions));
        } else {
          drawCheckMark(markGroup, halfWidth + 22, 0, 24);
        }
        targetNode.innerElement.appendChild(markGroup);

      } else if (commandType === 'flow') {
        // Something travelling along an arrow on a loop. It reads the arrow's
        // current shape every frame, so it follows when the layout changes.
        const flowEdgeId = command.along || (command.from + '>' + command.to);
        if (!edgesById[flowEdgeId]) { return; }
        const travellerGroup = svgElement('g');
        if (command.icon && ICON_SHAPES[command.icon]) {
          drawIcon(travellerGroup, command.icon, 0, 0, 24, command.color || 'yellow');
        } else {
          const dotElement = svgElement('circle', { cx: 0, cy: 0, r: 6, fill: colorOf(command.color, 'yellow') });
          dotElement.style.filter = `drop-shadow(0 0 6px ${colorOf(command.color, 'yellow')})`;
          travellerGroup.appendChild(dotElement);
        }
        travellerGroup.style.opacity = 0;
        motionLayer.appendChild(travellerGroup);
        const travelMilliseconds = (command.seconds || 1.8) * 1000;
        const pauseMilliseconds = 450;
        const loopStartTime = performance.now();
        function moveTraveller(currentTime) {
          const edgeGeometry = edgesById[flowEdgeId].geometry;
          if (edgeGeometry) {
            const loopProgress = ((currentTime - loopStartTime) % (travelMilliseconds + pauseMilliseconds)) / travelMilliseconds;
            const travelProgress = Math.min(loopProgress, 1);
            const [travellerX, travellerY] = edgeGeometry.pointAt(travelProgress);
            travellerGroup.setAttribute('transform', `translate(${travellerX},${travellerY})`);
            travellerGroup.style.opacity = loopProgress > 1 ? 0 : Math.min(1, travelProgress * 8, (1 - travelProgress) * 8);
          }
          requestAnimationFrame(moveTraveller);
        }
        requestAnimationFrame(moveTraveller);

      } else if (commandType === 'pulse' || commandType === 'shake') {
        const targetNode = nodesById[command.id];
        if (targetNode) { targetNode.innerElement.classList.add(commandType === 'pulse' ? 'hb-pulse' : 'hb-shake'); }

      // MARK: geometry mode — exact shapes on fixed coordinates, for math,
      // physics, and anything where true angles and proportions matter.

      } else if (commandType === 'polygon') {
        const polygonPoints = command.points || [];
        if (polygonPoints.length < 2) { return; }
        const isOutlineOnly = command.fill === false;
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.polygon(polygonPoints, {
          ...CLEAN_STROKE, stroke: isOutlineOnly ? colorOf(command.color, 'fg') : outlineOf(command.color, 'blue'),
          strokeWidth: command.width || (command.color === 'bluestrong' ? 2.5 : 2),
          seed: stableSeed(JSON.stringify(polygonPoints)), fill: isOutlineOnly ? undefined : fillOf(command.color, 'blue'), fillStyle: 'solid' }));
        mainLayer.appendChild(groupElement);
        registerGeometryShape(command.id, polygonPoints, groupElement);

      } else if (commandType === 'square_on') {
        // The square standing on segment from→to, on the side away from
        // "away" (or to the left of from→to). Computed exactly here, so a
        // square on a slanted hypotenuse is still a perfect square.
        const [fromX, fromY] = command.from, [toX, toY] = command.to;
        let normalX = -(toY - fromY), normalY = toX - fromX;
        if (command.away) {
          const middleX = (fromX + toX) / 2, middleY = (fromY + toY) / 2;
          if ((command.away[0] - middleX) * normalX + (command.away[1] - middleY) * normalY > 0) { normalX = -normalX; normalY = -normalY; }
        }
        const squarePoints = [[fromX, fromY], [toX, toY], [toX + normalX, toY + normalY], [fromX + normalX, fromY + normalY]];
        const squareColor = outlineOf(command.color, 'blue');
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.polygon(squarePoints, { ...CLEAN_STROKE, stroke: squareColor, strokeWidth: 2,
          seed: stableSeed(JSON.stringify(squarePoints)), fill: fillOf(command.color, 'blue'), fillStyle: 'solid' }));
        // Optional unit grid (grid: 3 → 3×3 cells) so areas can be counted.
        const gridCount = command.grid || 0;
        for (let gridIndex = 1; gridIndex < gridCount; gridIndex += 1) {
          const fraction = gridIndex / gridCount;
          const gridOptions = { stroke: translucent(squareColor, 0.55), strokeWidth: 1.1, roughness: 0.4, seed: stableSeed('grid' + gridIndex + JSON.stringify(squarePoints)) };
          groupElement.appendChild(roughCanvas.line(fromX + (toX - fromX) * fraction, fromY + (toY - fromY) * fraction,
            fromX + (toX - fromX) * fraction + normalX, fromY + (toY - fromY) * fraction + normalY, gridOptions));
          groupElement.appendChild(roughCanvas.line(fromX + normalX * fraction, fromY + normalY * fraction,
            toX + normalX * fraction, toY + normalY * fraction, gridOptions));
        }
        mainLayer.appendChild(groupElement);
        if (command.label) {
          addText(mainLayer, (fromX + toX) / 2 + normalX / 2, (fromY + toY) / 2 + normalY / 2, command.label,
            { size: command.size || 22, color: command.color || 'blue' });
        }
        registerGeometryShape(command.id, squarePoints, groupElement);

      } else if (commandType === 'line') {
        const [fromX, fromY] = command.from, [toX, toY] = command.to;
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.line(fromX, fromY, toX, toY, { stroke: colorOf(command.color, 'fg'), strokeWidth: command.width || 2,
          roughness: 0.5, seed: stableSeed(JSON.stringify([command.from, command.to])), strokeLineDash: command.style === 'dashed' ? [8, 7] : undefined }));
        mainLayer.appendChild(groupElement);
        if (command.label) {
          // Beside the middle of the line, on the side given by labelSide (default: outward from the line's left).
          const lineLength = Math.hypot(toX - fromX, toY - fromY) || 1;
          const sideSign = command.labelSide === 'right' ? -1 : 1;
          addText(mainLayer, (fromX + toX) / 2 - (toY - fromY) / lineLength * 22 * sideSign, (fromY + toY) / 2 + (toX - fromX) / lineLength * 22 * sideSign,
            command.label, { size: command.size || 20, color: command.color || 'fg' });
        }

      } else if (commandType === 'point') {
        const groupElement = svgElement('g');
        groupElement.appendChild(svgElement('circle', { cx: command.x, cy: command.y, r: 4.5, fill: colorOf(command.color, 'fg') }));
        mainLayer.appendChild(groupElement);
        if (command.label) { addText(mainLayer, command.x + (command.dx ?? 14), command.y + (command.dy ?? -14), command.label, { size: 18, color: command.color || 'fg' }); }

      } else if (commandType === 'ring') {
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.circle(command.x, command.y, command.r * 2, { stroke: colorOf(command.color, 'purple'), strokeWidth: 2.4,
          roughness: 0.5, seed: stableSeed('ring' + command.x + command.y + command.r), fill: command.fill ? fillOf(command.color, 'purple') : undefined, fillStyle: 'solid' }));
        mainLayer.appendChild(groupElement);

      } else if (commandType === 'angle') {
        // Angle mark at a vertex between the rays to "from" and "to": a small
        // square for right angles, otherwise an arc.
        const [vertexX, vertexY] = command.at;
        const firstAngle = Math.atan2(command.from[1] - vertexY, command.from[0] - vertexX);
        const secondAngle = Math.atan2(command.to[1] - vertexY, command.to[0] - vertexX);
        const markColor = colorOf(command.color, 'yellow');
        const groupElement = svgElement('g');
        const markSize = command.size || 22;
        if (command.right) {
          const cornerA = [vertexX + Math.cos(firstAngle) * markSize, vertexY + Math.sin(firstAngle) * markSize];
          const cornerB = [vertexX + Math.cos(secondAngle) * markSize, vertexY + Math.sin(secondAngle) * markSize];
          const cornerC = [cornerA[0] + Math.cos(secondAngle) * markSize, cornerA[1] + Math.sin(secondAngle) * markSize];
          groupElement.appendChild(roughCanvas.linearPath([cornerA, cornerC, cornerB], { stroke: markColor, strokeWidth: 2.2, roughness: 0.3 }));
        } else {
          let sweep = secondAngle - firstAngle;
          while (sweep <= -Math.PI) { sweep += Math.PI * 2; }
          while (sweep > Math.PI) { sweep -= Math.PI * 2; }
          const arcPoints = [];
          for (let stepIndex = 0; stepIndex <= 16; stepIndex += 1) {
            const stepAngle = firstAngle + sweep * stepIndex / 16;
            arcPoints.push([vertexX + Math.cos(stepAngle) * markSize * 1.4, vertexY + Math.sin(stepAngle) * markSize * 1.4]);
          }
          groupElement.appendChild(roughCanvas.linearPath(arcPoints, { stroke: markColor, strokeWidth: 2.2, roughness: 0.3 }));
          if (command.label) {
            const middleAngle = firstAngle + sweep / 2;
            addText(groupElement, vertexX + Math.cos(middleAngle) * markSize * 2.6, vertexY + Math.sin(middleAngle) * markSize * 2.6, command.label, { size: 17, color: command.color || 'yellow' });
          }
        }
        mainLayer.appendChild(groupElement);

      } else if (commandType === 'axes') {
        currentAxes = axesFrame(command);
        const frame = currentAxes;
        const axisOptions = { stroke: colorOf(command.color, 'muted'), strokeWidth: 1.8, roughness: 0.4, seed: stableSeed('axes' + JSON.stringify(command)) };
        // Axes cross at the origin when it's in view, otherwise along the bottom/left edge.
        const xAxisMathY = frame.yMin <= 0 && frame.yMax >= 0 ? 0 : frame.yMin;
        const yAxisMathX = frame.xMin <= 0 && frame.xMax >= 0 ? 0 : frame.xMin;
        const [, xAxisBoardY] = frameToBoard(frame, 0, xAxisMathY);
        const [yAxisBoardX] = frameToBoard(frame, yAxisMathX, 0);
        const groupElement = svgElement('g');
        if (command.grid) {
          // Light grid lines every gridStep units (default 1).
          const gridStep = command.gridStep || 1;
          const gridOptions = { stroke: 'rgba(84,72,40,0.13)', strokeWidth: 1, roughness: 0.2, seed: 11 };
          for (let gridX = Math.ceil(frame.xMin / gridStep) * gridStep; gridX <= frame.xMax + 1e-9; gridX += gridStep) {
            const [gridBoardX] = frameToBoard(frame, gridX, 0);
            groupElement.appendChild(roughCanvas.line(gridBoardX, frame.top, gridBoardX, frame.top + frame.height, gridOptions));
          }
          for (let gridY = Math.ceil(frame.yMin / gridStep) * gridStep; gridY <= frame.yMax + 1e-9; gridY += gridStep) {
            const [, gridBoardY] = frameToBoard(frame, 0, gridY);
            groupElement.appendChild(roughCanvas.line(frame.left, gridBoardY, frame.left + frame.width, gridBoardY, gridOptions));
          }
        }
        groupElement.appendChild(roughCanvas.line(frame.left, xAxisBoardY, frame.left + frame.width + 12, xAxisBoardY, axisOptions));
        groupElement.appendChild(roughCanvas.line(yAxisBoardX, frame.top + frame.height, yAxisBoardX, frame.top - 12, axisOptions));
        groupElement.appendChild(roughCanvas.linearPath([[frame.left + frame.width + 4, xAxisBoardY - 6], [frame.left + frame.width + 12, xAxisBoardY], [frame.left + frame.width + 4, xAxisBoardY + 6]], axisOptions));
        groupElement.appendChild(roughCanvas.linearPath([[yAxisBoardX - 6, frame.top - 4], [yAxisBoardX, frame.top - 12], [yAxisBoardX + 6, frame.top - 4]], axisOptions));
        mainLayer.appendChild(groupElement);
        if (command.xLabel) { addText(mainLayer, frame.left + frame.width + 26, xAxisBoardY, command.xLabel, { size: 17, color: 'muted' }); }
        if (command.yLabel) { addText(mainLayer, yAxisBoardX, frame.top - 26, command.yLabel, { size: 17, color: 'muted' }); }
        // Without explicit ticks, integer-ish numbers along both axes so the scale can be read.
        if (!command.xTicks && !command.yTicks) {
          const niceTickStep = (range) => { const roughStep = range / 8; const power = Math.pow(10, Math.floor(Math.log10(roughStep)));
            return [1, 2, 5, 10].map((multiple) => multiple * power).find((step) => step >= roughStep) || roughStep; };
          const xTickStep = command.grid ? (command.gridStep || 1) : niceTickStep(frame.xMax - frame.xMin);
          const yTickStep = command.grid ? (command.gridStep || 1) : niceTickStep(frame.yMax - frame.yMin);
          const tickText = (value) => String(Math.round(value * 1000) / 1000);
          command.xTicks = [];
          command.yTicks = [];
          for (let tickValue = Math.ceil(frame.xMin / xTickStep) * xTickStep; tickValue <= frame.xMax + 1e-9; tickValue += xTickStep) {
            if (Math.abs(tickValue) > 1e-9) { command.xTicks.push([tickValue, tickText(tickValue)]); }
          }
          for (let tickValue = Math.ceil(frame.yMin / yTickStep) * yTickStep; tickValue <= frame.yMax + 1e-9; tickValue += yTickStep) {
            if (Math.abs(tickValue) > 1e-9) { command.yTicks.push([tickValue, tickText(tickValue)]); }
          }
        }
        (command.xTicks || []).forEach((tick) => {
          const [tickX] = frameToBoard(frame, tick[0], xAxisMathY);
          addText(mainLayer, tickX, xAxisBoardY + 18, tick[1], { size: 17, color: 'muted' });
        });
        (command.yTicks || []).forEach((tick) => {
          const [, tickY] = frameToBoard(frame, yAxisMathX, tick[0]);
          addText(mainLayer, yAxisBoardX - 14, tickY, tick[1], { size: 17, color: 'muted', anchor: 'end' });
        });

      } else if (commandType === 'plot') {
        // A smooth curve of a math expression, sampled finely inside the axes.
        const frame = currentAxes || axesFrame({});
        let curveFunction;
        try { curveFunction = compileExpression(command.expr); } catch (expressionError) { console.log(expressionError); return; }
        const plotXMin = command.from ?? frame.xMin, plotXMax = command.to ?? frame.xMax;
        const curveSegments = [];
        let currentSegment = [];
        for (let sampleIndex = 0; sampleIndex <= 400; sampleIndex += 1) {
          const mathX = plotXMin + (plotXMax - plotXMin) * sampleIndex / 400;
          const mathY = curveFunction(mathX);
          // Break the curve at gaps and asymptotes instead of drawing spikes.
          if (!Number.isFinite(mathY) || mathY > frame.yMax + (frame.yMax - frame.yMin) || mathY < frame.yMin - (frame.yMax - frame.yMin)) {
            if (currentSegment.length > 1) { curveSegments.push(currentSegment); }
            currentSegment = [];
            continue;
          }
          currentSegment.push(frameToBoard(frame, mathX, mathY));
        }
        if (currentSegment.length > 1) { curveSegments.push(currentSegment); }
        if (!curveSegments.length) { return; }
        const curveColor = colorOf(command.color, 'purple');
        const groupElement = svgElement('g');
        curveSegments.forEach((segmentPoints, segmentIndex) => {
          groupElement.appendChild(roughCanvas.linearPath(segmentPoints, { stroke: curveColor, strokeWidth: command.width || 2.8,
            roughness: 0.25, bowing: 0, seed: stableSeed(command.expr + segmentIndex), strokeLineDash: command.style === 'dashed' ? [9, 7] : undefined }));
        });
        mainLayer.appendChild(groupElement);
        const allCurvePoints = curveSegments.flat();
        if (command.label) {
          // Curve labels go in a stacked legend at the axes' top-right corner,
          // so several plots never pile their labels on top of each other.
          frame.legendCount = (frame.legendCount || 0) + 1;
          const legendX = frame.left + frame.width + 30, legendY = frame.top + (frame.legendCount - 1) * 26;
          const legendGroup = svgElement('g');
          legendGroup.appendChild(roughCanvas.line(legendX, legendY, legendX + 22, legendY, { stroke: curveColor, strokeWidth: 3, roughness: 0.3,
            strokeLineDash: command.style === 'dashed' ? [6, 5] : undefined, seed: stableSeed('legend' + command.expr) }));
          addText(legendGroup, legendX + 30, legendY, command.label, { size: 20, color: command.color || 'purple', anchor: 'start' });
          mainLayer.appendChild(legendGroup);
        }
        registerGeometryShape(command.id, allCurvePoints, groupElement);

      } else if (commandType === 'path') {
        // An open curve through points in board coordinates (polygon closes,
        // path doesn't); "smooth":false joins the points with straight lines.
        const pathPoints = command.points || [];
        if (pathPoints.length < 2) { return; }
        const groupElement = svgElement('g');
        const drawOpenPath = command.smooth === false ? roughCanvas.linearPath.bind(roughCanvas) : roughCanvas.curve.bind(roughCanvas);
        groupElement.appendChild(drawOpenPath(pathPoints, { stroke: colorOf(command.color, 'purple'), strokeWidth: command.width || 2,
          roughness: 0.4, seed: stableSeed(JSON.stringify(pathPoints)), strokeLineDash: command.style === 'dashed' ? [9, 7] : undefined }));
        mainLayer.appendChild(groupElement);
        registerGeometryShape(command.id, pathPoints, groupElement);

      } else if (commandType === 'parametric') {
        // A curve traced by (x(t), y(t)): circles, ellipses, spirals, orbits.
        const frame = currentAxes || axesFrame({});
        let xFunction, yFunction;
        try { xFunction = compileExpression(command.x); yFunction = compileExpression(command.y); } catch (expressionError) { console.log(expressionError); return; }
        const tStart = command.from ?? 0, tEnd = command.to ?? 2 * Math.PI;
        const curvePoints = [];
        for (let sampleIndex = 0; sampleIndex <= 500; sampleIndex += 1) {
          const tValue = tStart + (tEnd - tStart) * sampleIndex / 500;
          const mathX = xFunction(tValue), mathY = yFunction(tValue);
          if (Number.isFinite(mathX) && Number.isFinite(mathY)) { curvePoints.push(frameToBoard(frame, mathX, mathY)); }
        }
        if (curvePoints.length < 2) { return; }
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.linearPath(curvePoints, { stroke: colorOf(command.color, 'purple'), strokeWidth: command.width || 2.8,
          roughness: 0.25, bowing: 0, seed: stableSeed(command.x + command.y) }));
        mainLayer.appendChild(groupElement);
        if (command.label) {
          const [labelAnchorX, labelAnchorY] = curvePoints[Math.floor(curvePoints.length * 0.15)];
          addText(mainLayer, labelAnchorX + 12, labelAnchorY - 16, command.label, { size: 17, color: command.color || 'purple', anchor: 'start' });
        }
        registerGeometryShape(command.id, curvePoints, groupElement);

      } else if (commandType === 'area') {
        // Shaded region between a curve and the x-axis (or a second curve)
        // from "from" to "to": integrals, probabilities, regions.
        const frame = currentAxes || axesFrame({});
        let upperFunction, lowerFunction;
        try {
          upperFunction = compileExpression(command.expr);
          lowerFunction = command.under ? compileExpression(command.under) : () => 0;
        } catch (expressionError) { console.log(expressionError); return; }
        const areaFrom = command.from ?? frame.xMin, areaTo = command.to ?? frame.xMax;
        const upperPoints = [], lowerPoints = [];
        for (let sampleIndex = 0; sampleIndex <= 200; sampleIndex += 1) {
          const mathX = areaFrom + (areaTo - areaFrom) * sampleIndex / 200;
          const upperY = upperFunction(mathX), lowerY = lowerFunction(mathX);
          if (!Number.isFinite(upperY) || !Number.isFinite(lowerY)) { continue; }
          upperPoints.push(frameToBoard(frame, mathX, Math.min(Math.max(upperY, frame.yMin), frame.yMax)));
          lowerPoints.push(frameToBoard(frame, mathX, Math.min(Math.max(lowerY, frame.yMin), frame.yMax)));
        }
        if (upperPoints.length < 2) { return; }
        const areaColor = colorOf(command.color, 'yellow');
        const regionPoints = upperPoints.concat(lowerPoints.reverse());
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.polygon(regionPoints, { stroke: 'none', fill: translucent(fillOf(command.color, 'yellow'), 0.85), fillStyle: 'solid',
          hachureGap: 6, fillWeight: 1.4, roughness: 0.6, seed: stableSeed('area' + command.expr + areaFrom + areaTo) }));
        mainLayer.insertBefore(groupElement, mainLayer.firstChild);
        if (command.label) {
          const [labelX, labelY] = frameToBoard(frame, (areaFrom + areaTo) / 2, (upperFunction((areaFrom + areaTo) / 2) + lowerFunction((areaFrom + areaTo) / 2)) / 2);
          addText(mainLayer, labelX, labelY, command.label, { size: 18, color: command.color || 'yellow' });
        }
        // Later labels (a tangent's slope, a point name) keep off the shaded
        // region's edge: points along its outline count as placed labels.
        regionPoints.forEach((outlinePoint, pointIndex) => {
          if (pointIndex % 6 === 0) { placedLabelBoxes.push({ x: outlinePoint[0], y: outlinePoint[1], width: 1, height: 1 }); }
        });
        registerGeometryShape(command.id, regionPoints, groupElement);

      } else if (commandType === 'tangent') {
        // Tangent line to a curve at x = at, slope computed numerically.
        const frame = currentAxes || axesFrame({});
        let curveFunction;
        try { curveFunction = compileExpression(command.expr); } catch (expressionError) { console.log(expressionError); return; }
        const touchX = command.at, touchY = curveFunction(touchX);
        const stepSize = 1e-5 * Math.max(1, Math.abs(touchX));
        const slope = (curveFunction(touchX + stepSize) - curveFunction(touchX - stepSize)) / (2 * stepSize);
        if (!Number.isFinite(touchY) || !Number.isFinite(slope)) { return; }
        const halfLength = (command.length || (frame.xMax - frame.xMin) * 0.4) / 2;
        const [lineStartX, lineStartY] = frameToBoard(frame, touchX - halfLength, touchY - slope * halfLength);
        const [lineEndX, lineEndY] = frameToBoard(frame, touchX + halfLength, touchY + slope * halfLength);
        const [touchBoardX, touchBoardY] = frameToBoard(frame, touchX, touchY);
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.line(lineStartX, lineStartY, lineEndX, lineEndY, { stroke: colorOf(command.color, 'orange'),
          strokeWidth: 2.4, roughness: 0.3, seed: stableSeed('tangent' + command.expr + touchX) }));
        groupElement.appendChild(svgElement('circle', { cx: touchBoardX, cy: touchBoardY, r: 5.5, fill: colorOf(command.color, 'orange') }));
        mainLayer.appendChild(groupElement);
        const slopeText = command.label ?? ('slope = ' + (Math.round(slope * 100) / 100));
        if (slopeText) { addText(mainLayer, lineEndX + 10, lineEndY - 14, slopeText, { size: 16, color: command.color || 'orange', anchor: 'start' }); }

      } else if (commandType === 'mark') {
        // A labelled point in math coordinates, with optional dashed guides to the axes.
        const frame = currentAxes || axesFrame({});
        const [markX, markY] = frameToBoard(frame, command.x, command.y);
        const markColor = colorOf(command.color, 'fg');
        const groupElement = svgElement('g');
        if (command.guides) {
          const [, axisBoardY] = frameToBoard(frame, 0, frame.yMin <= 0 && frame.yMax >= 0 ? 0 : frame.yMin);
          const [axisBoardX] = frameToBoard(frame, frame.xMin <= 0 && frame.xMax >= 0 ? 0 : frame.xMin, 0);
          const guideOptions = { stroke: translucent(markColor, 0.6), strokeWidth: 1.4, roughness: 0.3, strokeLineDash: [6, 6] };
          groupElement.appendChild(roughCanvas.line(markX, markY, markX, axisBoardY, guideOptions));
          groupElement.appendChild(roughCanvas.line(markX, markY, axisBoardX, markY, guideOptions));
        }
        groupElement.appendChild(svgElement('circle', { cx: markX, cy: markY, r: 5.5, fill: markColor }));
        mainLayer.appendChild(groupElement);
        if (command.label) { addText(mainLayer, markX + (command.dx ?? 12), markY + (command.dy ?? -16), command.label, { size: 17, color: command.color || 'fg', anchor: 'start' }); }

      } else if (commandType === 'vector') {
        // An arrow in math coordinates: vectors, forces, velocities.
        const frame = currentAxes || axesFrame({});
        const [startX, startY] = frameToBoard(frame, command.from[0], command.from[1]);
        const [endX, endY] = frameToBoard(frame, command.to[0], command.to[1]);
        const vectorColor = colorOf(command.color, 'pink');
        const headAngle = Math.atan2(endY - startY, endX - startX);
        const vectorOptions = { stroke: vectorColor, strokeWidth: 3, roughness: 0.3, seed: stableSeed('vector' + JSON.stringify(command)) };
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.line(startX, startY, endX, endY, vectorOptions));
        groupElement.appendChild(roughCanvas.polygon([[endX, endY], [endX - 15 * Math.cos(headAngle - 0.4), endY - 15 * Math.sin(headAngle - 0.4)],
          [endX - 15 * Math.cos(headAngle + 0.4), endY - 15 * Math.sin(headAngle + 0.4)]], { ...vectorOptions, fill: vectorColor, fillStyle: 'solid' }));
        mainLayer.appendChild(groupElement);
        if (command.label) {
          const vectorLength = Math.hypot(endX - startX, endY - startY) || 1;
          addText(mainLayer, (startX + endX) / 2 - (endY - startY) / vectorLength * 18, (startY + endY) / 2 + (endX - startX) / vectorLength * 18,
            command.label, { size: 19, color: command.color || 'pink' });
        }
        registerGeometryShape(command.id, [[startX, startY], [endX, endY]], groupElement);

      } else if (commandType === 'numberline') {
        // A number line with ticks, marked numbers, and highlighted intervals
        // (for inequalities, ranges, and number sense).
        const lineLeft = command.left ?? 80, lineWidth = command.width ?? 640, lineY = command.y ?? 300;
        const lineFrom = command.from ?? -5, lineTo = command.to ?? 5, tickStep = command.step ?? 1;
        const toBoardX = (value) => lineLeft + (value - lineFrom) / (lineTo - lineFrom) * lineWidth;
        const groupElement = svgElement('g');
        const lineOptions = { stroke: colorOf('muted'), strokeWidth: 2, roughness: 0.3, seed: stableSeed('numberline' + lineFrom + lineTo) };
        groupElement.appendChild(roughCanvas.line(lineLeft - 14, lineY, lineLeft + lineWidth + 14, lineY, lineOptions));
        for (let tickValue = Math.ceil(lineFrom / tickStep) * tickStep; tickValue <= lineTo + 1e-9; tickValue += tickStep) {
          const tickX = toBoardX(tickValue);
          groupElement.appendChild(roughCanvas.line(tickX, lineY - 7, tickX, lineY + 7, lineOptions));
          addText(groupElement, tickX, lineY + 24, Math.round(tickValue * 1000) / 1000, { size: 15, color: 'muted' });
        }
        (command.ranges || []).forEach((range, rangeIndex) => {
          const rangeColor = colorOf(range[2] || 'yellow');
          groupElement.appendChild(roughCanvas.line(toBoardX(range[0]), lineY - 2, toBoardX(range[1]), lineY - 2,
            { stroke: rangeColor, strokeWidth: 6, roughness: 0.4, seed: stableSeed('range' + rangeIndex) }));
        });
        (command.marks || []).forEach((markEntry) => {
          const markColor = colorOf(markEntry[2] || 'pink');
          groupElement.appendChild(svgElement('circle', { cx: toBoardX(markEntry[0]), cy: lineY, r: 7, fill: markColor }));
          if (markEntry[1]) { addText(groupElement, toBoardX(markEntry[0]), lineY - 24, markEntry[1], { size: 17, color: markEntry[2] || 'pink' }); }
        });
        mainLayer.appendChild(groupElement);

      } else if (commandType === 'pie') {
        // Sectors for fractions, probabilities, and shares.
        const values = (command.values || []).map(Number);
        const valueTotal = values.reduce((sum, value) => sum + value, 0) || 1;
        const pieX = command.x ?? 400, pieY = command.y ?? 260, pieRadius = command.r ?? 110;
        const pieColors = ['purple', 'blue', 'green', 'orange', 'pink', 'cyan', 'yellow', 'red'];
        const groupElement = svgElement('g');
        let startAngle = -Math.PI / 2;
        values.forEach((value, sectorIndex) => {
          const sweepAngle = value / valueTotal * Math.PI * 2;
          const sectorColor = outlineOf((command.colors && command.colors[sectorIndex]) || pieColors[sectorIndex % pieColors.length]);
          const sectorPoints = [[pieX, pieY]];
          for (let stepIndex = 0; stepIndex <= 40; stepIndex += 1) {
            const stepAngle = startAngle + sweepAngle * stepIndex / 40;
            sectorPoints.push([pieX + Math.cos(stepAngle) * pieRadius, pieY + Math.sin(stepAngle) * pieRadius]);
          }
          groupElement.appendChild(roughCanvas.polygon(sectorPoints, { stroke: sectorColor, strokeWidth: 2.2, roughness: 0.6,
            fill: fillOf((command.colors && command.colors[sectorIndex]) || pieColors[sectorIndex % pieColors.length]), fillStyle: 'solid', seed: stableSeed('pie' + sectorIndex + value) }));
          if (command.labels && command.labels[sectorIndex]) {
            const middleAngle = startAngle + sweepAngle / 2;
            addText(groupElement, pieX + Math.cos(middleAngle) * pieRadius * 1.25, pieY + Math.sin(middleAngle) * pieRadius * 1.25,
              command.labels[sectorIndex], { size: 17, color: (command.colors && command.colors[sectorIndex]) || pieColors[sectorIndex % pieColors.length] });
          }
          startAngle += sweepAngle;
        });
        mainLayer.appendChild(groupElement);
        registerGeometryShape(command.id, [[pieX - pieRadius, pieY - pieRadius], [pieX + pieRadius, pieY + pieRadius]], groupElement);

      } else if (commandType === 'latex') {
        // Properly typeset math (fractions, integrals, sums, matrices, roots)
        // via KaTeX, embedded in the board and centered at x, y.
        // It's HTML laid over the board, not a foreignObject: WebKit paints
        // KaTeX's relatively positioned spans inside a foreignObject without
        // the SVG camera transform, so the formula showed up somewhere else.
        // An invisible rect of the same size holds its place in the drawing.
        if (!window.katex) { return; }
        const formulaSize = (command.size || 1.3) * 22;
        const formulaElement = document.createElement('div');
        formulaElement.style.cssText = `font-size:${formulaSize}px;color:${colorOf(command.color, 'fg')}`;
        try {
          katex.render(String(command.tex || ''), formulaElement, { throwOnError: false, displayMode: true, trust: false, strict: 'ignore' });
        } catch (formulaError) { console.log(formulaError); return; }
        formulaLayer.appendChild(formulaElement);
        const formulaWidth = formulaElement.offsetWidth + 4, formulaHeight = formulaElement.offsetHeight + 4;
        // Without coordinates, it goes centered under everything drawn so far.
        const formulaBounds = drawingBounds();
        const formulaCenterX = command.x ?? (formulaBounds.minX + formulaBounds.maxX) / 2;
        const formulaCenterY = command.y ?? formulaBounds.maxY + formulaHeight / 2 + 24;
        const placeholderElement = svgElement('rect', { x: formulaCenterX - formulaWidth / 2, y: formulaCenterY - formulaHeight / 2,
          width: formulaWidth, height: formulaHeight, fill: 'none', 'data-pen': 'done' });
        mainLayer.appendChild(placeholderElement);
        formulaOverlays.push({ formulaElement, placeholderElement });
        positionFormulaOverlays();
        queuePenLeaves([formulaElement]);

      } else if (commandType === 'label') {
        const labelElement = addText(mainLayer, command.x, command.y, command.text,
          { size: command.size || 24, color: command.color || 'fg', maxCharacters: command.wrap || 0, anchor: command.anchor === 'start' ? 'start' : 'middle' });
        if (command.pill) {
          // A faint gray highlight pill behind the label, drawn first.
          labelElement.classList.remove('halo');
          const labelBox = labelElement.getBBox();
          const pillElement = svgElement('rect', { x: labelBox.x - 6, y: labelBox.y - 1, width: labelBox.width + 12, height: labelBox.height + 2,
            rx: (labelBox.height + 2) / 2, fill: '#e9e7e0', 'fill-opacity': 0.7 });
          // Feathered edge, so it reads as a soft highlight hugging the text.
          pillElement.style.filter = 'blur(1.2px)';
          labelElement.before(pillElement);
        }

      } else if (commandType === 'formula') {
        // A big centered equation; x,y default to under the drawing.
        // Without coordinates, it goes centered under everything drawn so far.
        const formulaBounds = drawingBounds();
        addText(mainLayer, command.x ?? (formulaBounds.minX + formulaBounds.maxX) / 2, command.y ?? formulaBounds.maxY + 46, command.text,
          { size: command.size || 34, color: command.color || 'fg' });

      } else if (commandType === 'like') {
        // The everyday thing this shape is like, as a small caption under it,
        // so the analogy is drawn right next to the real concept.
        const targetNode = nodesById[command.id];
        if (!targetNode) { return; }
        const captionMaxCharacters = Math.max(16, Math.floor(targetNode.width / 8));
        addText(targetNode.innerElement, 0, targetNode.height / 2 + 20, '≈ ' + command.text,
          { size: 19, color: 'muted', maxCharacters: captionMaxCharacters });
        // Reserve room for the caption (above and below, so the shape itself
        // stays centered) and let the layout push neighbours away from it.
        const captionHeight = 22 * wrapIntoLines('≈ ' + command.text, captionMaxCharacters).length + 10;
        layoutGraph.setNode(command.id, { width: targetNode.width, height: targetNode.height + captionHeight * 2 });
        relayout();
      }
    }

    // MARK: infinite canvas camera
    // The SVG viewBox is a camera over an unbounded drawing. While following,
    // it eases toward a frame that fits everything drawn so far (never closer
    // than an 800x500 frame). Panning or zooming by hand stops following
    // until a double-click fits everything again.
    const MINIMUM_FRAME = { width: 760, height: 550 };
    const FRAME_PADDING_LEFT = 22, FRAME_PADDING_TOP = 30, FRAME_PADDING_RIGHT = 30, FRAME_PADDING_BOTTOM = 34;
    const camera = { x: -22, y: -110, width: 760, height: 550 };
    let isFollowingDrawing = true;

    function viewportAspectRatio() {
      const boardRect = board.getBoundingClientRect();
      return boardRect.height / Math.max(boardRect.width, 1);
    }

    // The frame with the viewport's shape that fits the drawing (with the
    // heading at its top-left). A small drawing is scaled up to fill the
    // panel's width, up to MAXIMUM_ZOOM_IN, and centered horizontally if it
    // still doesn't fill it; a big one zooms out to fit.
    const MAXIMUM_ZOOM_IN = 1.4;
    // The title and captions don't count toward centering (they sit at the
    // frame's left edge, see positionTitleAndCaptions), only toward its
    // height and minimum width, so there's no feedback between the two.
    function frameFittingDrawing() {
      const drawing = drawingBounds();
      const titleBounds = titleLayer.childElementCount ? titleLayer.getBBox() : null;
      const hasTitleBounds = titleBounds && titleBounds.height > 0;
      const contentTop = hasTitleBounds ? Math.min(drawing.minY, titleBounds.y) : drawing.minY;
      const contentBottom = hasTitleBounds ? Math.max(drawing.maxY, titleBounds.y + titleBounds.height) : drawing.maxY;
      const boardRect = board.getBoundingClientRect();
      const aspectRatio = viewportAspectRatio();
      const paddedContentWidth = Math.max(drawing.maxX - drawing.minX, hasTitleBounds ? titleBounds.width : 0) + FRAME_PADDING_LEFT + FRAME_PADDING_RIGHT;
      let frameWidth = Math.max(paddedContentWidth, (boardRect.width || MINIMUM_FRAME.width) / MAXIMUM_ZOOM_IN);
      let frameHeight = frameWidth * aspectRatio;
      if (frameHeight < contentBottom - contentTop + FRAME_PADDING_TOP + FRAME_PADDING_BOTTOM) {
        frameHeight = contentBottom - contentTop + FRAME_PADDING_TOP + FRAME_PADDING_BOTTOM;
        frameWidth = frameHeight / aspectRatio;
      }
      // Centered on the drawing, but never so far left that the drawing's
      // left edge leaves the padding (a title wider than the drawing).
      const centeredX = (drawing.minX + drawing.maxX) / 2 - frameWidth / 2;
      return { x: Math.min(centeredX, drawing.minX - FRAME_PADDING_LEFT), y: contentTop - FRAME_PADDING_TOP, width: frameWidth, height: frameHeight };
    }

    // The paper grid pans and zooms with the camera; its cell doubles or
    // halves so it always shows as a fine grid of about 12-24pt.
    const GRID_CELL_BOARD_UNITS = 17;
    function moveGridWithCamera() {
      const unitsPerPoint = boardUnitsPerScreenPoint();
      let cellSizeOnScreen = GRID_CELL_BOARD_UNITS / unitsPerPoint;
      while (cellSizeOnScreen < 12) { cellSizeOnScreen *= 2; }
      while (cellSizeOnScreen > 24) { cellSizeOnScreen /= 2; }
      const titleBarHeight = 28;
      document.body.style.backgroundSize = `${cellSizeOnScreen}px ${cellSizeOnScreen}px`;
      document.body.style.backgroundPosition = `${(-camera.x / unitsPerPoint) % cellSizeOnScreen}px ${(titleBarHeight - camera.y / unitsPerPoint) % cellSizeOnScreen}px`;
    }

    // Labels and the title depend on the zoom, so they're refreshed whenever
    // it changes noticeably.
    let zoomUsedForLabels = 0;
    function applyCamera() {
      board.setAttribute('viewBox', `${camera.x} ${camera.y} ${camera.width} ${camera.height}`);
      moveGridWithCamera();
      positionFormulaOverlays();
      const unitsPerPoint = boardUnitsPerScreenPoint();
      if (Math.abs(unitsPerPoint - zoomUsedForLabels) / unitsPerPoint > 0.02) {
        zoomUsedForLabels = unitsPerPoint;
        for (const layer of [mainLayer, edgeLayer]) {
          layer.querySelectorAll('text.halo').forEach((textElement) => { if (textElement.baseFontSize) { applyMinimumLabelSize(textElement); } });
        }
        positionTitleAndCaptions();
      }
    }

    function easeCameraTowardTarget() {
      if (isFollowingDrawing) {
        const cameraTarget = frameFittingDrawing();
        let largestChange = 0;
        for (const key of ['x', 'y', 'width', 'height']) {
          const change = (cameraTarget[key] - camera[key]) * 0.1;
          camera[key] += change;
          largestChange = Math.max(largestChange, Math.abs(change));
        }
        // Once settled, stop touching the viewBox: endless sub-pixel updates
        // re-render the whole board (and its glow filters) every frame.
        if (largestChange > 0.05) { applyCamera(); }
      }
      requestAnimationFrame(easeCameraTowardTarget);
    }

    function stopFollowingDrawing() {
      if (!isFollowingDrawing) { return; }
      isFollowingDrawing = false;
    }

    // Screen point (relative to the board) → drawing coordinates.
    function drawingPointAt(clientX, clientY) {
      const boardRect = board.getBoundingClientRect();
      return [camera.x + (clientX - boardRect.left) / boardRect.width * camera.width,
        camera.y + (clientY - boardRect.top) / boardRect.height * camera.height];
    }

    function zoomCamera(zoomFactor, clientX, clientY) {
      const [anchorX, anchorY] = drawingPointAt(clientX, clientY);
      const newWidth = Math.min(Math.max(camera.width * zoomFactor, 120), 12000);
      const appliedFactor = newWidth / camera.width;
      camera.x = anchorX - (anchorX - camera.x) * appliedFactor;
      camera.y = anchorY - (anchorY - camera.y) * appliedFactor;
      camera.width = newWidth;
      camera.height = newWidth * viewportAspectRatio();
      applyCamera();
    }

    function panCameraByScreenDistance(screenDeltaX, screenDeltaY) {
      const boardRect = board.getBoundingClientRect();
      camera.x += screenDeltaX * camera.width / boardRect.width;
      camera.y += screenDeltaY * camera.height / boardRect.height;
      applyCamera();
    }

    board.addEventListener('wheel', (wheelEvent) => {
      wheelEvent.preventDefault();
      stopFollowingDrawing();
      // Trackpad pinch arrives as a wheel event with ctrlKey set.
      if (wheelEvent.ctrlKey) { zoomCamera(Math.exp(wheelEvent.deltaY * 0.01), wheelEvent.clientX, wheelEvent.clientY); }
      else { panCameraByScreenDistance(wheelEvent.deltaX, wheelEvent.deltaY); }
    }, { passive: false });

    // WebKit also reports pinches as gesture events.
    let gestureStartWidth = null;
    board.addEventListener('gesturestart', (gestureEvent) => { gestureEvent.preventDefault(); stopFollowingDrawing(); gestureStartWidth = camera.width; });
    board.addEventListener('gesturechange', (gestureEvent) => {
      gestureEvent.preventDefault();
      if (gestureStartWidth) { zoomCamera(gestureStartWidth / gestureEvent.scale / camera.width, gestureEvent.clientX, gestureEvent.clientY); }
    });
    board.addEventListener('gestureend', () => { gestureStartWidth = null; });

    let lastPanPointer = null;
    board.addEventListener('mousedown', (mouseEvent) => {
      lastPanPointer = [mouseEvent.clientX, mouseEvent.clientY];
      board.classList.add('panning');
    });
    window.addEventListener('mousemove', (mouseEvent) => {
      if (!lastPanPointer) { return; }
      stopFollowingDrawing();
      panCameraByScreenDistance(lastPanPointer[0] - mouseEvent.clientX, lastPanPointer[1] - mouseEvent.clientY);
      lastPanPointer = [mouseEvent.clientX, mouseEvent.clientY];
    });
    window.addEventListener('mouseup', () => { lastPanPointer = null; board.classList.remove('panning'); });
    board.addEventListener('dblclick', () => { isFollowingDrawing = true; });

    // When the panel is resized, keep the frame filling it.
    new ResizeObserver(() => {
      if (!isFollowingDrawing) { camera.height = camera.width * viewportAspectRatio(); applyCamera(); }
    }).observe(board);

    Object.assign(camera, frameFittingDrawing());
    applyCamera();
    requestAnimationFrame(easeCameraTowardTarget);

    // Once the stream has ended and the pen has drawn everything, the pen can rest out of view.
    function markFinishedIfComplete() {
      if (isStreamFinished && !isDrawingCommand && commandQueue.length === 0 && !pendingPenLeaves.length) {
        isPenStreamFinished = true;
        return true;
      }
      return false;
    }

    // Commands are built as soon as they arrive (hidden), and the pen then
    // reveals them in order, so the pacing comes from the pen, not a timer.
    // Text is measured, so nothing is built until the handwriting font loads.
    let isHandwritingFontReady = false;
    function drawNextQueuedCommand() {
      if (isDrawingCommand || !isHandwritingFontReady) { return; }
      if (commandQueue.length === 0) { markFinishedIfComplete(); return; }
      isDrawingCommand = true;
      const nextCommand = commandQueue.shift();
      try { drawCommand(nextCommand); positionTitleAndCaptions(); queueUnrevealedBoardElements(); } catch (drawError) { console.log(drawError); }
      setTimeout(() => { isDrawingCommand = false; drawNextQueuedCommand(); }, 120);
    }
    document.fonts.load('24px "Patrick Hand"').catch(() => {}).finally(() => { isHandwritingFontReady = true; drawNextQueuedCommand(); });

    window.hb = {
      add(commandLine) {
        const trimmedLine = commandLine.trim();
        if (!trimmedLine.startsWith('{')) { return; }
        try { commandQueue.push(JSON.parse(trimmedLine)); } catch (parseError) { return; }
        drawNextQueuedCommand();
      },
      done() {
        isStreamFinished = true;
        // The pen may still be drawing; check until it has finished.
        const completionCheckTimer = setInterval(() => { if (markFinishedIfComplete()) { clearInterval(completionCheckTimer); } }, 300);
      },
      stopDrawing: stopPenDrawing
    };
    """

    // MARK: - Notes Page

    private static var notesPageHTML: String {
        """
        <!doctype html>
        <html>
        <head>
        <meta charset="utf-8">
        <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; font-src data:">
        <style>
        @font-face { font-family: "Patrick Hand"; src: url(data:font/woff2;base64,\(handwritingFontBase64)) format("woff2"); }
        \(sharedPaperStylesheet)
        \(notesStylesheet)
        \(katexCSSSource)
        </style>
        <script>\(roughJSSource)</script>
        <script>\(katexJSSource)</script>
        <script>window.Prism = window.Prism || {}; Prism.manual = true;</script>
        <script>\(prismJSSource)</script>
        </head>
        <body>
        <div id="titlebar"><span id="topic"></span></div>
        <main id="notes"></main>
        \(penElementHTML)
        <script>\(penAndScratchSoundScript)</script>
        <script>\(notesRendererScript)</script>
        </body>
        </html>
        """
    }

    private static let notesStylesheet = """
    body { color: #2a2a2a; font: 21px/1.4 "Patrick Hand", "Chalkboard SE", system-ui, sans-serif; }
    #notes { position: absolute; inset: 28px 0 0 0; overflow-y: auto; padding: 18px 30px 52px; scroll-behavior: smooth; }
    #notes::-webkit-scrollbar { width: 0; }
    h1 { font: 33px/1.15 "Patrick Hand", sans-serif; margin: 4px 0 14px; color: #2a2a2a;
      -webkit-text-stroke: 1.1px #2a2a2a; letter-spacing: 0.6px; }
    h2 { font: 24px/1.2 "Patrick Hand", sans-serif; margin: 20px 0 6px; color: #8a8578; }
    .bullet { position: relative; padding-left: 24px; margin: 5px 0; }
    .bullet::before { content: ""; position: absolute; left: 5px; top: 0.6em; width: 8px; height: 8px; border-radius: 50%;
      background: #f7edc4; border: 1.3px solid #3a3a3a; box-sizing: border-box; }
    .bullet.level-2 { margin-left: 24px; color: #57534a; }
    .bullet.level-2::before { background: transparent; border-color: #8a8578; }
    mark { background: #f8ecc0; color: inherit; border-radius: 5px; padding: 0 4px; }
    .remember, .analogy { margin: 14px 0; padding: 10px 14px 10px 16px; border-radius: 6px; border-left: 6px solid; }
    .remember { background: #f8ecc0; border-left-color: #e3c35a; }
    .analogy { background: #d6e4f5; border-left-color: #6fa0e3; }
    .remember-label, .analogy-label { display: block; font-size: 17px; color: #8a8578; margin-bottom: 1px; }
    .todo { display: flex; gap: 10px; align-items: flex-start; margin: 6px 0; }
    .todo input { margin-top: 7px; accent-color: #d4a72c; }
    .quote { margin: 10px 0; padding: 2px 0 2px 14px; border-left: 3px solid #dcdcd8; color: #6d685d; }
    .code-block { position: relative; margin: 12px 0; padding: 14px 16px 12px; border-radius: 8px; background: #efebdf;
      border: 1.5px solid #dcd7c9; overflow-x: auto; font: 13px/1.55 "SF Mono", Menlo, monospace; color: #2a2a2a; white-space: pre; }
    .code-language { position: absolute; top: 4px; right: 10px; font: 15px "Patrick Hand", sans-serif; color: #8a8578; }
    .token.comment, .token.prolog { color: #9a9486; font-style: italic; }
    .token.keyword, .token.boolean, .token.important { color: #6a4bd8; }
    .token.string, .token.char, .token.attr-value, .token.template-string { color: #3f8a47; }
    .token.number, .token.constant { color: #c9702a; }
    .token.function, .token.class-name { color: #2f5fc4; }
    .token.operator, .token.punctuation { color: #7a7568; }
    .token.tag, .token.property, .token.builtin { color: #c9604f; }
    .table-wrapper { margin: 12px 0; overflow-x: auto; border-radius: 6px; border: 1.5px solid #8d8d8d; background: #fbf9f3; }
    table { border-collapse: collapse; width: 100%; font-size: 19px; }
    th { text-align: left; font-weight: normal; color: #2a2a2a; background: #d6e4f5; padding: 6px 12px; border-bottom: 1.5px solid #6fa0e3; }
    td { padding: 5px 12px; border-bottom: 1px solid #e4e0d4; }
    tr:last-child td { border-bottom: none; }
    .math-block { margin: 10px 0; font-size: 0.95em; overflow-x: auto; }
    .chart { margin: 12px 0 6px; }
    .chart-title { color: #8a8578; margin-bottom: 2px; }
    .chart svg { width: 100%; height: 190px; overflow: visible; }
    .chart text { font-family: "Patrick Hand", sans-serif; }
    """

    /// Writes streamed note lines one at a time. All text is inserted as text
    /// nodes (==term== becomes a <mark>), never as HTML, so summarized pages
    /// can't inject markup into the panel.
    private static let notesRendererScript = """
    const notesElement = document.getElementById('notes');
    const commandQueue = [];
    let isWritingLine = false;
    let isStreamFinished = false;

    // Appends text to an element, turning ==highlighted== spans into <mark>.
    function appendTextWithHighlights(targetElement, text) {
      String(text).split(/(==[^=]+==)/).forEach((textPart) => {
        if (textPart.startsWith('==') && textPart.endsWith('==') && textPart.length > 4) {
          const markElement = document.createElement('mark');
          markElement.textContent = textPart.slice(2, -2);
          targetElement.appendChild(markElement);
        } else if (textPart) {
          targetElement.appendChild(document.createTextNode(textPart));
        }
      });
    }

    // A hand-drawn bar or line chart in the same rough.js style as the whiteboard.
    function drawSketchChart(command) {
      const chartElement = document.createElement('div');
      chartElement.className = 'chart';
      if (command.title) {
        const titleElement = document.createElement('div');
        titleElement.className = 'chart-title';
        titleElement.textContent = command.title;
        chartElement.appendChild(titleElement);
      }
      const svgNamespace = 'http://www.w3.org/2000/svg';
      const chartSvg = document.createElementNS(svgNamespace, 'svg');
      chartSvg.setAttribute('viewBox', '0 0 600 190');
      chartElement.appendChild(chartSvg);
      const roughChart = rough.svg(chartSvg, { options: { roughness: 0.45, bowing: 0.5, disableMultiStroke: true, disableMultiStrokeFill: true, preserveVertices: true } });
      const values = (command.values || []).map(Number);
      const labels = command.labels || [];
      const maximumValue = Math.max(...values, 1);
      const plotTop = 22, plotBottom = 160, plotLeft = 20, plotRight = 580;
      const slotWidth = (plotRight - plotLeft) / Math.max(values.length, 1);
      // The board's pastels: [fill, outline].
      const chartColors = [['#d6e4f5', '#6fa0e3'], ['#f6d5d0', '#e39a90'], ['#d5efd5', '#8fcf8f'], ['#f8ecc0', '#e3c35a'],
        ['#b9a6f5', '#7c5ce6'], ['#dcdcd8', '#8d8d8d']];
      function addChartText(x, y, text, color) {
        const textElement = document.createElementNS(svgNamespace, 'text');
        textElement.setAttribute('x', x); textElement.setAttribute('y', y);
        textElement.setAttribute('text-anchor', 'middle'); textElement.setAttribute('font-size', 19);
        textElement.setAttribute('fill', color);
        textElement.textContent = text;
        chartSvg.appendChild(textElement);
      }
      chartSvg.appendChild(roughChart.line(plotLeft, plotBottom, plotRight, plotBottom, { stroke: '#3a3a3a', strokeWidth: 1.5 }));
      const pointPositions = values.map((value, valueIndex) => [
        plotLeft + slotWidth * valueIndex + slotWidth / 2, plotBottom - (plotBottom - plotTop) * value / maximumValue ]);
      if (command.kind === 'line') {
        chartSvg.appendChild(roughChart.linearPath(pointPositions, { stroke: '#2f5fc4', strokeWidth: 2.4 }));
        pointPositions.forEach(([pointX, pointY]) => chartSvg.appendChild(roughChart.circle(pointX, pointY, 11, { stroke: '#2f5fc4', fill: '#d6e4f5', fillStyle: 'solid' })));
      } else {
        pointPositions.forEach(([pointX, pointY], valueIndex) => {
          const [barFill, barOutline] = chartColors[valueIndex % chartColors.length];
          chartSvg.appendChild(roughChart.rectangle(pointX - slotWidth * 0.3, pointY, slotWidth * 0.6, plotBottom - pointY,
            { stroke: barOutline, strokeWidth: 2, fill: barFill, fillStyle: 'solid' }));
        });
      }
      pointPositions.forEach(([pointX, pointY], valueIndex) => {
        addChartText(pointX, pointY - 9, (command.prefix || '') + values[valueIndex] + (command.suffix || ''), '#2a2a2a');
        if (labels[valueIndex]) { addChartText(pointX, plotBottom + 22, labels[valueIndex], '#8a8578'); }
      });
      return chartElement;
    }

    function writeLine(command) {
      let lineElement = null;
      if (command.t === 'title') {
        lineElement = document.createElement('h1');
        lineElement.textContent = command.text;
      } else if (command.t === 'heading') {
        lineElement = document.createElement('h2');
        lineElement.textContent = command.text;
      } else if (command.t === 'bullet') {
        lineElement = document.createElement('div');
        lineElement.className = 'bullet' + (command.level === 2 ? ' level-2' : '');
        appendTextWithHighlights(lineElement, command.text);
      } else if (command.t === 'remember') {
        lineElement = document.createElement('div');
        lineElement.className = 'remember';
        const labelElement = document.createElement('span');
        labelElement.className = 'remember-label';
        labelElement.textContent = 'Remember';
        lineElement.appendChild(labelElement);
        appendTextWithHighlights(lineElement, command.text);
      } else if (command.t === 'analogy') {
        lineElement = document.createElement('div');
        lineElement.className = 'analogy';
        const analogyLabel = document.createElement('span');
        analogyLabel.className = 'analogy-label';
        analogyLabel.textContent = 'Think of it like';
        lineElement.appendChild(analogyLabel);
        appendTextWithHighlights(lineElement, command.text);
      } else if (command.t === 'todo') {
        lineElement = document.createElement('label');
        lineElement.className = 'todo';
        const checkboxElement = document.createElement('input');
        checkboxElement.type = 'checkbox';
        const todoTextElement = document.createElement('span');
        appendTextWithHighlights(todoTextElement, command.text);
        lineElement.append(checkboxElement, todoTextElement);
      } else if (command.t === 'quote') {
        lineElement = document.createElement('div');
        lineElement.className = 'quote';
        lineElement.textContent = command.text;
      } else if (command.t === 'code') {
        lineElement = document.createElement('pre');
        lineElement.className = 'code-block';
        const languageName = String(command.lang || '').toLowerCase();
        const codeElement = document.createElement('code');
        // Prism.highlight escapes the source and only adds its own token spans.
        const prismGrammar = window.Prism && Prism.languages && Prism.languages[languageName];
        if (prismGrammar) { codeElement.innerHTML = Prism.highlight(String(command.text || ''), prismGrammar, languageName); }
        else { codeElement.textContent = command.text || ''; }
        const languageLabel = document.createElement('span');
        languageLabel.className = 'code-language';
        languageLabel.textContent = languageName;
        lineElement.append(languageLabel, codeElement);
      } else if (command.t === 'table') {
        lineElement = document.createElement('div');
        lineElement.className = 'table-wrapper';
        const tableElement = document.createElement('table');
        const headerRow = document.createElement('tr');
        (command.columns || []).forEach((columnName) => {
          const headerCell = document.createElement('th');
          headerCell.textContent = columnName;
          headerRow.appendChild(headerCell);
        });
        tableElement.appendChild(headerRow);
        (command.rows || []).forEach((rowValues) => {
          const rowElement = document.createElement('tr');
          (rowValues || []).forEach((cellValue) => {
            const cellElement = document.createElement('td');
            appendTextWithHighlights(cellElement, cellValue);
            rowElement.appendChild(cellElement);
          });
          tableElement.appendChild(rowElement);
        });
        lineElement.appendChild(tableElement);
      } else if (command.t === 'chart') {
        lineElement = drawSketchChart(command);
      } else if (command.t === 'math' && window.katex) {
        lineElement = document.createElement('div');
        lineElement.className = 'math-block';
        katex.render(String(command.tex || ''), lineElement, { throwOnError: false, displayMode: true, trust: false, strict: 'ignore' });
      }
      if (!lineElement) { return; }
      if (command.t === 'title') { setPanelTopic(command.topic || command.text); }
      notesElement.appendChild(lineElement);
      // Hidden until the pen writes it, left to right.
      queuePenLeaves([lineElement]);
    }

    // The page follows the pen, not the build: lines are built as soon as
    // they arrive, so scrolling to the newest one would hide the line the
    // pen is writing (the title, at first) off the top of the page.
    onPenLeafStart = (leafElement) => {
      const notesRect = notesElement.getBoundingClientRect(), leafRect = leafElement.getBoundingClientRect();
      if (leafRect.bottom > notesRect.bottom - 24) {
        notesElement.scrollTop += leafRect.bottom - notesRect.bottom + 48;
      } else if (leafRect.top < notesRect.top) {
        notesElement.scrollTop += leafRect.top - notesRect.top - 12;
      }
    };

    function markFinishedIfComplete() {
      if (isStreamFinished && !isWritingLine && commandQueue.length === 0 && !pendingPenLeaves.length) {
        isPenStreamFinished = true;
        return true;
      }
      return false;
    }

    // Lines are built as they arrive; the pen paces how fast they appear.
    function writeNextQueuedLine() {
      if (isWritingLine) { return; }
      if (commandQueue.length === 0) { markFinishedIfComplete(); return; }
      isWritingLine = true;
      try { writeLine(commandQueue.shift()); } catch (writeError) { console.log(writeError); }
      setTimeout(() => { isWritingLine = false; writeNextQueuedLine(); }, 60);
    }

    window.hb = {
      add(commandLine) {
        const trimmedLine = commandLine.trim();
        if (!trimmedLine.startsWith('{')) { return; }
        try { commandQueue.push(JSON.parse(trimmedLine)); } catch (parseError) { return; }
        writeNextQueuedLine();
      },
      done() {
        isStreamFinished = true;
        // The pen may still be writing; check until it has finished.
        const completionCheckTimer = setInterval(() => { if (markFinishedIfComplete()) { clearInterval(completionCheckTimer); } }, 300);
      },
      stopDrawing: stopPenDrawing
    };
    """
}

/// The panel's content view. Reports when the pointer enters or leaves the
/// panel so the close and zoom lights can appear only on hover, like Clicky's.
final class PanelHoverTrackingView: NSView {
    var onHoverChanged: ((Bool) -> Void)?

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        // activeAlways: the panel never becomes key, so hover has to be
        // tracked even while another app is active.
        addTrackingArea(NSTrackingArea(
            rect: .zero,
            options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
            owner: self,
            userInfo: nil
        ))
    }

    override func mouseEntered(with event: NSEvent) {
        onHoverChanged?(true)
    }

    override func mouseExited(with event: NSEvent) {
        onHoverChanged?(false)
    }
}

/// A transparent strip over the page-drawn title bar that lets the window be
/// dragged from there (the web view underneath would otherwise eat the click).
final class WindowDragStripView: NSView {
    override var mouseDownCanMoveWindow: Bool { true }

    override func mouseDown(with event: NSEvent) {
        window?.performDrag(with: event)
    }
}
