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
        printf '%s\n' '{"DNSValidationHostname":"_acme-challenge.pwmtf.hyperchad.dev","DNSValidationTarget":"pwmtf.hyperchad.dev.example.flydns.net."}'
        ;;
    "status --app")
        case "${PWMTF_TEST_MACHINE_STATE:-one}" in
            one) printf '%s\n' '{"Machines":[{"id":"machine-1","state":"started"}]}' ;;
            two) printf '%s\n' '{"Machines":[{"id":"machine-1","state":"started"},{"id":"machine-2","state":"started"}]}' ;;
            *) printf '%s\n' '{"Machines":[]}' ;;
        esac
        ;;
    "machine exec")
        command=$6
        backup=$(printf '%s' "$command" | sed -n "s|.*\(/data/backups/pwmtf-[0-9TZ]*\.db\).*|\1|p")
        test -n "$backup"
        printf '{"exit_code":0,"stdout":"%s\\n","stderr":""}\n' "$backup"
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
jq -e '.rules | length == 2 and any(.[]; .ref == "existing_rule" and (.id | not) and (.version | not) and (.last_updated | not)) and any(.[]; .ref == "pwmtf_games_directory_redirect" and .action_parameters.from_value.status_code == 308 and .action_parameters.from_value.target_url.value == "https://pwmtf.hyperchad.dev" and (.expression | contains("hyperchad.dev") and contains("/games/pool-with-more-than-friends")))' "$tmp/redirect-payload" >/dev/null

for ruleset_state in none two; do
    if PATH="$tmp/bin:$PATH" PWMTF_TEST_TMP="$tmp" PWMTF_TEST_RULESET_STATE="$ruleset_state" \
        CLOUDFLARE_API_TOKEN=test-token CLOUDFLARE_ACCOUNT_ID=account-1 \
        "$root/scripts/configure-production-edge.sh" >/dev/null 2>&1; then
        printf '%s\n' "edge configuration unexpectedly accepted $ruleset_state shared redirect rulesets" >&2
        exit 1
    fi
done

backup=$(PATH="$tmp/bin:$PATH" "$root/scripts/backup-production.sh")
printf '%s\n' "$backup" | grep -Eq '^/data/backups/pwmtf-[0-9]{8}T[0-9]{6}Z\.db$'
if PATH="$tmp/bin:$PATH" PWMTF_TEST_MACHINE_STATE=two \
    "$root/scripts/backup-production.sh" >/dev/null 2>&1; then
    printf '%s\n' "production backup unexpectedly accepted multiple started Machines" >&2
    exit 1
fi
if PATH="$tmp/bin:$PATH" PWMTF_DATABASE_PATH=/tmp/pwmtf.db \
    "$root/scripts/backup-production.sh" >/dev/null 2>&1; then
    printf '%s\n' "production backup unexpectedly accepted a database outside /data" >&2
    exit 1
fi

printf '%s\n' "production operation self-tests passed"
