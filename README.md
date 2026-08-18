# Pool with More Than Friends

Pool with More Than Friends (PWMTF) is a mobile-browser-first, two-player online 8-ball game written primarily in Rust.

The planned production experience uses a Bevy WebAssembly client with immediate local shot prediction and a native Rust server that remains authoritative for physics, rules, timers, and results. Players sign in with Google and start private games through exact-handle challenges or single-use invitation links.

## Status

The repository is in its foundation and technical-feasibility stage. Product requirements and the production completion path are tracked in the local `pool-with-more-than-friends-progress.md` document.

## Architecture

- Domain-driven Cargo packages live under `packages/` and use the `pwmtf_` prefix.
- Canonical gameplay and physics remain independent of rendering and infrastructure.
- Browser presentation may predict and reconcile, but never becomes authoritative.
- Persistence uses Switchy builders and local Turso on a Fly Volume initially.
- The canonical production origin will be `https://pwmtf.hyperchad.dev`.

Read `INVARIANTS.md` for durable architecture and `AGENTS.md` for contributor requirements.

## Development

The workspace contains the concrete `pwmtf_client` feasibility package and no speculative domain, protocol, or server packages. As implementation packages are introduced, use:

```sh
cargo fmt --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace --no-fail-fast
cargo machete --with-metadata
cargo deny check
./scripts/check-native.sh
./scripts/check-wasm.sh
./scripts/build-wasm.sh
./scripts/report-wasm-size.sh
./scripts/serve-feasibility.sh
./scripts/summarize-feasibility.py --require-mobile-matrix path/to/reports/*.json
./scripts/test-feasibility-tools.sh
./scripts/test-feasibility-server.sh
./scripts/test-browser-telemetry.js
./scripts/test-browser-smoke.sh
./scripts/test-firefox-smoke.sh
./scripts/test-safari-smoke.sh
./scripts/check-architecture.sh
./scripts/test-architecture-checks.sh
```

With Nix:

```sh
nix develop
```

## License

PWMTF is licensed under the Mozilla Public License 2.0. See `LICENSE`.
