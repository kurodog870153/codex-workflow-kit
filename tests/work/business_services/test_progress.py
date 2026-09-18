from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"))

from worklib.business_services.progress import preview_progress
from worklib.models.common.errors import WorkError


class ProgressBusinessServiceTests(unittest.TestCase):
    def test_preview_parses_request_before_validation(self) -> None:
        with self.assertRaises(WorkError) as caught:
            preview_progress(Path.cwd(), b"not json", source="test", expected_revision=0)
        self.assertEqual(caught.exception.code, "invalid_json_contract")


if __name__ == "__main__":
    unittest.main()
