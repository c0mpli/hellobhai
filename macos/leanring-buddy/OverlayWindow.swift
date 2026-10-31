//
//  OverlayWindow.swift
//  leanring-buddy
//
//  System-wide transparent overlay windows, one per screen. The blue
//  pointer flies out of the orb (OrbWindow.swift), or from beside the mouse
//  in the cursor voice style, onto any screen when Claude points, clicks or
//  draws; the overlays also show the gold circles,
//  the user's own strokes, and the notch speaking pill.
//

import AppKit
import SwiftUI

// An NSPanel with .nonactivatingPanel so that while the hotkey is held and the
// overlay accepts mouse events for drawing, clicking it never activates the app.
class OverlayWindow: NSPanel {
    /// Receives the user's freehand strokes while drawing is enabled.
    weak var companionManager: CompanionManager?

    init(screen: NSScreen) {
        // Create window covering entire screen
        super.init(
            contentRect: screen.frame,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )

        // Make window transparent and non-interactive
        self.isOpaque = false
        self.backgroundColor = .clear
        self.level = .screenSaver  // Always on top, above submenus and popups
        self.ignoresMouseEvents = true  // Click-through
        self.collectionBehavior = [.canJoinAllSpaces, .stationary, .fullScreenAuxiliary]
        self.isReleasedWhenClosed = false
        self.hasShadow = false

        // Important: Allow the window to appear even when app is not active
        self.hidesOnDeactivate = false

        // Cover the entire screen
        self.setFrame(screen.frame, display: true)

        // Make sure it's on the right screen
        if let screenForWindow = NSScreen.screens.first(where: { $0.frame == screen.frame }) {
            self.setFrameOrigin(screenForWindow.frame.origin)
        }
    }

    // Prevent window from becoming key (no focus stealing)
    override var canBecomeKey: Bool {
        return false
    }

    override var canBecomeMain: Bool {
        return false
    }

    // Mouse events only reach this window while the hotkey is held (see
    // OverlayWindowManager.setAcceptsUserDrawing). Handled here, before
    // NSWindow's first-mouse logic, so the very first drag already draws.
    // Ctrl+click can arrive as a right-click, so both buttons draw.
    override func sendEvent(_ event: NSEvent) {
        guard let companionManager else { return super.sendEvent(event) }
        let globalPoint = convertPoint(toScreen: event.locationInWindow)
        switch event.type {
        case .leftMouseDown, .rightMouseDown:
            companionManager.userDrawnStrokes.append([globalPoint])
        case .leftMouseDragged, .rightMouseDragged:
            if !companionManager.userDrawnStrokes.isEmpty {
                companionManager.userDrawnStrokes[companionManager.userDrawnStrokes.count - 1].append(globalPoint)
            }
        case .leftMouseUp, .rightMouseUp:
            break
        default:
            super.sendEvent(event)
        }
    }
}

/// The pointer that launches out of the orb: a soft arrow with rounded
/// corners whose tip points straight up at 0° rotation (the flight code
/// rotates it to face its direction of travel).
struct BuddyPointerArrowShape: Shape {
    func path(in rect: CGRect) -> Path {
        let tipPoint = CGPoint(x: rect.midX, y: rect.minY)
        let rightWingPoint = CGPoint(x: rect.minX + rect.width * 0.94, y: rect.minY + rect.height * 0.9)
        let tailNotchPoint = CGPoint(x: rect.midX, y: rect.minY + rect.height * 0.7)
        let leftWingPoint = CGPoint(x: rect.minX + rect.width * 0.06, y: rect.minY + rect.height * 0.9)
        let cornerPoints = [tipPoint, rightWingPoint, tailNotchPoint, leftWingPoint]
        // Rounding per corner, relative to a 16pt-wide arrow.
        let sizeScale = rect.width / 16
        let cornerRadii: [CGFloat] = [1.6 * sizeScale, 2.6 * sizeScale, 1.4 * sizeScale, 2.6 * sizeScale]

        var arrowPath = Path()
        // Start halfway along the left edge so every corner gets an arc.
        arrowPath.move(to: CGPoint(x: (leftWingPoint.x + tipPoint.x) / 2, y: (leftWingPoint.y + tipPoint.y) / 2))
        for cornerIndex in cornerPoints.indices {
            arrowPath.addArc(
                tangent1End: cornerPoints[cornerIndex],
                tangent2End: cornerPoints[(cornerIndex + 1) % cornerPoints.count],
                radius: cornerRadii[cornerIndex]
            )
        }
        arrowPath.closeSubpath()
        return arrowPath
    }
}

/// The blue pointer as drawn on screen: gradient fill, thin white outline,
/// soft blue glow.
struct BuddyPointerView: View {
    /// Extra glow while flying (0 at rest).
    let extraGlowRadius: CGFloat

    static let pointerGradient = LinearGradient(
        colors: [Color(hex: "#3380FF"), Color(hex: "#2563EB")],
        startPoint: .top,
        endPoint: .bottom
    )

    var body: some View {
        BuddyPointerArrowShape()
            .fill(Self.pointerGradient)
            .overlay(
                BuddyPointerArrowShape()
                    .stroke(Color.white.opacity(0.95), style: StrokeStyle(lineWidth: 1.3, lineJoin: .round))
            )
            .frame(width: 16, height: 19)
            .shadow(color: Color(hex: "#3380FF").opacity(0.55), radius: 5 + extraGlowRadius, x: 0, y: 0)
            .shadow(color: Color.black.opacity(0.18), radius: 1.5, x: 0, y: 1)
    }
}

// PreferenceKey for tracking bubble size
struct SizePreferenceKey: PreferenceKey {
    static var defaultValue: CGSize = .zero
    static func reduce(value: inout CGSize, nextValue: () -> CGSize) {
        value = nextValue()
    }
}

struct NavigationBubbleSizePreferenceKey: PreferenceKey {
    static var defaultValue: CGSize = .zero
    static func reduce(value: inout CGSize, nextValue: () -> CGSize) {
        value = nextValue()
    }
}

/// What the blue pointer is doing. Its home depends on the voice style: in
/// the orb style it launches out of the orb for a job and flies back in; in
/// the cursor style it follows the mouse and flies back to it.
enum BuddyNavigationMode {
    /// Default. Orb style: no pointer on screen, bhai is just the orb.
    /// Cursor style: the pointer follows the mouse.
    case restingAtHome
    /// Flying toward a detected UI element or the drawing pen.
    case navigatingToTarget
    /// Arrived at the target and pointing at it with a speech bubble.
    case pointingAtTarget
    /// Sitting on the whiteboard or 3D pen's drawing head and moving with it.
    case followingDrawingPen
    /// Flying back into the orb (shrinking as it goes) or to the mouse.
    case returningHome
}

