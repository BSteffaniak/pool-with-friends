#!/usr/bin/env python3
"""Validate and summarize PWMTF physical-browser feasibility reports."""

from __future__ import annotations

import argparse
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

REQUIRED_RUNS = 3
REQUIRED_PLATFORMS = {"iphone", "ipad", "android-phone", "android-tablet"}
ALLOWED_PLATFORMS = REQUIRED_PLATFORMS | {"desktop"}
ALLOWED_BROWSER_FAMILIES = {
    "safari",
    "chrome",
    "firefox",
    "samsung-internet",
    "edge",
    "chromium",
}
REQUIRED_BROWSER_MATRIX = {
    "iphone": {"safari"},
    "ipad": {"safari"},
    "android-phone": {"chrome", "firefox", "samsung-internet"},
    "android-tablet": {"chrome"},
}
REQUIRED_CACHE_STATES = {"cold", "warm", "lifecycle"}
REQUIRED_VERSION_STATUSES = {"yes", "no"}
REQUIRED_PHYSICAL_CHECKS = {
    "first_load",
    "aiming",
    "power",
    "touch_reset",
    "resize",
    "safe_area",
    "orientation",
    "background",
    "audio",
    "input_response",
    "memory",
    "thermal",
}
COLD_VISIBLE_BUDGET_MS = 8_000
WARM_VISIBLE_BUDGET_MS = 3_000
MINIMUM_FPS_FLOOR = 30
FULL_QUALITY_FPS_THRESHOLD = 55
MINIMUM_AIM_CAPTURE_MS = 60 * 1_000
LIFECYCLE_DURATION_MS = 10 * 60 * 1_000
REQUIRED_TEST_FIELDS = (
    "platform",
    "hardware_model",
    "os_version",
    "browser_family",
    "browser_version",
    "cache_state",
    "minimum_version_run",
    "presentation_tier",
    "run_number",
)
REQUIRED_CANDIDATE_FIELDS = (
    "build_id",
    "source_hash",
    "bevy",
    "renderer",
)


def number(value: Any) -> float | None:
    """Return a finite numeric value or None."""
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        return None
    result = float(value)
    if result != result or result in (float("inf"), float("-inf")):
        return None
    return result


