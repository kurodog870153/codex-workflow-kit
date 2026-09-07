from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.artifacts.plan import create_plan_file
from worklib.foundation.errors import ExitCode, WorkError


class PlanArtifactTests(unittest.TestCase):
    def test_create_writes_validated_bytes_without_overwriting(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            project_root = Path(temporary).resolve()
            validation = {
                "schema": "work-plan-validation/v1",
                "requirement_id": "example",
                "status": "confirmed",
                "plan_sha256": "a" * 64,
            }
            rendered = b"canonical plan"
            with patch(
                "worklib.artifacts.plan.prepare_plan_json_contract",
                return_value=(validation, rendered),
            ), patch(
                "worklib.artifacts.plan.validate_plan_file",
                return_value=validation,
            ):
                result = create_plan_file(
                    b"request",
                    source="test",
                    raw_plan_path="outputs/plan.md",
                    project_root=project_root,
                    user_config_root=temporary,
                )
                path = project_root / "outputs" / "plan.md"
                self.assertEqual(path.read_bytes(), rendered)
                self.assertEqual(result["schema"], "work-plan-create/v1")
                self.assertEqual(result["path"], "outputs/plan.md")

                with self.assertRaises(WorkError) as context:
                    create_plan_file(
                        b"request",
                        source="test",
                        raw_plan_path="outputs/plan.md",
                        project_root=project_root,
                        user_config_root=temporary,
                    )
                self.assertEqual(context.exception.exit_code, ExitCode.WORKFLOW_STATE)
                self.assertEqual(context.exception.code, "plan_already_exists")
                self.assertEqual(path.read_bytes(), rendered)


if __name__ == "__main__":
    unittest.main()
