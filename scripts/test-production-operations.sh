#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-production-operations.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
mkdir "$tmp/bin"

cat >"$tmp/bin/flyctl" <<'MOCK'
#!/bin/sh
set -eu
case "$1 $2" in
    "ips list")
        printf '%s\n' '[{"Type":"v6","Address":"2a09:8280:1::1234"}]'
        ;;
    "certs show")
        if [ "${PWMTF_TEST_CERTIFICATE_STATE:-ready}" = ready ]; then
            printf '%s\n' '{"DNSValidationHostname":"_acme-challenge.pwmtf.hyperchad.dev","DNSValidationTarget":"pwmtf.hyperchad.dev.example.flydns.net.","ClientStatus":"Ready"}'
        else
            printf '%s\n' '{"DNSValidationHostname":"_acme-challenge.pwmtf.hyperchad.dev","DNSValidationTarget":"pwmtf.hyperchad.dev.example.flydns.net.","ClientStatus":"Awaiting configuration"}'
        fi
        ;;
    "status --app")
        case "${PWMTF_TEST_MACHINE_STATE:-one}" in
            one) printf '%s\n' '{"Machines":[{"id":"machine-1","state":"started"}]}' ;;
            two) printf '%s\n' '{"Machines":[{"id":"machine-1","state":"started"},{"id":"machine-2","state":"started"}]}' ;;
            *) printf '%s\n' '{"Machines":[]}' ;;
        esac
        ;;
    "secrets import")
        cat >"$PWMTF_TEST_TMP/staged-secrets"
        ;;
    "config validate"|"deploy --app")
        :
        ;;
    "volumes list")
        case "${PWMTF_TEST_VOLUME_STATE:-one}" in
            one) printf '%s\n' '[{"name":"pwmtf_data","state":"created","encrypted":true,"region":"ord","snapshot_retention":14,"auto_backup_enabled":true}]' ;;
            unencrypted) printf '%s\n' '[{"name":"pwmtf_data","state":"created","encrypted":false,"region":"ord","snapshot_retention":14,"auto_backup_enabled":true}]' ;;
            wrong_region) printf '%s\n' '[{"name":"pwmtf_data","state":"created","encrypted":true,"region":"iad","snapshot_retention":14,"auto_backup_enabled":true}]' ;;
            no_backups) printf '%s\n' '[{"name":"pwmtf_data","state":"created","encrypted":true,"region":"ord","snapshot_retention":14,"auto_backup_enabled":false}]' ;;
            wrong_retention) printf '%s\n' '[{"name":"pwmtf_data","state":"created","encrypted":true,"region":"ord","snapshot_retention":7,"auto_backup_enabled":true}]' ;;
            two) printf '%s\n' '[{"name":"pwmtf_data","state":"created","encrypted":true,"region":"ord","snapshot_retention":14,"auto_backup_enabled":true},{"name":"pwmtf_data","state":"created","encrypted":true,"region":"ord","snapshot_retention":14,"auto_backup_enabled":true}]' ;;
            *) printf '%s\n' '[]' ;;
        esac
        ;;
    "machine exec")
        command=$6
        case "$command" in
            *pwmtf-build-id*)
                case "${PWMTF_TEST_IDENTITY_STATE:-valid}" in
                    valid) printf '%s\n' '{"exit_code":0,"stdout":"","stderr":""}' ;;
                    *) printf '%s\n' '{"exit_code":1,"stdout":"","stderr":"identity mismatch"}' ;;
                esac
                ;;
            *)
                backup=$(printf '%s' "$command" | sed -n "s|.*\(/data/backups/pwmtf-[0-9TZ]*\.db\).*|\1|p")
                test -n "$backup"
                printf '{"exit_code":0,"stdout":"%s\\n","stderr":""}\n' "$backup"
                ;;
        esac
        ;;
    "machine restart")
        : >"$PWMTF_TEST_TMP/restart-ran"
        ;;
    "machine status")
        if [ "${PWMTF_TEST_MACHINE_CONFIG:-valid}" = valid ]; then
            cat <<EOF
Machine ID: machine-1
Config:
{
  "mounts": [{"name":"pwmtf_data","path":"/data","encrypted":true}],
  "services": [{"internal_port":8080,"autostop":false,"autostart":true,"min_machines_running":1,"ports":[{"port":80,"handlers":["http"],"force_https":true},{"port":443,"handlers":["http","tls"]}],"checks":[{"type":"http","path":"/readyz"}]}]
}
EOF
        else
            cat <<EOF
