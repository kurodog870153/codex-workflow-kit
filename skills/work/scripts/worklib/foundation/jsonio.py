from __future__ import annotations

import json
from typing import Any, TextIO


def canonical_json(value: Any, *, sort_keys: bool = True) -> str:
    return json.dumps(
        value,
        ensure_ascii=False,
        indent=2,
        sort_keys=sort_keys,
        separators=(",", ": "),
    ) + "\n"


def write_json(stream: TextIO, value: Any, *, sort_keys: bool = True) -> None:
    stream.write(canonical_json(value, sort_keys=sort_keys))
