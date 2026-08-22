# Production redirect rule

The managed games-directory entry must redirect only this exact source path:

- Source: `https://hyperchad.dev/games/pool-with-more-than-friends`
- Target: `https://pwmtf.hyperchad.dev`
- Status: `308 Permanent Redirect`

The owning HyperChad deployment must merge this rule into its managed redirect set rather than replacing unrelated rules. PWMTF infrastructure must not mutate the parent site's unmanaged configuration.

Production smoke qualification uses `./scripts/test-production-smoke.sh`; it asserts canonical health and application metadata plus the exact redirect source, target, and status, and rejects redirect chains or any target on a noncanonical origin.