Machine ID: machine-1
Config:
{"mounts":[],"services":[{"internal_port":8080,"autostop":true,"autostart":true,"min_machines_running":0,"checks":[]}]}
EOF
        fi
        ;;
    *)
        printf '%s\n' "unexpected flyctl invocation: $*" >&2
        exit 1
        ;;
esac
MOCK
chmod +x "$tmp/bin/flyctl"

cat >"$tmp/bin/curl" <<'MOCK'
#!/bin/sh
set -eu
method=GET
data=
url=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --request) method=$2; shift 2 ;;
        --data) data=$2; shift 2 ;;
        --header|--connect-timeout|--max-time|--output|--write-out|--dump-header)
            shift 2
            ;;
        --fail|--silent|--show-error) shift ;;
        *) url=$1; shift ;;
    esac
done
case "$url" in
    https://pwmtf.hyperchad.dev/readyz)
        printf '%s\n' ready
        ;;
    */zones\?account.id=account-1\&name=hyperchad.dev)
        printf '%s\n' '{"success":true,"result":[{"id":"zone-1"}]}'
        ;;
    */zones/zone-1/dns_records\?*)
        printf '%s\n' '{"success":true,"result":[]}'
        ;;
    */zones/zone-1/dns_records)
        test "$method" = POST
        printf '%s\n' "$data" >>"$PWMTF_TEST_TMP/dns-payloads"
        printf '%s\n' '{"success":true}'
        ;;
    */zones/zone-1/settings/ssl)
        test "$method" = PATCH
        printf '%s\n' "$data" >"$PWMTF_TEST_TMP/ssl-payload"
        if [ "${PWMTF_TEST_SSL_STATE:-strict}" = strict ]; then
            printf '%s\n' '{"success":true,"result":{"value":"strict"}}'
        else
            printf '%s\n' '{"success":true,"result":{"value":"full"}}'
        fi
        ;;
    */zones/zone-1/rulesets)
        case "${PWMTF_TEST_RULESET_STATE:-one}" in
            one) printf '%s\n' '{"success":true,"result":[{"id":"ruleset-1","phase":"http_request_dynamic_redirect","kind":"zone"}]}' ;;
            none) printf '%s\n' '{"success":true,"result":[]}' ;;
            *) printf '%s\n' '{"success":true,"result":[{"id":"ruleset-1","phase":"http_request_dynamic_redirect","kind":"zone"},{"id":"ruleset-2","phase":"http_request_dynamic_redirect","kind":"zone"}]}' ;;
        esac
        ;;
    */zones/zone-1/rulesets/ruleset-1)
        if [ "$method" = GET ]; then
            printf '%s\n' '{"success":true,"result":{"description":"shared redirects","kind":"zone","name":"Shared redirects","phase":"http_request_dynamic_redirect","rules":[{"id":"rule-id","version":"4","last_updated":"2026-08-22T00:00:00Z","action":"redirect","action_parameters":{"from_value":{"status_code":301,"target_url":{"value":"https://example.com"}}},"expression":"http.host eq old.example","description":"Existing rule","enabled":true,"ref":"existing_rule"}]}}'
        else
            test "$method" = PUT
            printf '%s\n' "$data" >"$PWMTF_TEST_TMP/redirect-payload"
            printf '%s\n' '{"success":true}'
        fi
        ;;
    *)
        printf '%s\n' "unexpected curl URL: $url" >&2
        exit 1
        ;;
esac
MOCK
chmod +x "$tmp/bin/curl"

PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" \
    CLOUDFLARE_API_TOKEN=test-token CLOUDFLARE_ACCOUNT_ID=account-1 \
    "$root/scripts/configure-production-edge.sh" >/dev/null

