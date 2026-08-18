# Mobile browser feasibility record

This record defines the evidence required before PWMTF accepts Bevy as its production browser client. A successful build or desktop smoke test is not acceptance.

## Candidate configuration

- Bevy: `0.19.1`
- Renderer: WebGL2
- Enabled workspace features: `2d`, `ui`, `web`, `webgl2`
- Release profile: optimized, symbols stripped; Binaryen `wasm-opt -Oz` when available
- Browser shell: viewport safe areas, portrait overlay, startup progress, explicit WebGL2/load failure state, and an opt-in feasibility capture panel

Open the built client with `?feasibility` to expose the capture panel. The normal product URL does not show it. Run `./scripts/report-wasm-size.sh` for the exact candidate; its output includes the same build ID and source hash injected into reports so bundle measurements cannot be attributed to a different build. Before capture, the tester must identify the platform, hardware model, exact OS/browser family and version, whether this is a proposed-minimum-version run, cache state, presentation tier, and run number; the build/release ID and exact source hash are injected by the build and these fields are embedded in the report and its filename. Before export, every required observable physical check must be recorded as Pass or Fail, and platform-tool measurements for first-visible table, first accepted input, steady/peak memory, thermal status, and reload/eviction must be entered. The panel will not export until the tester explicitly starts and stops a non-zero capture. The panel also records browser/display metadata, including both operator-declared and user-agent-detected browser family, startup and first-contact approximations, one-second frame-rate windows, median/95th/99th-percentile/worst animation-frame intervals, interaction/lifecycle counters (including page hide/show and page-cache restoration), operator-marked events, and JavaScript heap data where the browser exposes `performance.memory`. It exports a versioned JSON report for attachment to this record. Use **Mark event** for short non-sensitive annotations such as orientation changes, background/foreground boundaries, audio checks, visible throttling, or thermal warnings; never enter identity, credentials, tokens, or unrelated browser data.

The panel also provides a generated Web Audio probe. It creates the audio context only from the probe button's user gesture, supports mute/unmute, deliberately suspends a running probe when the document becomes hidden, and requires a subsequent explicit button gesture to resume. The generated tone is measurement instrumentation, not shipped game audio.

The prototype deliberately contains presentation and input only. It does not establish canonical physics, gameplay authority, networking, or production asset choices.

## Required device matrix

Record the exact OS, browser version, hardware model, date, and result for each row. Test the oldest version PWMTF intends to support and the current stable version.

| Platform | Required browser | Device/version tested | Result |
| --- | --- | --- | --- |
| iPhone | Safari | Pending | Pending |
| iPad | Safari | Pending | Pending |
| Android phone | Chrome | Pending | Pending |
| Android tablet | Chrome | Pending | Pending |
| Desktop compatibility | Chrome | Google Chrome 151.0.7922.138; macOS 26.6.1; MacBook Air (M4); 2026-08-16 | Pass — automated WebGL2/SwiftShader loads at 1280×720 reached the ready canvas on normal, default-feasibility, and reduced-feasibility entry points; required capture controls and tier reporting were present |
| Desktop compatibility | Safari | Pending | Pending |
| Desktop compatibility | Firefox | Firefox 152.0.4; macOS 26.6.1; MacBook Air (M4); 2026-08-17 | Pass — automated WebDriver loads reached the ready canvas on normal, default-feasibility, and reduced-feasibility entry points; required capture controls and reduced-tier reporting were present |
| Desktop compatibility | Edge | Pending | Pending |

Final-gate mode requires the complete current/minimum browser matrix for compatibility, but the mobile startup/FPS/fallback budgets apply only to iPhone, iPad, Android phone, and Android tablet evidence; desktop rows must still provide complete captures, passing checks, frame metrics, lifecycle/audio evidence, and no thermal warning or reload/eviction.