def load_report(path: Path) -> dict[str, Any]:
    """Load and structurally validate one feasibility report."""
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"{path}: cannot read report: {error}") from error

    if not isinstance(report, dict):
        raise ValueError(f"{path}: report root must be an object")
    if report.get("schema_version") != 1:
        raise ValueError(f"{path}: unsupported schema_version")

    candidate = report.get("candidate")
    if not isinstance(candidate, dict):
        raise ValueError(f"{path}: candidate metadata must be an object")
    if set(candidate) != set(REQUIRED_CANDIDATE_FIELDS):
        raise ValueError(f"{path}: candidate metadata keys are invalid")
    for field in REQUIRED_CANDIDATE_FIELDS:
        value = candidate.get(field)
        if not isinstance(value, str) or not value:
            raise ValueError(f"{path}: missing candidate.{field}")
    if not 1 <= len(candidate["build_id"]) <= 128 or not all(
        character.isascii() and (character.isalnum() or character in "._-")
        for character in candidate["build_id"]
    ):
        raise ValueError(f"{path}: invalid candidate.build_id")
    if len(candidate["source_hash"]) != 64 or any(
        character not in "0123456789abcdef" for character in candidate["source_hash"]
    ):
        raise ValueError(f"{path}: invalid candidate.source_hash")
    if candidate["bevy"] != "0.19.1":
        raise ValueError(f"{path}: unexpected candidate.bevy")
    if candidate["renderer"] != "WebGL2":
        raise ValueError(f"{path}: unexpected candidate.renderer")

    test = report.get("test")
    if not isinstance(test, dict):
        raise ValueError(f"{path}: test metadata must be an object")
    for field in REQUIRED_TEST_FIELDS:
        value = test.get(field)
        if value is None or value == "":
            raise ValueError(f"{path}: missing test.{field}")
    if test["platform"] not in ALLOWED_PLATFORMS:
        raise ValueError(f"{path}: invalid test.platform")
    if test["browser_family"] not in ALLOWED_BROWSER_FAMILIES:
        raise ValueError(f"{path}: invalid test.browser_family")
    if test["cache_state"] not in REQUIRED_CACHE_STATES:
        raise ValueError(f"{path}: invalid test.cache_state")
    if test["minimum_version_run"] not in ("yes", "no"):
        raise ValueError(f"{path}: invalid test.minimum_version_run")
    if test["presentation_tier"] not in ("default", "reduced"):
        raise ValueError(f"{path}: invalid test.presentation_tier")
    if not isinstance(test["run_number"], int) or isinstance(test["run_number"], bool):
        raise ValueError(f"{path}: test.run_number must be an integer")
    if not 1 <= test["run_number"] <= 99:
        raise ValueError(f"{path}: test.run_number must be between 1 and 99")

    for section in (
        "browser",
        "display",
        "timing_ms",
        "performance",
        "interaction",
        "audio",
        "physical_checks",
        "external_observations",
    ):
        if not isinstance(report.get(section), dict):
            raise ValueError(f"{path}: {section} must be an object")

    browser = report["browser"]
    if browser.get("declared_family") != test["browser_family"]:
        raise ValueError(f"{path}: browser.declared_family does not match test.browser_family")
    detected_family = browser.get("detected_family")
    if detected_family not in ALLOWED_BROWSER_FAMILIES | {"unknown"}:
        raise ValueError(f"{path}: invalid browser.detected_family")
    if detected_family != "unknown" and detected_family != test["browser_family"]:
        raise ValueError(f"{path}: detected browser family does not match test.browser_family")
    if not isinstance(browser.get("user_agent"), str) or not browser["user_agent"]:
        raise ValueError(f"{path}: browser.user_agent must be a non-empty string")

    checks = report["physical_checks"]
    if set(checks) != REQUIRED_PHYSICAL_CHECKS:
        missing = sorted(REQUIRED_PHYSICAL_CHECKS - set(checks))
        unknown = sorted(set(checks) - REQUIRED_PHYSICAL_CHECKS)
        details = []
        if missing:
            details.append(f"missing {', '.join(missing)}")
        if unknown:
            details.append(f"unknown {', '.join(unknown)}")
        raise ValueError(f"{path}: physical_checks keys invalid: {'; '.join(details)}")
    invalid_checks = [name for name, result in checks.items() if result not in ("pass", "fail")]
    if invalid_checks:
        raise ValueError(f"{path}: invalid physical check results: {', '.join(sorted(invalid_checks))}")

    observations = report["external_observations"]
    for name in (
        "first_visible_table_ms",
        "first_accepted_input_ms",
        "steady_memory_mib",
        "peak_memory_mib",
    ):
        if number(observations.get(name)) is None:
            raise ValueError(f"{path}: external_observations.{name} must be finite")
    if observations.get("thermal_result") not in ("no-warning", "warning"):
        raise ValueError(f"{path}: invalid external_observations.thermal_result")
    if observations.get("reload_or_eviction_observed") not in ("yes", "no"):
        raise ValueError(f"{path}: invalid external_observations.reload_or_eviction_observed")
    if observations["peak_memory_mib"] < observations["steady_memory_mib"]:
        raise ValueError(f"{path}: peak memory cannot be lower than steady memory")

    captured_at = report.get("captured_at")
    if not isinstance(captured_at, str) or not captured_at:
        raise ValueError(f"{path}: captured_at must be a non-empty string")

    return report


def group_key(report: dict[str, Any]) -> tuple[str, ...]:
    """Return the fields that identify comparable repeated runs."""
    test = report["test"]
    candidate = report["candidate"]
    return (
        str(candidate["build_id"]),
        str(candidate["source_hash"]),
        str(candidate["bevy"]),
        str(candidate["renderer"]),
        str(test["platform"]),
        str(test["hardware_model"]),
        str(test["os_version"]),
        str(test["browser_family"]),
        str(test["browser_version"]),
        str(test["cache_state"]),
        str(test["minimum_version_run"]),
        str(test["presentation_tier"]),
    )


def metric(report: dict[str, Any], section: str, name: str) -> float | None:
    """Read one optional finite report metric."""
    return number(report[section].get(name))


def summarize(values: list[float], *, lower_is_better: bool) -> str:
    """Format the median and worst observed value."""
    if not values:
        return "unavailable"
    worst = max(values) if lower_is_better else min(values)
    return f"{statistics.median(values):.2f} / {worst:.2f}"


