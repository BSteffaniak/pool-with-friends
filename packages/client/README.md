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

Open `http://localhost:8080`. The release bundle uses Bevy's WebGL2 renderer for broad modern-mobile-browser compatibility.
