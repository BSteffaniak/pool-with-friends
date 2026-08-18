const shell = document.querySelector("#game-shell");
const canvas = document.querySelector("#pwmtf-canvas");
const loading = document.querySelector("#loading");
const loadError = document.querySelector("#load-error");
const reload = loadError.querySelector("button");
const tools = document.querySelector("#feasibility-tools");
const platformInput = document.querySelector("#test-platform");
const hardwareModelInput = document.querySelector("#hardware-model");
const osVersionInput = document.querySelector("#os-version");
const browserFamilyInput = document.querySelector("#browser-family");
const browserVersionInput = document.querySelector("#browser-version");
const cacheStateInput = document.querySelector("#cache-state");
const minimumVersionInput = document.querySelector("#minimum-version-run");
const presentationTierInput = document.querySelector("#presentation-tier");
const runNumberInput = document.querySelector("#run-number");
const physicalChecks = document.querySelector("#physical-checks");
const firstVisibleInput = document.querySelector("#first-visible-ms");
const firstInputInput = document.querySelector("#first-input-ms");
const steadyMemoryInput = document.querySelector("#steady-memory-mib");
const peakMemoryInput = document.querySelector("#peak-memory-mib");
const thermalResultInput = document.querySelector("#thermal-result");
const reloadObservedInput = document.querySelector("#reload-observed");
const metricsOutput = document.querySelector("#feasibility-metrics");
const candidateOutput = document.querySelector("#candidate-identity");
const captureButton = document.querySelector("#capture-toggle");
const audioButton = document.querySelector("#audio-probe");
const muteButton = document.querySelector("#audio-mute");
const markEventButton = document.querySelector("#mark-event");
const eventLabelInput = document.querySelector("#event-label");
const downloadButton = document.querySelector("#download-report");
const statusOutput = document.querySelector("#feasibility-status");
const query = new URLSearchParams(window.location.search);
const feasibilityEnabled = query.has("feasibility");
const activePresentationTier = query.get("tier") === "reduced" ? "reduced" : "default";
const candidateBuildId = "__PWMTF_BUILD_ID__";
const candidateSourceHash = "__PWMTF_SOURCE_HASH__";
const navigationStartedAt = performance.now();
const PHYSICAL_CHECKS = [
  ["first_load", "First/warm load reaches the table"],
  ["aiming", "Held-contact aiming is continuous"],
  ["power", "Power drag is bounded"],
  ["touch_reset", "New touch has no stale state"],
  ["resize", "Landscape resize preserves the table"],
  ["safe_area", "Chrome and safe areas do not hide controls"],
  ["orientation", "Portrait notice and landscape restore work"],
  ["background", "Background/foreground restores render and input"],
  ["audio", "Gesture, mute, suspend, and explicit resume work"],
  ["input_response", "No visible delayed aiming"],
  ["memory", "No reload/eviction; memory reaches a plateau"],
  ["thermal", "No severe throttling or thermal warning"],
];