def markdown_table(groups: dict[tuple[str, ...], list[dict[str, Any]]]) -> str:
    """Build a Markdown summary for validated report groups."""
    lines = [
        "| Platform/device | OS / browser | Cache / version / tier | Runs | Physical checks | Visible table ms median/worst | Accepted input ms median/worst | Min FPS median/worst | p95 frame ms median/worst | Steady memory MiB median/worst | Thermal/reload |",
        "| --- | --- | --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | --- |",
    ]
    for key in sorted(groups):
        (
            build_id,
            source_hash,
            bevy,
            renderer,
            platform,
            hardware,
            os_version,
            browser_family,
            browser_version,
            cache_state,
            minimum_version,
            presentation_tier,
        ) = key
        reports = groups[key]
        visible = [number(report["external_observations"]["first_visible_table_ms"]) for report in reports]
        accepted_input = [number(report["external_observations"]["first_accepted_input_ms"]) for report in reports]
        minimum_fps = [value for report in reports if (value := metric(report, "performance", "minimum_one_second_fps")) is not None]
        p95_frame = [value for report in reports if (value := metric(report, "performance", "p95_frame_time_ms")) is not None]
        steady_memory = [number(report["external_observations"]["steady_memory_mib"]) for report in reports]
        failed_checks = sorted(
            {
                name
                for report in reports
                for name, result in report["physical_checks"].items()
                if result == "fail"
            }
        )
        checks_summary = "PASS" if not failed_checks else "FAIL: " + ", ".join(failed_checks)
        thermal_or_reload = any(
            report["external_observations"]["thermal_result"] == "warning"
            or report["external_observations"]["reload_or_eviction_observed"] == "yes"
            for report in reports
        )
        lifecycle_summary = "FAIL" if thermal_or_reload else "PASS"
        lines.append(
            "| "
            + " | ".join(
                (
                    f"{platform} / {hardware} / {build_id[:12]} / Bevy {bevy} {renderer}",
                    f"{os_version} / {browser_family} {browser_version}",
                    f"{cache_state} / {'minimum' if minimum_version == 'yes' else 'current'} / {presentation_tier}",
                    str(len(reports)),
                    checks_summary,
                    summarize([value for value in visible if value is not None], lower_is_better=True),
                    summarize([value for value in accepted_input if value is not None], lower_is_better=True),
                    summarize(minimum_fps, lower_is_better=False),
                    summarize(p95_frame, lower_is_better=True),
                    summarize([value for value in steady_memory if value is not None], lower_is_better=True),
                    lifecycle_summary,
                )
            )
            + " |"
        )
    return "\n".join(lines)


def acceptance_errors(groups: dict[tuple[str, ...], list[dict[str, Any]]]) -> list[str]:
    """Return objective budget and lifecycle failures in final-gate reports."""
    errors: list[str] = []
    for key, reports in groups.items():
        cache_state = key[9]
        presentation_tier = key[11]
        for report in reports:
            run = report["test"]["run_number"]
            observations = report["external_observations"]
            failed_checks = sorted(
                name for name, result in report["physical_checks"].items() if result == "fail"
            )
            if failed_checks:
                errors.append(f"{key} run {run}: failed checks: {', '.join(failed_checks)}")
            if observations["thermal_result"] == "warning":
                errors.append(f"{key} run {run}: thermal warning or severe throttling")
            if observations["reload_or_eviction_observed"] == "yes":
                errors.append(f"{key} run {run}: reload or eviction observed")

            visible = number(observations["first_visible_table_ms"])
            visible_budget = COLD_VISIBLE_BUDGET_MS if cache_state == "cold" else WARM_VISIBLE_BUDGET_MS
            if cache_state in ("cold", "warm") and visible is not None and visible > visible_budget:
                errors.append(f"{key} run {run}: first-visible {visible:.2f}ms exceeds {visible_budget}ms")

            duration = metric(report, "timing_ms", "capture_duration")
            if duration is None:
                errors.append(f"{key} run {run}: capture duration is unavailable")
            elif cache_state != "lifecycle" and duration < MINIMUM_AIM_CAPTURE_MS:
                errors.append(f"{key} run {run}: interaction capture {duration:.2f}ms is shorter than 1 minute")

            minimum_fps = metric(report, "performance", "minimum_one_second_fps")
            if minimum_fps is None:
                errors.append(f"{key} run {run}: minimum FPS is unavailable")
            elif minimum_fps < MINIMUM_FPS_FLOOR:
                errors.append(f"{key} run {run}: minimum FPS {minimum_fps:.2f} is below {MINIMUM_FPS_FLOOR}")
            elif minimum_fps < FULL_QUALITY_FPS_THRESHOLD and presentation_tier != "reduced":
                errors.append(
                    f"{key} run {run}: minimum FPS {minimum_fps:.2f} requires a measured quality fallback"
                )

            p95_frame = metric(report, "performance", "p95_frame_time_ms")
            if p95_frame is None:
                errors.append(f"{key} run {run}: p95 frame time is unavailable")

            if cache_state == "lifecycle":
                if duration is None or duration < LIFECYCLE_DURATION_MS:
                    actual = "unavailable" if duration is None else f"{duration:.2f}ms"
                    errors.append(f"{key} run {run}: lifecycle capture {actual} is shorter than 10 minutes")

            audio = report["audio"]
            if audio.get("state") != "running":
                errors.append(f"{key} run {run}: audio context was not running when exported")
            if audio.get("muted") is not False:
                errors.append(f"{key} run {run}: audio remained muted when exported")
            if audio.get("gestureStarts", 0) < 2:
                errors.append(f"{key} run {run}: audio probe did not cover pre/post-mute playback")
            if cache_state == "lifecycle":
                if audio.get("backgroundSuspensions", 0) < 1:
                    errors.append(f"{key} run {run}: audio background suspension was not observed")
                if audio.get("explicitResumes", 0) < 1:
                    errors.append(f"{key} run {run}: explicit audio resume was not observed")
                if report["interaction"].get("visibility_changes", 0) < 4:
                    errors.append(f"{key} run {run}: fewer than two background/foreground cycles")
                if report["interaction"].get("orientation_changes", 0) < 2:
                    errors.append(f"{key} run {run}: fewer than two orientation changes")
    return errors


