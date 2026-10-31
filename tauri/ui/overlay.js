// Per-monitor overlay: the blue buddy that follows the pointer, flies to
// [POINT] targets with a label bubble, the waveform while listening, the
// spinner while thinking, gold [CIRCLE] ellipses, the user's own gold strokes
// while the hotkey is held, the onboarding bubble and the speaking pill.
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
// The overlays can't see each other's flights, so while another monitor's buddy
// is pointing this one stays hidden for about one full point-and-return.
// ponytail: fixed guess; send a "pointing-finished" event if it ever looks wrong.
const OTHER_MONITOR_POINTING_HIDE_MILLISECONDS = 6000;
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
let anotherMonitorIsPointingUntil = 0;

// ---- Buddy state (BlueCursorView) ----
const buddy = {
  mode: 'following', // 'following' | 'navigating' | 'pointing'
  position: { x: -1000, y: -1000 },
  velocity: { x: 0, y: 0 },
  hasPlacedPosition: false,
  rotationDegrees: DEFAULT_TRIANGLE_ROTATION_DEGREES,
  targetRotationDegrees: DEFAULT_TRIANGLE_ROTATION_DEGREES,
  flightScale: 1,
  isReturningToCursor: false,
  pointerPositionWhenNavigationStarted: { x: 0, y: 0 },
  flight: null, // { start, end, control, startSeconds, durationSeconds, onComplete }
  navigationGeneration: 0, // invalidates bubble timers when a navigation is cancelled or replaced
};
const triangleOpacity = createAnimatedValue(0, 0.25);
const waveformOpacity = createAnimatedValue(0, 0.15);
const spinnerOpacity = createAnimatedValue(0, 0.15);
const pointerHiddenFactor = createAnimatedValue(1, 0.3); // 1 = shown, 0 = hidden with the system pointer

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

// ---- Buddy navigation (port of startNavigatingToElement and friends) ----
function pointerPagePosition() {
  return { x: globalToPageX(pointerGlobal.x), y: globalToPageY(pointerGlobal.y) };
}

function startNavigatingToElement(targetPageX, targetPageY, bubbleText) {
  // Beside the element rather than on top of it: 8 px right, 12 px below; 20 px inside the screen.
  const clampedTarget = {
    x: clamp(targetPageX + 8, 20, pageWidth() - 20),
    y: clamp(targetPageY + 12, 20, pageHeight() - 20),
  };
  buddy.pointerPositionWhenNavigationStarted = pointerPagePosition();
  buddy.mode = 'navigating';
  buddy.isReturningToCursor = false;
  buddy.navigationGeneration += 1;
  navigationBubble.text = '';
  navigationBubble.opacity.value = navigationBubble.opacity.target = 0;
  const navigationGeneration = buddy.navigationGeneration;
  animateBezierFlightArc(clampedTarget, () => {
    if (buddy.mode !== 'navigating' || buddy.navigationGeneration !== navigationGeneration) return;
    startPointingAtElement(bubbleText, navigationGeneration);
  });
}

function animateBezierFlightArc(destination, onComplete) {
  const start = { ...buddy.position };
  const distance = Math.hypot(destination.x - start.x, destination.y - start.y);
  // Short hops are quick, long flights more dramatic: 0.6 s to 1.4 s.
  const durationSeconds = clamp(distance / 800, 0.6, 1.4);
  // Midpoint raised so the buddy flies in a parabolic arc.
  const arcHeight = Math.min(distance * 0.2, 80);
  const control = { x: (start.x + destination.x) / 2, y: (start.y + destination.y) / 2 - arcHeight };
  buddy.flight = { start, end: destination, control, startSeconds: nowInSeconds(), durationSeconds, onComplete };
}

function stepFlight(currentSeconds) {
  const flight = buddy.flight;
  if (!flight) return;
  const linearProgress = (currentSeconds - flight.startSeconds) / flight.durationSeconds;
  if (linearProgress >= 1) {
    buddy.position = { ...flight.end };
    buddy.velocity = { x: 0, y: 0 };
    buddy.flightScale = 1;
    buddy.flight = null;
    flight.onComplete();
    return;
  }
  const easedProgress = smoothstep(Math.max(linearProgress, 0));
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
  buddy.flightScale = 1 + Math.sin(Math.max(linearProgress, 0) * Math.PI) * 0.3;
}