const telemetry = {
  schemaVersion: 1,
  clientReadyMs: null,
  firstCanvasContactMs: null,
  pointerContacts: 0,
  captureStartedAt: null,
  captureStoppedAt: null,
  frameCount: 0,
  minimumFps: null,
  maximumFps: null,
  fpsSamples: [],
  frameGapSamplesMs: [],
  maximumFrameGapMs: 0,
  currentFps: null,
  currentJsHeapBytes: null,
  peakJsHeapBytes: null,
  visibilityChanges: 0,
  orientationChanges: 0,
  pageHideCount: 0,
  pageShowCount: 0,
  restoredFromPageCache: false,
  events: [],
  audio: {
    supported: Boolean(window.AudioContext || window.webkitAudioContext),
    state: "not started",
    gestureStarts: 0,
    backgroundSuspensions: 0,
    explicitResumes: 0,
    muteChanges: 0,
    mutedPlaybackAttempts: 0,
    audiblePlaybackAttempts: 0,
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

function detectBrowserFamily(userAgent) {
  if (/SamsungBrowser\//u.test(userAgent)) {
    return "samsung-internet";
  }
  if (/(?:Edg|EdgA|EdgiOS)\//u.test(userAgent)) {
    return "edge";
  }
  if (/(?:Firefox|FxiOS)\//u.test(userAgent)) {
    return "firefox";
  }
  if (/(?:Chrome|CriOS)\//u.test(userAgent)) {
    return "chrome";
  }
  if (/Version\/.+Safari\//u.test(userAgent)) {
    return "safari";
  }
  return "unknown";
}

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

function percentile(samples, percentage) {
  if (samples.length === 0) {
    return null;
  }
  const sorted = [...samples].sort((left, right) => left - right);
  const index = Math.ceil((percentage / 100) * sorted.length) - 1;
  return sorted[Math.max(0, index)];
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
    `build: ${candidateBuildId}`,
    `tier: ${activePresentationTier}`,
    `client ready: ${formatDuration(telemetry.clientReadyMs)}`,
    `first canvas contact: ${formatDuration(telemetry.firstCanvasContactMs)}`,
    `capture: ${captureActive ? "running" : "stopped"} (${formatDuration(duration)})`,
    `frame rate: ${telemetry.currentFps === null ? "pending" : `${telemetry.currentFps.toFixed(1)} FPS`}`,
    `minimum/median FPS: ${telemetry.minimumFps === null ? "pending" : `${telemetry.minimumFps.toFixed(1)} / ${percentile(telemetry.fpsSamples, 50).toFixed(1)}`}`,
    `p95/worst frame time: ${telemetry.frameGapSamplesMs.length === 0 ? "pending" : `${percentile(telemetry.frameGapSamplesMs, 95).toFixed(1)} / ${telemetry.maximumFrameGapMs.toFixed(1)} ms`}`,
    `JS heap: ${formatBytes(telemetry.currentJsHeapBytes)} (peak ${formatBytes(telemetry.peakJsHeapBytes)})`,
    `canvas contacts/events: ${telemetry.pointerContacts}/${telemetry.events.length}`,
    `visibility/orientation changes: ${telemetry.visibilityChanges}/${telemetry.orientationChanges}`,
    `page hide/show: ${telemetry.pageHideCount}/${telemetry.pageShowCount}`,
    `restored from page cache: ${telemetry.restoredFromPageCache ? "yes" : "no"}`,
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
      const frameGap = timestamp - lastFrameAt;
      telemetry.frameGapSamplesMs.push(frameGap);
      telemetry.maximumFrameGapMs = Math.max(telemetry.maximumFrameGapMs, frameGap);
    }
    lastFrameAt = timestamp;
    frameWindowStartedAt ??= timestamp;

    const windowDuration = timestamp - frameWindowStartedAt;
    if (windowDuration >= 1000) {
      const fps = (frameWindowCount * 1000) / windowDuration;
      telemetry.currentFps = fps;
      telemetry.fpsSamples.push(fps);
      telemetry.minimumFps = Math.min(telemetry.minimumFps ?? fps, fps);
      telemetry.maximumFps = Math.max(telemetry.maximumFps ?? fps, fps);
      frameWindowStartedAt = timestamp;
      frameWindowCount = 0;
      sampleMemory();
      refreshMetrics();
    }
  }

  window.requestAnimationFrame(frame);
}

function externalObservations() {
  const numericValue = (input) => {
    const value = Number.parseFloat(input.value);
    return Number.isFinite(value) ? value : null;
  };
  return {
    first_visible_table_ms: numericValue(firstVisibleInput),
    first_accepted_input_ms: numericValue(firstInputInput),
    steady_memory_mib: numericValue(steadyMemoryInput),
    peak_memory_mib: numericValue(peakMemoryInput),
    thermal_result: thermalResultInput.value || null,
    reload_or_eviction_observed: reloadObservedInput.value || null,
  };
}

function requireExternalObservations() {
  const inputs = [
    firstVisibleInput,
    firstInputInput,
    steadyMemoryInput,
    peakMemoryInput,
    thermalResultInput,
    reloadObservedInput,
  ];
  for (const input of inputs) {
    if (!input.reportValidity()) {
      showStatus(`Complete the required observation: ${input.labels?.[0]?.textContent.trim() ?? input.id}`);
      return false;
    }
  }
  const observations = externalObservations();
  if (observations.first_accepted_input_ms < observations.first_visible_table_ms) {
    showStatus("First accepted input cannot precede the first visible table.");
    firstInputInput.focus();
    return false;
  }
  if (observations.peak_memory_mib < observations.steady_memory_mib) {
    showStatus("Peak memory cannot be lower than steady memory.");
    peakMemoryInput.focus();
    return false;
  }
  return true;
}

