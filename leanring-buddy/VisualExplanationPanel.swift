//
//  VisualExplanationPanel.swift
//  leanring-buddy
//
//  A floating whiteboard near the cursor where Claude sketches an animated
//  explanation live, Excalidraw-style. Claude streams one JSON drawing
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

    /// Opens a blank board beside the cursor, ready for streamed commands.
    func beginLiveDrawing() {
        presentPanelNearCursor()
        isBoardPageReady = false
        pendingDrawingCommandLines.removeAll()
        webView?.loadHTMLString(Self.boardPageHTML, baseURL: nil)
    }

    /// Draws one command line. Lines that aren't JSON objects, like stray
    /// prose or code fences, are ignored by the board.
    func addDrawingCommandLine(_ drawingCommandLine: String) {
        guard isBoardPageReady else {
            pendingDrawingCommandLines.append(drawingCommandLine)
            return
        }
        callBoardFunction("add", withStringArgument: drawingCommandLine)
    }

    /// Hides the "drawing…" indicator once every step has been drawn.
    func finishLiveDrawing() {
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
        </style>
        <script>\(roughJSSource)</script>
        <script>const ICON_SHAPES = \(iconShapesJSON);</script>
        </head>
        <body>
        <svg id="board" viewBox="0 0 800 500" preserveAspectRatio="xMidYMid meet">
          <g id="zone-layer"></g>
          <g id="main-layer"></g>
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
    .hb-pulse { transform-box: fill-box; transform-origin: center; animation: hb-pulse 1.4s infinite ease-in-out; }
    .hb-shake { animation: hb-shake 0.6s ease-in-out 2; }
    .hb-grow { transform-box: fill-box; transform-origin: bottom; animation: hb-grow 0.8s ease-out both; }
    @keyframes hb-appear { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: none; } }
    @keyframes hb-pop { from { opacity: 0; transform: scale(0.3); } 70% { transform: scale(1.12); } to { opacity: 1; transform: scale(1); } }
    @keyframes hb-pulse { 0%,100% { transform: scale(1); filter: none; } 50% { transform: scale(1.05); filter: drop-shadow(0 0 10px #a855f7); } }
    @keyframes hb-shake { 0%,100% { transform: translateX(0); } 20% { transform: translateX(-6px); } 40% { transform: translateX(6px); } 60% { transform: translateX(-4px); } 80% { transform: translateX(4px); } }
    @keyframes hb-grow { from { transform: scaleY(0); } to { transform: scaleY(1); } }
    @keyframes hb-blink { 0%,100% { opacity: 0.35; } 50% { opacity: 1; } }
    """

    /// Draws handed-over commands one at a time with a short gap, so even a
    /// burst of commands still looks like it's being sketched live.
    private static let boardRendererScript = """
    const SVG_NS = 'http://www.w3.org/2000/svg';
    const board = document.getElementById('board');
    const zoneLayer = document.getElementById('zone-layer');
    const mainLayer = document.getElementById('main-layer');
    // Things that move forever (flows) live here so they never affect auto-zoom.
    const motionLayer = document.getElementById('motion-layer');
    const roughCanvas = rough.svg(board);
    const HAND_FONT = 'Virgil, "SF Pro Rounded", system-ui, sans-serif';
    const COLORS = { purple: '#a855f7', blue: '#60a5fa', green: '#4ade80', orange: '#fb923c', pink: '#f472b6',
      cyan: '#22d3ee', yellow: '#facc15', red: '#f87171', fg: '#f4f2f7', muted: '#a59fb3' };
    const elementsById = {};
    const arrowsById = {};
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
      const { size = 17, color = 'fg', anchor = 'middle', maxCharacters = 0, fill } = options || {};
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
      textElement.classList.add('hb-appear');
      if (fill) { textElement.classList.add('on-fill'); }
      parentElement.appendChild(textElement);
      return textElement;
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
      const strokeOptions = { stroke: colorOf(colorName, 'fg'), strokeWidth: 2.3 / scale, roughness: 0.45, bowing: 0.4 };
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

    function shapeOptions(colorName) {
      const strokeColor = colorOf(colorName, 'purple');
      return { stroke: strokeColor, strokeWidth: 2.4, roughness: 1.4, bowing: 1.2,
        fill: translucent(strokeColor, 0.2), fillStyle: 'hachure', hachureGap: 7, fillWeight: 1.1, hachureAngle: -41 };
    }

    // Icon on top, label underneath, or whichever of the two is given.
    function drawIconAndLabel(parentElement, centerX, centerY, availableWidth, availableHeight, command) {
      const hasIcon = command.icon && ICON_SHAPES[command.icon];
      if (hasIcon && command.label) {
        const iconSize = Math.min(38, availableHeight * 0.42);
        drawIcon(parentElement, command.icon, centerX, centerY - iconSize * 0.45, iconSize, command.color);
        addText(parentElement, centerX, centerY + iconSize * 0.62, command.label,
          { size: 17, maxCharacters: Math.max(8, Math.floor(availableWidth / 9)) });
      } else if (hasIcon) {
        drawIcon(parentElement, command.icon, centerX, centerY, Math.min(48, availableHeight * 0.6), command.color);
      } else if (command.label) {
        addText(parentElement, centerX, centerY, command.label, { size: 18, maxCharacters: Math.max(8, Math.floor(availableWidth / 9)) });
      }
    }

    function registerElement(id, element) { if (id) { elementsById[id] = element; } }

    // A point on the element's outline in the direction of (towardX, towardY),
    // so arrows start and end at edges instead of centers.
    function outlinePointToward(element, towardX, towardY) {
      const deltaX = towardX - element.centerX, deltaY = towardY - element.centerY;
      if (element.radius) {
        const distance = Math.hypot(deltaX, deltaY) || 1;
        return [element.centerX + deltaX / distance * (element.radius + 7), element.centerY + deltaY / distance * (element.radius + 7)];
      }
      const halfWidth = element.width / 2 + 7, halfHeight = element.height / 2 + 7;
      const scale = 1 / Math.max(Math.abs(deltaX) / halfWidth, Math.abs(deltaY) / halfHeight, 1e-6);
      return [element.centerX + deltaX * scale, element.centerY + deltaY * scale];
    }

    // Quadratic curve through start, end, and a point bent sideways at the middle.
    function arrowGeometry(fromElement, toElement, bend) {
      const centerDeltaX = toElement.centerX - fromElement.centerX, centerDeltaY = toElement.centerY - fromElement.centerY;
      const centerDistance = Math.hypot(centerDeltaX, centerDeltaY) || 1;
      const perpendicularX = -centerDeltaY / centerDistance, perpendicularY = centerDeltaX / centerDistance;
      const aimX = (fromElement.centerX + toElement.centerX) / 2 + perpendicularX * bend * centerDistance;
      const aimY = (fromElement.centerY + toElement.centerY) / 2 + perpendicularY * bend * centerDistance;
      const [startX, startY] = outlinePointToward(fromElement, aimX, aimY);
      const [endX, endY] = outlinePointToward(toElement, aimX, aimY);
      const chordLength = Math.hypot(endX - startX, endY - startY) || 1;
      const midX = (startX + endX) / 2 + perpendicularX * bend * chordLength;
      const midY = (startY + endY) / 2 + perpendicularY * bend * chordLength;
      // Bezier control point that makes the curve pass through (midX, midY).
      const controlX = 2 * midX - (startX + endX) / 2, controlY = 2 * midY - (startY + endY) / 2;
      const pointAt = (progress) => [
        (1 - progress) ** 2 * startX + 2 * (1 - progress) * progress * controlX + progress ** 2 * endX,
        (1 - progress) ** 2 * startY + 2 * (1 - progress) * progress * controlY + progress ** 2 * endY ];
      return { startX, startY, endX, endY, midX, midY, controlX, controlY, perpendicularX, perpendicularY, pointAt };
    }

    function drawCommand(command) {
      const commandType = command.t;

      if (commandType === 'title') {
        addText(mainLayer, 400, 28, command.text, { size: 30, color: command.color || 'fg' });

      } else if (commandType === 'zone') {
        const zoneColor = colorOf(command.color, 'blue');
        const groupElement = svgElement('g');
        groupElement.appendChild(roughCanvas.path(roundedRectanglePath(command.x, command.y, command.w, command.h, 18),
          { stroke: zoneColor, strokeWidth: 1.8, roughness: 1.1, strokeLineDash: [9, 7], fill: translucent(zoneColor, 0.06), fillStyle: 'solid' }));
        zoneLayer.appendChild(groupElement);
        drawStrokesIn(groupElement, 0.8);
        if (command.label) { addText(zoneLayer, command.x + 14, command.y + 18, command.label, { size: 16, color: command.color || 'blue', anchor: 'start' }); }

      } else if (commandType === 'box' || commandType === 'circle') {
        const isCircle = commandType === 'circle';
        const width = command.w || 140, height = command.h || 90, radius = command.r || 50;
        const centerX = isCircle ? command.x : command.x + width / 2;
        const centerY = isCircle ? command.y : command.y + height / 2;
        const groupElement = svgElement('g');
        groupElement.appendChild(isCircle
          ? roughCanvas.circle(centerX, centerY, radius * 2, shapeOptions(command.color))
          : roughCanvas.path(roundedRectanglePath(command.x, command.y, width, height, 14), shapeOptions(command.color)));
        mainLayer.appendChild(groupElement);
        drawStrokesIn(groupElement);
        drawIconAndLabel(groupElement, centerX, centerY, isCircle ? radius * 1.5 : width, isCircle ? radius * 1.6 : height, command);
        registerElement(command.id, { centerX, centerY, width: isCircle ? radius * 2 : width, height: isCircle ? radius * 2 : height,
          radius: isCircle ? radius : 0, groupElement });

      } else if (commandType === 'icon') {
        const iconSize = command.size || 54;
        const groupElement = svgElement('g');
        mainLayer.appendChild(groupElement);
        drawIcon(groupElement, command.name, command.x, command.y, iconSize, command.color || 'fg');
        if (command.label) { addText(groupElement, command.x, command.y + iconSize * 0.5 + 16, command.label, { size: 16, maxCharacters: 16 }); }
        registerElement(command.id, { centerX: command.x, centerY: command.y, width: iconSize, height: iconSize,
          radius: iconSize * 0.6, groupElement });

      } else if (commandType === 'arrow') {
        const fromElement = elementsById[command.from], toElement = elementsById[command.to];
        if (!fromElement || !toElement) { return; }
        const geometry = arrowGeometry(fromElement, toElement, command.bend || 0);
        const strokeOptions = { stroke: colorOf(command.color, 'muted'), strokeWidth: 2.3, roughness: 1.1, bowing: 0.8,
          strokeLineDash: command.style === 'dashed' ? [8, 7] : undefined };
        const headAngle = Math.atan2(geometry.endY - geometry.controlY, geometry.endX - geometry.controlX);
        const headLength = 13;
        const groupElement = svgElement('g');
        groupElement.appendChild(command.bend
          ? roughCanvas.curve([[geometry.startX, geometry.startY], [geometry.midX, geometry.midY], [geometry.endX, geometry.endY]], strokeOptions)
          : roughCanvas.line(geometry.startX, geometry.startY, geometry.endX, geometry.endY, strokeOptions));
        const solidHeadOptions = { ...strokeOptions, strokeLineDash: undefined };
        for (const headSide of [-0.45, 0.45]) {
          groupElement.appendChild(roughCanvas.line(geometry.endX, geometry.endY,
            geometry.endX - headLength * Math.cos(headAngle + headSide), geometry.endY - headLength * Math.sin(headAngle + headSide), solidHeadOptions));
        }
        mainLayer.appendChild(groupElement);
        drawStrokesIn(groupElement, 0.6);
        const labelOffset = command.bend ? Math.sign(command.bend) * 18 : 18;
        const labelX = geometry.midX + geometry.perpendicularX * labelOffset, labelY = geometry.midY + geometry.perpendicularY * labelOffset;
        if (command.label) { addText(mainLayer, labelX, labelY, command.label, { size: 15, color: command.color || 'muted', maxCharacters: 18 }); }
        arrowsById[command.id || (command.from + '>' + command.to)] = { ...geometry, labelX, labelY };

      } else if (commandType === 'text') {
        addText(mainLayer, command.x, command.y, command.text, { size: command.size || 17, color: command.color || 'muted', maxCharacters: command.wrap || 0 });

      } else if (commandType === 'note') {
        // A slightly tilted sticky note, optionally tied to an element.
        const noteWidth = command.w || 170;
        const noteLines = wrapIntoLines(command.text, Math.floor(noteWidth / 9));
        const noteHeight = 26 + noteLines.length * 19;
        const noteColor = colorOf(command.color, 'yellow');
        if (command.to && elementsById[command.to]) {
          const targetElement = elementsById[command.to];
          const [targetX, targetY] = outlinePointToward(targetElement, command.x + noteWidth / 2, command.y + noteHeight / 2);
          const connectorGroup = svgElement('g');
          connectorGroup.appendChild(roughCanvas.line(command.x + noteWidth / 2, command.y + noteHeight / 2, targetX, targetY,
            { stroke: noteColor, strokeWidth: 1.6, roughness: 1.2, strokeLineDash: [5, 6] }));
          mainLayer.appendChild(connectorGroup);
          drawStrokesIn(connectorGroup);
        }
        const noteGroup = svgElement('g', { transform: `rotate(-2 ${command.x + noteWidth / 2} ${command.y + noteHeight / 2})` });
        noteGroup.appendChild(roughCanvas.rectangle(command.x, command.y, noteWidth, noteHeight,
          { stroke: noteColor, strokeWidth: 1.6, roughness: 1.3, fill: noteColor, fillStyle: 'solid' }));
        noteGroup.classList.add('hb-pop');
        mainLayer.appendChild(noteGroup);
        addText(noteGroup, command.x + noteWidth / 2, command.y + noteHeight / 2, command.text,
          { size: 16, fill: '#1c1917', maxCharacters: Math.floor(noteWidth / 9) });

      } else if (commandType === 'step') {
        // Numbered badge on an element's top-left corner, or beside an arrow's label.
        const targetElement = elementsById[command.on];
        const targetArrow = arrowsById[command.on];
        let badgeX, badgeY;
        if (targetElement) { badgeX = targetElement.centerX - targetElement.width / 2 - 2; badgeY = targetElement.centerY - targetElement.height / 2 - 2; }
        else if (targetArrow) {
          // Near the arrow's start, on the side away from its label.
          const [pointX, pointY] = targetArrow.pointAt(0.18);
          badgeX = pointX - targetArrow.perpendicularX * 18; badgeY = pointY - targetArrow.perpendicularY * 18;
        }
        else { return; }
        const badgeColor = colorOf(command.color, 'orange');
        const badgeGroup = svgElement('g');
        badgeGroup.appendChild(roughCanvas.circle(badgeX, badgeY, 28, { stroke: badgeColor, strokeWidth: 2, roughness: 1, fill: badgeColor, fillStyle: 'solid' }));
        addText(badgeGroup, badgeX, badgeY + 1, command.n, { size: 17, fill: '#1c1917' });
        badgeGroup.classList.add('hb-pop');
        mainLayer.appendChild(badgeGroup);

      } else if (commandType === 'highlight' || commandType === 'cross' || commandType === 'tick') {
        const targetElement = elementsById[command.id];
        if (!targetElement) { return; }
        const markGroup = svgElement('g');
        const halfWidth = targetElement.width / 2, halfHeight = targetElement.height / 2;
        if (commandType === 'highlight') {
          // Marker scribble around the element.
          markGroup.appendChild(roughCanvas.ellipse(targetElement.centerX, targetElement.centerY, targetElement.width + 46, targetElement.height + 40,
            { stroke: colorOf(command.color, 'yellow'), strokeWidth: 3, roughness: 2.2, bowing: 2 }));
        } else if (commandType === 'cross') {
          const crossOptions = { stroke: colorOf(command.color, 'red'), strokeWidth: 4, roughness: 1.6 };
          markGroup.appendChild(roughCanvas.line(targetElement.centerX - halfWidth, targetElement.centerY - halfHeight,
            targetElement.centerX + halfWidth, targetElement.centerY + halfHeight, crossOptions));
          markGroup.appendChild(roughCanvas.line(targetElement.centerX + halfWidth, targetElement.centerY - halfHeight,
            targetElement.centerX - halfWidth, targetElement.centerY + halfHeight, crossOptions));
        } else {
          const tickX = targetElement.centerX + halfWidth - 4, tickY = targetElement.centerY - halfHeight + 4;
          markGroup.appendChild(roughCanvas.linearPath([[tickX - 14, tickY], [tickX - 4, tickY + 11], [tickX + 16, tickY - 14]],
            { stroke: colorOf(command.color, 'green'), strokeWidth: 4.5, roughness: 1.2 }));
        }
        mainLayer.appendChild(markGroup);
        drawStrokesIn(markGroup, 0.8);

      } else if (commandType === 'bars') {
        // Bars that grow from the baseline, with labels and values.
        const values = command.values || [];
        const maximumValue = Math.max(...values, 1);
        const slotWidth = command.w / Math.max(values.length, 1);
        const barWidth = slotWidth * 0.58;
        values.forEach((value, barIndex) => {
          const barHeight = (command.h - 30) * value / maximumValue;
          const barX = command.x + barIndex * slotWidth + (slotWidth - barWidth) / 2;
          const barY = command.y + command.h - 22 - barHeight;
          const barColorName = (command.colors && command.colors[barIndex]) || command.color || 'purple';
          const barGroup = svgElement('g');
          barGroup.appendChild(roughCanvas.rectangle(barX, barY, barWidth, barHeight, shapeOptions(barColorName)));
          barGroup.classList.add('hb-grow');
          barGroup.style.animationDelay = `${barIndex * 0.18}s`;
          mainLayer.appendChild(barGroup);
          addText(mainLayer, barX + barWidth / 2, barY - 12, (command.prefix || '') + value + (command.suffix || ''), { size: 15, color: barColorName });
          if (command.labels && command.labels[barIndex]) {
            addText(mainLayer, barX + barWidth / 2, command.y + command.h - 6, command.labels[barIndex], { size: 14, color: 'muted' });
          }
        });

      } else if (commandType === 'flow') {
        // Something travelling along an arrow on a loop: an icon, or a glowing dot.
        const arrow = arrowsById[command.along];
        if (!arrow) { return; }
        const travellerGroup = svgElement('g');
        if (command.icon && ICON_SHAPES[command.icon]) {
          drawIcon(travellerGroup, command.icon, 0, 0, 24, command.color || 'yellow');
        } else {
          const dotElement = svgElement('circle', { cx: 0, cy: 0, r: 6, fill: colorOf(command.color, 'yellow') });
          dotElement.style.filter = `drop-shadow(0 0 6px ${colorOf(command.color, 'yellow')})`;
          travellerGroup.appendChild(dotElement);
        }
        const [firstX, firstY] = arrow.pointAt(0);
        travellerGroup.setAttribute('transform', `translate(${firstX},${firstY})`);
        travellerGroup.style.opacity = 0;
        motionLayer.appendChild(travellerGroup);
        const travelMilliseconds = (command.seconds || 1.8) * 1000;
        const pauseMilliseconds = 450;
        const loopStartTime = performance.now();
        function moveTraveller(currentTime) {
          const loopProgress = ((currentTime - loopStartTime) % (travelMilliseconds + pauseMilliseconds)) / travelMilliseconds;
          const travelProgress = Math.min(loopProgress, 1);
          const [travellerX, travellerY] = arrow.pointAt(travelProgress);
          travellerGroup.setAttribute('transform', `translate(${travellerX},${travellerY})`);
          travellerGroup.style.opacity = loopProgress > 1 ? 0 : Math.min(1, travelProgress * 8, (1 - travelProgress) * 8);
          requestAnimationFrame(moveTraveller);
        }
        requestAnimationFrame(moveTraveller);

      } else if (commandType === 'pulse' || commandType === 'shake') {
        const targetElement = elementsById[command.id];
        if (targetElement) { targetElement.groupElement.classList.add(commandType === 'pulse' ? 'hb-pulse' : 'hb-shake'); }
      }
    }

    // MARK: infinite canvas camera
    // The SVG viewBox is a camera over an unbounded drawing. While following,
    // it eases toward a frame that fits everything drawn so far (never closer
    // than the 800x500 starting area). Panning or zooming by hand stops
    // following until a double-click fits everything again.
    const STARTING_AREA = { x: 0, y: 0, width: 800, height: 500 };
    const FRAME_PADDING = 36;
    const camera = { ...STARTING_AREA };
    let cameraTarget = { ...STARTING_AREA };
    let isFollowingDrawing = true;

    function viewportAspectRatio() {
      const boardRect = board.getBoundingClientRect();
      return boardRect.height / Math.max(boardRect.width, 1);
    }

    function drawnContentBounds() {
      let minX = STARTING_AREA.x, minY = STARTING_AREA.y;
      let maxX = STARTING_AREA.x + STARTING_AREA.width, maxY = STARTING_AREA.y + STARTING_AREA.height;
      for (const layer of [zoneLayer, mainLayer]) {
        if (!layer.childElementCount) { continue; }
        const layerBounds = layer.getBBox();
        minX = Math.min(minX, layerBounds.x); minY = Math.min(minY, layerBounds.y);
        maxX = Math.max(maxX, layerBounds.x + layerBounds.width); maxY = Math.max(maxY, layerBounds.y + layerBounds.height);
      }
      return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
    }

    // The smallest frame with the viewport's shape that contains everything.
    function frameFittingDrawing() {
      const contentBounds = drawnContentBounds();
      const aspectRatio = viewportAspectRatio();
      let frameWidth = contentBounds.width + FRAME_PADDING * 2;
      let frameHeight = frameWidth * aspectRatio;
      if (frameHeight < contentBounds.height + FRAME_PADDING * 2) {
        frameHeight = contentBounds.height + FRAME_PADDING * 2;
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
        cameraTarget = frameFittingDrawing();
        for (const key of ['x', 'y', 'width', 'height']) { camera[key] += (cameraTarget[key] - camera[key]) * 0.12; }
        applyCamera();
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
      try { drawCommand(nextCommand); } catch (drawError) { console.log(drawError); }
      setTimeout(() => { isDrawingCommand = false; drawNextQueuedCommand(); }, 280);
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
}
