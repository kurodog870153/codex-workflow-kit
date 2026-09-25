//! Pure planning index and discussion draft validation.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};

use crate::identifiers::RequirementId;
use crate::protocol::{INVALID_SHA256_ERROR_CODE, PLANNING_STATUSES, TASK_ID_PREFIX, valid_sha256};
use crate::task::candidate::validate_semantic_candidate;
use crate::task::{TaskIssue, resolve_dependencies};

fn issue(reason_code: &'static str, message: &'static str, location: &str) -> TaskIssue {
    TaskIssue {
        reason_code,
        message,
        details: json!({"location": location}),
    }
}

fn object<'a>(
    value: &'a Value,
    location: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, TaskIssue> {
    let Some(object) = value.as_object() else {
        return Err(issue(
            "expected_object",
            "A JSON object is required.",
            location,
        ));
    };
    let missing: Vec<_> = required
        .iter()
        .filter(|field| !object.contains_key(**field))
        .copied()
        .collect();
    let unknown: Vec<_> = object
        .keys()
        .filter(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(TaskIssue {
            reason_code: "invalid_object_fields",
            message: "The JSON object has missing or unknown fields.",
            details: json!({"location": location, "missing": missing, "unknown": unknown}),
        });
    }
    Ok(object)
}

fn text<'a>(value: &'a Value, location: &str) -> Result<&'a str, TaskIssue> {
    value
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| {
            issue(
                "empty_text_value",
                "A non-empty string is required.",
                location,
            )
        })
}

fn revision(value: &Value, location: &str) -> Result<u64, TaskIssue> {
    value
        .as_u64()
        .filter(|revision| *revision >= 1)
        .ok_or_else(|| {
            issue(
                "invalid_draft_revision",
                "A positive integer revision is required.",
                location,
            )
        })
}

fn sha(value: &Value, location: &str) -> Result<(), TaskIssue> {
    let valid = value.as_str().is_some_and(valid_sha256);
    if !valid {
        return Err(issue(
            INVALID_SHA256_ERROR_CODE,
            "A lowercase SHA-256 fingerprint is required.",
            location,
        ));
    }
    Ok(())
}

fn task_id<'a>(value: &'a Value, location: &str) -> Result<&'a str, TaskIssue> {
    let id = text(value, location)?;
    if id
        .strip_prefix(TASK_ID_PREFIX)
        .is_none_or(|suffix| suffix.len() != 3 || !suffix.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(issue(
            "invalid_draft_task_id",
            "A TASK-NNN identifier is required.",
            location,
        ));
    }
    Ok(id)
}

fn texts(value: &Value, location: &str, required: bool) -> Result<Vec<String>, TaskIssue> {
    let Some(values) = value
        .as_array()
        .filter(|values| !required || !values.is_empty())
    else {
        return Err(issue(
            "invalid_draft_array",
            "A text array with the required cardinality is required.",
            location,
        ));
    };
    values
        .iter()
        .enumerate()
        .map(|(index, item)| text(item, &format!("{location}[{index}]")).map(str::to_owned))
        .collect()
}

fn source(value: &Value) -> Result<(), TaskIssue> {
    let fields = [
        "plan_sha256",
        "hierarchy_selection_sha256",
        "skill_selection_sha256",
    ];
    let source = object(value, "source", &fields, &[])?;
    for field in fields {
        sha(&source[field], &format!("source.{field}"))?;
    }
    Ok(())
}

fn identity(value: &Value, schema: &str) -> Result<(), TaskIssue> {
    if value["schema"] != schema {
        return Err(issue(
            "invalid_draft_schema",
            "A planning contract schema is required.",
            "schema",
        ));
    }
    let requirement = text(&value["requirement_id"], "requirement_id")?;
    requirement
        .parse::<RequirementId>()
        .map_err(|failure| TaskIssue {
            reason_code: failure.reason_code(),
            message: "The requirement ID is invalid.",
            details: json!({"location": "requirement_id"}),
        })?;
    source(&value["source"])
}

