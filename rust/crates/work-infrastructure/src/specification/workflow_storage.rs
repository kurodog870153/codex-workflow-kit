//! Specification update validation, publication, recovery and verification.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::instruction::{
    load as load_instructions, select as select_instructions, task_document_selection,
};
use work_feature::ports::{ArtifactStore, WriterLock};
use work_feature::skill::SkillRoot;
use work_feature::specification::{SpecificationBaseline, preview_update};
use work_feature::task::load_collection_with_file_state;
use work_operations::canonical::parse_json_contract;
#[cfg(test)]
use work_operations::canonical::sha256_hex as raw_sha256;
use work_operations::derivation::fingerprint;
use work_operations::derivation::graph::{
    ArtifactNode, bind_plan_source, reconcile_artifact_bindings,
};
use work_operations::derivation::identity::derived_transaction_id;
use work_operations::derivation::publication::completion_marker;
use work_operations::execution::attempt::{validate_attempt_bytes, validate_attempt_file_path};
use work_operations::execution::correction::{
    render_correction, validate_correction, validate_correction_file_path,
};
use work_operations::execution::index::validate_execution_index;
use work_operations::plan::render_plan_value;
use work_operations::specification::prepare::validate_prepare_request;
use work_operations::specification::transaction::{render_transaction, validate_transaction};
use work_operations::specification::update::validate_update_request;
use work_operations::specification::verification::validate_request as validate_verification_request;
use work_operations::task::candidate::{build_semantic_candidate, build_semantic_patch};
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::files::LocalFiles;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::{
    execution_history_fingerprints, publish_journal, require_no_spec_update, storage_path,
    write_journal,
};
use crate::task::storage::LocalTaskStorage;
use crate::writer_lock::LocalWriterLock;

fn work_json_files(root: &Path) -> Result<Vec<String>, WorkError> {
    let mut pending = vec!["outputs/work".to_owned()];
    let mut files = Vec::new();
    while let Some(relative) = pending.pop() {
        let directory = storage_path(root, &relative)?;
        if !directory.is_dir() {
            continue;
        }
        for entry in fs::read_dir(directory).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "file_read_failed",
                "The managed Work directory cannot be inspected.",
                json!({}),
            )
        })? {
            let entry = entry.map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "file_read_failed",
                    "The managed Work directory cannot be inspected.",
                    json!({}),
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

pub enum SpecOperation {
    Validate,
    Apply,
    Recover,
}

pub struct SpecificationProjectRequest<'a> {
    pub raw: &'a [u8],
    pub operation: SpecOperation,
    pub approved_sha256: Option<&'a str>,
}

pub struct SpecificationPrepareInput<'a> {
    pub raw: &'a [u8],
    pub date: &'a str,
    pub output_file: Option<&'a Path>,
}

struct OwnedBaseline {
    plan_path: String,
    task_path: String,
    execution_path: String,
    plan_raw: Vec<u8>,
    index_raw: Vec<u8>,
    items: BTreeMap<String, Vec<u8>>,
    execution_raw: Vec<u8>,
    history: BTreeMap<String, Vec<u8>>,
}

impl OwnedBaseline {
    fn borrow(&self) -> SpecificationBaseline<'_> {
        SpecificationBaseline {
            plan_path: &self.plan_path,
            task_path: &self.task_path,
            execution_path: &self.execution_path,
            plan_raw: &self.plan_raw,
            index_raw: &self.index_raw,
            items: &self.items,
            execution_raw: &self.execution_raw,
            history: &self.history,
        }
    }
}

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

fn with_migration_route(mut error: WorkError) -> WorkError {
    if let Some(details) = error.details.as_object_mut() {
        details.insert("next_command".into(), json!("migration analyze"));
    } else {
        error.details = json!({"next_command":"migration analyze"});
    }
    error
}

fn validate_specification_history(
    root: &Path,
    execution: &Value,
    execution_dir: &str,
    fingerprints: &BTreeMap<String, String>,
) -> Result<(), WorkError> {
    let invalid = |path: &str, reason: &str| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            "spec_update_history_invalid",
            "The execution history must be valid before revision.",
            json!({"path":path,"cause":reason,"next_command":"migration analyze"}),
        )
    };
    for path in fingerprints.keys() {
        if !(path.ends_with("/attempt.json")
            || path.contains("/corrections/") && path.ends_with(".json"))
        {
            continue;
        }
        let raw = LocalFiles.read_raw(&storage_path(root, path)?)?;
        let value =
            parse_json_contract(&raw).map_err(|_| invalid(path, "invalid_json_contract"))?;
        if path.ends_with("/attempt.json") {
            validate_attempt_bytes(&value, &raw)
                .map_err(|issue| invalid(path, issue.reason_code))?;
            validate_attempt_file_path(path, &value)
                .map_err(|issue| invalid(path, issue.reason_code))?;
        } else {
            validate_correction(&value).map_err(|issue| invalid(path, issue.reason_code))?;
            validate_correction_file_path(path, &value)
                .map_err(|issue| invalid(path, issue.reason_code))?;
            if render_correction(&value).map_err(|issue| invalid(path, issue.reason_code))? != raw {
                return Err(invalid(path, "noncanonical_json"));
            }
        }
    }
    for row in execution["tasks"].as_array().into_iter().flatten() {
        if let (Some(task), Some(attempt)) = (row["id"].as_str(), row["latest_attempt"].as_str()) {
            let path = format!("{execution_dir}/{task}/{attempt}/attempt.json");
            if !fingerprints.contains_key(&path) {
                return Err(invalid(&path, "missing_latest_attempt"));
            }
        }
    }
    Ok(())
}

fn semantic_error(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::Contract, reason, message, json!({}))
}

fn semantic_positions(value: &Value, rows: &Value) -> Result<Vec<String>, WorkError> {
    let positions = value.as_array().ok_or_else(|| {
        semantic_error("invalid_semantic_position", "Expected one-based positions.")
    })?;
    let source = rows.as_array().ok_or_else(|| {
        semantic_error(
            "invalid_semantic_reference",
            "The referenced collection is unavailable.",
        )
    })?;
    let mut result = Vec::new();
    for position in positions {
        let id = position
            .as_u64()
            .filter(|position| *position > 0)
            .and_then(|position| source.get(position as usize - 1))
            .and_then(|row| row["id"].as_str())
            .ok_or_else(|| {
                semantic_error(
                    "invalid_semantic_position",
                    "A one-based position must identify an existing item.",
                )
            })?;
        if result.iter().any(|previous| previous == id) {
            return Err(semantic_error(
                "invalid_semantic_position",
                "Semantic positions must be unique.",
            ));
        }
        result.push(id.to_owned());
    }
    Ok(result)
}

fn semantic_plan_rows(plan: &Value, field: &str, choices: &Value) -> Result<Value, WorkError> {
    let (prefix, required, references): (&str, &[&str], &[(&str, &str)]) = match field {
        "goals" => ("GOAL", &["statement"], &[]),
        "scope" => (
            "SCOPE",
            &["kind", "statement"],
            &[("goal_positions", "goals")],
        ),
        "dependencies" => ("DEPENDENCY", &["statement", "applies_to"], &[]),
        "risks" => (
            "RISK",
            &["condition", "impact", "mitigation", "applies_to"],
            &[],
        ),
        "milestones" => (
            "MILESTONE",
            &["statement", "deliverable_positions"],
            &[("deliverable_positions", "deliverables")],
        ),
        "deliverables" => (
            "DELIVERABLE",
            &["statement", "goal_positions", "acceptance_positions"],
            &[
                ("goal_positions", "goals"),
                ("acceptance_positions", "acceptance_criteria"),
            ],
        ),
        "acceptance_criteria" => (
            "ACCEPTANCE",
            &["statement", "deliverable_positions"],
            &[("deliverable_positions", "deliverables")],
        ),
        "decisions" => ("DECISION", &["statement", "rationale", "applies_to"], &[]),
        _ => {
            return Err(semantic_error(
                "spec_prepare_field",
                "Unsupported semantic Plan field.",
            ));
        }
    };
    let choices = choices.as_array().ok_or_else(|| {
        semantic_error(
            "invalid_semantic_items",
            "A semantic collection must be an array.",
        )
    })?;
    let source = plan[field].as_array();
    let mut next = source
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str())
        .filter_map(|id| id.rsplit_once('-')?.1.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    let mut keys = std::collections::BTreeSet::new();
    let mut positions = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for choice in choices {
        let object = choice.as_object().ok_or_else(|| {
            semantic_error("invalid_semantic_object", "A semantic object is required.")
        })?;
        if required.iter().any(|name| !object.contains_key(*name))
            || object.keys().any(|name| {
                name != "key"
                    && name != "existing_position"
                    && !required.contains(&name.as_str())
                    && !references.iter().any(|(semantic, _)| name == semantic)
            })
        {
            return Err(semantic_error(
                "invalid_object_fields",
                "The semantic object has missing or unknown fields.",
            ));
        }
        let key = choice["key"]
            .as_str()
            .filter(|key| {
                !key.is_empty()
                    && key.as_bytes()[0].is_ascii_lowercase()
                    && key.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || byte == b'_'
                            || byte == b'-'
                    })
            })
            .ok_or_else(|| semantic_error("invalid_semantic_key", "Use a lowercase local key."))?;
        if !keys.insert(key) {
            return Err(semantic_error(
                "duplicate_semantic_key",
                "Local keys must be unique.",
            ));
        }
        let id = if let Some(position) = object.get("existing_position") {
            let position = position
                .as_u64()
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    semantic_error("invalid_semantic_position", "Invalid source position.")
                })? as usize;
            if !positions.insert(position) {
                return Err(semantic_error(
                    "duplicate_semantic_position",
                    "A source item may be retained once.",
                ));
            }
            source
                .and_then(|rows| rows.get(position - 1))
                .and_then(|row| row["id"].as_str())
                .ok_or_else(|| {
                    semantic_error("invalid_semantic_position", "Invalid source position.")
                })?
                .to_owned()
        } else {
            next += 1;
            format!("{prefix}-{next:03}")
        };
        let mut formal = object.clone();
        formal.remove("key");
        formal.remove("existing_position");
        formal.insert("id".into(), json!(id));
        for (semantic, collection) in references {
            if let Some(value) = formal.remove(*semantic) {
                let ids = semantic_positions(&value, &plan[*collection])?;
                formal.insert(semantic.replace("_positions", "_ids"), json!(ids));
            }
        }
        if let Some(value) = formal.remove("applies_to") {
            let rows = value
                .as_array()
                .filter(|rows| !rows.is_empty())
                .ok_or_else(|| {
                    semantic_error(
                        "invalid_semantic_reference",
                        "applies_to requires semantic references.",
                    )
                })?;
            let mut ids = Vec::new();
            for reference in rows {
                let collection = reference["collection"].as_str().ok_or_else(|| {
                    semantic_error("invalid_semantic_reference", "Invalid Plan reference.")
                })?;
                let id = if collection == "plan"
                    && reference.as_object().is_some_and(|value| value.len() == 1)
                {
                    "PLAN".to_owned()
                } else {
                    let position = reference["position"]
                        .as_u64()
                        .filter(|position| *position > 0)
                        .ok_or_else(|| {
                            semantic_error("invalid_semantic_position", "Invalid Plan position.")
                        })?;
                    plan.get(collection)
                        .and_then(Value::as_array)
                        .and_then(|rows| rows.get(position as usize - 1))
                        .and_then(|row| row["id"].as_str())
                        .ok_or_else(|| {
                            semantic_error("invalid_semantic_reference", "Unknown Plan reference.")
                        })?
                        .to_owned()
                };
                if ids.contains(&id) {
                    return Err(semantic_error(
                        "invalid_semantic_reference",
                        "Plan references must be unique.",
                    ));
                }
                ids.push(id);
            }
            formal.insert("applies_to".into(), json!(ids));
        }
        result.push(Value::Object(formal));
    }
    Ok(Value::Array(result))
}

