from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from ..models.common.base import ContractKind, WorkContract


def contract_mapping(value: WorkContract) -> Mapping[str, Any]:
    return value.to_canonical_dict()


__all__ = ["ContractKind", "WorkContract", "contract_mapping"]
