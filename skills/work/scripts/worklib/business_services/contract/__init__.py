"""Contract catalog use cases."""

from __future__ import annotations

from ...services.contract import registry


def list_contracts() -> dict[str, object]:
    return registry.catalog().to_canonical_dict()


def describe_contract(contract_id: str) -> dict[str, object]:
    return registry.describe(contract_id).to_canonical_dict()


def scaffold_contract(contract_id: str) -> dict[str, object]:
    return registry.scaffold(contract_id).to_canonical_dict()


__all__ = ["describe_contract", "list_contracts", "scaffold_contract"]
