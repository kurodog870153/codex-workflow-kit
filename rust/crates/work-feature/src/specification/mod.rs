//! Reviewed Specification update candidates and exact transaction evidence.

pub mod artifact_migration;
pub mod migration_prepare;
pub mod migration_preview;
pub mod migration_preview_project;
pub mod migration_publication;
pub mod migration_transaction;
pub mod reconciliation_input;
pub mod reconciliation_project;
pub mod reconciliation_publication;
pub mod reconciliation_semantic;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use work_operations::canonical::{parse_json_contract, sha256_hex};
use work_operations::execution::index::{render_execution_index, validate_execution_index};
use work_operations::plan::render_plan_value;
use work_operations::specification::migration_diff::unified_diff;
use work_operations::specification::transaction::{
    approval_sha256, derived_transaction_id, encode_snapshot, validate_transaction,
};
use work_operations::specification::update::{content_fingerprints, rebuild_execution_index};
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::error::{ExitCode, WorkError};
use crate::instruction::InstructionSourceRepository;
use crate::plan::PlanPathRepository;
use crate::skill::{SkillRoot, SkillSnapshotRepository};
use crate::task::{CollectionInput, validate_collection};

pub struct SpecificationBaseline<'a> {
    pub plan_path: &'a str,
    pub task_path: &'a str,
    pub execution_path: &'a str,
    pub plan_raw: &'a [u8],
    pub index_raw: &'a [u8],
    pub items: &'a BTreeMap<String, Vec<u8>>,
    pub execution_raw: &'a [u8],
    pub history_sha256: &'a BTreeMap<String, String>,
}

pub struct SpecificationPreview {
    pub result: Value,
    pub transaction: Value,
}

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

fn parse(raw: &[u8]) -> Result<Value, WorkError> {
    parse_json_contract(raw).map_err(|_| {
        fail(
            "invalid_json_contract",
            "A Specification source is not valid JSON.",
        )
    })
}

fn render_task_value(value: &Value, kind: TaskDocumentKind) -> Result<Vec<u8>, WorkError> {
    render_task(value, kind).map_err(|_| {
        fail(
            "invalid_contract_value",
            "A candidate TASK document cannot be rendered.",
        )
    })
}

fn changed_fields(
    old_plan: &Value,
    plan: &Value,
    old_index: &Value,
    index: &Value,
    old_items: &BTreeMap<String, Value>,
    items: &BTreeMap<String, Value>,
) -> Vec<String> {
    let mut result = BTreeSet::new();
    for key in old_plan
        .as_object()
        .into_iter()
        .flat_map(|row| row.keys())
        .chain(plan.as_object().into_iter().flat_map(|row| row.keys()))
    {
        if key != "changes" && old_plan.get(key) != plan.get(key) {
            result.insert(format!("/plan/{key}"));
        }
    }
    for key in old_index
        .as_object()
        .into_iter()
        .flat_map(|row| row.keys())
        .chain(index.as_object().into_iter().flat_map(|row| row.keys()))
    {
        if !["tasks", "spec_id", "readiness", "changes"].contains(&key.as_str())
            && old_index.get(key) != index.get(key)
        {
            result.insert(format!("/task_index/{key}"));
        }
    }
    for id in old_items.keys().chain(items.keys()) {
        if let (Some(before), Some(after)) = (old_items.get(id), items.get(id)) {
            for key in before
                .as_object()
                .into_iter()
                .flat_map(|row| row.keys())
                .chain(after.as_object().into_iter().flat_map(|row| row.keys()))
            {
                if before.get(key) != after.get(key) {
                    result.insert(format!("/task_items/{id}/{key}"));
                }
            }
        } else {
            result.insert(format!("/task_items/{id}"));
        }
    }
    result.into_iter().collect()
}