/// Reports whether macOS is currently drawing the real mouse pointer, system-wide.
/// `CGCursorIsVisible` is marked unavailable in the Swift SDK but is still exported
/// by CoreGraphics and correctly reports pointers hidden by other apps (video
/// players, hide-while-typing), so it is looked up at runtime. If the symbol ever
/// disappears, the pointer is treated as always visible and the orb never hides.
enum SystemPointerVisibility {
    private typealias CursorIsVisibleFunction = @convention(c) () -> Int32

    private static let cursorIsVisibleFunction: CursorIsVisibleFunction? = {
        guard let coreGraphicsHandle = dlopen("/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics", RTLD_NOW),
              let symbolAddress = dlsym(coreGraphicsHandle, "CGCursorIsVisible") else {
            return nil
        }
        return unsafeBitCast(symbolAddress, to: CursorIsVisibleFunction.self)
    }()

    static func isSystemPointerVisible() -> Bool {
        guard let cursorIsVisibleFunction else { return true }
        return cursorIsVisibleFunction() != 0
    }
}

// SwiftUI content of one screen's overlay. The blue pointer is hidden at
// rest (the orb is bhai's home); when Claude points, clicks or the pen
// draws, the screen that holds the target launches the pointer from the
// orb's position (off this screen if the orb is on another display, so it
// flies in from that side), plays the job, and flies it back into the orb.
// In the cursor voice style the pointer's home is beside the mouse instead:
// it follows the mouse on the mouse's screen (waveform while listening,
// spinner while thinking) and launches from and returns to there.
// Also draws the gold circles, the user's strokes, the notch pill, and the
// onboarding bubbles next to the orb (or the pointer, in the cursor style).
struct BlueCursorView: View {
    let screenFrame: CGRect
    let isFirstAppearance: Bool
    @ObservedObject var companionManager: CompanionManager
    @ObservedObject var orbWindowManager: OrbWindowManager

    /// Where the pointer is, in this screen's SwiftUI coordinates.
    @State private var cursorPosition: CGPoint = .zero
    @State private var welcomeText: String = ""
    @State private var showWelcome: Bool = true
    @State private var bubbleSize: CGSize = .zero
    @State private var bubbleOpacity: Double = 1.0

    // MARK: - Buddy Navigation State

    @State private var buddyNavigationMode: BuddyNavigationMode = .restingAtHome

    /// The rotation angle of the pointer in degrees. Default is -35° (cursor-like).
    /// Changes to face the direction of travel while flying.
    @State private var pointerRotationDegrees: Double = -35.0

    /// Fades the pointer out as it shrinks back into the orb.
    @State private var pointerOpacity: Double = 0.0

    /// Speech bubble text shown when pointing at a detected element.
    @State private var navigationBubbleText: String = ""
    @State private var navigationBubbleOpacity: Double = 0.0
    @State private var navigationBubbleSize: CGSize = .zero

    /// Timer driving the frame-by-frame bezier arc flight animation.
    /// Invalidated when the flight completes, is replaced, or the view disappears.
    @State private var navigationAnimationTimer: Timer?

    /// Polls the drawing pen's location while the overlay is up.
    @State private var drawingPenTrackingTimer: Timer?
    /// The pen rests between strokes for a moment; the pointer only flies
    /// home once it has been gone for a while.
    @State private var lastTimeDrawingPenWasSeen: Date = .distantPast

    /// Scale factor applied to the pointer during flight. Grows to ~1.3x
    /// at the midpoint of the arc, starts small leaving the orb and shrinks
    /// to nothing going back in.
    @State private var buddyFlightScale: CGFloat = 1.0

    /// Scale factor for the navigation speech bubble's pop-in entrance.
    /// Starts at 0.5 and springs to 1.0 when the first character appears.
    @State private var navigationBubbleScale: CGFloat = 1.0

    /// Changes for every new pointing job, so the delayed "fade the bubble,
    /// fly home" steps of an earlier job can't cut a newer one short.
    @State private var pointingJobIdentifier = UUID()

    /// Cursor style only: whether the mouse is on this screen. Only that
    /// screen shows the pointer following it.
    @State private var isMouseOnThisScreen: Bool = false

    /// Cursor style only: true while macOS is not drawing the real pointer
    /// (a video player hid it, or it is hidden while typing). The following
    /// pointer hides with it so it never floats alone on screen.
    @State private var isPointerHiddenBecauseSystemPointerIsHidden: Bool = false

    private let fullWelcomeMessage = "hey! i'm hellobhai"

    /// Seconds the pen must stay at rest before the pointer flies home.
    private let drawingPenRestSecondsBeforeReturning: TimeInterval = 1.2

    private let navigationPointerPhrases = [
        "right here!",
        "this one!",
        "over here!",
        "click this!",
        "here it is!",
        "found it!"
    ]

