//! Record-finish candidates after active lock and authorization checks.

use serde_json::{Value, json};

use crate::derivation::identity::next_record_id;
use crate::execution::attempt::render_attempt;
use crate::execution::authorization::{
    effective_task, require_modified_files, require_record_finish_result_evidence,
};
use crate::execution::index::{render_execution_index, validate_execution_index};
use crate::execution::requests::validate_record_finish_request;
use crate::execution::{
    ExecutionIssue, finish_attempt_candidate, finished_record_index, formal_record_kind,
};

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

pub struct RecordFinishStagingInput<'a> {
    pub canonical_root: &'a str,
    pub requirement: &'a crate::identifiers::RequirementId,
    pub execution_dir: &'a str,
    pub task_id: &'a str,
    pub attempt_id: &'a str,
    pub record_id: &'a str,
    pub task: &'a Value,
    pub request: &'a Value,
    pub index_before: &'a [u8],
    pub index_after: &'a [u8],
    pub attempt_before: &'a [u8],
    pub attempt_after: &'a [u8],
}

pub fn build_record_finish_staging(
    input: RecordFinishStagingInput<'_>,
) -> Result<work_model::runtime::RuntimeManifest, ExecutionIssue> {
    use crate::canonical::parse_json_contract;
    use crate::derivation::fingerprint;
    use work_model::runtime::{RuntimeBytes, RuntimeTarget};
    let parse = |raw| {
        parse_json_contract(raw).map_err(|_| {
            issue(
                "record_finish_staging_contract",
                "Canonical artifacts are required.",
                json!({}),
            )
        })
    };
    let index = parse(input.index_before)?;
    let attempt = parse(input.attempt_before)?;
    validate_execution_index(&index, input.index_before)?;
    crate::execution::attempt::validate_attempt_bytes(&attempt, input.attempt_before)?;
    if index["requirement_id"] != input.requirement.as_str()
        || attempt["task_id"] != input.task_id
        || attempt["attempt_id"] != input.attempt_id
        || attempt["status"] != "in_progress"
    {
        return Err(issue(
            "record_finish_staging_identity",
            "The execution identities disagree.",
            json!({}),
        ));
    }
    let expected =
        build_record_finish_candidates(input.task, &attempt, &index, input.task_id, input.request)?;
    if expected.record_id != input.record_id
        || render_attempt(&expected.attempt)? != input.attempt_after
        || render_execution_index(&expected.index).map_err(|_| {
            issue(
                "record_finish_staging_contract",
                "The index cannot be rendered.",
                json!({}),
            )
        })? != input.index_after
    {
        return Err(issue(
            "record_finish_staging_transition",
            "The prepared artifacts do not match the authorized result.",
            json!({}),
        ));
    }
    let bytes = |raw: &[u8]| RuntimeBytes {
        bytes: raw.to_vec(),
        sha256: fingerprint::raw(raw),
    };
    let payloads = std::collections::BTreeMap::from([
        ("attempt.json.tmp".into(), input.attempt_after.to_vec()),
        ("index.json.tmp".into(), input.index_after.to_vec()),
    ]);
    crate::execution::recovery::build_execution_staging_manifest(
        crate::execution::recovery::ExecutionStagingInput {
            canonical_root: input.canonical_root,
            requirement: input.requirement,
            execution_dir: input.execution_dir,
            operation: crate::derivation::publication::RuntimeOperation::RecordFinish,
            approval_sha256: &fingerprint::structured(input.request).map_err(|_| {
                issue(
                    "record_finish_staging_contract",
                    "The request cannot be fingerprinted.",
                    json!({}),
                )
            })?,
            business_identity: json!({"task_id":input.task_id,"attempt_id":input.attempt_id,"record_id":input.record_id,"record_finish_request":input.request}),
            targets: vec![
                RuntimeTarget {
                    path: format!(
                        "{}/{}/{}/attempt.json",
                        input.execution_dir, input.task_id, input.attempt_id
                    ),
                    before: Some(bytes(input.attempt_before)),
                    after: Some(bytes(input.attempt_after)),
                },
                RuntimeTarget {
                    path: format!("{}/index.json", input.execution_dir),
                    before: Some(bytes(input.index_before)),
                    after: Some(bytes(input.index_after)),
                },
            ],
            payloads: &payloads,
        },
    )
}

