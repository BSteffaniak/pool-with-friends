#!/bin/sh
set -eu

hostname=${PWMTF_HOSTNAME:-pwmtf.hyperchad.dev}
app=${FLY_APP_NAME:-pwmtf}
zone_name=${PWMTF_ZONE_NAME:-hyperchad.dev}
directory_path=/games/pool-with-more-than-friends
directory_target=https://pwmtf.hyperchad.dev
mode=${1:-all}
if [ "$#" -gt 1 ] || { [ "$mode" != all ] && [ "$mode" != origin ] && [ "$mode" != redirect ]; }; then
    printf '%s\n' "usage: $0 [origin|redirect]" >&2
    exit 2
fi

for name in CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID; do
    eval "value=\${$name-}"
    if [ -z "$value" ]; then
        printf '%s\n' "$name is required" >&2
        exit 1
    fi
done
for command in curl flyctl jq; do
    command -v "$command" >/dev/null 2>&1 || {
        printf '%s\n' "$command is required" >&2
        exit 1
    }
done
if [ "$app" != pwmtf ] || [ "$hostname" != pwmtf.hyperchad.dev ] || [ "$zone_name" != hyperchad.dev ]; then
    printf '%s\n' "production infrastructure must use the canonical PWMTF app, hostname, and zone" >&2
    exit 1
fi

# Track only fixed operation labels, never request arguments or provider payloads.
stage=initialization
report_exit() {
    status=$?
    if [ "$status" -ne 0 ]; then
        printf 'Production edge setup failed: %s (exit %s)\n' "$stage" "$status" >&2
    fi
}
trap report_exit EXIT
step() {
    stage=$1
    printf 'Production edge: %s\n' "$stage" >&2
}

api=https://api.cloudflare.com/client/v4
cloudflare() {
    # Keep the response in memory and print only numeric provider error codes.
    response=$(curl --fail --silent --show-error --connect-timeout 10 --max-time 60 \
        --header "Authorization: Bearer $CLOUDFLARE_API_TOKEN" \
        --header 'Content-Type: application/json' "$@") || {
        printf 'Cloudflare transport/HTTP failure during %s; check the HTTP error above and token permissions\n' "$stage" >&2
        return 1
    }
    if ! printf '%s' "$response" | jq -e '.success == true' >/dev/null 2>&1; then
        codes=$(printf '%s' "$response" | jq -r '[.errors[]? | .code | select(type == "number") | tostring] | join(",")' 2>/dev/null) || codes=unavailable
        printf 'Cloudflare rejected %s; error codes: %s (response body suppressed)\n' "$stage" "${codes:-unavailable}" >&2
        return 1
    fi
    printf '%s\n' "$response"
}

step 'discover Cloudflare zone'
zone_id=$(cloudflare "$api/zones?account.id=$CLOUDFLARE_ACCOUNT_ID&name=$zone_name" \
    | jq -er 'if .success and (.result | length) == 1 then .result[0].id else error("zone lookup failed") end')
step 'discover Fly IPv6 address'
fly_ipv6=$(flyctl ips list --app "$app" --json \
    | jq -er '[.[] | select((.Type // .type) == "v6") | .Address // .address] | if length == 1 then .[0] else error("expected one Fly IPv6 address") end')
step 'read Fly certificate'
certificate=$(flyctl certs check --app "$app" --json "$hostname")
step 'extract Fly ownership TXT value'
validation_target=$(jq -er '.dns_requirements.ownership.app_value | if type == "string" and length > 0 then . else error("Fly certificate is missing dns_requirements.ownership.app_value; use flyctl 0.4.107") end' <<EOF
$certificate
EOF
)
validation_hostname="_fly-ownership.$hostname"
step 'extract Fly ACME DNS challenge'
acme_target=$(printf '%s' "$certificate" | jq -er '.dns_requirements.acme_challenge.target | if type == "string" and length > 0 then . else error("Fly ACME challenge target is missing") end')
acme_hostname=$(printf '%s' "$certificate" | jq -er '.dns_requirements.acme_challenge.name | if type == "string" and length > 0 then . else error("Fly ACME challenge hostname is missing") end')
if [ "$acme_hostname" != "_acme-challenge.$hostname" ]; then
    printf '%s\n' 'Unexpected Fly ACME challenge hostname; refusing DNS mutation' >&2
    exit 1