The desktop environment currently has no connected iOS or Android hardware, so those rows cannot be honestly completed from automation here. Firefox 152.0.4 with geckodriver 0.37.1 passes `./scripts/test-firefox-smoke.sh`; this desktop compatibility result does not establish Firefox as a supported mobile browser. Safari 26.6 and its WebDriver are installed, but Safari rejects WebDriver sessions until the user enables **Allow remote automation** in Safari's Developer settings. After enabling it, run `./scripts/test-safari-smoke.sh` to exercise normal, default-feasibility, and reduced-feasibility entry points plus capture controls and tier reporting in desktop Safari. The Safari row remains pending until that command succeeds; enabling a browser security setting automatically is intentionally not part of the script, and a driver session alone would not substitute for the required physical interaction and visual checks.

## Interaction and lifecycle checks

Run every check on each physical mobile class:

- [ ] First load and warm-cache load both reach the table without a blank canvas.
- [ ] Aiming starts only while the pointer or one stable touch contact is held.
- [ ] Dragging in the table area rotates the cue continuously without jumps.
- [ ] Dragging in the right-side power area changes power and remains bounded.
- [ ] Releasing and beginning a new touch does not inherit stale contact state.
- [ ] Resizing preserves the full table at narrow and wide landscape ratios.
- [ ] Browser chrome expansion/collapse does not hide controls.
- [ ] Notches, rounded corners, and home indicators do not cover the canvas content.
- [ ] Portrait orientation covers the game with the rotate notice.
- [ ] Returning to landscape restores the table without a reload.
- [ ] Backgrounding and foregrounding restores rendering and input.
- [ ] The audio probe starts only from its button gesture, mute/unmute is immediate, hiding the page suspends a running context, and returning requires the explicit resume button.

The current prototype has no shipped audio content. Its opt-in generated-tone probe makes audio lifecycle testing possible without adding a production audio asset. Audio lifecycle remains open until the behavior above is measured on iOS Safari and Android Chrome.

## Measurement procedure

Use an uncached production build served over HTTPS or a representative throttled local connection. Capture at least three runs per device and report the median plus the worst observed run.

1. Run `./scripts/report-wasm-size.sh` and copy the raw and gzip totals into this record. The script also reports Brotli sizes when the `brotli` executable is available.
2. Build and serve `dist/` over HTTPS, then open `/?feasibility` for the default tier or `/?feasibility&tier=reduced` for a required fallback measurement. Enter the exact device/browser metadata and run number, then start a fresh capture after the table is visible. The tier is URL-bound, displayed by the Rust client, and exported automatically rather than accepted as a tester assertion. `./scripts/serve-feasibility.sh` provides a local-network HTTPS server when given a device-trusted certificate; its exact environment variables are documented in `packages/client/README.md`. A self-signed certificate that the device does not trust is not representative evidence.
3. Record navigation start to first visible table and navigation start to first accepted input. The exported report supplies client-ready and first-contact approximations; use browser tooling for the final visible-paint measurement.
4. Record steady-state memory after 60 seconds and peak memory during load using platform tooling. Treat the JSON JavaScript heap figures as supporting evidence only because Safari and Firefox may not expose them and they exclude WASM/graphics memory.
5. Capture one minute of repeated aim and power interaction and report median, 95th-percentile, 99th-percentile, and worst frame time. The panel exports those animation-frame interval summaries plus one-second FPS windows as supporting evidence, not a replacement for browser/platform traces.
6. Keep the app active for 10 minutes, including repeated orientation changes and at least two 30-second background/foreground cycles.
7. Exercise the audio probe before and after mute, background the page while its context is running, then explicitly resume from the button after foregrounding.
8. Record every physical check in the panel as Pass or Fail. Enter first-visible/accepted-input timings and steady/peak memory from browser/platform tooling, plus thermal and reload/eviction results. Use **Mark event** at lifecycle and observable thermal boundaries, then download the JSON report and note any graphics reset, audio failure, or input loss. Keep labels short and free of identity or secret data.
9. After at least three runs for each comparable device/browser/cache-state group, validate and summarize the downloaded files with `./scripts/summarize-feasibility.py path/to/*.json`. It rejects malformed reports, missing or unknown required checks, duplicate run numbers, and groups with fewer than three runs, then prints a Markdown median/worst table for this record. For final mobile-gate evidence, add `--require-mobile-matrix`; it also rejects missing platform/cache groups or proposed-minimum-version evidence for any mobile class, failed physical checks, budget failures, captures shorter than the required one or ten minutes, unavailable FPS/frame-time summaries, FPS below 30, an unproven 30–55 FPS fallback, thermal/reload failures, missing background/orientation cycles, and incomplete pre/post-mute or suspend/resume audio evidence. `./scripts/test-feasibility-tools.sh` self-tests a complete synthetic matrix and representative matrix/minimum-version, FPS, startup-budget, interaction/lifecycle-duration, and audio-lifecycle rejection paths. Use `--allow-incomplete` only for interim troubleshooting; it cannot be combined with final-gate mode.
10. Repeat on the quality fallback if the default tier cannot hold the target frame rate.
11. Save screenshots or traces with this record's date/device labels; do not store user credentials or unrelated browser data. Downloaded `pwmtf-feasibility-*.json`, `*.trace`, and `*.har` evidence is ignored by Git by default and should remain local unless deliberately sanitized and approved for publication.

