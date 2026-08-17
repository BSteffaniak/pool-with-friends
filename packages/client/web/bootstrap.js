const shell = document.querySelector("#game-shell");
const canvas = document.querySelector("#pwmtf-canvas");
const loading = document.querySelector("#loading");
const loadError = document.querySelector("#load-error");
const reload = loadError.querySelector("button");
const tools = document.querySelector("#feasibility-tools");
const metricsOutput = document.querySelector("#feasibility-metrics");
const captureButton = document.querySelector("#capture-toggle");
const audioButton = document.querySelector("#audio-probe");
const muteButton = document.querySelector("#audio-mute");
const downloadButton = document.querySelector("#download-report");
const feasibilityEnabled = new URLSearchParams(window.location.search).has("feasibility");
const navigationStartedAt = performance.now();

const telemetry = {
  schemaVersion: 1,
  clientReadyMs: null,
  firstCanvasContactMs: null,
  pointerContacts: 0,
  captureStartedAt: null,
  captureStoppedAt: null,
  frameCount: 0,
  minimumFps: null,
  maximumFrameGapMs: 0,
  currentFps: null,
  currentJsHeapBytes: null,
  peakJsHeapBytes: null,
  visibilityChanges: 0,
  orientationChanges: 0,
  audio: {
    supported: Boolean(window.AudioContext || window.webkitAudioContext),
    state: "not started",
    gestureStarts: 0,
    backgroundSuspensions: 0,
    explicitResumes: 0,
    muted: false,
  },
};

let captureActive = false;
let lastFrameAt = null;
let frameWindowStartedAt = null;
let frameWindowCount = 0;
let audioContext = null;
let masterGain = null;
let audioNeedsExplicitResume = false;

function formatDuration(milliseconds) {
  if (milliseconds === null) {
    return "pending";
  }
  return `${(milliseconds / 1000).toFixed(2)} s`;
}

