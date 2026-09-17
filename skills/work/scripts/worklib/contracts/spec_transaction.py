from __future__ import annotations

import base64
from typing import Any

from ..foundation.fingerprint import canonical_json_sha256, raw_sha256
from ..foundation.markdown import parse_json_contract, render_json_contract, require_canonical_json_contract
from .spec_transaction_models import SpecTransactionContract


def encode_snapshot(raw: bytes) -> dict[str, str]:
    return {
        "raw_sha256": raw_sha256(raw),
        "base64": base64.b64encode(raw).decode("ascii"),
    }


def transaction_approval_sha256(files: list[dict[str, Any]], metadata: dict[str, Any]) -> str:
    return canonical_json_sha256({"files": files, "metadata": metadata})


def canonicalize_spec_transaction(contract: object) -> dict[str, Any]:
    try:
        model = SpecTransactionContract.model_validate(contract)
    except Exception as error:
        from pydantic import ValidationError

        if isinstance(error, ValidationError):
            raise SpecTransactionContract._work_error(error) from error
        raise
    return model.to_canonical_dict()


def render_spec_transaction(contract: object) -> bytes:
    return render_json_contract(canonicalize_spec_transaction(contract))


def validate_spec_transaction(raw: bytes, *, source: str) -> dict[str, Any]:
    contract = canonicalize_spec_transaction(parse_json_contract(raw, source=source))
    require_canonical_json_contract(raw, contract=contract, source=source)
    return contract
