from __future__ import annotations

import sys
import unittest
from pathlib import Path


sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"))

from worklib.models.common.base import WorkContract as LegacyWorkContract
from worklib.models.common.cli import CliResultContract as LegacyCliResultContract
from worklib.models.common.cli import ErrorContract as LegacyErrorContract
from worklib.models.common import CliResultContract, ErrorContract, WorkContract


class CliContractTests(unittest.TestCase):
    def test_legacy_exports_preserve_class_identity(self) -> None:
        self.assertIs(LegacyWorkContract, WorkContract)
        self.assertIs(LegacyCliResultContract, CliResultContract)
        self.assertIs(LegacyErrorContract, ErrorContract)

    def test_cli_result_uses_stable_canonical_order(self) -> None:
        result = CliResultContract(
            schema="work-cli-result/v1", status="success", reason_code="ok",
            message="Done.", data={"value": 1},
        )
        self.assertEqual(
            list(result.to_canonical_dict()),
            ["schema", "status", "reason_code", "message", "data"],
        )

    def test_error_contract_is_strict_and_canonical(self) -> None:
        error = ErrorContract(
            schema="work-error/v1", code="example", message="Example.", details={},
        )
        self.assertEqual(
            error.to_canonical_dict(),
            {"schema": "work-error/v1", "code": "example", "message": "Example.", "details": {}},
        )


if __name__ == "__main__":
    unittest.main()