    var body: some View {
        ZStack {
            // Nearly transparent background (helps with compositing)
            Color.black.opacity(0.001)

            // Gold hand-drawn ellipses from [CIRCLE] tags, traced live around
            // the on-screen text the reply is talking about.
            ForEach(companionManager.screenCircleAnnotations.filter { $0.globalAppKitFrame.intersects(screenFrame) }) { screenCircleAnnotation in
                let topLeftInSwiftUI = convertScreenPointToSwiftUICoordinates(
                    CGPoint(x: screenCircleAnnotation.globalAppKitFrame.minX, y: screenCircleAnnotation.globalAppKitFrame.maxY)
                )
                HandDrawnScreenCircleView(
                    wobbleSeed: screenCircleAnnotation.wobbleSeed,
                    appearDelaySeconds: screenCircleAnnotation.appearDelaySeconds
                )
                .frame(width: screenCircleAnnotation.globalAppKitFrame.width, height: screenCircleAnnotation.globalAppKitFrame.height)
                .position(
                    x: topLeftInSwiftUI.x + screenCircleAnnotation.globalAppKitFrame.width / 2,
                    y: topLeftInSwiftUI.y + screenCircleAnnotation.globalAppKitFrame.height / 2
                )
                .transition(.opacity)
            }

            // The user's own freehand strokes, drawn while holding the hotkey.
            if !companionManager.userDrawnStrokes.isEmpty {
                Path { strokePath in
                    for userDrawnStroke in companionManager.userDrawnStrokes {
                        strokePath.addLines(userDrawnStroke.map(convertScreenPointToSwiftUICoordinates))
                    }
                }
                .stroke(HandDrawnScreenCircleView.goldStrokeColor, style: HandDrawnScreenCircleView.goldStrokeStyle)
                .transition(.opacity)
            }

            // Dynamic-Island-style pill that grows out of the notch while
            // speaking. Only mounted while speaking so its animation costs
            // nothing at rest; the wrapping ZStack animates it in and out.
            ZStack {
                if companionManager.isSpeakingAloud {
                    NotchSpeakingIndicatorView(screenFrame: screenFrame)
                        .transition(.scale(scale: 0.4, anchor: .top).combined(with: .opacity))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .animation(.spring(response: 0.45, dampingFraction: 0.72), value: companionManager.isSpeakingAloud)
            .allowsHitTesting(false)

            // Welcome speech bubble above the orb, or beside the following
            // pointer in the cursor style (first launch only)
            if isBubbleHomeOnThisScreen && showWelcome && !welcomeText.isEmpty {
                orbBubbleText(welcomeText)
                    .opacity(bubbleOpacity)
                    .position(bubbleCenterNextToHome(bubbleSize: bubbleSize))
                    .animation(isCursorStyle ? .spring(response: 0.2, dampingFraction: 0.6, blendDuration: 0) : nil, value: cursorPosition)
                    .animation(.easeOut(duration: 0.5), value: bubbleOpacity)
                    .onPreferenceChange(SizePreferenceKey.self) { newSize in
                        bubbleSize = newSize
                    }
            }

            // Onboarding prompt above the orb (beside the pointer in the
            // cursor style) — "press control + option and say hi" streamed
            // after the welcome demo
            if isBubbleHomeOnThisScreen && companionManager.showOnboardingPrompt && !companionManager.onboardingPromptText.isEmpty {
                orbBubbleText(companionManager.onboardingPromptText)
                    .opacity(companionManager.onboardingPromptOpacity)
                    .position(bubbleCenterNextToHome(bubbleSize: bubbleSize))
                    .animation(isCursorStyle ? .spring(response: 0.2, dampingFraction: 0.6, blendDuration: 0) : nil, value: cursorPosition)
                    .animation(.easeOut(duration: 0.4), value: companionManager.onboardingPromptOpacity)
                    .onPreferenceChange(SizePreferenceKey.self) { newSize in
                        bubbleSize = newSize
                    }
            }

            // Navigation pointer bubble — shown when the pointer arrives at a detected element.
            // Pops in with a scale-bounce (0.5x → 1.0x spring) and a bright initial
            // glow that settles, creating a "materializing" effect.
            if buddyNavigationMode == .pointingAtTarget && !navigationBubbleText.isEmpty {
                Text(navigationBubbleText)
                    .font(.system(size: 11, weight: .medium))
                    .foregroundColor(.white)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(
                        RoundedRectangle(cornerRadius: 6, style: .continuous)
                            .fill(DS.Colors.overlayCursorColor)
                            .shadow(
                                color: DS.Colors.overlayCursorColor.opacity(0.5 + (1.0 - navigationBubbleScale) * 1.0),
                                radius: 6 + (1.0 - navigationBubbleScale) * 16,
                                x: 0, y: 0
                            )
                    )
                    .fixedSize()
                    .overlay(
                        GeometryReader { geo in
                            Color.clear
                                .preference(key: NavigationBubbleSizePreferenceKey.self, value: geo.size)
                        }
                    )
                    .scaleEffect(navigationBubbleScale)
                    .opacity(navigationBubbleOpacity)
                    .position(x: cursorPosition.x + 10 + (navigationBubbleSize.width / 2), y: cursorPosition.y + 18)
                    .animation(.spring(response: 0.4, dampingFraction: 0.6), value: navigationBubbleScale)
                    .animation(.easeOut(duration: 0.5), value: navigationBubbleOpacity)
                    .onPreferenceChange(NavigationBubbleSizePreferenceKey.self) { newSize in
                        navigationBubbleSize = newSize
                    }
            }

            // The blue pointer. Orb style: only on screen while it has a job.
            // Cursor style: also beside the mouse at rest, swapped for the
            // waveform/spinner below while listening/thinking. While it
            // follows the pen or the mouse a fast spring smooths the jumps;
            // while flying there is NO implicit animation — the frame-by-frame
            // bezier timer moves it directly at 60fps for a smooth arc.
            BuddyPointerView(extraGlowRadius: max(0, buddyFlightScale - 1.0) * 20)
                .rotationEffect(.degrees(pointerRotationDegrees))
                .scaleEffect(buddyFlightScale)
                .opacity(pointerDisplayOpacity)
                .position(cursorPosition)
                .animation(
                    isPointerFollowingSomething
                        ? .spring(response: 0.2, dampingFraction: 0.6, blendDuration: 0)
                        : nil,
                    value: cursorPosition
                )
                .animation(
                    buddyNavigationMode == .pointingAtTarget || isPointerFollowingSomething
                        ? .easeInOut(duration: 0.3)
                        : nil,
                    value: pointerRotationDegrees
                )
                .animation(.easeInOut(duration: 0.3), value: isPointerHiddenBecauseSystemPointerIsHidden)
                .animation(.easeIn(duration: 0.25), value: companionManager.voiceState)

            // Cursor style: blue waveform beside the mouse while listening.
            BlueCursorWaveformView(audioPowerLevel: companionManager.currentAudioPowerLevel)
                .opacity(isFollowingPointerShownOnThisScreen && companionManager.voiceState == .listening ? 1 : 0)
                .position(cursorPosition)
                .animation(.spring(response: 0.2, dampingFraction: 0.6, blendDuration: 0), value: cursorPosition)
                .animation(.easeIn(duration: 0.15), value: companionManager.voiceState)

            // Cursor style: blue spinner beside the mouse while thinking
            // (transcription + Claude + waiting for TTS).
            BlueCursorSpinnerView()
                .opacity(isFollowingPointerShownOnThisScreen && companionManager.voiceState == .processing ? 1 : 0)
                .position(cursorPosition)
                .animation(.spring(response: 0.2, dampingFraction: 0.6, blendDuration: 0), value: cursorPosition)
                .animation(.easeIn(duration: 0.15), value: companionManager.voiceState)
        }
        .frame(width: screenFrame.width, height: screenFrame.height)
        .ignoresSafeArea()
        .onAppear {
            isMouseOnThisScreen = screenFrame.contains(NSEvent.mouseLocation)
            cursorPosition = pointerHomePosition
            startTrackingDrawingPen()

            // Only show the welcome message on first appearance (app start),
            // on the screen the orb (or, in the cursor style, the mouse) is on.
            if isFirstAppearance && isBubbleHomeOnThisScreen {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
                    self.bubbleOpacity = 0.0
                    startWelcomeAnimation()
                }
            }
        }
        .onDisappear {
            drawingPenTrackingTimer?.invalidate()
            navigationAnimationTimer?.invalidate()
        }
        .onChange(of: companionManager.detectedElementScreenLocation) { newLocation in
            // When a UI element location is detected, launch the pointer at
            // that position so it points at the element.
            guard let screenLocation = newLocation,
                  let displayFrame = companionManager.detectedElementDisplayFrame else {
                return
            }

            // Only the screen that holds the target animates it.
            guard screenFrame.contains(CGPoint(x: displayFrame.midX, y: displayFrame.midY))
                  || displayFrame == screenFrame else {
                return
            }

            startNavigatingToElement(screenLocation: screenLocation)
        }
    }

    /// The small blue text bubble used for the onboarding messages.
    private func orbBubbleText(_ bubbleText: String) -> some View {
        Text(bubbleText)
            .font(.system(size: 11, weight: .medium))
            .foregroundColor(.white)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(
                RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(DS.Colors.overlayCursorColor)
                    .shadow(color: DS.Colors.overlayCursorColor.opacity(0.5), radius: 6, x: 0, y: 0)
            )
            .fixedSize()
            .overlay(
                GeometryReader { geo in
                    Color.clear
                        .preference(key: SizePreferenceKey.self, value: geo.size)
                }
            )
    }

    // MARK: - Pointer Home (orb or mouse)

    /// The cursor voice style only applies in voice mode; chat mode keeps
    /// the orb's position as the launch point (the orb is hidden there).
    private var isCursorStyle: Bool {
        companionManager.voiceBuddyStyle == .cursor && !companionManager.isChatInputMode
    }

    /// Following the pen, or (cursor style) resting beside the mouse.
    private var isPointerFollowingSomething: Bool {
        buddyNavigationMode == .followingDrawingPen
            || (isCursorStyle && buddyNavigationMode == .restingAtHome)
    }

    /// Cursor style at rest: the following pointer (or its waveform/spinner)
    /// shows only on the screen with the mouse, and not while another
    /// screen's pointer is out pointing or drawing, so only one is visible.
    private var isFollowingPointerShownOnThisScreen: Bool {
        isCursorStyle
            && buddyNavigationMode == .restingAtHome
            && isMouseOnThisScreen
            && companionManager.detectedElementScreenLocation == nil
            && companionManager.buddyDrawingPenScreenLocation == nil
    }

    private var pointerDisplayOpacity: Double {
        if buddyNavigationMode != .restingAtHome {
            return pointerOpacity
        }
        // At rest the arrow gives way to the waveform and spinner.
        let isVoiceStateShowingArrow = companionManager.voiceState == .idle || companionManager.voiceState == .responding
        let isShowingFollowingArrow = isFollowingPointerShownOnThisScreen
            && isVoiceStateShowingArrow
            && !isPointerHiddenBecauseSystemPointerIsHidden
        return isShowingFollowingArrow ? 1 : 0
    }

    /// Where the pointer launches from and returns to, in this screen's
    /// SwiftUI coordinates (may lie off this screen).
    private var pointerHomePosition: CGPoint {
        isCursorStyle ? mouseFollowingPosition : orbCenterInThisScreen
    }

    /// Cursor style: the pointer sits just below-right of the real mouse tip.
    private var mouseFollowingPosition: CGPoint {
        let mouseInSwiftUI = convertScreenPointToSwiftUICoordinates(NSEvent.mouseLocation)
        return CGPoint(x: mouseInSwiftUI.x + 35, y: mouseInSwiftUI.y + 25)
    }

    /// The onboarding bubbles sit by the orb, or by the pointer in the
    /// cursor style.
    private var isBubbleHomeOnThisScreen: Bool {
        isCursorStyle ? isMouseOnThisScreen : isOrbOnThisScreen
    }

    private func bubbleCenterNextToHome(bubbleSize: CGSize) -> CGPoint {
        if isCursorStyle {
            return CGPoint(x: cursorPosition.x + 10 + bubbleSize.width / 2, y: cursorPosition.y + 18)
        }
        return bubbleCenterNextToOrb(bubbleSize: bubbleSize)
    }

    /// Cursor style at rest: keep the pointer beside the mouse and hide it
    /// while the real pointer is hidden (never mid-answer).
    private func followMouseWhileResting() {
        let mouseLocation = NSEvent.mouseLocation
        let mouseIsOnThisScreen = screenFrame.contains(mouseLocation)
        if mouseIsOnThisScreen != isMouseOnThisScreen {
            isMouseOnThisScreen = mouseIsOnThisScreen
        }
        let shouldHideWithSystemPointer = !SystemPointerVisibility.isSystemPointerVisible()
            && companionManager.voiceState == .idle
        if shouldHideWithSystemPointer != isPointerHiddenBecauseSystemPointerIsHidden {
            isPointerHiddenBecauseSystemPointerIsHidden = shouldHideWithSystemPointer
        }
        // Tracked even off this screen, so a flight launched here starts
        // from the mouse's side.
        cursorPosition = mouseFollowingPosition
    }

    // MARK: - Orb Position

    private var isOrbOnThisScreen: Bool {
        screenFrame.contains(orbWindowManager.orbCenterInScreenCoordinates)
    }

    /// The orb's center in this screen's SwiftUI coordinates. Lies outside
    /// this screen when the orb is on another display.
    private var orbCenterInThisScreen: CGPoint {
        convertScreenPointToSwiftUICoordinates(orbWindowManager.orbCenterInScreenCoordinates)
    }

    /// Puts an onboarding bubble just above the orb (below it if the orb is
    /// near the top), kept fully on screen.
    private func bubbleCenterNextToOrb(bubbleSize: CGSize) -> CGPoint {
        let orbCenter = orbCenterInThisScreen
        let orbHalfHeight = OrbWindowManager.orbWindowSize.height / 2
        let gapBetweenOrbAndBubble: CGFloat = 6
        let screenEdgePadding: CGFloat = 8
        let centerAboveOrbY = orbCenter.y - orbHalfHeight - gapBetweenOrbAndBubble - bubbleSize.height / 2
        let centerBelowOrbY = orbCenter.y + orbHalfHeight + gapBetweenOrbAndBubble + bubbleSize.height / 2
        let bubbleCenterY = centerAboveOrbY - bubbleSize.height / 2 < screenEdgePadding ? centerBelowOrbY : centerAboveOrbY
        let bubbleCenterX = min(
            max(orbCenter.x, screenEdgePadding + bubbleSize.width / 2),
            screenFrame.width - screenEdgePadding - bubbleSize.width / 2
        )
        return CGPoint(x: bubbleCenterX, y: bubbleCenterY)
    }

    /// Converts a macOS screen point (AppKit, bottom-left origin) to SwiftUI
    /// coordinates (top-left origin) relative to this screen's overlay window.
    private func convertScreenPointToSwiftUICoordinates(_ screenPoint: CGPoint) -> CGPoint {
        let x = screenPoint.x - screenFrame.origin.x
        let y = (screenFrame.origin.y + screenFrame.height) - screenPoint.y
        return CGPoint(x: x, y: y)
    }

    /// Puts the pointer at home, visible and ready to fly out: small inside
    /// the orb, or full size beside the mouse in the cursor style.
    private func placePointerAtHomeReadyToLaunch() {
        cursorPosition = pointerHomePosition
        buddyFlightScale = isCursorStyle ? 1.0 : 0.3
        pointerOpacity = 1.0
    }

    // MARK: - Drawing Pen

    /// While the whiteboard or 3D model is drawing, the pointer is the pen:
    /// it flies out of its home to the drawing head, moves with it, and
    /// flies home once the pen has rested for a moment. In the cursor style
    /// the same timer keeps the resting pointer beside the mouse.
    private func startTrackingDrawingPen() {
        drawingPenTrackingTimer?.invalidate()
        drawingPenTrackingTimer = Timer.scheduledTimer(withTimeInterval: 1.0 / 60.0, repeats: true) { _ in
            let drawingPenLocation = self.companionManager.buddyDrawingPenScreenLocation

            switch self.buddyNavigationMode {
            case .restingAtHome, .returningHome:
                guard let drawingPenLocation, self.screenFrame.contains(drawingPenLocation) else {
                    if self.buddyNavigationMode == .restingAtHome && self.isCursorStyle {
                        self.followMouseWhileResting()
                    }
                    return
                }
                if self.buddyNavigationMode == .restingAtHome {
                    self.placePointerAtHomeReadyToLaunch()
                }
                self.lastTimeDrawingPenWasSeen = Date()
                self.buddyNavigationMode = .navigatingToTarget
                self.animateBezierFlightArc(
                    to: self.pointerPositionOnDrawingHead(drawingPenLocation),
                    growsFromOrb: self.buddyFlightScale < 1.0
                ) {
                    guard self.buddyNavigationMode == .navigatingToTarget else { return }
                    self.buddyNavigationMode = .followingDrawingPen
                    self.pointerRotationDegrees = -35.0
                }
            case .followingDrawingPen:
                if let drawingPenLocation {
                    self.lastTimeDrawingPenWasSeen = Date()
                    self.cursorPosition = self.pointerPositionOnDrawingHead(drawingPenLocation)
                } else if Date().timeIntervalSince(self.lastTimeDrawingPenWasSeen) > self.drawingPenRestSecondsBeforeReturning {
                    self.startFlyingBackHome()
                }
            case .navigatingToTarget, .pointingAtTarget:
                break
            }
        }
    }

    /// The pointer's center for its tip to sit on the drawing head.
    private func pointerPositionOnDrawingHead(_ drawingPenLocation: CGPoint) -> CGPoint {
        let penPositionInSwiftUI = convertScreenPointToSwiftUICoordinates(drawingPenLocation)
        return CGPoint(x: penPositionInSwiftUI.x + 6, y: penPositionInSwiftUI.y + 7)
    }

    // MARK: - Element Navigation

    /// Launches the pointer toward a detected UI element location: out of
    /// its home (orb or mouse) if it was resting, or from wherever it is if
    /// already out.
    private func startNavigatingToElement(screenLocation: CGPoint) {
        // Don't interrupt welcome animation
        guard !showWelcome || welcomeText.isEmpty else { return }

        // Convert the AppKit screen location to SwiftUI coordinates for this screen
        let targetInSwiftUI = convertScreenPointToSwiftUICoordinates(screenLocation)

        // Offset the target so the pointer sits beside the element rather than
        // directly on top of it — 8px to the right, 12px below.
        let offsetTarget = CGPoint(
            x: targetInSwiftUI.x + 8,
            y: targetInSwiftUI.y + 12
        )

        // Clamp target to screen bounds with padding
        let clampedTarget = CGPoint(
            x: max(20, min(offsetTarget.x, screenFrame.width - 20)),
            y: max(20, min(offsetTarget.y, screenFrame.height - 20))
        )

        if buddyNavigationMode == .restingAtHome {
            placePointerAtHomeReadyToLaunch()
        }
        pointingJobIdentifier = UUID()
        navigationBubbleText = ""
        navigationBubbleOpacity = 0.0
        buddyNavigationMode = .navigatingToTarget

        // Only an orb launch starts small (placePointerAtHomeReadyToLaunch).
        animateBezierFlightArc(to: clampedTarget, growsFromOrb: buddyFlightScale < 1.0) {
            guard self.buddyNavigationMode == .navigatingToTarget else { return }
            self.startPointingAtElement()
        }
    }

    /// Animates the pointer along a quadratic bezier arc from its current position
    /// to the specified destination. The pointer rotates to face its direction
    /// of travel (tangent to the curve) each frame, scales up at the midpoint
    /// for a "swooping" feel, and the glow intensifies during flight.
    /// `growsFromOrb` starts it small (leaving the orb); `shrinksIntoOrb`
    /// shrinks and fades it to nothing on arrival (going back in).
    private func animateBezierFlightArc(
        to destination: CGPoint,
        growsFromOrb: Bool = false,
        shrinksIntoOrb: Bool = false,
        onComplete: @escaping () -> Void
    ) {
        navigationAnimationTimer?.invalidate()

        let startPosition = cursorPosition
        let endPosition = destination
        let startScale = buddyFlightScale

        let deltaX = endPosition.x - startPosition.x
        let deltaY = endPosition.y - startPosition.y
        let distance = hypot(deltaX, deltaY)

        // Flight duration scales with distance: short hops are quick, long
        // flights are more dramatic. Clamped to 0.6s–1.4s.
        let flightDurationSeconds = min(max(distance / 800.0, 0.6), 1.4)
        let frameInterval: Double = 1.0 / 60.0
        let totalFrames = Int(flightDurationSeconds / frameInterval)
        var currentFrame = 0

        // Control point for the quadratic bezier arc. Offset the midpoint
        // upward (negative Y in SwiftUI) so the pointer flies in a parabolic arc.
        let midPoint = CGPoint(
            x: (startPosition.x + endPosition.x) / 2.0,
            y: (startPosition.y + endPosition.y) / 2.0
        )
        let arcHeight = min(distance * 0.2, 80.0)
        let controlPoint = CGPoint(x: midPoint.x, y: midPoint.y - arcHeight)

        navigationAnimationTimer = Timer.scheduledTimer(withTimeInterval: frameInterval, repeats: true) { _ in
            currentFrame += 1

            if currentFrame > totalFrames {
                self.navigationAnimationTimer?.invalidate()
                self.navigationAnimationTimer = nil
                self.cursorPosition = endPosition
                self.buddyFlightScale = shrinksIntoOrb ? 0 : 1.0
                onComplete()
                return
            }

            // Linear progress 0→1 over the flight duration
            let linearProgress = Double(currentFrame) / Double(totalFrames)

            // Smoothstep easeInOut: 3t² - 2t³ (Hermite interpolation)
            let t = linearProgress * linearProgress * (3.0 - 2.0 * linearProgress)

            // Quadratic bezier: B(t) = (1-t)²·P0 + 2(1-t)t·P1 + t²·P2
            let oneMinusT = 1.0 - t
            let bezierX = oneMinusT * oneMinusT * startPosition.x
                        + 2.0 * oneMinusT * t * controlPoint.x
                        + t * t * endPosition.x
            let bezierY = oneMinusT * oneMinusT * startPosition.y
                        + 2.0 * oneMinusT * t * controlPoint.y
                        + t * t * endPosition.y

            self.cursorPosition = CGPoint(x: bezierX, y: bezierY)

            // Rotation: face the direction of travel by computing the tangent
            // to the bezier curve. B'(t) = 2(1-t)(P1-P0) + 2t(P2-P1)
            let tangentX = 2.0 * oneMinusT * (controlPoint.x - startPosition.x)
                         + 2.0 * t * (endPosition.x - controlPoint.x)
            let tangentY = 2.0 * oneMinusT * (controlPoint.y - startPosition.y)
                         + 2.0 * t * (endPosition.y - controlPoint.y)
            // +90° offset because the pointer's tip points up at 0° rotation,
            // and atan2 returns 0° for rightward movement
            self.pointerRotationDegrees = atan2(tangentY, tangentX) * (180.0 / .pi) + 90.0

            // Scale pulse: sin curve peaks at midpoint of the flight.
            // The pointer grows to ~1.3x at the apex, then back to 1.0x on landing.
            var flightScale = 1.0 + sin(linearProgress * .pi) * 0.3
            if growsFromOrb {
                // Pops out of the orb over the first third of the flight.
                let growProgress = min(1.0, linearProgress * 3.0)
                flightScale *= startScale + (1.0 - startScale) * growProgress
            }
            if shrinksIntoOrb {
                // Shrinks and fades into the orb over the last 40%.
                let remainingFraction = min(1.0, (1.0 - linearProgress) / 0.4)
                flightScale *= remainingFraction
                self.pointerOpacity = remainingFraction
            }
            self.buddyFlightScale = flightScale
        }
    }

    /// Transitions to pointing mode — shows a speech bubble with a bouncy
    /// scale-in entrance and variable-speed character streaming.
    private func startPointingAtElement() {
        buddyNavigationMode = .pointingAtTarget

        // Rotate back to default pointer angle now that we've arrived
        pointerRotationDegrees = -35.0

        // Reset navigation bubble state — start small for the scale-bounce entrance
        navigationBubbleText = ""
        navigationBubbleOpacity = 1.0
        navigationBubbleSize = .zero
        navigationBubbleScale = 0.5

        // Use custom bubble text from the companion manager (e.g. onboarding demo)
        // if available, otherwise fall back to a random pointer phrase
        let pointerPhrase = companionManager.detectedElementBubbleText
            ?? navigationPointerPhrases.randomElement()
            ?? "right here!"

        let thisPointingJobIdentifier = pointingJobIdentifier
        streamNavigationBubbleCharacter(phrase: pointerPhrase, characterIndex: 0) {
            // All characters streamed — hold for 3 seconds, then fly home
            DispatchQueue.main.asyncAfter(deadline: .now() + 3.0) {
                guard self.buddyNavigationMode == .pointingAtTarget,
                      self.pointingJobIdentifier == thisPointingJobIdentifier else { return }
                self.navigationBubbleOpacity = 0.0
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                    guard self.buddyNavigationMode == .pointingAtTarget,
                          self.pointingJobIdentifier == thisPointingJobIdentifier else { return }
                    self.startFlyingBackHome()
                }
            }
        }
    }

