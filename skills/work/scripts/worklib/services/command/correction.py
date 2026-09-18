from __future__ import annotations

import copy
from typing import Any


def apply_command_correction(
    index: dict[str, Any], correction: dict[str, Any]
) -> dict[str, Any]:
    result = copy.deepcopy(index)
    result["lock"]["command_correction"] = correction
    return result
