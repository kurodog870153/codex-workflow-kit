from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.cli import main


class WorkspaceCliTests(unittest.TestCase):
    def test_create_returns_python_owned_path(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = io.StringIO()
            code = main(["--project-root", temporary, "workspace", "create",
                         "--requirement-id", "example", "--workflow-id", "specification"], stdout=output)
            self.assertEqual(code, 0)
            value = json.loads(output.getvalue())["data"]
            self.assertEqual(value["schema"], "work-transaction-workspace/v1")
            self.assertTrue(Path(temporary, value["path"]).is_dir())
            self.assertIn(value["transaction_id"], value["path"])


if __name__ == "__main__":
    unittest.main()
