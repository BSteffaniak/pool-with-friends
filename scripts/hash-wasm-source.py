#!/usr/bin/env python3
"""Print the deterministic source identity used by PWMTF browser builds."""

from __future__ import annotations

import hashlib
from pathlib import Path


def main() -> None:
    """Hash the complete declared browser-build input set."""
    root = Path(__file__).resolve().parent.parent
    inputs = [root / "Cargo.lock", root / "Cargo.toml", root / "rust-toolchain.toml"]
    inputs.extend(
        path
        for directory in (root / ".cargo", root / "packages" / "client")
        for path in directory.rglob("*")
        if path.is_file()
    )
    digest = hashlib.sha256()
    for path in sorted(inputs, key=lambda candidate: candidate.relative_to(root).as_posix()):
        relative = path.relative_to(root).as_posix().encode()
        contents = path.read_bytes()
        digest.update(len(relative).to_bytes(4, "big"))
        digest.update(relative)
        digest.update(len(contents).to_bytes(8, "big"))
        digest.update(contents)
    print(digest.hexdigest())


if __name__ == "__main__":
    main()
