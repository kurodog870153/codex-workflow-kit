//! TASK repair source collection and recoverable publication.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::instruction::{
    load as load_instructions, select as select_instructions, task_document_selection,
};
use work_feature::ports::{ArtifactStore, WriterLock};
use work_feature::skill::SkillRoot;
use work_feature::task::load_collection;
use work_feature::task::repair::{
    PreparedRepair, RepairInput, build_repair_transaction, validate_repair_request,
};
use work_operations::canonical::parse_json_contract;
use work_operations::specification::transaction::{render_transaction, validate_transaction};
use work_operations::task::candidate::{build_semantic_candidate, build_semantic_patch};
use work_operations::task::ordering::{TaskDocumentKind, render_task};
use work_operations::task::repair::{evidence_fingerprints, transaction_id};

use crate::files::LocalFiles;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::{
    execution_history_fingerprints, publish_journal, require_no_spec_update, storage_path,
    write_journal,
};
use crate::task::diagnostics::diagnose_task_collection;
use crate::task::storage::LocalTaskStorage;
use crate::writer_lock::LocalWriterLock;

pub enum RepairOperation {
    Validate,
    Apply,
    Recover,
}

pub struct RepairProjectRequest<'a> {
    pub raw: &'a [u8],
    pub operation: RepairOperation,
    pub approved_sha256: Option<&'a str>,
}

pub struct PrepareRepairRequest<'a> {
    pub raw: &'a [u8],
    pub output_file: Option<&'a Path>,
}

fn fail(code: ExitCode, reason: &str, message: &str) -> WorkError {
    WorkError::new(code, reason, message, json!({}))
}

fn required_path<'a>(value: &'a Value, key: &str) -> Result<&'a str, WorkError> {
    value[key].as_str().ok_or_else(|| {
        fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_paths",
            "Repair requires normalized artifact paths.",
        )
    })
}

pub(crate) fn work_json_files(root: &Path) -> Result<Vec<String>, WorkError> {
    let mut pending = vec!["outputs/work".to_owned()];
    let mut files = Vec::new();
    while let Some(relative) = pending.pop() {
        let directory = storage_path(root, &relative)?;
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory).map_err(|_| {
            fail(
                ExitCode::IoFailure,
                "file_read_failed",
                "The managed Work directory cannot be inspected.",
            )
        })? {
            let entry = entry.map_err(|_| {
                fail(
                    ExitCode::IoFailure,
                    "file_read_failed",
                    "The managed Work directory cannot be inspected.",
                )
            })?;
            let path = entry.path();
            if path.is_symlink() {
                continue;
            }
            let child = format!("{relative}/{}", entry.file_name().to_string_lossy());
            if path.is_dir() {
                pending.push(child);
            } else if path.is_file() && path.extension().is_some_and(|value| value == "json") {
                files.push(child);
            }
        }
    }
    files.sort();
    Ok(files)
}

pub fn resolve_repair_artifacts(root: &Path, requirement_id: &str) -> Result<Value, WorkError> {
    let mut bindings = Vec::new();
    let mut execution_paths = Vec::new();
    for relative in work_json_files(root)? {
        let bytes = LocalFiles.read_raw(&storage_path(root, &relative)?)?;
        let Ok(document) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if document["requirement_id"] != requirement_id {
            continue;
        }
        let schema = document["schema"].as_str().unwrap_or("");
        if schema == "work-execution-index/v1" {
            execution_paths.push(relative);
            continue;
        }
        let identity = match schema {
            "work-plan/v1" => "plan",
            "work-task-index/v1" => "task",
            _ => continue,
        };
        let artifacts = &document["artifacts"];
        if artifacts.as_object().is_none_or(|fields| {
            fields.len() != 3
                || !["plan", "task", "execution"]
                    .iter()
                    .all(|field| fields.contains_key(*field))
        }) || artifacts[identity] != relative
        {
            return Err(fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_binding_invalid",
                "A matching repair source has a conflicting artifact binding.",
            ));
        }
        bindings.push(artifacts.clone());
    }
    if bindings.is_empty()
        || bindings.len() > 2
        || bindings.iter().any(|value| value != &bindings[0])
    {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_source_ambiguous",
            "Repair sources must establish one consistent artifact binding.",
        ));
    }
    let expected = format!(
        "{}/index.json",
        bindings[0]["execution"].as_str().unwrap_or("")
    );
    if execution_paths.len() > 1 || execution_paths.iter().any(|path| path != &expected) {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_execution_binding_conflict",
            "The execution index conflicts with the source artifact binding.",
        ));
    }
    Ok(bindings.remove(0))
}

fn read_sources(
    root: &Path,
    index: &str,
    execution: &str,
) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
    let directory = index.rsplit_once('/').map_or("", |(parent, _)| parent);
    let item_dir = format!("{directory}/tasks");
    let folder = storage_path(root, &item_dir)?;
    let mut result = BTreeMap::new();
    if folder.exists() {
        if folder.is_symlink() || !folder.is_dir() {
            return Err(fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_item_directory",
                "The TASK item storage is not a safe directory.",
            ));
        }
        for entry in fs::read_dir(&folder).map_err(|_| {
            fail(
                ExitCode::IoFailure,
                "file_read_failed",
                "The TASK item directory cannot be read.",
            )
        })? {
            let entry = entry.map_err(|_| {
                fail(
                    ExitCode::IoFailure,
                    "file_read_failed",
                    "The TASK item directory cannot be read.",
                )
            })?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_symlink() || !path.is_file() || !name.ends_with(".json") {
                return Err(fail(
                    ExitCode::ArtifactIntegrity,
                    "task_repair_unknown_storage",
                    "TASK repair does not remove unknown non-TASK storage.",
                ));
            }
            let relative = format!("{item_dir}/{name}");
            result.insert(
                relative.clone(),
                LocalFiles.read_raw(&storage_path(root, &relative)?)?,
            );
        }
    }
    for relative in [index.to_owned(), format!("{execution}/index.json")] {
        let path = storage_path(root, &relative)?;
        if path.is_file() {
            result.insert(relative, LocalFiles.read_raw(&path)?);
        }
    }
    Ok(result)
}

fn strings(value: &Value) -> Result<Vec<String>, WorkError> {
    value
        .as_array()
        .ok_or_else(|| {
            fail(
                ExitCode::Contract,
                "invalid_string_array",
                "Expected a list of strings.",
            )
        })?
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                fail(
                    ExitCode::Contract,
                    "invalid_string_array",
                    "Expected a list of strings.",
                )
            })
        })
        .collect()
}

