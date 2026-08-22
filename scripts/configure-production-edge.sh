#!/bin/sh
set -eu

hostname=${PWMTF_HOSTNAME:-pwmtf.hyperchad.dev}
app=${FLY_APP_NAME:-pwmtf}
zone_name=${PWMTF_ZONE_NAME:-hyperchad.dev}
directory_path=/games/pool-with-more-than-friends
directory_target=https://pwmtf.hyperchad.dev

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
if [ "$hostname" != pwmtf.hyperchad.dev ] || [ "$zone_name" != hyperchad.dev ]; then
    printf '%s\n' "production infrastructure must use the canonical PWMTF hostname and zone" >&2
    exit 1
fi

api=https://api.cloudflare.com/client/v4
cloudflare() {
    curl --fail --silent --show-error \
        --header "Authorization: Bearer $CLOUDFLARE_API_TOKEN" \
        --header 'Content-Type: application/json' "$@"
}

zone_id=$(cloudflare "$api/zones?account.id=$CLOUDFLARE_ACCOUNT_ID&name=$zone_name" \
    | jq -er 'if .success and (.result | length) == 1 then .result[0].id else error("zone lookup failed") end')
fly_ipv6=$(flyctl ips list --app "$app" --json \
    | jq -er '[.[] | select((.Type // .type) == "v6") | .Address // .address] | if length == 1 then .[0] else error("expected one Fly IPv6 address") end')
certificate=$(flyctl certs show "$hostname" --app "$app" --json)
validation_hostname=$(jq -er '.DNSValidationHostname' <<EOF
$certificate
EOF
)
validation_target=$(jq -er '.DNSValidationTarget' <<EOF
$certificate
EOF
)

upsert_record() {
    type=$1
    name=$2
    content=$3
    proxied=$4
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
        cloudflare --request PUT --data "$payload" "$api/zones/$zone_id/dns_records/$id" \
            | jq -e '.success == true' >/dev/null
    else
        cloudflare --request POST --data "$payload" "$api/zones/$zone_id/dns_records" \
            | jq -e '.success == true' >/dev/null
    fi
}

upsert_record AAAA "$hostname" "$fly_ipv6" true
upsert_record CNAME "$validation_hostname" "$validation_target" false

# The dynamic redirect phase is a shared zone resource. Preserve every existing
# rule and replace only PWMTF's stable ref, refusing duplicate ownership.
rulesets=$(cloudflare "$api/zones/$zone_id/rulesets")
ruleset_id=$(jq -er '[.result[] | select(.phase == "http_request_dynamic_redirect" and .kind == "zone")] | if length == 1 then .[0].id else error("expected one zone dynamic redirect ruleset") end' <<EOF
$rulesets
EOF
)
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
cloudflare --request PUT --data "$payload" "$api/zones/$zone_id/rulesets/$ruleset_id" \
    | jq -e '.success == true' >/dev/null

printf '%s\n' "PWMTF DNS, certificate challenge, and managed directory redirect applied"
