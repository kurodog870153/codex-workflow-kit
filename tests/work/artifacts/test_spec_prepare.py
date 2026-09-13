from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from artifacts import test_specification as fixtures
from worklib.artifacts.spec_prepare import prepare_specification
from worklib.foundation.errors import WorkError


class SpecificationPreparationTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.SpecificationUpdateTests()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)

    def prepare(self, request):
        return prepare_specification(json.dumps(request).encode("utf-8"),
            project_root=self.fixture.root, user_config_root=str(self.fixture.root))

    def test_duplicate_and_unchanged_edits_are_rejected_without_writes(self):
        for duplicate, code in ((True, "spec_prepare_duplicate"), (False, "spec_prepare_unchanged")):
            with self.subTest(duplicate=duplicate):
                request = self.fixture.prepare_request()
                if duplicate:
                    request["edits"].append(copy.deepcopy(request["edits"][0]))
                else:
                    request["edits"][0]["after"] = request["edits"][0]["before"]
                before = self.fixture.snapshot()
                with self.assertRaises(WorkError) as caught:
                    self.prepare(request)
                self.assertEqual(caught.exception.code, code)
                self.assertEqual(self.fixture.snapshot(), before)

    def test_missing_plan_change_evidence_is_rejected(self):
        request = self.fixture.prepare_request()
        request["edits"] = [{"artifact": "plan", "field": "summary",
                             "before": "Original result", "after": "Confirmed result"}]
        before = self.fixture.snapshot()
        with self.assertRaises(WorkError) as caught:
            self.prepare(request)
        self.assertEqual(caught.exception.code, "spec_prepare_plan_evidence")
        self.assertEqual(self.fixture.snapshot(), before)

    def test_prepared_transport_revalidates_to_identical_approval(self):
        before = self.fixture.snapshot()
        result = self.prepare(self.fixture.prepare_request())
        transported = json.loads(json.dumps(result["request"], ensure_ascii=False))
        self.assertEqual(self.fixture.run_update(transported), result["preview"])
        self.assertEqual(self.fixture.snapshot(), before)
        self.assertEqual(result["request"]["task"]["tasks"][2], self.fixture.task["tasks"][2])
