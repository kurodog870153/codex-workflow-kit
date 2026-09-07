from __future__ import annotations

import io
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.jsonio import canonical_json, write_json


class JsonIoTests(unittest.TestCase):
    def test_canonical_json_sorts_keys_and_preserves_unicode(self) -> None:
        result = canonical_json({"z": 1, "名稱": "工作"})

        self.assertEqual(
            result,
            '{\n  "z": 1,\n  "名稱": "工作"\n}\n',
        )

    def test_write_json_honors_insertion_order_when_sorting_is_disabled(self) -> None:
        stream = io.StringIO()

        write_json(stream, {"z": 1, "a": 2}, sort_keys=False)

        self.assertEqual(
            stream.getvalue(),
            '{\n  "z": 1,\n  "a": 2\n}\n',
        )


if __name__ == "__main__":
    unittest.main()
