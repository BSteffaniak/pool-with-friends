#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-production-smoke-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

cat >"$tmp/curl" <<'SH'
#!/bin/sh
set -eu
headers=
body=
url=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --dump-header)
            headers=$2
            shift 2
            ;;
        --output)
            body=$2
            shift 2
            ;;
        --connect-timeout|--max-time|--max-redirs)
            shift 2
            ;;
        --fail|--silent|--show-error)
            shift
            ;;
        *)
            url=$1
            shift
            ;;
    esac
done
case "$url" in
    https://pwmtf.hyperchad.dev/healthz)
        printf 'HTTP/1.1 200 OK\r\nStrict-Transport-Security: max-age=31536000\r\nX-Content-Type-Options: nosniff\r\n\r\n' >"$headers"
        printf ok >"$body"
        ;;
    https://pwmtf.hyperchad.dev/readyz)
        printf 'HTTP/1.1 200 OK\r\nStrict-Transport-Security: max-age=31536000\r\nX-Content-Type-Options: nosniff\r\n\r\n' >"$headers"
        printf ready >"$body"
        ;;
    https://pwmtf.hyperchad.dev/)
        printf 'HTTP/1.1 200 OK\r\nContent-Security-Policy: default-src self\r\nX-Content-Type-Options: nosniff\r\n\r\n' >"$headers"
        printf '%s' '<link rel="canonical" href="https://pwmtf.hyperchad.dev/" /><main data-client-state="loading">' >"$body"
        ;;
    https://hyperchad.dev/games/pool-with-more-than-friends)
        printf 'HTTP/1.1 %s Redirect\r\nLocation: %s\r\n\r\n' "${PWMTF_FAKE_REDIRECT_STATUS:-308}" "${PWMTF_FAKE_REDIRECT_TARGET:-https://pwmtf.hyperchad.dev}" >"$headers"
        : >"$body"
        ;;
    *)
        printf '%s\n' "unexpected fake curl URL: $url" >&2
        exit 1
        ;;
esac
SH
chmod +x "$tmp/curl"

PWMTF_CURL_BIN="$tmp/curl" "$root/scripts/test-production-smoke.sh" >"$tmp/pass.out"
grep -q 'production origin and directory redirect smoke passed' "$tmp/pass.out"

if PWMTF_CURL_BIN="$tmp/curl" PWMTF_FAKE_REDIRECT_STATUS=302 \
    "$root/scripts/test-production-smoke.sh" >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
    printf '%s\n' "production smoke accepted a temporary redirect" >&2
    exit 1
fi
grep -q 'returned 302 instead of 308' "$tmp/rejected.err"

if PWMTF_CURL_BIN="$tmp/curl" PWMTF_FAKE_REDIRECT_TARGET=https://evil.example \
    "$root/scripts/test-production-smoke.sh" >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
    printf '%s\n' "production smoke accepted a noncanonical redirect target" >&2
    exit 1
fi
grep -q 'target is not the canonical origin' "$tmp/rejected.err"

if PWMTF_CURL_BIN="$tmp/curl" PWMTF_CANONICAL_ORIGIN=https://evil.example \
    "$root/scripts/test-production-smoke.sh" >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
    printf '%s\n' "production smoke accepted a noncanonical application origin" >&2
    exit 1
fi
grep -q 'must be the canonical production origin' "$tmp/rejected.err"

printf '%s\n' "production smoke self-tests passed"
