//
//  VisualExplanationPanel.swift
//  leanring-buddy
//
//  A floating whiteboard near the cursor where Claude sketches an animated
//  explanation live, Excalidraw-style. Claude streams one JSON drawing
//  command per line (box, arrow, icon, text, flow, pulse…); each command is
//  drawn the moment its line arrives, in hand-drawn rough.js strokes that
//  animate in as if drawn by a pen. Nothing waits for the whole response.
//
//  The web view is locked down: a CSP blocks all network loads, navigation
//  away from the page is refused, storage is non-persistent, and commands are
//  passed in as JSON data (parsed with JSON.parse, never evaluated as code).
//

import AppKit
import WebKit

@MainActor
final class VisualExplanationPanelManager: NSObject, WKNavigationDelegate {
    private static let panelContentSize = NSSize(width: 580, height: 450)

    private var panel: NSPanel?
    private var webView: WKWebView?

    /// Commands that arrive before the board page has finished loading.
    private var pendingDrawingCommandLines: [String] = []
    private var isBoardPageReady = false

    /// rough.js, loaded once from the app bundle and inlined into the page so
    /// the panel never needs file or network access.
    private static let roughJSScriptTag: String = {
        guard let roughJSURL = Bundle.main.url(forResource: "rough", withExtension: "js"),
              let roughJSSource = try? String(contentsOf: roughJSURL, encoding: .utf8) else {
            print("⚠️ Visual panel: missing bundled rough.js")
            return ""
        }
        return "<script>\(roughJSSource)</script>"
    }()

    /// Opens a blank board near the cursor, ready for streamed commands.
    func beginLiveDrawing() {
        presentPanelNearCursorIfNeeded()
        isBoardPageReady = false
        pendingDrawingCommandLines.removeAll()
        webView?.loadHTMLString(Self.boardPageHTML, baseURL: nil)
    }

    /// Draws one streamed line (one JSON command). Lines that aren't JSON
    /// objects, like stray prose or code fences, are ignored by the board.
    func addDrawingCommandLine(_ drawingCommandLine: String) {
        guard isBoardPageReady else {
            pendingDrawingCommandLines.append(drawingCommandLine)
            return
        }
        callBoardFunction("add", withStringArgument: drawingCommandLine)
    }

    /// Hides the "drawing…" indicator once the stream ends.
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