function physicalCheckResults() {
  return Object.fromEntries(
    PHYSICAL_CHECKS.map(([id]) => [
      id,
      document.querySelector(`input[name="check-${id}"]:checked`)?.value ?? null,
    ]),
  );
}

function showStatus(message) {
  statusOutput.textContent = message;
}

function requirePhysicalChecks() {
  const results = physicalCheckResults();
  const missing = PHYSICAL_CHECKS.find(([id]) => results[id] === null);
  if (missing === undefined) {
    return true;
  }
  showStatus(`Missing physical check: ${missing[1]}`);
  document.querySelector(`input[name="check-${missing[0]}"]`)?.focus();
  return false;
}

function testMetadata() {
  return {
    platform: platformInput.value,
    hardware_model: hardwareModelInput.value.trim(),
    os_version: osVersionInput.value.trim(),
    browser_family: browserFamilyInput.value,
    browser_version: browserVersionInput.value.trim(),
    cache_state: cacheStateInput.value,
    minimum_version_run: minimumVersionInput.value,
    presentation_tier: presentationTierInput.value,
    run_number: Number.parseInt(runNumberInput.value, 10) || null,
  };
}

function requireTestMetadata() {
  const inputs = [
    platformInput,
    hardwareModelInput,
    osVersionInput,
    browserFamilyInput,
    browserVersionInput,
    cacheStateInput,
    minimumVersionInput,
    presentationTierInput,
    runNumberInput,
  ];
  for (const input of inputs) {
    if (!input.reportValidity()) {
      showStatus(`Complete the required test metadata: ${input.labels?.[0]?.textContent.trim() ?? input.id}`);
      return false;
    }
  }
  if (platformInput.value === "desktop" && presentationTierInput.value !== "default") {
    showStatus("Desktop compatibility captures must use the default presentation tier.");
    return false;
  }
  return true;
}

function startCapture() {
  showStatus("");
  if (!requireTestMetadata()) {
    return;
  }
  telemetry.captureStartedAt = performance.now();
  telemetry.captureStoppedAt = null;
  telemetry.frameCount = 0;
  telemetry.minimumFps = null;
  telemetry.maximumFps = null;
  telemetry.fpsSamples = [];
  telemetry.frameGapSamplesMs = [];
  telemetry.maximumFrameGapMs = 0;
  telemetry.currentFps = null;
  telemetry.currentJsHeapBytes = null;
  telemetry.peakJsHeapBytes = null;
  telemetry.pointerContacts = 0;
  telemetry.visibilityChanges = 0;
  telemetry.orientationChanges = 0;
  telemetry.events = [];
  lastFrameAt = null;
  frameWindowStartedAt = null;
  frameWindowCount = 0;
  captureActive = true;
  captureButton.textContent = "Stop capture";
  showStatus("Capture started.");
  refreshMetrics();
}

function stopCapture() {
  telemetry.captureStoppedAt = performance.now();
  captureActive = false;
  captureButton.textContent = "Restart capture";
  showStatus("Capture stopped and ready for validation.");
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
  if (telemetry.audio.muted) {
    telemetry.audio.mutedPlaybackAttempts += 1;
  } else {
    telemetry.audio.audiblePlaybackAttempts += 1;
  }

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
  telemetry.audio.muteChanges += 1;
  masterGain.gain.setValueAtTime(telemetry.audio.muted ? 0 : 0.12, audioContext.currentTime);
  updateAudioControls();
}

function markEvent() {
  if (telemetry.captureStartedAt === null) {
    startCapture();
    if (telemetry.captureStartedAt === null) {
      return;
    }
  }
  const label = eventLabelInput.value.trim();
  if (label === "") {
    showStatus("Enter a short event label before marking the event.");
    eventLabelInput.focus();
    return;
  }
  telemetry.events.push({
    elapsed_ms: captureDuration(),
    label: label.slice(0, 120),
    visibility: document.visibilityState,
    orientation: window.screen.orientation?.type ?? null,
  });
  eventLabelInput.value = "";
  showStatus(`Marked event: ${label.slice(0, 120)}`);
  refreshMetrics();
}

