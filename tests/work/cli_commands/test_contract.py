from __future__ import annotations

import io
import json
import sys
import unittest
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[3]
SCRIPT_ROOT = PROJECT_ROOT / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.cli import main as _main
from worklib.models.common.errors import ExitCode


def main(arguments, **kwargs):
    return _main(["--verbose", *arguments], **kwargs)


class ContractCliTests(unittest.TestCase):
    def run_cli(self, *arguments: str) -> tuple[int, dict[str, object], str]:
        stdout = io.StringIO()
        stderr = io.StringIO()
        code = main(
            ["--project-root", str(PROJECT_ROOT), "contract", *arguments],
            stdout=stdout,
            stderr=stderr,
        )
        return code, json.loads(stdout.getvalue()), stderr.getvalue()

    def test_list_returns_registered_contracts(self) -> None:
        code, result, stderr = self.run_cli("list")

        self.assertEqual(code, ExitCode.SUCCESS)
        self.assertEqual(stderr, "")
        self.assertEqual(result["data"]["schema"], "work-contract-catalog/v1")
        contract_ids = [item["id"] for item in result["data"]["contracts"]]
        self.assertTrue(contract_ids)
        self.assertEqual(contract_ids, sorted(contract_ids))
        self.assertIn("work-contract-catalog/v1", contract_ids)
        self.assertIn("work-contract-description/v1", contract_ids)
        self.assertIn("work-contract-scaffold/v1", contract_ids)
        self.assertIn("work-plan-prepare-request/v1", contract_ids)

    def test_describe_returns_stable_public_description(self) -> None:
        code, result, stderr = self.run_cli(
            "describe", "work-contract-catalog/v1"
        )

        self.assertEqual(code, ExitCode.SUCCESS)
        self.assertEqual(stderr, "")
        self.assertEqual(
            result["data"]["schema"], "work-contract-description/v1"
        )
        self.assertEqual(result["data"]["id"], "work-contract-catalog/v1")

    def test_unknown_contract_is_rejected(self) -> None:
        code, result, stderr = self.run_cli("describe", "work-unknown/v1")

        self.assertEqual(code, ExitCode.CONTRACT)
        self.assertEqual(stderr, "")
        self.assertEqual(result["reason_code"], "unknown_contract_id")

    def test_scaffold_returns_request_shape_order_and_example(self) -> None:
        code, result, stderr = self.run_cli(
            "scaffold", "work-record-finish-request/v1"
        )

        self.assertEqual(code, ExitCode.SUCCESS)
        self.assertEqual(stderr, "")
        self.assertEqual(result["data"]["schema"], "work-contract-scaffold/v1")
        self.assertEqual(
            list(result["data"]["scaffold"]), result["data"]["canonical_order"]
        )

    def test_scaffold_rejects_nonrequest_contract(self) -> None:
        code, result, stderr = self.run_cli(
            "scaffold", "work-contract-catalog/v1"
        )

        self.assertEqual(code, ExitCode.CONTRACT)
        self.assertEqual(stderr, "")
        self.assertEqual(
            result["reason_code"], "contract_scaffold_requires_request"
        )

    def test_plan_prepare_scaffold_contains_complete_nested_shapes(self) -> None:
        code, result, stderr = self.run_cli(
            "scaffold", "work-plan-prepare-request/v1"
        )

        self.assertEqual(code, ExitCode.SUCCESS)
        self.assertEqual(stderr, "")
        scaffold = result["data"]["scaffold"]
        self.assertIn("dependencies", scaffold["content"])
        self.assertEqual(
            list(scaffold["content"]["dependencies"][0]),
            ["id", "statement", "applies_to"],
        )


if __name__ == "__main__":
    unittest.main()
