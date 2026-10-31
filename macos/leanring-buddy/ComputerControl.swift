//
//  ComputerControl.swift
//  leanring-buddy
//
//  Lets hellobhai click and type for the user. Claude appends action tags
//  to its reply — [CLICK:x,y:label:screenN], [TYPE:text], [KEY:cmd+s] — and
//  these are replayed as synthetic mouse/keyboard events. Posting events
//  needs the Accessibility permission the app already has for the hotkey.
//

import AppKit
import Carbon.HIToolbox
import CoreGraphics

enum ComputerAction {
    /// A click at a point in the screenshot's pixel space (top-left origin).
    case click(screenshotPoint: CGPoint, label: String?, screenNumber: Int?)
    case type(text: String)
    case pressKey(keyCombo: String)
}

extension ComputerAction {
    /// Short past-tense description for conversation history, so later
    /// rounds know what already happened.
    var summary: String {
        switch self {
        case .click(_, let label, _):
            return "clicked \(label ?? "something")"
        case .type(let text):
            return "typed \"\(text)\""
        case .pressKey(let keyCombo):
            return "pressed \(keyCombo)"
        }
    }
}

enum ComputerControl {

    // MARK: - Parsing

    /// Pulls every [CLICK:...], [TYPE:...], and [KEY:...] tag out of Claude's
    /// reply, in order. Returns the reply with those tags removed (so they're
    /// never spoken) plus the parsed actions.
    static func extractActions(from responseText: String) -> (textWithoutActions: String, actions: [ComputerAction]) {
        let actionTagPattern = #"\[(CLICK|TYPE|KEY):([^\]]*)\]"#
        guard let actionTagRegex = try? NSRegularExpression(pattern: actionTagPattern) else {
            return (responseText, [])
        }

        let fullRange = NSRange(responseText.startIndex..., in: responseText)
        var parsedActions: [ComputerAction] = []
        for match in actionTagRegex.matches(in: responseText, range: fullRange) {
            guard let actionNameRange = Range(match.range(at: 1), in: responseText),
                  let actionPayloadRange = Range(match.range(at: 2), in: responseText) else { continue }
            let actionPayload = String(responseText[actionPayloadRange])

            switch responseText[actionNameRange] {
            case "CLICK":
                if let clickAction = parseClickPayload(actionPayload) {
                    parsedActions.append(clickAction)
                }
            case "TYPE":
                parsedActions.append(.type(text: actionPayload))
            case "KEY":
                parsedActions.append(.pressKey(keyCombo: actionPayload.trimmingCharacters(in: .whitespaces)))
            default:
                break
            }
        }

        let textWithoutActions = actionTagRegex
            .stringByReplacingMatches(in: responseText, range: fullRange, withTemplate: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return (textWithoutActions, parsedActions)
    }

    /// Parses "x,y", "x,y:label", or "x,y:label:screenN".
    private static func parseClickPayload(_ clickPayload: String) -> ComputerAction? {
        // The lookahead stops a bare ":screen2" from being read as the label.
        let clickPayloadPattern = #"^\s*(\d+)\s*,\s*(\d+)(?::(?!screen\d+\s*$)([^:]*?))?(?::screen(\d+))?\s*$"#
        guard let clickPayloadRegex = try? NSRegularExpression(pattern: clickPayloadPattern),
              let match = clickPayloadRegex.firstMatch(in: clickPayload, range: NSRange(clickPayload.startIndex..., in: clickPayload)),
              let xRange = Range(match.range(at: 1), in: clickPayload),
              let yRange = Range(match.range(at: 2), in: clickPayload),
              let x = Double(clickPayload[xRange]),
              let y = Double(clickPayload[yRange]) else {
            return nil
        }
        let label = Range(match.range(at: 3), in: clickPayload).map { String(clickPayload[$0]) }
        let screenNumber = Range(match.range(at: 4), in: clickPayload).flatMap { Int(clickPayload[$0]) }
        return .click(screenshotPoint: CGPoint(x: x, y: y), label: label, screenNumber: screenNumber)
    }

    // MARK: - Mouse

    /// Moves the mouse to a global AppKit point (bottom-left origin) and left-clicks.
    static func click(atAppKitGlobalPoint appKitGlobalPoint: CGPoint) {
        // CGEvent uses Quartz coordinates: origin at the top-left of the
        // primary display, y increasing downward.
        let primaryDisplayHeight = NSScreen.screens.first?.frame.height ?? 0
        let quartzPoint = CGPoint(x: appKitGlobalPoint.x, y: primaryDisplayHeight - appKitGlobalPoint.y)

        for mouseEventType in [CGEventType.mouseMoved, .leftMouseDown, .leftMouseUp] {
            CGEvent(
                mouseEventSource: nil,
                mouseType: mouseEventType,
                mouseCursorPosition: quartzPoint,
                mouseButton: .left
            )?.post(tap: .cghidEventTap)
            // Some apps drop a mouse-down that arrives in the same instant as the move.
            usleep(30_000)
        }
    }

    // MARK: - Keyboard

    /// Types text into whatever field has focus. Uses Unicode keyboard events,
    /// so any script works (including Devanagari) regardless of keyboard layout.
    static func type(text: String) {
        // A single event carries at most ~20 UTF-16 units, so send the text
        // in chunks — split on Character boundaries so emoji and combining
        // Devanagari marks are never cut in half.
        var currentChunk = ""
        for character in text {
            if currentChunk.utf16.count + character.utf16.count > 20 {
                postUnicodeKeystroke(currentChunk)
                currentChunk = ""
            }
            currentChunk.append(character)
        }
        if !currentChunk.isEmpty {
            postUnicodeKeystroke(currentChunk)
        }
    }

    private static func postUnicodeKeystroke(_ textChunk: String) {
        var utf16CodeUnits = Array(textChunk.utf16)
        for isKeyDown in [true, false] {
            let keyboardEvent = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: isKeyDown)
            keyboardEvent?.keyboardSetUnicodeString(stringLength: utf16CodeUnits.count, unicodeString: &utf16CodeUnits)
            keyboardEvent?.post(tap: .cghidEventTap)
        }
        usleep(8_000)
    }