pub struct RecordFinishCandidates {
    pub attempt: Value,
    pub index: Value,
    pub record_id: String,
    pub record_kind: &'static str,
}

pub fn build_record_finish_candidates(
    task: &Value,
    attempt: &Value,
    index: &Value,
    task_id: &str,
    request: &Value,
) -> Result<RecordFinishCandidates, ExecutionIssue> {
    validate_record_finish_request(request)?;
    let lock = &index["lock"];
    if lock["kind"] != "execution" {
        return Err(issue(
            "record_finish_execution_lock_required",
            "A matching execution lock is required.",
            json!({"lock":lock}),
        ));
    }
    let attempt_id = attempt["attempt_id"].as_str().unwrap_or("");
    if lock["task_id"] != task_id
        || lock["attempt_id"] != attempt_id
        || lock["execute_instructions_sha256"] != attempt["execute_instructions_sha256"]
    {
        return Err(issue(
            "record_finish_lock_mismatch",
            "The execution lock does not match the active Attempt.",
            json!({}),
        ));
    }
    let record_id = lock["record_id"].as_str().ok_or_else(|| {
        issue(
            "record_finish_record_not_reserved",
            "The execution lock does not reserve a record.",
            json!({}),
        )
    })?;
    let base = record_id.split('#').next().unwrap_or(record_id);
    let effective = effective_task(task, attempt)?;
    let kind = formal_record_kind(&effective, base)?;
    let expected = next_record_id(base, attempt)?;
    if record_id != expected {
        return Err(issue(
            "record_finish_retry_sequence_mismatch",
            "The reserved record ID is not the next record instance.",
            json!({"expected":expected,"actual":record_id}),
        ));
    }
    if request["record"]["status"] == "skipped" && request.get("modified_files").is_some() {
        return Err(issue(
            "record_finish_skipped_modified_files",
            "A skipped record cannot report modified files.",
            json!({"record_id":record_id}),
        ));
    }
    let modified: Vec<String> = request["modified_files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    require_modified_files(attempt, &modified, Some(base))?;
    require_record_finish_result_evidence(
        attempt,
        &request["record"],
        record_id,
        kind,
        request["authorization_evidence"].as_str(),
    )?;
    let mut finished_attempt = finish_attempt_candidate(
        attempt,
        request,
        record_id,
        kind,
        lock.get("command_correction"),
    )?;
    finished_attempt["acceptance_results"] =
        crate::execution::acceptance::reset(&finished_attempt["acceptance_results"])?;
    finished_attempt["acceptance_results"] =
        crate::execution::acceptance::aggregate_attempt(&effective, &finished_attempt)?;
    render_attempt(&finished_attempt)?;
    let finished_index = finished_record_index(index)?;
    let finished_index =
        crate::execution::acceptance::update_index_attempt(&finished_index, &finished_attempt)?;
    let index_raw = render_execution_index(&finished_index).expect("validated JSON");
    validate_execution_index(&finished_index, &index_raw)?;
    Ok(RecordFinishCandidates {
        attempt: finished_attempt,
        index: finished_index,
        record_id: record_id.to_owned(),
        record_kind: kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::canonical_json_sha256;
    use crate::execution::build_execution_lock;
    use crate::execution::index::build_initial_execution_index;

    #[test]
    fn failed_validation_requires_new_authorization_before_candidate_build() {
        let task = json!({"commands":[],"operations":[],"validations":[{"id":"VAL-001"}]});
        let attempt = json!({"attempt_id":"ATTEMPT-001","records":[],
            "authorization":{"authorization_evidence":"Original","modifiable_files":[]}});
        let index = json!({"lock":{"kind":"execution","task_id":"TASK-001",
            "attempt_id":"ATTEMPT-001","record_id":"VAL-001",
            "execute_instructions_sha256":null}});
        let request = json!({"schema":"work-record-finish-request",
            "record":{"outcome":"failed","evidence":"Validation failed."}});
        assert_eq!(
            build_record_finish_candidates(&task, &attempt, &index, "TASK-001", &request)
                .err()
                .unwrap()
                .reason_code,
            "execution_authorization_result_required"
        );
        let mut reused = request;
        reused["authorization_evidence"] = json!("Original");
        assert_eq!(
            build_record_finish_candidates(&task, &attempt, &index, "TASK-001", &reused)
                .err()
                .unwrap()
                .reason_code,
            "execution_authorization_result_evidence_reused"
        );
    }

    #[test]
    fn successful_validation_records_attempt_and_releases_record_lock() {
        let authorization = json!({"schema":"work-attempt-authorization",
            "task_id":"TASK-001","commands":[],"validations":[{"id":"VAL-001","acceptance_ids":["ACCEPTANCE-001"]}],
            "modifiable_files":[],"working_directories":[],"external_operations":[],
            "allowed_deviations":[],"reapproval_conditions":["scope_expansion",
                "source_or_worktree_drift","failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let attempt = json!({"acceptance_results":[{"id":"ACCEPTANCE-001","status":"pending","evidence":[]}],"schema":"work-attempt","attempt_id":"ATTEMPT-001",
            "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","skill_id":null,
            "status":"in_progress","task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"task_item_sha256":"c".repeat(64),
            "task_instructions_sha256":"d".repeat(64),
            "execute_instructions_sha256":"e".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),
            "execute_skill_selection_sha256":"0".repeat(64),
            "authorization_sha256":canonical_json_sha256(&authorization).unwrap(),
            "authorization":authorization,"started_at":"2026-09-01T10:00+08:00","records":[]});
        let collection = json!({"requirement_id":"demo","spec_id":"TASK-SPEC-001",
            "acceptance_criteria":[{"id":"ACCEPTANCE-001"}],"tasks":[{"id":"TASK-001","traceability":{"acceptance_ids":["ACCEPTANCE-001"]}}]});
        let validation = json!({"task_collection_sha256":"a".repeat(64),
            "task_index_sha256":"b".repeat(64),"instructions_sha256":"d".repeat(64),
            "hierarchy_selection_sha256":"f".repeat(64),"skill_selection_sha256":"0".repeat(64),
            "task_item_sha256":{"TASK-001":"c".repeat(64)},
            "task_instructions_sha256":{"TASK-001":"d".repeat(64)},
            "task_skill_ids":{"TASK-001":null}});
        let mut index = build_initial_execution_index(&collection, &validation).unwrap();
        index["overall_status"] = json!("in_progress");
        index["tasks"][0]["status"] = json!("in_progress");
        index["tasks"][0]["latest_attempt"] = json!("ATTEMPT-001");
        index["lock"] = build_execution_lock("TASK-001", "ATTEMPT-001", &"e".repeat(64));
        index["lock"]["record_id"] = json!("VAL-001");
        let task = json!({"id":"TASK-001","traceability":{"acceptance_ids":["ACCEPTANCE-001"]},"commands":[],"operations":[],"validations":[{"id":"VAL-001","acceptance_ids":["ACCEPTANCE-001"]}]});
        let request = json!({"schema":"work-record-finish-request",
            "record":{"outcome":"passed","evidence":"Validation passed."}});
        let candidate =
            build_record_finish_candidates(&task, &attempt, &index, "TASK-001", &request).unwrap();
        assert_eq!(
            candidate.attempt["acceptance_results"][0]["status"],
            "completed"
        );
        assert_eq!(
            candidate.index["acceptance_results"][0]["status"],
            "completed"
        );
        assert_eq!(candidate.record_id, "VAL-001");
        assert_eq!(candidate.record_kind, "validation");
        assert_eq!(candidate.attempt["records"][0]["id"], "VAL-001");
        assert_eq!(candidate.attempt["records"][0]["kind"], "validation");
        assert!(candidate.index["lock"].get("record_id").is_none());
    }
}