| Metric | Budget / acceptance rule | Measured result |
| --- | --- | --- |
| WASM compressed transfer | Record first; optimize or reject if it prevents reliable startup | 26,973,574 bytes raw / 8,108,520 bytes gzip (`wasm-opt -Oz`, 2026-08-16 local release build) |
| Complete generated bundle | Record all generated assets | 27,116,773 bytes raw / 8,132,636 bytes gzip (2026-08-16 local release build; gzip members summed) |
| Cold first-visible table | <= 8 s on representative mobile broadband | Pending |
| Warm first-visible table | <= 3 s | Pending |
| Steady aiming frame rate | 60 FPS target; >= 30 FPS fallback floor | Pending |
| Input response | No visible frame-delayed aiming under steady load | Pending |
| Memory | No reload/eviction and a stable plateau during the 10-minute run | Pending |
| Thermal behavior | No sustained severe throttling or OS thermal warning | Pending |

## Quality fallback

If a supported device cannot sustain 55 FPS during the one-minute interaction capture but remains at or above 30 FPS, repeat the matrix with `tier=reduced`. The reduced-tier URL currently proves fallback selection/reporting; this sparse feasibility scene has no nonessential effects to remove. Before Phase 8, any device requiring reduced mode must receive concrete reductions to decorative effects, resolution scale, animation density, or nonessential audio voices without changing canonical simulation, input quantization, table geometry, or rules. A device below 30 FPS, one that repeatedly reloads from memory pressure, or one that cannot reliably restore after foregrounding fails this gate.

## Acceptance decision

**Status: pending physical-device evidence.**

The 2026-08-16 local release build produced a 26,973,574-byte optimized WASM file (8,108,520 bytes with gzip). The complete generated bundle is 27,116,773 bytes raw and 8,132,636 bytes when each asset is gzip-compressed. The automated Google Chrome 151.0.7922.138 smoke test on macOS 26.6.1 (MacBook Air, Apple M4) loads normal, default-feasibility, and reduced-feasibility entry points at 1280×720, reaches the ready canvas, finds all required capture controls, and verifies reduced-tier reporting. On the same machine, Firefox 152.0.4 with geckodriver 0.37.1 passes the equivalent three-entry-point WebDriver coverage. Desktop smoke skips the slow Binaryen post-pass but still uses the release Rust/WASM build; the recorded size and all physical acceptance runs use the optimized bundle. Desktop smoke is not mobile compatibility or performance evidence.

Bevy is accepted only after every required mobile row passes the interaction/lifecycle checks, measured results meet the budget or the defined quality fallback, exact minimum browser versions are recorded, and audio lifecycle is proven. Otherwise record the failing evidence and reject or revise the client direction before introducing dependent client architecture.
