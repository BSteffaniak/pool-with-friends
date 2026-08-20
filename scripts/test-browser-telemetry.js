#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const vm = require("node:vm");

const path = process.argv[2] ?? "packages/client/web/bootstrap.js";
const source = fs.readFileSync(path, "utf8");
const startMarker = "function hasRequiredBackgroundIntervals(durations) {";
const endMarker = "function externalObservations() {";
const start = source.indexOf(startMarker);
const end = source.indexOf(endMarker, start);
if (start < 0 || end < 0) {
  throw new Error("cannot locate frame sampler in bootstrap.js");
}

const rafCallbacks = [];
const context = {
  captureActive: false,
  lastFrameAt: null,
  frameWindowStartedAt: null,
  frameWindowCount: 0,
  telemetry: {
    captureStartedAt: null,
    captureStoppedAt: null,
    hiddenStartedAt: null,
    hiddenDurationMs: 0,
    frameCount: 0,
    frameGapSamplesMs: [],
    maximumFrameGapMs: 0,
    currentFps: null,
    fpsSamples: [],
    minimumFps: null,
    maximumFps: null,
  },
  feasibilityEnabled: false,
  performance: { now: () => 0, memory: null },
  refreshMetrics() {},
  window: {
    requestAnimationFrame(callback) {
      rafCallbacks.push(callback);
    },
  },
};
vm.createContext(context);
vm.runInContext(
  `${source.slice(start, end)}\nthis.frame = frame; this.resetFrameWindow = resetFrameWindow; this.captureDuration = captureDuration; this.hasRequiredBackgroundIntervals = hasRequiredBackgroundIntervals;`,
  context,
);

context.captureActive = true;
context.frame(0);
context.frame(16);
context.frame(1_016);
if (context.telemetry.maximumFrameGapMs !== 1_000) {
  throw new Error(`expected active frame gap to be sampled, got ${context.telemetry.maximumFrameGapMs}`);
}

context.resetFrameWindow();
context.frame(61_016);
context.frame(61_032);
if (context.telemetry.maximumFrameGapMs !== 1_000) {
  throw new Error("hidden interval leaked into maximum frame gap");
}
if (context.telemetry.frameGapSamplesMs.some((gap) => gap > 1_000)) {
  throw new Error("hidden interval leaked into frame-gap samples");
}
if (context.telemetry.minimumFps !== null && context.telemetry.minimumFps < 1) {
  throw new Error("hidden interval produced a false near-zero FPS sample");
}
if (rafCallbacks.length !== 5) {
  throw new Error("frame sampler stopped scheduling animation frames");
}

context.telemetry.captureStartedAt = 1_000;
context.telemetry.captureStoppedAt = 71_000;
context.telemetry.hiddenDurationMs = 10_000;
if (context.captureDuration() !== 60_000) {
  throw new Error("completed capture duration included hidden time");
}
context.telemetry.captureStoppedAt = null;
context.telemetry.hiddenStartedAt = 61_000;
context.performance.now = () => 71_000;
if (context.captureDuration() !== 50_000) {
  throw new Error("active capture duration included the current hidden interval");
}
if (!context.hasRequiredBackgroundIntervals([30_000, 45_000])) {
  throw new Error("two valid background intervals were rejected");
}
if (context.hasRequiredBackgroundIntervals([30_000, 29_999])) {
  throw new Error("a short background interval was accepted");
}

const observationsStart = source.indexOf("function externalObservations() {");
const observationsEnd = source.indexOf("function physicalCheckResults() {", observationsStart);
if (observationsStart < 0 || observationsEnd < 0) {
  throw new Error("cannot locate external observation validation in bootstrap.js");
}
function observationInput(value) {
  return {
    value,
    labels: [{ textContent: "fixture" }],
    reportValidity: () => true,
    focus() {},
  };
}
const observationsContext = {
  firstVisibleInput: observationInput("4000"),
  firstInputInput: observationInput("4200"),
  steadyMemoryInput: observationInput("120"),
  peakMemoryInput: observationInput("180"),
  thermalResultInput: observationInput("no-warning"),
  reloadObservedInput: observationInput("no"),
  telemetry: { clientReadyMs: 1000, firstCanvasContactMs: 1200 },
  showStatus(message) {
    observationsContext.status = message;
  },
  Number,
  Object,
};
vm.createContext(observationsContext);
vm.runInContext(
  `${source.slice(observationsStart, observationsEnd)}\nthis.requireExternalObservations = requireExternalObservations;`,
  observationsContext,
);
observationsContext.firstVisibleInput.value = "not-a-number";
if (observationsContext.requireExternalObservations()) {
  throw new Error("external observations accepted a non-finite numeric value");
}
if (!observationsContext.status.includes("valid finite values")) {
  throw new Error("invalid external observation lacked actionable guidance");
}
observationsContext.firstVisibleInput.value = "4000";
observationsContext.telemetry.clientReadyMs = null;
if (observationsContext.requireExternalObservations()) {
  throw new Error("external observations accepted missing client timing evidence");
}

