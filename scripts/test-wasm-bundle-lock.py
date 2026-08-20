#!/usr/bin/env python3
"""Self-test PWMTF generated-bundle lock inheritance and serialization."""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from wasm_bundle_lock import generated_bundle_lock


def fail(message: str) -> None:
    """Exit with a focused lock self-test failure."""
    raise SystemExit(message)


def main() -> None:
    """Prove nested inheritance, environment restoration, and exclusion."""
    lock_environment = "PWMTF_WASM_BUNDLE_LOCKED"
    original = os.environ.get(lock_environment)
    os.environ.pop(lock_environment, None)
    try:
        with generated_bundle_lock():
            if os.environ.get(lock_environment) != "1":
                fail("bundle lock did not mark the current process as locked")
            with generated_bundle_lock():
                if os.environ.get(lock_environment) != "1":
                    fail("nested bundle lock did not inherit ownership")
            subprocess.run(
                [
                    sys.executable,
                    "-c",
                    (
                        "import os; "
                        "raise SystemExit(0 if os.environ.get('PWMTF_WASM_BUNDLE_LOCKED') == '1' else 1)"
                    ),
                ],
                check=True,
            )
        if lock_environment in os.environ:
            fail("bundle lock leaked ownership into the caller environment")

        root = Path(__file__).resolve().parent.parent
        with tempfile.TemporaryDirectory(prefix="pwmtf-lock-test-") as temporary:
            marker = Path(temporary) / "acquired"
            holder = subprocess.Popen(
                [
                    sys.executable,
                    "-c",
                    (
                        "import os, sys, time; "
                        "sys.path.insert(0, sys.argv[1]); "
                        "from wasm_bundle_lock import generated_bundle_lock; "
                        "from pathlib import Path; "
                        "os.environ.pop('PWMTF_WASM_BUNDLE_LOCKED', None); "
                        "\nwith generated_bundle_lock():\n"
                        " Path(sys.argv[2]).write_text('ready\\n', encoding='utf-8')\n"
                        " time.sleep(0.75)"
                    ),
                    str(root / "scripts"),
                    str(marker),
                ],
                cwd=root,
                env={key: value for key, value in os.environ.items() if key != lock_environment},
            )
            deadline = time.monotonic() + 5
            while not marker.exists():
                if holder.poll() is not None:
                    fail("bundle lock holder exited before acquiring the lock")
                if time.monotonic() >= deadline:
                    holder.terminate()
                    fail("bundle lock holder did not acquire the lock")
                time.sleep(0.01)
            started = time.monotonic()
            with generated_bundle_lock():
                elapsed = time.monotonic() - started
            holder.wait(timeout=5)
            if holder.returncode != 0:
                fail("bundle lock holder failed")
            if elapsed < 0.5:
                fail("bundle lock did not serialize independent processes")
    finally:
        if original is None:
            os.environ.pop(lock_environment, None)
        else:
            os.environ[lock_environment] = original

    print("WASM bundle lock self-tests passed")


if __name__ == "__main__":
    main()