fn semantic_index_decisions(index: &Value, choices: &Value) -> Result<Value, WorkError> {
    let rows = choices.as_array().ok_or_else(|| {
        semantic_error(
            "invalid_semantic_items",
            "Index decisions must be an array.",
        )
    })?;
    let source = index["decisions"].as_array();
    let mut next = source
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str())
        .filter_map(|id| id.rsplit_once('-')?.1.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    let mut used = std::collections::BTreeSet::new();
    let mut keys = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for row in rows {
        let fields = row.as_object().ok_or_else(|| {
            semantic_error(
                "invalid_semantic_object",
                "Index decisions require objects.",
            )
        })?;
        if !["key", "statement", "rationale", "task_positions"]
            .iter()
            .all(|name| fields.contains_key(*name))
            || fields.keys().any(|name| {
                ![
                    "key",
                    "statement",
                    "rationale",
                    "task_positions",
                    "existing_position",
                ]
                .contains(&name.as_str())
            })
        {
            return Err(semantic_error(
                "invalid_object_fields",
                "Invalid index decision fields.",
            ));
        }
        let key = row["key"]
            .as_str()
            .filter(|key| !key.is_empty())
            .ok_or_else(|| semantic_error("invalid_semantic_key", "A local key is required."))?;
        if !keys.insert(key) {
            return Err(semantic_error(
                "duplicate_semantic_key",
                "Local keys must be unique.",
            ));
        }
        let id = if let Some(position) = fields.get("existing_position") {
            let position = position
                .as_u64()
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    semantic_error("invalid_semantic_position", "Invalid decision position.")
                })? as usize;
            if !used.insert(position) {
                return Err(semantic_error(
                    "duplicate_semantic_position",
                    "Decision positions must be unique.",
                ));
            }
            source
                .and_then(|rows| rows.get(position - 1))
                .and_then(|row| row["id"].as_str())
                .ok_or_else(|| {
                    semantic_error("invalid_semantic_position", "Invalid decision position.")
                })?
                .to_owned()
        } else {
            next += 1;
            format!("TASK-DECISION-{next:03}")
        };
        let ids = semantic_positions(&row["task_positions"], &index["tasks"])?;
        result.push(json!({"id":id,"statement":row["statement"],
            "rationale":row["rationale"],"task_ids":ids}));
    }
    Ok(Value::Array(result))
}

fn semantic_traceability(plan: &Value, choices: &Value) -> Result<Value, WorkError> {
    let fields = choices.as_object().ok_or_else(|| {
        semantic_error(
            "invalid_semantic_object",
            "Traceability needs a semantic object.",
        )
    })?;
    let groups = [
        ("goal_positions", "goals"),
        ("deliverable_positions", "deliverables"),
        ("acceptance_positions", "acceptance_criteria"),
        ("milestone_positions", "milestones"),
    ];
    if fields
        .keys()
        .any(|key| !groups.iter().any(|(name, _)| key == name))
        || groups[..3]
            .iter()
            .any(|(name, _)| !fields.contains_key(*name))
    {
        return Err(semantic_error(
            "invalid_object_fields",
            "Traceability has missing or unknown positions.",
        ));
    }
    let mut result = serde_json::Map::new();
    for (name, collection) in groups {
        if let Some(value) = fields.get(name) {
            result.insert(
                name.replace("_positions", "_ids"),
                json!(semantic_positions(value, &plan[collection])?),
            );
        }
    }
    Ok(Value::Object(result))
}

fn semantic_constraint_rows(plan: &Value, choices: &Value) -> Result<Value, WorkError> {
    let contract = |reason, message| WorkError::new(ExitCode::Contract, reason, message, json!({}));
    let choices = choices.as_array().ok_or_else(|| {
        contract(
            "invalid_semantic_items",
            "A semantic collection must be an array.",
        )
    })?;
    let source = plan["constraints"].as_array();
    let mut next_number = source
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str())
        .filter_map(|id| id.rsplit_once('-'))
        .filter_map(|(_, number)| number.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    let mut keys = std::collections::BTreeSet::new();
    let mut positions = std::collections::BTreeSet::new();
    let mut rows = Vec::new();
    for choice in choices {
        let object = choice
            .as_object()
            .ok_or_else(|| contract("invalid_semantic_object", "A semantic object is required."))?;
        if !["key", "statement", "applies_to"]
            .iter()
            .all(|key| object.contains_key(*key))
            || object.keys().any(|key| {
                !["key", "statement", "applies_to", "existing_position"].contains(&key.as_str())
            })
        {
            return Err(contract(
                "invalid_object_fields",
                "The semantic object has missing or unknown fields.",
            ));
        }
        let key = choice["key"]
            .as_str()
            .filter(|key| {
                !key.is_empty()
                    && key.as_bytes()[0].is_ascii_lowercase()
                    && key.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || byte == b'_'
                            || byte == b'-'
                    })
            })
            .ok_or_else(|| {
                contract(
                    "invalid_semantic_key",
                    "Use a local lowercase semantic key.",
                )
            })?;
        if !keys.insert(key) {
            return Err(contract(
                "duplicate_semantic_key",
                "Local semantic keys must be unique.",
            ));
        }
        let id = if let Some(value) = object.get("existing_position") {
            let position = value
                .as_u64()
                .filter(|position| *position > 0)
                .ok_or_else(|| {
                    contract(
                        "invalid_semantic_position",
                        "A one-based position must identify a source item.",
                    )
                })? as usize;
            let id = source
                .and_then(|source| source.get(position - 1))
                .and_then(|row| row["id"].as_str())
                .ok_or_else(|| {
                    contract(
                        "invalid_semantic_position",
                        "A one-based position must identify a source item.",
                    )
                })?;
            if !positions.insert(position) {
                return Err(contract(
                    "duplicate_semantic_position",
                    "An existing item may be retained once.",
                ));
            }
            id.to_owned()
        } else {
            next_number += 1;
            format!("CONSTRAINT-{next_number:03}")
        };
        let references = choice["applies_to"]
            .as_array()
            .filter(|rows| !rows.is_empty())
            .ok_or_else(|| {
                contract(
                    "invalid_semantic_reference",
                    "applies_to requires semantic references.",
                )
            })?;
        let mut applies_to = Vec::new();
        for reference in references {
            let group = reference["collection"].as_str().ok_or_else(|| {
                contract(
                    "invalid_semantic_reference",
                    "applies_to must identify one semantic Plan item.",
                )
            })?;
            let id = if group == "plan"
                && reference
                    .as_object()
                    .is_some_and(|object| object.len() == 1)
            {
                "PLAN"
            } else {
                let position = reference["position"]
                    .as_u64()
                    .filter(|position| *position > 0)
                    .ok_or_else(|| {
                        contract(
                            "invalid_semantic_position",
                            "A one-based position must identify a source item.",
                        )
                    })? as usize;
                plan.get(group)
                    .and_then(Value::as_array)
                    .and_then(|rows| rows.get(position - 1))
                    .and_then(|row| row["id"].as_str())
                    .ok_or_else(|| {
                        contract(
                            "invalid_semantic_reference",
                            "applies_to must identify one semantic Plan item.",
                        )
                    })?
            };
            if applies_to.contains(&id.to_owned()) {
                return Err(contract(
                    "invalid_semantic_reference",
                    "applies_to references must be unique.",
                ));
            }
            applies_to.push(id.to_owned());
        }
        rows.push(json!({"id":id,"statement":choice["statement"],"applies_to":applies_to}));
    }
    Ok(json!(rows))
}

