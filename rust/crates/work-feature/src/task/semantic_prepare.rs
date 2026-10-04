//! Project-backed semantic TASK planning preparation.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use work_operations::canonical::parse_json_contract;
use work_operations::protocol::TASK_ID_PREFIX;
use work_operations::task::draft::{validate_draft_instruction_selection, validate_planning_index};

use crate::artifact_paths::ArtifactPathRepository;
use crate::error::{ExitCode, WorkError};
use crate::hierarchy::validate_task_paths;
use crate::instruction::{InstructionSourceRepository, load as load_instructions};
use crate::ports::SourceSnapshotReader;
use crate::skill::{SkillRoot, SkillSnapshotRepository};
use crate::task::draft::{
    TaskDraftHistoryRepository, prepare_list_update, resolve_instruction_selection,
};

pub trait SemanticTaskRepository: TaskDraftHistoryRepository + SourceSnapshotReader {
    fn require_initial_storage_free(&self, requirement_id: &str) -> Result<(), WorkError>;
    fn read_planning_index(&self, requirement_id: &str) -> Result<Value, WorkError>;
}

pub struct SemanticTaskRequest<'a> {
    pub requirement_id: &'a str,
    pub expected_revision: u64,
    pub semantic: &'a Value,
}

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::WorkflowState, reason, message, json!({}))
}

fn contract(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::Contract, reason, message, json!({}))
}

fn exact_keys(value: &Value, required: &[&str], optional: &[&str]) -> Result<(), WorkError> {
    let object = value
        .as_object()
        .ok_or_else(|| contract("expected_object", "A JSON object is required."))?;
    if required.iter().any(|key| !object.contains_key(*key))
        || object
            .keys()
            .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err(contract(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
        ));
    }
    Ok(())
}

fn task_reference(
    reference: &Value,
    old: &BTreeSet<String>,
    removed: &BTreeSet<String>,
    ids: &[String],
) -> Result<String, WorkError> {
    exact_keys(reference, &[], &["existing_task_id", "upsert_position"])?;
    let existing = reference["existing_task_id"].as_str();
    let position = reference["upsert_position"].as_u64();
    if existing.is_some() == position.is_some() {
        return Err(fail(
            "invalid_semantic_task_reference",
            "A TASK reference needs exactly one existing ID or upsert position.",
        ));
    }
    if let Some(id) = existing {
        if !old.contains(id) || removed.contains(id) {
            return Err(fail(
                "invalid_semantic_task_reference",
                "The referenced existing TASK is unavailable.",
            ));
        }
        return Ok(id.to_owned());
    }
    let position = position.expect("one reference kind");
    if position == 0 || position as usize > ids.len() {
        return Err(fail(
            "invalid_semantic_task_reference",
            "The upsert position is out of range.",
        ));
    }
    Ok(ids[position as usize - 1].clone())
}

