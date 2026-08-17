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

Open `http://localhost:8080`. The release bundle uses Bevy's WebGL2 renderer for broad modern-mobile-browser compatibility. When Binaryen is available, the build script applies size-oriented WebAssembly optimization.

Append `?feasibility` to expose the opt-in measurement panel and generated audio-lifecycle probe. The panel can capture startup/input timings, frame-rate windows, lifecycle counters, available JavaScript heap data, and a versioned JSON report. It is measurement support only; final acceptance still requires the physical-device and platform-tooling evidence in `docs/mobile-feasibility.md`.

## Feasibility validation

The prototype now includes an opt-in `?feasibility` capture panel and generated audio-lifecycle probe for collecting physical-device evidence. Use `docs/mobile-feasibility.md` for the required browser/device matrix, interaction checks, performance capture procedure, fallback threshold, and acceptance record. The checked-in browser smoke test validates the generated shell in a local Chromium installation and prints the exact browser version used:

```sh
./scripts/test-browser-smoke.sh
```