fn positions(value: &Value, ids: &[String], nonempty: bool) -> Result<Vec<String>, WorkError> {
    let rows = value
        .as_array()
        .filter(|rows| !nonempty || !rows.is_empty())
        .ok_or_else(|| {
            fail(
                ExitCode::Contract,
                "invalid_semantic_position",
                "Positions must identify existing one-based items.",
            )
        })?;
    let result = rows
        .iter()
        .map(|row| {
            row.as_u64()
                .and_then(|position| ids.get(position.wrapping_sub(1) as usize))
                .cloned()
                .ok_or_else(|| {
                    fail(
                        ExitCode::Contract,
                        "invalid_semantic_position",
                        "Positions must identify existing one-based items.",
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if result
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != result.len()
    {
        return Err(fail(
            ExitCode::Contract,
            "invalid_semantic_position",
            "Positions must be unique.",
        ));
    }
    Ok(result)
}

fn traceability(value: &Value, plan: &Value) -> Result<Value, WorkError> {
    let keys = [
        ("goal", "goals"),
        ("deliverable", "deliverables"),
        ("acceptance", "acceptance_criteria"),
        ("milestone", "milestones"),
    ];
    let fields = value.as_object().ok_or_else(|| {
        fail(
            ExitCode::Contract,
            "invalid_semantic_object",
            "Traceability needs a semantic object.",
        )
    })?;
    if fields.keys().any(|field| {
        !keys
            .iter()
            .any(|(name, _)| field == &format!("{name}_positions"))
    }) || keys[..3]
        .iter()
        .any(|(name, _)| !fields.contains_key(&format!("{name}_positions")))
    {
        return Err(fail(
            ExitCode::Contract,
            "invalid_object_fields",
            "Traceability has missing or unknown position fields.",
        ));
    }
    let mut result = serde_json::Map::new();
    for (name, group) in keys {
        if let Some(source) = fields.get(&format!("{name}_positions")) {
            let ids = plan[group]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| row["id"].as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            result.insert(format!("{name}_ids"), json!(positions(source, &ids, true)?));
        }
    }
    Ok(Value::Object(result))
}

fn index_decisions(value: &Value, index: &Value) -> Result<Value, WorkError> {
    let rows = value.as_array().ok_or_else(|| {
        fail(
            ExitCode::Contract,
            "invalid_semantic_items",
            "Index decisions must be a semantic array.",
        )
    })?;
    let old = index["decisions"].as_array().cloned().unwrap_or_default();
    let ids = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let mut next = old
        .iter()
        .filter_map(|row| row["id"].as_str())
        .filter_map(|id| id.rsplit_once('-')?.1.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    let mut used = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for row in rows {
        let object = row.as_object().ok_or_else(|| {
            fail(
                ExitCode::Contract,
                "invalid_semantic_object",
                "An index decision must be an object.",
            )
        })?;
        if !["key", "statement", "rationale", "task_positions"]
            .iter()
            .all(|field| object.contains_key(*field))
            || object.keys().any(|field| {
                ![
                    "key",
                    "statement",
                    "rationale",
                    "task_positions",
                    "existing_position",
                ]
                .contains(&field.as_str())
            })
        {
            return Err(fail(
                ExitCode::Contract,
                "invalid_object_fields",
                "The index decision has missing or unknown fields.",
            ));
        }
        let id = if let Some(position) = row.get("existing_position") {
            let position = position
                .as_u64()
                .filter(|position| *position > 0 && (*position as usize) <= old.len())
                .ok_or_else(|| {
                    fail(
                        ExitCode::Contract,
                        "invalid_semantic_position",
                        "An existing position must identify one retained record.",
                    )
                })? as usize;
            if !used.insert(position) {
                return Err(fail(
                    ExitCode::Contract,
                    "duplicate_semantic_position",
                    "An existing item may be retained once.",
                ));
            }
            old[position - 1]["id"].clone()
        } else {
            next += 1;
            json!(format!("TASK-DECISION-{next:03}"))
        };
        result.push(
            json!({"id":id,"statement":row["statement"],"rationale":row["rationale"],
            "task_ids":positions(&row["task_positions"], &ids, false)?}),
        );
    }
    Ok(Value::Array(result))
}

fn semantic_only(value: &Value) -> Result<(), WorkError> {
    const FORMAL: &[&str] = &[
        "id",
        "command_ids",
        "command_id",
        "validation_id",
        "acceptance_ids",
        "goal_ids",
        "deliverable_ids",
        "milestone_ids",
        "task_ids",
        "canonical_sha256",
        "raw_sha256",
        "instructions_sha256",
        "fingerprint",
        "revision",
        "status",
        "transaction_id",
        "spec_id",
        "change_id",
    ];
    const PREFIXES: &[&str] = &[
        "TASK",
        "STEP",
        "CMD",
        "VAL",
        "OP",
        "FILE",
        "INPUT",
        "RISK",
        "GOAL",
        "SCOPE",
        "CONSTRAINT",
        "DEPENDENCY",
        "MILESTONE",
        "DELIVERABLE",
        "ACCEPTANCE",
        "PLAN-DECISION",
        "TASK-DECISION",
        "PLAN-CHANGE",
        "TASK-CHANGE",
        "TASK-SPEC",
    ];
    match value {
        Value::Object(fields) => {
            if fields.keys().any(|key| FORMAL.contains(&key.as_str())) {
                return Err(fail(
                    ExitCode::Contract,
                    "invalid_contract_value",
                    "Semantic input cannot contain formal fields.",
                ));
            }
            for child in fields.values() {
                semantic_only(child)?;
            }
        }
        Value::Array(rows) => {
            for row in rows {
                semantic_only(row)?;
            }
        }
        Value::String(text) => {
            if let Some((prefix, suffix)) = text.rsplit_once('-') {
                if PREFIXES.contains(&prefix)
                    && suffix.len() == 3
                    && suffix.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Err(fail(
                        ExitCode::Contract,
                        "invalid_contract_value",
                        "Semantic input cannot contain formal IDs.",
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_prepare_request(value: &Value) -> Result<(), WorkError> {
    let fields = value.as_object().ok_or_else(|| {
        fail(
            ExitCode::Contract,
            "invalid_contract_value",
            "TASK repair preparation must be an object.",
        )
    })?;
    if value["schema"] != "work-task-repair-prepare-request/v1" {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_prepare_schema",
            "Use work-task-repair-prepare-request/v1.",
        ));
    }
    let required = ["schema", "stage", "requirement_id", "decisions"];
    let optional = ["edits", "missing_task"];
    if required.iter().any(|field| !fields.contains_key(*field))
        || fields
            .keys()
            .any(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
    {
        return Err(fail(
            ExitCode::Contract,
            "invalid_object_fields",
            "TASK repair preparation has missing or unknown fields.",
        ));
    }
    if !matches!(value["stage"].as_str(), Some("format" | "complete"))
        || value["requirement_id"]
            .as_str()
            .is_none_or(|text| text.trim().is_empty())
    {
        return Err(fail(
            ExitCode::Contract,
            "invalid_contract_value",
            "TASK repair preparation needs a stage and requirement.",
        ));
    }
    let decisions = value["decisions"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_decisions",
                "TASK repair requires explicit reviewed decisions.",
            )
        })?;
    if decisions.iter().any(|row| {
        row.as_object().is_none_or(|fields| {
            fields.len() != 2
                || ["location", "decision"].iter().any(|key| {
                    fields
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_none_or(|text| text.is_empty())
                })
        })
    }) {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_decisions",
            "TASK repair requires explicit reviewed decisions.",
        ));
    }
    if value.get("edits").is_some() && value.get("missing_task").is_some() {
        return Err(fail(
            ExitCode::Contract,
            "invalid_contract_value",
            "Use edits or one missing semantic TASK, not both.",
        ));
    }
    if let Some(edits) = value.get("edits") {
        let rows = edits.as_array().ok_or_else(|| {
            fail(
                ExitCode::Contract,
                "invalid_contract_value",
                "Repair edits must be an array.",
            )
        })?;
        for edit in rows {
            let fields = edit.as_object().ok_or_else(|| {
                fail(
                    ExitCode::Contract,
                    "invalid_contract_value",
                    "Repair edits must be objects.",
                )
            })?;
            if !fields.contains_key("field")
                || fields.keys().any(|field| {
                    !["field", "task_id", "after", "semantic_after", "remove"]
                        .contains(&field.as_str())
                })
            {
                return Err(fail(
                    ExitCode::Contract,
                    "invalid_object_fields",
                    "Repair edits have missing or unknown fields.",
                ));
            }
            let field = edit["field"].as_str().unwrap_or("");
            let item = edit["task_id"].as_str().is_some();
            let simple = matches!(
                (item, field),
                (true, "title" | "goal") | (false, "title" | "summary")
            );
            let semantic = if item {
                [
                    "traceability",
                    "dependencies",
                    "inputs",
                    "decisions",
                    "files",
                    "risks",
                    "steps",
                    "commands",
                    "operations",
                    "validations",
                ]
                .contains(&field)
            } else {
                ["decisions", "execution_defaults"].contains(&field)
            };
            if !simple && !semantic {
                return Err(fail(
                    ExitCode::Contract,
                    "invalid_contract_value",
                    "This repair field has no semantic operation.",
                ));
            }
            let remove = edit["remove"] == true;
            let valid = if remove {
                !simple && !fields.contains_key("after") && !fields.contains_key("semantic_after")
            } else if simple {
                edit["after"].is_string() && !fields.contains_key("semantic_after")
            } else {
                fields.contains_key("semantic_after")
                    && !edit["semantic_after"].is_null()
                    && !fields.contains_key("after")
            };
            if !valid {
                return Err(fail(
                    ExitCode::Contract,
                    "invalid_contract_value",
                    "Repair edit evidence is inconsistent with its field.",
                ));
            }
            if !remove && semantic {
                semantic_only(&edit["semantic_after"])?;
            }
        }
    }
    if let Some(missing) = value.get("missing_task") {
        let fields = missing.as_object().ok_or_else(|| {
            fail(
                ExitCode::Contract,
                "invalid_contract_value",
                "A missing TASK needs semantic evidence.",
            )
        })?;
        let required = [
            "task_position",
            "title",
            "goal",
            "skill_id",
            "selected_paths",
            "references",
            "candidate",
        ];
        if fields.len() != required.len() && fields.len() != required.len() + 1
            || required.iter().any(|key| !fields.contains_key(*key))
            || fields
                .keys()
                .any(|key| !required.contains(&key.as_str()) && key != "dependency_positions")
        {
            return Err(fail(
                ExitCode::Contract,
                "invalid_object_fields",
                "A missing TASK has missing or unknown semantic fields.",
            ));
        }
        if missing["task_position"]
            .as_u64()
            .is_none_or(|position| position == 0)
            || missing["title"].as_str().is_none_or(str::is_empty)
            || missing["goal"].as_str().is_none_or(str::is_empty)
        {
            return Err(fail(
                ExitCode::Contract,
                "invalid_contract_value",
                "A missing TASK needs a position, title and goal.",
            ));
        }
        strings(&missing["selected_paths"])?;
        strings(&missing["references"])?;
        semantic_only(&missing["candidate"])?;
    }
    Ok(())
}

pub fn prepare_repair_from_project(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    input: PrepareRepairRequest<'_>,
) -> Result<Value, WorkError> {
    let semantic = parse_json_contract(input.raw).map_err(|_| {
        fail(
            ExitCode::Contract,
            "invalid_json_contract",
            "The TASK repair preparation is not valid JSON.",
        )
    })?;
    validate_prepare_request(&semantic)?;
    let _: work_model::task::request::TaskRepairPrepareRequest =
        serde_json::from_value(semantic.clone()).map_err(|_| {
            fail(
                ExitCode::Contract,
                "invalid_contract_value",
                "The semantic TASK repair request is invalid.",
            )
        })?;
    let requirement = semantic["requirement_id"].as_str().unwrap();
    let artifacts = resolve_repair_artifacts(root, requirement)?;
    let index_path = required_path(&artifacts, "task")?;
    if !index_path.ends_with("/index.json") {
        return Err(fail(
            ExitCode::WorkflowState,
            "task_collection_required",
            "TASK writes require a collection index.json artifact.",
        ));
    }
    let execution = required_path(&artifacts, "execution")?;
    let plan_path = required_path(&artifacts, "plan")?;
    let source = read_sources(root, index_path, execution)?;
    let index_raw = source.get(index_path).ok_or_else(|| {
        fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_ambiguous_source",
            "A missing TASK index cannot be reconstructed without a confirmed source.",
        )
    })?;
    let mut index = parse_json_contract(index_raw).map_err(|_| {
        fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_ambiguous_source",
            "The TASK index is not an unambiguous source.",
        )
    })?;
    if index["requirement_id"] != requirement || index["artifacts"] != artifacts {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_identity",
            "Repair must preserve requirement and artifact routing.",
        ));
    }
    let plan_raw = LocalFiles.read_raw(&storage_path(root, plan_path)?)?;
    let plan = parse_json_contract(&plan_raw).map_err(|_| {
        fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_ambiguous_source",
            "The source Plan is not an unambiguous source.",
        )
    })?;
    if plan["requirement_id"] != requirement || plan["artifacts"] != artifacts {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_ambiguous_source",
            "The source Plan does not establish the TASK identity and routing.",
        ));
    }
    let references = index["tasks"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| {
            fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_ambiguous_source",
                "A TASK index with identified item references is required.",
            )
        })?
        .clone();
    let directory = index_path.rsplit_once('/').map_or("", |(parent, _)| parent);
    let instructions = LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
    let mut items = serde_json::Map::new();
    let mut used_missing = false;
    let missing = semantic.get("missing_task");
    for (position, reference) in references.iter().enumerate() {
        let id = reference["id"].as_str().ok_or_else(|| {
            fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_ambiguous_source",
                "A TASK item reference has no identity.",
            )
        })?;
        if reference["path"] != format!("tasks/{id}.json") {
            return Err(fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_ambiguous_source",
                "TASK item references must have unambiguous IDs and paths.",
            ));
        }
        let path = format!("{directory}/tasks/{id}.json");
        if let Some(replacement) =
            missing.filter(|row| row["task_position"].as_u64() == Some((position + 1) as u64))
        {
            if source.contains_key(&path) {
                return Err(fail(
                    ExitCode::ArtifactIntegrity,
                    "task_repair_ambiguous_source",
                    "A semantic missing TASK can only fill an absent item.",
                ));
            }
            let selected = strings(&replacement["selected_paths"])?;
            let names = strings(&replacement["references"])?;
            let selection = select_instructions(&instructions, "task", &selected, &names)?;
            let positions = replacement["dependency_positions"]
                .as_array()
                .ok_or_else(|| {
                    fail(
                        ExitCode::Contract,
                        "invalid_semantic_position",
                        "Missing TASK dependencies need positions.",
                    )
                })?;
            let dependencies = positions
                .iter()
                .map(|item| {
                    item.as_u64()
                        .and_then(|position| references.get(position.wrapping_sub(1) as usize))
                        .and_then(|row| row["id"].as_str())
                        .map(str::to_owned)
                        .ok_or_else(|| {
                            fail(
                                ExitCode::ArtifactIntegrity,
                                "task_repair_ambiguous_source",
                                "Missing TASK dependency positions are invalid.",
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if dependencies.iter().any(|dependency| dependency == id)
                || dependencies
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != dependencies.len()
            {
                return Err(fail(
                    ExitCode::ArtifactIntegrity,
                    "task_repair_ambiguous_source",
                    "Missing TASK dependency positions are invalid.",
                ));
            }
            let acceptance = plan["acceptance_criteria"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| row["id"].as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            let candidate = build_semantic_candidate(
                &replacement["candidate"],
                &acceptance,
                &dependencies,
                &BTreeMap::new(),
            )
            .map_err(|issue| {
                WorkError::new(
                    ExitCode::Contract,
                    issue.reason_code,
                    issue.message,
                    issue.details,
                )
            })?;
            let mut item = json!({"schema":"work-task-item/v1","id":id,
                "title":replacement["title"],"goal":replacement["goal"],
                "skill_id":replacement["skill_id"],"instruction_selection":selection,
                "traceability":{"goal_ids":plan["goals"].as_array().into_iter().flatten().map(|row| row["id"].clone()).collect::<Vec<_>>(),
                    "deliverable_ids":plan["deliverables"].as_array().into_iter().flatten().map(|row| row["id"].clone()).collect::<Vec<_>>(),
                    "acceptance_ids":acceptance}});
            if !dependencies.is_empty() {
                item["dependencies"] = json!(dependencies);
            }
            for (field, value) in candidate.as_object().expect("candidate object") {
                item[field] = value.clone();
            }
            items.insert(id.to_owned(), item);
            used_missing = true;
        } else if let Some(raw) = source.get(&path) {
            let item = parse_json_contract(raw).map_err(|_| {
                fail(
                    ExitCode::ArtifactIntegrity,
                    "task_repair_ambiguous_source",
                    "A source TASK item has an ambiguous shape.",
                )
            })?;
            if item["id"] != id {
                return Err(fail(
                    ExitCode::ArtifactIntegrity,
                    "task_repair_ambiguous_source",
                    "A source TASK item has a conflicting identity.",
                ));
            }
            items.insert(id.to_owned(), item);
        } else {
            return Err(fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_ambiguous_source",
                "A missing TASK item requires a confirmed semantic TASK decision.",
            ));
        }
    }
    if missing.is_some() && !used_missing {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_ambiguous_source",
            "The missing TASK position does not match an index reference.",
        ));
    }
    let mut source_sets = Vec::new();
    for reference in &references {
        let selection = &items[reference["id"].as_str().unwrap()]["instruction_selection"];
        let selected = strings(&selection["selected_paths"])?;
        let names = strings(&selection["references"])?;
        source_sets.push(load_instructions(&instructions, "task", &selected, &names)?);
    }
    if task_document_selection(&source_sets)? != index["instruction_selection"] {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_ambiguous_source",
            "The reconstructed TASK selection does not match the source index.",
        ));
    }
    let edits = semantic["edits"].as_array().cloned().unwrap_or_default();
    let task_ids = references
        .iter()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let acceptance_ids = plan["acceptance_criteria"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let nested_groups = [
        "inputs",
        "decisions",
        "files",
        "risks",
        "steps",
        "commands",
        "operations",
        "validations",
    ];
    let mut nested = BTreeMap::<String, serde_json::Map<String, Value>>::new();
    for edit in &edits {
        let Some(id) = edit["task_id"].as_str() else {
            continue;
        };
        let Some(field) = edit["field"].as_str() else {
            continue;
        };
        if nested_groups.contains(&field) && edit["remove"] != true {
            nested
                .entry(id.to_owned())
                .or_default()
                .insert(field.to_owned(), edit["semantic_after"].clone());
        }
    }
    let mut nested_values = BTreeMap::new();
    for (id, replacements) in nested {
        let current = items.get(&id).ok_or_else(|| {
            fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_ambiguous_source",
                "An edit targets an unknown TASK item.",
            )
        })?;
        let dependency_edit = edits
            .iter()
            .find(|edit| edit["task_id"] == id && edit["field"] == "dependencies");
        let dependency_ids =
            if let Some(edit) = dependency_edit.filter(|edit| edit["remove"] != true) {
                positions(&edit["semantic_after"], &task_ids, false)?
            } else {
                current["dependencies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            };
        let dependency_files = dependency_ids
            .iter()
            .map(|dep_id| {
                let files = items[dep_id]["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter_map(|(position, row)| {
                        row["id"]
                            .as_str()
                            .map(|id| (format!("existing-{}", position + 1), id.to_owned()))
                    })
                    .collect::<BTreeMap<_, _>>();
                (dep_id.clone(), files)
            })
            .collect::<BTreeMap<_, _>>();
        let built = build_semantic_patch(
            current,
            &Value::Object(replacements),
            &acceptance_ids,
            &dependency_ids,
            &dependency_files,
        )
        .map_err(|issue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        nested_values.insert(id, built);
    }
    let mut seen = std::collections::BTreeSet::new();
    for edit in &edits {
        let field = edit["field"].as_str().ok_or_else(|| {
            fail(
                ExitCode::Contract,
                "invalid_contract_value",
                "A repair edit requires a field.",
            )
        })?;
        let id = edit["task_id"].as_str();
        if !seen.insert((id.map(str::to_owned), field.to_owned())) {
            return Err(fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_duplicate_edit",
                "Repair edits must target each field only once.",
            ));
        }
        let after = if edit["remove"] == true {
            None
        } else if matches!(
            (id.is_some(), field),
            (true, "title" | "goal") | (false, "title" | "summary")
        ) {
            edit.get("after").filter(|value| value.is_string()).cloned()
        } else if let Some(id) = id {
            match field {
                "traceability" => Some(traceability(&edit["semantic_after"], &plan)?),
                "dependencies" => {
                    Some(json!(positions(&edit["semantic_after"], &task_ids, false)?))
                }
                group if nested_groups.contains(&group) => {
                    nested_values.get(id).map(|value| value[group].clone())
                }
                _ => None,
            }
        } else {
            match field {
                "decisions" => Some(index_decisions(&edit["semantic_after"], &index)?),
                "execution_defaults" => {
                    let value = &edit["semantic_after"];
                    if value.as_object().is_none_or(|fields| {
                        fields.len() != 3
                            || !["working_directory", "os", "shell"]
                                .iter()
                                .all(|key| fields.contains_key(*key))
                    }) {
                        return Err(fail(
                            ExitCode::Contract,
                            "invalid_object_fields",
                            "Execution defaults need working directory, OS and shell.",
                        ));
                    }
                    Some(value.clone())
                }
                _ => None,
            }
        };
        let target = if let Some(id) = id {
            items.get_mut(id).ok_or_else(|| {
                fail(
                    ExitCode::ArtifactIntegrity,
                    "task_repair_ambiguous_source",
                    "An edit targets an unknown TASK item.",
                )
            })?
        } else {
            &mut index
        };
        if edit["remove"] == true {
            if matches!(
                (id.is_some(), field),
                (true, "title" | "goal") | (false, "title" | "summary")
            ) || target.get(field).is_none()
            {
                return Err(fail(
                    ExitCode::ArtifactIntegrity,
                    "task_repair_edit_state",
                    "A remove target does not exist or is required.",
                ));
            }
            target.as_object_mut().unwrap().remove(field);
        } else if let Some(after) = after {
            target[field] = after;
        } else {
            return Err(fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_protected_field",
                "This repair field has no semantic builder.",
            ));
        }
    }
    for reference in index["tasks"].as_array_mut().expect("references array") {
        let id = reference["id"].as_str().unwrap();
        let raw = render_task(&items[id], TaskDocumentKind::Item).map_err(|_| {
            fail(
                ExitCode::Contract,
                "invalid_contract_value",
                "The TASK item cannot be rendered.",
            )
        })?;
        reference["canonical_sha256"] = json!(work_operations::canonical::sha256_hex(&raw));
    }
    let mut candidate_paths = items
        .keys()
        .map(|id| format!("{directory}/tasks/{id}.json"))
        .collect::<Vec<_>>();
    candidate_paths.push(index_path.to_owned());
    candidate_paths.push(format!("{execution}/index.json"));
    let expected = repair_evidence_from_project(root, index_path, execution, &candidate_paths)?;
    let prepared = json!({"schema":"work-task-repair-request/v1",
        "stage":semantic["stage"],"requirement_id":requirement,
        "artifacts":artifacts,"expected":expected,"decisions":semantic["decisions"],
        "task_index":index,"task_items":items});
    let prepared = work_model::task::response::typed_response::<
        work_model::task::request::TaskRepairRequest,
    >(prepared);
    let raw = render_task(&prepared, TaskDocumentKind::RepairRequest).map_err(|_| {
        fail(
            ExitCode::Contract,
            "invalid_contract_value",
            "The prepared repair cannot be rendered.",
        )
    })?;
    let preview = repair_from_project(
        root,
        skill_root,
        configs,
        RepairProjectRequest {
            raw: &raw,
            operation: RepairOperation::Validate,
            approved_sha256: None,
        },
    )?;
    if read_sources(root, index_path, execution)? != source {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_source_changed",
            "TASK collection repair sources changed during preparation.",
        ));
    }
    if let Some(output) = input.output_file {
        LocalFiles.create_new(output, &raw)?;
    }
    Ok(work_model::task::response::typed_response::<
        work_model::task::response::TaskRepairPrepare,
    >(
        json!({"schema":"work-task-repair-prepare/v1","request":prepared,
        "preview":preview,"output_file":input.output_file.map(|path| path.to_string_lossy().to_string())}),
    ))
}

