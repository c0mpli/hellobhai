// Per-monitor overlay: the blue pointer that launches out of the orb (orb voice
// style) or follows the mouse with a waveform while listening and a spinner
// while thinking (cursor voice style), flies to [POINT] targets with a label
// bubble and rides the board/3D pen, gold [CIRCLE] ellipses, the user's own
// gold strokes while the hotkey is held, the onboarding bubbles (by the orb,
// or by the pointer in the cursor style) and the speaking pill.
// Port of macos/leanring-buddy/OverlayWindow.swift (same constants).
//
// All positions from Rust are global physical pixels; convert with
// (x - originX) / scaleFactor into this page's CSS px.
// Event names and payloads are the Rust <-> page contract in the plan (section 7).
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const monitorIndex = Number(new URLSearchParams(window.location.search).get('monitor') ?? 0);
const overlayCanvas = document.getElementById('overlay-canvas');
const drawingContext = overlayCanvas.getContext('2d');
let monitorConfig = null; // { originX, originY, width, height, scaleFactor }

// ---- Visual constants (DesignSystem.swift / OverlayWindow.swift) ----
const CURSOR_COLOR = '#3380FF'; // DS.Colors.overlayCursorColor
const CURSOR_COLOR_RGB = '51, 128, 255';
const GOLD_STROKE_COLOR = 'rgba(212, 167, 43, 0.85)'; // HandDrawnScreenCircleView.goldStrokeColor
const GOLD_STROKE_WIDTH = 2.5;
const BUBBLE_FONT = '500 11px "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';
const TRIANGLE_SIZE = 16;
const DEFAULT_TRIANGLE_ROTATION_DEGREES = -35;
const FOLLOW_OFFSET_X = 35;
const FOLLOW_OFFSET_Y = 25;
const PEN_TIP_OFFSET_X = 6;
const PEN_TIP_OFFSET_Y = 7;
const NAVIGATION_POINTER_PHRASES = ['right here!', 'this one!', 'over here!', 'click this!', 'here it is!', 'found it!'];
// Spring for following the pointer: SwiftUI .spring(response: 0.2, dampingFraction: 0.6).
const FOLLOW_SPRING_STIFFNESS = (2 * Math.PI / 0.2) ** 2;
const FOLLOW_SPRING_DAMPING = 2 * 0.6 * Math.sqrt(FOLLOW_SPRING_STIFFNESS);
// The pen rests between strokes for a moment; the pointer only flies home once it has been gone this long.
const DRAWING_PEN_REST_SECONDS_BEFORE_RETURNING = 1.2;
const SPEAKING_PILL_EAR_WIDTH = 58;
const SPEAKING_PILL_HEIGHT = 26; // No notch on Windows: one menu-bar-like height, hanging from the top centre.
const SPEAKING_PILL_VOICE_BAR_COUNT = 5;
const WAVEFORM_BAR_PROFILE = [0.4, 0.7, 1.0, 0.7, 0.4];

// ---- Small helpers ----
const clamp = (value, minimum, maximum) => Math.min(Math.max(value, minimum), maximum);
const smoothstep = (progress) => progress * progress * (3 - 2 * progress);
const easeInOutQuad = (progress) => (progress < 0.5 ? 2 * progress * progress : 1 - (-2 * progress + 2) ** 2 / 2);
const easeOutBack = (progress) => 1 + 2.70158 * (progress - 1) ** 3 + 1.70158 * (progress - 1) ** 2;
const nowInSeconds = () => performance.now() / 1000;

function globalToPageX(globalX) { return (globalX - monitorConfig.originX) / monitorConfig.scaleFactor; }
function globalToPageY(globalY) { return (globalY - monitorConfig.originY) / monitorConfig.scaleFactor; }
function pageToGlobalX(pageX) { return Math.round(pageX * monitorConfig.scaleFactor + monitorConfig.originX); }
function pageToGlobalY(pageY) { return Math.round(pageY * monitorConfig.scaleFactor + monitorConfig.originY); }
function isGlobalPointOnThisMonitor(globalX, globalY) {
  return globalX >= monitorConfig.originX && globalX < monitorConfig.originX + monitorConfig.width
    && globalY >= monitorConfig.originY && globalY < monitorConfig.originY + monitorConfig.height;
}
function pageWidth() { return monitorConfig.width / monitorConfig.scaleFactor; }
function pageHeight() { return monitorConfig.height / monitorConfig.scaleFactor; }

/// A value that moves linearly toward its target over `durationSeconds`
/// (the SwiftUI .animation(...) on opacity and scale values).
function createAnimatedValue(initialValue, durationSeconds) {
  return { value: initialValue, target: initialValue, durationSeconds };
}
function stepAnimatedValue(animatedValue, elapsedSeconds) {
  const maximumStep = animatedValue.durationSeconds > 0 ? elapsedSeconds / animatedValue.durationSeconds : 1;
  const remaining = animatedValue.target - animatedValue.value;
  animatedValue.value += clamp(remaining, -maximumStep, maximumStep);
}

// ---- State fed by Rust events ----
let pointerGlobal = { x: -100000, y: -100000, visible: true };
let voiceState = 'idle';
let rawAudioLevel = 0;
let smoothedAudioLevel = 0;
let isSpeaking = false;
let penGlobal = null; // { x, y } while the board or 3D pen is drawing
let isDrawingEnabled = false;
// buddy-home-changed: the voice style, chat mode and the orb's frame (global physical px).
let voiceBuddyStyle = 'orb';
let isChatMode = false;
let orbFrameGlobal = null; // { x, y, width, height }
// Monitors whose pointer is out (pointer-away-monitors), this one included.
let monitorIndicesWithPointerAway = [];

// The cursor voice style only applies in voice mode; chat mode keeps the
// orb's spot as the launch point (the orb itself is hidden there).
function isCursorStyle() {
  return voiceBuddyStyle === 'cursor' && !isChatMode;
}

// ---- Pointer state (BlueCursorView) ----
// restingAtHome: orb style = no pointer on screen; cursor style = beside the mouse.
// navigatingToTarget: flying to a [POINT]/click target or the drawing pen.
// pointingAtTarget: arrived, showing the label bubble.
// followingDrawingPen: sitting on the board/3D pen's drawing head.
// returningHome: flying back into the orb (shrinking) or to the mouse.
const buddy = {
  mode: 'restingAtHome',
  position: { x: -1000, y: -1000 },
  velocity: { x: 0, y: 0 },
  rotationDegrees: DEFAULT_TRIANGLE_ROTATION_DEGREES,
  targetRotationDegrees: DEFAULT_TRIANGLE_ROTATION_DEGREES,
  flightScale: 1,
  pointerOpacity: 0, // fades the pointer as it shrinks into the orb
  flight: null, // { start, end, control, startSeconds, durationSeconds, startScale, growsFromOrb, shrinksIntoOrb, onComplete }
  pointingJobGeneration: 0, // invalidates bubble timers when a job is cancelled or replaced
  lastSecondsDrawingPenWasSeen: -Infinity,
};
let isMouseOnThisScreen = false;
let isPointerHiddenBecauseSystemPointerIsHidden = false;
const restingPointerOpacity = createAnimatedValue(0, 0.25);
const waveformOpacity = createAnimatedValue(0, 0.15);
const spinnerOpacity = createAnimatedValue(0, 0.15);

