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

## Feasibility validation

The prototype must be accepted from measurements rather than its desktop build alone. Use `docs/mobile-feasibility.md` for the required browser/device matrix, interaction checks, performance capture procedure, fallback threshold, and acceptance record. The checked-in browser smoke test can validate the generated shell in a local Chromium installation:

```sh
./scripts/test-browser-smoke.sh
```
