# PWMTF browser client

This package owns the Bevy/WASM presentation feasibility prototype. It intentionally contains no canonical pool physics or rules.

## Native check

```sh
cargo run -p pwmtf_client
```

## Web build

```sh
./scripts/build-wasm.sh
python3 -m http.server --directory dist 8080
```

Open `http://localhost:8080`. The release bundle uses Bevy's WebGL2 renderer for broad modern-mobile-browser compatibility. Each build removes stale top-level files from `dist/` before generating the complete bundle, so size reports and served assets describe only the current build. The build always injects a deterministic SHA-256 of Cargo/toolchain/client inputs and uses `PWMTF_BUILD_ID` when supplied; otherwise it derives the build ID from the Git revision plus that source hash, so explicit release labels remain independently tied to exact candidate inputs and reports from changed working-tree candidates cannot be silently combined. When Binaryen is available, the build script applies size-oriented WebAssembly optimization. Desktop smoke scripts set `PWMTF_SKIP_WASM_OPT=1` to avoid repeating the slow Binaryen pass; physical measurements, bundle reports, and acceptance builds must not set it.

The selected presentation tier is URL-bound rather than self-reported: use `?feasibility` for default quality or `?feasibility&tier=reduced` for the reduced fallback, and the client displays/exports the active tier. The panel requires exact device/browser, candidate build ID, proposed-minimum-version, cache-state, presentation-tier, and run metadata plus Pass/Fail results and platform-tool timing/memory/thermal/reload observations before export. It captures frame-rate windows, median/95th/99th-percentile/worst frame intervals, lifecycle counters, non-sensitive operator-marked events, available JavaScript heap data, and a versioned JSON report with a device/run-specific filename. After collecting at least three comparable runs, `./scripts/summarize-feasibility.py path/to/*.json` validates required check keys and run uniqueness/completeness and prints a Markdown median/worst table including aggregate physical-check status. Add `--require-mobile-matrix` for final browser-gate evidence; it requires cold, warm, and lifecycle runs at both current and proposed-minimum versions for iPhone Safari, iPad Safari, Android phone Chrome/Firefox/Samsung Internet, Android tablet Chrome, and desktop Safari/Chrome/Firefox/Edge, and fails closed on mixed candidate identities, missing browser/version/cache groups, physical/budget/FPS/frame-time/duration/lifecycle/audio failures (including pre/post-mute playback), or an unproven quality fallback. Run `./scripts/test-feasibility-tools.sh` to self-test a complete passing synthetic matrix plus matrix/minimum-version, FPS, startup-budget, interaction/lifecycle-duration, and audio-lifecycle rejection paths. Downloaded feasibility JSON, trace, and HAR evidence is ignored by Git by default; keep it local unless deliberately sanitized and approved. These are measurement support only; final acceptance still requires the physical-device and platform-tooling evidence in `docs/mobile-feasibility.md`.

## Feasibility validation

The prototype now includes an opt-in `?feasibility` capture panel and generated audio-lifecycle probe for collecting physical-device evidence. Use `./scripts/report-wasm-size.sh` for a reproducible raw/gzip bundle report that prints the candidate build ID and source hash before per-asset totals. Set `PWMTF_SKIP_BUILD=1` only to inspect an existing identified `dist/` candidate without rebuilding, then follow `docs/mobile-feasibility.md` for the required browser/device matrix, interaction checks, performance capture procedure, fallback threshold, and acceptance record. The checked-in Chromium smoke test validates both the normal generated shell and the opt-in feasibility entry point, asserts the capture controls are present, and prints the exact browser version used:

```sh
./scripts/test-browser-smoke.sh
```

On macOS, Safari compatibility has a matching WebDriver smoke entry point. First enable **Allow remote automation** in Safari's Developer settings, then run:

```sh
./scripts/test-safari-smoke.sh
```

Firefox has an equivalent headless WebDriver check for normal, default-feasibility, and reduced-feasibility entry points. It also verifies required capture controls and reduced-tier reporting:

```sh
FIREFOX_BIN=/path/to/firefox \
GECKODRIVER_BIN=/path/to/geckodriver \
./scripts/test-firefox-smoke.sh
```

Safari exercises the same three entry points and feasibility assertions. Neither desktop smoke substitutes for the physical mobile matrix.

## Physical-device HTTPS serving

Mobile feasibility must be measured in a secure context using a certificate the device trusts. Prepare a certificate and key whose subject covers a hostname or IP reachable from the test devices, then run:

```sh
PWMTF_FEASIBILITY_HOST=pool-test.example.test \
PWMTF_TLS_CERT=/absolute/path/to/certificate.pem \
PWMTF_TLS_KEY=/absolute/path/to/private-key.pem \
./scripts/serve-feasibility.sh
```

Optional `PWMTF_FEASIBILITY_BIND` and `PWMTF_FEASIBILITY_PORT` values select the listener; the defaults are `0.0.0.0` and `8443`. Set `PWMTF_SKIP_BUILD=1` only to reuse an existing `dist/` build. The HTTPS physical-device server independently refuses unoptimized or source-unbound bundles before listening, so `PWMTF_SKIP_BUILD=1` cannot accidentally serve desktop-smoke artifacts as physical evidence. Open the printed `https://…/?feasibility` URL on each test device. The server verifies that the certificate covers `PWMTF_FEASIBILITY_HOST` before listening; device trust still must be established externally and warnings must never be bypassed. The server also rejects invalid, near-expiry, shared-file, or certificate/key-mismatch TLS inputs before serving. The `./scripts/test-feasibility-server.sh` self-test covers certificate-host mismatch, key mismatch, optimized server startup, and required response security headers. Do not expose the private key.