const navigationBubble = {
  text: '',
  opacity: createAnimatedValue(0, 0.5),
  scale: createAnimatedValue(1, 0.3),
};

const onboardingBubble = {
  isVisible: false, // false fades the bubble out over 0.4 s, keeping its text while it fades
  fullText: '',
  shownCharacterCount: 0,
  nextCharacterAtSeconds: 0,
  opacity: createAnimatedValue(0, 0.4),
};

const speakingPillProgress = createAnimatedValue(0, 0.35);

// ---- Annotations ----
let screenCircles = []; // { rect: {x, y, width, height} page px, points, cumulativeLengths, appearAtSeconds }
let userStrokes = []; // arrays of { x, y } page px
let currentStrokeGlobalPoints = null; // the stroke being drawn, sent on pointerup
let annotationFadeStartSeconds = null;

// ---- Canvas sizing ----
function resizeCanvasToWindow() {
  const devicePixelRatio = window.devicePixelRatio || 1;
  overlayCanvas.width = Math.round(window.innerWidth * devicePixelRatio);
  overlayCanvas.height = Math.round(window.innerHeight * devicePixelRatio);
}
window.addEventListener('resize', resizeCanvasToWindow);

// ---- Home (orb or mouse) ----
function pointerPagePosition() {
  return { x: globalToPageX(pointerGlobal.x), y: globalToPageY(pointerGlobal.y) };
}

/// Cursor style: just below-right of the real mouse tip.
function mouseFollowingPosition() {
  const pointerPage = pointerPagePosition();
  return { x: pointerPage.x + FOLLOW_OFFSET_X, y: pointerPage.y + FOLLOW_OFFSET_Y };
}

/// The orb's centre in this page's px; off this page when the orb is on another monitor.
function orbCenterInThisPage() {
  if (!orbFrameGlobal) return mouseFollowingPosition();
  return {
    x: globalToPageX(orbFrameGlobal.x + orbFrameGlobal.width / 2),
    y: globalToPageY(orbFrameGlobal.y + orbFrameGlobal.height / 2),
  };
}

function isOrbOnThisScreen() {
  if (!orbFrameGlobal) return false;
  return isGlobalPointOnThisMonitor(orbFrameGlobal.x + orbFrameGlobal.width / 2, orbFrameGlobal.y + orbFrameGlobal.height / 2);
}

/// Where the pointer launches from and returns to.
function pointerHomePosition() {
  return isCursorStyle() ? mouseFollowingPosition() : orbCenterInThisPage();
}

function setBuddyMode(newMode) {
  const wasAway = buddy.mode !== 'restingAtHome';
  buddy.mode = newMode;
  const isAway = newMode !== 'restingAtHome';
  // Shared with the other overlays so the mouse's screen hides its following pointer while this one is out.
  if (wasAway !== isAway) {
    invoke('overlay_pointer_away_changed', { monitorIndex, isAway }).catch(() => {});
  }
}

/// Visible at home, ready to fly out: small inside the orb, or full size beside the mouse.
function placePointerAtHomeReadyToLaunch() {
  buddy.position = pointerHomePosition();
  buddy.velocity = { x: 0, y: 0 };
  buddy.flightScale = isCursorStyle() ? 1 : 0.3;
  buddy.pointerOpacity = 1;
}

// ---- Flights (port of startNavigatingToElement and friends) ----
function startNavigatingToElement(targetPageX, targetPageY, bubbleText) {
  // Beside the element rather than on top of it: 8 px right, 12 px below; 20 px inside the screen.
  const clampedTarget = {
    x: clamp(targetPageX + 8, 20, pageWidth() - 20),
    y: clamp(targetPageY + 12, 20, pageHeight() - 20),
  };
  if (buddy.mode === 'restingAtHome') placePointerAtHomeReadyToLaunch();
  buddy.pointingJobGeneration += 1;
  navigationBubble.text = '';
  navigationBubble.opacity.value = navigationBubble.opacity.target = 0;
  setBuddyMode('navigatingToTarget');
  const pointingJobGeneration = buddy.pointingJobGeneration;
  // Only an orb launch starts small (placePointerAtHomeReadyToLaunch).
  animateBezierFlightArc(clampedTarget, { growsFromOrb: buddy.flightScale < 1 }, () => {
    if (buddy.mode !== 'navigatingToTarget' || buddy.pointingJobGeneration !== pointingJobGeneration) return;
    startPointingAtElement(bubbleText, pointingJobGeneration);
  });
}

/// `growsFromOrb` starts small (leaving the orb); `shrinksIntoOrb` shrinks and fades to nothing on arrival.
function animateBezierFlightArc(destination, { growsFromOrb = false, shrinksIntoOrb = false }, onComplete) {
  const start = { ...buddy.position };
  const distance = Math.hypot(destination.x - start.x, destination.y - start.y);
  // Short hops are quick, long flights more dramatic: 0.6 s to 1.4 s.
  const durationSeconds = clamp(distance / 800, 0.6, 1.4);
  // Midpoint raised so the pointer flies in a parabolic arc.
  const arcHeight = Math.min(distance * 0.2, 80);
  const control = { x: (start.x + destination.x) / 2, y: (start.y + destination.y) / 2 - arcHeight };
  buddy.flight = {
    start, end: destination, control, startSeconds: nowInSeconds(), durationSeconds,
    startScale: buddy.flightScale, growsFromOrb, shrinksIntoOrb, onComplete,
  };
}