    private func presentPanelNearCursorIfNeeded() {
        if panel == nil {
            createPanel()
        }
        guard let panel else { return }
        if !panel.isVisible {
            panel.setFrameOrigin(Self.panelOriginNearCursor(panelSize: panel.frame.size))
        }
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
        createdPanel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        createdPanel.backgroundColor = NSColor(red: 0.063, green: 0.071, blue: 0.067, alpha: 1)

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

    /// Below-right of the cursor, kept fully on the cursor's screen.
    private static func panelOriginNearCursor(panelSize: NSSize) -> NSPoint {
        let mouseLocation = NSEvent.mouseLocation
        let cursorScreen = NSScreen.screens.first(where: { $0.frame.contains(mouseLocation) }) ?? NSScreen.main
        let visibleFrame = cursorScreen?.visibleFrame ?? NSRect(origin: .zero, size: panelSize)

        var panelOrigin = NSPoint(x: mouseLocation.x + 40, y: mouseLocation.y - panelSize.height - 30)
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
        <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'">
        <style>\(boardStylesheet)</style>
        \(roughJSScriptTag)
        </head>
        <body>
        <svg id="board" viewBox="0 0 560 400" preserveAspectRatio="xMidYMid meet"></svg>
        <div id="drawing-indicator">✏️ drawing…</div>
        <script>\(boardRendererScript)</script>
        </body>
        </html>
        """
    }

    private static let boardStylesheet = """
    html, body { margin: 0; height: 100%; background: #101211; overflow: hidden;
      font-family: -apple-system, "SF Pro Rounded", system-ui, sans-serif; }
    #board { position: absolute; inset: 28px 8px 8px 8px; width: calc(100% - 16px); height: calc(100% - 36px); }
    #drawing-indicator { position: absolute; right: 14px; bottom: 10px; color: #a59fb3; font-size: 12px;
      animation: hb-blink 1.1s infinite ease-in-out; transition: opacity 0.4s; }
    #drawing-indicator.finished { opacity: 0; }
    text { font-family: -apple-system, "SF Pro Rounded", system-ui, sans-serif; }
    .hb-appear { animation: hb-appear 0.45s ease-out both; }
    .hb-pulse { transform-box: fill-box; transform-origin: center; animation: hb-pulse 1.4s infinite ease-in-out; }
    @keyframes hb-appear { from { opacity: 0; transform: translateY(6px); } to { opacity: 1; transform: none; } }
    @keyframes hb-pulse { 0%,100% { transform: scale(1); filter: none; } 50% { transform: scale(1.06); filter: drop-shadow(0 0 10px #a855f7); } }
    @keyframes hb-blink { 0%,100% { opacity: 0.35; } 50% { opacity: 1; } }
    """

    /// Draws streamed commands one at a time with a short gap, so even a
    /// burst of lines still looks like it's being sketched live.
    private static let boardRendererScript = """
    const SVG_NS = 'http://www.w3.org/2000/svg';
    const board = document.getElementById('board');
    const roughCanvas = rough.svg(board);
    const palette = { accent: '#a855f7', accent2: '#c084fc', fg: '#f4f2f7', muted: '#a59fb3',
      good: '#4ade80', warn: '#fbbf24', bad: '#f87171' };
    const elementsById = {};
    const arrowsById = {};
    const commandQueue = [];
    let isDrawingCommand = false;

    function resolveColor(colorName) { return palette[colorName] || colorName || palette.accent; }

    function drawStrokesIn(groupElement) {
      groupElement.querySelectorAll('path').forEach((pathElement) => {
        const strokeLength = pathElement.getTotalLength();
        pathElement.style.strokeDasharray = strokeLength;
        pathElement.style.strokeDashoffset = strokeLength;
        pathElement.getBoundingClientRect();
        pathElement.style.transition = 'stroke-dashoffset 0.55s ease-out';
        pathElement.style.strokeDashoffset = 0;
      });
    }

    function addText(x, y, textContent, fontSize, colorName, fontWeight) {
      const textElement = document.createElementNS(SVG_NS, 'text');
      textElement.setAttribute('x', x);
      textElement.setAttribute('y', y);
      textElement.setAttribute('text-anchor', 'middle');
      textElement.setAttribute('dominant-baseline', 'middle');
      textElement.setAttribute('font-size', fontSize);
      textElement.setAttribute('font-weight', fontWeight || 600);
      textElement.setAttribute('fill', resolveColor(colorName || 'fg'));
      textElement.textContent = textContent;
      textElement.classList.add('hb-appear');
      board.appendChild(textElement);
      return textElement;
    }

    function addShapeGroup(roughNode, id) {
      const groupElement = document.createElementNS(SVG_NS, 'g');
      groupElement.appendChild(roughNode);
      board.appendChild(groupElement);
      drawStrokesIn(groupElement);
      return groupElement;
    }

    function sketchOptions(colorName) {
      return { stroke: resolveColor(colorName), strokeWidth: 2.2, roughness: 1.5, bowing: 1.3,
        fill: 'rgba(168,85,247,0.18)', fillStyle: 'hachure', hachureGap: 7, fillWeight: 1.1 };
    }

    // A point on the element's outline in the direction of (towardX, towardY),
    // so arrows start and end at edges instead of centers.
    function outlinePointToward(element, towardX, towardY) {
      const deltaX = towardX - element.centerX, deltaY = towardY - element.centerY;
      if (element.radius) {
        const distance = Math.hypot(deltaX, deltaY) || 1;
        return [element.centerX + deltaX / distance * (element.radius + 6), element.centerY + deltaY / distance * (element.radius + 6)];
      }
      const halfWidth = element.width / 2 + 6, halfHeight = element.height / 2 + 6;
      const scale = 1 / Math.max(Math.abs(deltaX) / halfWidth, Math.abs(deltaY) / halfHeight, 1e-6);
      return [element.centerX + deltaX * scale, element.centerY + deltaY * scale];
    }

    function drawCommand(command) {
      const commandType = command.t;
      if (commandType === 'title') {
        addText(280, 22, command.text, 22, command.color || 'fg', 700);
      } else if (commandType === 'text') {
        addText(command.x, command.y, command.text, command.size || 15, command.color || 'muted', command.bold ? 700 : 500);
      } else if (commandType === 'box' || commandType === 'circle') {
        const isCircle = commandType === 'circle';
        const width = command.w || 120, height = command.h || 70, radius = command.r || 42;
        const centerX = isCircle ? command.x : command.x + width / 2;
        const centerY = isCircle ? command.y : command.y + height / 2;
        const roughNode = isCircle
          ? roughCanvas.circle(centerX, centerY, radius * 2, sketchOptions(command.color))
          : roughCanvas.rectangle(command.x, command.y, width, height, sketchOptions(command.color));
        const groupElement = addShapeGroup(roughNode);
        if (command.icon && command.label) {
          addText(centerX, centerY - 13, command.icon, 26);
          addText(centerX, centerY + 19, command.label, 14, 'fg', 600);
        } else if (command.icon) {
          addText(centerX, centerY, command.icon, 30);
        } else if (command.label) {
          addText(centerX, centerY, command.label, 15, 'fg', 600);
        }
        if (command.id) {
          elementsById[command.id] = { centerX, centerY, width: isCircle ? radius * 2 : width,
            height: isCircle ? radius * 2 : height, radius: isCircle ? radius : 0, groupElement };
        }
      } else if (commandType === 'icon') {
        const iconSize = command.size || 40;
        const textElement = addText(command.x, command.y, command.emoji, iconSize);
        if (command.label) { addText(command.x, command.y + iconSize * 0.75, command.label, 13, 'muted', 600); }
        if (command.id) {
          elementsById[command.id] = { centerX: command.x, centerY: command.y, width: iconSize, height: iconSize,
            radius: iconSize * 0.55, groupElement: textElement };
        }
      } else if (commandType === 'arrow') {
        const fromElement = elementsById[command.from], toElement = elementsById[command.to];
        if (!fromElement || !toElement) { return; }
        const [startX, startY] = outlinePointToward(fromElement, toElement.centerX, toElement.centerY);
        const [endX, endY] = outlinePointToward(toElement, fromElement.centerX, fromElement.centerY);
        const arrowAngle = Math.atan2(endY - startY, endX - startX);
        const headLength = 12;
        const strokeOptions = { stroke: resolveColor(command.color || 'accent2'), strokeWidth: 2.2, roughness: 1.2, bowing: 1.5 };
        const groupElement = document.createElementNS(SVG_NS, 'g');
        groupElement.appendChild(roughCanvas.line(startX, startY, endX, endY, strokeOptions));
        groupElement.appendChild(roughCanvas.line(endX, endY,
          endX - headLength * Math.cos(arrowAngle - 0.45), endY - headLength * Math.sin(arrowAngle - 0.45), strokeOptions));
        groupElement.appendChild(roughCanvas.line(endX, endY,
          endX - headLength * Math.cos(arrowAngle + 0.45), endY - headLength * Math.sin(arrowAngle + 0.45), strokeOptions));
        board.appendChild(groupElement);
        drawStrokesIn(groupElement);
        if (command.label) {
          addText((startX + endX) / 2, (startY + endY) / 2 - 13, command.label, 13, 'muted', 600);
        }
        const arrowId = command.id || (command.from + '>' + command.to);
        arrowsById[arrowId] = { startX, startY, endX, endY };
      } else if (commandType === 'flow') {
        // An emoji travelling along an arrow on a loop: shows things moving.
        const arrow = arrowsById[command.along] || arrowsById[command.from + '>' + command.to];
        if (!arrow) { return; }
        const travellerElement = addText(arrow.startX, arrow.startY, command.emoji || '✨', 22);
        const travelMilliseconds = (command.seconds || 1.6) * 1000;
        const loopStartTime = performance.now();
        function moveTraveller(currentTime) {
          const progress = ((currentTime - loopStartTime) % (travelMilliseconds + 500)) / travelMilliseconds;
          const clampedProgress = Math.min(progress, 1);
          travellerElement.setAttribute('x', arrow.startX + (arrow.endX - arrow.startX) * clampedProgress);
          travellerElement.setAttribute('y', arrow.startY + (arrow.endY - arrow.startY) * clampedProgress);
          travellerElement.style.opacity = progress > 1 ? 0 : 1;
          requestAnimationFrame(moveTraveller);
        }
        requestAnimationFrame(moveTraveller);
      } else if (commandType === 'pulse') {
        const element = elementsById[command.id];
        if (element) { element.groupElement.classList.add('hb-pulse'); }
      }
    }

    function drawNextQueuedCommand() {
      if (isDrawingCommand || commandQueue.length === 0) { return; }
      isDrawingCommand = true;
      const nextCommand = commandQueue.shift();
      try { drawCommand(nextCommand); } catch (drawError) { console.log(drawError); }
      setTimeout(() => { isDrawingCommand = false; drawNextQueuedCommand(); }, 260);
    }

    window.hb = {
      add(commandLine) {
        const trimmedLine = commandLine.trim();
        if (!trimmedLine.startsWith('{')) { return; }
        try { commandQueue.push(JSON.parse(trimmedLine)); } catch (parseError) { return; }
        drawNextQueuedCommand();
      },
      done() {
        const waitForQueue = () => commandQueue.length || isDrawingCommand
          ? setTimeout(waitForQueue, 200)
          : document.getElementById('drawing-indicator').classList.add('finished');
        waitForQueue();
      }
    };
    """
}
