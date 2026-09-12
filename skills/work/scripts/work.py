#!/usr/bin/env python3

import json
import sys


if __name__ == "__main__":
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding="utf-8", errors="strict", newline="\n")
    try:
        from worklib.cli import main
    except Exception:
        print(json.dumps({
            "schema": "work-cli-result/v1",
            "status": "failed",
            "reason_code": "cli_startup_failed",
            "message": "The Work CLI could not load. Check its runtime and dependencies.",
            "data": {},
        }, ensure_ascii=False, indent=2))
        raise SystemExit(10)
    raise SystemExit(main())
