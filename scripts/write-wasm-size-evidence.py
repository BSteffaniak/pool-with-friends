#!/usr/bin/env python3
"""Produce machine-readable size evidence for a verified PWMTF WASM bundle."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import subprocess
from pathlib import Path
from typing import Any

from wasm_bundle_lock import generated_bundle_locked

MANIFEST_NAME = "pwmtf-bundle-manifest.json"


@generated_bundle_locked
def main() -> int:
    """Read a verified manifest and write candidate-bound size evidence."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", default="dist", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    arguments = parser.parse_args()
    manifest_path = arguments.bundle / MANIFEST_NAME
    verifier = Path(__file__).resolve().parent / "verify-wasm-bundle.py"
    try:
        subprocess.run(
            [str(verifier), str(arguments.bundle)],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            text=True,
        )
    except subprocess.CalledProcessError as error:
        parser.error(f"bundle verification failed: {error.stderr.strip()}")
    try:
        manifest: Any = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        parser.error(f"cannot read verified bundle manifest: {error}")
    if not isinstance(manifest, dict) or not isinstance(manifest.get("candidate"), dict):
        parser.error("verified bundle manifest candidate is invalid")
    if not isinstance(manifest.get("assets"), dict):
        parser.error("verified bundle manifest assets are invalid")
    candidate = manifest["candidate"]
    required_candidate = {"build_id", "source_hash", "bundle_hash", "wasm_optimization"}
    if set(candidate) != required_candidate or any(
        not isinstance(candidate[field], str) or not candidate[field] for field in required_candidate
    ):
        parser.error("verified bundle candidate identity is invalid")
    if manifest.get("bundle_hash_algorithm") != "sha256-length-prefixed-v1":
        parser.error("verified bundle hash algorithm is unsupported")

    assets: dict[str, dict[str, Any]] = {}
    raw_total = 0
    gzip_total = 0
    for name in sorted(manifest["assets"]):
        path = arguments.bundle / name
        if path.is_symlink() or not path.is_file():
            parser.error(f"bundle asset must be a regular file: {name}")
        try:
            body = path.read_bytes()
        except OSError as error:
            parser.error(f"cannot read bundle asset {name}: {error}")
        compressed = gzip.compress(body, compresslevel=9, mtime=0)
        metadata = {
            "raw_bytes": len(body),
            "gzip_bytes": len(compressed),
            "sha256": hashlib.sha256(body).hexdigest(),
        }
        assets[name] = metadata
        raw_total += metadata["raw_bytes"]
        gzip_total += metadata["gzip_bytes"]

    evidence = {
        "schema_version": 1,
        "candidate": {
            **manifest["candidate"],
            "bundle_hash_algorithm": manifest.get("bundle_hash_algorithm"),
        },
        "assets": assets,
        "totals": {"raw_bytes": raw_total, "gzip_bytes": gzip_total},
    }
    try:
        arguments.output.write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    except OSError as error:
        parser.error(f"cannot write size evidence: {error}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
