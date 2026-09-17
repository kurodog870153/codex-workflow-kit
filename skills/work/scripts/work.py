#!/usr/bin/env python3

import json
import sys


MINIMUM_PYTHON = (3, 14)


def _startup_failure(message, data):
    print(json.dumps({
        "schema": "work-cli-result/v1",
        "status": "failed",
        "reason_code": "cli_startup_failed",
        "message": message,
        "data": data,
    }, ensure_ascii=False, indent=2))
    raise SystemExit(10)


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding="utf-8", errors="strict", newline="\n")
    if sys.version_info < MINIMUM_PYTHON:
        _startup_failure(
            "Python 3.14 or newer is required.",
            {"dependency": "python", "minimum_version": "3.14"},
        )
    try:
        import pydantic  # noqa: F401
    except ModuleNotFoundError:
        _startup_failure(
            "Pydantic is required. Install the latest version and try again.",
            {"dependency": "pydantic", "install": "python -m pip install --upgrade pydantic"},
        )
    try:
        from worklib.cli import main
    except Exception:
        _startup_failure(
            "The Work CLI could not load. Check its runtime and dependencies.",
            {},
        )
    raise SystemExit(main())