    /// Streams the navigation bubble text one character at a time with variable
    /// delays (30–60ms) for a natural "speaking" rhythm.
    private func streamNavigationBubbleCharacter(
        phrase: String,
        characterIndex: Int,
        onComplete: @escaping () -> Void
    ) {
        guard buddyNavigationMode == .pointingAtTarget else { return }
        guard characterIndex < phrase.count else {
            onComplete()
            return
        }

        let charIndex = phrase.index(phrase.startIndex, offsetBy: characterIndex)
        navigationBubbleText.append(phrase[charIndex])

        // On the first character, trigger the scale-bounce entrance
        if characterIndex == 0 {
            navigationBubbleScale = 1.0
        }

        let characterDelay = Double.random(in: 0.03...0.06)
        DispatchQueue.main.asyncAfter(deadline: .now() + characterDelay) {
            self.streamNavigationBubbleCharacter(
                phrase: phrase,
                characterIndex: characterIndex + 1,
                onComplete: onComplete
            )
        }
    }

    /// Flies the pointer back home (wherever home is now, even on another
    /// screen): into the orb, where it hides, or back beside the mouse.
    private func startFlyingBackHome() {
        buddyNavigationMode = .returningHome
        navigationBubbleText = ""
        navigationBubbleOpacity = 0.0
        navigationBubbleScale = 1.0

        animateBezierFlightArc(to: pointerHomePosition, shrinksIntoOrb: !isCursorStyle) {
            guard self.buddyNavigationMode == .returningHome else { return }
            self.finishNavigationAndRestAtHome()
        }
    }

