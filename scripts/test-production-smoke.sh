#!/bin/sh
set -eu

canonical_origin=${PWMTF_CANONICAL_ORIGIN:-https://pwmtf.hyperchad.dev}
directory_url=${PWMTF_DIRECTORY_URL:-https://hyperchad.dev/games/pool-with-more-than-friends}
expected_directory_target=${PWMTF_DIRECTORY_TARGET:-$canonical_origin}
max_time=${PWMTF_PRODUCTION_SMOKE_TIMEOUT_SECONDS:-20}
curl_bin=${PWMTF_CURL_BIN:-curl}

if ! command -v "$curl_bin" >/dev/null 2>&1; then
    printf '%s\n' "PWMTF_CURL_BIN must name an executable curl-compatible command" >&2
    exit 1
fi

case "$canonical_origin" in
    https://pwmtf.hyperchad.dev|https://pwmtf.hyperchad.dev/)
        ;;
    *)
        printf '%s\n' "PWMTF_CANONICAL_ORIGIN must be the canonical production origin" >&2
        exit 1
        ;;
esac
canonical_origin=${canonical_origin%/}
expected_directory_target=${expected_directory_target%/}

case "$max_time" in
    ''|*[!0-9]*)
        printf '%s\n' "PWMTF_PRODUCTION_SMOKE_TIMEOUT_SECONDS must be a positive integer" >&2
        exit 1
        ;;
esac
if [ "$max_time" -lt 1 ] || [ "$max_time" -gt 120 ]; then
    printf '%s\n' "PWMTF_PRODUCTION_SMOKE_TIMEOUT_SECONDS must be from 1 through 120" >&2
    exit 1
fi

response_headers=$(mktemp "${TMPDIR:-/tmp}/pwmtf-production-smoke.XXXXXX")
response_body=$(mktemp "${TMPDIR:-/tmp}/pwmtf-production-smoke-body.XXXXXX")
trap 'rm -f "$response_headers" "$response_body"' EXIT HUP INT TERM

request() {
    url=$1
    "$curl_bin" \
        --fail \
        --silent \
        --show-error \
        --connect-timeout "$max_time" \
        --max-time "$max_time" \
        --dump-header "$response_headers" \
        --output "$response_body" \
        "$url"
}

request "$canonical_origin/healthz"
status=$(awk 'NR == 1 { print $2 }' "$response_headers")
if [ "$status" != 200 ] || [ "$(cat "$response_body")" != ok ]; then
    printf '%s\n' "canonical health check did not return exact 200/ok" >&2
    exit 1
fi
grep -qi '^strict-transport-security:' "$response_headers"
grep -qi '^x-content-type-options: nosniff' "$response_headers"

request "$canonical_origin/readyz"
status=$(awk 'NR == 1 { print $2 }' "$response_headers")
if [ "$status" != 200 ] || [ "$(cat "$response_body")" != ready ]; then
    printf '%s\n' "canonical readiness check did not return exact 200/ready" >&2
    exit 1
fi
grep -qi '^strict-transport-security:' "$response_headers"
grep -qi '^x-content-type-options: nosniff' "$response_headers"

request "$canonical_origin/"
status=$(awk 'NR == 1 { print $2 }' "$response_headers")
if [ "$status" != 200 ]; then
    printf '%s\n' "canonical application did not return 200" >&2
    exit 1
fi
grep -Fq '<link rel="canonical" href="https://pwmtf.hyperchad.dev/"' "$response_body"
grep -Fq 'data-client-state="loading"' "$response_body"
grep -qi '^content-security-policy:' "$response_headers"
grep -qi '^x-content-type-options: nosniff' "$response_headers"

"$curl_bin" \
    --silent \
    --show-error \
    --connect-timeout "$max_time" \
    --max-time "$max_time" \
    --max-redirs 0 \
    --dump-header "$response_headers" \
    --output "$response_body" \
    "$directory_url"
status=$(awk 'NR == 1 { print $2 }' "$response_headers")
location=$(awk 'BEGIN { IGNORECASE=1 } /^location:/ { sub(/^[^:]+:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit }' "$response_headers")
location=${location%/}
if [ "$status" != 308 ]; then
    printf '%s\n' "games-directory redirect returned $status instead of 308" >&2
    exit 1
fi
if [ "$location" != "$expected_directory_target" ]; then
    printf '%s\n' "games-directory redirect target is not the canonical origin" >&2
    exit 1
fi

printf '%s\n' "production origin and directory redirect smoke passed"