function stepFlight(currentSeconds) {
  const flight = buddy.flight;
  if (!flight) return;
  const linearProgress = (currentSeconds - flight.startSeconds) / flight.durationSeconds;
  if (linearProgress >= 1) {
    buddy.position = { ...flight.end };
    buddy.velocity = { x: 0, y: 0 };
    buddy.flightScale = flight.shrinksIntoOrb ? 0 : 1;
    buddy.flight = null;
    flight.onComplete();
    return;
  }
  const clampedProgress = Math.max(linearProgress, 0);
  const easedProgress = smoothstep(clampedProgress);
  const oneMinus = 1 - easedProgress;
  buddy.position = {
    x: oneMinus * oneMinus * flight.start.x + 2 * oneMinus * easedProgress * flight.control.x + easedProgress * easedProgress * flight.end.x,
    y: oneMinus * oneMinus * flight.start.y + 2 * oneMinus * easedProgress * flight.control.y + easedProgress * easedProgress * flight.end.y,
  };
  // Face the direction of travel: tangent of the curve, +90 because the tip points up at 0.
  const tangentX = 2 * oneMinus * (flight.control.x - flight.start.x) + 2 * easedProgress * (flight.end.x - flight.control.x);
  const tangentY = 2 * oneMinus * (flight.control.y - flight.start.y) + 2 * easedProgress * (flight.end.y - flight.control.y);
  buddy.rotationDegrees = buddy.targetRotationDegrees = Math.atan2(tangentY, tangentX) * 180 / Math.PI + 90;
  // Grows to 1.3x at the apex and back to 1x on landing.
  let flightScale = 1 + Math.sin(clampedProgress * Math.PI) * 0.3;
  if (flight.growsFromOrb) {
    // Pops out of the orb over the first third of the flight.
    flightScale *= flight.startScale + (1 - flight.startScale) * Math.min(1, clampedProgress * 3);
  }
  if (flight.shrinksIntoOrb) {
    // Shrinks and fades into the orb over the last 40 %.
    const remainingFraction = Math.min(1, (1 - clampedProgress) / 0.4);
    flightScale *= remainingFraction;
    buddy.pointerOpacity = remainingFraction;
  }
  buddy.flightScale = flightScale;
}

function startPointingAtElement(bubbleText, pointingJobGeneration) {
  setBuddyMode('pointingAtTarget');
  buddy.targetRotationDegrees = DEFAULT_TRIANGLE_ROTATION_DEGREES;
  navigationBubble.text = '';
  navigationBubble.opacity.value = navigationBubble.opacity.target = 1;
  navigationBubble.scale.value = navigationBubble.scale.target = 0.5;
  const pointerPhrase = bubbleText
    || NAVIGATION_POINTER_PHRASES[Math.floor(Math.random() * NAVIGATION_POINTER_PHRASES.length)];
  const phraseCharacters = Array.from(pointerPhrase);
  const isStillThisJob = () => buddy.mode === 'pointingAtTarget' && buddy.pointingJobGeneration === pointingJobGeneration;

  const streamNextCharacter = (characterIndex) => {
    if (!isStillThisJob()) return;
    if (characterIndex >= phraseCharacters.length) {
      // Hold for 3 s, fade 0.5 s, then fly home.
      setTimeout(() => {
        if (!isStillThisJob()) return;
        navigationBubble.opacity.target = 0;
        setTimeout(() => { if (isStillThisJob()) startFlyingBackHome(); }, 500);
      }, 3000);
      return;
    }
    navigationBubble.text += phraseCharacters[characterIndex];
    if (characterIndex === 0) navigationBubble.scale.target = 1;
    // 30-60 ms per character for a natural "speaking" rhythm.
    setTimeout(() => streamNextCharacter(characterIndex + 1), 30 + Math.random() * 30);
  };
  streamNextCharacter(0);
}

/// Back home, wherever home is now (even on another screen): into the orb, or beside the mouse.
function startFlyingBackHome() {
  setBuddyMode('returningHome');
  navigationBubble.text = '';
  navigationBubble.opacity.value = navigationBubble.opacity.target = 0;
  navigationBubble.scale.value = navigationBubble.scale.target = 1;
  animateBezierFlightArc(pointerHomePosition(), { shrinksIntoOrb: !isCursorStyle() }, () => {
    if (buddy.mode === 'returningHome') finishNavigationAndRestAtHome();
  });
}

/// Hidden in the orb until the next job, or (cursor style) following the mouse again.
function finishNavigationAndRestAtHome() {
  buddy.flight = null;
  setBuddyMode('restingAtHome');
  buddy.targetRotationDegrees = buddy.rotationDegrees = DEFAULT_TRIANGLE_ROTATION_DEGREES;
  buddy.flightScale = 1;
  buddy.pointerOpacity = 0;
  buddy.pointingJobGeneration += 1;
  navigationBubble.text = '';
  navigationBubble.opacity.value = navigationBubble.opacity.target = 0;
  navigationBubble.scale.value = navigationBubble.scale.target = 1;
  if (isCursorStyle()) buddy.position = mouseFollowingPosition();
}

/// The style switched live, or chat mode switched to voice mode: move a resting
/// pointer to its new home without flying it across the screen, and re-aim a
/// return flight that is underway.
function handleHomeChanged(wasCursorStyle) {
  if (wasCursorStyle === isCursorStyle()) return;
  if (buddy.mode === 'restingAtHome') {
    isMouseOnThisScreen = isGlobalPointOnThisMonitor(pointerGlobal.x, pointerGlobal.y);
    buddy.position = pointerHomePosition();
    buddy.velocity = { x: 0, y: 0 };
  } else if (buddy.mode === 'returningHome') {
    // Undo a half-done fade into the orb before re-aiming.
    buddy.pointerOpacity = 1;
    startFlyingBackHome();
  }
}

// ---- Per-frame update (port of startTrackingDrawingPen and followMouseWhileResting) ----
function springBuddyToward(targetX, targetY, elapsedSeconds) {
  for (const axis of ['x', 'y']) {
    const target = axis === 'x' ? targetX : targetY;
    const springForce = FOLLOW_SPRING_STIFFNESS * (target - buddy.position[axis]) - FOLLOW_SPRING_DAMPING * buddy.velocity[axis];
    buddy.velocity[axis] += springForce * elapsedSeconds;
    buddy.position[axis] += buddy.velocity[axis] * elapsedSeconds;
  }
}

/// The pointer's centre for its tip to sit on the drawing head.
function pointerPositionOnDrawingHead() {
  return { x: globalToPageX(penGlobal.x) + PEN_TIP_OFFSET_X, y: globalToPageY(penGlobal.y) + PEN_TIP_OFFSET_Y };
}

/// Cursor style at rest: keep the pointer beside the mouse and hide it while
/// the real pointer is hidden (never mid-answer).
function followMouseWhileResting(elapsedSeconds) {
  const mouseIsOnThisScreen = isGlobalPointOnThisMonitor(pointerGlobal.x, pointerGlobal.y);
  const mouseJustEnteredThisScreen = mouseIsOnThisScreen && !isMouseOnThisScreen;
  isMouseOnThisScreen = mouseIsOnThisScreen;
  isPointerHiddenBecauseSystemPointerIsHidden = !pointerGlobal.visible && voiceState === 'idle';
  if (!mouseIsOnThisScreen) return;
  const followingPosition = mouseFollowingPosition();
  if (mouseJustEnteredThisScreen) {
    // Jump into place instead of springing in from a stale spot.
    buddy.position = followingPosition;
    buddy.velocity = { x: 0, y: 0 };
  } else {
    springBuddyToward(followingPosition.x, followingPosition.y, elapsedSeconds);
  }
}

