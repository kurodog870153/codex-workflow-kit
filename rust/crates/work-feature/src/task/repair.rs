//! Reviewed TASK repair transaction built from complete source evidence.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use work_operations::canonical::parse_json_contract;
use work_operations::execution::index::{render_execution_index, validate_execution_index};
use work_operations::protocol::valid_sha256;
use work_operations::specification::transaction::{
    approval_sha256, encode_snapshot, validate_transaction,
};
use work_operations::task::ordering::{TaskDocumentKind, render_task};
use work_operations::task::repair::{
    content_fingerprints, evidence_fingerprints, rebuild_execution_index, transaction_id,
};

use crate::error::{ExitCode, WorkError};
use crate::instruction::InstructionSourceRepository;
use crate::plan::PlanPathRepository;
use crate::skill::{SkillRoot, SkillSnapshotRepository};
use crate::task::{CollectionInput, validate_collection};

pub struct RepairInput<'a> {
    pub request: &'a Value,
    pub sources: &'a BTreeMap<String, Vec<u8>>,
    pub plan_raw: &'a [u8],
    pub history_sha256: &'a BTreeMap<String, String>,
}

pub struct PreparedRepair {
    pub transaction: Value,
    pub changed_paths: Vec<String>,
    pub affected_task_ids: Vec<String>,
}

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

fn render(value: &Value, kind: TaskDocumentKind) -> Result<Vec<u8>, WorkError> {
    render_task(value, kind).map_err(|_| {
        fail(
            "invalid_contract_value",
            "The TASK repair document cannot be rendered.",
        )
    })
}

pub fn validate_repair_request(request: &Value) -> Result<(), WorkError> {
    let fields = request.as_object().ok_or_else(|| {
        fail(
            "invalid_contract_value",
            "TASK repair requires a JSON object.",
        )
    })?;
    let required = [
        "schema",
        "stage",
        "requirement_id",
        "artifacts",
        "expected",
        "decisions",
        "task_index",
        "task_items",
    ];
    if fields.len() != required.len() || required.iter().any(|field| !fields.contains_key(*field)) {
        return Err(fail(
            "invalid_object_fields",
            "The TASK repair request has missing or unknown fields.",
        ));
    }
    if request["schema"] != "work-task-repair-request/v1" {
        return Err(fail(
            "task_repair_schema",
            "Use work-task-repair-request/v1.",
        ));
    }
    if !matches!(request["stage"].as_str(), Some("format" | "complete"))
        || request["requirement_id"]
            .as_str()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(fail(
            "invalid_contract_value",
            "The TASK repair identity is invalid.",
        ));
    }
    let artifacts = request["artifacts"]
        .as_object()
        .ok_or_else(|| fail("task_repair_paths", "Repair requires artifact routing."))?;
    if artifacts.len() != 3
        || ["plan", "task", "execution"].iter().any(|key| {
            artifacts
                .get(*key)
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        })
    {
        return Err(fail(
            "task_repair_paths",
            "Repair requires normalized artifact paths.",
        ));
    }
    let expected = request["expected"].as_object().ok_or_else(|| {
        fail(
            "task_repair_expected",
            "Expected evidence must map paths to SHA-256 fingerprints or null.",
        )
    })?;
    if expected.iter().any(|(path, value)| {
        path.is_empty() || !value.is_null() && value.as_str().is_none_or(|hash| !valid_sha256(hash))
    }) {
        return Err(fail(
            "task_repair_expected",
            "Expected evidence must map paths to SHA-256 fingerprints or null.",
        ));
    }
    let decisions = request["decisions"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            fail(
                "task_repair_decisions",
                "TASK repair requires explicit reviewed decisions.",
            )
        })?;
    if decisions.iter().any(|row| {
        row.as_object().is_none_or(|object| {
            object.len() != 2
                || ["location", "decision"].iter().any(|key| {
                    object
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                })
        })
    }) {
        return Err(fail(
            "task_repair_decisions",
            "TASK repair requires explicit reviewed decisions.",
        ));
    }
    if request["task_index"].as_object().is_none()
        || request["task_items"]
            .as_object()
            .is_none_or(|items| items.is_empty())
    {
        return Err(fail(
            "task_repair_candidate",
            "TASK repair requires a complete index and item map.",
        ));
    }
    Ok(())
}

