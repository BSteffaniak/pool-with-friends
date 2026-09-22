# Physics version two

New matches and solo practice use physics profile 2. Profile 1 remains executable
with its original coefficients, arithmetic, event ordering, and snapshot layout.
A regression checks the pre-change 12,000-shot corpus checksum.

## Implemented

- Preserve fixed-point impulse precision by multiplying before division.
- Four deterministic substeps per 240 Hz tick and four contact sweeps per substep.
- Sliding friction of 0.20g with solid-sphere inertia (2/5 mR²); transition to
  natural rolling by clamping the slip impulse. Center hits approach 5/7 initial
  speed before rolling resistance dominates.
- Rolling deceleration 0.10 m/s², versus the historical 1.20 m/s².
- Horizontal rolling surface velocity and separate vertical spin; follow/draw
  act through cloth contact rather than identifier-dependent object-ball boosts.
- Ball contact friction 0.05 and cushion friction 0.20, with bounded tangential
  impulses coupled to vertical spin. Restitution remains 0.96 / 0.82.
- Version-two table snapshots serialize the extra spin component. Version-one
  stationary racks and historical snapshots remain readable.
- Practice honors spin input and interpolates between adjacent canonical ticks,
  removing exponential position chasing from practice playback.

Sources for calibration ranges and contact mechanics:
- https://billiards.colostate.edu/faq/physics/physical-properties/
- https://doi.org/10.1119/1.14747
- https://ekiefl.github.io/2020/04/24/pooltool-theory/

These constants are candidate calibration, not a claim of measured agreement
with a particular physical table. Tests cover low-speed momentum transfer,
sliding-to-rolling speed, rolling deceleration, draw/follow, spin persistence,
identifier symmetry, cushion restitution, and historical replay.

## Still outstanding

- Multiplayer full-trajectory playback: the existing snapshot transport exposes
  settled state, not a shot-start trajectory for both participants.
- Physical cushion noses, pocket jaws, shelf depth and mouth geometry, with
  corresponding rendered geometry. Current boundaries remain rectangular with
  circular capture regions.
- Exact continuous collision detection and simultaneous-contact convergence;
  bounded substeps/sweeps improve but do not solve every grazing/contact case.
- Three-dimensional cue elevation, squirt, swerve, jumps, and masse.
- Rendered ball orientation is still derived from travel, not canonical angular
  velocity; sliding and vertical spin are not yet faithfully animated.
- Real-device frame-budget measurement and human calibration against recorded
  reference shots, including fresh and worn cloth.

Do not mark the complete realism effort finished until these are addressed or
explicitly excluded from scope. The single authoritative Rust domain remains
the source of gameplay physics; no browser-specific physical model is added.
