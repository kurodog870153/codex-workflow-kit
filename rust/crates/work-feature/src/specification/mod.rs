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
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::fingerprint;
use work_operations::derivation::transaction::{
    PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
};
use work_operations::execution::index::{render_execution_index, validate_execution_index};
use work_operations::specification::migration_diff::unified_diff;
use work_operations::specification::update::rebuild_execution_index;
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::artifact_paths::ArtifactPathRepository;
use crate::error::{ExitCode, WorkError};
use crate::instruction::InstructionSourceRepository;
use crate::ports::SourceSnapshotReader;
use crate::skill::{SkillRoot, SkillSnapshotRepository};
use crate::task::{CollectionInput, validate_collection};

pub struct SpecificationBaseline<'a> {
    pub task_path: &'a str,
    pub execution_path: &'a str,
    pub source_sha256: &'a str,
    pub source_evidence: &'a BTreeMap<String, Vec<u8>>,
    pub index_raw: &'a [u8],
    pub items: &'a BTreeMap<String, Vec<u8>>,
    pub execution_raw: &'a [u8],
    pub history: &'a BTreeMap<String, Vec<u8>>,
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
    old_index: &Value,
    index: &Value,
    old_items: &BTreeMap<String, Value>,
    items: &BTreeMap<String, Value>,
) -> Vec<String> {
    let mut result = BTreeSet::new();
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
    P: ArtifactPathRepository + SourceSnapshotReader,
{
    if request["schema"] != "work-spec-update-request/v1" {
        return Err(fail(
            "spec_update_schema",
            "Use work-spec-update-request/v1.",
        ));
    }
    work_operations::specification::update::validate_update_request(request).map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let expected = fingerprint::specification_baseline(
        baseline.source_sha256,
        baseline.index_raw,
        baseline.execution_raw,
        baseline.items,
    );
    if request["expected"] != expected {
        return Err(fail(
            "spec_update_source_changed",
            "The reviewed source fingerprints changed.",
        ));
    }
    let baseline_validation = validate_collection(
        instructions,
        skills,
        paths,
        skill_roots,
        CollectionInput {
            index_raw: baseline.index_raw,
            item_raw: baseline.items,
            index_path: baseline.task_path,
        },
    )?;
    if baseline_validation["source_sha256"] != baseline.source_sha256 {
        return Err(fail(
            "spec_update_source_changed",
            "The baseline must bind its validated Task provenance.",
        ));
    }
    let required_evidence = crate::task::source::evidence_paths(&parse(baseline.index_raw)?)?;
    if required_evidence.iter().cloned().collect::<BTreeSet<_>>()
        != baseline.source_evidence.keys().cloned().collect()
    {
        return Err(fail(
            "spec_update_source_changed",
            "The complete immutable Source proof is required.",
        ));
    }
    for (path, raw) in baseline.source_evidence {
        if paths.read_raw(path)? != *raw {
            return Err(fail(
                "spec_update_source_changed",
                "The immutable Source proof changed.",
            ));
        }
    }
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
    let artifacts = &request["task_index"]["artifacts"];
    if artifacts["task"] != baseline.task_path
        || artifacts["execution"]
            .as_str()
            .is_none_or(|path| format!("{path}/index.json") != baseline.execution_path)
    {
        return Err(fail(
            "spec_artifact_identity",
            "The candidate must route this revision to the same TASK collection.",
        ));
    }
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
    let context_fields = [
        "source",
        "hierarchy_selection",
        "skill_selection",
        "acceptance_criteria",
    ];
    if context_fields
        .iter()
        .any(|key| old_index[*key] != request["task_index"][*key])
    {
        let new_index = &request["task_index"];
        if new_index["source"]["kind"] != "snapshot" {
            return Err(fail(
                "source_confirmation_required",
                "Normal Revise requires one complete Snapshot.",
            ));
        }
        let new_source = json!({"snapshot":new_index["source"]["manifest"],
            "artifacts":new_index["artifacts"],"hierarchy_selection":new_index["hierarchy_selection"],
            "skill_selection":new_index["skill_selection"],"acceptance_criteria":new_index["acceptance_criteria"]});
        let active_ids: BTreeSet<String> = items.keys().cloned().collect();
        if affected.iter().cloned().collect::<BTreeSet<_>>() != active_ids {
            return Err(fail(
                "source_task_review_incomplete",
                "A context replacement must review every active Task.",
            ));
        }
        work_operations::task::draft_source::validate_context_confirmation(
            &old_index,
            &old_index,
            &new_source,
            &active_ids,
            &request["source_confirmation"],
        )
        .map_err(|e| WorkError::new(ExitCode::Contract, e.reason_code, e.message, e.details))?;
    }
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
        (baseline.task_path.to_owned(), baseline.index_raw.to_vec()),
        (
            baseline.execution_path.to_owned(),
            baseline.execution_raw.to_vec(),
        ),
    ]);
    let mut candidate = BTreeMap::from([
        (baseline.task_path.to_owned(), index_raw),
        (baseline.execution_path.to_owned(), execution_raw),
    ]);
    source.extend(baseline.source_evidence.clone());
    candidate.extend(baseline.source_evidence.clone());
    for path in crate::task::source::evidence_paths(&request["task_index"])? {
        let raw = paths.read_raw(&path)?;
        source.insert(path.clone(), raw.clone());
        candidate.insert(path, raw);
    }
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
            if path == baseline.task_path {
                30
            } else if path.contains("/tasks/") {
                20
            } else {
                40
            },
            path.clone(),
        )
    });
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
    }
    let derived = TransactionDeriver::derive(TransactionInput {
        kind: TransactionKind::Update,
        order: PublicationOrder::Artifact {
            task_index_path: baseline.task_path.into(),
        },
        request: request.clone(),
        artifacts: artifacts.clone(),
        affected_task_ids: affected.clone(),
        history: baseline.history.clone(),
        source,
        candidate,
    })
    .map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let approval = derived.approval_sha256;
    let transaction = derived.journal;
    let id = transaction["transaction_id"].clone();
    let old_items = baseline
        .items
        .iter()
        .map(|(id, raw)| parse(raw).map(|value| (id.clone(), value)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let changed = changed_fields(&old_index, &request["task_index"], &old_items, &items);
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
        "requirement_id":request["task_index"]["requirement_id"],"record_id":id,
        "approved_sha256":approval,"affected_task_ids":affected,"changed_fields":changed,
        "validation":{"task_collection":validation,"execution_index":"valid",
            "history":"validated"},"diff":review_diff,
        "lifecycle_impact":lifecycle_impact,
        "artifacts":artifacts,"candidate":{"task_index":request["task_index"],"task_items":request["task_items"]},
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