[ "$(jq -s 'length' "$tmp/dns-payloads")" = 2 ]
jq -e 'select(.type == "AAAA" and .name == "pwmtf.hyperchad.dev" and .content == "2a09:8280:1::1234" and .proxied == true)' "$tmp/dns-payloads" >/dev/null
jq -e 'select(.type == "CNAME" and .name == "_acme-challenge.pwmtf.hyperchad.dev" and .content == "pwmtf.hyperchad.dev.example.flydns.net." and .proxied == false)' "$tmp/dns-payloads" >/dev/null
jq -e '.value == "strict"' "$tmp/ssl-payload" >/dev/null
jq -e '.rules | length == 2 and any(.[]; .ref == "existing_rule" and (.id | not) and (.version | not) and (.last_updated | not)) and any(.[]; .ref == "pwmtf_games_directory_redirect" and .action_parameters.from_value.status_code == 308 and .action_parameters.from_value.target_url.value == "https://pwmtf.hyperchad.dev" and (.expression | contains("hyperchad.dev") and contains("/games/pool-with-more-than-friends")))' "$tmp/redirect-payload" >/dev/null

rm -f "$tmp/dns-payloads" "$tmp/ssl-payload" "$tmp/redirect-payload"
PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" \
    CLOUDFLARE_API_TOKEN=test-token CLOUDFLARE_ACCOUNT_ID=account-1 \
    "$root/scripts/configure-production-edge.sh" origin >/dev/null
[ -f "$tmp/dns-payloads" ]
[ -f "$tmp/ssl-payload" ]
[ ! -e "$tmp/redirect-payload" ]
rm -f "$tmp/dns-payloads" "$tmp/ssl-payload" "$tmp/redirect-payload"
PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" \
    CLOUDFLARE_API_TOKEN=test-token CLOUDFLARE_ACCOUNT_ID=account-1 \
    "$root/scripts/configure-production-edge.sh" redirect >/dev/null
[ ! -e "$tmp/dns-payloads" ]
[ ! -e "$tmp/ssl-payload" ]
[ -f "$tmp/redirect-payload" ]

for ruleset_state in none two; do
    if PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" PWMTF_TEST_RULESET_STATE="$ruleset_state" \
        CLOUDFLARE_API_TOKEN=test-token CLOUDFLARE_ACCOUNT_ID=account-1 \
        "$root/scripts/configure-production-edge.sh" >/dev/null 2>&1; then
        printf '%s\n' "edge configuration unexpectedly accepted $ruleset_state shared redirect rulesets" >&2
        exit 1
    fi
done
if PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" PWMTF_TEST_SSL_STATE=weak \
    CLOUDFLARE_API_TOKEN=test-token CLOUDFLARE_ACCOUNT_ID=account-1 \
    "$root/scripts/configure-production-edge.sh" >/dev/null 2>&1; then
    printf '%s\n' "edge configuration unexpectedly accepted weak origin TLS" >&2
    exit 1
fi
if PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" FLY_APP_NAME=other-app \
    CLOUDFLARE_API_TOKEN=test-token CLOUDFLARE_ACCOUNT_ID=account-1 \
    "$root/scripts/configure-production-edge.sh" >/dev/null 2>&1; then
    printf '%s\n' "edge configuration unexpectedly accepted a noncanonical Fly app" >&2
    exit 1
fi

backup=$(PATH="$tmp/bin:$PATH" "$root/scripts/backup-production.sh")
printf '%s\n' "$backup" | grep -Eq '^/data/backups/pwmtf-[0-9]{8}T[0-9]{6}Z\.db$'
if PATH="$tmp/bin:$PATH" PWMTF_TEST_MACHINE_STATE=two \
    "$root/scripts/backup-production.sh" >/dev/null 2>&1; then
    printf '%s\n' "production backup unexpectedly accepted multiple started Machines" >&2
    exit 1
fi
skip_output=$(PATH="$tmp/bin:$PATH" PWMTF_TEST_MACHINE_STATE=none \
    "$root/scripts/backup-production.sh" --if-running)
[ "$skip_output" = "No started production Machine; backup skipped" ]
if PATH="$tmp/bin:$PATH" PWMTF_TEST_MACHINE_STATE=two \
    "$root/scripts/backup-production.sh" --if-running >/dev/null 2>&1; then
    printf '%s\n' "conditional production backup unexpectedly accepted multiple Machines" >&2
    exit 1
fi
if PATH="$tmp/bin:$PATH" PWMTF_DATABASE_PATH=/tmp/pwmtf.db \
    "$root/scripts/backup-production.sh" >/dev/null 2>&1; then
    printf '%s\n' "production backup unexpectedly accepted a database outside /data" >&2
    exit 1
