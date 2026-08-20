#!/usr/bin/env python3
"""Run a command while holding the PWMTF generated-bundle lock."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import subprocess

from wasm_bundle_lock import generated_bundle_lock


def main() -> None:
    """Serialize commands that read or replace the shared ``dist`` bundle."""
    parser = argparse.ArgumentParser()
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[:1] == ["--"]:
        command = command[1:]
    if not command:
        parser.error("a command is required after --")

    root = Path(__file__).resolve().parent.parent
    os.environ.pop("PWMTF_WASM_BUNDLE_LOCKED", None)
    with generated_bundle_lock():
        result = subprocess.run(command, cwd=root, check=False, env=os.environ)
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
