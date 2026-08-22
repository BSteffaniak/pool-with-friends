# Multiplayer transport selection

## Decision

PWMTF selects the focused secure-WebSocket protocol already implemented in `pwmtf_protocol`, `pwmtf_client`, and `pwmtf_server` for launch protocol version 1. Lightyear is rejected for the launch transport boundary.

This decision does not reject Lightyear as a library in general. It rejects introducing its Bevy/ECS replication model into PWMTF's authoritative boundary after the focused protocol has already satisfied the product's narrower two-player, turn-based requirements.

## Focused protocol evidence

The selected protocol provides:

- Same-origin secure-WebSocket upgrades authenticated by hash-only secure session cookies.
- Participant authorization for every match subscription and state-changing command.
- Bounded, explicitly versioned negotiation, commands, snapshots, and complete canonical payloads.
- Revision checks and cryptographic command identifiers for idempotent acceptance.
- Durable-before-acknowledgement command processing in the native server.
- Server-owned absolute deadlines that continue without a client connection.
- Complete settled snapshots for initialization, reconciliation, and reconnect.
- Client prediction that runs the renderer-independent canonical domain and is abandoned or corrected on transport loss/divergence.
- Multiple concurrent matches, tabs, and connections without selecting a primary client.
- Browser lifecycle reconnect with bounded backoff.

The launch game is turn-based and sends one bounded shot or placement command followed by a settled authoritative snapshot. It does not need generic high-frequency ECS component replication.

## Lightyear comparison

Lightyear's documentation describes a Bevy game-networking library with a client-server model in which the server is authoritative. Its prediction model creates `Confirmed` and `Predicted` Bevy entities and corrects mismatch through rollback and component history. Those are useful facilities for continuously simulated ECS games.

For PWMTF, adopting that model would add a second entity/component replication and rollback lifecycle beside the existing canonical aggregate, command journal, snapshot protocol, and renderer-owned interpolation. It would also couple transport prediction to Bevy entities at a boundary that must remain usable by the native server, persistence recovery, protocol tests, and non-Bevy consumers. Avoiding that coupling is more important than acquiring generic replication features that the launch traffic pattern does not require.

The focused protocol also has a smaller security surface for the product: its only state-changing messages are bounded canonical commands, and no client entity/component state can be mistaken for authority.

## Replication and snapshot policy

Launch protocol version 1 uses command/result snapshots rather than periodic component replication:

- Send a complete snapshot immediately after successful negotiation.
- Send one complete settled snapshot after every accepted canonical command or authoritative timeout.
- Send a complete current snapshot on reconnect.
- Do not replicate intermediate canonical ball positions over the network for launch; local prediction and presentation interpolation provide responsiveness while the server resolves the same canonical command.
- Keep command and snapshot bounds fixed and reject unknown protocol or canonical payload versions.

This policy is appropriate for a two-player turn-based game and avoids an arbitrary cadence that would create bandwidth and ordering work without changing authority.

## Revisit triggers

Reconsider Lightyear or a different transport only if measured product behavior requires continuous authoritative state streaming that the settled-snapshot protocol cannot satisfy, or if browser impairment evidence reveals a defect that cannot be corrected within this protocol. Any replacement must preserve canonical-domain independence, durable-before-acknowledgement acceptance, exact recovery, bounded input, and protocol version compatibility.

## Sources

- [Lightyear book introduction](https://cbournhonesque.github.io/lightyear/book/) — describes Lightyear as a Bevy networking library with an authoritative client-server architecture.
- [Lightyear client-side prediction](https://cbournhonesque.github.io/lightyear/book/concepts/advanced_replication/prediction.html) — describes confirmed/predicted Bevy entities, correction, rollback, and component histories.