function updateBuddy(elapsedSeconds, currentSeconds) {
  stepFlight(currentSeconds);
  const isPenOnThisScreen = penGlobal !== null && isGlobalPointOnThisMonitor(penGlobal.x, penGlobal.y);

  switch (buddy.mode) {
    case 'restingAtHome':
    case 'returningHome':
      if (!isPenOnThisScreen) {
        if (buddy.mode === 'restingAtHome' && isCursorStyle()) followMouseWhileResting(elapsedSeconds);
        return;
      }
      // The pen started drawing on this screen: the pointer flies out to it.
      if (buddy.mode === 'restingAtHome') placePointerAtHomeReadyToLaunch();
      buddy.lastSecondsDrawingPenWasSeen = currentSeconds;
      setBuddyMode('navigatingToTarget');
      animateBezierFlightArc(pointerPositionOnDrawingHead(), { growsFromOrb: buddy.flightScale < 1 }, () => {
        if (buddy.mode !== 'navigatingToTarget') return;
        setBuddyMode('followingDrawingPen');
        buddy.targetRotationDegrees = DEFAULT_TRIANGLE_ROTATION_DEGREES;
      });
      return;
    case 'followingDrawingPen':
      if (penGlobal) {
        buddy.lastSecondsDrawingPenWasSeen = currentSeconds;
        const drawingHeadPosition = pointerPositionOnDrawingHead();
        springBuddyToward(drawingHeadPosition.x, drawingHeadPosition.y, elapsedSeconds);
      } else if (currentSeconds - buddy.lastSecondsDrawingPenWasSeen > DRAWING_PEN_REST_SECONDS_BEFORE_RETURNING) {
        startFlyingBackHome();
      }
      return;
    default:
      // navigatingToTarget, pointingAtTarget: the flight and the bubble timers drive it.
      return;
  }
}

/// Cursor style at rest: only the mouse's screen shows the following pointer,
/// and not while another screen's pointer is out, so only one is visible.
function isFollowingPointerShownOnThisScreen() {
  return isCursorStyle() && buddy.mode === 'restingAtHome' && isMouseOnThisScreen && monitorIndicesWithPointerAway.length === 0;
}

/// Cursor style: the waveform/spinner show beside the mouse at rest, or wherever
/// this screen's pointer is while it is out (a hotkey press mid-pointing still shows listening).
function isCursorStyleIndicatorShownOnThisScreen() {
  return isFollowingPointerShownOnThisScreen() || (isCursorStyle() && buddy.mode !== 'restingAtHome');
}

/// Cursor style: the arrow gives way to the waveform and spinner.
function isVoiceStateShowingArrow() {
  return voiceState === 'idle' || voiceState === 'responding';
}

function pointerDisplayOpacity() {
  if (buddy.mode !== 'restingAtHome') {
    return isCursorStyle() && !isVoiceStateShowingArrow() ? 0 : buddy.pointerOpacity;
  }
  return restingPointerOpacity.value;
}

/// The onboarding bubbles sit by the orb, or by the pointer in the cursor style
/// (waiting while it is out, so they never ride the "right here!" flight).
function isBubbleHomeOnThisScreen() {
  return isCursorStyle() ? isMouseOnThisScreen && buddy.mode === 'restingAtHome' : isOrbOnThisScreen();
}

function updateAnimatedValues(elapsedSeconds, currentSeconds) {
  const isShowingFollowingArrow = isFollowingPointerShownOnThisScreen()
    && isVoiceStateShowingArrow()
    && !isPointerHiddenBecauseSystemPointerIsHidden;
  restingPointerOpacity.target = isShowingFollowingArrow ? 1 : 0;
  // Leaving the following state (a launch, a style switch) is instant; voice state changes cross-fade.
  if (buddy.mode !== 'restingAtHome' || !isCursorStyle()) restingPointerOpacity.value = restingPointerOpacity.target;
  const isIndicatorShown = isCursorStyleIndicatorShownOnThisScreen();
  waveformOpacity.target = isIndicatorShown && voiceState === 'listening' ? 1 : 0;
  spinnerOpacity.target = isIndicatorShown && voiceState === 'processing' ? 1 : 0;
  // The pill hangs from the top of every screen while bhai speaks, like the Mac's notch pill.
  speakingPillProgress.target = isSpeaking ? 1 : 0;
  onboardingBubble.opacity.target = onboardingBubble.isVisible && onboardingBubble.fullText && isBubbleHomeOnThisScreen() ? 1 : 0;

  for (const animatedValue of [restingPointerOpacity, waveformOpacity, spinnerOpacity,
    speakingPillProgress, onboardingBubble.opacity, navigationBubble.opacity, navigationBubble.scale]) {
    stepAnimatedValue(animatedValue, elapsedSeconds);
  }

  // Rotation eases back to the pointer angle over 0.3 s, except mid-flight where the tangent drives it.
  if (!buddy.flight) {
    buddy.rotationDegrees += (buddy.targetRotationDegrees - buddy.rotationDegrees) * clamp(elapsedSeconds * 10, 0, 1);
  }

  // The audio level eases over 0.08 s like the Mac waveform.
  smoothedAudioLevel += (rawAudioLevel - smoothedAudioLevel) * clamp(elapsedSeconds / 0.08, 0, 1);

  // The onboarding text types itself out at 30 ms per character.
  const onboardingCharacters = Array.from(onboardingBubble.fullText);
  while (onboardingBubble.shownCharacterCount < onboardingCharacters.length && currentSeconds >= onboardingBubble.nextCharacterAtSeconds) {
    onboardingBubble.shownCharacterCount += 1;
    onboardingBubble.nextCharacterAtSeconds = currentSeconds + 0.03;
  }

  if (annotationFadeStartSeconds !== null && currentSeconds - annotationFadeStartSeconds >= 0.6) {
    screenCircles = [];
    userStrokes = [];
    annotationFadeStartSeconds = null;
  }
}

// ---- Drawing ----
/// Extra glow while flying (0 at rest), like the Mac's extraGlowRadius.
function extraPointerGlowRadius() {
  return Math.max(0, buddy.flightScale - 1) * 20;
}

