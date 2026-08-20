#!/usr/bin/env python3
"""Process locking for readers and writers of the shared PWMTF WASM bundle."""

from __future__ import annotations

from collections.abc import Callable, Generator
from contextlib import contextmanager
import fcntl
import os
from pathlib import Path
from typing import ParamSpec, TypeVar

_LOCK_ENVIRONMENT = "PWMTF_WASM_BUNDLE_LOCKED"
_PARAMETERS = ParamSpec("_PARAMETERS")
_RETURN = TypeVar("_RETURN")


@contextmanager
def generated_bundle_lock() -> Generator[None]:
    """Hold the repository-wide generated-bundle lock unless inherited."""
    if os.environ.get(_LOCK_ENVIRONMENT) == "1":
        yield
        return

    root = Path(__file__).resolve().parent.parent
    lock_path = root / "target" / "pwmtf-wasm-bundle.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        previous = os.environ.get(_LOCK_ENVIRONMENT)
        os.environ[_LOCK_ENVIRONMENT] = "1"
        try:
            yield
        finally:
            if previous is None:
                os.environ.pop(_LOCK_ENVIRONMENT, None)
            else:
                os.environ[_LOCK_ENVIRONMENT] = previous


def generated_bundle_locked(
    function: Callable[_PARAMETERS, _RETURN],
) -> Callable[_PARAMETERS, _RETURN]:
    """Decorate a generated-bundle reader or writer with the process lock."""

    def locked(*args: _PARAMETERS.args, **kwargs: _PARAMETERS.kwargs) -> _RETURN:
        with generated_bundle_lock():
            return function(*args, **kwargs)

    return locked