pub fn validate_draft_instruction_selection(value: &Value) -> Result<(), TaskIssue> {
    let selection = object(
        value,
        "instruction_selection",
        &["selected_paths", "references"],
        &[],
    )?;
    for field in ["selected_paths", "references"] {
        let values = texts(&selection[field], field, false).map_err(|_| {
            issue(
                "invalid_source_selection",
                "Instruction selections must be unique string arrays.",
                field,
            )
        })?;
        if values.iter().collect::<HashSet<_>>().len() != values.len() {
            return Err(issue(
                "invalid_source_selection",
                "Instruction selections must be unique string arrays.",
                field,
            ));
        }
    }
    Ok(())
}

pub fn validate_planning_index(value: &Value) -> Result<Value, TaskIssue> {
    let index = object(
        value,
        "planning_index",
        &[
            "schema",
            "requirement_id",
            "revision",
            "source",
            "current_task_id",
            "tasks",
        ],
        &["retired_task_ids"],
    )?;
    identity(value, "work-task-planning-index/v1")?;
    let current_revision = revision(&index["revision"], "revision")?;
    let tasks = index["tasks"]
        .as_array()
        .filter(|tasks| !tasks.is_empty())
        .ok_or_else(|| {
            issue(
                "invalid_draft_array",
                "The planning index requires TASK entries.",
                "tasks",
            )
        })?;
    let mut ids = Vec::new();
    let mut dependencies = BTreeMap::new();
    for (position, task) in tasks.iter().enumerate() {
        let location = format!("tasks[{position}]");
        let task = object(
            task,
            &location,
            &[
                "id",
                "title",
                "goal",
                "scope",
                "skill_id",
                "dependencies",
                "status",
                "boundary_revision",
                "instructions_sha256",
            ],
            &["draft_ref", "instruction_selection"],
        )?;
        let id = task_id(&task["id"], &format!("{location}.id"))?.to_owned();
        if ids.contains(&id) {
            return Err(issue(
                "duplicate_draft_task_id",
                "TASK identifiers must be unique.",
                &location,
            ));
        }
        ids.push(id.clone());
        text(&task["title"], &format!("{location}.title"))?;
        text(&task["goal"], &format!("{location}.goal"))?;
        texts(&task["scope"], &format!("{location}.scope"), true)?;
        if !task["skill_id"].is_null() {
            text(&task["skill_id"], &format!("{location}.skill_id"))?;
        }
        let status = task["status"].as_str();
        if !status.is_some_and(|status| PLANNING_STATUSES.contains(&status)) {
            return Err(issue(
                "invalid_draft_status",
                "A planning status is required.",
                &location,
            ));
        }
        revision(
            &task["boundary_revision"],
            &format!("{location}.boundary_revision"),
        )?;
        sha(
            &task["instructions_sha256"],
            &format!("{location}.instructions_sha256"),
        )?;
        if let Some(selection) = task.get("instruction_selection") {
            validate_draft_instruction_selection(selection)?;
        }
        if let Some(reference) = task.get("draft_ref") {
            let reference = object(
                reference,
                &format!("{location}.draft_ref"),
                &["save_revision", "revision", "sha256"],
                &[],
            )?;
            let save = revision(
                &reference["save_revision"],
                &format!("{location}.draft_ref.save_revision"),
            )?;
            revision(
                &reference["revision"],
                &format!("{location}.draft_ref.revision"),
            )?;
            sha(
                &reference["sha256"],
                &format!("{location}.draft_ref.sha256"),
            )?;
            if save > current_revision || status == Some("planned") {
                return Err(issue(
                    "invalid_draft_reference",
                    "A draft reference must identify an existing discussion version.",
                    &location,
                ));
            }
        }
        let direct = texts(
            &task["dependencies"],
            &format!("{location}.dependencies"),
            false,
        )?;
        if direct.iter().collect::<HashSet<_>>().len() != direct.len() {
            return Err(issue(
                "duplicate_draft_dependency",
                "Dependencies must be unique.",
                &location,
            ));
        }
        dependencies.insert(id, direct);
    }
    for (id, direct) in &dependencies {
        if direct
            .iter()
            .any(|dependency| dependency == id || !ids.contains(dependency))
        {
            return Err(issue(
                "invalid_task_dependency",
                "A TASK dependency is unknown or refers to itself.",
                id,
            ));
        }
    }
    let order = resolve_dependencies(&ids, &dependencies)?;
    let retired = texts(
        index
            .get("retired_task_ids")
            .unwrap_or(&Value::Array(Vec::new())),
        "retired_task_ids",
        false,
    )?;
    if retired.iter().collect::<HashSet<_>>().len() != retired.len()
        || retired.iter().any(|id| ids.contains(id))
    {
        return Err(issue(
            "invalid_retired_task_ids",
            "Retired TASK IDs must be unique and absent from the active list.",
            "retired_task_ids",
        ));
    }
    for id in &retired {
        task_id(&json!(id), "retired_task_ids")?;
    }
    if !index["current_task_id"].is_null() {
        let current = task_id(&index["current_task_id"], "current_task_id")?;
        if !ids.contains(&current.to_owned()) {
            return Err(issue(
                "unknown_current_task",
                "The resume TASK must exist in the index.",
                "current_task_id",
            ));
        }
    }
    Ok(work_model::task::response::typed_response::<
        work_model::task::response::TaskPlanningIndexValidation,
    >(
        json!({"schema": "work-task-planning-index-validation/v1", "requirement_id": index["requirement_id"], "revision": current_revision, "task_count": ids.len(), "task_order": order.order, "status": "valid"}),
    ))
}