fn preview(request: &Value, prepared: &PreparedRepair, diagnostics: Value) -> Value {
    work_model::task::response::typed_response::<work_model::task::response::TaskRepairResponse>(
        json!({"schema":"work-task-repair/v1","status":"preview","stage":request["stage"],
        "approved_sha256":prepared.transaction["approval_sha256"],
        "artifacts":request["artifacts"],"decisions":request["decisions"],
        "affected_task_ids":prepared.affected_task_ids,"changed_paths":prepared.changed_paths,
        "task_diagnostics":diagnostics,"file_readiness":"requires_execute_preflight"}),
    )
}

pub fn repair_from_project(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: RepairProjectRequest<'_>,
) -> Result<Value, WorkError> {
    let value = parse_json_contract(request.raw).map_err(|_| {
        fail(
            ExitCode::Contract,
            "invalid_json_contract",
            "The TASK repair request is not valid JSON.",
        )
    })?;
    validate_repair_request(&value)?;
    let artifacts = &value["artifacts"];
    let index = required_path(artifacts, "task")?;
    let execution = required_path(artifacts, "execution")?;
    let plan = required_path(artifacts, "plan")?;
    let request_raw = render_task(&value, TaskDocumentKind::RepairRequest).map_err(|_| {
        fail(
            ExitCode::Contract,
            "invalid_contract_value",
            "The repair request cannot be rendered.",
        )
    })?;
    let transaction_id = transaction_id(&request_raw);
    let journal = format!("{execution}/.work-task-repair-{transaction_id}.json");
    let marker = format!("{journal}.done");
    let (transaction, prepared) = if matches!(request.operation, RepairOperation::Recover) {
        let raw = LocalFiles.read_raw(&storage_path(root, &journal)?)?;
        let parsed = parse_json_contract(&raw).map_err(|_| {
            fail(
                ExitCode::ArtifactIntegrity,
                "invalid_json_contract",
                "The TASK repair journal is invalid.",
            )
        })?;
        validate_transaction(&parsed).map_err(|issue| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        if render_transaction(&parsed).ok().as_deref() != Some(raw.as_slice()) {
            return Err(fail(
                ExitCode::ArtifactIntegrity,
                "noncanonical_json",
                "The TASK repair journal is not canonical.",
            ));
        }
        if parsed["metadata"]["request"] != value || parsed["transaction_id"] != transaction_id {
            return Err(fail(
                ExitCode::ArtifactIntegrity,
                "task_repair_recovery_changed",
                "Recovery requires the identical approved request.",
            ));
        }
        let changed_paths = parsed["files"]
            .as_array()
            .expect("validated files")
            .iter()
            .filter_map(|row| row["path"].as_str().map(str::to_owned))
            .collect();
        let affected_task_ids = parsed["metadata"]["affected_task_ids"]
            .as_array()
            .expect("validated metadata")
            .iter()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect();
        (
            parsed.clone(),
            PreparedRepair {
                transaction: parsed,
                changed_paths,
                affected_task_ids,
            },
        )
    } else {
        LocalWriterLock.require_idle(&storage_path(
            root,
            &format!("{execution}/.work-state-writer.lock"),
        )?)?;
        require_no_spec_update(root, execution, None)?;
        let sources = read_sources(root, index, execution)?;
        let plan_raw = LocalFiles.read_raw(&storage_path(root, plan)?)?;
        let history = execution_history_fingerprints(root, execution)?;
        let instructions = LocalHierarchyCatalog {
            skill_root: skill_root.to_path_buf(),
        };
        let skills = LocalSkillCatalog {
            roots: configs.to_vec(),
        };
        let roots = configs
            .iter()
            .map(|config| SkillRoot {
                scope: config.scope.clone(),
                locator: config.locator.clone(),
            })
            .collect::<Vec<_>>();
        let paths = LocalPlanStorage {
            project_root: root.to_path_buf(),
        };
        let built = build_repair_transaction(
            &instructions,
            &skills,
            &paths,
            &roots,
            RepairInput {
                request: &value,
                sources: &sources,
                plan_raw: &plan_raw,
                history_sha256: &history,
            },
        )?;
        (built.transaction.clone(), built)
    };
    let diagnostics = diagnose_task_collection(root, skill_root, configs, index);
    let mut result = preview(&value, &prepared, diagnostics);
    if matches!(request.operation, RepairOperation::Validate) {
        return Ok(result);
    }
    if request.approved_sha256 != transaction["approval_sha256"].as_str() {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_approval_changed",
            "The approved repair transaction changed.",
        ));
    }
    require_no_spec_update(
        root,
        execution,
        if matches!(request.operation, RepairOperation::Recover) {
            Some(&journal)
        } else {
            None
        },
    )?;
    if serde_json::to_value(execution_history_fingerprints(root, execution)?)
        .expect("history serializes")
        != transaction["metadata"]["history_sha256"]
    {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_history_changed",
            "Execution history changed before repair publication.",
        ));
    }
    let lock = storage_path(root, &format!("{execution}/.work-state-writer.lock"))?;
    let _guard = LocalWriterLock.acquire(&lock)?;
    if matches!(request.operation, RepairOperation::Apply) {
        write_journal(root, &journal, &transaction)?;
    }
    let published = publish_journal(root, &journal, &marker).map_err(|_| {
        WorkError::new(
            ExitCode::IoFailure,
            "task_repair_interrupted",
            "Preserve the repair transaction and obtain recovery authorization.",
            json!({"recovery_required":true,"record":journal}),
        )
    })?;
    let publication = published["status"].as_str().unwrap_or("published");
    let instructions = LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
    let skills = LocalSkillCatalog {
        roots: configs.to_vec(),
    };
    let roots = configs
        .iter()
        .map(|config| SkillRoot {
            scope: config.scope.clone(),
            locator: config.locator.clone(),
        })
        .collect::<Vec<_>>();
    let paths = LocalPlanStorage {
        project_root: root.to_path_buf(),
    };
    let repository = LocalTaskStorage {
        project_root: root.to_path_buf(),
    };
    load_collection(&instructions, &skills, &paths, &repository, &roots, index).map_err(|_| {
        fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_post_validation",
            "The installed repair is not valid.",
        )
    })?;
    result["status"] = json!(if publication == "already_published" {
        "already_completed"
    } else if matches!(request.operation, RepairOperation::Recover) {
        "recovered"
    } else {
        "repaired"
    });
    result["publication_status"] = json!(publication);
    result["task_diagnostics"] = diagnose_task_collection(root, skill_root, configs, index);
    if result["task_diagnostics"]["normal_use_allowed"] != true {
        return Err(fail(
            ExitCode::ArtifactIntegrity,
            "task_repair_post_validation",
            "The installed repair is not valid.",
        ));
    }
    Ok(work_model::task::response::typed_response::<
        work_model::task::response::TaskRepairResponse,
    >(result))
}

