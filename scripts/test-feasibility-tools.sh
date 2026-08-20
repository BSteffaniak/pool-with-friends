#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

if [ "${PWMTF_WASM_BUNDLE_LOCKED:-0}" != 1 ]; then
    exec "$root/scripts/with-wasm-bundle-lock.py" -- "$0" "$@"
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-feasibility-test.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

if [ ! -f "$root/dist/pwmtf-bundle-manifest.json" ] || \
   ! grep -q '^const candidateWasmOptimization = "wasm-opt-Oz";$' "$root/dist/bootstrap.js"; then
    "$root/scripts/build-wasm.sh"
fi
mkdir -p "$tmp/expected-bundle"
cp -R "$root/dist/." "$tmp/expected-bundle/"

python3 - "$tmp" "$tmp/expected-bundle/pwmtf-bundle-manifest.json" <<'PY'
import json
import sys
from pathlib import Path

output = Path(sys.argv[1])
manifest = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
candidate = {
    **manifest["candidate"],
    "bundle_hash_algorithm": manifest["bundle_hash_algorithm"],
    "bevy": "0.19.1",
    "renderer": "WebGL2",
}
platforms = {
    "iphone": ("safari",),
    "ipad": ("safari",),
    "android-phone": ("chrome", "firefox", "samsung-internet"),
    "android-tablet": ("chrome",),
    "desktop": ("safari", "chrome", "firefox", "edge"),
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
    "browser_chrome": "pass",
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
                    browser_version = "151" if minimum_version_run == "no" else "150"
                    report = {
                        "schema_version": 11,
                        "captured_at": (
                            f"2026-08-{run_number + (10 if minimum_version_run == 'yes' else 0):02d}"
                            f"T00:00:00.000Z"
                        ),
                        "candidate": candidate,
                        "test": {
                            "platform": platform,
                            "hardware_model": f"fixture-{platform}",
                            "os_version": "fixture-os",
                            "browser_family": browser_family,
                            "browser_version": browser_version,
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
                            "detected_platform": platform,
                            "declared_version": browser_version,
                            "detected_version": browser_version,
                            "user_agent": "fixture-user-agent",
                            "language": "en-US",
                            "hardware_concurrency": 8,
                            "device_memory_gib": 8,
                        },
                        "display": {
                            "screen_width": 1280,
                            "screen_height": 720,
                            "viewport_width": 1280,
                            "viewport_height": 720,
                            "device_pixel_ratio": 2,
                            "orientation": "landscape-primary",
                        },
                        "timing_ms": {
                            "client_ready": 1_000,
                            "first_canvas_contact": 1_200,
                            "capture_duration": 610_000 if lifecycle else 65_000,
                            "hidden_duration": 60_000 if lifecycle else 0,
                            "hidden_durations": [30_000, 30_000] if lifecycle else [],
                        },
                        "performance": {
                            "frame_count": 3_600,
                            "current_fps": 60,
                            "minimum_one_second_fps": 58,
                            "median_one_second_fps": 60,
                            "maximum_one_second_fps": 61,
                            "median_frame_time_ms": 16.7,
                            "p95_frame_time_ms": 18,
                            "p99_frame_time_ms": 20,
                            "maximum_frame_gap_ms": 24,
                            "current_js_heap_bytes": None,
                            "peak_js_heap_bytes": None,
                        },
                        "interaction": {
                            "canvas_contacts": 8,
                            "visibility_changes": 4 if lifecycle else 0,
                            "orientation_changes": 2 if lifecycle else 0,
                            "initial_orientation": "landscape-primary",
                            "final_orientation": "landscape-primary",
                            "orientation_states": ["portrait-primary", "landscape-primary"] if lifecycle else [],
                            "page_hide_count": 2 if lifecycle else 0,
                            "page_show_count": 2 if lifecycle else 0,
                            "restored_from_page_cache": False,
                            "capture_started_visible": True,
                            "capture_stopped_visible": True,
                            "marked_events": [],
                        },
                        "audio": {
                            "supported": True,
                            "state": "running",
                            "muted": False,
                            "gestureStarts": 2,
                            "backgroundSuspensions": 2 if lifecycle else 0,
                            "explicitResumes": 2 if lifecycle else 0,
                            "muteChanges": 2,
                            "mutedPlaybackAttempts": 1,
                            "audiblePlaybackAttempts": 1,
                            "transitionFailures": 0,
                        },
                    }
                    version_label = "minimum" if minimum_version_run == "yes" else "current"
                    path = output / (
                        f"{platform}-{browser_family}-{cache_state}-{version_label}-{run_number}.json"
                    )
                    path.write_text(json.dumps(report), encoding="utf-8")
PY

reports="$tmp"/*.json
python3 - "$root/scripts/summarize-feasibility.py" <<'PY'
import importlib.util
import sys
from pathlib import Path

path = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location("summarize_feasibility", path)
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
sys.path.insert(0, str(path.parent))
spec.loader.exec_module(module)
assert module.browser_version("151.0.7922.138") == (151, 0, 7922, 138)
assert module.browser_version("١٥١") is None
PY
if "$root/scripts/summarize-feasibility.py" --write-decision "$tmp/invalid-decision.md" $reports >"$tmp/decision.out" 2>"$tmp/decision.err"; then
    printf '%s\n' "feasibility summarizer wrote a decision without final-matrix mode" >&2
    exit 1
fi
grep -q -- '--write-decision requires --require-mobile-matrix' "$tmp/decision.err"
if [ -e "$tmp/invalid-decision.md" ]; then
    printf '%s\n' "failed decision validation created an output file" >&2
    exit 1
fi
if PWMTF_FEASIBILITY_REPORTS="$tmp" PWMTF_FEASIBILITY_BUNDLE="$tmp/expected-bundle" \
    "$root/scripts/validate-feasibility-matrix.sh" unexpected >"$tmp/wrapper.out" 2>"$tmp/wrapper.err"; then
    printf '%s\n' "final feasibility wrapper accepted positional arguments" >&2
    exit 1
fi
grep -q '^usage:' "$tmp/wrapper.err"
mkdir -p "$tmp/evidence"
PWMTF_FEASIBILITY_REPORTS="$tmp" PWMTF_FEASIBILITY_BUNDLE="$tmp/expected-bundle" \
    PWMTF_FEASIBILITY_DECISION="$tmp/evidence/acceptance-decision.md" \
    PWMTF_FEASIBILITY_SIZE_EVIDENCE="$tmp/evidence/wasm-size-evidence.json" \
    "$root/scripts/validate-feasibility-matrix.sh" >"$tmp/wrapper-summary.md"
grep -q '^\*\*Status: ACCEPT — Bevy/WebGL2 is selected for the production browser client\.\*\*$' "$tmp/evidence/acceptance-decision.md"
grep -q '^- Evidence completed: 2026-08-13T00:00:00.000Z$' "$tmp/evidence/acceptance-decision.md"
grep -Eq '^- Report set: `[0-9a-f]{64}` \(`sha256-canonical-json-length-prefixed-v1`\)$' "$tmp/evidence/acceptance-decision.md"
grep -q '^  - iphone / safari: 150$' "$tmp/evidence/acceptance-decision.md"
grep -q '^  - desktop / edge: 150$' "$tmp/evidence/acceptance-decision.md"
python3 - "$tmp/evidence/wasm-size-evidence.json" "$tmp/expected-bundle/pwmtf-bundle-manifest.json" <<'PY'
import json
import sys
from pathlib import Path

size = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
manifest = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
assert size["schema_version"] == 1
assert size["candidate"]["bundle_hash"] == manifest["candidate"]["bundle_hash"]
assert size["candidate"]["wasm_optimization"] == "wasm-opt-Oz"
assert size["totals"]["raw_bytes"] > 0
assert size["totals"]["gzip_bytes"] > 0
PY
if PWMTF_FEASIBILITY_REPORTS="$tmp" PWMTF_FEASIBILITY_BUNDLE="$tmp/expected-bundle" \
    PWMTF_FEASIBILITY_DECISION="$tmp/evidence/acceptance-decision.md" \
    PWMTF_FEASIBILITY_SIZE_EVIDENCE="$tmp/replacement-size-evidence.json" \
    "$root/scripts/validate-feasibility-matrix.sh" >"$tmp/existing.out" 2>"$tmp/existing.err"; then
    printf '%s\n' "final feasibility wrapper overwrote an existing acceptance decision" >&2
    exit 1
fi
grep -q 'acceptance decision already exists' "$tmp/existing.err"
if "$root/scripts/summarize-feasibility.py" --require-mobile-matrix $reports >"$tmp/unbound.out" 2>"$tmp/unbound.err"; then
    printf '%s\n' "final feasibility validation accepted reports without --expected-bundle" >&2
    exit 1
fi
grep -q -- '--require-mobile-matrix requires --expected-bundle' "$tmp/unbound.err"
"$root/scripts/summarize-feasibility.py" --require-mobile-matrix --expected-bundle "$tmp/expected-bundle" $reports >"$tmp/summary.md"
cmp "$tmp/summary.md" "$tmp/wrapper-summary.md"
grep -q '| iphone / fixture-iphone / ' "$tmp/summary.md"
grep -q '| android-tablet / fixture-android-tablet / ' "$tmp/summary.md"
grep -q '| desktop / fixture-desktop / ' "$tmp/summary.md"

"$root/scripts/summarize-feasibility.py" \
    --require-mobile-matrix \
    --expected-bundle "$tmp/expected-bundle" \
    $reports >"$tmp/bound-summary.md"
cmp "$tmp/summary.md" "$tmp/bound-summary.md"
"$root/scripts/summarize-feasibility.py" \
    --require-mobile-matrix \
    --expected-bundle "$tmp/expected-bundle" \
    --write-decision "$tmp/evidence/second-decision.md" \
    $(printf '%s\n' $reports | sort -r) >"$tmp/reordered-summary.md"
cmp "$tmp/wrapper-summary.md" "$tmp/reordered-summary.md"
cmp "$tmp/evidence/acceptance-decision.md" "$tmp/evidence/second-decision.md"

expect_rejected() {
    label=$1
    shift
    if "$root/scripts/summarize-feasibility.py" \
        --require-mobile-matrix \
        --expected-bundle "$tmp/expected-bundle" \
        "$@" >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
        printf '%s\n' "feasibility summarizer accepted $label" >&2
        exit 1
    fi
}

python3 - "$tmp/expected-bundle/pwmtf-bundle-manifest.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["candidate"]["bundle_hash"] = "2" * 64
path.write_text(json.dumps(manifest), encoding="utf-8")
PY
expect_rejected "reports from a different generated bundle" $reports
grep -q 'expected bundle verification failed: WASM bundle integrity error: bundle manifest candidate identity does not match bootstrap.js' "$tmp/rejected.err"

python3 - "$tmp/expected-bundle/pwmtf-bundle-manifest.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["candidate"]["bundle_hash"] = "1" * 64
path.write_text(json.dumps(manifest), encoding="utf-8")
PY

python3 - "$tmp/expected-bundle/pwmtf-bundle-manifest.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["bundle_hash_algorithm"] = "unknown"
path.write_text(json.dumps(manifest), encoding="utf-8")
PY
expect_rejected "an unsupported expected bundle hash algorithm" $reports
grep -q 'expected bundle verification failed: WASM bundle integrity error: bundle hash algorithm is unsupported' "$tmp/rejected.err"
python3 - "$tmp/expected-bundle/pwmtf-bundle-manifest.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
manifest = json.loads(path.read_text(encoding="utf-8"))
manifest["bundle_hash_algorithm"] = "sha256-length-prefixed-v1"
path.write_text(json.dumps(manifest), encoding="utf-8")
PY

mv "$tmp/expected-bundle/pwmtf-bundle-manifest.json" "$tmp/expected-bundle/manifest.json"
ln -s "$tmp/expected-bundle/manifest.json" "$tmp/expected-bundle/pwmtf-bundle-manifest.json"
expect_rejected "a symlinked expected bundle manifest" $reports
grep -q 'expected bundle verification failed: WASM bundle integrity error: bundle manifest must be a regular file' "$tmp/rejected.err"
rm "$tmp/expected-bundle/pwmtf-bundle-manifest.json"
mv "$tmp/expected-bundle/manifest.json" "$tmp/expected-bundle/pwmtf-bundle-manifest.json"
cp "$root/dist/pwmtf-bundle-manifest.json" "$tmp/expected-bundle/pwmtf-bundle-manifest.json"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["schema_version"] = 1
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "the superseded report schema" $reports
grep -q 'unsupported schema_version' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["schema_version"] = 11
report["candidate"]["wasm_optimization"] = "not-applied"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a final matrix without Binaryen optimization" $reports
grep -q 'requires the wasm-opt-Oz candidate' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["candidate"]["wasm_optimization"] = "wasm-opt-Oz"
path.write_text(json.dumps(report), encoding="utf-8")
PY

expect_rejected "an incomplete matrix" "$tmp"/iphone-*.json "$tmp"/ipad-*.json "$tmp"/android-phone-*.json

if "$root/scripts/summarize-feasibility.py" \
    --allow-incomplete \
    --require-mobile-matrix \
    --expected-bundle "$tmp/expected-bundle" \
    $reports >"$tmp/rejected.out" 2>"$tmp/rejected.err"; then
    printf '%s\n' "feasibility summarizer allowed incomplete final-gate mode" >&2
    exit 1
fi
grep -q 'cannot be combined' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-2.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["captured_at"] = "2026-08-01T00:00:00.000Z"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "duplicate capture timestamps" $reports
grep -q 'duplicate capture timestamps' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-2.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["captured_at"] = "2026-08-02T00:00:00.000Z"
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-cold-current-2.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["captured_at"] = "2026-06-01T00:00:00.000Z"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a final matrix collected over too long a window" $reports
grep -q 'span more than 30 days' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-2.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["captured_at"] = "2026-08-02T00:00:00.000Z"
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["captured_at"] = "2999-01-01T00:00:00.000Z"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a future capture timestamp" $reports
grep -q 'more than five minutes in the future' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["captured_at"] = "2026-08-01T00:00:00.000Z"
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["captured_at"] = "2026-08-01T00:00:00.000+01:00"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a non-UTC capture timestamp" $reports
grep -q 'captured_at must be normalized to UTC' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["captured_at"] = "2026-08-01T00:00:00Z"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a non-canonical UTC capture timestamp" $reports
grep -q 'captured_at must use canonical UTC millisecond spelling' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["captured_at"] = "2026-08-01T00:00:00.000Z"
path.write_text(json.dumps(report), encoding="utf-8")
PY

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
report["candidate"]["bundle_hash"] = "not-a-hash"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "an invalid candidate bundle hash" $reports
grep -q 'invalid candidate.bundle_hash' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["candidate"]["bundle_hash"] = "1" * 64
report["candidate"]["bundle_hash_algorithm"] = "unknown"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "an unsupported candidate bundle hash algorithm" $reports
grep -q 'unsupported candidate.bundle_hash_algorithm' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["candidate"]["bundle_hash_algorithm"] = "sha256-length-prefixed-v1"
report["candidate"]["bundle_hash"] = "1" * 64
report["candidate"]["source_hash"] = "0" * 64
report["candidate"]["build_id"] = "unbound-build"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a build ID not bound to its source hash" $reports
grep -q 'build_id must include candidate.source_hash' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["candidate"]["build_id"] = "fixture-build-" + report["candidate"]["source_hash"]
report["timing_ms"]["capture_duration"] = 0
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a zero-duration capture" $reports
grep -q 'capture_duration must be greater than zero' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["timing_ms"]["capture_duration"] = 65_000
report["timing_ms"]["first_canvas_contact"] = 999
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "canvas contact before client readiness" $reports
grep -q 'first canvas contact cannot precede client readiness' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["timing_ms"]["first_canvas_contact"] = 1_200
report["performance"]["p95_frame_time_ms"] = 21
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "non-monotonic frame percentiles" $reports
grep -q 'p95 frame time cannot exceed p99' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["performance"]["p95_frame_time_ms"] = 18
report["performance"]["current_fps"] = -1
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "negative current FPS" $reports
grep -q 'performance.current_fps must be null or non-negative and finite' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["performance"]["current_fps"] = 60
report["performance"]["frame_count"] = 3600.5
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a fractional frame count" $reports
grep -q 'performance.frame_count must be a positive integer' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["performance"]["frame_count"] = 3600
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["performance"]["current_fps"] = 60
report["display"]["orientation"] = ""
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a missing display orientation" $reports
grep -q 'display.orientation must be a non-empty string' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["display"]["orientation"] = "landscape-primary"
report["display"]["viewport_width"] = 1280.5
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a fractional viewport dimension" $reports
grep -q 'display.viewport_width must be a positive integer' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["display"]["viewport_width"] = 1280
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["display"]["orientation"] = "landscape-primary"
report["interaction"]["initial_orientation"] = "x" * 81
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "an oversized initial orientation" $reports
grep -q 'invalid interaction.initial_orientation' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["interaction"]["initial_orientation"] = "landscape-primary"
report["interaction"]["marked_events"] = [
    {
        "elapsed_ms": 10,
        "label": "fixture",
        "visibility": "visible",
        "orientation": "bad\norientation",
    }
]
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "control characters in marked-event orientation" $reports
grep -q 'invalid marked event orientation' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["interaction"]["marked_events"][0]["orientation"] = "landscape-primary"
report["interaction"]["marked_events"][0]["visibility"] = "hidden"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a hidden marked event" $reports
grep -q 'marked events must be recorded while visible' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["interaction"]["marked_events"][0]["visibility"] = "visible"
report["interaction"]["marked_events"][0]["orientation"] = None
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a marked event without orientation" $reports
grep -q 'marked events require a known orientation' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["interaction"]["marked_events"] = []
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["performance"]["current_fps"] = 60
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["browser"]["user_agent"] = "x" * 1_025
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "an oversized user agent" $reports
grep -q 'browser.user_agent is invalid' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["browser"]["user_agent"] = "fixture-user-agent"
report["browser"]["language"] = "en-US\ninvalid"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "control characters in browser language" $reports
grep -q 'browser.language contains control characters' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["browser"]["language"] = "en-US"
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["timing_ms"]["capture_duration"] = 65_000
report["timing_ms"]["first_canvas_contact"] = 1_200
report["performance"]["p95_frame_time_ms"] = 18
report["test"]["hardware_model"] = "bad\nmodel"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "control characters in test metadata" $reports
grep -q 'hardware_model contains control characters' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["test"]["hardware_model"] = " fixture-iphone "
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "non-canonical whitespace in test metadata" $reports
grep -q 'invalid test.hardware_model' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["test"]["hardware_model"] = "fixture-iphone"
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["test"]["hardware_model"] = "fixture-iphone"
report["external_observations"]["steady_memory_mib"] = 0
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "zero memory evidence" $reports
grep -q 'memory observations must be greater than zero' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["test"]["hardware_model"] = "fixture-iphone"
report["external_observations"]["steady_memory_mib"] = 120
report["external_observations"]["first_accepted_input_ms"] = 3_999
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "input timing before visible table" $reports
grep -q 'first accepted input cannot precede' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["test"]["hardware_model"] = "fixture-iphone"
report["external_observations"]["first_accepted_input_ms"] = 4_200
report["test"]["browser_family"] = "chrome"
report["browser"]["declared_family"] = "chrome"
report["browser"]["detected_family"] = "chrome"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "an impossible platform/browser pairing" $reports
grep -q 'browser family is invalid for test.platform' "$tmp/rejected.err"

python3 - "$tmp/desktop-edge-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["test"]["presentation_tier"] = "reduced"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a reduced desktop compatibility row" $reports
grep -q 'desktop compatibility reports must use the default' "$tmp/rejected.err"

python3 - "$tmp/desktop-edge-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["test"]["presentation_tier"] = "default"
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["candidate"]["source_hash"] = "0" * 64
report["timing_ms"]["capture_duration"] = 65_000
report["test"]["hardware_model"] = "fixture-iphone"
report["test"]["browser_family"] = "safari"
report["browser"]["declared_family"] = "safari"
report["browser"]["detected_family"] = "chrome"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a mismatched detected browser family" $reports
grep -q 'detected browser family does not match' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["browser"]["detected_family"] = "unknown"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "an unknown detected browser family" $reports
grep -q 'invalid browser.detected_family' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["browser"]["detected_family"] = "safari"
report["browser"]["detected_version"] = "150"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a mismatched detected browser version" $reports
grep -q 'detected browser version does not match' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["browser"]["detected_version"] = report["test"]["browser_version"]
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/desktop-edge-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["candidate"]["build_id"] = "different-build-" + report["candidate"]["source_hash"]
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a mixed-candidate final matrix" $reports
grep -q 'final browser matrix must use exactly one' "$tmp/rejected.err"

python3 - "$tmp/desktop-edge-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["candidate"]["build_id"] = "fixture-build-" + report["candidate"]["source_hash"]
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp"/desktop-edge-cold-current-*.json "$tmp"/android-tablet-chrome-cold-current-*.json <<'PY'
import json
import sys
from pathlib import Path

for value in sys.argv[1:4]:
    path = Path(value)
    report = json.loads(path.read_text(encoding="utf-8"))
    report["test"]["hardware_model"] = "different-desktop"
    path.write_text(json.dumps(report), encoding="utf-8")

for value in sys.argv[4:]:
    path = Path(value)
    report = json.loads(path.read_text(encoding="utf-8"))
    report["test"]["os_version"] = "different-os"
    path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "inconsistent comparison environments" $reports
grep -q 'one hardware model for desktop / edge' "$tmp/rejected.err"
grep -q 'one OS version for android-tablet' "$tmp/rejected.err"

python3 - "$tmp"/desktop-edge-cold-current-*.json "$tmp"/android-tablet-chrome-cold-current-*.json <<'PY'
import json
import sys
from pathlib import Path

for value in sys.argv[1:4]:
    path = Path(value)
    report = json.loads(path.read_text(encoding="utf-8"))
    report["test"]["hardware_model"] = "fixture-desktop"
    path.write_text(json.dumps(report), encoding="utf-8")

for value in sys.argv[4:]:
    path = Path(value)
    report = json.loads(path.read_text(encoding="utf-8"))
    report["test"]["os_version"] = "fixture-os"
    path.write_text(json.dumps(report), encoding="utf-8")
PY

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

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["test"]["browser_version"] = "152"
report["browser"]["declared_version"] = "152"
report["browser"]["detected_version"] = "152"
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "mixed exact current versions" $reports
grep -q 'one current browser version for iphone / safari' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["test"]["browser_version"] = "151"
report["browser"]["declared_version"] = "151"
report["browser"]["detected_version"] = "151"
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp"/iphone-safari-cold-minimum-*.json "$tmp"/iphone-safari-warm-minimum-*.json "$tmp"/iphone-safari-lifecycle-minimum-*.json <<'PY'
import json
import sys
from pathlib import Path

for value in sys.argv[1:]:
    path = Path(value)
    report = json.loads(path.read_text(encoding="utf-8"))
    report["test"]["browser_version"] = "152"
    report["browser"]["declared_version"] = "152"
    report["browser"]["detected_version"] = "152"
    path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a proposed minimum newer than current" $reports
grep -q 'proposed-minimum browser version must be older than current for iphone / safari' "$tmp/rejected.err"

python3 - "$tmp"/iphone-safari-cold-minimum-*.json "$tmp"/iphone-safari-warm-minimum-*.json "$tmp"/iphone-safari-lifecycle-minimum-*.json <<'PY'
import json
import sys
from pathlib import Path

for value in sys.argv[1:]:
    path = Path(value)
    report = json.loads(path.read_text(encoding="utf-8"))
    report["test"]["browser_version"] = "150"
    report["browser"]["declared_version"] = "150"
    report["browser"]["detected_version"] = "150"
    path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/iphone-safari-warm-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["timing_ms"]["hidden_duration"] = 30_000
report["timing_ms"]["hidden_durations"] = [30_000]
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "backgrounded interaction capture" $reports
grep -q 'non-lifecycle capture contains background intervals' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-warm-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["timing_ms"]["hidden_duration"] = 0
report["timing_ms"]["hidden_durations"] = []
report["interaction"]["orientation_changes"] = 1
report["interaction"]["orientation_states"] = ["portrait-primary"]
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "rotated interaction capture" $reports
grep -q 'non-lifecycle capture contains orientation changes' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-warm-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["interaction"]["orientation_changes"] = 0
report["interaction"]["orientation_states"] = []
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
report["external_observations"]["first_accepted_input_ms"] = 3_200
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "warm startup over budget" $reports
grep -q 'first-visible 3001.00ms exceeds 3000ms' "$tmp/rejected.err"
grep -q 'first accepted input 3200.00ms exceeds 3000ms' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-warm-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["external_observations"]["first_visible_table_ms"] = 2_900
report["external_observations"]["first_accepted_input_ms"] = 3_001
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "accepted input over budget after a timely visible table" $reports
grep -q 'first accepted input 3001.00ms exceeds 3000ms' "$tmp/rejected.err"

python3 - "$tmp/iphone-safari-warm-current-1.json" "$tmp/ipad-safari-lifecycle-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

warm = Path(sys.argv[1])
warm_report = json.loads(warm.read_text(encoding="utf-8"))
warm_report["external_observations"]["first_visible_table_ms"] = 1_500
warm_report["external_observations"]["first_accepted_input_ms"] = 1_700
warm.write_text(json.dumps(warm_report), encoding="utf-8")

lifecycle = Path(sys.argv[2])
lifecycle_report = json.loads(lifecycle.read_text(encoding="utf-8"))
lifecycle_report["timing_ms"]["capture_duration"] = 599_999
lifecycle.write_text(json.dumps(lifecycle_report), encoding="utf-8")
PY
expect_rejected "short lifecycle capture" $reports
grep -q 'shorter than 10 minutes' "$tmp/rejected.err"

python3 - "$tmp/ipad-safari-lifecycle-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["timing_ms"]["capture_duration"] = 610_000
report["interaction"]["orientation_states"] = ["landscape-primary", "portrait-primary"]
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "reversed orientation lifecycle" $reports
grep -q 'exact portrait-to-landscape transition was not observed' "$tmp/rejected.err"

python3 - "$tmp/ipad-safari-lifecycle-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["timing_ms"]["capture_duration"] = 610_000
report["interaction"]["orientation_states"] = ["portrait-primary", "landscape-primary"]
report["interaction"]["page_hide_count"] = 1
report["interaction"]["page_show_count"] = 1
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "incomplete page lifecycle evidence" $reports
grep -q 'lifecycle capture must contain exactly two page-hide events' "$tmp/rejected.err"
grep -q 'lifecycle capture must contain exactly two page-show events' "$tmp/rejected.err"

python3 - "$tmp/ipad-safari-lifecycle-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["interaction"]["page_hide_count"] = 2
report["interaction"]["page_show_count"] = 2
report["timing_ms"]["hidden_duration"] = 59_999
report["timing_ms"]["hidden_durations"] = [30_000, 29_999]
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "short background intervals" $reports
grep -q 'fewer than two 30-second background intervals' "$tmp/rejected.err"

python3 - "$tmp/ipad-safari-lifecycle-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["timing_ms"]["hidden_duration"] = 60_000
report["timing_ms"]["hidden_durations"] = [30_000, 30_000]
report["audio"]["backgroundSuspensions"] = 3
report["audio"]["explicitResumes"] = 2
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "impossible audio lifecycle counters" $reports
grep -q 'exactly two audio suspensions' "$tmp/rejected.err"

python3 - "$tmp/ipad-safari-lifecycle-current-1.json" "$tmp/android-phone-chrome-lifecycle-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

short = Path(sys.argv[1])
short_report = json.loads(short.read_text(encoding="utf-8"))
short_report["timing_ms"]["capture_duration"] = 610_000
short_report["interaction"]["page_hide_count"] = 2
short_report["interaction"]["page_show_count"] = 2
short_report["timing_ms"]["hidden_duration"] = 60_000
short_report["timing_ms"]["hidden_durations"] = [30_000, 30_000]
short_report["audio"]["backgroundSuspensions"] = 2
short_report["audio"]["explicitResumes"] = 2
short.write_text(json.dumps(short_report), encoding="utf-8")

audio = Path(sys.argv[2])
audio_report = json.loads(audio.read_text(encoding="utf-8"))
audio_report["audio"]["explicitResumes"] = 0
audio.write_text(json.dumps(audio_report), encoding="utf-8")
PY
expect_rejected "missing explicit audio resume" $reports
grep -q 'exactly two explicit audio resumes' "$tmp/rejected.err"

python3 - "$tmp/android-tablet-chrome-warm-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["audio"]["backgroundSuspensions"] = -1
report["audio"]["explicitResumes"] = -1
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "negative non-lifecycle audio counters" $reports
grep -q 'audio.backgroundSuspensions must be a non-negative integer' "$tmp/rejected.err"

python3 - "$tmp/android-tablet-chrome-warm-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["audio"]["backgroundSuspensions"] = 0
report["audio"]["explicitResumes"] = 0
path.write_text(json.dumps(report), encoding="utf-8")
PY

python3 - "$tmp/android-phone-chrome-lifecycle-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["audio"]["explicitResumes"] = 2
report["audio"]["transitionFailures"] = 1
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "a failed audio lifecycle transition" $reports
grep -q 'audio transition failures invalidate the capture' "$tmp/rejected.err"

python3 - "$tmp/android-phone-chrome-lifecycle-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["audio"].update(
    {
        "supported": False,
        "state": "unsupported",
        "gestureStarts": 1,
        "backgroundSuspensions": 0,
        "explicitResumes": 0,
        "muteChanges": 0,
        "mutedPlaybackAttempts": 1,
        "audiblePlaybackAttempts": 0,
        "transitionFailures": 0,
        "muted": False,
    }
)
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "unsupported audio with playback evidence" $reports
grep -q 'unsupported audio telemetry is inconsistent' "$tmp/rejected.err"

python3 - "$tmp/android-phone-chrome-lifecycle-current-1.json" "$tmp/android-tablet-chrome-warm-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

resume = Path(sys.argv[1])
resume_report = json.loads(resume.read_text(encoding="utf-8"))
resume_report["audio"].update(
    {
        "supported": True,
        "state": "running",
        "gestureStarts": 2,
        "backgroundSuspensions": 2,
        "explicitResumes": 2,
        "muteChanges": 2,
        "mutedPlaybackAttempts": 1,
        "audiblePlaybackAttempts": 1,
        "transitionFailures": 0,
        "muted": False,
    }
)
resume.write_text(json.dumps(resume_report), encoding="utf-8")

mute = Path(sys.argv[2])
mute_report = json.loads(mute.read_text(encoding="utf-8"))
mute_report["audio"]["gestureStarts"] = 1
mute_report["audio"]["audiblePlaybackAttempts"] = 0
mute.write_text(json.dumps(mute_report), encoding="utf-8")
PY
expect_rejected "incomplete repeated audio playback" $reports
grep -q 'exactly two plays' "$tmp/rejected.err"

python3 - "$tmp/android-tablet-chrome-warm-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["audio"]["muteChanges"] = 4
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "extra audio mute cycles" $reports
grep -q 'exactly one mute/unmute cycle' "$tmp/rejected.err"

python3 - "$tmp/android-tablet-chrome-warm-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["audio"]["muteChanges"] = 2
report["audio"]["gestureStarts"] = 3
report["audio"]["audiblePlaybackAttempts"] = 2
path.write_text(json.dumps(report), encoding="utf-8")
PY
expect_rejected "extra audio probe playback" $reports
grep -q 'exactly two plays' "$tmp/rejected.err"

python3 - "$tmp/android-tablet-chrome-warm-current-1.json" "$tmp/iphone-safari-cold-current-1.json" <<'PY'
import json
import sys
from pathlib import Path

mute = Path(sys.argv[1])
mute_report = json.loads(mute.read_text(encoding="utf-8"))
mute_report["audio"]["gestureStarts"] = 2
mute_report["audio"]["audiblePlaybackAttempts"] = 1
mute_report["audio"]["muteChanges"] = 2
mute.write_text(json.dumps(mute_report), encoding="utf-8")

duration = Path(sys.argv[2])
duration_report = json.loads(duration.read_text(encoding="utf-8"))
duration_report["timing_ms"]["capture_duration"] = 59_999
duration.write_text(json.dumps(duration_report), encoding="utf-8")
PY
expect_rejected "short interaction capture" $reports
grep -q 'shorter than 1 minute' "$tmp/rejected.err"

printf '%s\n' "feasibility tool self-tests passed"