pub fn validate_task_draft(value: &Value, index: &Value) -> Result<Value, TaskIssue> {
    validate_planning_index(index)?;
    let draft = object(
        value,
        "task_draft",
        &[
            "schema",
            "requirement_id",
            "task_id",
            "revision",
            "boundary_revision",
            "source",
            "instructions_sha256",
            "status",
            "notes",
            "confirmed_decisions",
            "tentative",
            "open_questions",
            "next_discussion_point",
        ],
        &["task_candidate"],
    )?;
    identity(value, "work-task-draft/v1")?;
    let id = task_id(&draft["task_id"], "task_id")?;
    let current_revision = revision(&draft["revision"], "revision")?;
    revision(&draft["boundary_revision"], "boundary_revision")?;
    sha(&draft["instructions_sha256"], "instructions_sha256")?;
    let status = draft["status"]
        .as_str()
        .filter(|status| matches!(*status, "in_progress" | "refined" | "needs_review"))
        .ok_or_else(|| {
            issue(
                "invalid_draft_status",
                "A saved discussion requires a draft status.",
                "status",
            )
        })?;
    for field in ["notes", "tentative", "open_questions"] {
        texts(&draft[field], field, false)?;
    }
    let decisions = draft["confirmed_decisions"].as_array().ok_or_else(|| {
        issue(
            "invalid_draft_array",
            "Confirmed decisions must be an array.",
            "confirmed_decisions",
        )
    })?;
    for (position, decision) in decisions.iter().enumerate() {
        let location = format!("confirmed_decisions[{position}]");
        let decision = object(decision, &location, &["statement", "rationale"], &[])?;
        text(&decision["statement"], &format!("{location}.statement"))?;
        text(&decision["rationale"], &format!("{location}.rationale"))?;
    }
    if !draft["next_discussion_point"].is_null() {
        text(&draft["next_discussion_point"], "next_discussion_point")?;
    }
    if status == "refined" {
        if draft["tentative"]
            .as_array()
            .is_some_and(|values| !values.is_empty())
            || draft["open_questions"]
                .as_array()
                .is_some_and(|values| !values.is_empty())
            || !draft["next_discussion_point"].is_null()
        {
            return Err(issue(
                "unfinished_refined_draft",
                "A refined discussion cannot contain pending decisions or questions.",
                "status",
            ));
        }
    } else if draft["next_discussion_point"].is_null() {
        return Err(issue(
            "missing_draft_resume_point",
            "An unfinished discussion requires a resume point.",
            "next_discussion_point",
        ));
    }
    if draft["requirement_id"] != index["requirement_id"] || draft["source"] != index["source"] {
        return Err(issue(
            "draft_source_mismatch",
            "The draft must match the index requirement and source fingerprints.",
            "source",
        ));
    }
    let entry = index["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["id"] == id)
        .ok_or_else(|| {
            issue(
                "draft_task_not_in_index",
                "The draft TASK must exist in the index.",
                "task_id",
            )
        })?;
    for field in ["boundary_revision", "instructions_sha256", "status"] {
        if draft[field] != entry[field] {
            return Err(issue(
                "draft_index_mismatch",
                "The draft must match its indexed boundary, instructions and status.",
                field,
            ));
        }
    }
    if entry
        .get("draft_ref")
        .is_some_and(|reference| draft["revision"] != reference["revision"])
    {
        return Err(issue(
            "draft_index_mismatch",
            "The draft revision must match its index reference.",
            "revision",
        ));
    }
    if let Some(candidate) = draft.get("task_candidate") {
        validate_semantic_candidate(candidate, status == "refined")?;
    }
    Ok(work_model::task::response::typed_response::<
        work_model::task::response::TaskDraftValidation,
    >(
        json!({"schema": "work-task-draft-validation/v1", "requirement_id": draft["requirement_id"], "task_id": id, "revision": current_revision, "planning_status": status, "status": "valid"}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_sha_rejects_uppercase_with_field_location() {
        let mut value = json!({"plan_sha256":"a".repeat(64),
            "hierarchy_selection_sha256":"b".repeat(64),
            "skill_selection_sha256":"c".repeat(64)});
        source(&value).unwrap();
        value["plan_sha256"] = json!("A".repeat(64));
        let error = source(&value).unwrap_err();
        assert_eq!(error.reason_code, "invalid_sha256");
        assert_eq!(error.details["location"], "source.plan_sha256");
    }

    #[test]
    fn draft_model_boundaries_keep_nulls_and_reject_invalid_input() {
        let source = json!({"plan_sha256":"a".repeat(64),
            "hierarchy_selection_sha256":"b".repeat(64),
            "skill_selection_sha256":"c".repeat(64)});
        let mut index = json!({"schema":"work-task-planning-index/v1",
            "requirement_id":"example","revision":1,"source":source,
            "current_task_id":null,"tasks":[{"id":"TASK-001","title":"Example",
                "goal":"Deliver.","scope":["Implementation"],"skill_id":null,
                "dependencies":[],"status":"refined","boundary_revision":1,
                "instructions_sha256":"d".repeat(64)}]});
        let mut draft = json!({"schema":"work-task-draft/v1","requirement_id":"example",
            "task_id":"TASK-001","revision":1,"boundary_revision":1,"source":source,
            "instructions_sha256":"d".repeat(64),"status":"refined","notes":[],
            "confirmed_decisions":[],"tentative":[],"open_questions":[],
            "next_discussion_point":null});
        validate_planning_index(&index).unwrap();
        validate_task_draft(&draft, &index).unwrap();
        let typed_index: work_model::task::draft::TaskPlanningIndex =
            serde_json::from_value(index.clone()).unwrap();
        let typed_draft: work_model::task::draft::TaskDraft =
            serde_json::from_value(draft.clone()).unwrap();
        assert_eq!(serde_json::to_value(typed_index).unwrap(), index);
        assert_eq!(serde_json::to_value(typed_draft).unwrap(), draft);
        assert!(index["current_task_id"].is_null());
        assert!(index["tasks"][0]["skill_id"].is_null());
        assert!(draft["next_discussion_point"].is_null());
        assert!(index.get("retired_task_ids").is_none());
        assert!(draft.get("task_candidate").is_none());
        index["revision"] = json!("1");
        assert_eq!(
            validate_planning_index(&index).unwrap_err().reason_code,
            "invalid_draft_revision"
        );
        index["revision"] = json!(1);
        index["tasks"][0]["unknown"] = json!(true);
        assert_eq!(
            validate_planning_index(&index).unwrap_err().reason_code,
            "invalid_object_fields"
        );
        index["tasks"][0].as_object_mut().unwrap().remove("unknown");
        draft["schema"] = json!("work-task-draft/v2");
        assert_eq!(
            validate_task_draft(&draft, &index).unwrap_err().reason_code,
            "invalid_draft_schema"
        );
        draft["schema"] = json!("work-task-draft/v1");
        draft["unknown"] = json!(true);
        assert_eq!(
            validate_task_draft(&draft, &index).unwrap_err().reason_code,
            "invalid_object_fields"
        );
    }

    #[test]
    fn index_example_and_cycles() {
        let index = json!({"schema": "work-task-planning-index/v1", "requirement_id": "example", "revision": 1,
            "source": {"plan_sha256": "a".repeat(64), "hierarchy_selection_sha256": "b".repeat(64), "skill_selection_sha256": "c".repeat(64)},
            "current_task_id": "TASK-001", "tasks": [{"id": "TASK-001", "title": "Example", "goal": "Deliver the result.", "scope": ["Implementation"], "skill_id": null, "dependencies": [], "status": "planned", "boundary_revision": 1, "instructions_sha256": "d".repeat(64)}]});
        assert_eq!(
            validate_planning_index(&index).unwrap()["task_order"],
            json!(["TASK-001"])
        );
        for status in ["planned", "in_progress", "refined", "needs_review"] {
            let mut candidate = index.clone();
            candidate["tasks"][0]["status"] = json!(status);
            assert_eq!(
                validate_planning_index(&candidate).unwrap()["status"],
                "valid"
            );
        }
        let mut unsupported = index.clone();
        unsupported["tasks"][0]["status"] = json!("unsupported");
        assert_eq!(
            validate_planning_index(&unsupported)
                .unwrap_err()
                .reason_code,
            "invalid_draft_status"
        );
        let mut invalid_id = index.clone();
        invalid_id["tasks"][0]["id"] = json!("TASK-1");
        assert_eq!(
            validate_planning_index(&invalid_id)
                .unwrap_err()
                .reason_code,
            "invalid_draft_task_id"
        );
        let mut invalid_sha = index.clone();
        invalid_sha["source"]["plan_sha256"] = json!("A".repeat(64));
        assert_eq!(
            validate_planning_index(&invalid_sha)
                .unwrap_err()
                .reason_code,
            "invalid_sha256"
        );
        let mut active = index;
        active["tasks"][0]["status"] = json!("in_progress");
        let draft = json!({"schema": "work-task-draft/v1", "requirement_id": "example", "task_id": "TASK-001", "revision": 1,
            "boundary_revision": 1, "source": active["source"], "instructions_sha256": "d".repeat(64),
            "status": "in_progress", "notes": [], "confirmed_decisions": [], "tentative": [], "open_questions": [],
            "next_discussion_point": "Continue discussion."});
        assert_eq!(
            validate_task_draft(&draft, &active).unwrap()["status"],
            "valid"
        );
        let mut incomplete = draft;
        incomplete["next_discussion_point"] = Value::Null;
        assert_eq!(
            validate_task_draft(&incomplete, &active)
                .unwrap_err()
                .reason_code,
            "missing_draft_resume_point"
        );
    }

    #[test]
    fn malformed_draft_instruction_selections_are_rejected() {
        for value in [
            Value::Null,
            json!({}),
            json!({"selected_paths":[],"references":[],"extra":true}),
            json!({"selected_paths":["web","web"],"references":[]}),
            json!({"selected_paths":[],"references":[" "]}),
            json!({"selected_paths":[],"references":[[]]}),
            json!({"selected_paths":"web","references":[]}),
        ] {
            assert!(
                validate_draft_instruction_selection(&value).is_err(),
                "{value}"
            );
        }
    }

    fn python_contract_fixture() -> (Value, Value) {
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Update source","goal":"Produce the result.",
                "scope":["Source and its validation."],"skill_id":null,"dependencies":[],"status":"in_progress",
                "boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        let draft = json!({"schema":"work-task-draft/v1","requirement_id":"example","task_id":"TASK-001",
            "revision":1,"boundary_revision":1,"source":index["source"],"instructions_sha256":"d".repeat(64),
            "status":"in_progress","notes":[],"confirmed_decisions":[{"statement":"Keep the API.","rationale":"Preserve callers."}],
            "tentative":["Consider a focused regression test."],"open_questions":["Which input should the test cover?"],
            "next_discussion_point":"Confirm the regression input."});
        (index, draft)
    }

    #[test]
    fn python_draft_discussion_states_and_selection_cases() {
        let (index, draft) = python_contract_fixture();
        let before = (index.clone(), draft.clone());
        let valid = validate_task_draft(&draft, &index).unwrap();
        assert_eq!(valid["status"], "valid");
        assert_eq!(valid["planning_status"], "in_progress");
        assert!(valid.get("readiness").is_none());
        assert_eq!((index.clone(), draft.clone()), before);

        let mut selected = index.clone();
        selected["tasks"][0]["instruction_selection"] =
            json!({"selected_paths":[],"references":[]});
        assert_eq!(
            validate_task_draft(&draft, &selected).unwrap()["status"],
            "valid"
        );
        selected["tasks"][0]["instruction_selection"]["references"] = json!(["same", "same"]);
        assert_eq!(
            validate_task_draft(&draft, &selected)
                .unwrap_err()
                .reason_code,
            "invalid_source_selection"
        );

        let mut planned = index.clone();
        planned["tasks"][0]["status"] = json!("planned");
        planned["current_task_id"] = Value::Null;
        assert_eq!(validate_planning_index(&planned).unwrap()["task_count"], 1);
        let mut revised = index.clone();
        revised["revision"] = json!(8);
        assert_eq!(
            validate_task_draft(&draft, &revised).unwrap()["status"],
            "valid"
        );

        let mut refined_index = index.clone();
        refined_index["tasks"][0]["status"] = json!("refined");
        let mut refined = draft.clone();
        refined["status"] = json!("refined");
        refined["tentative"] = json!([]);
        refined["open_questions"] = json!([]);
        refined["next_discussion_point"] = Value::Null;
        assert_eq!(
            validate_task_draft(&refined, &refined_index).unwrap()["planning_status"],
            "refined"
        );
        for (field, value) in [
            ("tentative", json!(["Maybe"])),
            ("open_questions", json!(["Which?"])),
            ("next_discussion_point", json!("Continue")),
        ] {
            let mut unfinished = refined.clone();
            unfinished[field] = value;
            assert_eq!(
                validate_task_draft(&unfinished, &refined_index)
                    .unwrap_err()
                    .reason_code,
                "unfinished_refined_draft"
            );
        }
        for status in ["in_progress", "needs_review"] {
            let mut active_index = index.clone();
            active_index["tasks"][0]["status"] = json!(status);
            let mut active = draft.clone();
            active["status"] = json!(status);
            active["next_discussion_point"] = Value::Null;
            assert_eq!(
                validate_task_draft(&active, &active_index)
                    .unwrap_err()
                    .reason_code,
                "missing_draft_resume_point"
            );
        }
    }

    #[test]
    fn python_draft_identity_dependency_and_reference_cases() {
        let (index, draft) = python_contract_fixture();
        for field in [
            "plan_sha256",
            "hierarchy_selection_sha256",
            "skill_selection_sha256",
        ] {
            let mut changed = draft.clone();
            changed["source"][field] = json!("e".repeat(64));
            assert_eq!(
                validate_task_draft(&changed, &index)
                    .unwrap_err()
                    .reason_code,
                "draft_source_mismatch"
            );
        }
        let mut changed = draft.clone();
        changed["requirement_id"] = json!("other");
        assert_eq!(
            validate_task_draft(&changed, &index)
                .unwrap_err()
                .reason_code,
            "draft_source_mismatch"
        );
        for (field, value) in [
            ("boundary_revision", json!(2)),
            ("instructions_sha256", json!("e".repeat(64))),
            ("status", json!("needs_review")),
        ] {
            let mut changed = index.clone();
            changed["tasks"][0][field] = value;
            assert_eq!(
                validate_task_draft(&draft, &changed)
                    .unwrap_err()
                    .reason_code,
                "draft_index_mismatch"
            );
        }
        let mut unknown = draft.clone();
        unknown["task_id"] = json!("TASK-002");
        assert_eq!(
            validate_task_draft(&unknown, &index)
                .unwrap_err()
                .reason_code,
            "draft_task_not_in_index"
        );
        let mut unknown = index.clone();
        unknown["current_task_id"] = json!("TASK-002");
        assert_eq!(
            validate_planning_index(&unknown).unwrap_err().reason_code,
            "unknown_current_task"
        );
        let mut duplicate = index.clone();
        duplicate["tasks"]
            .as_array_mut()
            .unwrap()
            .push(index["tasks"][0].clone());
        assert_eq!(
            validate_planning_index(&duplicate).unwrap_err().reason_code,
            "duplicate_draft_task_id"
        );

        let mut ordered = index.clone();
        let mut second = index["tasks"][0].clone();
        second["id"] = json!("TASK-002");
        second["dependencies"] = json!(["TASK-001"]);
        second["status"] = json!("planned");
        ordered["tasks"].as_array_mut().unwrap().insert(0, second);
        assert_eq!(
            validate_planning_index(&ordered).unwrap()["task_order"],
            json!(["TASK-001", "TASK-002"])
        );
        for (dependencies, code) in [
            (
                json!(["TASK-001", "TASK-001"]),
                "duplicate_draft_dependency",
            ),
            (json!(["TASK-002"]), "invalid_task_dependency"),
            (json!(["TASK-003"]), "invalid_task_dependency"),
        ] {
            let mut changed = ordered.clone();
            changed["tasks"][0]["dependencies"] = dependencies;
            assert_eq!(
                validate_planning_index(&changed).unwrap_err().reason_code,
                code
            );
        }
        ordered["tasks"][1]["dependencies"] = json!(["TASK-002"]);
        assert_eq!(
            validate_planning_index(&ordered).unwrap_err().reason_code,
            "cyclic_task_dependency"
        );

        let mut referenced = index.clone();
        referenced["tasks"][0]["draft_ref"] =
            json!({"save_revision":1,"revision":1,"sha256":"a".repeat(64)});
        assert_eq!(
            validate_task_draft(&draft, &referenced).unwrap()["status"],
            "valid"
        );
        referenced["tasks"][0]["draft_ref"]["revision"] = json!(2);
        assert_eq!(
            validate_task_draft(&draft, &referenced)
                .unwrap_err()
                .reason_code,
            "draft_index_mismatch"
        );
        referenced["tasks"][0]["draft_ref"]["revision"] = json!(1);
        referenced["tasks"][0]["draft_ref"]["save_revision"] = json!(2);
        assert_eq!(
            validate_task_draft(&draft, &referenced)
                .unwrap_err()
                .reason_code,
            "invalid_draft_reference"
        );
        referenced["tasks"][0]["draft_ref"]["save_revision"] = json!(1);
        referenced["tasks"][0]["draft_ref"]["sha256"] = json!("bad");
        assert_eq!(
            validate_task_draft(&draft, &referenced)
                .unwrap_err()
                .reason_code,
            "invalid_sha256"
        );
    }

    #[test]
    fn python_draft_field_errors_and_formal_boundary() {
        let (index, draft) = python_contract_fixture();
        for (field, value, code) in [
            ("revision", json!(true), "invalid_draft_revision"),
            ("revision", json!(0), "invalid_draft_revision"),
            ("boundary_revision", json!("1"), "invalid_draft_revision"),
            ("task_id", json!("TASK-1"), "invalid_draft_task_id"),
            (
                "instructions_sha256",
                json!("D".repeat(64)),
                "invalid_sha256",
            ),
            ("status", json!("confirmed"), "invalid_draft_status"),
            ("status", json!({}), "invalid_draft_status"),
            ("notes", json!("text"), "invalid_draft_array"),
            (
                "schema",
                json!("work-task-collection-projection/v1"),
                "invalid_draft_schema",
            ),
        ] {
            let mut invalid = draft.clone();
            invalid[field] = value;
            assert_eq!(
                validate_task_draft(&invalid, &index)
                    .unwrap_err()
                    .reason_code,
                code
            );
        }
        let mut missing = draft.clone();
        missing.as_object_mut().unwrap().remove("open_questions");
        assert_eq!(
            validate_task_draft(&missing, &index)
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        let mut approval = draft.clone();
        approval["approved"] = json!(true);
        assert_eq!(
            validate_task_draft(&approval, &index)
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        let mut decision = draft.clone();
        decision["confirmed_decisions"][0]["rationale"] = json!(" ");
        assert_eq!(
            validate_task_draft(&decision, &index)
                .unwrap_err()
                .reason_code,
            "empty_text_value"
        );

        let raw = serde_json::to_vec(&draft).unwrap();
        assert_eq!(
            crate::task::item::validate_task_item(&draft, &raw, "TASK-001")
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
    }
}
