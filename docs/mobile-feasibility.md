# Mobile browser feasibility record

This record defines the evidence required before PWMTF accepts Bevy as its production browser client. A successful build or desktop smoke test is not acceptance.

## Candidate configuration

- Bevy: `0.19.1`
- Renderer: WebGL2
- Enabled workspace features: `2d`, `ui`, `web`, `webgl2`
- Release profile: optimized, symbols stripped; Binaryen `wasm-opt -Oz` when available
- Browser shell: viewport safe areas, portrait overlay, startup progress, and explicit WebGL2/load failure state

The prototype deliberately contains presentation and input only. It does not establish canonical physics, gameplay authority, networking, or production asset choices.

## Required device matrix

Record the exact OS, browser version, hardware model, date, and result for each row. Test the oldest version PWMTF intends to support and the current stable version.

| Platform | Required browser | Device/version tested | Result |
| --- | --- | --- | --- |
| iPhone | Safari | Pending | Pending |
| iPad | Safari | Pending | Pending |
| Android phone | Chrome | Pending | Pending |
| Android tablet | Chrome | Pending | Pending |
| Desktop smoke | Chrome or Chromium | Automated by `scripts/test-browser-smoke.sh` | Pending current validation |
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
- [ ] A future audio probe starts only from a user gesture, suspends in background, and resumes intentionally.

The current prototype has no shipped audio content. Audio lifecycle remains an open feasibility item until a gesture-gated probe is measured on iOS Safari and Android Chrome.

## Measurement procedure

Use an uncached production build served over HTTPS or a representative throttled local connection. Capture at least three runs per device and report the median plus the worst observed run.

1. Build with `./scripts/build-wasm.sh` and record raw and gzip/Brotli transfer sizes for all generated assets.
2. Record navigation start to first visible table and navigation start to first accepted input.
3. Record steady-state memory after 60 seconds and peak memory during load using platform browser tooling.
4. Record frame rate and frame-time spikes during 60 seconds of continuous aiming and power adjustment.
5. Repeat the interaction loop for 10 minutes while recording battery/thermal warnings and visible throttling.
6. Rotate twice, resize/browser-chrome collapse twice, background for 30 seconds, then foreground and repeat input.
7. Save screenshots or traces with this record's date/device labels; do not store user credentials or unrelated browser data.

| Metric | Budget / acceptance rule | Measured result |
| --- | --- | --- |
| WASM compressed transfer | Record first; optimize or reject if it prevents reliable startup | 8,108,546 bytes gzip (2026-08-16 local release build) |
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

The 2026-08-16 local release build produced a 26,973,566-byte optimized WASM file (8,108,546 bytes with gzip), plus 112,783 bytes of JavaScript. The automated Chromium smoke test loads that build at 1280×720 and reaches the canvas, but it is not mobile compatibility or performance evidence.

Bevy is accepted only after every required mobile row passes the interaction/lifecycle checks, measured results meet the budget or the defined quality fallback, exact minimum browser versions are recorded, and audio lifecycle is proven. Otherwise record the failing evidence and reject or revise the client direction before introducing dependent client architecture.