    /// Back home: hidden in the orb until the next job, or (cursor style)
    /// following the mouse again.
    private func finishNavigationAndRestAtHome() {
        navigationAnimationTimer?.invalidate()
        navigationAnimationTimer = nil
        buddyNavigationMode = .restingAtHome
        pointerRotationDegrees = -35.0
        buddyFlightScale = 1.0
        pointerOpacity = 0.0
        navigationBubbleText = ""
        navigationBubbleOpacity = 0.0
        navigationBubbleScale = 1.0
        companionManager.clearDetectedElementLocation()
    }

    // MARK: - Welcome Animation

    private func startWelcomeAnimation() {
        withAnimation(.easeIn(duration: 0.4)) {
            self.bubbleOpacity = 1.0
        }

        var currentIndex = 0
        Timer.scheduledTimer(withTimeInterval: 0.03, repeats: true) { timer in
            guard currentIndex < self.fullWelcomeMessage.count else {
                timer.invalidate()
                // Hold the text for 2 seconds, then fade it out
                DispatchQueue.main.asyncAfter(deadline: .now() + 2.0) {
                    self.bubbleOpacity = 0.0
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 2.5) {
                    self.showWelcome = false
                    self.companionManager.startOnboardingDemo()
                }
                return
            }

            let index = self.fullWelcomeMessage.index(self.fullWelcomeMessage.startIndex, offsetBy: currentIndex)
            self.welcomeText.append(self.fullWelcomeMessage[index])
            currentIndex += 1
        }
    }
}