/// Cursor style: the original pointer, a 16 px equilateral triangle in the
/// overlay blue with a blue glow (ClassicTrianglePointerView).
function drawClassicTrianglePointer() {
  const triangleHeight = TRIANGLE_SIZE * Math.sqrt(3) / 2;
  drawingContext.shadowColor = CURSOR_COLOR;
  drawingContext.shadowBlur = (8 + extraPointerGlowRadius()) * (window.devicePixelRatio || 1);
  drawingContext.fillStyle = CURSOR_COLOR;
  drawingContext.beginPath();
  drawingContext.moveTo(0, -triangleHeight / 1.5);
  drawingContext.lineTo(-TRIANGLE_SIZE / 2, triangleHeight / 3);
  drawingContext.lineTo(TRIANGLE_SIZE / 2, triangleHeight / 3);
  drawingContext.closePath();
  drawingContext.fill();
}

/// Orb style: the soft rounded arrow that launches out of the orb, 16 x 19,
/// tip up at 0 degrees (BuddyPointerArrowShape), drawn around its centre.
function traceBuddyPointerArrowPath() {
  const arrowWidth = 16;
  const arrowHeight = 19;
  const left = -arrowWidth / 2;
  const top = -arrowHeight / 2;
  const tipPoint = { x: 0, y: top };
  const rightWingPoint = { x: left + arrowWidth * 0.94, y: top + arrowHeight * 0.9 };
  const tailNotchPoint = { x: 0, y: top + arrowHeight * 0.7 };
  const leftWingPoint = { x: left + arrowWidth * 0.06, y: top + arrowHeight * 0.9 };
  const cornerPoints = [tipPoint, rightWingPoint, tailNotchPoint, leftWingPoint];
  const cornerRadii = [1.6, 2.6, 1.4, 2.6];
  drawingContext.beginPath();
  // Start halfway along the left edge so every corner gets an arc.
  drawingContext.moveTo((leftWingPoint.x + tipPoint.x) / 2, (leftWingPoint.y + tipPoint.y) / 2);
  cornerPoints.forEach((cornerPoint, cornerIndex) => {
    const nextCornerPoint = cornerPoints[(cornerIndex + 1) % cornerPoints.length];
    drawingContext.arcTo(cornerPoint.x, cornerPoint.y, nextCornerPoint.x, nextCornerPoint.y, cornerRadii[cornerIndex]);
  });
  drawingContext.closePath();
}

/// Gradient fill, thin white outline, soft blue glow plus a faint drop shadow (BuddyPointerView).
function drawBuddyPointerArrow() {
  const devicePixelRatio = window.devicePixelRatio || 1;
  const pointerGradient = drawingContext.createLinearGradient(0, -9.5, 0, 9.5);
  pointerGradient.addColorStop(0, '#3380FF');
  pointerGradient.addColorStop(1, '#2563EB');
  traceBuddyPointerArrowPath();
  drawingContext.fillStyle = pointerGradient;
  drawingContext.shadowColor = 'rgba(0, 0, 0, 0.18)';
  drawingContext.shadowBlur = 1.5 * devicePixelRatio;
  drawingContext.shadowOffsetY = 1 * devicePixelRatio;
  drawingContext.fill();
  drawingContext.shadowOffsetY = 0;
  drawingContext.shadowColor = `rgba(${CURSOR_COLOR_RGB}, 0.55)`;
  drawingContext.shadowBlur = (5 + extraPointerGlowRadius()) * devicePixelRatio;
  drawingContext.fill();
  drawingContext.shadowColor = 'transparent';
  drawingContext.strokeStyle = 'rgba(255, 255, 255, 0.95)';
  drawingContext.lineWidth = 1.3;
  drawingContext.lineJoin = 'round';
  drawingContext.stroke();
}

function drawPointer(centerX, centerY, opacity) {
  if (opacity <= 0.001 || buddy.flightScale <= 0.001) return;
  drawingContext.save();
  drawingContext.globalAlpha = opacity;
  drawingContext.translate(centerX, centerY);
  drawingContext.rotate(buddy.rotationDegrees * Math.PI / 180);
  drawingContext.scale(buddy.flightScale, buddy.flightScale);
  if (isCursorStyle()) {
    drawClassicTrianglePointer();
  } else {
    drawBuddyPointerArrow();
  }
  drawingContext.restore();
}

function drawWaveform(centerX, centerY, opacity, currentSeconds) {
  if (opacity <= 0.001) return;
  const barWidth = 2;
  const barSpacing = 2;
  const totalWidth = WAVEFORM_BAR_PROFILE.length * barWidth + (WAVEFORM_BAR_PROFILE.length - 1) * barSpacing;
  const normalizedAudioLevel = Math.max(smoothedAudioLevel - 0.008, 0);
  const easedAudioLevel = Math.min(normalizedAudioLevel * 2.85, 1) ** 0.76;
  drawingContext.save();
  drawingContext.globalAlpha = opacity;
  drawingContext.fillStyle = CURSOR_COLOR;
  drawingContext.shadowColor = `rgba(${CURSOR_COLOR_RGB}, 0.6)`;
  drawingContext.shadowBlur = 6 * (window.devicePixelRatio || 1);
  WAVEFORM_BAR_PROFILE.forEach((barProfile, barIndex) => {
    const animationPhase = currentSeconds * 3.6 + barIndex * 0.35;
    const reactiveHeight = easedAudioLevel * 10 * barProfile;
    const idlePulse = (Math.sin(animationPhase) + 1) / 2 * 1.5;
    const barHeight = 3 + reactiveHeight + idlePulse;
    const barX = centerX - totalWidth / 2 + barIndex * (barWidth + barSpacing);
    drawingContext.beginPath();
    drawingContext.roundRect(barX, centerY - barHeight / 2, barWidth, barHeight, 1.5);
    drawingContext.fill();
  });
  drawingContext.restore();
}

function drawSpinner(centerX, centerY, opacity, currentSeconds) {
  if (opacity <= 0.001) return;
  drawingContext.save();
  drawingContext.globalAlpha = opacity;
  drawingContext.translate(centerX, centerY);
  drawingContext.rotate(((currentSeconds % 0.8) / 0.8) * Math.PI * 2);
  const angularGradient = drawingContext.createConicGradient(0, 0, 0);
  angularGradient.addColorStop(0, `rgba(${CURSOR_COLOR_RGB}, 0)`);
  angularGradient.addColorStop(1, CURSOR_COLOR);
  drawingContext.strokeStyle = angularGradient;
  drawingContext.lineWidth = 2.5;
  drawingContext.lineCap = 'round';
  drawingContext.shadowColor = `rgba(${CURSOR_COLOR_RGB}, 0.6)`;
  drawingContext.shadowBlur = 6 * (window.devicePixelRatio || 1);
  drawingContext.beginPath();
  drawingContext.arc(0, 0, 7, 0.15 * Math.PI * 2, 0.85 * Math.PI * 2);
  drawingContext.stroke();
  drawingContext.restore();
}

