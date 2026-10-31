//
//  CompanionScreenCaptureUtility.swift
//  leanring-buddy
//
//  Standalone screenshot capture for the companion voice flow.
//  Decoupled from the legacy ScreenshotManager so the companion mode
//  can capture screenshots independently without session state.
//

import AppKit
import ScreenCaptureKit

struct CompanionScreenCapture {
    let imageData: Data
    let label: String
    let isCursorScreen: Bool
    let displayWidthInPoints: Int
    let displayHeightInPoints: Int
    let displayFrame: CGRect
    let screenshotWidthInPixels: Int
    let screenshotHeightInPixels: Int
}

@MainActor
enum CompanionScreenCaptureUtility {

    /// Captures all connected displays as JPEG data, labeling each with
    /// whether the user's cursor is on that screen. This gives the AI
    /// full context across multiple monitors.
    static func captureAllScreensAsJPEG() async throws -> [CompanionScreenCapture] {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)

        guard !content.displays.isEmpty else {
            throw NSError(domain: "CompanionScreenCapture", code: -1,
                          userInfo: [NSLocalizedDescriptionKey: "No display available for capture"])
        }

        let mouseLocation = NSEvent.mouseLocation

        // Exclude all windows belonging to this app so the AI sees
        // only the user's content, not our overlays or panels.
        let ownBundleIdentifier = Bundle.main.bundleIdentifier
        let ownAppWindows = content.windows.filter { window in
            window.owningApplication?.bundleIdentifier == ownBundleIdentifier
        }

        // Build a lookup from display ID to NSScreen so we can use AppKit-coordinate
        // frames instead of CG-coordinate frames. NSEvent.mouseLocation and NSScreen.frame
        // both use AppKit coordinates (bottom-left origin), while SCDisplay.frame uses
        // Core Graphics coordinates (top-left origin). On multi-display setups, the Y
        // origins differ for secondary displays, which breaks cursor-contains checks
        // and downstream coordinate conversions.
        var nsScreenByDisplayID: [CGDirectDisplayID: NSScreen] = [:]
        for screen in NSScreen.screens {
            if let screenNumber = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID {
                nsScreenByDisplayID[screenNumber] = screen
            }
        }

        // Sort displays so the cursor screen is always first
        let sortedDisplays = content.displays.sorted { displayA, displayB in
            let frameA = nsScreenByDisplayID[displayA.displayID]?.frame ?? displayA.frame
            let frameB = nsScreenByDisplayID[displayB.displayID]?.frame ?? displayB.frame
            let aContainsCursor = frameA.contains(mouseLocation)
            let bContainsCursor = frameB.contains(mouseLocation)
            if aContainsCursor != bContainsCursor { return aContainsCursor }
            return false
        }

        var capturedScreens: [CompanionScreenCapture] = []

        for (displayIndex, display) in sortedDisplays.enumerated() {
            // Use NSScreen.frame (AppKit coordinates, bottom-left origin) so
            // displayFrame is in the same coordinate system as NSEvent.mouseLocation
            // and the overlay window's screenFrame in BlueCursorView.
            let displayFrame = nsScreenByDisplayID[display.displayID]?.frame
                ?? CGRect(x: display.frame.origin.x, y: display.frame.origin.y,
                          width: CGFloat(display.width), height: CGFloat(display.height))
            let isCursorScreen = displayFrame.contains(mouseLocation)

            let filter = SCContentFilter(display: display, excludingWindows: ownAppWindows)

            let configuration = SCStreamConfiguration()
            let maxDimension = 1280
            let aspectRatio = CGFloat(display.width) / CGFloat(display.height)
            if display.width >= display.height {
                configuration.width = maxDimension
                configuration.height = Int(CGFloat(maxDimension) / aspectRatio)
            } else {
                configuration.height = maxDimension
                configuration.width = Int(CGFloat(maxDimension) * aspectRatio)
            }

            let cgImage = try await SCScreenshotManager.captureImage(
                contentFilter: filter,
                configuration: configuration
            )

            guard let jpegData = NSBitmapImageRep(cgImage: cgImage)
                    .representation(using: .jpeg, properties: [.compressionFactor: 0.8]) else {
                continue
            }

            let screenLabel: String
            if sortedDisplays.count == 1 {
                screenLabel = "user's screen (cursor is here)"
            } else if isCursorScreen {
                screenLabel = "screen \(displayIndex + 1) of \(sortedDisplays.count) — cursor is on this screen (primary focus)"
            } else {
                screenLabel = "screen \(displayIndex + 1) of \(sortedDisplays.count) — secondary screen"
            }

            capturedScreens.append(CompanionScreenCapture(
                imageData: jpegData,
                label: screenLabel,
                isCursorScreen: isCursorScreen,
                displayWidthInPoints: Int(displayFrame.width),
                displayHeightInPoints: Int(displayFrame.height),
                displayFrame: displayFrame,
                screenshotWidthInPixels: configuration.width,
                screenshotHeightInPixels: configuration.height
            ))
        }