fn resolve_plan_path(root: &Path, requirement_id: &str) -> Result<String, WorkError> {
    let mut candidates = Vec::new();
    for relative in work_json_files(root)? {
        let Ok(raw) = std::fs::read(storage_path(root, &relative)?) else {
            continue;
        };
        let Ok(document) = serde_json::from_slice::<Value>(&raw) else {
            continue;
        };
        if document["schema"] != "work-plan/v1" || document["requirement_id"] != requirement_id {
            continue;
        }
        if document["artifacts"]["plan"] != relative {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "spec_plan_binding_invalid",
                "A matching Plan has an invalid artifact binding.",
                json!({"path":relative}),
            ));
        }
        candidates.push(relative);
    }
    if candidates.len() != 1 {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            if candidates.is_empty() {
                "spec_plan_source_missing"
            } else {
                "spec_plan_source_ambiguous"
            },
            "Exactly one trusted Plan must match the requirement.",
            json!({"requirement_id":requirement_id,"candidates":candidates}),
        ));
    }
    Ok(candidates.remove(0))
}

fn sources(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    plan_path: &str,
) -> Result<OwnedBaseline, WorkError> {
    let plan_raw = LocalFiles.read_raw(&storage_path(root, plan_path)?)?;
    let plan = parse_json_contract(&plan_raw).map_err(|_| {
        with_migration_route(fail("invalid_json_contract", "The source Plan is invalid."))
    })?;
    let artifacts = &plan["artifacts"];
    let task_path = artifacts["task"]
        .as_str()
        .filter(|path| path.ends_with("/index.json"))
        .ok_or_else(|| {
            with_migration_route(fail(
                "spec_artifact_identity",
                "The Plan must route to a TASK collection index.",
            ))
        })?;
    if artifacts["plan"] != plan_path {
        return Err(with_migration_route(fail(
            "spec_artifact_identity",
            "The Plan artifact binding is invalid.",
        )));
    }
    let execution = artifacts["execution"].as_str().ok_or_else(|| {
        with_migration_route(fail(
            "spec_artifact_identity",
            "The Plan execution binding is invalid.",
        ))
    })?;
    let execution_path = format!("{execution}/index.json");
    let index_raw = LocalFiles
        .read_raw(&storage_path(root, task_path)?)
        .map_err(with_migration_route)?;
    let index = parse_json_contract(&index_raw).map_err(|_| {
        with_migration_route(fail("invalid_json_contract", "The TASK index is invalid."))
    })?;
    let directory = task_path.rsplit_once('/').map_or("", |(parent, _)| parent);
    let mut items = BTreeMap::new();
    for reference in index["tasks"].as_array().ok_or_else(|| {
        with_migration_route(fail(
            "invalid_task_index",
            "The TASK index has no item references.",
        ))
    })? {
        let id = reference["id"].as_str().ok_or_else(|| {
            with_migration_route(fail("invalid_task_index", "A TASK item has no ID."))
        })?;
        let relative = reference["path"].as_str().ok_or_else(|| {
            with_migration_route(fail("invalid_task_index", "A TASK item has no path."))
        })?;
        items.insert(
            id.to_owned(),
            LocalFiles
                .read_raw(&storage_path(root, &format!("{directory}/{relative}"))?)
                .map_err(with_migration_route)?,
        );
    }
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
    load_collection_with_file_state(
        &instructions,
        &skills,
        &paths,
        &repository,
        &roots,
        task_path,
        true,
    )
    .map_err(with_migration_route)?;
    let execution_raw = LocalFiles
        .read_raw(&storage_path(root, &execution_path)?)
        .map_err(with_migration_route)?;
    let execution_value = parse_json_contract(&execution_raw).map_err(|_| {
        with_migration_route(fail(
            "invalid_json_contract",
            "The execution index is invalid.",
        ))
    })?;
    validate_execution_index(&execution_value, &execution_raw).map_err(|issue| {
        with_migration_route(WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        ))
    })?;
    let history = crate::specification::storage::execution_history_bytes(root, execution)?;
    let history_sha256 = history
        .iter()
        .map(|(path, raw)| (path.clone(), fingerprint::history(raw)))
        .collect();
    validate_specification_history(root, &execution_value, execution, &history_sha256)?;
    Ok(OwnedBaseline {
        plan_path: plan_path.into(),
        task_path: task_path.into(),
        execution_path,
        plan_raw,
        index_raw,
        items,
        execution_raw,
        history,
    })
}

