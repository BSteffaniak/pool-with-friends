# Production deployment

PWMTF production uses one always-running Fly Machine and one encrypted Fly Volume:

```text
Browser -> Cloudflare -> Fly Proxy -> one Machine -> /data/pwmtf.db
```

The canonical origin is `https://pwmtf.hyperchad.dev`. The exact directory path
`https://hyperchad.dev/games/pool-with-more-than-friends` redirects there with
status `308`.

## One-time protected setup

The public GitHub repository has `Production` and `production-approval`
environments. The approval environment requires `BSteffaniak` review and accepts
deployments only from `master`. Store these values only as secrets in
`Production`:

- `FLY_API_TOKEN`: a token authorized for the existing `pwmtf` Fly application.
- `CLOUDFLARE_ACCOUNT_ID`: the account containing `hyperchad.dev`.
- `CLOUDFLARE_API_TOKEN`: a token with zone read, DNS edit, and Single Redirect
  Edit (Dynamic URL Redirects Write) for `hyperchad.dev`.
- `PWMTF_GOOGLE_CLIENT_ID` and `PWMTF_GOOGLE_CLIENT_SECRET`: a dedicated Google
  OAuth 2.0 **Web application** client. Its authorized JavaScript origin must be
  exactly `https://pwmtf.hyperchad.dev`, and its authorized redirect URI must be
  exactly `https://pwmtf.hyperchad.dev/auth/google/callback`.

Do not reuse WWMTF's client or expose secret values in files, command arguments,
logs, issues, or workflow output. Configure required reviewers on
`production-approval` before dispatching production deployment.

The Fly application, dedicated IPv6 address, certificate request, and encrypted
`pwmtf_data` volume already exist. The current Dockerfile has completed a real
Fly remote build as a 41 MB image, including SQLite and the backup/restore
helpers; build-only qualification did not create a release or Machine.
`scripts/configure-production-edge.sh`
idempotently configures the proxied AAAA record and Fly ACME challenge. It
preserves the shared Cloudflare redirect ruleset and replaces only the stable
`pwmtf_games_directory_redirect` rule. `scripts/deploy-production.sh` atomically
stages both Google credentials, validates `fly.toml`, deploys exactly one
Machine, waits for the certificate, and runs the production smoke test. The
runtime image includes only the SQLite CLI and reviewed backup/restore helpers
needed for application-consistent operational backups. After deployment,
`scripts/backup-production.sh` creates a timestamped backup on the encrypted
volume and verifies it by restoring and checking a temporary copy.

Production operation scripts have hermetic adapter self-tests in
`scripts/test-production-operations.sh`; CI runs them without requiring live
credentials and proves DNS payloads, preservation of unrelated shared redirect
rules, the exact `308` rule, one-Machine backup gating, and path bounds.

## Deploy

After adding all protected secrets, dispatch **Deploy Production** from the
GitHub Actions page and approve the `production-approval` environment. The job
must finish successfully before the service is treated as deployed.

## Required manual acceptance

Infrastructure success does not establish product closure. Using two independent
browser profiles and two real Google accounts:

1. Complete Google sign-in and create distinct handles.
2. Verify exact-handle challenge and private invitation entry paths.
3. Explicitly ready both lobby participants and start one match.
4. Complete a legal or illegal 8-ball result through the browser controls.
5. Verify reconnect, deployed-process restart, result recovery, concession, and
   a linked rematch with alternating breaker.
6. Repeat the complete flow on supported iOS Safari and Android Chrome devices.
7. Perform and verify an application-consistent backup and restore after real
   accepted play. The deployment workflow's initial empty-database check proves
   the mechanism only; repeat `scripts/backup-production.sh` after the played
   match before accepting this criterion.

Record only non-secret outcomes. Never record credentials, cookies, OIDC values,
raw invitation/session tokens, provider claims, or complete identity-linked shot
payloads.