fi
if PATH="$tmp/bin:$PATH" FLY_APP_NAME=other-app \
    "$root/scripts/backup-production.sh" >/dev/null 2>&1; then
    printf '%s\n' "production backup unexpectedly accepted a noncanonical Fly app" >&2
    exit 1
fi

cat >"$tmp/smoke" <<'MOCK'
#!/bin/sh
set -eu
[ "${1:-}" = --origin-only ]
: >"$PWMTF_TEST_TMP/smoke-ran"
MOCK
chmod +x "$tmp/smoke"
PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" \
    PWMTF_GOOGLE_CLIENT_ID=client-id PWMTF_GOOGLE_CLIENT_SECRET=client-secret \
    PWMTF_PRODUCTION_SMOKE_SCRIPT="$tmp/smoke" \
    "$root/scripts/deploy-production.sh" >/dev/null
[ "$(cat "$tmp/staged-secrets")" = "PWMTF_GOOGLE_CLIENT_ID=client-id
PWMTF_GOOGLE_CLIENT_SECRET=client-secret" ]
[ -f "$tmp/smoke-ran" ]
[ -f "$tmp/restart-ran" ]
if PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" PWMTF_TEST_MACHINE_STATE=two \
    PWMTF_GOOGLE_CLIENT_ID=client-id PWMTF_GOOGLE_CLIENT_SECRET=client-secret \
    PWMTF_PRODUCTION_SMOKE_SCRIPT="$tmp/smoke" \
    "$root/scripts/deploy-production.sh" >/dev/null 2>&1; then
    printf '%s\n' "production deployment unexpectedly accepted multiple Machines" >&2
    exit 1
fi
for volume_state in none unencrypted wrong_region no_backups wrong_retention two; do
    if PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" PWMTF_TEST_VOLUME_STATE="$volume_state" \
        PWMTF_GOOGLE_CLIENT_ID=client-id PWMTF_GOOGLE_CLIENT_SECRET=client-secret \
        PWMTF_PRODUCTION_SMOKE_SCRIPT="$tmp/smoke" \
        "$root/scripts/deploy-production.sh" >/dev/null 2>&1; then
        printf '%s\n' "production deployment unexpectedly accepted $volume_state production volumes" >&2
        exit 1
    fi
done
if PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" PWMTF_TEST_MACHINE_CONFIG=invalid \
    PWMTF_GOOGLE_CLIENT_ID=client-id PWMTF_GOOGLE_CLIENT_SECRET=client-secret \
    PWMTF_PRODUCTION_SMOKE_SCRIPT="$tmp/smoke" \
    "$root/scripts/deploy-production.sh" >/dev/null 2>&1; then
    printf '%s\n' "production deployment unexpectedly accepted an invalid Machine configuration" >&2
    exit 1
fi
if PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" PWMTF_TEST_IDENTITY_STATE=invalid \
    PWMTF_GOOGLE_CLIENT_ID=client-id PWMTF_GOOGLE_CLIENT_SECRET=client-secret \
    PWMTF_PRODUCTION_SMOKE_SCRIPT="$tmp/smoke" \
    "$root/scripts/deploy-production.sh" >/dev/null 2>&1; then
    printf '%s\n' "production deployment unexpectedly accepted mismatched bundle identity" >&2
    exit 1
fi
if PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" FLY_APP_NAME=other-app \
    PWMTF_GOOGLE_CLIENT_ID=client-id PWMTF_GOOGLE_CLIENT_SECRET=client-secret \
    PWMTF_PRODUCTION_SMOKE_SCRIPT="$tmp/smoke" \
    "$root/scripts/deploy-production.sh" >/dev/null 2>&1; then
    printf '%s\n' "production deployment unexpectedly accepted a noncanonical Fly app" >&2
    exit 1
fi
if PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" \
    PWMTF_GOOGLE_CLIENT_ID=client-id PWMTF_GOOGLE_CLIENT_SECRET= \
    PWMTF_PRODUCTION_SMOKE_SCRIPT="$tmp/smoke" \
    "$root/scripts/deploy-production.sh" >/dev/null 2>&1; then
    printf '%s\n' "production deployment unexpectedly accepted a partial OAuth secret pair" >&2
    exit 1
fi

printf '%s\n' "production operation self-tests passed"