pub fn repair_evidence_from_project(
    root: &Path,
    index: &str,
    execution: &str,
    candidate_paths: &[String],
) -> Result<BTreeMap<String, Value>, WorkError> {
    let sources = read_sources(root, index, execution)?;
    let mut paths = sources.keys().cloned().collect::<Vec<_>>();
    paths.extend_from_slice(candidate_paths);
    paths.sort();
    paths.dedup();
    Ok(evidence_fingerprints(&sources, &paths))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn prepare_request_rejects_artifacts_and_bad_shapes_before_io() {
        let valid = json!({"schema":"work-task-repair-prepare-request/v1","stage":"complete",
            "requirement_id":"example","decisions":[{"location":"/","decision":"Use the reviewed collection."}]});
        validate_prepare_request(&valid).unwrap();
        assert_eq!(
            validate_prepare_request(&json!([]))
                .unwrap_err()
                .reason_code,
            "invalid_contract_value"
        );
        let mut invalid = valid.clone();
        invalid["artifacts"] = json!({"plan":"p","task":"t","execution":"e"});
        assert_eq!(
            validate_prepare_request(&invalid).unwrap_err().reason_code,
            "invalid_object_fields"
        );
        invalid = valid.clone();
        invalid["schema"] = json!("work-task-repair-prepare-request/v2");
        assert_eq!(
            validate_prepare_request(&invalid).unwrap_err().reason_code,
            "task_repair_prepare_schema"
        );
        invalid = valid.clone();
        invalid["decisions"] = json!([]);
        assert_eq!(
            validate_prepare_request(&invalid).unwrap_err().reason_code,
            "task_repair_decisions"
        );
        for field in ["task_items", "unexpected"] {
            invalid = valid.clone();
            invalid[field] = if field == "task_items" {
                json!({})
            } else {
                json!(true)
            };
            assert_eq!(
                validate_prepare_request(&invalid).unwrap_err().reason_code,
                "invalid_object_fields"
            );
        }
    }

    #[test]
    fn prepare_request_rejects_formal_nested_edits_and_accepts_semantic_positions() {
        let mut request = json!({"schema":"work-task-repair-prepare-request/v1","stage":"complete",
            "requirement_id":"example","decisions":[{"location":"/","decision":"Use the reviewed collection."}]});
        for edit in [
            json!({"task_id":"TASK-001","field":"files","after":[{"id":"FILE-001","action":"modify","path":"src.txt"}]}),
            json!({"task_id":"TASK-001","field":"files","semantic_after":[{"key":"source","id":"FILE-001","action":"modify","path":"src.txt"}]}),
            json!({"task_id":"TASK-001","field":"files","semantic_after":[{"key":"source","action":"modify","path":"FILE-001"}]}),
        ] {
            request["edits"] = json!([edit]);
            assert!(validate_prepare_request(&request).is_err());
        }
        request["edits"] = json!([{"task_id":"TASK-001","field":"files",
            "semantic_after":[{"key":"source","existing_position":1,"action":"modify","path":"src.txt"}]}]);
        validate_prepare_request(&request).unwrap();
    }

    #[test]
    fn binding_only_repair_matches_python_approval_and_publishes() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-repair");
        let root = std::env::temp_dir().join(format!(
            "work-repair-parity-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let raw = fs::read(fixture.join("request.json")).unwrap();
        let mut invalid: Value = serde_json::from_slice(&raw).unwrap();
        invalid["expected"]["outputs/work/executions/example/index.json"] = json!("invalid");
        let rejected = repair_from_project(
            &root,
            &repo.join("../skills/work"),
            &[],
            RepairProjectRequest {
                raw: &serde_json::to_vec(&invalid).unwrap(),
                operation: RepairOperation::Validate,
                approved_sha256: None,
            },
        )
        .unwrap_err();
        assert_eq!(rejected.reason_code, "task_repair_expected");
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        assert_eq!(
            resolve_repair_artifacts(&root, "example").unwrap(),
            serde_json::from_slice::<Value>(&raw).unwrap()["artifacts"]
        );
        let skill = repo.join("../skills/work");
        let semantic = fs::read(fixture.join("semantic-request.json")).unwrap();
        let prepared = prepare_repair_from_project(
            &root,
            &skill,
            &[],
            PrepareRepairRequest {
                raw: &semantic,
                output_file: None,
            },
        )
        .unwrap();
        assert_eq!(
            prepared["request"],
            serde_json::from_slice::<Value>(&raw).unwrap()
        );
        assert_eq!(
            prepared["preview"]["approved_sha256"],
            expected["approved_sha256"]
        );
        let preview = repair_from_project(
            &root,
            &skill,
            &[],
            RepairProjectRequest {
                raw: &raw,
                operation: RepairOperation::Validate,
                approved_sha256: None,
            },
        )
        .unwrap();
        assert_eq!(preview["approved_sha256"], expected["approved_sha256"]);
        assert!(preview.get("publication_status").is_none());
        assert_eq!(preview["changed_paths"], expected["changed_paths"]);
        assert_eq!(preview["affected_task_ids"], expected["affected_task_ids"]);
        for field in [
            "status",
            "format_status",
            "contract_status",
            "normal_use_allowed",
            "execution_binding_status",
        ] {
            assert_eq!(
                preview["task_diagnostics"][field],
                expected["diagnostics"][field]
            );
        }
        let codes = preview["task_diagnostics"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|issue| issue["code"].clone())
            .collect::<Vec<_>>();
        assert_eq!(json!(codes), expected["diagnostic_issue_codes"]);
        let checks = preview["task_diagnostics"]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| json!({"name":row["name"],"status":row["status"]}))
            .collect::<Vec<_>>();
        assert_eq!(json!(checks), expected["diagnostic_checks"]);
        let approval = preview["approved_sha256"].as_str().unwrap();
        let result = repair_from_project(
            &root,
            &skill,
            &[],
            RepairProjectRequest {
                raw: &raw,
                operation: RepairOperation::Apply,
                approved_sha256: Some(approval),
            },
        )
        .unwrap();
        assert_eq!(result["status"], "repaired");
        assert_eq!(result["publication_status"], "published");
        let execution: Value = serde_json::from_slice(
            &fs::read(root.join("outputs/work/executions/example/index.json")).unwrap(),
        )
        .unwrap();
        assert_ne!(execution["task_collection_sha256"], "0".repeat(64));
        assert_eq!(execution["tasks"][0]["status"], "pending");
        let repeated = repair_from_project(
            &root,
            &skill,
            &[],
            RepairProjectRequest {
                raw: &raw,
                operation: RepairOperation::Recover,
                approved_sha256: Some(approval),
            },
        )
        .unwrap();
        assert_eq!(repeated["status"], "already_completed");
        let recovery_root = std::env::temp_dir().join(format!(
            "work-repair-recovery-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = recovery_root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let request_value: Value = serde_json::from_slice(&raw).unwrap();
        let request_raw = render_task(&request_value, TaskDocumentKind::RepairRequest).unwrap();
        let journal_path = format!(
            "outputs/work/executions/example/.work-task-repair-{}.json",
            transaction_id(&request_raw)
        );
        let mut journal: Value =
            serde_json::from_slice(&fs::read(root.join(&journal_path)).unwrap()).unwrap();
        journal["state"] = json!("prepared");
        journal["published_count"] = json!(0);
        write_journal(&recovery_root, &journal_path, &journal).unwrap();
        let first = &journal["files"][0];
        let first_path = recovery_root.join(first["path"].as_str().unwrap());
        let first_after =
            work_operations::specification::transaction::decode_snapshot(&first["after"]).unwrap();
        fs::write(first_path, first_after).unwrap();
        let resumed = repair_from_project(
            &recovery_root,
            &skill,
            &[],
            RepairProjectRequest {
                raw: &raw,
                operation: RepairOperation::Recover,
                approved_sha256: Some(approval),
            },
        )
        .unwrap();
        assert_eq!(resumed["status"], "recovered");
        assert_eq!(
            fs::read(recovery_root.join(&journal_path)).unwrap(),
            fs::read(root.join(&journal_path)).unwrap()
        );
    }

    #[test]
    fn semantic_missing_and_title_repairs_match_python_requests() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let skill = repo.join("../skills/work");
        for variant in ["missing", "title", "nested_file"] {
            let fixture = repo
                .join("crates/work-infrastructure/fixtures/task-repair")
                .join(variant);
            let root = std::env::temp_dir().join(format!(
                "work-repair-{variant}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            for relative in [
                "outputs/work/plans/example.json",
                "outputs/work/tasks/example/index.json",
                "outputs/work/tasks/example/tasks/TASK-001.json",
                "outputs/work/executions/example/index.json",
            ] {
                let source = fixture.join(relative);
                if source.is_file() {
                    let destination = root.join(relative);
                    fs::create_dir_all(destination.parent().unwrap()).unwrap();
                    fs::copy(source, destination).unwrap();
                }
            }
            let semantic = fs::read(fixture.join("semantic-request.json")).unwrap();
            let expected_request: Value =
                serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
            let expected: Value =
                serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
            let prepared = prepare_repair_from_project(
                &root,
                &skill,
                &[],
                PrepareRepairRequest {
                    raw: &semantic,
                    output_file: None,
                },
            )
            .unwrap();
            assert_eq!(prepared["request"], expected_request, "variant {variant}");
            for field in ["approved_sha256", "changed_paths", "affected_task_ids"] {
                assert_eq!(
                    prepared["preview"][field], expected[field],
                    "variant {variant}: {field}"
                );
            }
            let request = fs::read(fixture.join("request.json")).unwrap();
            let approval = expected["approved_sha256"].as_str().unwrap();
            let repaired = repair_from_project(
                &root,
                &skill,
                &[],
                RepairProjectRequest {
                    raw: &request,
                    operation: RepairOperation::Apply,
                    approved_sha256: Some(approval),
                },
            )
            .unwrap();
            assert_eq!(repaired["status"], "repaired");
        }
    }

    #[test]
    fn semantic_repair_rejects_missing_index_and_unreviewed_orphan() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-repair");
        let skill = repo.join("../skills/work");
        let semantic = fs::read(fixture.join("semantic-request.json")).unwrap();
        for (variant, expected_reason) in [
            ("missing_index", "task_repair_ambiguous_source"),
            ("orphan", "task_repair_orphan_decision"),
        ] {
            let root = std::env::temp_dir().join(format!(
                "work-repair-{variant}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            for relative in [
                "outputs/work/plans/example.json",
                "outputs/work/executions/example/index.json",
                "outputs/work/tasks/example/index.json",
                "outputs/work/tasks/example/tasks/TASK-001.json",
            ] {
                if variant == "missing_index" && relative.contains("tasks/example/index.json") {
                    continue;
                }
                let destination = root.join(relative);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::copy(fixture.join(relative), destination).unwrap();
            }
            if variant == "orphan" {
                fs::copy(
                    fixture.join("outputs/work/tasks/example/tasks/TASK-001.json"),
                    root.join("outputs/work/tasks/example/tasks/TASK-999.json"),
                )
                .unwrap();
            }
            let error = prepare_repair_from_project(
                &root,
                &skill,
                &[],
                PrepareRepairRequest {
                    raw: &semantic,
                    output_file: None,
                },
            )
            .unwrap_err();
            assert_eq!(error.reason_code, expected_reason, "variant {variant}");
            if variant == "orphan" {
                let mut reviewed: Value = serde_json::from_slice(&semantic).unwrap();
                reviewed["decisions"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"location":"/orphans/TASK-999.json",
                        "decision":"Remove reviewed orphan."}));
                let reviewed_raw = serde_json::to_vec(&reviewed).unwrap();
                let prepared = prepare_repair_from_project(
                    &root,
                    &skill,
                    &[],
                    PrepareRepairRequest {
                        raw: &reviewed_raw,
                        output_file: None,
                    },
                )
                .unwrap();
                assert!(prepared["request"]["task_items"].get("TASK-999").is_none());
            }
        }
    }

    #[test]
    fn diagnostics_keeps_independent_checks_when_item_json_breaks() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-repair");
        let root = std::env::temp_dir().join(format!(
            "work-diagnostic-item-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        fs::write(
            root.join("outputs/work/tasks/example/tasks/TASK-001.json"),
            b"{",
        )
        .unwrap();
        let before = fs::read(root.join("outputs/work/tasks/example/tasks/TASK-001.json")).unwrap();
        let report = diagnose_task_collection(
            &root,
            &repo.join("../skills/work"),
            &[],
            "outputs/work/tasks/example/index.json",
        );
        let statuses = report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| {
                Some((
                    row["name"].as_str()?.to_owned(),
                    row["status"].as_str()?.to_owned(),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(statuses["index:contract"], "passed");
        assert_eq!(statuses["plan:contract"], "passed");
        assert_eq!(statuses["item:TASK-001:json"], "failed");
        assert_eq!(report["contract_status"], "not_checked");
        assert_eq!(report["normal_use_allowed"], false);
        assert_eq!(
            fs::read(root.join("outputs/work/tasks/example/tasks/TASK-001.json")).unwrap(),
            before
        );
    }

    #[test]
    fn diagnostics_reports_item_contract_and_index_fingerprint_independently() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let root = std::env::temp_dir().join(format!(
            "work-diagnostic-contract-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let index = "outputs/work/tasks/example/index.json";
        let item = "outputs/work/tasks/example/tasks/TASK-001.json";
        for relative in [
            "outputs/work/plans/example.json",
            index,
            item,
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let original_item = fs::read(root.join(item)).unwrap();
        let mut invalid_item: Value = serde_json::from_slice(&original_item).unwrap();
        invalid_item.as_object_mut().unwrap().remove("goal");
        invalid_item["validations"] = json!([]);
        fs::write(
            root.join(item),
            render_task(&invalid_item, TaskDocumentKind::Item).unwrap(),
        )
        .unwrap();
        let report = diagnose_task_collection(&root, &repo.join("../skills/work"), &[], index);
        assert_eq!(report["contract_status"], "not_checked");
        assert_eq!(report["normal_use_allowed"], false);
        fs::write(root.join(item), &original_item).unwrap();
        let mut invalid_index: Value =
            serde_json::from_slice(&fs::read(root.join(index)).unwrap()).unwrap();
        invalid_index["tasks"][0]["canonical_sha256"] = json!("0".repeat(64));
        fs::write(
            root.join(index),
            render_task(&invalid_index, TaskDocumentKind::Index).unwrap(),
        )
        .unwrap();
        let report = diagnose_task_collection(&root, &repo.join("../skills/work"), &[], index);
        assert!(
            report["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|issue| { issue["code"] == "task_item_fingerprint_mismatch" })
        );
    }

    #[test]
    fn diagnostics_malformed_sources_match_python_check_sequence_and_codes() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let prerequisite_reference: Value =
            serde_json::from_slice(&fs::read(fixture.join("required-prerequisites.json")).unwrap())
                .unwrap();
        for variant in [
            "valid",
            "index-invalid",
            "index-duplicate",
            "index-nan",
            "index-array",
            "item-invalid",
            "item-bom",
            "item-utf16",
        ] {
            let root = std::env::temp_dir().join(format!(
                "work-diagnostic-parity-{variant}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            for relative in [
                "outputs/work/plans/example.json",
                "outputs/work/tasks/example/index.json",
                "outputs/work/tasks/example/tasks/TASK-001.json",
                "outputs/work/executions/example/index.json",
            ] {
                let destination = root.join(relative);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::copy(fixture.join(relative), destination).unwrap();
            }
            let raw = match variant {
                "index-invalid" => Some(b"{\"title\":".to_vec()),
                "index-duplicate" => Some(b"{\"a\":1,\"a\":2}".to_vec()),
                "index-nan" => Some(b"{\"value\":NaN}".to_vec()),
                "index-array" => Some(b"[]".to_vec()),
                "item-invalid" => Some(b"{".to_vec()),
                "item-bom" => Some(
                    [
                        b"\xef\xbb\xbf".as_slice(),
                        &fs::read(fixture.join("outputs/work/tasks/example/tasks/TASK-001.json"))
                            .unwrap(),
                    ]
                    .concat(),
                ),
                "item-utf16" => {
                    let source = fs::read_to_string(
                        fixture.join("outputs/work/tasks/example/tasks/TASK-001.json"),
                    )
                    .unwrap();
                    let mut bytes = vec![0xff, 0xfe];
                    for unit in source.encode_utf16() {
                        bytes.extend(unit.to_le_bytes());
                    }
                    Some(bytes)
                }
                _ => None,
            };
            if let Some(raw) = raw {
                let relative = if variant.starts_with("index-") {
                    "outputs/work/tasks/example/index.json"
                } else {
                    "outputs/work/tasks/example/tasks/TASK-001.json"
                };
                fs::write(root.join(relative), raw).unwrap();
            }
            let result = diagnose_task_collection(
                &root,
                &repo.join("../skills/work"),
                &[],
                "outputs/work/tasks/example/index.json",
            );
            let expected: Value = serde_json::from_slice(
                &fs::read(fixture.join(variant).join("expected.json")).unwrap(),
            )
            .unwrap();
            for field in [
                "status",
                "format_status",
                "contract_status",
                "normal_use_allowed",
                "execution_binding_status",
                "raw_sha256",
            ] {
                assert_eq!(result[field], expected[field], "{variant}: {field}");
            }
            let checks = result["checks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| json!({"name":row["name"],"status":row["status"]}))
                .collect::<Vec<_>>();
            assert_eq!(json!(checks), expected["checks"], "{variant}: checks");
            let requires = result["checks"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|row| {
                    row.get("requires")
                        .map(|value| (row["name"].as_str().unwrap().to_owned(), value.clone()))
                })
                .collect::<BTreeMap<_, _>>();
            let reference_key = if variant.starts_with("index-") {
                "index-malformed"
            } else {
                variant
            };
            assert_eq!(
                json!(requires),
                prerequisite_reference[reference_key],
                "{variant}: check prerequisites"
            );
            let codes = result["issues"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["code"].clone())
                .collect::<Vec<_>>();
            assert_eq!(json!(codes), expected["issue_codes"], "{variant}: issues");
            match variant {
                "index-invalid" => assert_eq!(
                    result["issues"][0],
                    json!({
                        "stage":"index:json","code":"invalid_json_contract",
                        "location":"outputs/work/tasks/example/index.json",
                        "category":"user_decision","message":"The TASK JSON is invalid.",
                        "suggestion":"Preserve the original bytes; ask the user to resolve any ambiguous decoding or JSON interpretation.",
                        "details":{"line":1,"column":10,"byte_offset":9}
                    })
                ),
                "index-duplicate" => {
                    assert_eq!(result["issues"][0]["details"], json!({"keys":["a"]}))
                }
                "item-invalid" => {
                    assert_eq!(
                        result["issues"][0]["details"],
                        json!({"line":1,"column":2,"byte_offset":1})
                    );
                    assert_eq!(
                        result["issues"][0]["location"],
                        root.canonicalize()
                            .unwrap()
                            .join("outputs/work/tasks/example/tasks/TASK-001.json")
                            .to_string_lossy()
                            .as_ref()
                    );
                }
                "item-bom" => {
                    let issue = result["issues"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|issue| issue["stage"] == "item:TASK-001:canonical")
                        .unwrap();
                    let source = fs::read_to_string(
                        fixture.join("outputs/work/tasks/example/tasks/TASK-001.json"),
                    )
                    .unwrap();
                    assert_eq!(
                        issue["details"],
                        json!({"expected":source,"actual":format!("\u{feff}{source}")})
                    );
                }
                "item-utf16" => {
                    let source = root
                        .canonicalize()
                        .unwrap()
                        .join("outputs/work/tasks/example/tasks/TASK-001.json")
                        .to_string_lossy()
                        .into_owned();
                    assert_eq!(result["issues"][0]["location"], source);
                    assert_eq!(
                        result["issues"][0]["details"],
                        json!({"source":source,"byte_offset":0})
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn diagnostics_reports_missing_or_conflicting_sources_without_writes() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let plan = "outputs/work/plans/example.json";
        let index = "outputs/work/tasks/example/index.json";
        let item = "outputs/work/tasks/example/tasks/TASK-001.json";
        let execution = "outputs/work/executions/example/index.json";
        for case in [
            "missing-index",
            "missing-item",
            "missing-execution",
            "orphan-and-broken-item",
            "plan-binding",
            "execution-binding",
            "pending-migration",
            "item-crlf",
        ] {
            let root = std::env::temp_dir().join(format!(
                "work-diagnostic-sources-{case}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            for relative in [plan, index, item, execution] {
                let destination = root.join(relative);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::copy(fixture.join(relative), destination).unwrap();
            }
            match case {
                "missing-index" => fs::remove_file(root.join(index)).unwrap(),
                "missing-item" => fs::remove_file(root.join(item)).unwrap(),
                "missing-execution" => fs::remove_file(root.join(execution)).unwrap(),
                "orphan-and-broken-item" => {
                    fs::write(root.join(item), b"{").unwrap();
                    fs::write(
                        root.join("outputs/work/tasks/example/tasks/TASK-999.json"),
                        b"{}\n",
                    )
                    .unwrap();
                }
                "plan-binding" => {
                    let mut value: Value =
                        serde_json::from_slice(&fs::read(root.join(index)).unwrap()).unwrap();
                    value["source_plan"]["canonical_sha256"] = json!("0".repeat(64));
                    fs::write(
                        root.join(index),
                        render_task(&value, TaskDocumentKind::Index).unwrap(),
                    )
                    .unwrap();
                }
                "execution-binding" => {
                    let mut value: Value =
                        serde_json::from_slice(&fs::read(root.join(execution)).unwrap()).unwrap();
                    value["task_spec_id"] = json!("TASK-SPEC-999");
                    fs::write(
                        root.join(execution),
                        work_operations::execution::index::render_execution_index(&value).unwrap(),
                    )
                    .unwrap();
                }
                "pending-migration" => {
                    fs::write(
                        root.join("outputs/work/executions/example/.work-spec-migration-ABC.json"),
                        b"{}\n",
                    )
                    .unwrap();
                }
                "item-crlf" => {
                    let raw = fs::read(root.join(item)).unwrap();
                    let text = String::from_utf8(raw).unwrap();
                    fs::write(root.join(item), text.replace('\n', "\r\n")).unwrap();
                }
                _ => unreachable!(),
            }
            let before = [plan, index, item, execution]
                .iter()
                .map(|relative| (relative.to_string(), fs::read(root.join(relative)).ok()))
                .collect::<BTreeMap<_, _>>();
            let report = diagnose_task_collection(&root, &repo.join("../skills/work"), &[], index);
            for (relative, bytes) in before {
                assert_eq!(fs::read(root.join(relative)).ok(), bytes, "{case}");
            }
            let codes = report["issues"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|row| row["code"].as_str())
                .collect::<Vec<_>>();
            let status = |name: &str| {
                report["checks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["name"] == name)
                    .unwrap()["status"]
                    .clone()
            };
            match case {
                "missing-index" => {
                    assert!(report["raw_sha256"].is_null());
                    assert_eq!(report["normal_use_allowed"], false);
                }
                "missing-item" => assert!(codes.contains(&"file_not_found")),
                "missing-execution" => {
                    assert_eq!(report["normal_use_allowed"], true);
                    assert_eq!(status("execution:file"), "failed");
                    assert_eq!(status("execution:contract"), "not_checked");
                }
                "orphan-and-broken-item" => {
                    assert!(codes.contains(&"invalid_json_contract"));
                    assert!(codes.contains(&"task_collection_directory_mismatch"));
                }
                "plan-binding" => assert!(codes.contains(&"source_plan_fingerprint_mismatch")),
                "execution-binding" => {
                    assert_eq!(report["execution_binding_status"], "failed");
                    assert!(codes.contains(&"execution_task_binding_mismatch"));
                }
                "pending-migration" => assert!(codes.contains(&"task_repair_pending_transaction")),
                "item-crlf" => {
                    assert_eq!(status("item:TASK-001:normalization"), "failed");
                    assert_eq!(report["normal_use_allowed"], false);
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn repair_binding_uses_task_index_without_plan_and_rejects_other_execution() {
        let root = std::env::temp_dir().join(format!(
            "work-repair-binding-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let artifacts = json!({"plan":"outputs/work/custom/confirmed-plan.json",
            "task":"outputs/work/custom/tasks/index.json",
            "execution":"outputs/work/custom/execution"});
        let task = root.join(artifacts["task"].as_str().unwrap());
        fs::create_dir_all(task.parent().unwrap()).unwrap();
        fs::write(
            &task,
            serde_json::to_vec(&json!({"schema":"work-task-index/v1",
                "requirement_id":"example","artifacts":artifacts}))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            resolve_repair_artifacts(&root, "example").unwrap(),
            artifacts
        );
        let other = root.join("outputs/work/other-execution/index.json");
        fs::create_dir_all(other.parent().unwrap()).unwrap();
        fs::write(
            &other,
            serde_json::to_vec(&json!({"schema":"work-execution-index/v1",
                "requirement_id":"example"}))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            resolve_repair_artifacts(&root, "example")
                .unwrap_err()
                .reason_code,
            "task_repair_execution_binding_conflict"
        );
    }
}