function report() {
  sampleMemory();
  return {
    schema_version: telemetry.schemaVersion,
    captured_at: new Date().toISOString(),
    candidate: {
      build_id: candidateBuildId,
      source_hash: candidateSourceHash,
      bevy: "0.19.1",
      renderer: "WebGL2",
    },
    test: testMetadata(),
    physical_checks: physicalCheckResults(),
    external_observations: externalObservations(),
    browser: {
      declared_family: browserFamilyInput.value,
      detected_family: detectBrowserFamily(navigator.userAgent),
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
      median_one_second_fps: percentile(telemetry.fpsSamples, 50),
      maximum_one_second_fps: telemetry.maximumFps,
      median_frame_time_ms: percentile(telemetry.frameGapSamplesMs, 50),
      p95_frame_time_ms: percentile(telemetry.frameGapSamplesMs, 95),
      p99_frame_time_ms: percentile(telemetry.frameGapSamplesMs, 99),
      maximum_frame_gap_ms: telemetry.maximumFrameGapMs,
      current_js_heap_bytes: telemetry.currentJsHeapBytes,
      peak_js_heap_bytes: telemetry.peakJsHeapBytes,
    },
    interaction: {
      canvas_contacts: telemetry.pointerContacts,
      visibility_changes: telemetry.visibilityChanges,
      orientation_changes: telemetry.orientationChanges,
      page_hide_count: telemetry.pageHideCount,
      page_show_count: telemetry.pageShowCount,
      restored_from_page_cache: telemetry.restoredFromPageCache,
      marked_events: telemetry.events,
    },
    audio: telemetry.audio,
  };
}

function requireStoppedCapture() {
  if (telemetry.captureStartedAt !== null && telemetry.captureStoppedAt !== null && !captureActive) {
    return true;
  }
  showStatus("Start and stop the capture before downloading its report.");
  captureButton.focus();
  return false;
}

function downloadReport() {
  showStatus("");
  if (
    !requireTestMetadata() ||
    !requirePhysicalChecks() ||
    !requireExternalObservations() ||
    !requireStoppedCapture()
  ) {
    return;
  }
  const contents = JSON.stringify(report(), null, 2);
  const url = URL.createObjectURL(new Blob([contents], { type: "application/json" }));
  const link = document.createElement("a");
  link.href = url;
  const test = testMetadata();
  const platform = test.platform || "unknown-device";
  const browser = test.browser_family || "unknown-browser";
  const build = candidateBuildId.slice(0, 20);
  const run = test.run_number === null ? "unknown-run" : `run-${test.run_number}`;
  link.download = `pwmtf-feasibility-${build}-${platform}-${browser}-${test.cache_state || "unknown-cache"}-${test.minimum_version_run === "yes" ? "minimum" : "current"}-${test.presentation_tier || "unknown-tier"}-${run}-${new Date().toISOString().replaceAll(":", "-")}.json`;
  link.click();
  URL.revokeObjectURL(url);
  showStatus(`Downloaded ${link.download}`);
}

reload.addEventListener("click", () => window.location.reload());
captureButton.addEventListener("click", () => (captureActive ? stopCapture() : startCapture()));
audioButton.addEventListener("click", () => void playAudioProbe());
muteButton.addEventListener("click", toggleMute);
markEventButton.addEventListener("click", markEvent);
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
window.addEventListener("pagehide", () => {
  telemetry.pageHideCount += 1;
  refreshMetrics();
});
window.addEventListener("pageshow", (event) => {
  telemetry.pageShowCount += 1;
  if (event.persisted) {
    telemetry.restoredFromPageCache = true;
    telemetry.events.push({
      elapsed_ms: captureDuration(),
      label: "restored from page cache",
      visibility: document.visibilityState,
      orientation: window.screen.orientation?.type ?? null,
    });
  }
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
  candidateOutput.textContent = `Build ${candidateBuildId}\nSource ${candidateSourceHash}`;
  presentationTierInput.value = activePresentationTier;
  presentationTierInput.disabled = true;
  for (const [id, label] of PHYSICAL_CHECKS) {
    const row = document.createElement("div");
    row.className = "physical-check";
    const description = document.createElement("span");
    description.textContent = label;
    row.append(description);
    for (const value of ["pass", "fail"]) {
      const option = document.createElement("label");
      const input = document.createElement("input");
      input.type = "radio";
      input.name = `check-${id}`;
      input.value = value;
      option.append(input, value === "pass" ? "Pass" : "Fail");
      row.append(option);
    }
    physicalChecks.append(row);
  }
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
