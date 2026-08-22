# PWMTF server

This package owns authoritative command authorization, revision/idempotency enforcement, durable acceptance, scheduling, recovery, identity, social/lobby/rematch persistence, and native HTTP/OIDC/secure-WebSocket operations. Canonical gameplay decisions remain in `pwmtf_game_domain`; this package authenticates and durably orchestrates them. A rejected WebSocket command receives a stable rejection marker followed by the current complete authoritative snapshot, allowing the client to roll back prediction without dropping a healthy connection.

This package also exposes an explicitly insecure, compile-time-gated localhost
adapter for manual two-browser testing. `--features insecure` and
`PWMTF_DEV_MODE=true` must agree exactly; the configured origin must be loopback
HTTP and Google credentials must be absent. Username-only login creates the same
hash-only durable sessions and stable handles used by normal product flows, but
never creates or impersonates a Google identity. Production builds do not contain
the development route.

`./scripts/test-native-deployment-smoke.sh` qualifies the native boundary with independently authenticated WebSocket participants, measured base delay, deterministic jitter, first-transmission loss/retransmission, duplicate/noise delivery, intermediate reconnect, terminal concession convergence under the same impairment model, terminal reconnect for both participants, exact durable command counts, deadline removal, process restart, and application-consistent backup/restore. It also checks authenticated match-access seat/revision/active-player/completion/deadline projections before and after terminal convergence.