pub fn build_repair_transaction<H, S, P>(
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    input: RepairInput<'_>,
) -> Result<PreparedRepair, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    let request = input.request;
    validate_repair_request(request)?;
    let artifacts = &request["artifacts"];
    let index_path = artifacts["task"].as_str().ok_or_else(|| {
        fail(
            "task_repair_paths",
            "A normalized TASK index path is required.",
        )
    })?;
    let execution_dir = artifacts["execution"].as_str().ok_or_else(|| {
        fail(
            "task_repair_paths",
            "A normalized execution path is required.",
        )
    })?;
    let execution_path = format!("{execution_dir}/index.json");
    let index = request["task_index"].as_object().ok_or_else(|| {
        fail(
            "task_repair_candidate",
            "TASK repair requires a complete formal index.",
        )
    })?;
    let items = request["task_items"]
        .as_object()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| {
            fail(
                "task_repair_candidate",
                "TASK repair requires a complete TASK item map.",
            )
        })?;
    if request["requirement_id"] != index["requirement_id"] || artifacts != &index["artifacts"] {
        return Err(fail(
            "task_repair_identity",
            "Repair must preserve requirement and artifact routing.",
        ));
    }
    let index_raw = render(&request["task_index"], TaskDocumentKind::Index)?;
    let mut item_raw = BTreeMap::new();
    let directory = index_path.rsplit_once('/').map_or("", |(parent, _)| parent);
    let mut candidates = BTreeMap::from([(index_path.to_owned(), index_raw.clone())]);
    for (task_id, item) in items {
        let raw = render(item, TaskDocumentKind::Item)?;
        candidates.insert(format!("{directory}/tasks/{task_id}.json"), raw.clone());
        item_raw.insert(task_id.clone(), raw);
    }
    let validation = validate_collection(
        instructions,
        skills,
        paths,
        skill_roots,
        CollectionInput {
            index_raw: &index_raw,
            item_raw: &item_raw,
            index_path,
            source_plan_raw: input.plan_raw,
        },
    )?;
    let _: work_model::task::request::TaskRepairRequest = serde_json::from_value(request.clone())
        .map_err(|_| {
        fail(
            "task_repair_candidate",
            "TASK repair requires a complete typed collection.",
        )
    })?;
    let mut paths = input.sources.keys().cloned().collect::<BTreeSet<_>>();
    paths.extend(candidates.keys().cloned());
    paths.insert(execution_path.clone());
    let evidence = evidence_fingerprints(input.sources, &paths.into_iter().collect::<Vec<_>>());
    if request["expected"] != serde_json::to_value(&evidence).expect("evidence serializes") {
        return Err(fail(
            "task_repair_source_changed",
            "The reviewed artifact set changed.",
        ));
    }
    let decisions = request["decisions"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            fail(
                "task_repair_decisions",
                "TASK repair requires explicit reviewed decisions.",
            )
        })?;
    let locations = decisions
        .iter()
        .filter_map(|row| row["location"].as_str())
        .collect::<BTreeSet<_>>();
    for path in input
        .sources
        .keys()
        .filter(|path| path.contains("/tasks/") && !candidates.contains_key(*path))
    {
        let name = path.rsplit('/').next().unwrap_or("");
        if !locations.contains(format!("/orphans/{name}").as_str()) {
            return Err(fail(
                "task_repair_orphan_decision",
                "Orphans require explicit removal decisions.",
            ));
        }
    }
    let old_raw = input.sources.get(&execution_path).ok_or_else(|| {
        fail(
            "task_repair_execution_missing",
            "TASK repair requires preserved execution index evidence.",
        )
    })?;
    let old = parse_json_contract(old_raw).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The execution index is not valid JSON.",
        )
    })?;
    validate_execution_index(&old, old_raw).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let affected = if candidates
        .iter()
        .any(|(path, raw)| input.sources.get(path) != Some(raw))
    {
        item_raw.keys().cloned().collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let request_raw = render(request, TaskDocumentKind::RepairRequest)?;
    let id = transaction_id(&request_raw);
    let rebuilt = rebuild_execution_index(
        &old,
        &validation["collection_contract"],
        &validation,
        &affected,
        &id,
    )
    .map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let rebuilt_raw = render_execution_index(&rebuilt).map_err(|_| {
        fail(
            "invalid_contract_value",
            "The repaired execution index cannot be rendered.",
        )
    })?;
    validate_execution_index(&rebuilt, &rebuilt_raw).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    candidates.insert(execution_path.clone(), rebuilt_raw);
    let mut rows = Vec::new();
    let mut all_paths = input
        .sources
        .keys()
        .chain(candidates.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    all_paths.sort_by_key(|path| {
        (
            if path.contains("/tasks/") {
                20
            } else if path == index_path {
                30
            } else {
                40
            },
            path.clone(),
        )
    });
    for path in all_paths {
        let before = input.sources.get(&path);
        let after = candidates.get(&path);
        if before == after {
            continue;
        }
        let phase = if path.contains("/tasks/") {
            20
        } else if path == index_path {
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
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(fail(
            "task_repair_no_change",
            "The repair candidate makes no change.",
        ));
    }
    let files = Value::Array(rows);
    let metadata = json!({"request":request,"artifacts":artifacts,"affected_task_ids":affected,
        "history_sha256":input.history_sha256,"source_sha256":content_fingerprints(input.sources),
        "candidate_sha256":content_fingerprints(&candidates)});
    let transaction = json!({"schema":"work-spec-transaction/v1","transaction_id":id,
        "approval_sha256":approval_sha256(&files, &metadata),"state":"prepared",
        "published_count":0,"metadata":metadata,"files":files});
    validate_transaction(&transaction).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let changed_paths = transaction["files"]
        .as_array()
        .expect("validated files")
        .iter()
        .filter_map(|row| row["path"].as_str().map(str::to_owned))
        .collect();
    Ok(PreparedRepair {
        transaction,
        changed_paths,
        affected_task_ids: affected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_expected_fingerprints_reject_invalid_values_before_sources() {
        let request = json!({
            "schema":"work-task-repair-request/v1","stage":"complete","requirement_id":"example",
            "artifacts":{"plan":"plans/example.json","task":"tasks/example/index.json","execution":"executions/example"},
            "expected":{"index.json":"0".repeat(64)},
            "decisions":[{"location":"/","decision":"Use the reviewed collection."}],
            "task_index":{},"task_items":{"TASK-001":{}}
        });
        validate_repair_request(&request).unwrap();
        for invalid in [
            json!("A".repeat(64)),
            json!("0".repeat(63)),
            json!(1),
            json!(false),
        ] {
            let mut changed = request.clone();
            changed["expected"]["index.json"] = invalid;
            assert_eq!(
                validate_repair_request(&changed).unwrap_err().reason_code,
                "task_repair_expected"
            );
        }
        let mut with_missing = request.clone();
        with_missing["expected"]["missing.json"] = Value::Null;
        validate_repair_request(&with_missing).unwrap();
        assert!(
            with_missing["expected"]
                .as_object()
                .unwrap()
                .contains_key("missing.json")
        );
        assert!(with_missing["expected"]["missing.json"].is_null());
    }
}