const metadataStart = source.indexOf("function detectBrowserVersion(userAgent, family) {");
const metadataEnd = source.indexOf("const captureLockedInputs = [", metadataStart);
if (metadataStart < 0 || metadataEnd < 0) {
  throw new Error("cannot locate capture metadata validation in bootstrap.js");
}

function input(value) {
  return {
    value,
    labels: [{ textContent: "fixture" }],
    reportValidity: () => true,
    focus() {},
  };
}

const metadataContext = {
  platformInput: input("desktop"),
  hardwareModelInput: input("fixture-hardware"),
  osVersionInput: input("fixture-os"),
  browserFamilyInput: input("chrome"),
  browserVersionInput: input("151.0.7922.138"),
  cacheStateInput: input("cold"),
  minimumVersionInput: input("no"),
  presentationTierInput: input("default"),
  runNumberInput: input("1"),
  detectedBrowserFamily: "chrome",
  detectedPlatform: "desktop",
  navigator: {
    userAgent:
      "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.7922.138 Safari/537.36",
  },
  statusOutput: { textContent: "" },
  showStatus(message) {
    metadataContext.statusOutput.textContent = message;
  },
};
vm.createContext(metadataContext);
vm.runInContext(
  `${source.slice(metadataStart, metadataEnd)}\nthis.requireTestMetadata = requireTestMetadata; this.testMetadata = testMetadata;`,
  metadataContext,
);

if (!metadataContext.requireTestMetadata()) {
  throw new Error(
    `matching declared/detected browser was rejected: ${metadataContext.statusOutput.textContent}`,
  );
}
metadataContext.platformInput.value = "iphone";
if (metadataContext.requireTestMetadata()) {
  throw new Error("mismatched declared/detected platform was accepted");
}
if (!metadataContext.statusOutput.textContent.includes("platform")) {
  throw new Error("platform mismatch did not produce an actionable error");
}
metadataContext.platformInput.value = "desktop";
metadataContext.browserFamilyInput.value = "firefox";
if (metadataContext.requireTestMetadata()) {
  throw new Error("mismatched declared/detected browser was accepted");
}
if (!metadataContext.statusOutput.textContent.includes("does not match")) {
  throw new Error("browser mismatch did not produce an actionable error");
}
metadataContext.browserFamilyInput.value = "chrome";
metadataContext.browserVersionInput.value = "150.0.0.0";
if (metadataContext.requireTestMetadata()) {
  throw new Error("mismatched declared/detected browser version was accepted");
}
if (!metadataContext.statusOutput.textContent.includes("version")) {
  throw new Error("browser-version mismatch did not produce an actionable error");
}
metadataContext.browserVersionInput.value = "0151.0.7922.138";
if (metadataContext.requireTestMetadata()) {
  throw new Error("non-canonical declared browser version was accepted");
}
metadataContext.browserVersionInput.value = "151.0.7922";
if (metadataContext.requireTestMetadata()) {
  throw new Error("partial declared browser version was accepted");
}
metadataContext.browserVersionInput.value = "151.0.7922.138";
metadataContext.hardwareModelInput.value = " fixture-hardware";
if (metadataContext.requireTestMetadata()) {
  throw new Error("capture metadata accepted surrounding whitespace");
}
metadataContext.hardwareModelInput.value = "fixture-hardware";
metadataContext.runNumberInput.value = "1.5";
if (metadataContext.requireTestMetadata()) {
  throw new Error("capture metadata accepted a non-integer run number");
}
metadataContext.runNumberInput.value = "1";
if (metadataContext.testMetadata().run_number !== 1) {
  throw new Error("test metadata did not preserve the validated integer run number");
}
metadataContext.detectedBrowserFamily = "unknown";
if (metadataContext.requireTestMetadata()) {
  throw new Error("unknown detected browser was accepted");
}