def main() -> int:
    """Validate reports and print a repeatable Markdown summary."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", nargs="+", type=Path, help="downloaded feasibility JSON reports")
    parser.add_argument(
        "--allow-incomplete",
        action="store_true",
        help=f"summarize groups with fewer than {REQUIRED_RUNS} distinct runs",
    )
    parser.add_argument(
        "--require-mobile-matrix",
        action="store_true",
        help="require iPhone, iPad, Android phone/tablet and cold, warm, lifecycle groups",
    )
    args = parser.parse_args()

    groups: dict[tuple[str, ...], list[dict[str, Any]]] = defaultdict(list)
    errors: list[str] = []
    for path in args.reports:
        try:
            groups[group_key(report := load_report(path))].append(report)
        except ValueError as error:
            errors.append(str(error))

    for key, reports in groups.items():
        run_numbers = [report["test"]["run_number"] for report in reports]
        if len(run_numbers) != len(set(run_numbers)):
            errors.append(f"{key}: duplicate run numbers")
        if len(set(run_numbers)) < REQUIRED_RUNS and not args.allow_incomplete:
            errors.append(f"{key}: requires {REQUIRED_RUNS} distinct runs, found {len(set(run_numbers))}")

    if args.require_mobile_matrix:
        if args.allow_incomplete:
            errors.append("--allow-incomplete cannot be combined with --require-mobile-matrix")
        present = {(key[4], key[7], key[9], key[10]) for key in groups}
        missing = sorted(
            (platform, browser_family, cache_state, version_status)
            for platform, browser_families in REQUIRED_BROWSER_MATRIX.items()
            for browser_family in browser_families
            for cache_state in REQUIRED_CACHE_STATES
            for version_status in REQUIRED_VERSION_STATUSES
            if (platform, browser_family, cache_state, version_status) not in present
        )
        for platform, browser_family, cache_state, version_status in missing:
            version_label = "minimum" if version_status == "yes" else "current"
            errors.append(
                f"mobile matrix missing {platform} / {browser_family} / {cache_state} / {version_label}"
            )
        required_browsers = {
            (platform, browser_family)
            for platform, browser_families in REQUIRED_BROWSER_MATRIX.items()
            for browser_family in browser_families
        }
        present_browsers = {(key[4], key[7]) for key in groups}
        for platform, browser_family in sorted(required_browsers - present_browsers):
            errors.append(f"mobile matrix missing browser evidence for {platform} / {browser_family}")
        errors.extend(acceptance_errors(groups))

    if errors:
        for error in errors:
            print(f"feasibility report error: {error}", file=sys.stderr)
        return 1
    if not groups:
        print("feasibility report error: no valid reports", file=sys.stderr)
        return 1

    print(markdown_table(groups))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