pub fn prepare_simple_update(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    input: SpecificationPrepareInput<'_>,
) -> Result<Value, WorkError> {
    let semantic = parse_json_contract(input.raw).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The Specification preparation is invalid.",
        )
    })?;
    validate_prepare_request(&semantic).map_err(|issue| {
        WorkError::new(
            if issue.artifact_integrity {
                ExitCode::ArtifactIntegrity
            } else {
                ExitCode::Contract
            },
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    if semantic["schema"] != "work-spec-prepare-request/v1" {
        return Err(fail(
            "spec_prepare_schema",
            "Use work-spec-prepare-request/v1.",
        ));
    }
    let requirement = semantic["requirement_id"]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| fail("empty_text_value", "A non-empty requirement is required."))?;
    let reason = semantic["reason"].as_str().ok_or_else(|| {
        fail(
            "invalid_contract_value",
            "A specification reason is required.",
        )
    })?;
    let edits = semantic["edits"]
        .as_array()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| fail("spec_prepare_edits", "Supply non-empty collection edits."))?;
    let plan_path = resolve_plan_path(root, requirement).map_err(with_migration_route)?;
    let plan_raw = LocalFiles.read_raw(&storage_path(root, &plan_path)?)?;
    let plan = parse_json_contract(&plan_raw).map_err(|_| {
        with_migration_route(fail("invalid_json_contract", "The source Plan is invalid."))
    })?;
    if plan["artifacts"]["task"]
        .as_str()
        .is_none_or(|path| !path.ends_with("/index.json"))
    {
        return Err(with_migration_route(WorkError::new(
            ExitCode::WorkflowState,
            "task_collection_required",
            "TASK writes require a collection index.json artifact.",
            json!({}),
        )));
    }
    let baseline = sources(root, skill_root, configs, &plan_path)?;
    let execution_dir = baseline
        .execution_path
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    require_no_spec_update(root, execution_dir, None)?;
    LocalWriterLock.require_idle(&storage_path(
        root,
        &format!("{execution_dir}/.work-state-writer.lock"),
    )?)?;
    let mut plan = parse_json_contract(&baseline.plan_raw)
        .map_err(|_| fail("invalid_json_contract", "The source Plan is invalid."))?;
    let mut index = parse_json_contract(&baseline.index_raw)
        .map_err(|_| fail("invalid_json_contract", "The TASK index is invalid."))?;
    let mut items = baseline
        .items
        .iter()
        .map(|(id, raw)| {
            parse_json_contract(raw)
                .map(|item| (id.clone(), item))
                .map_err(|_| fail("invalid_json_contract", "A TASK item is invalid."))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut normalized = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut plan_changed = false;
    let mut shared_index = false;
    let mut changed_item_set = false;
    let mut next_task = items
        .keys()
        .filter_map(|id| id.rsplit_once('-'))
        .filter_map(|(_, number)| number.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    let mut task_ids = index["tasks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let instruction_catalog = LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
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
    let mut nested_edits = BTreeMap::<String, serde_json::Map<String, Value>>::new();
    for edit in edits {
        if edit["target"]["artifact"] == "task_item" {
            if let (Some(id), Some(field)) =
                (edit["target"]["task_id"].as_str(), edit["field"].as_str())
            {
                if nested_groups.contains(&field) {
                    nested_edits
                        .entry(id.to_owned())
                        .or_default()
                        .insert(field.to_owned(), edit["semantic_after"].clone());
                }
            }
        }
    }
    let mut nested_candidates = BTreeMap::<String, Value>::new();
    for edit in edits {
        if edit["operation"] == "remove_task" {
            let position = edit["task_position"].as_u64().ok_or_else(|| {
                fail(
                    "invalid_semantic_position",
                    "A one-based TASK position is required.",
                )
            })? as usize;
            let original_tasks = index["tasks"].as_array().expect("validated TASK index");
            let id = original_tasks
                .get(position.wrapping_sub(1))
                .and_then(|row| row["id"].as_str())
                .filter(|_| position > 0)
                .ok_or_else(|| {
                    fail(
                        "invalid_semantic_position",
                        "A one-based TASK position must identify a source item.",
                    )
                })?
                .to_owned();
            if !seen.insert(("task_item".to_owned(), Some(id.clone()), "/".to_owned())) {
                return Err(fail(
                    "spec_prepare_duplicate",
                    "Each collection target may be edited once.",
                ));
            }
            let execution = parse_json_contract(&baseline.execution_raw)
                .map_err(|_| fail("invalid_json_contract", "The execution index is invalid."))?;
            if execution["tasks"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|row| {
                    row["id"] == id
                        && (row["latest_attempt"].is_object() || row["status"] != "pending")
                })
            {
                return Err(fail(
                    "spec_remove_task_history",
                    "A TASK with execution history requires a dedicated lifecycle operation.",
                ));
            }
            let before = items
                .remove(&id)
                .ok_or_else(|| fail("spec_prepare_task_id", "Unknown TASK ID."))?;
            normalized.push(json!({"artifact":"task_item","task_id":id,
                "operation":"remove","path":"/","before":before}));
            task_ids.retain(|task_id| task_id != &id);
            changed_item_set = true;
            continue;
        }
        if edit["operation"] == "add_task" {
            let candidate = &edit["task"];
            next_task += 1;
            let id = format!("TASK-{next_task:03}");
            let dependency_positions = candidate["dependency_positions"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let dependencies = dependency_positions
                .iter()
                .map(|position| {
                    position
                        .as_u64()
                        .and_then(|number| task_ids.get(number.wrapping_sub(1) as usize))
                        .cloned()
                        .ok_or_else(|| {
                            fail(
                                "invalid_semantic_position",
                                "A TASK dependency position is invalid.",
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if dependencies.iter().any(|dep| dep == &id)
                || dependencies
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != dependencies.len()
            {
                return Err(fail(
                    "invalid_semantic_reference",
                    "TASK dependencies must be unique and cannot include self.",
                ));
            }
            let selected = candidate["selected_paths"]
                .as_array()
                .ok_or_else(|| {
                    fail(
                        "invalid_source_selection",
                        "TASK selected instruction paths are required.",
                    )
                })?
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        fail(
                            "invalid_source_selection",
                            "TASK selected instruction paths must be strings.",
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let references = candidate["references"]
                .as_array()
                .ok_or_else(|| {
                    fail(
                        "invalid_source_selection",
                        "TASK instruction references are required.",
                    )
                })?
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        fail(
                            "invalid_source_selection",
                            "TASK instruction references must be strings.",
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let selection =
                select_instructions(&instruction_catalog, "task", &selected, &references)?;
            let acceptance = plan["acceptance_criteria"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| row["id"].as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            let dependency_files = dependencies
                .iter()
                .map(|dep| {
                    (
                        dep.clone(),
                        items[dep]["files"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .enumerate()
                            .filter_map(|(position, row)| {
                                row["id"]
                                    .as_str()
                                    .map(|id| (format!("existing-{}", position + 1), id.to_owned()))
                            })
                            .collect::<BTreeMap<_, _>>(),
                    )
                })
                .collect::<BTreeMap<_, _>>();
            let nested = build_semantic_candidate(
                &candidate["candidate"],
                &acceptance,
                &dependencies,
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
            let mut item = json!({"schema":"work-task-item/v1","id":id,
                "title":candidate["title"],"goal":candidate["goal"],"skill_id":candidate["skill_id"],
                "instruction_selection":selection,
                "traceability":{"goal_ids":plan["goals"].as_array().into_iter().flatten().map(|row| row["id"].clone()).collect::<Vec<_>>(),
                    "deliverable_ids":plan["deliverables"].as_array().into_iter().flatten().map(|row| row["id"].clone()).collect::<Vec<_>>(),
                    "acceptance_ids":acceptance}});
            if !dependencies.is_empty() {
                item["dependencies"] = json!(dependencies);
            }
            for (field, value) in nested.as_object().expect("nested candidate") {
                item[field] = value.clone();
            }
            normalized.push(
                json!({"artifact":"task_item","task_id":id,"operation":"add",
                "path":"/","after":item}),
            );
            items.insert(id.clone(), item);
            task_ids.push(id);
            changed_item_set = true;
            continue;
        }
        let target = &edit["target"];
        let artifact = target["artifact"].as_str().unwrap_or("");
        let field = edit["field"].as_str().unwrap_or("");
        let task_id = target["task_id"].as_str();
        let simple = match artifact {
            "plan" | "task_index" => ["title", "summary"].contains(&field),
            "task_item" => ["title", "goal"].contains(&field),
            _ => false,
        };
        let semantic = match artifact {
            "plan" => [
                "goals",
                "scope",
                "constraints",
                "dependencies",
                "risks",
                "milestones",
                "deliverables",
                "acceptance_criteria",
                "decisions",
            ]
            .contains(&field),
            "task_index" => ["decisions", "execution_defaults"].contains(&field),
            "task_item" => {
                ["traceability", "dependencies"].contains(&field) || nested_groups.contains(&field)
            }
            _ => false,
        };
        if !(simple || semantic)
            || (simple && edit["after"].as_str().is_none())
            || !seen.insert((
                artifact.to_owned(),
                task_id.map(str::to_owned),
                field.to_owned(),
            ))
        {
            return Err(fail(
                "spec_prepare_field",
                "This Specification preparation needs a supported, unique field edit.",
            ));
        }
        let after = match (artifact, field) {
            ("plan", "constraints") => semantic_constraint_rows(&plan, &edit["semantic_after"])?,
            ("plan", _) if semantic => semantic_plan_rows(&plan, field, &edit["semantic_after"])?,
            ("task_index", "decisions") => {
                semantic_index_decisions(&index, &edit["semantic_after"])?
            }
            ("task_index", "execution_defaults") => {
                let value = &edit["semantic_after"];
                let valid = value.as_object().is_some_and(|fields| {
                    fields.len() == 3
                        && ["working_directory", "os", "shell"]
                            .iter()
                            .all(|key| fields.get(*key).is_some_and(Value::is_string))
                });
                if !valid {
                    return Err(semantic_error(
                        "invalid_object_fields",
                        "Execution defaults need working directory, OS and shell.",
                    ));
                }
                value.clone()
            }
            ("task_item", "traceability") => semantic_traceability(&plan, &edit["semantic_after"])?,
            ("task_item", "dependencies") => {
                let refs = task_ids
                    .iter()
                    .map(|id| json!({"id":id}))
                    .collect::<Vec<_>>();
                json!(semantic_positions(&edit["semantic_after"], &json!(refs))?)
            }
            ("task_item", group) if nested_groups.contains(&group) => {
                let id = task_id.expect("validated TASK target");
                if !nested_candidates.contains_key(id) {
                    let current = items
                        .get(id)
                        .ok_or_else(|| fail("spec_prepare_task_id", "Unknown TASK ID."))?;
                    let dependencies = edits
                        .iter()
                        .find(|row| {
                            row["target"]["task_id"] == id && row["field"] == "dependencies"
                        })
                        .map(|row| {
                            let refs = task_ids
                                .iter()
                                .map(|id| json!({"id":id}))
                                .collect::<Vec<_>>();
                            semantic_positions(&row["semantic_after"], &json!(refs))
                        })
                        .transpose()?
                        .unwrap_or_else(|| {
                            current["dependencies"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|value| value.as_str().map(str::to_owned))
                                .collect()
                        });
                    let dependency_files = dependencies
                        .iter()
                        .map(|dep| {
                            let files = items[dep]["files"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .enumerate()
                                .filter_map(|(position, row)| {
                                    row["id"].as_str().map(|id| {
                                        (format!("existing-{}", position + 1), id.to_owned())
                                    })
                                })
                                .collect::<BTreeMap<_, _>>();
                            (dep.clone(), files)
                        })
                        .collect::<BTreeMap<_, _>>();
                    let acceptance = plan["acceptance_criteria"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|row| row["id"].as_str().map(str::to_owned))
                        .collect::<Vec<_>>();
                    let replacements = Value::Object(nested_edits[id].clone());
                    let built = build_semantic_patch(
                        current,
                        &replacements,
                        &acceptance,
                        &dependencies,
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
                    nested_candidates.insert(id.to_owned(), built);
                }
                nested_candidates[id][group].clone()
            }
            _ => edit["after"].clone(),
        };
        let source = match artifact {
            "plan" => &mut plan,
            "task_index" => &mut index,
            _ => items
                .get_mut(task_id.expect("TASK ID"))
                .ok_or_else(|| fail("spec_prepare_task_id", "Unknown TASK ID."))?,
        };
        let before = source
            .get(field)
            .cloned()
            .ok_or_else(|| fail("spec_edit_state", "A replace target does not exist."))?;
        if before == after {
            return Err(fail(
                "spec_edit_state",
                "A specification edit must change its target.",
            ));
        }
        source[field] = after.clone();
        if artifact == "plan" {
            plan_changed = true;
        } else {
            if artifact == "task_index" {
                shared_index = true;
            }
            let mut row = json!({"artifact":artifact,"operation":"replace",
                "path":format!("/{field}"),"before":before,"after":after});
            if let Some(id) = task_id {
                row["task_id"] = json!(id);
            }
            normalized.push(row);
        }
    }
    let plan_raw = if plan_changed {
        render_plan_value(&plan).map_err(|_| {
            fail(
                "invalid_contract_value",
                "The revised Plan cannot be rendered.",
            )
        })?
    } else {
        baseline.plan_raw.clone()
    };
    if plan_changed {
        bind_plan_source(&plan_raw, &mut index).map_err(|_| {
            fail(
                "invalid_contract_value",
                "The revised Plan cannot be bound.",
            )
        })?;
    }
    if changed_item_set {
        let mut selections = Vec::new();
        for item in items.values() {
            let value = &item["instruction_selection"];
            let selected = value["selected_paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| row.as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            let references = value["references"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|row| row.as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            selections.push(load_instructions(
                &instruction_catalog,
                "task",
                &selected,
                &references,
            )?);
        }
        index["instruction_selection"] = task_document_selection(&selections)?;
    }
    let old_spec = index["spec_id"]
        .as_str()
        .and_then(|id| id.rsplit_once('-'))
        .and_then(|(_, number)| number.parse::<usize>().ok())
        .ok_or_else(|| {
            fail(
                "spec_update_candidate",
                "The source TASK spec ID is invalid.",
            )
        })?;
    let spec_id = format!("TASK-SPEC-{:03}", old_spec + 1);
    index["spec_id"] = json!(spec_id);
    index["readiness"]["spec_id"] = json!(spec_id);
    let mut updated_references = Vec::new();
    let mut item_raw = BTreeMap::new();
    for (id, item) in &items {
        let bytes = render_task(item, TaskDocumentKind::Item).map_err(|_| {
            fail(
                "invalid_contract_value",
                "The revised TASK item cannot be rendered.",
            )
        })?;
        updated_references.push(json!({"id":id,"path":format!("tasks/{id}.json")}));
        item_raw.insert(id.clone(), bytes);
    }
    index["tasks"] = Value::Array(updated_references);
    let change_number = index["changes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            row["id"]
                .as_str()
                .and_then(|id| id.rsplit_once('-'))
                .and_then(|(_, number)| number.parse::<usize>().ok())
        })
        .max()
        .unwrap_or(0)
        + 1;
    let removed_item = baseline.items.keys().any(|id| !items.contains_key(id));
    let mut affected = if plan_changed || shared_index || removed_item {
        items
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
    } else {
        items
            .iter()
            .filter_map(|(id, item)| {
                baseline
                    .items
                    .get(id)
                    .and_then(|raw| parse_json_contract(raw).ok())
                    .filter(|old| old == item)
                    .map_or(Some(id.clone()), |_| None)
            })
            .collect::<std::collections::BTreeSet<_>>()
    };
    loop {
        let downstream = items
            .iter()
            .filter_map(|(id, item)| {
                item["dependencies"]
                    .as_array()
                    .is_some_and(|rows| {
                        rows.iter()
                            .any(|row| row.as_str().is_some_and(|dep| affected.contains(dep)))
                    })
                    .then_some(id.clone())
            })
            .collect::<std::collections::BTreeSet<_>>();
        if downstream.is_subset(&affected) {
            break;
        }
        affected.extend(downstream);
    }
    let affected = affected.into_iter().collect::<Vec<_>>();
    if normalized.is_empty() {
        normalized.push(json!({"artifact":"task_index","operation":"replace","path":"/source_plan",
            "before":parse_json_contract(&baseline.index_raw).map_err(|_| fail("invalid_json_contract", "The TASK index is invalid."))?["source_plan"],
            "after":index["source_plan"]}));
    }
    let mut changes = index["changes"].as_array().cloned().unwrap_or_default();
    changes.push(json!({"id":format!("TASK-CHANGE-{change_number:03}"),
        "spec_id":spec_id,"date":input.date,"reason":reason,
        "affected_ids":affected,"edits":normalized}));
    index["changes"] = Value::Array(changes);
    let index_raw = reconcile_artifact_bindings(
        &plan_raw,
        &mut index,
        &item_raw,
        None,
        &BTreeSet::from([ArtifactNode::PlanBytes]),
    )
    .map_err(|_| {
        fail(
            "invalid_contract_value",
            "The revised TASK index cannot be rendered.",
        )
    })?;
    if index_raw == baseline.index_raw {
        return Err(fail("spec_edit_state", "No Specification bytes changed."));
    }
    let expected = fingerprint::specification_baseline(
        &baseline.plan_raw,
        &baseline.index_raw,
        &baseline.execution_raw,
        &baseline.items,
    );
    let request = json!({"schema":"work-spec-update-request/v1","reason":reason,
        "expected":expected,"plan":plan,"task_index":index,"task_items":items});
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
    let preview = preview_update(
        &instructions,
        &skills,
        &paths,
        &roots,
        &request,
        baseline.borrow(),
    )?
    .result;
    if let Some(output) = input.output_file {
        let raw = render_task(&request, TaskDocumentKind::SpecificationRequest).map_err(|_| {
            fail(
                "invalid_contract_value",
                "The Specification request cannot be rendered.",
            )
        })?;
        LocalFiles.create_new(output, &raw)?;
    }
    Ok(work_model::specification::verified::<
        work_model::specification::SpecPrepare,
    >(
        json!({"schema":"work-spec-prepare/v1","request":request,"preview":preview,
        "output_file":input.output_file.map(|path| path.to_string_lossy().to_string()),
        "transport":{"request_field":"request","request_schema":"work-spec-update-request/v1",
            "output_file":input.output_file.map(|path| path.to_string_lossy().to_string())},
        "next_step":{"command":"specification preview","input":"request"}}),
    ))
}

pub fn update_from_project(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    input: SpecificationProjectRequest<'_>,
) -> Result<Value, WorkError> {
    let request = parse_json_contract(input.raw).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The Specification update request is invalid.",
        )
    })?;
    validate_update_request(&request).map_err(|issue| {
        WorkError::new(
            if issue.artifact_integrity {
                ExitCode::ArtifactIntegrity
            } else {
                ExitCode::Contract
            },
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let artifacts = &request["plan"]["artifacts"];
    let plan_path = artifacts["plan"].as_str().ok_or_else(|| {
        fail(
            "spec_artifact_identity",
            "The request Plan has no artifact path.",
        )
    })?;
    let execution = artifacts["execution"].as_str().ok_or_else(|| {
        fail(
            "spec_artifact_identity",
            "The request has no execution path.",
        )
    })?;
    let (transaction, mut result) = if matches!(input.operation, SpecOperation::Recover) {
        let approval = input.approved_sha256.ok_or_else(|| {
            fail(
                "spec_update_approval_changed",
                "Recovery requires the approved SHA-256.",
            )
        })?;
        let id = derived_transaction_id("UPDATE", approval).map_err(|issue| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        let relative = work_operations::derivation::publication::journal_path(
            execution,
            work_operations::derivation::publication::JournalKind::SpecificationUpdate(&id),
        );
        let raw = LocalFiles.read_raw(&storage_path(root, &relative)?)?;
        let journal = parse_json_contract(&raw).map_err(|_| {
            fail(
                "invalid_json_contract",
                "The Specification journal is invalid.",
            )
        })?;
        validate_transaction(&journal).map_err(|issue| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
        if render_transaction(&journal).ok().as_deref() != Some(raw.as_slice())
            || journal["metadata"]["request"] != request
        {
            return Err(fail(
                "spec_update_recovery_request",
                "Recovery requires the identical request.",
            ));
        }
        let result = work_model::specification::verified::<work_model::specification::SpecUpdate>(
            json!({"schema":"work-spec-update/v1","status":"valid",
            "requirement_id":request["plan"]["requirement_id"],"record_id":id,
            "approved_sha256":journal["approval_sha256"],
            "affected_task_ids":journal["metadata"]["affected_task_ids"],"artifacts":artifacts}),
        );
        (journal, result)
    } else {
        require_no_spec_update(root, execution, None)?;
        let baseline = sources(root, skill_root, configs, plan_path)?;
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
        let preview = preview_update(
            &instructions,
            &skills,
            &paths,
            &roots,
            &request,
            baseline.borrow(),
        )?;
        (preview.transaction, preview.result)
    };
    if matches!(input.operation, SpecOperation::Validate) {
        return Ok(result);
    }
    if input.approved_sha256 != transaction["approval_sha256"].as_str() {
        return Err(fail(
            "spec_update_approval_changed",
            "The approved transaction changed.",
        ));
    }
    let journal = work_operations::derivation::publication::journal_path(
        execution,
        work_operations::derivation::publication::JournalKind::SpecificationUpdate(
            transaction["transaction_id"]
                .as_str()
                .expect("transaction ID"),
        ),
    );
    let marker = work_operations::derivation::publication::completion_marker_path(&journal);
    let ignored = if matches!(input.operation, SpecOperation::Recover) {
        Some(journal.as_str())
    } else {
        None
    };
    require_no_spec_update(root, execution, ignored)?;
    if json!(execution_history_fingerprints(root, execution)?)
        != transaction["metadata"]["history_sha256"]
    {
        return Err(fail(
            "spec_update_history_changed",
            "Execution history changed before publication.",
        ));
    }
    let writer = storage_path(root, &format!("{execution}/.work-state-writer.lock"))?;
    let _guard = LocalWriterLock.acquire(&writer)?;
    if json!(execution_history_fingerprints(root, execution)?)
        != transaction["metadata"]["history_sha256"]
    {
        return Err(fail(
            "spec_update_history_changed",
            "Execution history changed before exclusive publication.",
        ));
    }
    if matches!(input.operation, SpecOperation::Apply) {
        write_journal(root, &journal, &transaction)?;
    }
    let published = publish_journal(root, &journal, &marker).map_err(|_| {
        WorkError::new(
            ExitCode::IoFailure,
            "spec_update_interrupted",
            "Preserve the specification transaction and obtain recovery authorization.",
            json!({"recovery_required":true,"record":journal}),
        )
    })?;
    sources(root, skill_root, configs, plan_path)?;
    for (path, expected) in transaction["metadata"]["candidate_sha256"]
        .as_object()
        .expect("candidate SHA")
    {
        if fingerprint::raw(&LocalFiles.read_raw(&storage_path(root, path)?)?)
            != expected.as_str().unwrap_or("")
        {
            return Err(fail(
                "spec_update_post_write",
                "An installed collection artifact differs from approval.",
            ));
        }
    }
    result
        .as_object_mut()
        .expect("result object")
        .remove("transaction");
    result
        .as_object_mut()
        .expect("result object")
        .remove("next_step");
    result["status"] = json!(if matches!(input.operation, SpecOperation::Recover) {
        "recovered"
    } else {
        "updated"
    });
    result["publication_status"] = published["status"].clone();
    let verification = json!({"schema":"work-spec-verification-request/v1",
        "requirement_id":request["plan"]["requirement_id"],"artifacts":artifacts,
        "record_id":transaction["transaction_id"]});
    result["verification_request"] = verification;
    result["next_step"] = json!({"command":"specification verify","input":"verification_request"});
    Ok(result)
}

pub fn verify_from_project(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request_raw: &[u8],
) -> Result<Value, WorkError> {
    let request = parse_json_contract(request_raw).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The Specification verification request is invalid.",
        )
    })?;
    validate_verification_request(&request).map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    if request["schema"] != "work-spec-verification-request/v1" {
        return Err(fail(
            "invalid_contract_value",
            "The verification request schema is invalid.",
        ));
    }
    let artifacts = &request["artifacts"];
    let execution = artifacts["execution"].as_str().ok_or_else(|| {
        fail(
            "invalid_contract_value",
            "The verification execution path is invalid.",
        )
    })?;
    let id = request["record_id"].as_str().ok_or_else(|| {
        fail(
            "invalid_contract_value",
            "The verification record ID is invalid.",
        )
    })?;
    let relative = work_operations::derivation::publication::journal_path(
        execution,
        work_operations::derivation::publication::JournalKind::SpecificationUpdate(id),
    );
    let raw = LocalFiles.read_raw(&storage_path(root, &relative)?)?;
    let journal = parse_json_contract(&raw).map_err(|_| {
        fail(
            "invalid_json_contract",
            "The Specification journal is invalid.",
        )
    })?;
    validate_transaction(&journal).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let marker = LocalFiles.read_raw(&storage_path(
        root,
        &work_operations::derivation::publication::completion_marker_path(&relative),
    )?)?;
    if marker != completion_marker(&raw) {
        return Err(fail(
            "spec_verify_completion_mismatch",
            "The completion marker does not match.",
        ));
    }
    for (path, expected) in journal["metadata"]["candidate_sha256"]
        .as_object()
        .expect("candidate SHA")
    {
        if fingerprint::raw(&LocalFiles.read_raw(&storage_path(root, path)?)?)
            != expected.as_str().unwrap_or("")
        {
            return Err(fail(
                "spec_verify_state_changed",
                "A candidate file changed.",
            ));
        }
    }
    let baseline = sources(
        root,
        skill_root,
        configs,
        artifacts["plan"].as_str().unwrap_or(""),
    )?;
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
    let collection = load_collection_with_file_state(
        &instructions,
        &skills,
        &paths,
        &repository,
        &roots,
        &baseline.task_path,
        true,
    )?;
    Ok(work_model::specification::verified::<
        work_model::specification::SpecVerification,
    >(
        json!({"schema":"work-spec-verification/v1","status":"verified","verified":true,
        "record_id":id,"requirement_id":request["requirement_id"],"artifacts":artifacts,
        "task_collection_sha256":collection["task_collection_sha256"],"journal_sha256":fingerprint::journal(&raw),
        "verification_scope":"exact_specification_result","execution_authorized":false,
        "next_step":"normal_execute_preflight"}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constraint_semantic_replacement_preserves_id_and_resolves_plan_position() {
        let plan = json!({"goals":[{"id":"GOAL-001"}],"constraints":[
            {"id":"CONSTRAINT-001","statement":"Original","applies_to":["GOAL-001"]}]});
        let choices = json!([{"key":"boundary","existing_position":1,
            "statement":"Confirmed boundary","applies_to":[{"collection":"goals","position":1}]}]);
        assert_eq!(
            semantic_constraint_rows(&plan, &choices).unwrap(),
            json!([{"id":"CONSTRAINT-001","statement":"Confirmed boundary",
                "applies_to":["GOAL-001"]}])
        );
        assert_eq!(
            semantic_constraint_rows(
                &plan,
                &json!([{"key":"new",
            "statement":"New boundary","applies_to":[{"collection":"plan"}]}])
            )
            .unwrap(),
            json!([{"id":"CONSTRAINT-002","statement":"New boundary","applies_to":["PLAN"]}])
        );
    }

    #[test]
    fn constraint_semantic_prepare_update_and_verify_preserve_formal_identity() {
        use work_operations::execution::index::{
            build_initial_execution_index, render_execution_index,
        };
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-update/plan-summary");
        let root = std::env::temp_dir().join(format!(
            "work-spec-constraint-{}-{}",
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
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let plan_path = root.join("outputs/work/plans/example.json");
        let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        plan["constraints"] = json!([{"id":"CONSTRAINT-001","statement":"Original boundary",
            "applies_to":["GOAL-001"]}]);
        let plan_raw = render_plan_value(&plan).unwrap();
        fs::write(&plan_path, &plan_raw).unwrap();
        let index_path = root.join("outputs/work/tasks/example/index.json");
        let mut index: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
        index["source_plan"]["canonical_sha256"] = json!(raw_sha256(&plan_raw));
        fs::write(
            &index_path,
            render_task(&index, TaskDocumentKind::Index).unwrap(),
        )
        .unwrap();
        let skill = repo.join("../skills/work");
        let collection = load_collection_with_file_state(
            &LocalHierarchyCatalog {
                skill_root: skill.clone(),
            },
            &LocalSkillCatalog { roots: vec![] },
            &LocalPlanStorage {
                project_root: root.clone(),
            },
            &LocalTaskStorage {
                project_root: root.clone(),
            },
            &[],
            "outputs/work/tasks/example/index.json",
            true,
        )
        .unwrap();
        let execution =
            build_initial_execution_index(&collection["collection_contract"], &collection).unwrap();
        let execution_path = root.join("outputs/work/executions/example/index.json");
        fs::create_dir_all(execution_path.parent().unwrap()).unwrap();
        fs::write(&execution_path, render_execution_index(&execution).unwrap()).unwrap();
        let semantic = serde_json::to_vec(&json!({"schema":"work-spec-prepare-request/v1",
            "requirement_id":"example","reason":"Confirm the constraint wording.",
            "edits":[{"target":{"artifact":"plan"},"field":"constraints",
                "semantic_after":[{"key":"boundary","existing_position":1,
                    "statement":"Confirmed boundary","applies_to":[{"collection":"goals","position":1}]}]}]})).unwrap();
        let prepared = prepare_simple_update(
            &root,
            &skill,
            &[],
            SpecificationPrepareInput {
                raw: &semantic,
                date: "2026-09-26",
                output_file: None,
            },
        )
        .unwrap();
        assert_eq!(
            prepared["preview"]["changed_fields"],
            json!(["/plan/constraints", "/task_index/source_plan"])
        );
        let request = serde_json::to_vec(&prepared["request"]).unwrap();
        let preview = update_from_project(
            &root,
            &skill,
            &[],
            SpecificationProjectRequest {
                raw: &request,
                operation: SpecOperation::Validate,
                approved_sha256: None,
            },
        )
        .unwrap();
        let approved = preview["approved_sha256"].as_str().unwrap();
        let published = update_from_project(
            &root,
            &skill,
            &[],
            SpecificationProjectRequest {
                raw: &request,
                operation: SpecOperation::Apply,
                approved_sha256: Some(approved),
            },
        )
        .unwrap();
        assert_eq!(published["status"], "updated");
        let installed: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        assert_eq!(
            installed["constraints"],
            json!([{"id":"CONSTRAINT-001",
            "statement":"Confirmed boundary","applies_to":["GOAL-001"]}])
        );
        let verified = verify_from_project(
            &root,
            &skill,
            &[],
            &serde_json::to_vec(&published["verification_request"]).unwrap(),
        )
        .unwrap();
        assert_eq!(verified["verified"], true);
        assert_eq!(verified["execution_authorized"], false);
    }

    #[test]
    fn invalid_update_requests_are_rejected_before_source_lookup_for_every_operation() {
        let registry: Value = work_model::contract_data::registry_value();
        let example = &registry["items"]["work-spec-update-request/v1"]["description"]["example"];
        let root = std::env::temp_dir().join(format!(
            "work-spec-update-invalid-before-io-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skill = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        for operation in [
            SpecOperation::Validate,
            SpecOperation::Apply,
            SpecOperation::Recover,
        ] {
            for old_schema in [false, true] {
                let mut request = example.clone();
                if old_schema {
                    request["schema"] = json!("work-spec-update-request/v2");
                } else {
                    request["task"] = request
                        .as_object_mut()
                        .unwrap()
                        .remove("task_index")
                        .unwrap();
                }
                let raw = serde_json::to_vec(&request).unwrap();
                let error = update_from_project(
                    &root,
                    skill,
                    &[],
                    SpecificationProjectRequest {
                        raw: &raw,
                        operation: match operation {
                            SpecOperation::Validate => SpecOperation::Validate,
                            SpecOperation::Apply => SpecOperation::Apply,
                            SpecOperation::Recover => SpecOperation::Recover,
                        },
                        approved_sha256: None,
                    },
                )
                .unwrap_err();
                assert_eq!(
                    error.reason_code,
                    if old_schema {
                        "spec_update_schema"
                    } else {
                        "invalid_object_fields"
                    }
                );
                assert!(!root.exists());
            }
        }
    }

    #[test]
    fn invalid_prepare_request_is_rejected_before_source_lookup() {
        let root = std::env::temp_dir().join(format!(
            "work-spec-invalid-before-io-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skill = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        for edits in [json!([]), json!({})] {
            let raw = serde_json::to_vec(&json!({"schema":"work-spec-prepare-request/v1",
                "requirement_id":"example","reason":"Review","edits":edits}))
            .unwrap();
            let error = prepare_simple_update(
                &root,
                skill,
                &[],
                SpecificationPrepareInput {
                    raw: &raw,
                    date: "2026-09-01",
                    output_file: None,
                },
            )
            .unwrap_err();
            assert_eq!(error.reason_code, "spec_prepare_edits");
            assert!(!root.exists());
        }
    }

    #[test]
    fn malformed_verification_request_is_rejected_before_journal_lookup() {
        let root = std::env::temp_dir().join(format!(
            "work-spec-verify-invalid-before-io-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skill = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        let raw = serde_json::to_vec(&json!({"schema":"work-spec-verification-request/v1",
            "requirement_id":"example","artifacts":{"plan":"p"},
            "record_id":"SPEC-UPDATE-ABCDEF012345"}))
        .unwrap();
        let error = verify_from_project(&root, skill, &[], &raw).unwrap_err();
        assert_eq!(error.exit_code, ExitCode::Contract);
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert!(!root.exists());
    }

    #[test]
    fn plan_source_resolution_requires_one_matching_custom_binding() {
        let root = std::env::temp_dir().join(format!(
            "work-spec-plan-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(
            resolve_plan_path(&root, "example").unwrap_err().reason_code,
            "spec_plan_source_missing"
        );
        let custom = "outputs/work/custom/confirmed-plan.json";
        let first = root.join(custom);
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::write(
            &first,
            serde_json::to_vec(&json!({"schema":"work-plan/v1","requirement_id":"example",
                "artifacts":{"plan":custom,"task":"outputs/work/custom/tasks/index.json",
                    "execution":"outputs/work/custom/execution"}}))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(resolve_plan_path(&root, "example").unwrap(), custom);
        let other = "outputs/work/other-plan.json";
        std::fs::write(
            root.join(other),
            serde_json::to_vec(&json!({"schema":"work-plan/v1","requirement_id":"example",
                "artifacts":{"plan":other}}))
            .unwrap(),
        )
        .unwrap();
        let error = resolve_plan_path(&root, "example").unwrap_err();
        assert_eq!(error.reason_code, "spec_plan_source_ambiguous");
        assert_eq!(error.details["candidates"], json!([custom, other]));
        std::fs::write(
            root.join(other),
            serde_json::to_vec(&json!({"schema":"work-plan/v1","requirement_id":"example",
                "artifacts":{"plan":"outputs/work/wrong.json"}}))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            resolve_plan_path(&root, "example").unwrap_err().reason_code,
            "spec_plan_binding_invalid"
        );
    }

    fn assert_request_bytes(value: &Value, expected: &[u8]) {
        let actual = render_task(value, TaskDocumentKind::SpecificationRequest).unwrap();
        let actual_text = String::from_utf8(actual.clone()).unwrap();
        let expected_text = String::from_utf8(expected.to_vec()).unwrap();
        let first_difference = actual_text
            .lines()
            .zip(expected_text.lines())
            .enumerate()
            .find(|(_, (left, right))| left != right);
        assert!(
            actual == expected,
            "first differing line: {first_difference:?}"
        );
    }
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn semantic_plan_and_task_edits_build_one_complete_candidate() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/specification-update");
        let root = std::env::temp_dir().join(format!(
            "work-spec-semantic-candidate-{}-{}",
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
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let semantic = serde_json::to_vec(&json!({
            "schema":"work-spec-prepare-request/v1",
            "requirement_id":"example",
            "reason":"Confirmed semantic revision",
            "edits":[
                {"target":{"artifact":"plan"},"field":"goals",
                    "semantic_after":[{"key":"outcome","existing_position":1,
                        "statement":"Deliver the revised outcome."}]},
                {"target":{"artifact":"plan"},"field":"scope",
                    "semantic_after":[{"key":"boundary","existing_position":1,
                        "kind":"in_scope","statement":"Handle the revised scope.",
                        "goal_positions":[1]}]},
                {"target":{"artifact":"plan"},"field":"acceptance_criteria",
                    "semantic_after":[{"key":"accepted","existing_position":1,
                        "statement":"The revised result is observable.",
                        "deliverable_positions":[1]}]},
                {"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"commands",
                    "semantic_after":[{"key":"check","existing_position":1,
                        "mode":"argv","argv":["python","-V"]}]},
                {"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"steps",
                    "semantic_after":[
                        {"key":"modify","existing_position":1,"action":"Modify the source.",
                            "references":[{"kind":"files","key":"existing-1"}]},
                        {"key":"validate","existing_position":2,"action":"Run revised validation.",
                            "references":[{"kind":"commands","key":"check"},
                                {"kind":"validations","key":"verify"}]}]},
                {"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"validations",
                    "semantic_after":[{"key":"verify","existing_position":1,
                        "kind":"automated","command_keys":["check"],
                        "pass_condition":"Version command succeeds.",
                        "acceptance_positions":[1]}]}
            ]
        }))
        .unwrap();
        let prepared = prepare_simple_update(
            &root,
            &repo.join("../skills/work"),
            &[],
            SpecificationPrepareInput {
                raw: &semantic,
                date: "2026-09-30",
                output_file: None,
            },
        )
        .unwrap();
        assert_eq!(prepared["preview"]["status"], "valid");
        assert_eq!(
            prepared["preview"]["validation"]["execution_index"],
            "valid"
        );
        assert_eq!(prepared["preview"]["validation"]["history"], "validated");
        assert!(
            prepared["preview"]["diff"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str().is_some_and(|text| text.contains("GOAL-001")))
        );
        assert!(
            prepared["preview"]["lifecycle_impact"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["task_id"] == "TASK-001"
                    && row["before"] == "pending"
                    && row["after"] == "pending")
        );
        assert_eq!(prepared["request"]["plan"]["goals"][0]["id"], "GOAL-001");
        assert_eq!(
            prepared["request"]["plan"]["scope"][0]["goal_ids"],
            json!(["GOAL-001"])
        );
        assert_eq!(
            prepared["request"]["plan"]["acceptance_criteria"][0]["deliverable_ids"],
            json!(["DELIVERABLE-001"])
        );
        assert_eq!(
            prepared["request"]["task_items"]["TASK-001"]["commands"][0]["id"],
            "CMD-001"
        );
        assert_eq!(
            prepared["request"]["task_items"]["TASK-001"]["validations"][0]["command_ids"],
            json!(["CMD-001"])
        );
        assert_eq!(
            prepared["request"]["task_items"]["TASK-001"]["steps"][1]["references"],
            json!(["CMD-001", "VAL-001"])
        );
        let files = prepared["preview"]["transaction"]["files"]
            .as_array()
            .unwrap();
        assert!(
            files
                .iter()
                .any(|row| row["path"] == "outputs/work/plans/example.json")
        );
        assert!(
            files
                .iter()
                .any(|row| row["path"] == "outputs/work/tasks/example/index.json")
        );
        assert!(
            files
                .iter()
                .any(|row| row["path"] == "outputs/work/executions/example/index.json")
        );
    }

    #[test]
    fn untrusted_specification_sources_direct_to_migration() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/specification-update");
        let semantic = fs::read(fixture.join("semantic-request.json")).unwrap();
        for (case, include_item, corrupt_plan, corrupt_history) in [
            ("missing-item", false, false, false),
            ("invalid-plan", true, true, false),
            ("invalid-history", true, false, true),
        ] {
            let root = std::env::temp_dir().join(format!(
                "work-spec-untrusted-{case}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            for relative in [
                "outputs/work/plans/example.json",
                "outputs/work/tasks/example/index.json",
                "outputs/work/executions/example/index.json",
            ] {
                let destination = root.join(relative);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::copy(fixture.join(relative), destination).unwrap();
            }
            if include_item {
                let relative = "outputs/work/tasks/example/tasks/TASK-001.json";
                let destination = root.join(relative);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::copy(fixture.join(relative), destination).unwrap();
            }
            if corrupt_plan {
                fs::write(root.join("outputs/work/plans/example.json"), b"{").unwrap();
            }
            if corrupt_history {
                let attempt =
                    root.join("outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json");
                fs::create_dir_all(attempt.parent().unwrap()).unwrap();
                fs::write(attempt, b"{").unwrap();
            }
            let error = prepare_simple_update(
                &root,
                &repo.join("../skills/work"),
                &[],
                SpecificationPrepareInput {
                    raw: &semantic,
                    date: "2026-09-30",
                    output_file: None,
                },
            )
            .unwrap_err();
            assert_eq!(error.details["next_command"], "migration analyze", "{case}");
        }
    }

    #[test]
    fn semantic_dependency_and_defaults_rebuild_execution_binding() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-update/remove-task");
        let root = std::env::temp_dir().join(format!(
            "work-spec-dependencies-{}-{}",
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
            "outputs/work/tasks/example/tasks/TASK-002.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let semantic = serde_json::to_vec(&json!({
            "schema":"work-spec-prepare-request/v1",
            "requirement_id":"example",
            "reason":"Confirmed dependency and default revision",
            "edits":[
                {"target":{"artifact":"task_item","task_id":"TASK-002"},
                    "field":"dependencies","semantic_after":[]},
                {"target":{"artifact":"task_index"},"field":"execution_defaults",
                    "semantic_after":{"working_directory":".","os":"windows","shell":"pwsh"}}
            ]
        }))
        .unwrap();
        let prepared = prepare_simple_update(
            &root,
            &repo.join("../skills/work"),
            &[],
            SpecificationPrepareInput {
                raw: &semantic,
                date: "2026-09-30",
                output_file: None,
            },
        )
        .unwrap();
        assert_eq!(prepared["preview"]["status"], "valid");
        assert_eq!(
            prepared["request"]["task_items"]["TASK-002"]["dependencies"],
            json!([])
        );
        assert_eq!(
            prepared["request"]["task_index"]["execution_defaults"]["shell"],
            "pwsh"
        );
        assert!(
            prepared["preview"]["transaction"]["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["path"] == "outputs/work/executions/example/index.json")
        );
    }

    #[test]
    fn index_revision_preview_publication_and_verify_match_python() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/specification-update");
        let root = std::env::temp_dir().join(format!(
            "work-spec-update-parity-{}-{}",
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
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let request = fs::read(fixture.join("request.json")).unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let skill = repo.join("../skills/work");
        let semantic = fs::read(fixture.join("semantic-request.json")).unwrap();
        let expected_request: Value = serde_json::from_slice(&request).unwrap();
        let date = expected_request["task_index"]["changes"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["date"]
            .as_str()
            .unwrap();
        let prepared = prepare_simple_update(
            &root,
            &skill,
            &[],
            SpecificationPrepareInput {
                raw: &semantic,
                date,
                output_file: None,
            },
        )
        .unwrap();
        assert_eq!(
            prepared["output_file"],
            prepared["transport"]["output_file"]
        );
        assert_eq!(prepared["preview"]["status"], "valid");
        assert!(prepared["preview"]["candidate"].is_object());
        assert!(prepared["preview"]["transaction"].is_object());
        assert!(prepared["preview"].get("publication_status").is_none());
        let transport_path = root.join("prepared-spec-request.json");
        let prepared_file = prepare_simple_update(
            &root,
            &skill,
            &[],
            SpecificationPrepareInput {
                raw: &semantic,
                date,
                output_file: Some(&transport_path),
            },
        )
        .unwrap();
        assert_eq!(
            prepared_file["output_file"],
            prepared_file["transport"]["output_file"]
        );
        assert_eq!(fs::read(&transport_path).unwrap(), request);
        assert_eq!(prepared["request"], expected_request);
        assert_request_bytes(&prepared["request"], &request);
        assert_eq!(
            prepared["preview"]["approved_sha256"],
            expected["approved_sha256"]
        );
        let preview = update_from_project(
            &root,
            &skill,
            &[],
            SpecificationProjectRequest {
                raw: &request,
                operation: SpecOperation::Validate,
                approved_sha256: None,
            },
        )
        .unwrap();
        for field in [
            "record_id",
            "approved_sha256",
            "affected_task_ids",
            "changed_fields",
            "transaction",
        ] {
            assert_eq!(preview[field], expected[field], "field {field}");
        }
        let approval = expected["approved_sha256"].as_str().unwrap();
        let published = update_from_project(
            &root,
            &skill,
            &[],
            SpecificationProjectRequest {
                raw: &request,
                operation: SpecOperation::Apply,
                approved_sha256: Some(approval),
            },
        )
        .unwrap();
        assert_eq!(published["status"], "updated");
        let verification = verify_from_project(
            &root,
            &skill,
            &[],
            &serde_json::to_vec(&published["verification_request"]).unwrap(),
        )
        .unwrap();
        assert_eq!(verification["verified"], true);
        assert_eq!(verification["execution_authorized"], false);
        assert_eq!(
            verification["verification_scope"],
            "exact_specification_result"
        );
        let repeated = update_from_project(
            &root,
            &skill,
            &[],
            SpecificationProjectRequest {
                raw: &request,
                operation: SpecOperation::Recover,
                approved_sha256: Some(approval),
            },
        )
        .unwrap();
        assert_eq!(repeated["status"], "recovered");
        assert_eq!(repeated["publication_status"], "already_published");
    }

    #[test]
    fn item_and_plan_revisions_match_python_requests_and_transactions() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        for variant in ["item-goal", "plan-summary", "add-task", "remove-task"] {
            let fixture = repo
                .join("crates/work-infrastructure/fixtures/specification-update")
                .join(variant);
            let root = std::env::temp_dir().join(format!(
                "work-spec-item-parity-{}-{}",
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
            fs::write(root.join("src.txt"), b"source\n").unwrap();
            if variant == "remove-task" {
                let relative = "outputs/work/tasks/example/tasks/TASK-002.json";
                let destination = root.join(relative);
                fs::create_dir_all(destination.parent().unwrap()).unwrap();
                fs::copy(fixture.join(relative), destination).unwrap();
            }
            let request = fs::read(fixture.join("request.json")).unwrap();
            let expected_request: Value = serde_json::from_slice(&request).unwrap();
            let expected: Value =
                serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
            let semantic = fs::read(fixture.join("semantic-request.json")).unwrap();
            let date = expected_request["task_index"]["changes"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["date"]
                .as_str()
                .unwrap();
            let skill = repo.join("../skills/work");
            let prepared = prepare_simple_update(
                &root,
                &skill,
                &[],
                SpecificationPrepareInput {
                    raw: &semantic,
                    date,
                    output_file: None,
                },
            )
            .unwrap();
            assert_eq!(prepared["request"], expected_request);
            assert_request_bytes(&prepared["request"], &request);
            assert_eq!(
                prepared["preview"]["transaction"]["metadata"]["candidate_sha256"],
                expected["transaction"]["metadata"]["candidate_sha256"],
                "variant {variant}: candidate SHA"
            );
            for field in [
                "record_id",
                "approved_sha256",
                "affected_task_ids",
                "changed_fields",
                "transaction",
            ] {
                assert_eq!(
                    prepared["preview"][field], expected[field],
                    "variant {variant}: field {field}"
                );
            }
            if variant == "remove-task" {
                let approved = expected["approved_sha256"].as_str().unwrap();
                let publication = update_from_project(
                    &root,
                    &skill,
                    &[],
                    SpecificationProjectRequest {
                        raw: &serde_json::to_vec(&prepared["request"]).unwrap(),
                        operation: SpecOperation::Apply,
                        approved_sha256: Some(approved),
                    },
                )
                .unwrap();
                assert_eq!(publication["status"], "updated");
                assert!(
                    !root
                        .join("outputs/work/tasks/example/tasks/TASK-002.json")
                        .exists()
                );
                let verified = verify_from_project(
                    &root,
                    &skill,
                    &[],
                    &serde_json::to_vec(&publication["verification_request"]).unwrap(),
                )
                .unwrap();
                assert_eq!(verified["verified"], true);
            }
        }
    }

    #[test]
    fn item_revision_preserves_other_item_and_recovers_partial_publication() {
        use work_operations::derivation::snapshot::decode_snapshot;

        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-update/remove-task");
        let root = std::env::temp_dir().join(format!(
            "work-spec-item-recovery-{}-{}",
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
            "outputs/work/tasks/example/tasks/TASK-002.json",
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let other = root.join("outputs/work/tasks/example/tasks/TASK-002.json");
        let other_before = fs::read(&other).unwrap();
        let semantic = fs::read(
            repo.join("crates/work-infrastructure/fixtures/specification-update/item-goal/semantic-request.json"),
        )
        .unwrap();
        let skill = repo.join("../skills/work");
        let prepared = prepare_simple_update(
            &root,
            &skill,
            &[],
            SpecificationPrepareInput {
                raw: &semantic,
                date: "2026-09-01",
                output_file: None,
            },
        )
        .unwrap();
        assert_eq!(
            prepared["preview"]["changed_fields"],
            json!(["/task_items/TASK-001/goal"])
        );
        let item_files = prepared["preview"]["transaction"]["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| row["path"].as_str())
            .filter(|path| path.contains("/tasks/TASK-"))
            .collect::<Vec<_>>();
        assert_eq!(
            item_files,
            ["outputs/work/tasks/example/tasks/TASK-001.json"]
        );
        let request =
            render_task(&prepared["request"], TaskDocumentKind::SpecificationRequest).unwrap();
        let approval = prepared["preview"]["approved_sha256"].as_str().unwrap();
        let mut journal = prepared["preview"]["transaction"].clone();
        assert!(journal["files"].as_array().unwrap().len() > 1);
        let first = &journal["files"][0];
        let first_path = root.join(first["path"].as_str().unwrap());
        let first_bytes = decode_snapshot(&first["after"]).unwrap();
        fs::write(&first_path, first_bytes).unwrap();
        journal["published_count"] = json!(1);
        journal["state"] = json!("publishing");
        let relative = format!(
            "outputs/work/executions/example/.work-spec-update-{}.json",
            journal["transaction_id"].as_str().unwrap()
        );
        write_journal(&root, &relative, &journal).unwrap();
        let recovered = update_from_project(
            &root,
            &skill,
            &[],
            SpecificationProjectRequest {
                raw: &request,
                operation: SpecOperation::Recover,
                approved_sha256: Some(approval),
            },
        )
        .unwrap();
        assert_eq!(recovered["status"], "recovered");
        assert_eq!(recovered["publication_status"], "published");
        assert_eq!(fs::read(&other).unwrap(), other_before);
        let verified = verify_from_project(
            &root,
            &skill,
            &[],
            &serde_json::to_vec(&recovered["verification_request"]).unwrap(),
        )
        .unwrap();
        assert_eq!(verified["verified"], true);
    }
}