        guard !capturedScreens.isEmpty else {
            throw NSError(domain: "CompanionScreenCapture", code: -2,
                          userInfo: [NSLocalizedDescriptionKey: "Failed to capture any screen"])
        }

        return capturedScreens
    }

    /// Recaptures one region (global AppKit coords) at the display's native
    /// resolution, so small text the user circled is legible. The full-screen
    /// screenshots above are downscaled to 1280px.
    static func captureRegionAsJPEG(globalAppKitRect: CGRect) async throws -> Data? {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        let regionCenter = CGPoint(x: globalAppKitRect.midX, y: globalAppKitRect.midY)
        guard let screen = NSScreen.screens.first(where: { $0.frame.contains(regionCenter) }),
              let displayID = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID,
              let display = content.displays.first(where: { $0.displayID == displayID }) else {
            return nil
        }

        let ownBundleIdentifier = Bundle.main.bundleIdentifier
        let ownAppWindows = content.windows.filter { $0.owningApplication?.bundleIdentifier == ownBundleIdentifier }

        // sourceRect is in display-local points with a top-left origin.
        let regionOnScreen = globalAppKitRect.intersection(screen.frame)
        guard !regionOnScreen.isEmpty else { return nil }
        let configuration = SCStreamConfiguration()
        configuration.sourceRect = CGRect(
            x: regionOnScreen.minX - screen.frame.minX,
            y: screen.frame.maxY - regionOnScreen.maxY,
            width: regionOnScreen.width,
            height: regionOnScreen.height
        )
        configuration.width = Int(regionOnScreen.width * screen.backingScaleFactor)
        configuration.height = Int(regionOnScreen.height * screen.backingScaleFactor)

        let cgImage = try await SCScreenshotManager.captureImage(
            contentFilter: SCContentFilter(display: display, excludingWindows: ownAppWindows),
            configuration: configuration
        )
        return NSBitmapImageRep(cgImage: cgImage).representation(using: .jpeg, properties: [.compressionFactor: 0.85])
    }
}

extension CompanionScreenCapture {
    /// Inverse of CompanionManager.globalAppKitLocation: maps a rect in global
    /// AppKit coords (bottom-left origin) into this screenshot's pixel space
    /// (top-left origin), clipped to the screenshot.
    func screenshotPixelRect(forGlobalAppKitRect globalAppKitRect: CGRect) -> CGRect {
        let pixelsPerPointHorizontally = CGFloat(screenshotWidthInPixels) / CGFloat(displayWidthInPoints)
        let pixelsPerPointVertically = CGFloat(screenshotHeightInPixels) / CGFloat(displayHeightInPoints)
        let pixelRect = CGRect(
            x: (globalAppKitRect.minX - displayFrame.minX) * pixelsPerPointHorizontally,
            y: (displayFrame.maxY - globalAppKitRect.maxY) * pixelsPerPointVertically,
            width: globalAppKitRect.width * pixelsPerPointHorizontally,
            height: globalAppKitRect.height * pixelsPerPointVertically
        )
        return pixelRect.intersection(CGRect(x: 0, y: 0, width: screenshotWidthInPixels, height: screenshotHeightInPixels))
    }
}

extension CGRect {
    /// Smallest rect containing every point of a freehand stroke the user drew.
    init(boundingBoxOfStroke strokePoints: [CGPoint]) {
        let xValues = strokePoints.map(\.x)
        let yValues = strokePoints.map(\.y)
        let minimumX = xValues.min() ?? 0
        let minimumY = yValues.min() ?? 0
        self.init(x: minimumX, y: minimumY, width: (xValues.max() ?? 0) - minimumX, height: (yValues.max() ?? 0) - minimumY)
    }
}