const BUBBLE_HEIGHT = 22;

function bubbleWidthFor(bubbleText) {
  drawingContext.save();
  drawingContext.font = BUBBLE_FONT;
  const bubbleWidth = drawingContext.measureText(bubbleText).width + 16;
  drawingContext.restore();
  return bubbleWidth;
}

/// Left edge 10 px right of the pointer, vertically centred 18 px below it.
function bubbleCenterNextToPointer(bubbleWidth) {
  return { x: buddy.position.x + 10 + bubbleWidth / 2, y: buddy.position.y + 18 };
}

/// Just above the orb (below it when the orb is near the top), kept fully on screen.
function bubbleCenterNextToOrb(bubbleWidth) {
  const orbCenter = orbCenterInThisPage();
  const orbHalfHeight = orbFrameGlobal.height / monitorConfig.scaleFactor / 2;
  const gapBetweenOrbAndBubble = 6;
  const screenEdgePadding = 8;
  const centerAboveOrbY = orbCenter.y - orbHalfHeight - gapBetweenOrbAndBubble - BUBBLE_HEIGHT / 2;
  const centerBelowOrbY = orbCenter.y + orbHalfHeight + gapBetweenOrbAndBubble + BUBBLE_HEIGHT / 2;
  const bubbleCenterY = centerAboveOrbY - BUBBLE_HEIGHT / 2 < screenEdgePadding ? centerBelowOrbY : centerAboveOrbY;
  const bubbleCenterX = clamp(orbCenter.x, screenEdgePadding + bubbleWidth / 2, pageWidth() - screenEdgePadding - bubbleWidth / 2);
  return { x: bubbleCenterX, y: bubbleCenterY };
}

/// The blue rounded label (navigation and onboarding bubbles), centred at `bubbleCenter`.
function drawBubble(bubbleText, bubbleCenter, opacity, scale, glowBoost) {
  if (!bubbleText || opacity <= 0.001) return;
  drawingContext.save();
  drawingContext.font = BUBBLE_FONT;
  const bubbleWidth = bubbleWidthFor(bubbleText);
  const bubbleHeight = BUBBLE_HEIGHT;
  const bubbleCenterX = bubbleCenter.x;
  const bubbleCenterY = bubbleCenter.y;
  drawingContext.globalAlpha = opacity;
  drawingContext.translate(bubbleCenterX, bubbleCenterY);
  drawingContext.scale(scale, scale);
  drawingContext.shadowColor = `rgba(${CURSOR_COLOR_RGB}, ${Math.min(0.5 + glowBoost, 1)})`;
  drawingContext.shadowBlur = (6 + glowBoost * 16) * (window.devicePixelRatio || 1);
  drawingContext.fillStyle = CURSOR_COLOR;
  drawingContext.beginPath();
  drawingContext.roundRect(-bubbleWidth / 2, -bubbleHeight / 2, bubbleWidth, bubbleHeight, 6);
  drawingContext.fill();
  drawingContext.shadowColor = 'transparent';
  drawingContext.fillStyle = '#FFFFFF';
  drawingContext.textBaseline = 'middle';
  drawingContext.fillText(bubbleText, -bubbleWidth / 2 + 8, 0.5);
  drawingContext.restore();
}

/// Black pill hanging from the top centre while bhai speaks: a breathing blue
/// orb on the left, bobbing voice bars on the right (NotchSpeakingIndicatorView).
function drawSpeakingPill(currentSeconds) {
  const progress = speakingPillProgress.value;
  if (progress <= 0.001) return;
  const pillWidth = SPEAKING_PILL_EAR_WIDTH * 2;
  const pillHeight = SPEAKING_PILL_HEIGHT;
  const bottomCornerRadius = pillHeight * 0.45;
  const pillScale = 0.4 + 0.6 * easeOutBack(progress);
  const blueGradientBetween = (topY, bottomY) => {
    const blueGradient = drawingContext.createLinearGradient(0, topY, 0, bottomY);
    blueGradient.addColorStop(0, '#7AB0FF');
    blueGradient.addColorStop(1, '#2563EB');
    return blueGradient;
  };

  drawingContext.save();
  drawingContext.globalAlpha = progress;
  drawingContext.translate(pageWidth() / 2, 0);
  drawingContext.scale(pillScale, pillScale); // grows from the top edge
  drawingContext.shadowColor = 'rgba(51, 128, 255, 0.55)';
  drawingContext.shadowBlur = 12 * (window.devicePixelRatio || 1);
  drawingContext.shadowOffsetY = 2;
  drawingContext.fillStyle = '#000000';
  drawingContext.beginPath();
  drawingContext.roundRect(-pillWidth / 2, 0, pillWidth, pillHeight, [0, 0, bottomCornerRadius, bottomCornerRadius]);
  drawingContext.fill();
  drawingContext.shadowOffsetY = 0;

  // Orb, centred in the left ear.
  const orbRadius = 6.5 * (0.85 + 0.15 * Math.sin(currentSeconds * 4));
  const orbCenterX = -pillWidth / 2 + SPEAKING_PILL_EAR_WIDTH / 2;
  const earCenterY = pillHeight / 2;
  drawingContext.shadowColor = CURSOR_COLOR;
  drawingContext.shadowBlur = 5 * (window.devicePixelRatio || 1);
  drawingContext.fillStyle = blueGradientBetween(earCenterY - orbRadius, earCenterY + orbRadius);
  drawingContext.beginPath();
  drawingContext.arc(orbCenterX, earCenterY, orbRadius, 0, Math.PI * 2);
  drawingContext.fill();
  drawingContext.shadowColor = 'transparent';

  // Voice bars, centred in the right ear.
  const barWidth = 3;
  const barSpacing = 3;
  const maximumBarHeight = 16;
  const barsWidth = SPEAKING_PILL_VOICE_BAR_COUNT * barWidth + (SPEAKING_PILL_VOICE_BAR_COUNT - 1) * barSpacing;
  const barsLeftX = pillWidth / 2 - SPEAKING_PILL_EAR_WIDTH / 2 - barsWidth / 2;
  drawingContext.fillStyle = blueGradientBetween(earCenterY - maximumBarHeight / 2, earCenterY + maximumBarHeight / 2);
  for (let barIndex = 0; barIndex < SPEAKING_PILL_VOICE_BAR_COUNT; barIndex += 1) {
    const barPhase = barIndex * 0.9;
    const barSpeed = 7 + (barIndex % 3) * 1.7;
    const barHeight = maximumBarHeight * (0.35 + 0.65 * Math.abs(Math.sin(currentSeconds * barSpeed + barPhase)));
    drawingContext.beginPath();
    drawingContext.roundRect(barsLeftX + barIndex * (barWidth + barSpacing), earCenterY - barHeight / 2, barWidth, barHeight, barWidth / 2);
    drawingContext.fill();
  }
  drawingContext.restore();
}