// MARK: - Notch Speaking Indicator

/// A black pill hugging the MacBook notch while hellobhai speaks, like the
/// iPhone's Dynamic Island: a pulsing blue orb in the left "ear" and
/// animated blue voice bars in the right one. On displays without a notch
/// the same pill hangs from the top center of the screen.
private struct NotchSpeakingIndicatorView: View {
    let screenFrame: CGRect

    /// Width of each side section that sticks out past the notch.
    private static let earWidth: CGFloat = 58
    private static let voiceBarCount = 5
    private static let blueGradient = LinearGradient(
        colors: [Color(hex: "#7AB0FF"), Color(hex: "#2563EB")],
        startPoint: .top,
        endPoint: .bottom
    )

    /// The notch's size on this screen, or a menu-bar-height zero-width
    /// "notch" on displays that don't have one.
    private var notchSize: CGSize {
        let menuBarHeight = NSStatusBar.system.thickness
        guard let screen = NSScreen.screens.first(where: { $0.frame == screenFrame }),
              screen.safeAreaInsets.top > 0,
              let menuBarAreaLeftOfNotch = screen.auxiliaryTopLeftArea,
              let menuBarAreaRightOfNotch = screen.auxiliaryTopRightArea else {
            return CGSize(width: 0, height: menuBarHeight)
        }
        let notchWidth = screenFrame.width - menuBarAreaLeftOfNotch.width - menuBarAreaRightOfNotch.width
        return CGSize(width: notchWidth, height: screen.safeAreaInsets.top)
    }

