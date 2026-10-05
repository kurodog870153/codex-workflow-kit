//! Complete formal preview from a fixed committed discussion revision.

use crate::artifact_paths::ArtifactPathRepository;
use crate::error::{ExitCode, WorkError};
use crate::instruction::{InstructionSourceRepository, load, select, task_document_selection};
use crate::ports::SourceSnapshotReader;
use crate::skill::{SkillRoot, SkillSnapshotRepository};
use crate::task::create::{PreparedTaskCreate, TaskCreateInput, prepare_task_create};
use serde_json::{Value, json};
use work_model::common::Nullable;
use work_model::discussion::DiscussionSession;
use work_operations::derivation::fingerprint;
use work_operations::discussion;
use work_operations::task::ordering::{TaskDocumentKind, render_task};

pub struct PreparedDiscussion {
    pub prepared: PreparedTaskCreate,
    pub preview: Value,
    pub task_path: String,
    pub execution_dir: String,
    pub approval_sha256: String,
}

pub struct PublicationRequest<'a> {
    pub requirement_id: &'a str,
    pub expected_revision: u64,
    pub session_sha256: &'a str,
    pub metadata: &'a Value,
    pub approved_sha256: &'a str,
    pub publication_evidence: &'a str,
    pub recovery: bool,
}

pub trait DiscussionPublicationRepository {
    fn preview(&self, requirement: &str, metadata: &Value) -> Result<Value, WorkError>;
    fn publish(&self, request: PublicationRequest<'_>) -> Result<Value, WorkError>;
}

pub fn error(reason: &str) -> WorkError {
    WorkError::new(
        ExitCode::ArtifactIntegrity,
        reason,
        "The discussion preview is not verified for publication. Return to the same Session to resolve the gap.",
        json!({"published":false}),
    )
}

pub fn prepare<H, S, P>(
    instructions: &H,
    skills: &S,
    paths: &P,
    roots: &[SkillRoot],
    session: &DiscussionSession,
    metadata: &Value,
) -> Result<PreparedDiscussion, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
{
    discussion::ready_to_generate(session).map_err(|e| error(e.0))?;
    let metadata = metadata
        .as_object()
        .ok_or_else(|| error("invalid_discussion_metadata"))?;
    if !metadata.contains_key("title")
        || !metadata.contains_key("summary")
        || metadata
            .keys()
            .any(|key| !["title", "summary", "execution_defaults"].contains(&key.as_str()))
    {
        return Err(error("invalid_discussion_metadata"));
    }
    let Nullable::Value(source) = &session.context.confirmed_source else {
        unreachable!("checked source")
    };
    if session.context.acceptance_criteria != source.acceptance_criteria {
        return Err(error("discussion_acceptance_source_mismatch"));
    }
    let source_value = serde_json::to_value(source).expect("source serializes");
    let (_, snapshot) = crate::task::source::validate_context(
        paths,
        instructions,
        skills,
        paths,
        roots,
        &session.requirement_id,
        &source_value,
    )?;
    let mut records = discussion::assembly::task_records(session).map_err(|e| error(e.0))?;
    let mut sources = Vec::new();
    for record in &mut records {
        let task = session
            .tasks
            .iter()
            .find(|t| record["id"] == t.id)
            .expect("derived task");
        let Nullable::Value(selection) = &task.instruction_selection else {
            unreachable!("checked selection")
        };
        let loaded = load(
            instructions,
            "task",
            &selection.selected_paths,
            &selection.references,
        )?;
        let selected = select(
            instructions,
            "task",
            &selection.selected_paths,
            &selection.references,
        )?;
        let selected = serde_json::to_value(selected).expect("selection serializes");
        if selected["instructions_sha256"]
            != serde_json::to_value(&task.instructions_sha256).expect("hash serializes")
        {
            return Err(error("discussion_instruction_drift"));
        }
        sources.push(loaded);
        record["instruction_selection"] = selected;
    }
    let mut contract = Value::Object(metadata.clone());
    contract["schema"] = json!("work-task-collection-projection");
    contract["requirement_id"] = json!(session.requirement_id);
    contract["spec_id"] = json!("TASK-SPEC-001");
    contract["status"] = json!("confirmed");
    for name in [
        "artifacts",
        "hierarchy_selection",
        "skill_selection",
        "acceptance_criteria",
    ] {
        contract[name] = source_value[name].clone();
    }
    contract["source"] = json!({"kind":"snapshot","manifest":source.snapshot});
    contract["instruction_selection"] = task_document_selection(&sources)?;
    contract["discussion"] = json!(discussion::assembly::trace(session));
    contract["tasks"] = json!(records);
    contract["readiness"] = json!({"status":"passed","spec_id":"TASK-SPEC-001"});
    let raw = render_task(&contract, TaskDocumentKind::Collection)
        .map_err(|_| error("invalid_contract_value"))?;
    let prepared = prepare_task_create(
        instructions,
        skills,
        paths,
        roots,
        TaskCreateInput {
            raw: &raw,
            index_path: &source.artifacts.task,
            source_root: &source.artifacts.source,
            execution_dir: &source.artifacts.execution,
        },
    )?;
    crate::task::validate_collection_with_file_state(
        instructions,
        skills,
        paths,
        roots,
        crate::task::CollectionInput {
            index_raw: &prepared.index_raw,
            item_raw: &prepared.items,
            index_path: &source.artifacts.task,
        },
        true,
    )?;
    let (_, current) = crate::task::source::validate_context(
        paths,
        instructions,
        skills,
        paths,
        roots,
        &session.requirement_id,
        &source_value,
    )?;
    if current != snapshot {
        return Err(error("source_snapshot_mismatch"));
    }
    let approval =
        fingerprint::discussion_approval(&session.commit.content_sha256, &prepared.approval_bytes);
    let mut targets = serde_json::Map::new();
    targets.insert(
        source.artifacts.task.clone(),
        json!(String::from_utf8(prepared.index_raw.clone()).expect("JSON UTF-8")),
    );
    let parent = source
        .artifacts
        .task
        .rsplit_once('/')
        .expect("validated path")
        .0;
    for (id, bytes) in &prepared.items {
        targets.insert(
            format!("{parent}/tasks/{id}.json"),
            json!(String::from_utf8(bytes.clone()).expect("JSON UTF-8")),
        );
    }
    targets.insert(
        format!("{}/index.json", source.artifacts.execution),
        json!(String::from_utf8(prepared.execution_raw.clone()).expect("JSON UTF-8")),
    );
    let preview = json!({"requirement_id":session.requirement_id,"revision":session.revision,
        "session_sha256":session.commit.content_sha256,"source_sha256":prepared.validation["source_sha256"],
        "approval_sha256":approval,"task_collection_sha256":prepared.validation["task_collection_sha256"],
        "contract":contract,"targets":targets,"execution_index":prepared.initial_execution});
    Ok(PreparedDiscussion {
        prepared,
        preview,
        task_path: source.artifacts.task.clone(),
        execution_dir: source.artifacts.execution.clone(),
        approval_sha256: approval,
    })
}