/// HandDrawnEllipseShape: the radius wobbles a little and the line goes ~1.1
/// turns, drifting slightly outward, so it overshoots its start like a pen circle.
function handDrawnEllipsePoints(rect, wobbleSeed) {
  const startAngle = -2.4 + Math.sin(wobbleSeed) * 0.3;
  const totalTurnRadians = 2 * Math.PI * 1.1;
  const firstWobblePhase = wobbleSeed * 1.7;
  const secondWobblePhase = wobbleSeed * 2.9;
  const pointCount = 140;
  // Wide items (a whole line of text) get a flatter, pill-like loop.
  const widthToHeightRatio = rect.width / Math.max(rect.height, 1);
  const superellipseExponent = 2 + Math.min(1.5, Math.max(0, widthToHeightRatio - 1.5) * 0.3);
  const superellipseComponent = (trigValue) => Math.sign(trigValue) * Math.abs(trigValue) ** (2 / superellipseExponent);
  const centerX = rect.x + rect.width / 2;
  const centerY = rect.y + rect.height / 2;
  const ellipsePoints = [];
  for (let pointIndex = 0; pointIndex <= pointCount; pointIndex += 1) {
    const progress = pointIndex / pointCount;
    const angle = startAngle + progress * totalTurnRadians;
    const outwardDrift = 0.95 + 0.09 * progress;
    const wobble = 1 + 0.035 * Math.sin(2 * angle + firstWobblePhase) + 0.02 * Math.sin(3 * angle + secondWobblePhase);
    const radiusScale = outwardDrift * wobble;
    ellipsePoints.push({
      x: centerX + rect.width / 2 * radiusScale * superellipseComponent(Math.cos(angle)),
      y: centerY + rect.height / 2 * radiusScale * superellipseComponent(Math.sin(angle)),
    });
  }
  return ellipsePoints;
}

function cumulativeLengthsOf(points) {
  const cumulativeLengths = [0];
  for (let pointIndex = 1; pointIndex < points.length; pointIndex += 1) {
    const previousPoint = points[pointIndex - 1];
    cumulativeLengths.push(cumulativeLengths[pointIndex - 1] + Math.hypot(points[pointIndex].x - previousPoint.x, points[pointIndex].y - previousPoint.y));
  }
  return cumulativeLengths;
}

function setGoldStrokeStyle() {
  drawingContext.strokeStyle = GOLD_STROKE_COLOR;
  drawingContext.lineWidth = GOLD_STROKE_WIDTH;
  drawingContext.lineCap = 'round';
  drawingContext.lineJoin = 'round';
}

function drawAnnotations(currentSeconds) {
  const fadeOpacity = annotationFadeStartSeconds === null ? 1 : clamp(1 - (currentSeconds - annotationFadeStartSeconds) / 0.6, 0, 1);
  drawingContext.save();
  drawingContext.globalAlpha = fadeOpacity;
  setGoldStrokeStyle();

  for (const screenCircle of screenCircles) {
    // Traced over 0.4 s ease-in-out once its delay passes (the SwiftUI trim animation).
    const tracedFraction = easeInOutQuad(clamp((currentSeconds - screenCircle.appearAtSeconds) / 0.4, 0, 1));
    if (tracedFraction <= 0) continue;
    const totalLength = screenCircle.cumulativeLengths[screenCircle.cumulativeLengths.length - 1];
    const tracedLength = totalLength * tracedFraction;
    drawingContext.beginPath();
    drawingContext.moveTo(screenCircle.points[0].x, screenCircle.points[0].y);
    for (let pointIndex = 1; pointIndex < screenCircle.points.length; pointIndex += 1) {
      const segmentStartLength = screenCircle.cumulativeLengths[pointIndex - 1];
      const segmentEndLength = screenCircle.cumulativeLengths[pointIndex];
      if (segmentEndLength <= tracedLength) {
        drawingContext.lineTo(screenCircle.points[pointIndex].x, screenCircle.points[pointIndex].y);
        continue;
      }
      const segmentFraction = (tracedLength - segmentStartLength) / Math.max(segmentEndLength - segmentStartLength, 0.0001);
      const previousPoint = screenCircle.points[pointIndex - 1];
      const nextPoint = screenCircle.points[pointIndex];
      drawingContext.lineTo(previousPoint.x + (nextPoint.x - previousPoint.x) * segmentFraction, previousPoint.y + (nextPoint.y - previousPoint.y) * segmentFraction);
      break;
    }
    drawingContext.stroke();
  }

  for (const userStroke of userStrokes) {
    if (userStroke.length === 0) continue;
    drawingContext.beginPath();
    drawingContext.moveTo(userStroke[0].x, userStroke[0].y);
    for (const strokePoint of userStroke.slice(1)) drawingContext.lineTo(strokePoint.x, strokePoint.y);
    // A single click still leaves a visible dot.
    if (userStroke.length === 1) drawingContext.lineTo(userStroke[0].x + 0.01, userStroke[0].y);
    drawingContext.stroke();
  }
  drawingContext.restore();
}

function drawFrame(currentSeconds) {
  const devicePixelRatio = window.devicePixelRatio || 1;
  drawingContext.setTransform(1, 0, 0, 1, 0, 0);
  drawingContext.clearRect(0, 0, overlayCanvas.width, overlayCanvas.height);
  drawingContext.setTransform(devicePixelRatio, 0, 0, devicePixelRatio, 0, 0);

  drawAnnotations(currentSeconds);
  drawSpeakingPill(currentSeconds);

  const onboardingText = Array.from(onboardingBubble.fullText).slice(0, onboardingBubble.shownCharacterCount).join('');
  if (onboardingText) {
    const onboardingBubbleWidth = bubbleWidthFor(onboardingText);
    const onboardingBubbleCenter = isCursorStyle() || !orbFrameGlobal
      ? bubbleCenterNextToPointer(onboardingBubbleWidth)
      : bubbleCenterNextToOrb(onboardingBubbleWidth);
    drawBubble(onboardingText, onboardingBubbleCenter, onboardingBubble.opacity.value, 1, 0);
  }
  if (buddy.mode === 'pointingAtTarget' && navigationBubble.text) {
    const navigationBubbleCenter = bubbleCenterNextToPointer(bubbleWidthFor(navigationBubble.text));
    drawBubble(navigationBubble.text, navigationBubbleCenter, navigationBubble.opacity.value, navigationBubble.scale.value, 1 - navigationBubble.scale.value);
  }

  drawPointer(buddy.position.x, buddy.position.y, pointerDisplayOpacity());
  drawWaveform(buddy.position.x, buddy.position.y, waveformOpacity.value, currentSeconds);
  drawSpinner(buddy.position.x, buddy.position.y, spinnerOpacity.value, currentSeconds);
}