    var body: some View {
        let notchSize = notchSize
        TimelineView(.animation) { timeline in
            let elapsedSeconds = timeline.date.timeIntervalSinceReferenceDate

            HStack(spacing: 0) {
                pulsingOrb(elapsedSeconds: elapsedSeconds)
                    .frame(width: Self.earWidth)
                // The notch itself is already black hardware, so leave it empty.
                Color.clear
                    .frame(width: notchSize.width)
                voiceBars(elapsedSeconds: elapsedSeconds)
                    .frame(width: Self.earWidth)
            }
            .frame(height: notchSize.height)
            .background(
                UnevenRoundedRectangle(
                    bottomLeadingRadius: notchSize.height * 0.45,
                    bottomTrailingRadius: notchSize.height * 0.45,
                    style: .continuous
                )
                .fill(Color.black)
                .shadow(color: Color(hex: "#3380FF").opacity(0.55), radius: 12, y: 2)
            )
        }
    }

    /// A small blue orb that gently breathes, like Siri's listening orb.
    private func pulsingOrb(elapsedSeconds: TimeInterval) -> some View {
        let breathingScale = 0.85 + 0.15 * sin(elapsedSeconds * 4)
        return Circle()
            .fill(Self.blueGradient)
            .frame(width: 13, height: 13)
            .scaleEffect(breathingScale)
            .shadow(color: Color(hex: "#3380FF"), radius: 5)
    }

    /// Bars that bob at slightly different speeds so they read as speech.
    private func voiceBars(elapsedSeconds: TimeInterval) -> some View {
        HStack(spacing: 3) {
            ForEach(0..<Self.voiceBarCount, id: \.self) { barIndex in
                let barPhase = Double(barIndex) * 0.9
                let barSpeed = 7.0 + Double(barIndex % 3) * 1.7
                let barHeightFraction = 0.35 + 0.65 * abs(sin(elapsedSeconds * barSpeed + barPhase))
                Capsule()
                    .fill(Self.blueGradient)
                    .frame(width: 3, height: 16 * barHeightFraction)
            }
        }
        .frame(height: 16)
    }
}

// MARK: - Blue Cursor Waveform

/// A small blue waveform that replaces the following pointer (cursor
/// style) while the user is holding the push-to-talk shortcut and speaking.
private struct BlueCursorWaveformView: View {
    let audioPowerLevel: CGFloat

    private let barCount = 5
    private let listeningBarProfile: [CGFloat] = [0.4, 0.7, 1.0, 0.7, 0.4]

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 36.0)) { timelineContext in
            HStack(alignment: .center, spacing: 2) {
                ForEach(0..<barCount, id: \.self) { barIndex in
                    RoundedRectangle(cornerRadius: 1.5, style: .continuous)
                        .fill(DS.Colors.overlayCursorColor)
                        .frame(
                            width: 2,
                            height: barHeight(
                                for: barIndex,
                                timelineDate: timelineContext.date
                            )
                        )
                }
            }
            .shadow(color: DS.Colors.overlayCursorColor.opacity(0.6), radius: 6, x: 0, y: 0)
            .animation(.linear(duration: 0.08), value: audioPowerLevel)
        }
    }

    private func barHeight(for barIndex: Int, timelineDate: Date) -> CGFloat {
        let animationPhase = CGFloat(timelineDate.timeIntervalSinceReferenceDate * 3.6) + CGFloat(barIndex) * 0.35
        let normalizedAudioPowerLevel = max(audioPowerLevel - 0.008, 0)
        let easedAudioPowerLevel = pow(min(normalizedAudioPowerLevel * 2.85, 1), 0.76)
        let reactiveHeight = easedAudioPowerLevel * 10 * listeningBarProfile[barIndex]
        let idlePulse = (sin(animationPhase) + 1) / 2 * 1.5
        return 3 + reactiveHeight + idlePulse
    }
}

