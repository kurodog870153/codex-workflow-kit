from __future__ import annotations

import sys
import unittest
from pathlib import Path
from typing import ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.base import WorkContract
from worklib.foundation.errors import WorkError


class ExampleContract(WorkContract):
    contract_id: ClassVar[str] = "work-example/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "count", "note")

    schema_: Literal["work-example/v1"] = Field(alias="schema")
    count: int
    note: str | None = None


class NullableNestedModel(BaseModel):
    model_config = ConfigDict(strict=True, extra="forbid", frozen=True)

    required_value: str | None
    optional_value: str | None = None


class NullableContract(WorkContract):
    contract_id: ClassVar[str] = "work-nullable/v1"
    contract_kind: ClassVar[Literal["request"]] = "request"
    canonical_order: ClassVar[tuple[str, ...]] = ("schema", "required_value", "optional_value", "nested")

    schema_: Literal["work-nullable/v1"] = Field(alias="schema")
    required_value: str | None
    optional_value: str | None = None
    nested: NullableNestedModel


class WorkContractBaseTests(unittest.TestCase):
    def test_parse_and_render_preserve_canonical_bytes(self) -> None:
        contract = ExampleContract.parse_json_bytes(
            b'{"count":1,"schema":"work-example/v1"}', source="example.json"
        )

        self.assertEqual(
            contract.render_canonical_json(),
            b'{\n  "schema": "work-example/v1",\n  "count": 1\n}\n',
        )

    def test_strict_values_are_not_coerced(self) -> None:
        with self.assertRaises(WorkError) as caught:
            ExampleContract.parse_json_bytes(
                b'{"schema":"work-example/v1","count":"1"}',
                source="example.json",
            )

        self.assertEqual(caught.exception.code, "invalid_contract_value")
        self.assertEqual(caught.exception.details["location"], "count")

    def test_missing_and_unknown_fields_use_stable_work_error(self) -> None:
        with self.assertRaises(WorkError) as caught:
            ExampleContract.parse_json_bytes(
                b'{"schema":"work-example/v1","extra":true}',
                source="example.json",
            )

        self.assertEqual(caught.exception.code, "invalid_object_fields")
        self.assertEqual(caught.exception.details["missing"], ["count"])
        self.assertEqual(caught.exception.details["unknown"], ["extra"])

    def test_duplicate_keys_keep_existing_parser_error(self) -> None:
        with self.assertRaises(WorkError) as caught:
            ExampleContract.parse_json_bytes(
                b'{"schema":"work-example/v1","count":1,"count":2}',
                source="example.json",
            )

        self.assertEqual(caught.exception.code, "duplicate_json_key")

    def test_contracts_are_frozen(self) -> None:
        contract = ExampleContract(schema="work-example/v1", count=1)

        with self.assertRaises(Exception):
            contract.count = 2

    def test_required_nulls_are_kept_and_unset_optional_nulls_are_omitted(self) -> None:
        contract = NullableContract.model_validate({
            "schema": "work-nullable/v1",
            "required_value": None,
            "nested": {"required_value": None},
        })

        self.assertEqual(
            contract.to_canonical_dict(),
            {
                "schema": "work-nullable/v1",
                "required_value": None,
                "nested": {"required_value": None},
            },
        )

    def test_canonical_order_must_match_model_fields(self) -> None:
        with self.assertRaises(TypeError):
            class InvalidOrder(WorkContract):
                contract_id: ClassVar[str] = "work-invalid/v1"
                contract_kind: ClassVar[Literal["request"]] = "request"
                canonical_order: ClassVar[tuple[str, ...]] = ("value", "schema")

                schema_: Literal["work-invalid/v1"] = Field(alias="schema")
                value: str


if __name__ == "__main__":
    unittest.main()
