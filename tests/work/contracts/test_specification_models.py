from __future__ import annotations

import copy
import json
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

from pydantic import ValidationError

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "skills/work/scripts"))

from worklib.orchestration.task import prepare_specification, update_specification, verify_specification
from worklib.models.specification.contracts import SpecificationVerificationRequestContract
from worklib.models.specification.contracts import (
    SpecificationPrepareContract, SpecificationUpdateContract, SpecificationVerificationContract,
)
from worklib.models.specification.contracts import SpecificationPrepareRequestContract, SpecificationUpdateRequestContract
from worklib.services.contract import registry
from worklib.models.common.errors import ExitCode, WorkError


class SpecificationPrepareContractTests(unittest.TestCase):
    def request(self):
        return copy.deepcopy(SpecificationPrepareRequestContract.contract_example)

    def parse(self, request):
        return SpecificationPrepareRequestContract.parse_json_bytes(json.dumps(request).encode(), source="test")

    def test_null_evidence_round_trips_without_inventing_missing_fields(self):
        for operation, evidence in (("add", {"after": None}), ("remove", {"before": None}),
                                    ("replace", {"before": None, "after": "reviewed"})):
            with self.subTest(operation=operation):
                request = self.request()
                request["edits"] = [{"artifact": "task_index", "operation": operation,
                                     "path": "/summary", **evidence}]
                model = self.parse(request)
                self.assertEqual(model.to_canonical_dict(), request)
                self.assertEqual(self.parse(json.loads(model.render_canonical_json())).to_canonical_dict(), request)

    def test_missing_evidence_is_not_equivalent_to_null(self):
        for field in ("before", "after"):
            request = self.request()
            del request["edits"][0][field]
            with self.subTest(field=field), self.assertRaises(WorkError) as caught:
                self.parse(request)
            self.assertEqual(caught.exception.code, "spec_edit_fields")
            self.assertEqual(caught.exception.exit_code, ExitCode.ARTIFACT_INTEGRITY)

    def test_unknown_fields_preserve_locations(self):
        for nested in (False, True):
            request = self.request()
            target = request["edits"][0] if nested else request
            target["unexpected"] = True
            with self.subTest(nested=nested), self.assertRaises(WorkError) as caught:
                self.parse(request)
            self.assertEqual(caught.exception.code, "invalid_object_fields")
            self.assertEqual(caught.exception.details["location"], "edits[0]" if nested else "spec_prepare")

    def test_invalid_request_is_rejected_before_source_io(self):
        for value, code in (([], "spec_prepare_edits"), ({}, "spec_prepare_edits")):
            request = self.request()
            request["edits"] = value
            with patch("worklib.business_services.specification.workflow._load") as load:
                with self.assertRaises(WorkError) as caught:
                    prepare_specification(json.dumps(request).encode(), project_root=Path("."), user_config_root=".")
                self.assertEqual(caught.exception.code, code)
                load.assert_not_called()

    def test_schema_and_operation_errors_keep_reason_codes(self):
        for field, value, code in (("schema", "unsupported", "spec_prepare_schema"),
                                   ("operation", "unknown", "spec_edit_fields"),
                                   ("artifact", "unknown", "spec_prepare_artifact")):
            request = self.request()
            target = request if field == "schema" else request["edits"][0]
            target[field] = value
            with self.subTest(field=field), self.assertRaises(WorkError) as caught:
                self.parse(request)
            self.assertEqual(caught.exception.code, code)

    def test_registered_description_has_valid_example(self):
        description = registry.describe("work-spec-prepare-request/v1")
        self.assertEqual(description.required, ["schema", "plan_path", "reason", "edits"])
        self.assertEqual(self.parse(description.example).to_canonical_dict(), description.example)


class SpecificationUpdateContractTests(unittest.TestCase):
    def request(self):
        return copy.deepcopy(SpecificationUpdateRequestContract.contract_example)

    def parse(self, request):
        return SpecificationUpdateRequestContract.parse_json_bytes(json.dumps(request).encode(), source="test")

    def test_nested_models_preserve_absent_and_explicit_null_fields(self):
        request = self.request()
        request["task_index"]["decisions"] = None
        model = self.parse(request)
        self.assertEqual(model.to_canonical_dict(), request)
        self.assertEqual(self.parse(json.loads(model.render_canonical_json())).to_canonical_dict(), request)

    def test_old_schema_and_single_file_request_are_rejected_before_io(self):
        for operation in ("validate", "apply", "recover"):
            for old_schema in (False, True):
                request = self.request()
                if old_schema:
                    request["schema"] = "work-spec-update-request/v2"
                else:
                    request["task"] = request.pop("task_index")
                with self.subTest(operation=operation, old_schema=old_schema):
                    with patch("worklib.business_services.specification.workflow._load") as load, patch("worklib.business_services.specification.workflow.read_raw") as read:
                        with self.assertRaises(WorkError) as caught:
                            update_specification(json.dumps(request).encode(), project_root=Path("."),
                                                 user_config_root=".", operation=operation)
                        self.assertEqual(caught.exception.code, "spec_update_schema" if old_schema else "invalid_object_fields")
                        load.assert_not_called()
                        read.assert_not_called()

    def test_invalid_fingerprints_are_rejected(self):
        for invalid in ("A" * 64, "0" * 63, 123, None):
            request = self.request()
            request["expected"]["task_item_sha256"]["TASK-001"] = invalid
            with self.subTest(invalid=invalid), self.assertRaises(WorkError) as caught:
                self.parse(request)
            self.assertEqual(caught.exception.code, "spec_update_source_changed")

    def test_nested_candidates_use_existing_contracts(self):
        request = self.request()
        request["task_items"]["TASK-001"]["unexpected"] = True
        with self.assertRaises(WorkError) as caught:
            self.parse(request)
        self.assertEqual(caught.exception.code, "invalid_object_fields")
        self.assertEqual(caught.exception.details["location"], "task_items.TASK-001")

    def test_registry_describes_candidate_contract_references(self):
        description = registry.describe("work-spec-update-request/v1")
        references = {field.name: field.reference for field in description.fields if field.reference}
        self.assertEqual(references, {"plan": "work-plan/v1", "task_index": "work-task-index/v1", "task_items": "work-task-item/v1"})
        self.parse(description.example)


