# Mobile browser feasibility record

This record defines the evidence required before PWMTF accepts Bevy as its production browser client. A successful build or desktop smoke test is not acceptance.

## Candidate configuration

- Bevy: `0.19.1`
- Renderer: WebGL2
- Enabled workspace features: `2d`, `ui`, `web`, `webgl2`
- Release profile: optimized, symbols stripped; Binaryen `wasm-opt -Oz` when available
- Browser shell: viewport safe areas, portrait overlay, startup progress, explicit WebGL2/load failure state, and an opt-in feasibility capture panel

Open the built client with `?feasibility` to expose the capture panel. The normal product URL does not show it. The panel records browser/display metadata, startup and first-contact timings, one-second frame-rate windows, the largest frame gap, interaction/lifecycle counters, and JavaScript heap data where the browser exposes `performance.memory`. It exports a versioned JSON report for attachment to this record.

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
| Desktop smoke | Chrome or Chromium | Google Chrome 151.0.7922.138; macOS 26.6.1; MacBook Air (M4); 2026-08-16 | Pass — automated WebGL2/SwiftShader shell load at 1280×720 reached the ready canvas |
| Desktop compatibility | Safari | Pending | Pending |
| Desktop compatibility | Firefox | Pending | Pending |

Do not declare minimum browser versions until the physical rows pass and are recorded here.

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

1. Build with `./scripts/build-wasm.sh` and record raw and gzip/Brotli transfer sizes for all generated assets.
2. Serve `dist/` over HTTPS, open `/?feasibility`, and start a fresh capture after the table is visible.
3. Record navigation start to first visible table and navigation start to first accepted input. The exported report supplies client-ready and first-contact approximations; use browser tooling for the final visible-paint measurement.
4. Record steady-state memory after 60 seconds and peak memory during load using platform tooling. Treat the JSON JavaScript heap figures as supporting evidence only because Safari and Firefox may not expose them and they exclude WASM/graphics memory.
5. Capture one minute of repeated aim and power interaction and report median, 95th-percentile, and worst frame time. The panel's one-second FPS windows and largest gap are supporting evidence, not a replacement for browser/platform traces.
6. Keep the app active for 10 minutes, including repeated orientation changes and at least two 30-second background/foreground cycles.
7. Exercise the audio probe before and after mute, background the page while its context is running, then explicitly resume from the button after foregrounding.
8. Download the JSON report, record battery and thermal observations available from the OS/device tooling, and note any browser reload, graphics reset, audio failure, or input loss.
9. Repeat on the quality fallback if the default tier cannot hold the target frame rate.
10. Save screenshots or traces with this record's date/device labels; do not store user credentials or unrelated browser data.

| Metric | Budget / acceptance rule | Measured result |
| --- | --- | --- |
| WASM compressed transfer | Record first; optimize or reject if it prevents reliable startup | 26,973,574 bytes raw / 8,108,520 bytes gzip (`wasm-opt -Oz`, 2026-08-16 local release build) |
| Complete generated bundle | Record all generated assets | 27,105,745 bytes raw / 8,129,868 bytes gzip (2026-08-16 local release build; gzip members summed) |
| Cold first-visible table | <= 8 s on representative mobile broadband | Pending |
| Warm first-visible table | <= 3 s | Pending |
| Steady aiming frame rate | 60 FPS target; >= 30 FPS fallback floor | Pending |
| Input response | No visible frame-delayed aiming under steady load | Pending |
| Memory | No reload/eviction and a stable plateau during the 10-minute run | Pending |
| Thermal behavior | No sustained severe throttling or OS thermal warning | Pending |

## Quality fallback

If a supported device cannot sustain 55 FPS during the one-minute interaction capture but remains at or above 30 FPS, the production client must select a reduced presentation tier before Phase 8 completion. That tier may reduce decorative effects, resolution scale, animation density, and nonessential audio voices, but may not alter canonical simulation, input quantization, table geometry, or rules. A device below 30 FPS, one that repeatedly reloads from memory pressure, or one that cannot reliably restore after foregrounding fails this gate.

## Acceptance decision

**Status: pending physical-device evidence.**

The 2026-08-16 local release build produced a 26,973,574-byte optimized WASM file (8,108,520 bytes with gzip). The complete generated bundle is 27,105,745 bytes raw and 8,129,868 bytes when each asset is gzip-compressed. The automated Google Chrome 151.0.7922.138 smoke test on macOS 26.6.1 (MacBook Air, Apple M4) loads that build at 1280×720 and reaches the ready canvas, but it is not mobile compatibility or performance evidence.

Bevy is accepted only after every required mobile row passes the interaction/lifecycle checks, measured results meet the budget or the defined quality fallback, exact minimum browser versions are recorded, and audio lifecycle is proven. Otherwise record the failing evidence and reject or revise the client direction before introducing dependent client architecture.