let previousFrameSeconds = nowInSeconds();
function animationFrame() {
  const currentSeconds = nowInSeconds();
  // Capped so a stalled frame (window hidden, system busy) can't overshoot the spring.
  const elapsedSeconds = Math.min(currentSeconds - previousFrameSeconds, 1 / 30);
  previousFrameSeconds = currentSeconds;
  updateBuddy(elapsedSeconds, currentSeconds);
  updateAnimatedValues(elapsedSeconds, currentSeconds);
  drawFrame(currentSeconds);
  requestAnimationFrame(animationFrame);
}

// ---- User freehand drawing (only while the hotkey is held) ----
function finishCurrentStroke() {
  if (!currentStrokeGlobalPoints) return;
  const finishedStrokeGlobalPoints = currentStrokeGlobalPoints;
  currentStrokeGlobalPoints = null;
  invoke('user_stroke_added', { points: finishedStrokeGlobalPoints }).catch(() => {});
}

overlayCanvas.addEventListener('pointerdown', (event) => {
  // Left and right buttons both draw (ctrl + click arrives as a right click on the Mac).
  if (!isDrawingEnabled || (event.button !== 0 && event.button !== 2)) return;
  event.preventDefault();
  overlayCanvas.setPointerCapture(event.pointerId);
  // A new drawing replaces a fading one instead of fading out with it.
  if (annotationFadeStartSeconds !== null) {
    screenCircles = [];
    userStrokes = [];
    annotationFadeStartSeconds = null;
  }
  userStrokes.push([{ x: event.clientX, y: event.clientY }]);
  currentStrokeGlobalPoints = [[pageToGlobalX(event.clientX), pageToGlobalY(event.clientY)]];
});
overlayCanvas.addEventListener('pointermove', (event) => {
  if (!currentStrokeGlobalPoints) return;
  userStrokes[userStrokes.length - 1].push({ x: event.clientX, y: event.clientY });
  currentStrokeGlobalPoints.push([pageToGlobalX(event.clientX), pageToGlobalY(event.clientY)]);
});
overlayCanvas.addEventListener('pointerup', finishCurrentStroke);
overlayCanvas.addEventListener('pointercancel', finishCurrentStroke);
window.addEventListener('contextmenu', (event) => event.preventDefault());

// ---- Rust events ----
async function startOverlay() {
  monitorConfig = await invoke('overlay_ready', { monitorIndex });
  ({ voiceBuddyStyle, isChatMode, orbFrame: orbFrameGlobal } = await invoke('buddy_home_state'));
  resizeCanvasToWindow();
  isMouseOnThisScreen = isGlobalPointOnThisMonitor(pointerGlobal.x, pointerGlobal.y);
  buddy.position = pointerHomePosition();

  await listen('pointer-state', (event) => { pointerGlobal = event.payload; });
  await listen('buddy-voice-state', (event) => { voiceState = event.payload.state; });
  await listen('audio-level', (event) => { rawAudioLevel = event.payload.level; });
  await listen('point-at', (event) => {
    const { x, y, bubbleText } = event.payload;
    // Only the screen holding the target animates; the pointer flies in from its
    // home, off this screen's edge when home is on another monitor.
    if (!isGlobalPointOnThisMonitor(x, y)) return;
    startNavigatingToElement(globalToPageX(x), globalToPageY(y), bubbleText);
  });
  await listen('buddy-home-changed', (event) => {
    const wasCursorStyle = isCursorStyle();
    ({ voiceBuddyStyle, isChatMode, orbFrame: orbFrameGlobal } = event.payload);
    handleHomeChanged(wasCursorStyle);
  });
  await listen('pointer-away-monitors', (event) => { monitorIndicesWithPointerAway = event.payload.monitorIndices; });
  await listen('pen-position', (event) => { penGlobal = event.payload; });
  await listen('screen-circles', (event) => {
    const currentSeconds = nowInSeconds();
    annotationFadeStartSeconds = null;
    for (const circle of event.payload.circles) {
      const rect = { x: globalToPageX(circle.x), y: globalToPageY(circle.y), width: circle.width / monitorConfig.scaleFactor, height: circle.height / monitorConfig.scaleFactor };
      const intersectsThisMonitor = rect.x < pageWidth() && rect.x + rect.width > 0 && rect.y < pageHeight() && rect.y + rect.height > 0;
      if (!intersectsThisMonitor) continue;
      const points = handDrawnEllipsePoints(rect, Math.random() * 1000);
      screenCircles.push({ rect, points, cumulativeLengths: cumulativeLengthsOf(points), appearAtSeconds: currentSeconds + circle.appearDelaySeconds });
    }
  });
  await listen('fade-annotations', () => { if (annotationFadeStartSeconds === null) annotationFadeStartSeconds = nowInSeconds(); });
  await listen('clear-annotations', () => {
    // The hotkey also cancels a pointing flight in progress.
    if (buddy.mode !== 'restingAtHome') finishNavigationAndRestAtHome();
    screenCircles = [];
    userStrokes = [];
    currentStrokeGlobalPoints = null;
    annotationFadeStartSeconds = null;
  });
  await listen('drawing-enabled', (event) => {
    isDrawingEnabled = event.payload.enabled;
    // Hotkey released mid-drag: the stroke so far still counts.
    if (!isDrawingEnabled) finishCurrentStroke();
  });
  await listen('speaking-changed', (event) => { isSpeaking = event.payload.isSpeaking; });
  // Linux only: blanked while bhai takes a screenshot, which X11 can't take without us.
  await listen('hidden-for-capture', (event) => {
    overlayCanvas.style.visibility = event.payload.hidden ? 'hidden' : '';
  });
  await listen('onboarding-text', (event) => {
    const { text, visible } = event.payload;
    onboardingBubble.isVisible = visible;
    if (!visible) return;
    // A text that continues the current one keeps typing from where it is.
    if (onboardingBubble.opacity.value <= 0.001 || !text.startsWith(Array.from(onboardingBubble.fullText).slice(0, onboardingBubble.shownCharacterCount).join(''))) {
      onboardingBubble.shownCharacterCount = 0;
    }
    onboardingBubble.fullText = text;
  });

  requestAnimationFrame(animationFrame);
}

startOverlay();
