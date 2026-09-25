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
idempotently configures the proxied AAAA record, Fly `_fly-ownership` TXT,
and unproxied `_acme-challenge` CNAME for certificate issuance behind Cloudflare,
and sets Cloudflare origin TLS to `strict`. Like WWMTF, it reads
`dns_requirements.ownership.app_value` from `flyctl certs check --json`.
All deployment workflows pin flyctl 0.4.107; use that version locally too.
DNS/TLS changes belong to **Configure Production Infrastructure**, not app releases.
Run and approve it once to configure DNS, TLS, and the directory redirect together,
matching WWMTF's single infrastructure apply. There are no operation choices or
application-health prerequisites. The link may be published before the first app
release is healthy; this is intentional. The script
preserves the shared Cloudflare redirect ruleset and replaces only the stable
`pwmtf_games_directory_redirect` rule. `scripts/deploy-production.sh` atomically
stages both Google credentials, validates `fly.toml`, deploys exactly one
Machine, waits for actual canonical HTTPS readiness, and runs the production smoke test. Before
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
runtime image includes SQLite backup/restore helpers for offline maintenance only.
Do not run `scripts/backup-production.sh` against the live Turso database: it
holds a lock incompatible with the SQLite CLI's online backup.

Production deployment, edge configuration, and backup scripts reject any
`FLY_APP_NAME` other than the canonical `pwmtf` app so protected credentials
cannot be redirected to another Fly application through workflow environment
overrides.

The deployment workflow runs `python3 scripts/snapshot-production.py` before
releasing the image. As in WWMTF, it saves the original Machine configuration,
disables proxy autostart/minimum-Machine triggers using `skip_launch`, waits for
the Machine to stop, snapshots the attached encrypted volume, then restores the
configuration and starts the Machine. Restoration is attempted even if snapshot
creation fails; failures block deployment. First deployment skips only when no
Machine exists. This causes bounded downtime. No online SQLite backup or
post-deploy backup/restore test runs. PWMTF has no WWMTF supervisor archive hook;
the stopped-volume snapshot preserves the database and WAL together instead.
A snapshot API success does not claim a tested restore. If the runner is killed
during this operation, inspect the Machine and restore its autostart/minimum
settings from `fly.toml` before resuming releases.

Production operation scripts have hermetic adapter self-tests in
`scripts/test-production-operations.sh`; CI runs them without requiring live
credentials and proves DNS payloads, preservation of unrelated shared redirect
rules, the exact `308` rule, atomic OAuth secret staging, canonical readiness and
smoke sequencing, exactly one created encrypted production volume in the pinned
region with backup/retention policy, post-deploy Machine
mount/availability/HTTPS/readiness configuration, one-Machine
deployment/backup gating, and path bounds.

## Automated secret provisioning

Run locally with Python 3, authenticated `gh`, and authenticated `flyctl`:

```sh
python3 scripts/provision-production-secrets.py --repo BSteffaniak/pool-with-friends
# Explicitly replace just one credential:
python3 scripts/provision-production-secrets.py --repo BSteffaniak/pool-with-friends \
  --refresh --only CLOUDFLARE_API_TOKEN
```

The existing `Production` environment is required; its protection settings are
never rewritten. Each existing GitHub environment secret is skipped before any
provider access for that secret. `--only` is repeatable. `--refresh` replaces
all selected secrets, even if already present. Run only one provisioner per
repository/environment at a time; do not rotate while deployments are running.

Supply `CLOUDFLARE_PROVISION_TOKEN` locally using a secure credential manager.
This separate provisioning token needs user API Tokens Read/Edit plus access to
read the target zone; it must be allowed to grant the requested zone permissions.
It is never installed in GitHub. The script discovers the account ID and creates
a separate project token with Zone Read, DNS Write, Zone Settings Write (for
strict TLS), and Dynamic URL Redirects Write, restricted to `--zone` (default
`hyperchad.dev`). Cloudflare DNS authorization is zone-wide, not subdomain-wide.
Old tokens with the exact managed repo/environment name are revoked only after
GitHub confirms the replacement write. Existing manually named tokens are not
revoked automatically.

Fly uses the local flyctl authentication to create an app-scoped deploy token
for `--fly-app` (default `pwmtf`) with `--fly-expiry` (default `8760h`). Its CLI
creation response does not expose a revocation ID, so old Fly tokens must be
reviewed with `flyctl tokens list --app APP` and revoked by ID manually.
Google OAuth client creation remains manual: supply `PWMTF_GOOGLE_CLIENT_ID` and
`PWMTF_GOOGLE_CLIENT_SECRET` locally when those secrets need installation.

Values travel in memory and over provider HTTPS or gh stdin, never command
arguments, files, or printed diagnostics. Do not run with credential-dumping
shell/debug instrumentation. Missing tools, permissions, inputs, or unexpected
responses stop the run; earlier successful writes remain. An uncertain GitHub
upload leaves the new credential intact because the write may have succeeded:
inspect provider state before retrying, then refresh/clean up as appropriate.
Token creation with a lost response can also leave an orphan. No automated
claim of credential validity is made merely because a GitHub secret exists.

## Deploy

Every push to `master` starts **Deploy Production**: a Fly remote
build/push runs before the `production-approval` gate. Manual dispatch on `master`
is also supported. The build uses the `Production` environment's Fly token but
never creates a release, stages secrets, or modifies Machines. Keep that
environment without required reviewers; reviewers belong on `production-approval`.

The build tags its image with the commit SHA, workflow run ID, and attempt.
After image preparation succeeds, approve the intended SHA.
Heavyweight verification runs independently in **Validate** on pushes and PRs;
its result does not gate deployment. Approval is the explicit release decision,
so review the separate validation results before approving.
Deployment receives that exact tag via job outputs and uses `flyctl deploy
--image` without a rebuild or digest lookup. Tags are technically mutable: do not
overwrite prepared tags, and preserve pending candidates in registry retention.
Only the release job is serialized, so waiting
for approval does not block subsequent builds. Approving an older run can deploy
an older image: cancel obsolete approvals and check the SHA before approving.

Backup, secret staging, Machine update/restart, and origin smoke tests
still execute after approval. Application releases do not use Cloudflare credentials
or reconfigure DNS/TLS/redirects. Infrastructure and app mutations share one concurrency
group. Approval therefore avoids build latency, but is
not an instantaneous or zero-downtime switch (production uses one Machine).
The job must finish successfully before the service is treated as deployed.
Local invocation of `scripts/deploy-production.sh` also requires
`PWMTF_DEPLOY_IMAGE=registry.fly.io/pwmtf:build-<40-character SHA>-<run ID>-<attempt>`.

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
7. Create a stopped-volume snapshot after real accepted play and verify restoration
   to an isolated recovery environment before accepting this criterion. Deployment
   snapshot creation alone does not prove restore correctness. Do not use the
   SQLite CLI against the live Turso database.

Record only non-secret outcomes. Never record credentials, cookies, OIDC values,
raw invitation/session tokens, provider claims, or complete identity-linked shot
payloads.
