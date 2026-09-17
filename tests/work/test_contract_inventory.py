from __future__ import annotations

import json
import re
import unittest
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
BASELINE = Path(__file__).with_name("baselines") / "contracts.json"
SCHEMA_PATTERN = re.compile(r"work-[a-z0-9-]+/v[0-9]+")
SOURCE_ROOTS = (
    PROJECT_ROOT / "skills" / "work" / "scripts" / "worklib",
    PROJECT_ROOT / "skills" / "work" / "references",
)


def _source_contracts() -> set[str]:
    result: set[str] = set()
    for root in SOURCE_ROOTS:
        for path in root.rglob("*"):
            if path.suffix not in {".py", ".md"}:
                continue
            result.update(SCHEMA_PATTERN.findall(path.read_text(encoding="utf-8")))
    return result


class ContractInventoryBaselineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.baseline = json.loads(BASELINE.read_text(encoding="utf-8"))

    def test_inventory_matches_current_public_contract_references(self) -> None:
        kinds = self.baseline["kinds"]
        listed = [contract for contracts in kinds.values() for contract in contracts]
        self.assertEqual(len(listed), len(set(listed)), "contract IDs must be unique")
        self.assertEqual(set(listed), _source_contracts())

    def test_every_contract_has_one_supported_classification(self) -> None:
        self.assertEqual(
            set(self.baseline["kinds"]),
            {"request", "response", "artifact", "envelope"},
        )
        for kind, contracts in self.baseline["kinds"].items():
            with self.subTest(kind=kind):
                self.assertEqual(contracts, sorted(contracts))
                self.assertTrue(contracts)


if __name__ == "__main__":
    unittest.main()