pub fn prepare_semantic_task_request(
    repository: &impl SemanticTaskRepository,
    instructions: &impl InstructionSourceRepository,
    skills: &impl SkillSnapshotRepository,
    paths: &impl ArtifactPathRepository,
    roots: &[SkillRoot],
    request: SemanticTaskRequest<'_>,
) -> Result<Value, WorkError> {
    let SemanticTaskRequest {
        requirement_id,
        expected_revision,
        semantic,
    } = request;
    exact_keys(
        semantic,
        &["upsert", "remove_task_ids", "current_task", "reason"],
        &["source"],
    )?;
    if expected_revision == 0 {
        repository.require_initial_storage_free(requirement_id)?;
    }
    let previous = if expected_revision == 0 {
        None
    } else {
        Some(repository.read_planning_index(requirement_id)?)
    };
    if previous
        .as_ref()
        .is_some_and(|index| index["revision"].as_u64() != Some(expected_revision))
    {
        return Err(fail(
            "draft_revision_conflict",
            "Reload the current index before preparing a list change.",
        ));
    }
    let removed = semantic["remove_task_ids"].as_array().ok_or_else(|| {
        contract(
            "invalid_contract_value",
            "Removed TASK IDs must be an array.",
        )
    })?;
    let removed = removed
        .iter()
        .map(|id| {
            id.as_str().map(str::to_owned).ok_or_else(|| {
                contract("invalid_contract_value", "A removed TASK ID must be text.")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if previous.is_none() && (!removed.is_empty() || !semantic["reason"].is_null()) {
        return Err(fail(
            "invalid_semantic_initial_request",
            "Initial TASK planning cannot remove tasks or supply a list-change reason.",
        ));
    }
    let reason = semantic["reason"].as_str();
    if previous.is_some() && reason.is_none_or(|value| value.trim().is_empty()) {
        return Err(fail(
            "invalid_semantic_reason",
            "List changes require a non-empty reason.",
        ));
    }
    let old: BTreeMap<String, Value> = previous
        .as_ref()
        .map(|index| {
            index["tasks"]
                .as_array()
                .expect("validated tasks")
                .iter()
                .filter_map(|row| row["id"].as_str().map(|id| (id.to_owned(), row.clone())))
                .collect()
        })
        .unwrap_or_default();
    let removed_set: BTreeSet<String> = removed.iter().cloned().collect();
    if removed_set.len() != removed.len() || !removed_set.is_subset(&old.keys().cloned().collect())
    {
        return Err(fail(
            "invalid_removed_task_ids",
            "Remove each active TASK at most once.",
        ));
    }
    let upsert = semantic["upsert"]
        .as_array()
        .ok_or_else(|| contract("invalid_contract_value", "TASK upserts must be an array."))?;
    let mut highest = old
        .keys()
        .map(String::as_str)
        .chain(previous.as_ref().into_iter().flat_map(|index| {
            index["retired_task_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
        }))
        .filter_map(|id| {
            id.strip_prefix(TASK_ID_PREFIX)
                .and_then(|number| number.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0);
    let mut ids = Vec::new();
    let mut seen_existing = BTreeSet::new();
    for item in upsert {
        exact_keys(
            item,
            &["title", "goal", "scope", "skill_id", "dependencies"],
            &["existing_task_id", "instruction_selection"],
        )?;
        if let Some(id) = item["existing_task_id"].as_str() {
            if !old.contains_key(id)
                || removed_set.contains(id)
                || !seen_existing.insert(id.to_owned())
            {
                return Err(fail(
                    "invalid_semantic_existing_task",
                    "An upsert may identify one active, unremoved TASK once.",
                ));
            }
            ids.push(id.to_owned());
        } else {
            highest += 1;
            ids.push(format!("TASK-{highest:03}"));
        }
    }
    let old_ids = old.keys().cloned().collect::<BTreeSet<_>>();
    let mut boundaries = Vec::new();
    for (id, item) in ids.iter().zip(upsert) {
        let dependencies = item["dependencies"].as_array().ok_or_else(|| {
            contract(
                "invalid_contract_value",
                "TASK dependencies must be an array.",
            )
        })?;
        let mut resolved = dependencies
            .iter()
            .map(|reference| task_reference(reference, &old_ids, &removed_set, &ids))
            .collect::<Result<Vec<_>, _>>()?;
        if resolved.contains(id) {
            return Err(fail(
                "invalid_semantic_task_dependency",
                "A TASK cannot depend on itself.",
            ));
        }
        resolved.sort();
        resolved.dedup();
        let mut boundary = json!({"id":id,"title":item["title"],"goal":item["goal"],
            "scope":item["scope"],"skill_id":item["skill_id"],"dependencies":resolved});
        if !item["instruction_selection"].is_null() {
            boundary["instruction_selection"] = item["instruction_selection"].clone();
        } else if !old.contains_key(id) {
            return Err(fail(
                "draft_selection_required",
                "New TASKs require a confirmed instruction selection.",
            ));
        }
        boundaries.push(boundary);
    }
    let current = if semantic["current_task"].is_null() {
        Value::Null
    } else {
        json!(task_reference(
            &semantic["current_task"],
            &old_ids,
            &removed_set,
            &ids
        )?)
    };
    let source = semantic
        .get("source")
        .cloned()
        .or_else(|| previous.as_ref().map(|index| index["source"].clone()))
        .ok_or_else(|| {
            contract(
                "missing_planning_source",
                "Initial planning requires a complete fixed Source and confirmed Task choices.",
            )
        })?;
    let (_, snapshot) = crate::task::source::validate_context(
        repository,
        instructions,
        skills,
        paths,
        roots,
        requirement_id,
        &source,
    )?;
    if previous
        .as_ref()
        .is_some_and(|index| index["source"] != source)
    {
        return Err(fail(
            "draft_source_drift",
            "Review changed planning sources before list edits.",
        ));
    }
    let mut choices = if let Some(index) = &previous {
        let mut all = Vec::new();
        for row in index["tasks"].as_array().expect("validated tasks") {
            let id = row["id"].as_str().expect("validated ID");
            if removed_set.contains(id) {
                continue;
            }
            all.push(boundaries.iter().find(|boundary| boundary["id"] == id).cloned().unwrap_or_else(|| {
                let mut kept = json!({"id":row["id"],"title":row["title"],"goal":row["goal"],
                    "scope":row["scope"],"skill_id":row["skill_id"],"dependencies":row["dependencies"]});
                if row.get("instruction_selection").is_some() { kept["instruction_selection"] = row["instruction_selection"].clone(); }
                kept
            }));
        }
        all.extend(
            boundaries
                .iter()
                .filter(|boundary| !old.contains_key(boundary["id"].as_str().unwrap_or("")))
                .cloned(),
        );
        all
    } else {
        boundaries
    };
    let task_skills = source["skill_selection"]["skills"]
        .as_array()
        .expect("validated Task skills");
    let mut entries = Vec::new();
    for boundary in choices.drain(..) {
        let id = boundary["id"]
            .as_str()
            .ok_or_else(|| contract("invalid_contract_value", "A TASK ID is required."))?;
        let original = old.get(id);
        let explicit = boundary.get("instruction_selection");
        if let Some(value) = explicit {
            validate_draft_instruction_selection(value)
                .map_err(|issue| contract(issue.reason_code, issue.message))?;
        }
        let selected_paths = explicit
            .and_then(|value| value["selected_paths"].as_array())
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            });
        let references = explicit
            .and_then(|value| value["references"].as_array())
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            });
        let selected = resolve_instruction_selection(
            original.unwrap_or(&Value::Null),
            selected_paths.as_deref(),
            references.as_deref(),
        )?;
        if let Some(skill_id) = boundary["skill_id"].as_str() {
            if !task_skills.iter().any(|skill| {
                skill["id"] == skill_id && skill["mode_support"]["task"] != "unsupported"
            }) {
                return Err(fail(
                    "draft_skill_not_available",
                    "Use an independently confirmed Task-capable skill.",
                ));
            }
        }
        let paths = selected["selected_paths"]
            .as_array()
            .expect("validated selection")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let references = selected["references"]
            .as_array()
            .expect("validated selection")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        validate_task_paths(
            instructions,
            &paths,
            &source["hierarchy_selection"],
            "boundary.instruction_selection",
        )?;
        let hash =
            load_instructions(instructions, "task", &paths, &references)?.instructions_sha256;
        if original.is_some_and(|row| row["instructions_sha256"] != hash) {
            return Err(fail(
                "draft_instruction_drift",
                "Review instruction drift through source-update first.",
            ));
        }
        let mut entry = original
            .cloned()
            .unwrap_or_else(|| json!({"status":"planned","boundary_revision":1}));
        for field in ["id", "title", "goal", "scope", "skill_id", "dependencies"] {
            entry[field] = boundary[field].clone();
        }
        entry["instructions_sha256"] = json!(hash);
        if original.is_none()
            || original.is_some_and(|row| row.get("instruction_selection").is_some())
        {
            entry["instruction_selection"] = selected;
        }
        entries.push(entry);
    }
    let mut index = previous.clone().unwrap_or_else(
        || json!({"schema":"work-task-planning-index","requirement_id":requirement_id}),
    );
    index["revision"] = json!(expected_revision + 1);
    index["source"] = source.clone();
    index["current_task_id"] = current;
    index["tasks"] = json!(entries);
    validate_planning_index(&index).map_err(|issue| contract(issue.reason_code, issue.message))?;
    let (_, fresh) = crate::task::source::validate_context(
        repository,
        instructions,
        skills,
        paths,
        roots,
        requirement_id,
        &source,
    )?;
    if fresh != snapshot {
        return Err(fail(
            "draft_source_drift",
            "Planning sources changed during preparation.",
        ));
    }
    if let Some(previous) = &previous {
        if repository.read_planning_index(requirement_id)? != *previous {
            return Err(fail(
                "draft_revision_conflict",
                "The planning index changed during preparation.",
            ));
        }
        let _: work_model::task::request::SemanticTaskRequest =
            serde_json::from_value(semantic.clone()).map_err(|_| {
                contract(
                    "invalid_contract_value",
                    "The semantic TASK request is invalid.",
                )
            })?;
        let prepared = prepare_list_update(
            repository,
            previous,
            &index,
            expected_revision,
            reason.expect("existing reason"),
        )?;
        let drafts = prepared
            .files
            .iter()
            .filter(|(path, _)| path.starts_with(TASK_ID_PREFIX) && path.ends_with(".json"))
            .map(|(path, raw)| {
                Ok((
                    path.trim_end_matches(".json").to_owned(),
                    parse_json_contract(raw).map_err(|_| {
                        contract("invalid_json_contract", "A prepared TASK draft is invalid.")
                    })?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>, WorkError>>()?;
        Ok(work_model::task::response::typed_response::<
            work_model::task::response::TaskDraftPrepare,
        >(
            json!({"schema":"work-task-draft-prepare","status":"prepared",
            "request":{"index":index,"reason":reason},"index":prepared.index,
            "affected_task_ids":prepared.affected_task_ids,"drafts":drafts}),
        ))
    } else {
        let _: work_model::task::request::SemanticTaskRequest =
            serde_json::from_value(semantic.clone()).map_err(|_| {
                contract(
                    "invalid_contract_value",
                    "The semantic TASK request is invalid.",
                )
            })?;
        Ok(work_model::task::response::typed_response::<
            work_model::task::response::TaskDraftPrepare,
        >(
            json!({"schema":"work-task-draft-prepare","status":"prepared",
            "request":index,"index":index,"affected_task_ids":ids,"drafts":{}}),
        ))
    }
}