// MARK: - Blue Cursor Spinner

/// A small blue spinning indicator that replaces the following pointer
/// (cursor style) while the AI is processing a voice input.
private struct BlueCursorSpinnerView: View {
    @State private var isSpinning = false

    var body: some View {
        Circle()
            .trim(from: 0.15, to: 0.85)
            .stroke(
                AngularGradient(
                    colors: [
                        DS.Colors.overlayCursorColor.opacity(0.0),
                        DS.Colors.overlayCursorColor
                    ],
                    center: .center
                ),
                style: StrokeStyle(lineWidth: 2.5, lineCap: .round)
            )
            .frame(width: 14, height: 14)
            .rotationEffect(.degrees(isSpinning ? 360 : 0))
            .shadow(color: DS.Colors.overlayCursorColor.opacity(0.6), radius: 6, x: 0, y: 0)
            .onAppear {
                withAnimation(.linear(duration: 0.8).repeatForever(autoreverses: false)) {
                    isSpinning = true
                }
            }
    }
}

// Manager for overlay windows — creates one per screen so the pointer can
// fly to targets on any monitor — and for the orb, which shows with them.
@MainActor
class OverlayWindowManager {
    private var overlayWindows: [OverlayWindow] = []
    var hasShownOverlayBefore = false
    let orbWindowManager = OrbWindowManager()

    func showOverlay(onScreens screens: [NSScreen], companionManager: CompanionManager) {
        // Hide any existing overlays
        hideOverlay()

        // Track if this is the first time showing overlay (welcome message)
        let isFirstAppearance = !hasShownOverlayBefore
        hasShownOverlayBefore = true

        // Create one overlay window per screen
        for screen in screens {
            let window = OverlayWindow(screen: screen)
            window.companionManager = companionManager

            let contentView = BlueCursorView(
                screenFrame: screen.frame,
                isFirstAppearance: isFirstAppearance,
                companionManager: companionManager,
                orbWindowManager: orbWindowManager
            )

            let hostingView = NSHostingView(rootView: contentView)
            hostingView.frame = screen.frame
            window.contentView = hostingView

            overlayWindows.append(window)
            window.orderFrontRegardless()
        }
        // After the overlays, so the orb sits above them.
        orbWindowManager.show()
    }

    func hideOverlay() {
        orbWindowManager.hide()
        for window in overlayWindows {
            window.orderOut(nil)
            window.contentView = nil
        }
        overlayWindows.removeAll()
    }

    func isShowingOverlay() -> Bool {
        return !overlayWindows.isEmpty
    }

    /// While the push-to-talk hotkey is held the overlay takes mouse events,
    /// so a drag draws on it instead of clicking the app underneath.
    /// Click-through is restored as soon as the hotkey is released.
    func setAcceptsUserDrawing(_ acceptsUserDrawing: Bool) {
        for window in overlayWindows {
            window.ignoresMouseEvents = !acceptsUserDrawing
        }
    }
}

// MARK: - Screen Circles

/// One gold ellipse Claude asked for with a [CIRCLE] tag.
struct ScreenCircleAnnotation: Identifiable {
    let id = UUID()
    /// Where to draw, in global AppKit coords (bottom-left origin).
    let globalAppKitFrame: CGRect
    /// Seconds after the reply starts before this circle starts tracing.
    let appearDelaySeconds: Double
    /// Gives each circle its own slightly different wobble.
    let wobbleSeed = Double.random(in: 0..<1000)
}

/// A hand-drawn ellipse: the radius wobbles a little and the line goes
/// ~1.1 turns, drifting slightly outward, so it overshoots its start the
/// way a pen circle does.
struct HandDrawnEllipseShape: Shape {
    let wobbleSeed: Double

    func path(in rect: CGRect) -> Path {
        let startAngle = -2.4 + sin(wobbleSeed) * 0.3
        let totalTurnRadians = 2.0 * Double.pi * 1.1
        let firstWobblePhase = wobbleSeed * 1.7
        let secondWobblePhase = wobbleSeed * 2.9
        let pointCount = 140
        // Wide items (a whole line of text) get a flatter, pill-like loop the
        // way a pen circles them; small items stay round.
        let widthToHeightRatio = rect.width / max(rect.height, 1)
        let superellipseExponent = 2.0 + min(1.5, max(0, widthToHeightRatio - 1.5) * 0.3)
        func superellipseComponent(_ trigValue: Double) -> Double {
            (trigValue < 0 ? -1 : 1) * pow(abs(trigValue), 2.0 / superellipseExponent)
        }

        var ellipsePath = Path()
        for pointIndex in 0...pointCount {
            let progress = Double(pointIndex) / Double(pointCount)
            let angle = startAngle + progress * totalTurnRadians
            // Start a bit inside the rect and end a bit outside, so the
            // overshoot sits beside the start instead of on top of it.
            let outwardDrift = 0.95 + 0.09 * progress
            let wobble = 1.0 + 0.035 * sin(2.0 * angle + firstWobblePhase) + 0.02 * sin(3.0 * angle + secondWobblePhase)
            let radiusScale = outwardDrift * wobble
            let point = CGPoint(
                x: rect.midX + rect.width / 2 * radiusScale * superellipseComponent(cos(angle)),
                y: rect.midY + rect.height / 2 * radiusScale * superellipseComponent(sin(angle))
            )
            if pointIndex == 0 {
                ellipsePath.move(to: point)
            } else {
                ellipsePath.addLine(to: point)
            }
        }
        return ellipsePath
    }
}

/// Traces a HandDrawnEllipseShape in thin warm gold once its delay passes.
struct HandDrawnScreenCircleView: View {
    let wobbleSeed: Double
    let appearDelaySeconds: Double

    static let goldStrokeColor = Color(red: 0.83, green: 0.655, blue: 0.17).opacity(0.85)
    static let goldStrokeStyle = StrokeStyle(lineWidth: 2.5, lineCap: .round, lineJoin: .round)

    @State private var tracedFraction: CGFloat = 0

    var body: some View {
        HandDrawnEllipseShape(wobbleSeed: wobbleSeed)
            .trim(from: 0, to: tracedFraction)
            .stroke(Self.goldStrokeColor, style: Self.goldStrokeStyle)
            .onAppear {
                withAnimation(.easeInOut(duration: 0.4).delay(appearDelaySeconds)) {
                    tracedFraction = 1
                }
            }
    }
}