function formatBytes(bytes) {
  if (bytes === null) {
    return "unavailable";
  }
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

function captureDuration() {
  if (telemetry.captureStartedAt === null) {
    return null;
  }
  return (telemetry.captureStoppedAt ?? performance.now()) - telemetry.captureStartedAt;
}

function refreshMetrics() {
  if (!feasibilityEnabled) {
    return;
  }

  const duration = captureDuration();
  metricsOutput.textContent = [
    `client ready: ${formatDuration(telemetry.clientReadyMs)}`,
    `first canvas contact: ${formatDuration(telemetry.firstCanvasContactMs)}`,
    `capture: ${captureActive ? "running" : "stopped"} (${formatDuration(duration)})`,
    `frame rate: ${telemetry.currentFps === null ? "pending" : `${telemetry.currentFps.toFixed(1)} FPS`}`,
    `minimum 1 s frame rate: ${telemetry.minimumFps === null ? "pending" : `${telemetry.minimumFps.toFixed(1)} FPS`}`,
    `largest frame gap: ${telemetry.maximumFrameGapMs.toFixed(1)} ms`,
    `JS heap: ${formatBytes(telemetry.currentJsHeapBytes)} (peak ${formatBytes(telemetry.peakJsHeapBytes)})`,
    `canvas contacts: ${telemetry.pointerContacts}`,
    `visibility/orientation changes: ${telemetry.visibilityChanges}/${telemetry.orientationChanges}`,
    `audio: ${telemetry.audio.state}${telemetry.audio.muted ? " (muted)" : ""}`,
    `viewport: ${window.innerWidth}×${window.innerHeight} @ ${window.devicePixelRatio.toFixed(2)}x`,
  ].join("\n");
}

function sampleMemory() {
  const memory = performance.memory;
  if (!memory || typeof memory.usedJSHeapSize !== "number") {
    return;
  }
  telemetry.currentJsHeapBytes = memory.usedJSHeapSize;
  telemetry.peakJsHeapBytes = Math.max(telemetry.peakJsHeapBytes ?? 0, memory.usedJSHeapSize);
}

function frame(timestamp) {
  if (captureActive) {
    telemetry.frameCount += 1;
    frameWindowCount += 1;

    if (lastFrameAt !== null) {
      telemetry.maximumFrameGapMs = Math.max(telemetry.maximumFrameGapMs, timestamp - lastFrameAt);
    }
    lastFrameAt = timestamp;
    frameWindowStartedAt ??= timestamp;

    const windowDuration = timestamp - frameWindowStartedAt;
    if (windowDuration >= 1000) {
      const fps = (frameWindowCount * 1000) / windowDuration;
      telemetry.currentFps = fps;
      telemetry.minimumFps = Math.min(telemetry.minimumFps ?? fps, fps);
      frameWindowStartedAt = timestamp;
      frameWindowCount = 0;
      sampleMemory();
      refreshMetrics();
    }
  }

  window.requestAnimationFrame(frame);
}

function startCapture() {
  telemetry.captureStartedAt = performance.now();
  telemetry.captureStoppedAt = null;
  telemetry.frameCount = 0;
  telemetry.minimumFps = null;
  telemetry.maximumFrameGapMs = 0;
  telemetry.currentFps = null;
  telemetry.currentJsHeapBytes = null;
  telemetry.peakJsHeapBytes = null;
  telemetry.pointerContacts = 0;
  telemetry.visibilityChanges = 0;
  telemetry.orientationChanges = 0;
  lastFrameAt = null;
  frameWindowStartedAt = null;
  frameWindowCount = 0;
  captureActive = true;
  captureButton.textContent = "Stop capture";
  refreshMetrics();
}

function stopCapture() {
  telemetry.captureStoppedAt = performance.now();
  captureActive = false;
  captureButton.textContent = "Restart capture";
  sampleMemory();
  refreshMetrics();
}

function updateAudioControls() {
  telemetry.audio.state = audioContext?.state ?? (telemetry.audio.supported ? "not started" : "unsupported");
  audioButton.textContent = audioNeedsExplicitResume ? "Resume audio probe" : "Play audio probe";
  audioButton.disabled = !telemetry.audio.supported;
  muteButton.disabled = audioContext === null;
  muteButton.textContent = telemetry.audio.muted ? "Unmute" : "Mute";
  refreshMetrics();
}

function createAudioGraph() {
  const AudioContext = window.AudioContext || window.webkitAudioContext;
  audioContext = new AudioContext();
  masterGain = audioContext.createGain();
  masterGain.gain.value = 0.12;
  masterGain.connect(audioContext.destination);
  audioContext.addEventListener("statechange", updateAudioControls);
}

async function playAudioProbe() {
  if (!telemetry.audio.supported) {
    return;
  }

  if (audioContext === null) {
    createAudioGraph();
  }

  if (audioContext.state === "suspended") {
    await audioContext.resume();
    if (audioNeedsExplicitResume) {
      telemetry.audio.explicitResumes += 1;
    }
  }

  audioNeedsExplicitResume = false;
  telemetry.audio.gestureStarts += 1;

  const oscillator = audioContext.createOscillator();
  const envelope = audioContext.createGain();
  const startedAt = audioContext.currentTime;
  oscillator.type = "triangle";
  oscillator.frequency.setValueAtTime(523.25, startedAt);
  oscillator.frequency.exponentialRampToValueAtTime(659.25, startedAt + 0.16);
  envelope.gain.setValueAtTime(0.0001, startedAt);
  envelope.gain.exponentialRampToValueAtTime(1, startedAt + 0.015);
  envelope.gain.exponentialRampToValueAtTime(0.0001, startedAt + 0.24);
  oscillator.connect(envelope);
  envelope.connect(masterGain);
  oscillator.start(startedAt);
  oscillator.stop(startedAt + 0.25);
  oscillator.addEventListener("ended", () => {
    oscillator.disconnect();
    envelope.disconnect();
  });
  updateAudioControls();
}

function toggleMute() {
  if (masterGain === null || audioContext === null) {
    return;
  }

  telemetry.audio.muted = !telemetry.audio.muted;
  masterGain.gain.setValueAtTime(telemetry.audio.muted ? 0 : 0.12, audioContext.currentTime);
  updateAudioControls();
}

function report() {
  sampleMemory();
  return {
    schema_version: telemetry.schemaVersion,
    captured_at: new Date().toISOString(),
    candidate: {
      bevy: "0.19.1",
      renderer: "WebGL2",
    },
    browser: {
      user_agent: navigator.userAgent,
      language: navigator.language,
      hardware_concurrency: navigator.hardwareConcurrency ?? null,
      device_memory_gib: navigator.deviceMemory ?? null,
    },
    display: {
      screen_width: window.screen.width,
      screen_height: window.screen.height,
      viewport_width: window.innerWidth,
      viewport_height: window.innerHeight,
      device_pixel_ratio: window.devicePixelRatio,
      orientation: window.screen.orientation?.type ?? null,
    },
    timing_ms: {
      client_ready: telemetry.clientReadyMs,
      first_canvas_contact: telemetry.firstCanvasContactMs,
      capture_duration: captureDuration(),
    },
    performance: {
      frame_count: telemetry.frameCount,
      current_fps: telemetry.currentFps,
      minimum_one_second_fps: telemetry.minimumFps,
      maximum_frame_gap_ms: telemetry.maximumFrameGapMs,
      current_js_heap_bytes: telemetry.currentJsHeapBytes,
      peak_js_heap_bytes: telemetry.peakJsHeapBytes,
    },
    interaction: {
      canvas_contacts: telemetry.pointerContacts,
      visibility_changes: telemetry.visibilityChanges,
      orientation_changes: telemetry.orientationChanges,
    },
    audio: telemetry.audio,
  };
}

function downloadReport() {
  const contents = JSON.stringify(report(), null, 2);
  const url = URL.createObjectURL(new Blob([contents], { type: "application/json" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = `pwmtf-feasibility-${new Date().toISOString().replaceAll(":", "-")}.json`;
  link.click();
  URL.revokeObjectURL(url);
}

reload.addEventListener("click", () => window.location.reload());
captureButton.addEventListener("click", () => (captureActive ? stopCapture() : startCapture()));
audioButton.addEventListener("click", () => void playAudioProbe());
muteButton.addEventListener("click", toggleMute);
downloadButton.addEventListener("click", downloadReport);
canvas.addEventListener("pointerdown", () => {
  telemetry.pointerContacts += 1;
  telemetry.firstCanvasContactMs ??= performance.now() - navigationStartedAt;
  refreshMetrics();
});
window.addEventListener("orientationchange", () => {
  telemetry.orientationChanges += 1;
  refreshMetrics();
});
window.addEventListener("resize", refreshMetrics);
document.addEventListener("visibilitychange", () => {
  telemetry.visibilityChanges += 1;
  if (document.hidden && audioContext?.state === "running") {
    audioNeedsExplicitResume = true;
    void audioContext.suspend().then(() => {
      telemetry.audio.backgroundSuspensions += 1;
      updateAudioControls();
    });
  }
  refreshMetrics();
});

if (feasibilityEnabled) {
  tools.hidden = false;
  window.requestAnimationFrame(frame);
  updateAudioControls();
  refreshMetrics();
}

try {
  if (!document.createElement("canvas").getContext("webgl2")) {
    throw new Error("WebGL 2 is unavailable");
  }
  const { default: init } = await import("./pwmtf_client.js");
  await init();
  await new Promise((resolve) => window.requestAnimationFrame(resolve));
  await new Promise((resolve) => window.requestAnimationFrame(resolve));
  telemetry.clientReadyMs = performance.now() - navigationStartedAt;
  shell.dataset.clientState = "ready";
  loading.remove();
  canvas.focus({ preventScroll: true });
  refreshMetrics();
} catch (error) {
  console.error("PWMTF client startup failed", error);
  shell.dataset.clientState = "failed";
  loading.remove();
  loadError.hidden = false;
  refreshMetrics();
}
