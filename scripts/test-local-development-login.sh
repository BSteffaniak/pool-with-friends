#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
port=${PWMTF_LOCAL_LOGIN_TEST_PORT:-4397}
tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-local-login.XXXXXX")
server_pid=
cleanup() {
    if [ -n "$server_pid" ]; then
        kill -TERM "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    rm -rf "$tmp"
}
trap cleanup EXIT HUP INT TERM

cargo build -p pwmtf_server --features insecure --bin pwmtf-server
PWMTF_DEV_MODE=true \
PWMTF_CANONICAL_ORIGIN="http://127.0.0.1:$port" \
PWMTF_BIND="127.0.0.1:$port" \
PWMTF_DATABASE_PATH="$tmp/pwmtf.db" \
PWMTF_WEB_ROOT="$root/dist" \
    "$root/target/debug/pwmtf-server" >"$tmp/server.log" 2>&1 &
server_pid=$!

ready=false
for _ in $(seq 1 80); do
    if curl --fail --silent --show-error "http://127.0.0.1:$port/readyz" >/dev/null 2>&1; then
        ready=true
        break
    fi
    sleep 0.25
done
if [ "$ready" != true ]; then
    cat "$tmp/server.log" >&2
    exit 1
fi

curl --fail --silent --show-error "http://127.0.0.1:$port/api/auth/runtime" \
    | jq -e '.development_login == true and .google_login == false' >/dev/null
status=$(curl --silent --show-error --output "$tmp/google.body" --write-out '%{http_code}' \
    --request POST --header "Origin: http://127.0.0.1:$port" \
    "http://127.0.0.1:$port/auth/google/start")
[ "$status" = 404 ]

curl --fail --silent --show-error --dump-header "$tmp/headers" --output /dev/null \
    --cookie-jar "$tmp/cookies" --request POST \
    --header "Origin: http://127.0.0.1:$port" \
    --header "X-PWMTF-Origin: http://127.0.0.1:$port" \
    --header 'Content-Type: application/json' \
    --data '{"username":"local_one"}' \
    "http://127.0.0.1:$port/auth/development"
grep -qi '^set-cookie: pwmtf_dev_session=' "$tmp/headers"
if grep -qi '^set-cookie: .*Secure' "$tmp/headers"; then
    printf '%s\n' "development session cookie unexpectedly used Secure" >&2
    exit 1
fi
curl --fail --silent --show-error --cookie "$tmp/cookies" \
    "http://127.0.0.1:$port/api/session" \
    | jq -e '.handle == "local_one" and .development_login == true' >/dev/null

curl --fail --silent --show-error --dump-header "$tmp/headers-two" --output /dev/null \
    --cookie-jar "$tmp/cookies-two" --request POST \
    --header "Origin: http://127.0.0.1:$port" \
    --header "X-PWMTF-Origin: http://127.0.0.1:$port" \
    --header 'Content-Type: application/json' \
    --data '{"username":"local_two"}' \
    "http://127.0.0.1:$port/auth/development"
curl --fail --silent --show-error --cookie "$tmp/cookies-two" \
    "http://127.0.0.1:$port/api/session" \
    | jq -e '.handle == "local_two" and .development_login == true' >/dev/null

curl --fail --silent --show-error --cookie "$tmp/cookies" --request POST \
    --header "Origin: http://127.0.0.1:$port" \
    --header "X-PWMTF-Origin: http://127.0.0.1:$port" \
    --header 'Content-Type: application/json' \
    --data '{"handle":"local_two"}' \
    "http://127.0.0.1:$port/api/challenges" \
    | jq -e '.challenge_id | strings' >/dev/null
curl --fail --silent --show-error --cookie "$tmp/cookies-two" \
    "http://127.0.0.1:$port/api/challenges" \
    | jq -e 'length == 1 and .[0].from_handle == "local_one"' >/dev/null

invitation_url=$(curl --fail --silent --show-error --cookie "$tmp/cookies" \
    --request POST --header "Origin: http://127.0.0.1:$port" \
    --header "X-PWMTF-Origin: http://127.0.0.1:$port" \
    "http://127.0.0.1:$port/api/invitations" | jq -r '.invitation_url')
invitation_token=${invitation_url#*?invite=}
lobby_id=$(curl --fail --silent --show-error --cookie "$tmp/cookies-two" \
    --request POST --header "Origin: http://127.0.0.1:$port" \
    --header "X-PWMTF-Origin: http://127.0.0.1:$port" \
    --header 'Content-Type: application/json' \
    --data "{\"token\":\"$invitation_token\"}" \
    "http://127.0.0.1:$port/api/invitations/redeem" | jq -r '.lobby_id')
curl --fail --silent --show-error --cookie "$tmp/cookies" \
    "http://127.0.0.1:$port/api/lobbies" \
    | jq -e --arg lobby "$lobby_id" 'any(.[]; .lobby_id == $lobby and .status == "waiting")' >/dev/null
connection_one=$(curl --fail --silent --show-error --cookie "$tmp/cookies" \
    --request POST --header "Origin: http://127.0.0.1:$port" \
    --header "X-PWMTF-Origin: http://127.0.0.1:$port" \
    "http://127.0.0.1:$port/api/lobbies/$lobby_id" | jq -r '.connection_id')
connection_two=$(curl --fail --silent --show-error --cookie "$tmp/cookies-two" \
    --request POST --header "Origin: http://127.0.0.1:$port" \
    --header "X-PWMTF-Origin: http://127.0.0.1:$port" \
    "http://127.0.0.1:$port/api/lobbies/$lobby_id" | jq -r '.connection_id')
[ -n "$connection_one" ]
[ -n "$connection_two" ]
curl --fail --silent --show-error --cookie "$tmp/cookies" \
    --request POST --header "Origin: http://127.0.0.1:$port" \
    --header "X-PWMTF-Origin: http://127.0.0.1:$port" \
    "http://127.0.0.1:$port/api/lobbies/$lobby_id/ready" >/dev/null
match_id=$(curl --fail --silent --show-error --cookie "$tmp/cookies-two" \
    --request POST --header "Origin: http://127.0.0.1:$port" \
    --header "X-PWMTF-Origin: http://127.0.0.1:$port" \
    "http://127.0.0.1:$port/api/lobbies/$lobby_id/ready" | jq -r '.match_id')
[ "$match_id" != null ]

printf '%s\n' "local username login, social discovery, lobby readiness, and match start acceptance passed"
