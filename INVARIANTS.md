# Architectural invariants

These are durable conditions of a valid Pool with More Than Friends implementation. They describe product and architectural truth rather than contributor workflow.

## Server gameplay authority

Only the native server's canonical match aggregate decides accepted commands, simulation state, pockets, groups, fouls, turns, deadlines, ball-in-hand, concessions, and results. Client, transport, render, prediction, interpolation, cache, and projection state never become gameplay authority.

## Domain independence

Canonical 8-ball rules and billiards physics are renderer- and infrastructure-independent. The game domain does not depend on Bevy, ECS query order, browser APIs, networking, HTTP, identity, persistence, Switchy, or deployment adapters.

## Deterministic simulation boundary

Every external gameplay input is finite, bounded, and quantized. Canonical simulation uses immutable versioned rules and physics profiles, stable identifiers and ordering, a fixed timestep, serializable complete state, and canonical event ordering. Every match pins its profile versions and rack seed.

## Durable command acceptance

Gameplay commands are authenticated, authorized, revision-checked, and idempotent. An accepted command is durably committed before acknowledgement. Recovery can reconstruct any acknowledged command and apply each timeout or transition exactly once.

## Timeout semantics

Disconnect never pauses a started turn deadline. A turn timeout ends only that turn and grants the opponent ball-in-hand. No number or pattern of timeouts automatically concedes, forfeits, or completes a match.

## Lobby and concurrency semantics

An accepted challenge creates a durable waiting lobby with no automatic expiration. A match starts atomically only while both participants are connected and explicitly ready. The product imposes no arbitrary per-user limit on pending challenges, waiting lobbies, or active matches, and no connection is a permanent primary client.

## Protocol and replay compatibility

Canonical wire commands, snapshots, and persisted payloads carry explicit versions. Readers reject unknown versions rather than guessing. Supported historical records remain replayable, and snapshot-plus-tail replay converges with full canonical replay.

## Persistence portability

Application schema and query access use Switchy builders rather than application-owned raw SQL or backend-specific branches. Read projections are derived and rebuildable and never become a second gameplay source of truth.

## Identity and secret custody

Production identity is keyed by verified Google issuer and subject. Raw session and invitation tokens are never persisted. Credentials, cookies, session or invitation tokens, OIDC state/nonce/code/token/subject values, provider picture URLs, and complete identity-linked shot payloads never enter logs, metrics, rendered output, or unauthorized transport payloads.

## Canonical web origin

`https://pwmtf.hyperchad.dev` is the canonical production application origin. `https://hyperchad.dev/games/pool-with-more-than-friends` is a managed directory redirect to that origin. Authentication callbacks, cookies, WebSocket origin checks, invitation links, canonical metadata, and smoke tests consistently use the canonical origin.

## Original presentation

Miniclip's game is a mechanics and quality reference only. PWMTF uses original branding, art, audio, interface assets, layouts, and presentation and does not ship copied assets or trade dress.