class SpecificationVerificationContractTests(unittest.TestCase):
    def test_registered_example_round_trips(self):
        description = registry.describe("work-spec-verification-request/v1")
        model = SpecificationVerificationRequestContract.model_validate(description.example)
        parsed = SpecificationVerificationRequestContract.parse_json_bytes(model.render_canonical_json(), source="test")
        self.assertEqual(parsed.to_canonical_dict(), description.example)
        self.assertEqual(description.required, ["schema", "requirement_id", "artifacts", "record_id"])

    def test_malformed_requests_are_rejected_before_journal_reads(self):
        example = SpecificationVerificationRequestContract.contract_example
        cases = [[], {**example, "schema": "work-spec-verification-request/v2"},
                 {**example, "unexpected": True}, {**example, "requirement_id": None},
                 {**example, "record_id": "../other"}, {**example, "record_id": 2},
                 {key: value for key, value in example.items() if key != "record_id"},
                 {**example, "artifacts": {"plan": "example.json"}},
                 {**example, "artifacts": {**example["artifacts"], "unexpected": True}}]
        for request in cases:
            with self.subTest(request=request), patch("worklib.business_services.specification.workflow.read_raw") as read:
                with self.assertRaises(WorkError) as caught:
                    verify_specification(json.dumps(request).encode(), project_root=Path("."), user_config_root=".")
                self.assertEqual(caught.exception.exit_code, ExitCode.CONTRACT)
                read.assert_not_called()

    def test_record_sequence_can_exceed_three_digits(self):
        request = {**SpecificationVerificationRequestContract.contract_example, "record_id": "SPEC-UPDATE-1000"}
        self.assertEqual(SpecificationVerificationRequestContract.model_validate(request).record_id, "SPEC-UPDATE-1000")


class SpecificationResponseContractTests(unittest.TestCase):
    def test_registered_examples_round_trip_with_null_transport(self):
        for contract in (SpecificationPrepareContract, SpecificationUpdateContract, SpecificationVerificationContract):
            with self.subTest(contract=contract.contract_id):
                example = registry.describe(contract.contract_id).example
                model = contract.model_validate(example)
                self.assertEqual(model.to_canonical_dict(), example)
                self.assertEqual(contract.parse_json_bytes(model.render_canonical_json(), source="test").to_canonical_dict(), example)
        prepared = SpecificationPrepareContract.model_validate(SpecificationPrepareContract.contract_example).to_canonical_dict()
        self.assertIsNone(prepared["output_file"])
        self.assertIsNone(prepared["transport"]["output_file"])

    def test_preview_cannot_claim_publication_or_omit_candidate(self):
        for changes in ({"candidate": None}, {"transaction": None}, {"publication_status": "published"}):
            with self.subTest(changes=changes), self.assertRaises(ValidationError):
                SpecificationUpdateContract.model_validate({**SpecificationUpdateContract.contract_example, **changes})

    def test_recovery_preserves_sparse_response(self):
        example = SpecificationUpdateContract.contract_example
        response = {key: example[key] for key in ("schema", "requirement_id", "record_id", "approved_sha256", "affected_task_ids", "artifacts")}
        response.update(status="recovered", publication_status="already_published",
                        verification_request=SpecificationVerificationRequestContract.contract_example,
                        next_step={"command": "task spec-verify", "input": "verification_request"})
        self.assertEqual(SpecificationUpdateContract.model_validate(response).to_canonical_dict(), response)
        del response["verification_request"]
        with self.assertRaises(ValidationError):
            SpecificationUpdateContract.model_validate(response)

    def test_prepare_transport_must_match_output_file(self):
        example = copy.deepcopy(SpecificationPrepareContract.contract_example)
        example["output_file"] = "prepared.json"
        with self.assertRaises(ValidationError):
            SpecificationPrepareContract.model_validate(example)

    def test_verification_never_authorizes_execution(self):
        example = copy.deepcopy(SpecificationVerificationContract.contract_example)
        example["execution_authorized"] = True
        with self.assertRaises(ValidationError):
            SpecificationVerificationContract.model_validate(example)