fi

upsert_record() {
    type=$1
    name=$2
    content=$3
    proxied=$4
    step "look up $type DNS record"
    existing=$(cloudflare "$api/zones/$zone_id/dns_records?type=$type&name=$name")
    count=$(jq -r '.result | length' <<EOF
$existing
EOF
)
    if [ "$count" -gt 1 ]; then
        printf '%s\n' "multiple existing $type records for $name; refusing an ambiguous update" >&2
        exit 1
    fi
    payload=$(jq -n --arg type "$type" --arg name "$name" --arg content "$content" \
        --argjson proxied "$proxied" \
        '{type:$type,name:$name,content:$content,ttl:1,proxied:$proxied}')
    if [ "$count" -eq 1 ]; then
        id=$(jq -r '.result[0].id' <<EOF
$existing
EOF
)
        step "update $type DNS record"
        cloudflare --request PUT --data "$payload" "$api/zones/$zone_id/dns_records/$id" \
            | jq -e '.success == true' >/dev/null
    else
        step "create $type DNS record"
        cloudflare --request POST --data "$payload" "$api/zones/$zone_id/dns_records" \
            | jq -e '.success == true' >/dev/null
    fi
}

if [ "$mode" != redirect ]; then
    upsert_record AAAA "$hostname" "$fly_ipv6" true
    upsert_record TXT "$validation_hostname" "$validation_target" false
    # Ownership alone does not issue a certificate. DNS-01 works through the
    # Cloudflare proxy without requiring an already-valid origin TLS connection.
    upsert_record CNAME "$acme_hostname" "$acme_target" false

    # Cloudflare must authenticate Fly's origin certificate. Flexible or Full mode
    # would weaken the canonical TLS boundary for every proxied request.
    step 'set strict Cloudflare origin TLS'
    ssl_payload='{"value":"strict"}'
    cloudflare --request PATCH --data "$ssl_payload" "$api/zones/$zone_id/settings/ssl" \
        | jq -e '.success == true and .result.value == "strict"' >/dev/null
fi

if [ "$mode" = origin ]; then
    printf '%s\n' "PWMTF DNS, certificate challenge, and strict origin TLS applied"
    exit 0
fi

# The dynamic redirect phase is a shared zone resource. Preserve every existing
# rule and replace only PWMTF's stable ref, refusing duplicate ownership.
step 'list Cloudflare redirect rulesets'
rulesets=$(cloudflare "$api/zones/$zone_id/rulesets")
ruleset_id=$(jq -er '[.result[] | select(.phase == "http_request_dynamic_redirect" and .kind == "zone")] | if length == 1 then .[0].id else error("expected one zone dynamic redirect ruleset") end' <<EOF
$rulesets
EOF
)
step 'read Cloudflare redirect ruleset'
ruleset=$(cloudflare "$api/zones/$zone_id/rulesets/$ruleset_id")
pwmtf_count=$(jq '[.result.rules[]? | select(.ref == "pwmtf_games_directory_redirect")] | length' <<EOF
$ruleset
EOF
)
if [ "$pwmtf_count" -gt 1 ]; then
    printf '%s\n' "multiple PWMTF redirect rules exist; refusing to replace shared state" >&2
    exit 1
fi
new_rule=$(jq -n --arg path "$directory_path" --arg target "$directory_target" '{action:"redirect",action_parameters:{from_value:{status_code:308,target_url:{value:$target},preserve_query_string:false}},expression:("http.host eq \"hyperchad.dev\" and http.request.uri.path eq \"" + $path + "\""),description:"Redirect Pool with More Than Friends to its canonical origin",enabled:true,ref:"pwmtf_games_directory_redirect"}')
payload=$(jq --argjson rule "$new_rule" '.result | .rules = ([.rules[]? | select(.ref != "pwmtf_games_directory_redirect") | del(.id, .version, .last_updated)] + [$rule]) | {description,kind,name,phase,rules}' <<EOF
$ruleset
EOF
)
step 'update managed directory redirect'
cloudflare --request PUT --data "$payload" "$api/zones/$zone_id/rulesets/$ruleset_id" \
    | jq -e '.success == true' >/dev/null

printf '%s\n' "PWMTF managed directory redirect applied"