pub fn preview_update<H, S, P>(
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    request: &Value,
    baseline: SpecificationBaseline<'_>,
) -> Result<SpecificationPreview, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    if request["schema"] != "work-spec-update-request/v1" {
        return Err(fail(
            "spec_update_schema",
            "Use work-spec-update-request/v1.",
        ));
    }
    if !request["plan"].is_object()
        || !request["task_index"].is_object()
        || !request["task_items"].is_object()
    {
        return Err(fail(
            "spec_update_candidate",
            "A complete Plan, TASK index and item map are required.",
        ));
    }
    let expected = json!({"plan_sha256":sha256_hex(baseline.plan_raw),
        "task_index_sha256":sha256_hex(baseline.index_raw),
        "execution_index_sha256":sha256_hex(baseline.execution_raw),
        "task_item_sha256":baseline.items.iter().map(|(id, raw)|
            (id.clone(), sha256_hex(raw))).collect::<BTreeMap<_, _>>()});
    if request["expected"] != expected {
        return Err(fail(
            "spec_update_source_changed",
            "The reviewed source fingerprints changed.",
        ));
    }
    let old_plan = parse(baseline.plan_raw)?;
    let old_index = parse(baseline.index_raw)?;
    let old_execution = parse(baseline.execution_raw)?;
    validate_execution_index(&old_execution, baseline.execution_raw).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    if old_execution
        .get("lock")
        .is_some_and(|value| !value.is_null())
    {
        return Err(fail(
            "spec_update_lock_present",
            "The execution index already contains a lock.",
        ));
    }
    let artifacts = &request["plan"]["artifacts"];
    if artifacts["plan"] != baseline.plan_path
        || artifacts["task"] != baseline.task_path
        || artifacts["execution"]
            .as_str()
            .is_none_or(|path| format!("{path}/index.json") != baseline.execution_path)
    {
        return Err(fail(
            "spec_artifact_identity",
            "The Plan must route this revision to the same TASK collection.",
        ));
    }
    let plan_raw = render_plan_value(&request["plan"]).map_err(|_| {
        fail(
            "invalid_contract_value",
            "The candidate Plan cannot be rendered.",
        )
    })?;
    let index_raw = render_task_value(&request["task_index"], TaskDocumentKind::Index)?;
    let mut item_raw = BTreeMap::new();
    let mut items = BTreeMap::new();
    for (id, item) in request["task_items"].as_object().expect("checked items") {
        item_raw.insert(id.clone(), render_task_value(item, TaskDocumentKind::Item)?);
        items.insert(id.clone(), item.clone());
    }
    let validation = validate_collection(
        instructions,
        skills,
        paths,
        skill_roots,
        CollectionInput {
            index_raw: &index_raw,
            item_raw: &item_raw,
            index_path: baseline.task_path,
            source_plan_raw: &plan_raw,
        },
    )?;
    let affected = request["task_index"]["changes"]
        .as_array()
        .and_then(|rows| rows.last())
        .and_then(|row| row["affected_ids"].as_array())
        .ok_or_else(|| {
            fail(
                "spec_update_candidate",
                "The candidate TASK index needs reviewed change evidence.",
            )
        })?
        .iter()
        .map(|row| {
            row.as_str()
                .map(str::to_owned)
                .ok_or_else(|| fail("spec_update_candidate", "Affected TASK IDs are invalid."))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let change_id = request["task_index"]["changes"]
        .as_array()
        .and_then(|rows| rows.last())
        .and_then(|row| row["id"].as_str())
        .ok_or_else(|| {
            fail(
                "spec_update_candidate",
                "The candidate TASK change ID is missing.",
            )
        })?;
    let rebuilt = rebuild_execution_index(
        &old_execution,
        &validation["collection_contract"],
        &validation,
        &affected,
        change_id,
    )
    .map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let execution_raw = render_execution_index(&rebuilt).map_err(|_| {
        fail(
            "invalid_contract_value",
            "The candidate execution index cannot be rendered.",
        )
    })?;
    validate_execution_index(&rebuilt, &execution_raw).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let directory = baseline
        .task_path
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    let mut source = BTreeMap::from([
        (baseline.plan_path.to_owned(), baseline.plan_raw.to_vec()),
        (baseline.task_path.to_owned(), baseline.index_raw.to_vec()),
        (
            baseline.execution_path.to_owned(),
            baseline.execution_raw.to_vec(),
        ),
    ]);
    let mut candidate = BTreeMap::from([
        (baseline.plan_path.to_owned(), plan_raw),
        (baseline.task_path.to_owned(), index_raw),
        (baseline.execution_path.to_owned(), execution_raw),
    ]);
    for (id, raw) in baseline.items {
        source.insert(format!("{directory}/tasks/{id}.json"), raw.clone());
    }
    for (id, raw) in item_raw {
        candidate.insert(format!("{directory}/tasks/{id}.json"), raw);
    }
    let mut all_paths = source
        .keys()
        .chain(candidate.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    all_paths.sort_by_key(|path| {
        (
            if path.contains("/tasks/") {
                20
            } else if path == baseline.plan_path {
                10
            } else if path == baseline.task_path {
                30
            } else {
                40
            },
            path.clone(),
        )
    });
    let mut files = Vec::new();
    let mut review_diff = Vec::new();
    for path in all_paths {
        let before = source.get(&path);
        let after = candidate.get(&path);
        if before == after {
            continue;
        }
        review_diff.push(unified_diff(
            &path,
            before.map(Vec::as_slice),
            after.map(Vec::as_slice),
        ));
        let phase = if path.contains("/tasks/") {
            20
        } else if path == baseline.plan_path {
            10
        } else if path == baseline.task_path {
            30
        } else {
            40
        };
        let mut row = json!({"phase":phase,"path":path,"operation":
            if before.is_none() {"add"} else if after.is_none() {"remove"} else {"replace"}});
        if let Some(raw) = before {
            row["before"] = encode_snapshot(raw);
        }
        if let Some(raw) = after {
            row["after"] = encode_snapshot(raw);
        }
        files.push(row);
    }
    let metadata = json!({"request":request,"artifacts":artifacts,"affected_task_ids":affected,
        "history_sha256":baseline.history_sha256,"source_sha256":content_fingerprints(&source),
        "candidate_sha256":content_fingerprints(&candidate)});
    let files = Value::Array(files);
    let approval = approval_sha256(&files, &metadata);
    let id = derived_transaction_id("UPDATE", &approval).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let transaction = json!({"schema":"work-spec-transaction/v1","transaction_id":id,
        "approval_sha256":approval,"state":"prepared","published_count":0,
        "metadata":metadata,"files":files});
    validate_transaction(&transaction).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let old_items = baseline
        .items
        .iter()
        .map(|(id, raw)| parse(raw).map(|value| (id.clone(), value)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let changed = changed_fields(
        &old_plan,
        &request["plan"],
        &old_index,
        &request["task_index"],
        &old_items,
        &items,
    );
    let old_rows = old_execution["tasks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let next_rows = rebuilt["tasks"].as_array().cloned().unwrap_or_default();
    let mut lifecycle_impact = Vec::new();
    for id in old_rows
        .iter()
        .chain(next_rows.iter())
        .filter_map(|row| row["id"].as_str())
    {
        if lifecycle_impact
            .iter()
            .any(|row: &Value| row["task_id"] == id)
        {
            continue;
        }
        let old_row = old_rows.iter().find(|row| row["id"] == id);
        let next_row = next_rows.iter().find(|row| row["id"] == id);
        let before = old_row
            .map(|row| row["status"].clone())
            .unwrap_or(Value::Null);
        let after = next_row
            .map(|row| row["status"].clone())
            .unwrap_or(Value::Null);
        if before != after || affected.iter().any(|task_id| task_id == id) {
            lifecycle_impact.push(json!({"task_id":id,"before":before,"after":after,
                "reason_before":old_row.map(|row| row["status_reason"].clone()).unwrap_or(Value::Null),
                "reason_after":next_row.map(|row| row["status_reason"].clone()).unwrap_or(Value::Null)}));
        }
    }
    let result = work_model::specification::verified::<work_model::specification::SpecUpdate>(
        json!({"schema":"work-spec-update/v1","status":"valid",
        "requirement_id":request["plan"]["requirement_id"],"record_id":id,
        "approved_sha256":approval,"affected_task_ids":affected,"changed_fields":changed,
        "validation":{"task_collection":validation,"execution_index":"valid",
            "history":"validated"},"diff":review_diff,
        "lifecycle_impact":lifecycle_impact,
        "artifacts":artifacts,"candidate":{"plan":request["plan"],
            "task_index":request["task_index"],"task_items":request["task_items"]},
        "transaction":transaction,"file_readiness":"requires_execute_preflight",
        "next_step":{"command":"specification apply","input":"same_request","approved_sha256":approval}}),
    );
    let _: work_model::specification::SpecUpdateRequest = serde_json::from_value(request.clone())
        .expect("validated Specification update request matches its model");
    Ok(SpecificationPreview {
        result,
        transaction,
    })
}

pub mod reconciliation;
