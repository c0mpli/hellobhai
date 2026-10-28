//
//  VisualExplanationPanel.swift
//  leanring-buddy
//
//  A floating panel near the cursor where Claude explains things live while
//  narrating: either a whiteboard it sketches on (drawings) or a notes page
//  it writes on (summaries, lists, key points). The whiteboard is
//  Excalidraw-style. Claude streams one JSON drawing
//  command per line (zones, boxes, icons, arrows, notes, step badges, flows,
//  charts…); each command is drawn as it's handed over, in hand-drawn
//  rough.js strokes and Excalidraw's Virgil handwriting font, animating in as
//  if drawn by a pen. Icons are real vector drawings (Lucide), never emoji.
//
//  The web view is locked down: a CSP blocks all network loads, navigation
//  away from the page is refused, storage is non-persistent, and commands are
//  passed in as JSON data (parsed with JSON.parse, never evaluated as code).
//

import AppKit
import WebKit

@MainActor
final class VisualExplanationPanelManager: NSObject, WKNavigationDelegate {
    private static let panelContentSize = NSSize(width: 720, height: 490)

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

    private static let virgilFontBase64: String = bundledResourceData(named: "Virgil", withExtension: "woff2")?
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

    /// Hides the "drawing…" / "writing…" indicator once every step is shown.
    func finishLiveContent() {
        guard isBoardPageReady else {
            pendingDrawingCommandLines.append(Self.finishedDrawingMarker)
            return
        }
        webView?.evaluateJavaScript("hb.done()")
    }

    func close() {
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
            styleMask: [.titled, .closable, .resizable, .nonactivatingPanel, .fullSizeContentView, .utilityWindow],
            backing: .buffered,
            defer: false
        )
        createdPanel.title = "hellobhai"
        createdPanel.titlebarAppearsTransparent = true
        createdPanel.isFloatingPanel = true
        createdPanel.level = .floating
        createdPanel.hidesOnDeactivate = false
        createdPanel.becomesKeyOnlyIfNeeded = true
        createdPanel.isReleasedWhenClosed = false
        // Resize freely; the canvas refits to any shape, but keep it usable.
        createdPanel.minSize = NSSize(width: 360, height: 260)
        createdPanel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        createdPanel.backgroundColor = NSColor(red: 0.071, green: 0.071, blue: 0.086, alpha: 1)

        let webViewConfiguration = WKWebViewConfiguration()
        // Nothing drawn here should ever be persisted to disk.
        webViewConfiguration.websiteDataStore = .nonPersistent()
        let createdWebView = WKWebView(frame: createdPanel.contentLayoutRect, configuration: webViewConfiguration)
        createdWebView.autoresizingMask = [.width, .height]
        createdWebView.navigationDelegate = self
        createdWebView.setValue(false, forKey: "drawsBackground")
        createdPanel.contentView = createdWebView