    /// Presses a key combo like "return", "cmd+s", or "cmd+shift+t".
    /// Returns false (and does nothing) if any part isn't recognised.
    @discardableResult
    static func press(keyCombo: String) -> Bool {
        var modifierFlags: CGEventFlags = []
        var mainKeyCode: Int?

        for keyName in keyCombo.lowercased().split(separator: "+").map({ $0.trimmingCharacters(in: .whitespaces) }) {
            if let modifierFlag = modifierFlagsByName[keyName] {
                modifierFlags.insert(modifierFlag)
            } else if let keyCode = keyCodesByName[keyName] {
                mainKeyCode = keyCode
            } else {
                print("⚠️ ComputerControl: unknown key \"\(keyName)\" in \"\(keyCombo)\"")
                return false
            }
        }
        guard let mainKeyCode else { return false }

        for isKeyDown in [true, false] {
            let keyboardEvent = CGEvent(keyboardEventSource: nil, virtualKey: CGKeyCode(mainKeyCode), keyDown: isKeyDown)
            keyboardEvent?.flags = modifierFlags
            keyboardEvent?.post(tap: .cghidEventTap)
        }
        return true
    }

    private static let modifierFlagsByName: [String: CGEventFlags] = [
        "cmd": .maskCommand, "command": .maskCommand,
        "shift": .maskShift,
        "option": .maskAlternate, "opt": .maskAlternate, "alt": .maskAlternate,
        "ctrl": .maskControl, "control": .maskControl
    ]

    private static let keyCodesByName: [String: Int] = [
        "return": kVK_Return, "enter": kVK_Return, "tab": kVK_Tab, "space": kVK_Space,
        "delete": kVK_Delete, "backspace": kVK_Delete, "forwarddelete": kVK_ForwardDelete,
        "escape": kVK_Escape, "esc": kVK_Escape,
        "left": kVK_LeftArrow, "right": kVK_RightArrow, "up": kVK_UpArrow, "down": kVK_DownArrow,
        "home": kVK_Home, "end": kVK_End, "pageup": kVK_PageUp, "pagedown": kVK_PageDown,
        "a": kVK_ANSI_A, "b": kVK_ANSI_B, "c": kVK_ANSI_C, "d": kVK_ANSI_D, "e": kVK_ANSI_E,
        "f": kVK_ANSI_F, "g": kVK_ANSI_G, "h": kVK_ANSI_H, "i": kVK_ANSI_I, "j": kVK_ANSI_J,
        "k": kVK_ANSI_K, "l": kVK_ANSI_L, "m": kVK_ANSI_M, "n": kVK_ANSI_N, "o": kVK_ANSI_O,
        "p": kVK_ANSI_P, "q": kVK_ANSI_Q, "r": kVK_ANSI_R, "s": kVK_ANSI_S, "t": kVK_ANSI_T,
        "u": kVK_ANSI_U, "v": kVK_ANSI_V, "w": kVK_ANSI_W, "x": kVK_ANSI_X, "y": kVK_ANSI_Y,
        "z": kVK_ANSI_Z,
        "0": kVK_ANSI_0, "1": kVK_ANSI_1, "2": kVK_ANSI_2, "3": kVK_ANSI_3, "4": kVK_ANSI_4,
        "5": kVK_ANSI_5, "6": kVK_ANSI_6, "7": kVK_ANSI_7, "8": kVK_ANSI_8, "9": kVK_ANSI_9,
        ",": kVK_ANSI_Comma, ".": kVK_ANSI_Period, "/": kVK_ANSI_Slash, ";": kVK_ANSI_Semicolon,
        "[": kVK_ANSI_LeftBracket, "]": kVK_ANSI_RightBracket, "-": kVK_ANSI_Minus, "=": kVK_ANSI_Equal,
        "`": kVK_ANSI_Grave
    ]
}
