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
platforms = {
    "iphone": ("safari",),
    "ipad": ("safari",),
    "android-phone": ("chrome", "firefox", "samsung-internet"),
    "android-tablet": ("chrome",),
}
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

for platform, browser_families in platforms.items():
    for browser_family in browser_families:
        for cache_state in cache_states:
            for minimum_version_run in ("no", "yes"):
                for run_number in range(1, 4):
                    lifecycle = cache_state == "lifecycle"
                    report = {
                        "schema_version": 1,
                        "captured_at": "2026-08-17T00:00:00Z",
                        "candidate": {
                            "build_id": "fixture-build",
                            "source_hash": "0" * 64,
                            "bevy": "0.19.1",
                            "renderer": "WebGL2",
                        },
                        "test": {
                            "platform": platform,
                            "hardware_model": f"fixture-{platform}",
                            "os_version": "fixture-os",
                            "browser_family": browser_family,
                            "browser_version": "fixture-browser",
                            "cache_state": cache_state,
                            "minimum_version_run": minimum_version_run,
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
                        "browser": {
                            "declared_family": browser_family,
                            "detected_family": browser_family,
                            "user_agent": "fixture-user-agent",
                        },
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
                            "page_hide_count": 2 if lifecycle else 0,
                            "page_show_count": 2 if lifecycle else 1,
                            "restored_from_page_cache": False,
                        },
                        "audio": {
                            "state": "running",
                            "muted": False,
                            "gestureStarts": 2,
                            "backgroundSuspensions": 1 if lifecycle else 0,
                            "explicitResumes": 1 if lifecycle else 0,
                        },
                    }
                    version_label = "minimum" if minimum_version_run == "yes" else "current"
                    path = output / (
                        f"{platform}-{browser_family}-{cache_state}-{version_label}-{run_number}.json"
                    )
                    path.write_text(json.dumps(report), encoding="utf-8")
PY

reports="$tmp"/*.json
"$root/scripts/summarize-feasibility.py" --require-mobile-matrix $reports >"$tmp/summary.md"
grep -q '| iphone / fixture-iphone / fixture-buil / Bevy 0.19.1 WebGL2 |' "$tmp/summary.md"
grep -q '| android-tablet / fixture-android-tablet / fixture-buil / Bevy 0.19.1 WebGL2 |' "$tmp/summary.md"

expect_rejected() {
    label=$1
    shift
    if "$root/scripts/summarize-feasibility.py" --require-mobile-matrix "$@" >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
        printf '%s\n' "feasibility summarizer accepted $label" >&2
        exit 1
    fi
}

expect_rejected "an incomplete matrix" "$tmp"/iphone-*.json "$tmp"/ipad-*.json "$tmp"/android-phone-*.json

if "$root/scripts/summarize-feasibility.py" --allow-incomplete --require-mobile-matrix $reports >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
    printf '%s\n' "feasibility summarizer allowed incomplete final-gate mode" >&2
    exit 1
fi
grep -q 'cannot be combined' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
del report["physical_checks"]["audio"]
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a missing required physical check" $reports
grep -q 'missing audio' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["physical_checks"]["audio"] = "pass"
report["candidate"]["source_hash"] = "not-a-hash"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "an invalid candidate source hash" $reports
grep -q 'invalid candidate.source_hash' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["candidate"]["source_hash"] = "0" * 64
report["browser"]["detected_family"] = "chrome"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a mismatched detected browser family" $reports
grep -q 'detected browser family does not match' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" "$tmp/iphone-safari-lifecycle-minimum-1.json" "$tmp/iphone-safari-lifecycle-minimum-2.json" "$tmp/iphone-safari-lifecycle-minimum-3.json" <<'PY'
import json
import sys
from pathlib import Path

check_path = Path(sys.argv[1])
check_report = json.loads(check_path.read_text(encoding="utf-8"))
check_report["physical_checks"]["audio"] = "pass"
check_report["candidate"]["source_hash"] = "0" * 64
check_report["browser"]["detected_family"] = "safari"
check_path.write_text(json.dumps(check_report), encoding="utf-8")

for value in sys.argv[2:]:
    path = Path(value)
    report = json.loads(path.read_text(encoding="utf-8"))
    report["test"]["minimum_version_run"] = "no"
    path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "missing minimum-version evidence" $reports
grep -q 'mobile matrix missing iphone / safari / lifecycle / minimum' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-lifecycle-minimum-1.json" "$tmp/iphone-safari-lifecycle-minimum-2.json" "$tmp/iphone-safari-lifecycle-minimum-3.json" <<'PY'
import json
import sys
from pathlib import Path

for value in sys.argv[1:]:
    path = Path(value)
    report = json.loads(path.read_text(encoding="utf-8"))
    report["test"]["minimum_version_run"] = "yes"
    path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/android-tablet-chrome-cold-current-1.json" <<'PY'
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
python3 - "$tmp/android-tablet-chrome-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["performance"]["minimum_one_second_fps"] = 58
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-warm-current-1.json" <<'PY'
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

python3 - "$tmp/iphone-safari-warm-current-1.json" "$tmp/ipad-safari-lifecycle-current-1.json" <<'PY'
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

python3 - "$tmp/ipad-safari-lifecycle-current-1.json" "$tmp/android-phone-chrome-lifecycle-current-1.json" <<'PY'
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

python3 - "$tmp/android-phone-chrome-lifecycle-current-1.json" "$tmp/android-tablet-chrome-warm-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

resume = Path(sys.argv[1])
resume_report = json.loads(resume.read_text(encoding="utf-8"))
resume_report["audio"]["explicitResumes"] = 1
resume.write_text(json.dumps(resume_report), encoding="utf-8")

mute = Path(sys.argv[2])
mute_report = json.loads(mute.read_text(encoding="utf-8"))
mute_report["audio"]["gestureStarts"] = 1
mute.write_text(json.dumps(mute_report), encoding="utf-8")
PY
expect_rejected "incomplete pre/post-mute playback" $reports
grep -q 'pre/post-mute playback' "$tmp/rejected.err"

python3 - "$tmp/android-tablet-chrome-warm-current-1.json" "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

mute = Path(sys.argv[1])
mute_report = json.loads(mute.read_text(encoding="utf-8"))
mute_report["audio"]["gestureStarts"] = 2
mute.write_text(json.dumps(mute_report), encoding="utf-8")

duration = Path(sys.argv[2])
duration_report = json.loads(duration.read_text(encoding="utf-8"))
duration_report["timing_ms"]["capture_duration"] = 59_999
duration.write_text(json.dumps(duration_report), encoding="utf-8")
PY
expect_rejected "short interaction capture" $reports
grep -q 'shorter than 1 minute' "$tmp/rejected.err"

printf '%s\n' "feasibility tool self-tests passed"
