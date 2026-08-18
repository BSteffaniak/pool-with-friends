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
vm.runInContext(`${source.slice(metadataStart, metadataEnd)}\nthis.requireTestMetadata = requireTestMetadata;`, metadataContext);

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
metadataContext.detectedBrowserFamily = "unknown";
if (metadataContext.requireTestMetadata()) {
  throw new Error("unknown detected browser was accepted");
}

console.log("browser telemetry self-tests passed");
