#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-feasibility-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

python3 - "$tmp" <<'PY'
import json
import sys
from pathlib import Path

output = Path(sys.argv[1])
platforms = ("iphone", "ipad", "android-phone", "android-tablet")
cache_states = ("cold", "warm", "lifecycle")
checks = {
    "first_load": "pass",
    "aiming": "pass",
    "power": "pass",
    "touch_reset": "pass",
    "resize": "pass",
    "safe_area": "pass",
    "orientation": "pass",
    "background": "pass",
    "audio": "pass",
    "input_response": "pass",
    "memory": "pass",
    "thermal": "pass",
}

for platform in platforms:
    for cache_state in cache_states:
        for run_number in range(1, 4):
            lifecycle = cache_state == "lifecycle"
            report = {
                "schema_version": 1,
                "candidate": {"bevy": "0.19.1", "renderer": "WebGL2"},
                "test": {
                    "platform": platform,
                    "hardware_model": f"fixture-{platform}",
                    "os_version": "fixture-os",
                    "browser_version": "fixture-browser",
                    "cache_state": cache_state,
                    "presentation_tier": "default",
                    "run_number": run_number,
                },
                "physical_checks": checks,
                "external_observations": {
                    "first_visible_table_ms": 4_000 if cache_state == "cold" else 1_500,
                    "first_accepted_input_ms": 4_200 if cache_state == "cold" else 1_700,
                    "steady_memory_mib": 120,
                    "peak_memory_mib": 180,
                    "thermal_result": "no-warning",
                    "reload_or_eviction_observed": "no",
                },
                "browser": {},
                "display": {},
                "timing_ms": {
                    "client_ready": 1_000,
                    "first_canvas_contact": 1_200,
                    "capture_duration": 610_000 if lifecycle else 65_000,
                },
                "performance": {
                    "minimum_one_second_fps": 58,
                    "p95_frame_time_ms": 18,
                },
                "interaction": {
                    "visibility_changes": 4 if lifecycle else 0,
                    "orientation_changes": 2 if lifecycle else 0,
                },
                "audio": {
                    "gestureStarts": 1,
                    "backgroundSuspensions": 1 if lifecycle else 0,
                    "explicitResumes": 1 if lifecycle else 0,
                },
            }
            path = output / f"{platform}-{cache_state}-{run_number}.json"
            path.write_text(json.dumps(report), encoding="utf-8")
PY

reports="$tmp"/*.json
"$root/scripts/summarize-feasibility.py" --require-mobile-matrix $reports >"$tmp/summary.md"
grep -q '| iphone / fixture-iphone |' "$tmp/summary.md"
grep -q '| android-tablet / fixture-android-tablet |' "$tmp/summary.md"

expect_rejected() {
    label=$1
    shift
    if "$root/scripts/summarize-feasibility.py" --require-mobile-matrix "$@" >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
        printf '%s\n' "feasibility summarizer accepted $label" >&2
        exit 1
    fi
}

expect_rejected "an incomplete matrix" "$tmp"/iphone-*.json "$tmp"/ipad-*.json "$tmp"/android-phone-*.json

python3 - "$tmp/android-tablet-cold-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["performance"]["minimum_one_second_fps"] = 29
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "sub-floor frame rate" $reports
grep -q 'below 30' "$tmp/rejected.err"

# Restore the passing fixture before testing independent rejection paths.
python3 - "$tmp/android-tablet-cold-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["performance"]["minimum_one_second_fps"] = 58
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-warm-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["external_observations"]["first_visible_table_ms"] = 3_001
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "warm startup over budget" $reports
grep -q 'exceeds 3000ms' "$tmp/rejected.err"

python3 - "$tmp/iphone-warm-1.json" "$tmp/ipad-lifecycle-1.json" <<'PY'
import json
import sys
from pathlib import Path

warm = Path(sys.argv[1])
warm_report = json.loads(warm.read_text(encoding="utf-8"))
warm_report["external_observations"]["first_visible_table_ms"] = 1_500
warm.write_text(json.dumps(warm_report), encoding="utf-8")

lifecycle = Path(sys.argv[2])
lifecycle_report = json.loads(lifecycle.read_text(encoding="utf-8"))
lifecycle_report["timing_ms"]["capture_duration"] = 599_999
lifecycle.write_text(json.dumps(lifecycle_report), encoding="utf-8")
PY
expect_rejected "short lifecycle capture" $reports
grep -q 'shorter than 10 minutes' "$tmp/rejected.err"

python3 - "$tmp/ipad-lifecycle-1.json" "$tmp/android-phone-lifecycle-1.json" <<'PY'
import json
import sys
from pathlib import Path

short = Path(sys.argv[1])
short_report = json.loads(short.read_text(encoding="utf-8"))
short_report["timing_ms"]["capture_duration"] = 610_000
short.write_text(json.dumps(short_report), encoding="utf-8")

audio = Path(sys.argv[2])
audio_report = json.loads(audio.read_text(encoding="utf-8"))
audio_report["audio"]["explicitResumes"] = 0
audio.write_text(json.dumps(audio_report), encoding="utf-8")
PY
expect_rejected "missing explicit audio resume" $reports
grep -q 'explicit audio resume was not observed' "$tmp/rejected.err"

printf '%s\n' "feasibility tool self-tests passed"