void (async () => {
  const audioStart = source.indexOf("function updateAudioControls() {");
const audioEnd = source.indexOf("function toggleMute() {", audioStart);
if (audioStart < 0 || audioEnd < 0) {
  throw new Error("cannot locate audio probe lifecycle in bootstrap.js");
}

function audioControl() {
  return { disabled: false, textContent: "" };
}

const audioContextFixture = {
  state: "suspended",
  currentTime: 1,
  destination: {},
  resumeCalls: 0,
  addEventListener() {},
  async resume() {
    this.resumeCalls += 1;
    this.state = "running";
  },
  createOscillator() {
    return {
      type: "",
      frequency: { setValueAtTime() {}, exponentialRampToValueAtTime() {} },
      connect() {},
      disconnect() {},
      start() {},
      stop() {},
      addEventListener() {},
    };
  },
  createGain() {
    return {
      gain: { setValueAtTime() {}, exponentialRampToValueAtTime() {} },
      connect() {},
      disconnect() {},
    };
  },
};
const audioContext = {
  audioContext: null,
  audioNeedsExplicitResume: false,
  audioProbeInFlight: false,
  audioLifecycleOperations: 0,
  audioLifecycleTransition: Promise.resolve(),
  audioButton: audioControl(),
  captureButton: { focus() {} },
  markEventButton: audioControl(),
  resetButton: audioControl(),
  downloadButton: audioControl(),
  toggleToolsButton: audioControl(),
  muteButton: audioControl(),
  masterGain: { gain: { setValueAtTime() {} } },
  captureActive: true,
  telemetry: {
    captureStoppedAt: null,
    audio: {
      supported: true,
      state: "suspended",
      gestureStarts: 0,
      backgroundSuspensions: 0,
      explicitResumes: 0,
      mutedPlaybackAttempts: 0,
      audiblePlaybackAttempts: 0,
      transitionFailures: 0,
      muted: false,
    },
  },
  refreshMetrics() {},
  updateCaptureControls() {},
  stoppedCaptureNeedsAudioResume() {
    return (
      !audioContext.captureActive &&
      audioContext.telemetry.captureStoppedAt !== null &&
      audioContext.audioContext !== null &&
      (audioContext.audioNeedsExplicitResume || audioContext.audioContext.state !== "running")
    );
  },
  Promise,
  window: {
    AudioContext: class FixtureAudioContext {
      constructor() {
        return audioContextFixture;
      }
    },
  },
  createAudioGraph: undefined,
  document: { hidden: false },
  showStatus(message) {
    audioContext.status = message;
  },
  Error,
};
vm.createContext(audioContext);
vm.runInContext(
  `${source.slice(audioStart, audioEnd)}\nthis.playAudioProbe = playAudioProbe;`,
  audioContext,
);
audioContext.audioNeedsExplicitResume = true;
await audioContext.playAudioProbe();
if (audioContextFixture.resumeCalls !== 1 || audioContext.telemetry.audio.explicitResumes !== 1) {
  throw new Error("explicit audio resume was not counted after a successful resume");
}
if (audioContext.telemetry.audio.gestureStarts !== 1 || audioContext.audioNeedsExplicitResume) {
  throw new Error("successful audio probe did not record a gesture and clear resume state");
}

const failedAudioContext = {
  ...audioContextFixture,
  state: "suspended",
  resumeCalls: 0,
  async resume() {
    this.resumeCalls += 1;
    throw new Error("fixture resume failure");
  },
};
audioContext.audioContext = failedAudioContext;
audioContext.audioNeedsExplicitResume = true;
await audioContext.playAudioProbe();
if (!audioContext.audioNeedsExplicitResume || !audioContext.status.includes("fixture resume failure")) {
  throw new Error("failed audio resume did not remain resumable with an actionable status");
}
if (audioContext.telemetry.audio.transitionFailures !== 1) {
  throw new Error("failed audio resume did not invalidate the active capture");
}
if (audioContext.telemetry.audio.gestureStarts !== 1) {
  throw new Error("failed audio resume was incorrectly counted as successful playback");
}

audioContext.captureActive = false;
audioContext.telemetry.audio.transitionFailures = 0;
await audioContext.playAudioProbe();
if (!audioContext.status.includes("Start a capture")) {
  throw new Error("audio probe was allowed outside an active capture");
}
if (audioContext.telemetry.audio.gestureStarts !== 1) {
  throw new Error("out-of-capture audio probe changed playback evidence");
}
audioContext.captureActive = true;
audioContext.telemetry.audio.transitionFailures = 1;

failedAudioContext.state = "suspended";
failedAudioContext.resumeCalls = 0;
failedAudioContext.resume = async function resume() {
  this.resumeCalls += 1;
  this.state = "running";
};
audioContext.audioNeedsExplicitResume = true;
audioContext.document.hidden = true;
await audioContext.playAudioProbe();
if (failedAudioContext.resumeCalls !== 0 || !audioContext.audioNeedsExplicitResume) {
  throw new Error("hidden page resumed audio or cleared explicit-resume state");
}
if (!audioContext.status.includes("foreground")) {
  throw new Error("hidden-page resume failure did not provide foreground guidance");
}
audioContext.document.hidden = false;

const stoppedCaptureAudioContext = {
  ...audioContextFixture,
  state: "interrupted",
  resumeCalls: 0,
  async resume() {
    this.resumeCalls += 1;
    this.state = "running";
  },
};
audioContext.audioContext = stoppedCaptureAudioContext;
audioContext.captureActive = false;
audioContext.telemetry.captureStoppedAt = 61_000;
audioContext.telemetry.audio.gestureStarts = 2;
audioContext.telemetry.audio.explicitResumes = 2;
audioContext.audioNeedsExplicitResume = true;
await audioContext.playAudioProbe();
if (
  stoppedCaptureAudioContext.resumeCalls !== 1 ||
  stoppedCaptureAudioContext.state !== "running" ||
  audioContext.telemetry.audio.gestureStarts !== 2 ||
  audioContext.telemetry.audio.explicitResumes !== 2 ||
  !audioContext.status.includes("ready for validation")
) {
  throw new Error("stopped capture could not recover audio without mutating evidence counters");
}

audioContext.captureActive = true;
audioContext.telemetry.captureStoppedAt = null;
audioContext.telemetry.audio.gestureStarts = 1;

const interruptedAudioContext = {
  ...audioContextFixture,
  state: "interrupted",
  resumeCalls: 0,
  async resume() {
    this.resumeCalls += 1;
    this.state = "running";
  },
};
audioContext.audioContext = interruptedAudioContext;
audioContext.captureActive = true;
audioContext.telemetry.audio.transitionFailures = 0;
audioContext.audioNeedsExplicitResume = true;
await audioContext.playAudioProbe();
if (
  interruptedAudioContext.resumeCalls !== 1 ||
  interruptedAudioContext.state !== "running" ||
  audioContext.telemetry.audio.explicitResumes !== 3
) {
  throw new Error("interrupted audio context did not explicitly resume");
}

const unknownStoppedAudioContext = {
  ...audioContextFixture,
  state: "closed-by-browser",
  resumeCalls: 0,
  async resume() {
    this.resumeCalls += 1;
    this.state = "running";
  },
};
audioContext.audioContext = unknownStoppedAudioContext;
audioContext.telemetry.audio.gestureStarts = 1;
audioContext.audioNeedsExplicitResume = true;
await audioContext.playAudioProbe();
if (
  unknownStoppedAudioContext.resumeCalls !== 1 ||
  unknownStoppedAudioContext.state !== "running" ||
  audioContext.telemetry.audio.explicitResumes !== 4
) {
  throw new Error("unknown non-running audio context did not explicitly resume");
}

const backgroundAudioContext = {
  state: "interrupted",
  suspendCalls: 0,
  async suspend() {
    this.suspendCalls += 1;
  },
};
audioContext.audioContext = backgroundAudioContext;
audioContext.audioNeedsExplicitResume = false;
audioContext.audioLifecycleOperations = 0;
audioContext.document.hidden = true;
audioContext.requestBackgroundAudioSuspension();
await audioContext.audioLifecycleTransition;
await Promise.resolve();
if (
  backgroundAudioContext.suspendCalls !== 0 ||
  audioContext.telemetry.audio.backgroundSuspensions !== 1 ||
  !audioContext.audioNeedsExplicitResume
) {
  throw new Error("browser-interrupted background audio was not recorded as suspended");
}

audioContext.document.hidden = false;
while (audioContext.audioLifecycleOperations > 0) {
  await Promise.resolve();
}

const resetLifecycleStart = source.indexOf("function updateAudioControls() {");
const resetLifecycleEnd = source.indexOf("function toggleTools() {", resetLifecycleStart);
if (resetLifecycleStart < 0 || resetLifecycleEnd < 0) {
  throw new Error("cannot locate audio close lifecycle in bootstrap.js");
}
const resetLifecycleContext = {
  feasibilityEnabled: false,
  captureActive: false,
  audioContextPendingClose: null,
  audioNeedsExplicitResume: false,
  audioProbeInFlight: false,
  audioLifecycleOperations: 0,
  audioButton: audioControl(),
  muteButton: audioControl(),
  markEventButton: audioControl(),
  resetButton: audioControl(),
  downloadButton: audioControl(),
  toggleToolsButton: audioControl(),
  captureButton: { focus() {} },
  masterGain: {},
  telemetry: { audio: { supported: true, state: "running", transitionFailures: 0, muted: false } },
  physicalChecks: { querySelectorAll: () => [] },
  platformInput: { value: "desktop", focus() {} },
  hardwareModelInput: { value: "fixture" },
  osVersionInput: { value: "fixture" },
  browserFamilyInput: { value: "chrome" },
  browserVersionInput: { value: "151" },
  cacheStateInput: { value: "warm" },
  minimumVersionInput: { value: "no" },
  runNumberInput: { value: "1" },
  firstVisibleInput: { value: "1" },
  firstInputInput: { value: "2" },
  steadyMemoryInput: { value: "3" },
  peakMemoryInput: { value: "4" },
  thermalResultInput: { value: "no-warning" },
  reloadObservedInput: { value: "no" },
  eventLabelInput: { value: "fixture" },
  rejectForAudioLifecycle: () => false,
  showStatus(message) {
    resetLifecycleContext.status = message;
  },
  refreshMetrics() {},
  updateCaptureControls() {},
  stoppedCaptureNeedsAudioResume: () => false,
  Promise,
  Error,
};
let resolveAudioClose;
const closingAudioContext = {
  state: "running",
  closeCalls: 0,
  close() {
    this.closeCalls += 1;
    return new Promise((resolve) => {
      resolveAudioClose = () => {
        this.state = "closed";
        resolve();
      };
    });
  },
};
resetLifecycleContext.audioContext = closingAudioContext;
vm.createContext(resetLifecycleContext);
vm.runInContext(
  `${source.slice(resetLifecycleStart, resetLifecycleEnd)}\nthis.resetReportForm = resetReportForm;`,
  resetLifecycleContext,
);
resetLifecycleContext.resetReportForm();
if (
  closingAudioContext.closeCalls !== 1 ||
  resetLifecycleContext.audioLifecycleOperations !== 1 ||
  resetLifecycleContext.audioContextPendingClose !== closingAudioContext ||
  resetLifecycleContext.telemetry.audio.state !== "running"
) {
  throw new Error("report reset did not preserve an in-flight audio close state");
}
resolveAudioClose();
await Promise.resolve();
await Promise.resolve();
await Promise.resolve();
if (
  resetLifecycleContext.audioLifecycleOperations !== 0 ||
  resetLifecycleContext.audioContextPendingClose !== null ||
  resetLifecycleContext.telemetry.audio.state !== "not started" ||
  resetLifecycleContext.telemetry.audio.transitionFailures !== 0
) {
  throw new Error("successful audio close did not settle cleanly");
}

const resetCloseFailureContext = {
  ...resetLifecycleContext,
  audioContextPendingClose: null,
  audioLifecycleOperations: 0,
  telemetry: { audio: { supported: true, state: "running", transitionFailures: 0, muted: false } },
  showStatus(message) {
    resetCloseFailureContext.status = message;
  },
};
const closeFailureContext = {
  state: "running",
  async close() {
    throw new Error("fixture close failure");
  },
};
resetCloseFailureContext.audioContext = closeFailureContext;
vm.createContext(resetCloseFailureContext);
vm.runInContext(
  `${source.slice(resetLifecycleStart, resetLifecycleEnd)}\nthis.resetReportForm = resetReportForm;`,
  resetCloseFailureContext,
);
resetCloseFailureContext.resetReportForm();
await Promise.resolve();
await Promise.resolve();
await Promise.resolve();
await Promise.resolve();
await Promise.resolve();
if (
  resetCloseFailureContext.audioLifecycleOperations !== 0 ||
  resetCloseFailureContext.audioContextPendingClose !== null ||
  resetCloseFailureContext.telemetry.audio.transitionFailures !== 1 ||
  !resetCloseFailureContext.status.includes("fixture close failure")
) {
  throw new Error("failed audio close did not settle fail-closed");
}

const muteStart = source.indexOf("function toggleMute() {");
const muteEnd = source.indexOf("function markEvent() {", muteStart);
if (muteStart < 0 || muteEnd < 0) {
  throw new Error("cannot locate audio mute capture guard in bootstrap.js");
}
const muteContext = {
  masterGain: { gain: { setValueAtTime() {} } },
  audioContext: { currentTime: 1 },
  captureActive: false,
  audioLifecycleBusy: () => false,
  showStatus(message) {
    muteContext.status = message;
  },
};
vm.createContext(muteContext);
vm.runInContext(`${source.slice(muteStart, muteEnd)}\nthis.toggleMute = toggleMute;`, muteContext);
muteContext.toggleMute();
if (!muteContext.status.includes("active capture")) {
  throw new Error("audio mute was allowed outside an active capture");
}

const startGuardStart = source.indexOf("function startCapture() {");
const startGuardEnd = source.indexOf("function stopCapture() {", startGuardStart);
if (startGuardStart < 0 || startGuardEnd < 0) {
  throw new Error("cannot locate capture-start audio-operation guard in bootstrap.js");
}
const startGuardContext = {
  telemetry: { audio: { transitionFailures: 0 } },
  audioContext: null,
  audioProbeInFlight: false,
  audioLifecycleOperations: 1,
  audioButton: { focus() {} },
  rejectForAudioLifecycle() {
    startGuardContext.status = "Wait for audio operations to finish";
    return true;
  },
  showStatus(message) {
    startGuardContext.status = message;
  },
};
vm.createContext(startGuardContext);
vm.runInContext(
  `${source.slice(startGuardStart, startGuardEnd)}\nthis.startCapture = startCapture;`,
  startGuardContext,
);
startGuardContext.startCapture();
if (!startGuardContext.status.includes("audio operations")) {
  throw new Error("capture start did not reject an in-flight audio operation");
}
startGuardContext.rejectForAudioLifecycle = () => false;
startGuardContext.telemetry = { audio: { transitionFailures: 1 } };
startGuardContext.audioContext = null;
startGuardContext.resetButton = { focus() {} };
startGuardContext.startCapture();
if (!startGuardContext.status.includes("Reset the report form")) {
  throw new Error("capture start did not preserve a pre-capture audio transition failure");
}
startGuardContext.telemetry.audio.transitionFailures = 0;
startGuardContext.audioContext = { state: "running" };
startGuardContext.startCapture();
if (!startGuardContext.status.includes("reusing an existing audio context")) {
  throw new Error("capture start reused setup audio without explicit reset");
}

const captureGuardStart = source.indexOf("function stopCapture() {");
const captureGuardEnd = source.indexOf("function updateAudioControls() {", captureGuardStart);
if (captureGuardStart < 0 || captureGuardEnd < 0) {
  throw new Error("cannot locate capture audio-operation guard in bootstrap.js");
}
const captureGuardContext = {
  audioProbeInFlight: false,
  audioLifecycleOperations: 1,
  document: { hidden: false },
  audioButton: { focus() {} },
  rejectForAudioLifecycle() {
    captureGuardContext.status = "Wait for audio operations to finish";
    return true;
  },
  showStatus(message) {
    captureGuardContext.status = message;
  },
};
vm.createContext(captureGuardContext);
vm.runInContext(
  `${source.slice(captureGuardStart, captureGuardEnd)}\nthis.stopCapture = stopCapture;`,
  captureGuardContext,
);
captureGuardContext.stopCapture();
if (!captureGuardContext.status.includes("audio operations")) {
  throw new Error("capture stop did not reject an in-flight audio operation");
}

const stoppedCaptureContext = {
  audioProbeInFlight: false,
  audioLifecycleOperations: 0,
  captureActive: true,
  telemetry: {
    captureStoppedAt: null,
    hiddenStartedAt: null,
    finalOrientation: null,
  },
  performance: { now: () => 61_000 },
  window: { screen: { orientation: { type: "landscape-primary" } } },
  document: { hidden: false },
  tools: { classList: { remove() {} } },
  toggleToolsButton: { textContent: "", setAttribute() {} },
  captureButton: { textContent: "" },
  audioContext: { state: "interrupted" },
  audioNeedsExplicitResume: true,
  rejectForAudioLifecycle: () => false,
  updateAudioControls() {
    stoppedCaptureContext.controlsUpdated = true;
  },
  showStatus(message) {
    stoppedCaptureContext.status = message;
  },
  sampleMemory() {},
  refreshMetrics() {},
};
vm.createContext(stoppedCaptureContext);
vm.runInContext(
  `${source.slice(captureGuardStart, captureGuardEnd)}\nthis.stopCapture = stopCapture;`,
  stoppedCaptureContext,
);
stoppedCaptureContext.stopCapture();
if (
  !stoppedCaptureContext.controlsUpdated ||
  !stoppedCaptureContext.status.includes("Resume audio") ||
  stoppedCaptureContext.captureButton.textContent !== "Restart capture"
) {
  throw new Error("capture stop did not expose required post-stop audio recovery");
}

const resetGuardStart = source.indexOf("function resetReportForm() {");
const resetGuardEnd = source.indexOf("function toggleTools() {", resetGuardStart);
if (resetGuardStart < 0 || resetGuardEnd < 0) {
  throw new Error("cannot locate report-reset audio-operation guard in bootstrap.js");
}
const resetGuardContext = {
  captureActive: false,
  audioProbeInFlight: false,
  audioLifecycleOperations: 1,
  captureButton: { focus() {} },
  rejectForAudioLifecycle() {
    resetGuardContext.status = "Wait for audio operations to finish";
    return true;
  },
  showStatus(message) {
    resetGuardContext.status = message;
  },
};
vm.createContext(resetGuardContext);
vm.runInContext(
  `${source.slice(resetGuardStart, resetGuardEnd)}\nthis.resetReportForm = resetReportForm;`,
  resetGuardContext,
);
resetGuardContext.resetReportForm();
if (!resetGuardContext.status.includes("audio operations")) {
  throw new Error("report reset did not reject an in-flight audio operation");
}

const markEventStart = source.indexOf("function markEvent() {");
const markEventEnd = source.indexOf("function report() {", markEventStart);
if (markEventStart < 0 || markEventEnd < 0) {
  throw new Error("cannot locate event-marker capture guard in bootstrap.js");
}
const markEventContext = {
  captureActive: false,
  audioLifecycleBusy: () => false,
  captureButton: { focus() {} },
  showStatus(message) {
    markEventContext.status = message;
  },
};
vm.createContext(markEventContext);
vm.runInContext(
  `${source.slice(markEventStart, markEventEnd)}\nthis.markEvent = markEvent;`,
  markEventContext,
);
markEventContext.markEvent();
if (!markEventContext.status.includes("Start a capture")) {
  throw new Error("event marker implicitly started or mutated capture evidence");
}
markEventContext.captureActive = true;
markEventContext.telemetry = { events: [] };
markEventContext.eventLabelInput = { value: "bad\nlabel", focus() {} };
markEventContext.document = { visibilityState: "visible" };
markEventContext.window = { screen: { orientation: { type: "landscape-primary" } } };
markEventContext.captureDuration = () => 10;
markEventContext.refreshMetrics = () => {};
markEventContext.markEvent();
if (markEventContext.telemetry.events.length !== 0 || !markEventContext.status.includes("control characters")) {
  throw new Error("event marker accepted a control-character label");
}
markEventContext.eventLabelInput.value = "fixture";
markEventContext.document.visibilityState = "hidden";
markEventContext.markEvent();
if (markEventContext.telemetry.events.length !== 0 || !markEventContext.status.includes("visible capture")) {
  throw new Error("event marker accepted hidden-page evidence");
}

const captureEvidenceStart = source.indexOf("function requireCaptureEvidence() {");
const captureEvidenceEnd = source.indexOf("function clearCaptureEvidence() {", captureEvidenceStart);
if (captureEvidenceStart < 0 || captureEvidenceEnd < 0) {
  throw new Error("cannot locate capture evidence validation in bootstrap.js");
}
const captureEvidenceContext = {
  audioButton: { focus() {} },
  cacheStateInput: { value: "warm" },
  captureButton: { focus() {} },
  canvas: { focus() {} },
  document: { hidden: false },
  telemetry: {
    audio: {
      state: "interrupted",
      transitionFailures: 0,
      audiblePlaybackAttempts: 1,
      mutedPlaybackAttempts: 1,
      gestureStarts: 2,
      muteChanges: 2,
      muted: false,
      backgroundSuspensions: 0,
      explicitResumes: 0,
    },
    frameCount: 1,
    fpsSamples: [60],
    frameGapSamplesMs: [16],
    pointerContacts: 1,
    captureStartedAt: 0,
    captureStoppedAt: 61_000,
    hiddenStartedAt: null,
    hiddenDurationMs: 0,
    hiddenDurationsMs: [],
    restoredFromPageCache: false,
    visibilityChanges: 0,
    orientationChanges: 0,
    pageHideCount: 0,
    pageShowCount: 0,
  },
  performance: { now: () => 61_000 },
  rejectForAudioLifecycle: () => false,
  showStatus(message) {
    captureEvidenceContext.status = message;
  },
};
vm.createContext(captureEvidenceContext);
vm.runInContext(
  `${source.slice(start, end)}\n${source.slice(captureEvidenceStart, captureEvidenceEnd)}\nthis.requireCaptureEvidence = requireCaptureEvidence;`,
  captureEvidenceContext,
);
if (captureEvidenceContext.requireCaptureEvidence()) {
  throw new Error("capture export accepted a non-running audio context");
}
if (!captureEvidenceContext.status.includes("Resume audio")) {
  throw new Error("non-running audio export rejection lacked recovery guidance");
}
captureEvidenceContext.telemetry.audio.state = "running";
captureEvidenceContext.telemetry.audio.backgroundSuspensions = -1;
captureEvidenceContext.telemetry.audio.explicitResumes = -1;
if (captureEvidenceContext.requireCaptureEvidence()) {
  throw new Error("interaction capture accepted negative audio lifecycle counters");
}
if (!captureEvidenceContext.status.includes("exactly zero")) {
  throw new Error("negative interaction audio counters lacked restart guidance");
}
captureEvidenceContext.telemetry.audio.backgroundSuspensions = 0;
captureEvidenceContext.telemetry.audio.explicitResumes = 0;

const captureControlsStart = source.indexOf("function updateCaptureControls() {");
const stoppedCaptureStart = source.indexOf("function stoppedCaptureNeedsAudioResume() {", captureControlsStart);
const captureControlsEnd = source.indexOf("function updateAudioControls() {", stoppedCaptureStart);
if (captureControlsStart < 0 || stoppedCaptureStart < 0 || captureControlsEnd < 0) {
  throw new Error("cannot locate capture control state in bootstrap.js");
}
const captureControlsContext = {
  captureActive: false,
  telemetry: {
    captureStoppedAt: 61_000,
    audio: { transitionFailures: 0 },
  },
  markEventButton: {},
  resetButton: {},
  downloadButton: {},
  toggleToolsButton: {},
  audioLifecycleBusy: () => false,
  audioContext: { state: "interrupted" },
  audioNeedsExplicitResume: true,
};
vm.createContext(captureControlsContext);
vm.runInContext(
  `${source.slice(captureControlsStart, captureControlsEnd)}\nthis.updateCaptureControls = updateCaptureControls;`,
  captureControlsContext,
);
captureControlsContext.updateCaptureControls();
if (!captureControlsContext.downloadButton.disabled) {
  throw new Error("download control enabled before stopped-capture audio recovery");
}
captureControlsContext.audioContext.state = "running";
captureControlsContext.audioNeedsExplicitResume = false;
captureControlsContext.updateCaptureControls();
if (captureControlsContext.downloadButton.disabled) {
  throw new Error("download control remained disabled after stopped-capture audio recovery");
}
captureControlsContext.telemetry.audio.transitionFailures = 1;
captureControlsContext.updateCaptureControls();
if (!captureControlsContext.downloadButton.disabled) {
  throw new Error("download control enabled with a failed audio transition");
}

const lifecycleOrder = [];
audioContext.audioLifecycleTransition = Promise.resolve();
audioContext.telemetry.audio.gestureStarts = 0;
const firstTransition = audioContext.queueAudioLifecycleTransition(async () => {
  lifecycleOrder.push("first-start");
  await Promise.resolve();
  lifecycleOrder.push("first-end");
});
if (audioContext.audioLifecycleOperations !== 1 || !audioContext.audioButton.disabled) {
  throw new Error("queued audio lifecycle transition did not expose its in-flight state");
}
const secondTransition = audioContext.queueAudioLifecycleTransition(async () => {
  lifecycleOrder.push("second");
});
await Promise.all([firstTransition, secondTransition]);
await Promise.resolve();
if (audioContext.audioLifecycleOperations !== 0 || audioContext.audioButton.disabled) {
  throw new Error("completed audio lifecycle transitions retained in-flight state");
}
if (lifecycleOrder.join(",") !== "first-start,first-end,second") {
  throw new Error(`audio lifecycle transitions overlapped: ${lifecycleOrder.join(",")}`);
}

  console.log("browser telemetry self-tests passed");
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