        panel = createdPanel
        webView = createdWebView
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
        @font-face { font-family: Virgil; src: url(data:font/woff2;base64,\(virgilFontBase64)) format("woff2"); }
        \(boardStylesheet)
        \(katexCSSSource)
        </style>
        <script>\(roughJSSource)</script>
        <script>\(dagreJSSource)</script>
        <script>\(katexJSSource)</script>
        <script>const ICON_SHAPES = \(iconShapesJSON);</script>
        </head>
        <body>
        <svg id="board" viewBox="0 0 800 500" preserveAspectRatio="xMidYMid meet">
          <g id="zone-layer"></g>
          <g id="edge-layer"></g>
          <g id="main-layer"></g>
          <g id="title-layer"></g>
          <g id="motion-layer"></g>
        </svg>
        <div id="drawing-indicator">drawing…</div>
        <div id="navigation-hint">scroll or drag to move · pinch to zoom · double-click to fit</div>
        <script>\(boardRendererScript)</script>
        </body>
        </html>
        """
    }

    private static let boardStylesheet = """
    html, body { margin: 0; height: 100%; background: #121216; overflow: hidden; }
    #board { position: absolute; inset: 26px 0 0 0; width: 100%; height: calc(100% - 26px); cursor: grab; touch-action: none; }
    #board.panning { cursor: grabbing; }
    #navigation-hint { position: absolute; left: 14px; bottom: 8px; color: #6f6a7d; font: 13px Virgil, sans-serif;
      opacity: 0; transition: opacity 0.6s; pointer-events: none; }
    #navigation-hint.visible { opacity: 1; }
    #drawing-indicator { position: absolute; right: 14px; bottom: 8px; color: #8b8598; font: 15px Virgil, sans-serif;
      animation: hb-blink 1.1s infinite ease-in-out; transition: opacity 0.4s; }
    #drawing-indicator.finished { animation: none; opacity: 0; }
    /* A dark outline behind every label keeps it readable over strokes. */
    text { stroke: #121216; stroke-width: 4px; stroke-linejoin: round; paint-order: stroke; }
    /* Text sitting on a filled shape (sticky notes, step badges) needs no outline. */
    text.on-fill { stroke: none; }
    .hb-appear { animation: hb-appear 0.45s ease-out both; }
    .hb-pop { transform-box: fill-box; transform-origin: center; animation: hb-pop 0.45s ease-out both; }
    .hb-pulse { transform-box: fill-box; transform-origin: center; animation: hb-pulse 1.4s 3 ease-in-out; }
    .hb-shake { animation: hb-shake 0.6s ease-in-out 2; }
    .hb-glow { filter: drop-shadow(0 0 6px #facc15) drop-shadow(0 0 2px #facc15); }
    .hb-grow { transform-box: fill-box; transform-origin: bottom; animation: hb-grow 0.8s ease-out both; }
    @keyframes hb-appear { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: none; } }
    @keyframes hb-pop { from { opacity: 0; transform: scale(0.3); } 70% { transform: scale(1.12); } to { opacity: 1; transform: scale(1); } }
    @keyframes hb-pulse { 0%,100% { transform: scale(1); filter: none; } 50% { transform: scale(1.05); filter: drop-shadow(0 0 10px #a855f7); } }
    @keyframes hb-shake { 0%,100% { transform: translateX(0); } 20% { transform: translateX(-6px); } 40% { transform: translateX(6px); } 60% { transform: translateX(-4px); } 80% { transform: translateX(4px); } }
    @keyframes hb-grow { from { transform: scaleY(0); } to { transform: scaleY(1); } }
    @keyframes hb-blink { 0%,100% { opacity: 0.35; } 50% { opacity: 1; } }
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
    const roughCanvas = rough.svg(board);
    const HAND_FONT = 'Virgil, "SF Pro Rounded", system-ui, sans-serif';
    const COLORS = { purple: '#a855f7', blue: '#60a5fa', green: '#4ade80', orange: '#fb923c', pink: '#f472b6',
      cyan: '#22d3ee', yellow: '#facc15', red: '#f87171', fg: '#f4f2f7', muted: '#a59fb3' };
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

    function colorOf(colorName, fallbackName) { return COLORS[colorName] || COLORS[fallbackName] || COLORS.purple; }
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

    function drawStrokesIn(groupElement, durationSeconds) {
      groupElement.querySelectorAll('path').forEach((pathElement) => {
        const strokeLength = pathElement.getTotalLength();
        pathElement.style.strokeDasharray = strokeLength;
        pathElement.style.strokeDashoffset = strokeLength;
        pathElement.getBoundingClientRect();
        pathElement.style.transition = `stroke-dashoffset ${durationSeconds || 0.55}s ease-out`;
        pathElement.style.strokeDashoffset = 0;
      });
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
      const { size = 17, color = 'fg', anchor = 'middle', maxCharacters = 0, fill, animate = true } = options || {};
      const lines = maxCharacters ? wrapIntoLines(text, maxCharacters) : [String(text)];
      const lineHeight = size * 1.15;
      const textElement = svgElement('text', { x, y: centerY - (lines.length - 1) * lineHeight / 2,
        'text-anchor': anchor, 'dominant-baseline': 'middle', 'font-size': size, 'font-family': HAND_FONT,
        fill: fill || colorOf(color, 'fg') });
      lines.forEach((lineText, lineIndex) => {
        const lineElement = svgElement('tspan', { x, dy: lineIndex === 0 ? 0 : lineHeight });
        lineElement.textContent = lineText;
        textElement.appendChild(lineElement);
      });
      if (animate) { textElement.classList.add('hb-appear'); }
      if (fill) { textElement.classList.add('on-fill'); }
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
      const strokeOptions = { stroke: colorOf(colorName, 'fg'), strokeWidth: 2.3 / scale, roughness: 0.45, bowing: 0.4, seed: stableSeed(iconName) };
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
      drawStrokesIn(groupElement, 0.7);
      return groupElement;
    }

    function shapeOptions(colorName, seedText) {
      const strokeColor = colorOf(colorName, 'purple');
      return { stroke: strokeColor, strokeWidth: 2.4, roughness: 1.4, bowing: 1.2, seed: stableSeed(seedText),
        fill: translucent(strokeColor, 0.2), fillStyle: 'hachure', hachureGap: 7, fillWeight: 1.1, hachureAngle: -41 };
    }

    // MARK: node sizes and drawing (all in local coordinates centered on 0,0)

    function nodeSize(command) {
      const labelLength = String(command.label || '').length;
      if (command.t === 'circle') { const diameter = (command.r || 54) * 2; return [diameter, diameter]; }
      if (command.t === 'icon') { return [Math.min(Math.max(labelLength * 9 + 24, 96), 180), 104]; }
      if (command.t === 'note') {
        const noteWidth = command.w || 200;
        return [noteWidth, 30 + wrapIntoLines(command.text, Math.floor(noteWidth / 9)).length * 19];
      }
      if (command.t === 'bars') { return [command.w || 270, command.h || 180]; }
      const boxWidth = Math.min(Math.max(labelLength * 9.5 + 50, 140), 240);
      const labelLineCount = wrapIntoLines(command.label || '', Math.floor(boxWidth / 9.5)).length;
      return [boxWidth, (command.icon ? 96 : 66) + (labelLineCount - 1) * 19];
    }

    function drawIconAndLabel(parentElement, width, height, command) {
      const hasIcon = command.icon && ICON_SHAPES[command.icon];
      const maxCharacters = Math.max(8, Math.floor(width / 9.5));
      if (hasIcon && command.label) {
        const iconSize = Math.min(36, height * 0.4);
        drawIcon(parentElement, command.icon, 0, -height / 2 + 12 + iconSize / 2, iconSize, command.color);
        addText(parentElement, 0, height / 2 - 12 - (wrapIntoLines(command.label, maxCharacters).length - 1) * 9.5 - 10, command.label,
          { size: 17, maxCharacters });
      } else if (hasIcon) {
        drawIcon(parentElement, command.icon, 0, 0, Math.min(46, height * 0.6), command.color);
      } else if (command.label) {
        addText(parentElement, 0, 0, command.label, { size: 18, maxCharacters });
      }
    }

    function drawNodeContents(innerElement, command, width, height) {
      if (command.t === 'circle') {
        innerElement.appendChild(roughCanvas.circle(0, 0, width, shapeOptions(command.color, command.id)));
        drawStrokesIn(innerElement);
        drawIconAndLabel(innerElement, width * 0.78, height * 0.82, command);
      } else if (command.t === 'icon') {
        drawIcon(innerElement, command.name || command.icon, 0, -14, 54, command.color || 'fg');
        if (command.label) { addText(innerElement, 0, 34, command.label, { size: 16, maxCharacters: 16 }); }
      } else if (command.t === 'note') {
        const noteColor = colorOf(command.color, 'yellow');
        const noteGroup = svgElement('g', { transform: 'rotate(-2)' });
        noteGroup.appendChild(roughCanvas.rectangle(-width / 2, -height / 2, width, height,
          { stroke: noteColor, strokeWidth: 1.6, roughness: 1.3, fill: noteColor, fillStyle: 'solid', seed: stableSeed(command.text) }));
        addText(noteGroup, 0, 0, command.text, { size: 16, fill: '#1c1917', maxCharacters: Math.floor(width / 9) });
        noteGroup.classList.add('hb-pop');
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
          barGroup.classList.add('hb-grow');
          barGroup.style.animationDelay = `${barIndex * 0.18}s`;
          innerElement.appendChild(barGroup);
          addText(innerElement, barX + barWidth / 2, barY - 12, (command.prefix || '') + value + (command.suffix || ''), { size: 15, color: barColorName });
          if (command.labels && command.labels[barIndex]) {
            addText(innerElement, barX + barWidth / 2, height / 2 - 6, command.labels[barIndex], { size: 14, color: 'muted' });
          }
        });
      } else {
        innerElement.appendChild(roughCanvas.path(roundedRectanglePath(-width / 2, -height / 2, width, height, 14), shapeOptions(command.color, command.id)));
        drawStrokesIn(innerElement);
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
      const strokeOptions = { stroke: isConnector ? colorOf('yellow') : colorOf(edge.command.color, 'muted'),
        strokeWidth: isConnector ? 1.6 : 2.3, roughness: 1.1, bowing: 0.8, seed: stableSeed(edgeId),
        strokeLineDash: (isConnector || edge.command.style === 'dashed') ? [8, 7] : undefined };
      const groupElement = svgElement('g');
      const pointPairs = points.map((point) => [point.x, point.y]);
      groupElement.appendChild(pointPairs.length > 2 ? roughCanvas.curve(pointPairs, strokeOptions) : roughCanvas.line(...pointPairs[0], ...pointPairs[1], strokeOptions));
      if (!isConnector) {
        const lastPoint = points[points.length - 1], previousPoint = points[points.length - 2];
        const headAngle = Math.atan2(lastPoint.y - previousPoint.y, lastPoint.x - previousPoint.x);
        const headOptions = { ...strokeOptions, strokeLineDash: undefined };
        for (const headSide of [-0.45, 0.45]) {
          groupElement.appendChild(roughCanvas.line(lastPoint.x, lastPoint.y,
            lastPoint.x - 13 * Math.cos(headAngle + headSide), lastPoint.y - 13 * Math.sin(headAngle + headSide), headOptions));
        }
      }
      edgeLayer.appendChild(groupElement);
      edge.element = groupElement;
      edge.geometry = polylineGeometry(points);
      if (edge.command.label) {
        // Just above the middle of the arrow, so the line never runs through it.
        const [middleX, middleY] = edge.geometry.pointAt(0.5);
        const [perpendicularX, perpendicularY] = edge.geometry.perpendicularAt(0.5);
        const offsetSide = perpendicularY > 0 ? -1 : 1;
        edge.labelElement = addText(edgeLayer, middleX + perpendicularX * 15 * offsetSide, middleY + perpendicularY * 15 * offsetSide,
          edge.command.label, { size: 15, color: edge.command.color || 'muted', maxCharacters: 18, animate: edge.isNew });
      }
      if (edge.isNew) { drawStrokesIn(groupElement, 0.6); edge.isNew = false; }
    }

    function redrawGroup(groupId) {
      const group = groupsById[groupId];
      const layoutGroup = layoutGraph.node(groupId);
      if (!layoutGroup || layoutGroup.width === undefined) { return; }
      if (group.element) { group.element.remove(); }
      const zoneColor = colorOf(group.command.color, 'blue');
      const zoneX = layoutGroup.x - layoutGroup.width / 2 - 14, zoneY = layoutGroup.y - layoutGroup.height / 2 - 34;
      const zoneWidth = layoutGroup.width + 28, zoneHeight = layoutGroup.height + 48;
      const groupElement = svgElement('g');
      groupElement.appendChild(roughCanvas.path(roundedRectanglePath(zoneX, zoneY, zoneWidth, zoneHeight, 18),
        { stroke: zoneColor, strokeWidth: 1.8, roughness: 1.1, strokeLineDash: [9, 7], fill: translucent(zoneColor, 0.06),
          fillStyle: 'solid', seed: stableSeed(groupId) }));
      if (group.command.label) {
        addText(groupElement, zoneX + 14, zoneY + 18, group.command.label, { size: 16, color: group.command.color || 'blue', anchor: 'start', animate: group.isNew });
      }
      zoneLayer.appendChild(groupElement);
      group.element = groupElement;
      if (group.isNew) { drawStrokesIn(groupElement, 0.8); group.isNew = false; }
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

    // Title centered above everything drawn (flowchart or geometry, including
    // arrows that arc over the top), captions centered below it.
    // Bounds of everything drawn except the title and captions.
    function drawingBounds() {
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      for (const layer of [zoneLayer, edgeLayer, mainLayer]) {
        if (!layer.childElementCount) { continue; }
        const layerBounds = layer.getBBox();
        if (!layerBounds.width && !layerBounds.height) { continue; }
        minX = Math.min(minX, layerBounds.x); minY = Math.min(minY, layerBounds.y);
        maxX = Math.max(maxX, layerBounds.x + layerBounds.width); maxY = Math.max(maxY, layerBounds.y + layerBounds.height);
      }
      if (minX === Infinity) { return { minX: 0, maxX: 800, minY: 60, maxY: 440 }; }
      return { minX, maxX, minY, maxY };
    }

    function positionTitleAndCaptions() {
      const { minX, maxX, minY, maxY } = drawingBounds();
      const centerX = (minX + maxX) / 2;
      if (titleElement) { titleElement.setAttribute('transform', `translate(${centerX},${minY - 40})`); }
      captionElements.forEach((captionElement, captionIndex) => {
        captionElement.setAttribute('transform', `translate(${centerX},${maxY + 34 + captionIndex * 28})`);
      });
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
          node.outerElement.style.transform = `translate(${layoutNode.x}px,${layoutNode.y}px)`;
          node.outerElement.getBoundingClientRect();
          node.outerElement.style.transition = `transform ${GLIDE_SECONDS}s ease-in-out`;
          node.outerElement.style.opacity = 1;
          drawNodeContents(node.innerElement, node.command, node.width, node.height);
          node.isPlaced = true;
        } else {
          node.outerElement.style.transform = `translate(${layoutNode.x}px,${layoutNode.y}px)`;
        }
      }
      positionTitleAndCaptions();
      clearTimeout(edgeRedrawTimer);
      edgeRedrawTimer = setTimeout(() => {
        Object.keys(groupsById).forEach(redrawGroup);
        Object.keys(edgesById).forEach(redrawEdge);
        positionStepBadges();
        positionTitleAndCaptions();
      }, GLIDE_SECONDS * 1000);
    }

    // MARK: commands

    function drawCommand(command) {
      const commandType = command.t;

      if (commandType === 'title') {
        titleElement = svgElement('g');
        addText(titleElement, 0, 0, command.text, { size: 30, color: command.color || 'fg' });
        titleLayer.appendChild(titleElement);
        positionTitleAndCaptions();

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
        addText(captionElement, 0, 0, command.text, { size: command.size || 17, color: command.color || 'muted', maxCharacters: 60 });
        titleLayer.appendChild(captionElement);
        captionElements.push(captionElement);
        positionTitleAndCaptions();

      } else if (commandType === 'step') {
        const badgeColor = colorOf(command.color, 'orange');
        const badgeElement = svgElement('g');
        badgeElement.appendChild(roughCanvas.circle(0, 0, 28, { stroke: badgeColor, strokeWidth: 2, roughness: 1, fill: badgeColor, fillStyle: 'solid', seed: stableSeed('step' + command.n) }));
        addText(badgeElement, 0, 1, command.n, { size: 17, fill: '#1c1917' });
        const badgeContainer = svgElement('g');
        badgeContainer.appendChild(badgeElement);
        badgeElement.classList.add('hb-pop');
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
          markGroup.appendChild(roughCanvas.ellipse(0, 0, targetNode.width + 46, targetNode.height + 40,
            { stroke: colorOf(command.color, 'yellow'), strokeWidth: 3, roughness: 2.2, bowing: 2 }));
        } else if (commandType === 'cross') {
          const crossOptions = { stroke: colorOf(command.color, 'red'), strokeWidth: 4, roughness: 1.6 };
          markGroup.appendChild(roughCanvas.line(-halfWidth, -halfHeight, halfWidth, halfHeight, crossOptions));
          markGroup.appendChild(roughCanvas.line(halfWidth, -halfHeight, -halfWidth, halfHeight, crossOptions));
        } else {
          markGroup.appendChild(roughCanvas.linearPath([[halfWidth - 18, -halfHeight + 4], [halfWidth - 8, -halfHeight + 15], [halfWidth + 12, -halfHeight - 10]],
            { stroke: colorOf(command.color, 'green'), strokeWidth: 4.5, roughness: 1.2 }));
        }
        targetNode.innerElement.appendChild(markGroup);
        drawStrokesIn(markGroup, 0.8);

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
        const polygonColor = colorOf(command.color, 'purple');
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.polygon(polygonPoints, {
          stroke: polygonColor, strokeWidth: 2.6, roughness: 0.6, bowing: 0.4, seed: stableSeed(JSON.stringify(polygonPoints)),
          fill: command.fill === false ? undefined : translucent(polygonColor, 0.18), fillStyle: 'hachure', hachureGap: 8 }));
        mainLayer.appendChild(groupElement);
        drawStrokesIn(groupElement, 0.8);
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
        const squareColor = colorOf(command.color, 'blue');
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.polygon(squarePoints, { stroke: squareColor, strokeWidth: 2.4, roughness: 0.6, bowing: 0.3,
          seed: stableSeed(JSON.stringify(squarePoints)), fill: translucent(squareColor, 0.16), fillStyle: 'solid' }));
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
        drawStrokesIn(groupElement, 0.9);
        if (command.label) {
          addText(mainLayer, (fromX + toX) / 2 + normalX / 2, (fromY + toY) / 2 + normalY / 2, command.label,
            { size: command.size || 22, color: command.color || 'blue' });
        }
        registerGeometryShape(command.id, squarePoints, groupElement);

      } else if (commandType === 'line') {
        const [fromX, fromY] = command.from, [toX, toY] = command.to;
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.line(fromX, fromY, toX, toY, { stroke: colorOf(command.color, 'fg'), strokeWidth: command.width || 2.4,
          roughness: 0.5, seed: stableSeed(JSON.stringify([command.from, command.to])), strokeLineDash: command.style === 'dashed' ? [8, 7] : undefined }));
        mainLayer.appendChild(groupElement);
        drawStrokesIn(groupElement, 0.6);
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
        groupElement.classList.add('hb-appear');
        mainLayer.appendChild(groupElement);
        if (command.label) { addText(mainLayer, command.x + (command.dx ?? 14), command.y + (command.dy ?? -14), command.label, { size: 18, color: command.color || 'fg' }); }

      } else if (commandType === 'ring') {
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.circle(command.x, command.y, command.r * 2, { stroke: colorOf(command.color, 'purple'), strokeWidth: 2.4,
          roughness: 0.5, seed: stableSeed('ring' + command.x + command.y + command.r), fill: command.fill ? translucent(colorOf(command.color, 'purple'), 0.15) : undefined, fillStyle: 'solid' }));
        mainLayer.appendChild(groupElement);
        drawStrokesIn(groupElement, 0.8);

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
        drawStrokesIn(groupElement, 0.5);

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
          const gridOptions = { stroke: 'rgba(165,159,179,0.18)', strokeWidth: 1, roughness: 0.2, seed: 11 };
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
        drawStrokesIn(groupElement, 0.6);
        if (command.xLabel) { addText(mainLayer, frame.left + frame.width + 26, xAxisBoardY, command.xLabel, { size: 17, color: 'muted' }); }
        if (command.yLabel) { addText(mainLayer, yAxisBoardX, frame.top - 26, command.yLabel, { size: 17, color: 'muted' }); }
        (command.xTicks || []).forEach((tick) => {
          const [tickX] = frameToBoard(frame, tick[0], xAxisMathY);
          addText(mainLayer, tickX, xAxisBoardY + 18, tick[1], { size: 14, color: 'muted' });
        });
        (command.yTicks || []).forEach((tick) => {
          const [, tickY] = frameToBoard(frame, yAxisMathX, tick[0]);
          addText(mainLayer, yAxisBoardX - 14, tickY, tick[1], { size: 14, color: 'muted', anchor: 'end' });
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
        drawStrokesIn(groupElement, 1.1);
        const allCurvePoints = curveSegments.flat();
        if (command.label) {
          // Curve labels go in a stacked legend at the axes' top-right corner,
          // so several plots never pile their labels on top of each other.
          frame.legendCount = (frame.legendCount || 0) + 1;
          const legendX = frame.left + frame.width + 30, legendY = frame.top + (frame.legendCount - 1) * 26;
          const legendGroup = svgElement('g');
          legendGroup.appendChild(roughCanvas.line(legendX, legendY, legendX + 22, legendY, { stroke: curveColor, strokeWidth: 3, roughness: 0.3,
            strokeLineDash: command.style === 'dashed' ? [6, 5] : undefined, seed: stableSeed('legend' + command.expr) }));
          addText(legendGroup, legendX + 30, legendY, command.label, { size: 16, color: command.color || 'purple', anchor: 'start' });
          mainLayer.appendChild(legendGroup);
        }
        registerGeometryShape(command.id, allCurvePoints, groupElement);

      } else if (commandType === 'path') {
        // An open curve through points in board coordinates (polygon closes, path doesn't).
        const pathPoints = command.points || [];
        if (pathPoints.length < 2) { return; }
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.curve(pathPoints, { stroke: colorOf(command.color, 'purple'), strokeWidth: command.width || 2.6,
          roughness: 0.4, seed: stableSeed(JSON.stringify(pathPoints)), strokeLineDash: command.style === 'dashed' ? [9, 7] : undefined }));
        mainLayer.appendChild(groupElement);
        drawStrokesIn(groupElement, 0.9);
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
        drawStrokesIn(groupElement, 1.2);
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
        groupElement.appendChild(roughCanvas.polygon(regionPoints, { stroke: 'none', fill: translucent(areaColor, 0.35), fillStyle: 'hachure',
          hachureGap: 6, fillWeight: 1.4, roughness: 0.6, seed: stableSeed('area' + command.expr + areaFrom + areaTo) }));
        groupElement.classList.add('hb-appear');
        mainLayer.insertBefore(groupElement, mainLayer.firstChild);
        if (command.label) {
          const [labelX, labelY] = frameToBoard(frame, (areaFrom + areaTo) / 2, (upperFunction((areaFrom + areaTo) / 2) + lowerFunction((areaFrom + areaTo) / 2)) / 2);
          addText(mainLayer, labelX, labelY, command.label, { size: 18, color: command.color || 'yellow' });
        }
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
        drawStrokesIn(groupElement, 0.7);
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
        groupElement.classList.add('hb-appear');
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
        drawStrokesIn(groupElement, 0.6);
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
        drawStrokesIn(groupElement, 0.7);

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
          const sectorColor = colorOf((command.colors && command.colors[sectorIndex]) || pieColors[sectorIndex % pieColors.length]);
          const sectorPoints = [[pieX, pieY]];
          for (let stepIndex = 0; stepIndex <= 40; stepIndex += 1) {
            const stepAngle = startAngle + sweepAngle * stepIndex / 40;
            sectorPoints.push([pieX + Math.cos(stepAngle) * pieRadius, pieY + Math.sin(stepAngle) * pieRadius]);
          }
          groupElement.appendChild(roughCanvas.polygon(sectorPoints, { stroke: sectorColor, strokeWidth: 2.2, roughness: 0.6,
            fill: translucent(sectorColor, 0.3), fillStyle: 'hachure', hachureGap: 7, seed: stableSeed('pie' + sectorIndex + value) }));
          if (command.labels && command.labels[sectorIndex]) {
            const middleAngle = startAngle + sweepAngle / 2;
            addText(groupElement, pieX + Math.cos(middleAngle) * pieRadius * 1.25, pieY + Math.sin(middleAngle) * pieRadius * 1.25,
              command.labels[sectorIndex], { size: 17, color: (command.colors && command.colors[sectorIndex]) || pieColors[sectorIndex % pieColors.length] });
          }
          startAngle += sweepAngle;
        });
        mainLayer.appendChild(groupElement);
        drawStrokesIn(groupElement, 0.9);
        registerGeometryShape(command.id, [[pieX - pieRadius, pieY - pieRadius], [pieX + pieRadius, pieY + pieRadius]], groupElement);

      } else if (commandType === 'latex') {
        // Properly typeset math (fractions, integrals, sums, matrices, roots)
        // via KaTeX, embedded in the board and centered at x, y.
        if (!window.katex) { return; }
        const formulaSize = (command.size || 1.3) * 22;
        const foreignElement = svgElement('foreignObject', { x: 0, y: 0, width: 1600, height: 400 });
        const formulaElement = document.createElement('div');
        formulaElement.style.cssText = `display:inline-block;white-space:nowrap;font-size:${formulaSize}px;color:${colorOf(command.color, 'fg')}`;
        try {
          katex.render(String(command.tex || ''), formulaElement, { throwOnError: false, displayMode: true, trust: false, strict: 'ignore' });
        } catch (formulaError) { console.log(formulaError); return; }
        foreignElement.appendChild(formulaElement);
        foreignElement.classList.add('hb-appear');
        // Measure it in the motion layer first, so its own placeholder size
        // doesn't count toward the bounds used to place it.
        motionLayer.appendChild(foreignElement);
        const formulaWidth = formulaElement.offsetWidth + 4, formulaHeight = formulaElement.offsetHeight + 4;
        foreignElement.setAttribute('width', formulaWidth);
        foreignElement.setAttribute('height', formulaHeight);
        // Without coordinates, it goes centered under everything drawn so far.
        const formulaBounds = drawingBounds();
        const formulaCenterX = command.x ?? (formulaBounds.minX + formulaBounds.maxX) / 2;
        const formulaCenterY = command.y ?? formulaBounds.maxY + formulaHeight / 2 + 24;
        foreignElement.setAttribute('x', formulaCenterX - formulaWidth / 2);
        foreignElement.setAttribute('y', formulaCenterY - formulaHeight / 2);
        mainLayer.appendChild(foreignElement);

      } else if (commandType === 'label') {
        addText(mainLayer, command.x, command.y, command.text, { size: command.size || 20, color: command.color || 'fg', maxCharacters: command.wrap || 0 });

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
          { size: 15, color: 'yellow', maxCharacters: captionMaxCharacters });
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
    const MINIMUM_FRAME = { width: 800, height: 500 };
    const FRAME_PADDING = 36;
    const camera = { x: 0, y: -60, width: 800, height: 500 };
    let isFollowingDrawing = true;

    function viewportAspectRatio() {
      const boardRect = board.getBoundingClientRect();
      return boardRect.height / Math.max(boardRect.width, 1);
    }

    function drawnContentBounds() {
      let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
      for (const layer of [zoneLayer, edgeLayer, mainLayer, titleLayer]) {
        if (!layer.childElementCount) { continue; }
        const layerBounds = layer.getBBox();
        if (!layerBounds.width && !layerBounds.height) { continue; }
        minX = Math.min(minX, layerBounds.x); minY = Math.min(minY, layerBounds.y);
        maxX = Math.max(maxX, layerBounds.x + layerBounds.width); maxY = Math.max(maxY, layerBounds.y + layerBounds.height);
      }
      if (minX === Infinity) { return { x: 0, y: -60, width: MINIMUM_FRAME.width, height: MINIMUM_FRAME.height }; }
      return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
    }

    // The smallest frame with the viewport's shape that contains everything.
    function frameFittingDrawing() {
      const contentBounds = drawnContentBounds();
      const aspectRatio = viewportAspectRatio();
      let frameWidth = Math.max(contentBounds.width + FRAME_PADDING * 2, MINIMUM_FRAME.width);
      let frameHeight = frameWidth * aspectRatio;
      if (frameHeight < Math.max(contentBounds.height + FRAME_PADDING * 2, MINIMUM_FRAME.height * 0.8)) {
        frameHeight = Math.max(contentBounds.height + FRAME_PADDING * 2, MINIMUM_FRAME.height * 0.8);
        frameWidth = frameHeight / aspectRatio;
      }
      return { x: contentBounds.x + contentBounds.width / 2 - frameWidth / 2,
        y: contentBounds.y + contentBounds.height / 2 - frameHeight / 2, width: frameWidth, height: frameHeight };
    }

    function applyCamera() {
      board.setAttribute('viewBox', `${camera.x} ${camera.y} ${camera.width} ${camera.height}`);
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
      document.getElementById('navigation-hint').classList.remove('visible');
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
    document.getElementById('navigation-hint').classList.add('visible');
    setTimeout(() => document.getElementById('navigation-hint').classList.remove('visible'), 5000);

    // Hide "drawing…" once the stream has ended and every queued shape is drawn.
    function hideDrawingIndicatorIfComplete() {
      if (isStreamFinished && !isDrawingCommand && commandQueue.length === 0) {
        document.getElementById('drawing-indicator').classList.add('finished');
      }
    }

    function drawNextQueuedCommand() {
      if (isDrawingCommand) { return; }
      if (commandQueue.length === 0) { hideDrawingIndicatorIfComplete(); return; }
      isDrawingCommand = true;
      const nextCommand = commandQueue.shift();
      try { drawCommand(nextCommand); positionTitleAndCaptions(); } catch (drawError) { console.log(drawError); }
      setTimeout(() => { isDrawingCommand = false; drawNextQueuedCommand(); }, 300);
    }

    window.hb = {
      add(commandLine) {
        const trimmedLine = commandLine.trim();
        if (!trimmedLine.startsWith('{')) { return; }
        try { commandQueue.push(JSON.parse(trimmedLine)); } catch (parseError) { return; }
        drawNextQueuedCommand();
      },
      done() {
        isStreamFinished = true;
        hideDrawingIndicatorIfComplete();
      }
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
        @font-face { font-family: Virgil; src: url(data:font/woff2;base64,\(virgilFontBase64)) format("woff2"); }
        \(notesStylesheet)
        \(katexCSSSource)
        </style>
        <script>\(roughJSSource)</script>
        <script>\(katexJSSource)</script>
        <script>window.Prism = window.Prism || {}; Prism.manual = true;</script>
        <script>\(prismJSSource)</script>
        </head>
        <body>
        <main id="notes"></main>
        <div id="drawing-indicator">writing…</div>
        <script>\(notesRendererScript)</script>
        </body>
        </html>
        """
    }

    private static let notesStylesheet = """
    html, body { margin: 0; height: 100%; background: #121216; color: #ece9f2; overflow: hidden;
      font: 15.5px/1.5 -apple-system, "SF Pro Rounded", system-ui, sans-serif; }
    #notes { position: absolute; inset: 30px 0 0 0; overflow-y: auto; padding: 6px 30px 48px; scroll-behavior: smooth; }
    #notes::-webkit-scrollbar { width: 0; }
    #drawing-indicator { position: absolute; right: 14px; bottom: 8px; color: #8b8598; font: 15px Virgil, sans-serif;
      animation: hb-blink 1.1s infinite ease-in-out; transition: opacity 0.4s; }
    #drawing-indicator.finished { animation: none; opacity: 0; }
    h1 { font: 30px/1.2 Virgil, sans-serif; margin: 6px 0 14px; color: #f4f2f7; }
    h2 { font: 21px/1.25 Virgil, sans-serif; margin: 20px 0 8px; color: #c084fc; }
    .bullet { position: relative; padding-left: 20px; margin: 6px 0; }
    .bullet::before { content: ""; position: absolute; left: 4px; top: 0.62em; width: 7px; height: 7px; border-radius: 50%; background: #a855f7; }
    .bullet.level-2 { margin-left: 22px; color: #c9c4d4; }
    .bullet.level-2::before { background: transparent; border: 1.5px solid #a855f7; width: 5px; height: 5px; }
    mark { background: rgba(250,204,21,0.22); color: #fde68a; border-radius: 4px; padding: 0 3px; }
    .remember { margin: 14px 0; padding: 12px 14px; border-radius: 12px; background: rgba(250,204,21,0.12);
      border: 1.5px solid rgba(250,204,21,0.55); color: #fef3c7; }
    .remember-label { display: block; font: 15px Virgil, sans-serif; color: #facc15; margin-bottom: 2px; }
    .analogy { margin: 12px 0 16px; padding: 12px 14px; border-radius: 12px; background: rgba(96,165,250,0.1);
      border: 1.5px dashed rgba(96,165,250,0.6); color: #dbeafe; }
    .analogy-label { display: block; font: 16px Virgil, sans-serif; color: #60a5fa; margin-bottom: 2px; }
    .todo { display: flex; gap: 10px; align-items: flex-start; margin: 7px 0; }
    .todo input { margin-top: 4px; accent-color: #a855f7; }
    .quote { margin: 10px 0; padding: 4px 0 4px 14px; border-left: 3px solid #60a5fa; color: #bfdbfe; font-style: italic; }
    .code-block { position: relative; margin: 12px 0; padding: 14px 16px 12px; border-radius: 12px; background: #1a1922;
      border: 1px solid #2c2a38; overflow-x: auto; font: 13px/1.55 "SF Mono", Menlo, monospace; color: #e4e1ec; white-space: pre; }
    .code-language { position: absolute; top: 6px; right: 10px; font: 12px Virgil, sans-serif; color: #8b8598; }
    .token.comment, .token.prolog { color: #6f6a7d; font-style: italic; }
    .token.keyword, .token.boolean, .token.important { color: #c084fc; }
    .token.string, .token.char, .token.attr-value, .token.template-string { color: #86efac; }
    .token.number, .token.constant { color: #fdba74; }
    .token.function, .token.class-name { color: #60a5fa; }
    .token.operator, .token.punctuation { color: #a59fb3; }
    .token.tag, .token.property, .token.builtin { color: #f472b6; }
    .table-wrapper { margin: 12px 0; overflow-x: auto; border-radius: 12px; border: 1px solid #2c2a38; }
    table { border-collapse: collapse; width: 100%; font-size: 14.5px; }
    th { text-align: left; font: 16px Virgil, sans-serif; color: #c084fc; background: #1a1922; padding: 9px 12px; border-bottom: 1px solid #2c2a38; }
    td { padding: 8px 12px; border-bottom: 1px solid #22212c; }
    tr:last-child td { border-bottom: none; }
    .math-block { margin: 10px 0; font-size: 1.15em; overflow-x: auto; }
    .chart { margin: 12px 0 6px; }
    .chart-title { font: 16px Virgil, sans-serif; color: #c9c4d4; margin-bottom: 2px; }
    .chart svg { width: 100%; height: 190px; overflow: visible; }
    .chart text { font-family: Virgil, sans-serif; }
    .writing-in { animation: hb-write 0.7s ease-out both; }
    @keyframes hb-grow { from { transform: scaleY(0); } to { transform: scaleY(1); } }
    @keyframes hb-write { from { opacity: 0.2; clip-path: inset(0 100% 0 0); transform: translateY(4px); }
      to { opacity: 1; clip-path: inset(0 0 0 0); transform: none; } }
    @keyframes hb-blink { 0%,100% { opacity: 0.35; } 50% { opacity: 1; } }
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
      const roughChart = rough.svg(chartSvg);
      const values = (command.values || []).map(Number);
      const labels = command.labels || [];
      const maximumValue = Math.max(...values, 1);
      const plotTop = 22, plotBottom = 160, plotLeft = 20, plotRight = 580;
      const slotWidth = (plotRight - plotLeft) / Math.max(values.length, 1);
      const chartColors = ['#a855f7', '#60a5fa', '#4ade80', '#fb923c', '#f472b6', '#22d3ee', '#facc15'];
      function addChartText(x, y, text, color) {
        const textElement = document.createElementNS(svgNamespace, 'text');
        textElement.setAttribute('x', x); textElement.setAttribute('y', y);
        textElement.setAttribute('text-anchor', 'middle'); textElement.setAttribute('font-size', 15);
        textElement.setAttribute('fill', color);
        textElement.textContent = text;
        chartSvg.appendChild(textElement);
      }
      chartSvg.appendChild(roughChart.line(plotLeft, plotBottom, plotRight, plotBottom, { stroke: '#4a4658', roughness: 1 }));
      const pointPositions = values.map((value, valueIndex) => [
        plotLeft + slotWidth * valueIndex + slotWidth / 2, plotBottom - (plotBottom - plotTop) * value / maximumValue ]);
      if (command.kind === 'line') {
        chartSvg.appendChild(roughChart.linearPath(pointPositions, { stroke: '#c084fc', strokeWidth: 2.6, roughness: 1.2 }));
        pointPositions.forEach(([pointX, pointY]) => chartSvg.appendChild(roughChart.circle(pointX, pointY, 10, { stroke: '#c084fc', fill: '#c084fc', fillStyle: 'solid' })));
      } else {
        pointPositions.forEach(([pointX, pointY], valueIndex) => {
          const barColor = chartColors[valueIndex % chartColors.length];
          const barNode = roughChart.rectangle(pointX - slotWidth * 0.3, pointY, slotWidth * 0.6, plotBottom - pointY,
            { stroke: barColor, strokeWidth: 2, roughness: 1.3, fill: barColor + '40', fillStyle: 'hachure', hachureGap: 6 });
          barNode.style.transformBox = 'fill-box'; barNode.style.transformOrigin = 'bottom';
          barNode.style.animation = `hb-grow 0.8s ${valueIndex * 0.15}s ease-out both`;
          chartSvg.appendChild(barNode);
        });
      }
      pointPositions.forEach(([pointX, pointY], valueIndex) => {
        addChartText(pointX, pointY - 9, (command.prefix || '') + values[valueIndex] + (command.suffix || ''), '#ece9f2');
        if (labels[valueIndex]) { addChartText(pointX, plotBottom + 22, labels[valueIndex], '#8b8598'); }
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
      lineElement.classList.add('writing-in');
      notesElement.appendChild(lineElement);
      notesElement.scrollTop = notesElement.scrollHeight;
    }

    function hideWritingIndicatorIfComplete() {
      if (isStreamFinished && !isWritingLine && commandQueue.length === 0) {
        document.getElementById('drawing-indicator').classList.add('finished');
      }
    }

    function writeNextQueuedLine() {
      if (isWritingLine) { return; }
      if (commandQueue.length === 0) { hideWritingIndicatorIfComplete(); return; }
      isWritingLine = true;
      try { writeLine(commandQueue.shift()); } catch (writeError) { console.log(writeError); }
      setTimeout(() => { isWritingLine = false; writeNextQueuedLine(); }, 320);
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
        hideWritingIndicatorIfComplete();
      }
    };
    """
}
