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
idempotently configures the proxied AAAA record and Fly ACME challenge, and
sets Cloudflare origin TLS to `strict` while failing unless the API confirms that
exact value. The workflow applies only origin DNS/TLS before deployment; it adds
the public directory redirect only after deployment, graceful restart, and two
origin-only smoke passes have succeeded, then runs the complete smoke including
the new redirect. A failed bring-up therefore cannot advertise an unavailable
product. It
preserves the shared Cloudflare redirect ruleset and replaces only the stable
`pwmtf_games_directory_redirect` rule. `scripts/deploy-production.sh` atomically
stages both Google credentials, validates `fly.toml`, deploys exactly one
Machine, waits for the certificate, and runs the production smoke test. Before
creating a Machine it also requires exactly one created, encrypted `pwmtf_data`
volume, in `ord` with 14-day snapshot retention and automatic Fly backups,
preventing an accidental ephemeral, unprotected, or ambiguous database
deployment. After deploy it reads the actual Machine configuration and rejects a
missing `/data` encrypted mount, autostop, absent always-running minimum, wrong
internal port, missing HTTP-to-HTTPS enforcement, missing TLS service, or missing
`/readyz` check. It also executes an in-Machine identity check that requires the
immutable build/source identity files to match the served `bootstrap.js`, without
printing either value. The deployment then performs a graceful real Machine restart,
requires the same sole Machine to return started, and reruns canonical readiness
and production smoke checks. This qualifies startup migration/recovery mechanics
for the deployed database without pretending to prove recovery after real play.
The
runtime image includes only the SQLite CLI and reviewed backup/restore helpers
needed for application-consistent operational backups. After deployment,
`scripts/backup-production.sh` creates a timestamped backup on the encrypted
volume and verifies it by restoring and checking a temporary copy.

Production deployment, edge configuration, and backup scripts reject any
`FLY_APP_NAME` other than the canonical `pwmtf` app so protected credentials
cannot be redirected to another Fly application through workflow environment
overrides.

The deployment workflow invokes `scripts/backup-production.sh --if-running`
before changing DNS or deploying. First deployment skips cleanly because no
Machine exists; every later deployment requires exactly one started Machine and
creates and restore-checks a timestamped pre-deploy database backup. After a
successful deployment it performs the same backup/restore verification again.

Production operation scripts have hermetic adapter self-tests in
`scripts/test-production-operations.sh`; CI runs them without requiring live
credentials and proves DNS payloads, preservation of unrelated shared redirect
rules, the exact `308` rule, atomic OAuth secret staging, canonical readiness and
smoke sequencing, exactly one created encrypted production volume in the pinned
region with backup/retention policy, post-deploy Machine
mount/availability/HTTPS/readiness configuration, one-Machine
deployment/backup gating, and path bounds.

## Deploy

After adding all protected secrets, dispatch **Deploy Production** from the
GitHub Actions page and approve the `production-approval` environment. The job
must finish successfully before the service is treated as deployed.

The production smoke test verifies the canonical application's HSTS, no-store
dynamic caching, referrer policy, content-type protection, permissions policy,
and cross-origin isolation headers. Application HTML must declare caching policy
and may not be publicly cacheable.

The production smoke test requires the internal WASM bundle integrity manifest to
remain an empty `404` at the canonical origin, preventing deployment metadata and
asset hashes from becoming an unintended public API.

The production smoke test also initiates the real Google authorization endpoint
with the canonical Origin without following the provider redirect. It requires a
`307` to `accounts.google.com`, the exact canonical callback, authorization-code
response type, profile scope, nonempty client/state/nonce/challenge parameters,
an S256 PKCE method, and a host-only Path=/ binding cookie with ten-minute
lifetime, Secure, HttpOnly, SameSite=Lax, and high priority attributes while
never printing or persisting opaque authorization query values. It
also sends a deliberately malformed callback and requires an unreflected `400`,
proving that malformed callback values fail before provider exchange. Finally,
it requires unauthenticated `/api/session` access and representative challenge,
invitation creation, and invitation-redemption mutations, plus a same-origin
WebSocket upgrade, to return empty `401` responses, proving public identity,
social workflow, and subscription boundaries remain closed. This proves
production OIDC boundary wiring, not successful user authentication.

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