function startPointingAtElement(bubbleText, navigationGeneration) {
  buddy.mode = 'pointing';
  buddy.targetRotationDegrees = DEFAULT_TRIANGLE_ROTATION_DEGREES;
  navigationBubble.text = '';
  navigationBubble.opacity.value = navigationBubble.opacity.target = 1;
  navigationBubble.scale.value = navigationBubble.scale.target = 0.5;
  const pointerPhrase = bubbleText
    || NAVIGATION_POINTER_PHRASES[Math.floor(Math.random() * NAVIGATION_POINTER_PHRASES.length)];
  const phraseCharacters = Array.from(pointerPhrase);
  const isStillThisNavigation = () => buddy.mode === 'pointing' && buddy.navigationGeneration === navigationGeneration;

  const streamNextCharacter = (characterIndex) => {
    if (!isStillThisNavigation()) return;
    if (characterIndex >= phraseCharacters.length) {
      // Hold for 3 s, fade 0.5 s, then fly back to the pointer.
      setTimeout(() => {
        if (!isStillThisNavigation()) return;
        navigationBubble.opacity.target = 0;
        setTimeout(() => { if (isStillThisNavigation()) startFlyingBackToCursor(); }, 500);
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

function startFlyingBackToCursor() {
  const pointerPage = pointerPagePosition();
  buddy.pointerPositionWhenNavigationStarted = pointerPage;
  buddy.mode = 'navigating';
  buddy.isReturningToCursor = true;
  animateBezierFlightArc({ x: pointerPage.x + FOLLOW_OFFSET_X, y: pointerPage.y + FOLLOW_OFFSET_Y }, finishNavigationAndResumeFollowing);
}

function finishNavigationAndResumeFollowing() {
  buddy.flight = null;
  buddy.mode = 'following';
  buddy.isReturningToCursor = false;
  buddy.targetRotationDegrees = DEFAULT_TRIANGLE_ROTATION_DEGREES;
  buddy.flightScale = 1;
  buddy.navigationGeneration += 1;
  navigationBubble.text = '';
  navigationBubble.opacity.value = navigationBubble.opacity.target = 0;
  navigationBubble.scale.value = navigationBubble.scale.target = 1;
}

// ---- Per-frame update (port of startTrackingCursor) ----
let buddyIsOnThisMonitor = false;
let isBuddyHiddenBecauseSystemPointerIsHidden = false;

function springBuddyToward(targetX, targetY, elapsedSeconds) {
  // Snap when the buddy arrives from another monitor, so it never sweeps across this one.
  if (!buddy.hasPlacedPosition || !buddyIsOnThisMonitor) {
    buddy.position = { x: targetX, y: targetY };
    buddy.velocity = { x: 0, y: 0 };
    buddy.hasPlacedPosition = true;
    return;
  }
  for (const axis of ['x', 'y']) {
    const target = axis === 'x' ? targetX : targetY;
    const springForce = FOLLOW_SPRING_STIFFNESS * (target - buddy.position[axis]) - FOLLOW_SPRING_DAMPING * buddy.velocity[axis];
    buddy.velocity[axis] += springForce * elapsedSeconds;
    buddy.position[axis] += buddy.velocity[axis] * elapsedSeconds;
  }
}

function updateBuddy(elapsedSeconds, currentSeconds) {
  stepFlight(currentSeconds);

  // While the whiteboard or 3D model draws, the buddy is the pen: its tip sits on the drawing head.
  if (penGlobal && buddy.mode === 'following') {
    const wasOnThisMonitor = buddyIsOnThisMonitor;
    buddyIsOnThisMonitor = isGlobalPointOnThisMonitor(penGlobal.x, penGlobal.y);
    isBuddyHiddenBecauseSystemPointerIsHidden = false;
    if (!wasOnThisMonitor) buddy.hasPlacedPosition = false;
    springBuddyToward(globalToPageX(penGlobal.x) + PEN_TIP_OFFSET_X, globalToPageY(penGlobal.y) + PEN_TIP_OFFSET_Y, elapsedSeconds);
    return;
  }

  const wasOnThisMonitor = buddyIsOnThisMonitor;
  buddyIsOnThisMonitor = isGlobalPointOnThisMonitor(pointerGlobal.x, pointerGlobal.y);
  if (!wasOnThisMonitor) buddy.hasPlacedPosition = false;

  // Follow the real pointer's visibility, but never hide mid-answer, mid-point or mid-flight.
  isBuddyHiddenBecauseSystemPointerIsHidden = !pointerGlobal.visible && voiceState === 'idle' && buddy.mode === 'following';

  // Only the return flight can be cancelled, by moving the pointer more than 100 px.
  if (buddy.mode === 'navigating' && buddy.isReturningToCursor) {
    const pointerPage = pointerPagePosition();
    const distanceFromNavigationStart = Math.hypot(
      pointerPage.x - buddy.pointerPositionWhenNavigationStarted.x,
      pointerPage.y - buddy.pointerPositionWhenNavigationStarted.y,
    );
    if (distanceFromNavigationStart > 100) finishNavigationAndResumeFollowing();
    return;
  }
  if (buddy.mode !== 'following') return;

  const pointerPage = pointerPagePosition();
  springBuddyToward(pointerPage.x + FOLLOW_OFFSET_X, pointerPage.y + FOLLOW_OFFSET_Y, elapsedSeconds);
}

function isBuddyVisibleOnThisMonitor() {
  if (buddy.mode !== 'following') return true;
  // Another monitor's buddy is pointing: only one buddy is ever visible.
  if (performance.now() < anotherMonitorIsPointingUntil) return false;
  return buddyIsOnThisMonitor;
}

function updateAnimatedValues(elapsedSeconds, currentSeconds) {
  const isVisibleHere = isBuddyVisibleOnThisMonitor();
  triangleOpacity.target = isVisibleHere && (voiceState === 'idle' || voiceState === 'responding') ? 1 : 0;
  waveformOpacity.target = isVisibleHere && voiceState === 'listening' ? 1 : 0;
  spinnerOpacity.target = isVisibleHere && voiceState === 'processing' ? 1 : 0;
  // Changing monitors is instant; voice state changes cross-fade.
  if (!isVisibleHere) triangleOpacity.value = waveformOpacity.value = spinnerOpacity.value = 0;
  pointerHiddenFactor.target = isBuddyHiddenBecauseSystemPointerIsHidden ? 0 : 1;
  speakingPillProgress.target = isSpeaking && buddyIsOnThisMonitor ? 1 : 0;
  onboardingBubble.opacity.target = onboardingBubble.isVisible && onboardingBubble.fullText && buddyIsOnThisMonitor ? 1 : 0;

  for (const animatedValue of [triangleOpacity, waveformOpacity, spinnerOpacity, pointerHiddenFactor,
    speakingPillProgress, onboardingBubble.opacity, navigationBubble.opacity, navigationBubble.scale]) {
    stepAnimatedValue(animatedValue, elapsedSeconds);
  }

  // Rotation eases back to the pointer angle over 0.3 s, except mid-flight where the tangent drives it.
  if (buddy.mode !== 'navigating') {
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
function drawTriangle(centerX, centerY, opacity) {
  if (opacity <= 0.001) return;
  const triangleHeight = TRIANGLE_SIZE * Math.sqrt(3) / 2;
  drawingContext.save();
  drawingContext.globalAlpha = opacity;
  drawingContext.translate(centerX, centerY);
  drawingContext.rotate(buddy.rotationDegrees * Math.PI / 180);
  drawingContext.scale(buddy.flightScale, buddy.flightScale);
  drawingContext.shadowColor = CURSOR_COLOR;
  drawingContext.shadowBlur = (8 + (buddy.flightScale - 1) * 20) * (window.devicePixelRatio || 1);
  drawingContext.fillStyle = CURSOR_COLOR;
  drawingContext.beginPath();
  drawingContext.moveTo(0, -triangleHeight / 1.5);
  drawingContext.lineTo(-TRIANGLE_SIZE / 2, triangleHeight / 3);
  drawingContext.lineTo(TRIANGLE_SIZE / 2, triangleHeight / 3);
  drawingContext.closePath();
  drawingContext.fill();
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

/// The blue rounded label next to the buddy (navigation and onboarding bubbles).
function drawBubble(bubbleText, opacity, scale, glowBoost) {
  if (!bubbleText || opacity <= 0.001) return;
  drawingContext.save();
  drawingContext.font = BUBBLE_FONT;
  const textWidth = drawingContext.measureText(bubbleText).width;
  const bubbleWidth = textWidth + 16;
  const bubbleHeight = 22;
  // Left edge 10 px right of the buddy, vertically centred 18 px below it.
  const bubbleCenterX = buddy.position.x + 10 + bubbleWidth / 2;
  const bubbleCenterY = buddy.position.y + 18;
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
  drawBubble(onboardingText, onboardingBubble.opacity.value, 1, 0);
  if (buddy.mode === 'pointing') {
    drawBubble(navigationBubble.text, navigationBubble.opacity.value, navigationBubble.scale.value, 1 - navigationBubble.scale.value);
  }

  const buddyOpacity = pointerHiddenFactor.value;
  drawTriangle(buddy.position.x, buddy.position.y, triangleOpacity.value * buddyOpacity);
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
  resizeCanvasToWindow();

  await listen('pointer-state', (event) => { pointerGlobal = event.payload; });
  await listen('buddy-voice-state', (event) => { voiceState = event.payload.state; });
  await listen('audio-level', (event) => { rawAudioLevel = event.payload.level; });
  await listen('point-at', (event) => {
    const { x, y, bubbleText } = event.payload;
    if (!isGlobalPointOnThisMonitor(x, y)) {
      // The buddy on the target's monitor flies; this one steps aside.
      anotherMonitorIsPointingUntil = performance.now() + OTHER_MONITOR_POINTING_HIDE_MILLISECONDS;
      if (buddy.mode !== 'following') finishNavigationAndResumeFollowing();
      return;
    }
    anotherMonitorIsPointingUntil = 0;
    // The buddy flies from wherever it is shown on this monitor.
    if (!buddyIsOnThisMonitor) {
      const pointerPage = pointerPagePosition();
      buddy.position = { x: clamp(pointerPage.x, 0, pageWidth()), y: clamp(pointerPage.y, 0, pageHeight()) };
    }
    startNavigatingToElement(globalToPageX(x), globalToPageY(y), bubbleText);
  });
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
