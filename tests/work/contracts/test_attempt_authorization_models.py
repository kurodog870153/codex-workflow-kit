from __future__ import annotations

import copy
import sys
import unittest
from pathlib import Path

from pydantic import ValidationError

SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.contracts.attempt_authorization_models import (
    AttemptAuthorizationContract, authorization_sha256, minimal_authorization,
    validate_authorization_scope,
)
from worklib.foundation.errors import WorkError


class AttemptAuthorizationTests(unittest.TestCase):
    def test_requires_fixed_reapproval_conditions_and_fingerprints_content(self):
        manifest = minimal_authorization()
        first = authorization_sha256(manifest)
        manifest["authorization_evidence"] = "A different reviewed authorization."
        self.assertNotEqual(first, authorization_sha256(manifest))
        manifest["reapproval_conditions"].pop()
        with self.assertRaises(ValidationError):
            AttemptAuthorizationContract.model_validate(manifest)

    def test_accepts_exact_task_subset_and_rejects_expansion(self):
        task = {"id": "TASK-001", "commands": [{"id": "CMD-001", "mode": "argv", "argv": ["tool"]}],
            "validations": [], "operations": [], "files": [{"id": "FILE-001", "action": "modify", "path": "src.txt"}]}
        manifest = minimal_authorization()
        manifest.update(commands=[copy.deepcopy(task["commands"][0])], modifiable_files=["src.txt"], working_directories=["."])
        self.assertEqual(validate_authorization_scope(manifest, task=task,
            defaults={"working_directory": ".", "os": "windows", "shell": "powershell"})["commands"], task["commands"])
        manifest["modifiable_files"] = ["other.txt"]
        with self.assertRaises(WorkError) as caught:
            validate_authorization_scope(manifest, task=task,
                defaults={"working_directory": ".", "os": "windows", "shell": "powershell"})
        self.assertEqual(caught.exception.code, "attempt_authorization_scope_expansion")


if __name__ == "__main__":
    unittest.main()
