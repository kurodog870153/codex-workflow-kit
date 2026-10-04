//! Independent formal Task validation over immutable Source and instruction ports.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Value, json};
use work_model::task::index::{TaskIndex, TaskItemReference};
use work_model::task::item::TaskItem;
use work_operations::canonical::{JsonContractIssue, parse_json_contract, portable_path_identity};
use work_operations::derivation::fingerprint;
use work_operations::task::TaskIssue;
use work_operations::task::collection::{require_item_set, semantic_projection};
use work_operations::task::index::validate_task_index;
use work_operations::task::item::validate_task_item;
use work_operations::task::semantic::{validate_task_file_state, validate_task_semantics};

use crate::artifact_paths::ArtifactPathRepository;
use crate::error::{ExitCode, WorkError};
use crate::hierarchy::validate_task_paths;
use crate::instruction::{
    InstructionSourceRepository, task_document_selection, validate_selection_value,
    validate_task_document_selection,
};
use crate::ports::SourceSnapshotReader;
use crate::skill::{SkillRoot, SkillSnapshotRepository};

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn domain(issue: TaskIssue) -> WorkError {
    error(
        ExitCode::Contract,
        issue.reason_code,
        issue.message,
        issue.details,
    )
}

fn parse(raw: &[u8], source: &str) -> Result<Value, WorkError> {
    parse_json_contract(raw).map_err(|issue| match issue {
        JsonContractIssue::InvalidUtf8(offset) => error(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source": source, "byte_offset": offset}),
        ),
        JsonContractIssue::MultipleBom => error(
            ExitCode::InputFormat,
            "input_file_multiple_bom",
            "The request file may contain at most one leading UTF-8 BOM.",
            json!({"source": source}),
        ),
        JsonContractIssue::DuplicateKey(key) => error(
            ExitCode::Contract,
            "duplicate_json_key",
            "Duplicate keys are ambiguous; no parsed document will be used.",
            json!({"keys": [key]}),
        ),
        JsonContractIssue::InvalidConstant(value) => error(
            ExitCode::InputFormat,
            "invalid_json_constant",
            "The JSON contract contains a non-standard numeric constant.",
            json!({"value": value}),
        ),
        JsonContractIssue::InvalidJson { line, column } => error(
            ExitCode::Contract,
            "invalid_json_contract",
            "The TASK JSON is invalid.",
            json!({"line": line, "column": column}),
        ),
        JsonContractIssue::NotObject => error(
            ExitCode::Contract,
            "json_contract_not_object",
            "The TASK document must be a JSON object.",
            json!({}),
        ),
    })
}

pub struct CollectionInput<'a> {
    pub index_raw: &'a [u8],
    pub item_raw: &'a BTreeMap<String, Vec<u8>>,
    pub index_path: &'a str,
}

pub trait TaskCollectionRepository {
    fn read_task_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError>;
    fn item_names(&self, index_path: &str) -> Result<Vec<String>, WorkError>;
}

struct CachedTaskCollection<'a, R> {
    repository: &'a R,
    raw: RefCell<HashMap<String, Vec<u8>>>,
}

impl<R: TaskCollectionRepository> TaskCollectionRepository for CachedTaskCollection<'_, R> {
    fn read_task_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        if let Some(raw) = self.raw.borrow().get(relative_path) {
            return Ok(raw.clone());
        }
        let raw = self.repository.read_task_file(relative_path)?;
        self.raw
            .borrow_mut()
            .insert(relative_path.to_owned(), raw.clone());
        Ok(raw)
    }

    fn item_names(&self, index_path: &str) -> Result<Vec<String>, WorkError> {
        self.repository.item_names(index_path)
    }
}

pub struct ExecutionTaskContext {
    pub contract: Value,
    pub validation: Value,
    pub sources: HashMap<String, Vec<u8>>,
    pub index: Value,
}

pub fn recheck_task_execution_context<P, R>(
    paths: &P,
    repository: &R,
    context: &ExecutionTaskContext,
) -> Result<(), WorkError>
where
    P: ArtifactPathRepository + SourceSnapshotReader,
    R: TaskCollectionRepository,
{
    for (path, original) in &context.sources {
        if repository.read_task_file(path)? != *original {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "execute_worktree_task_changed",
                "The formal TASK changed after preflight.",
                json!({}),
            ));
        }
    }
    let provenance = serde_json::from_value(context.index["source"].clone()).map_err(|_| {
        error(
            ExitCode::Contract,
            "invalid_task_source",
            "Task provenance is invalid.",
            json!({}),
        )
    })?;
    let artifacts = serde_json::from_value(context.index["artifacts"].clone()).map_err(|_| {
        error(
            ExitCode::Contract,
            "invalid_artifact_path",
            "Task artifact roots are invalid.",
            json!({}),
        )
    })?;
    let source_sha = source::verify_provenance(
        paths,
        context.index["requirement_id"]
            .as_str()
            .expect("validated requirement"),
        &provenance,
        &artifacts,
    )?;
    if source_sha != context.validation["source_sha256"] {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "source_snapshot_mismatch",
            "Task provenance changed after preflight.",
            json!({}),
        ));
    }

    Ok(())
}

pub fn load_task_execution_context<H, S, P, R>(
    instructions: &H,
    skills: &S,
    paths: &P,
    repository: &R,
    skill_roots: &[SkillRoot],
    index_path: &str,
    task_id: &str,
) -> Result<ExecutionTaskContext, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    R: TaskCollectionRepository,
{
    let cached = CachedTaskCollection {
        repository,
        raw: RefCell::new(HashMap::new()),
    };
    let validated = load_collection_with_file_state(
        instructions,
        skills,
        paths,
        &cached,
        skill_roots,
        index_path,
        true,
    )?;
    let index_raw = cached.read_task_file(index_path)?;
    if fingerprint::raw(&index_raw) != validated["task_index_sha256"] {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "task_execution_source_changed",
            "The formal TASK index changed while loading execution context.",
            json!({}),
        ));
    }
    let index = parse(&index_raw, index_path)?;
    let references = index["tasks"].as_array().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "invalid_item_array",
            "TASK index references must be an array.",
            json!({}),
        )
    })?;
    if !references.iter().any(|row| row["id"] == task_id) {
        return Err(error(
            ExitCode::Contract,
            "unknown_task_id",
            "The selected TASK does not exist.",
            json!({"task_id":task_id}),
        ));
    }
    let directory = index_path
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    let mut sources = HashMap::from([(index_path.to_owned(), index_raw)]);
    let items: HashMap<String, Value> = validated["collection_contract"]["tasks"]
        .as_array()
        .expect("validated TASK projection")
        .iter()
        .map(|item| {
            (
                item["id"].as_str().expect("validated TASK ID").to_owned(),
                item.clone(),
            )
        })
        .collect();
    for reference in references {
        let id = reference["id"].as_str().ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_task_item",
                "The TASK reference needs an ID.",
                json!({}),
            )
        })?;
        let relative = reference["path"].as_str().ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_task_item",
                "The TASK reference needs a path.",
                json!({}),
            )
        })?;
        let path = format!("{directory}/{relative}");
        let raw = cached.read_task_file(&path)?;
        if fingerprint::raw(&raw) != validated["task_item_sha256"][id] {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "task_execution_source_changed",
                "A formal TASK item changed while loading execution context.",
                json!({"task_id":id}),
            ));
        }
        sources.insert(path, raw);
    }
    let mut closure = HashSet::new();
    let mut pending = vec![task_id.to_owned()];
    while let Some(current) = pending.pop() {
        if !closure.insert(current.clone()) {
            continue;
        }
        let item = items.get(&current).ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_task_dependency",
                "A TASK dependency is unknown.",
                json!({"task_id":current}),
            )
        })?;
        for dependency in item["dependencies"].as_array().into_iter().flatten() {
            let dependency = dependency.as_str().ok_or_else(|| {
                error(
                    ExitCode::Contract,
                    "invalid_task_dependency",
                    "A TASK dependency is unknown.",
                    json!({"task_id":current}),
                )
            })?;
            pending.push(dependency.to_owned());
        }
    }
    let mut contract = index.clone();
    let object = contract.as_object_mut().expect("validated TASK index");
    object.remove("changes");
    object.insert("schema".into(), json!("work-task-execution-view"));
    object.insert(
        "tasks".into(),
        Value::Array(
            references
                .iter()
                .filter(|reference| {
                    reference["id"]
                        .as_str()
                        .is_some_and(|id| closure.contains(id))
                })
                .map(|reference| {
                    let id = reference["id"].as_str().expect("validated TASK reference");
                    items[id].clone()
                })
                .collect(),
        ),
    );
    let validation = json!({"schema":"work-task-execution-validation",
        "requirement_id":validated["requirement_id"],"spec_id":validated["spec_id"],
        "task_ids":validated["task_ids"],
        "task_collection_sha256":validated["task_collection_sha256"],
        "source_sha256":validated["source_sha256"],
        "task_index_sha256":validated["task_index_sha256"],
        "task_item_sha256":validated["task_item_sha256"],
        "instructions_sha256":validated["instructions_sha256"],
        "task_instructions_sha256":validated["task_instructions_sha256"],
        "task_skill_ids":validated["task_skill_ids"],
        "hierarchy_selection_sha256":validated["hierarchy_selection_sha256"]});
    Ok(ExecutionTaskContext {
        contract,
        validation,
        sources,
        index,
    })
}

pub fn load_collection<H, S, P, R>(
    instructions: &H,
    skills: &S,
    paths: &P,
    repository: &R,
    skill_roots: &[SkillRoot],
    index_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    R: TaskCollectionRepository,
{
    load_collection_with_file_state(
        instructions,
        skills,
        paths,
        repository,
        skill_roots,
        index_path,
        false,
    )
}

pub fn load_collection_with_file_state<H, S, P, R>(
    instructions: &H,
    skills: &S,
    paths: &P,
    repository: &R,
    skill_roots: &[SkillRoot],
    index_path: &str,
    validate_file_state: bool,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    R: TaskCollectionRepository,
{
    load_collection_with_policy(
        instructions,
        skills,
        paths,
        repository,
        skill_roots,
        index_path,
        CollectionPolicy {
            file_state: validate_file_state,
            revision_baseline: false,
        },
    )
}

/// Validate a current, byte-bound baseline for reviewed Revise only. This never
/// authorizes Task execution and does not accept historical schema or fields.
pub fn load_revision_baseline<H, S, P, R>(
    instructions: &H,
    skills: &S,
    paths: &P,
    repository: &R,
    skill_roots: &[SkillRoot],
    index_path: &str,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    R: TaskCollectionRepository,
{
    load_collection_with_policy(
        instructions,
        skills,
        paths,
        repository,
        skill_roots,
        index_path,
        CollectionPolicy {
            file_state: true,
            revision_baseline: true,
        },
    )
}

#[derive(Clone, Copy)]
struct CollectionPolicy {
    file_state: bool,
    revision_baseline: bool,
}

fn load_collection_with_policy<H, S, P, R>(
    instructions: &H,
    skills: &S,
    paths: &P,
    repository: &R,
    skill_roots: &[SkillRoot],
    index_path: &str,
    policy: CollectionPolicy,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    R: TaskCollectionRepository,
{
    let index_raw = repository.read_task_file(index_path)?;
    let index = parse(&index_raw, index_path)?;
    let references = index["tasks"].as_array().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "invalid_item_array",
            "TASK index references must be an array.",
            json!({}),
        )
    })?;
    let mut items = BTreeMap::new();
    let mut expected = std::collections::BTreeSet::new();
    let directory = index_path
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    for reference in references {
        let task_id = reference["id"].as_str().ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_task_item",
                "The TASK reference needs an ID.",
                json!({}),
            )
        })?;
        let relative = reference["path"].as_str().ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_task_item",
                "The TASK reference needs a path.",
                json!({}),
            )
        })?;
        if relative != format!("tasks/{task_id}.json") {
            return Err(error(
                ExitCode::Contract,
                "task_item_path_mismatch",
                "A TASK item path must exactly match tasks/<TASK-ID>.json.",
                json!({"task_id": task_id}),
            ));
        }
        items.insert(
            task_id.into(),
            repository.read_task_file(&format!("{directory}/{relative}"))?,
        );
        expected.insert(format!("{task_id}.json"));
    }
    let observed: std::collections::BTreeSet<_> =
        repository.item_names(index_path)?.into_iter().collect();
    if observed != expected {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "task_collection_directory_mismatch",
            "The TASK item directory does not exactly match the formal index.",
            json!({"missing": expected.difference(&observed).collect::<Vec<_>>(), "orphan": observed.difference(&expected).collect::<Vec<_>>()}),
        ));
    }
    validate_collection_with_policy(
        instructions,
        skills,
        paths,
        skill_roots,
        CollectionInput {
            index_raw: &index_raw,
            item_raw: &items,
            index_path,
        },
        policy,
    )
}

pub fn validate_collection<H, S, P>(
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    input: CollectionInput<'_>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
{
    validate_collection_with_file_state(instructions, skills, paths, skill_roots, input, false)
}

pub fn validate_collection_with_file_state<H, S, P>(
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    input: CollectionInput<'_>,
    validate_file_state: bool,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
{
    validate_collection_with_policy(
        instructions,
        skills,
        paths,
        skill_roots,
        input,
        CollectionPolicy {
            file_state: validate_file_state,
            revision_baseline: false,
        },
    )
}

pub fn validate_revision_baseline<H, S, P>(
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    input: CollectionInput<'_>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
{
    validate_collection_with_policy(
        instructions,
        skills,
        paths,
        skill_roots,
        input,
        CollectionPolicy {
            file_state: false,
            revision_baseline: true,
        },
    )
}

fn validate_collection_with_policy<H, S, P>(
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    input: CollectionInput<'_>,
    policy: CollectionPolicy,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
{
    let index = parse(input.index_raw, input.index_path)?;
    let index_validation =
        validate_task_index(&index, input.index_raw, input.index_path).map_err(domain)?;
    let task_ids: Vec<String> = index_validation["task_ids"]
        .as_array()
        .expect("validated task IDs")
        .iter()
        .map(|id| id.as_str().expect("validated ID").to_owned())
        .collect();
    require_item_set(input.item_raw, &task_ids).map_err(|issue| {
        error(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let provenance = serde_json::from_value(index["source"].clone()).map_err(|_| {
        error(
            ExitCode::Contract,
            "invalid_task_source",
            "A Task source is required.",
            json!({}),
        )
    })?;
    let artifacts = serde_json::from_value(index["artifacts"].clone()).map_err(|_| {
        error(
            ExitCode::Contract,
            "invalid_artifact_path",
            "Task artifact roots are required.",
            json!({}),
        )
    })?;
    let source_sha = source::verify_provenance(
        paths,
        index["requirement_id"]
            .as_str()
            .expect("validated requirement"),
        &provenance,
        &artifacts,
    )?;
    crate::hierarchy::validate_selection(instructions, &index["hierarchy_selection"])?;
    crate::skill::validate_selection(skills, skill_roots, &index["skill_selection"])?;
    let mut item_validations: BTreeMap<String, Value> = BTreeMap::new();
    let mut items = Vec::new();
    let mut source_sets = Vec::new();
    let mut stored_selections = Vec::new();
    for reference in index["tasks"].as_array().expect("validated references") {
        let task_id = reference["id"].as_str().expect("validated ID");
        let raw = &input.item_raw[task_id];
        let item = parse(raw, task_id)?;
        let validation = validate_task_item(&item, raw, task_id).map_err(domain)?;
        if validation["task_item_sha256"] != reference["canonical_sha256"] {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "task_item_fingerprint_mismatch",
                "A TASK item fingerprint does not match the formal index.",
                json!({"task_id": task_id, "expected": reference["canonical_sha256"], "actual": validation["task_item_sha256"]}),
            ));
        }
        let selection = &item["instruction_selection"];
        let selected_paths: Vec<String> = selection["selected_paths"]
            .as_array()
            .ok_or_else(|| {
                error(
                    ExitCode::Contract,
                    "invalid_string_array",
                    "TASK selected paths must be an array.",
                    json!({"task_id": task_id}),
                )
            })?
            .iter()
            .map(|path| path.as_str().unwrap_or("").to_owned())
            .collect();
        validate_task_paths(
            instructions,
            &selected_paths,
            &index["hierarchy_selection"],
            &format!("{task_id}.instruction_selection.selected_paths"),
        )?;
        if policy.revision_baseline {
            stored_selections.push(crate::instruction::stored_selection(
                selection,
                Some(&selected_paths),
            )?);
        } else {
            source_sets.push(validate_selection_value(
                instructions,
                "task",
                selection,
                &format!("{task_id}.instruction_selection"),
            )?);
        }
        item_validations.insert(task_id.into(), validation);
        items.push(
            serde_json::from_value::<TaskItem>(item).expect("validated TASK item matches Model"),
        );
    }
    let document_selection = if policy.revision_baseline {
        crate::instruction::stored_document_selection(
            &index["instruction_selection"],
            &stored_selections,
        )?
    } else {
        let current = task_document_selection(&source_sets)?;
        validate_task_document_selection(&index["instruction_selection"], &current)?;
        current
    };
    let typed_index: TaskIndex =
        serde_json::from_value(index.clone()).expect("validated TASK index matches Model");
    let projection =
        serde_json::to_value(semantic_projection(typed_index, items, input.index_path))
            .expect("TASK projection serializes");
    let semantics = validate_task_semantics(&projection).map_err(domain)?;
    if policy.file_state {
        let mut existence = BTreeMap::new();
        for task in projection["tasks"].as_array().expect("validated TASKs") {
            for file in task["files"].as_array().into_iter().flatten() {
                for field in ["path", "source", "destination"] {
                    if let Some(path) = file[field].as_str() {
                        let identity = portable_path_identity(path);
                        let exists = paths.exists(path)?;
                        existence
                            .entry(identity)
                            .and_modify(|prior| *prior |= exists)
                            .or_insert(exists);
                    }
                }
            }
        }
        let order = semantics["task_order"]
            .as_array()
            .expect("task order")
            .iter()
            .map(|value| value.as_str().expect("TASK ID").to_owned())
            .collect::<Vec<_>>();
        validate_task_file_state(&projection, &order, &existence).map_err(domain)?;
    }
    let references = index["tasks"]
        .as_array()
        .expect("validated references")
        .iter()
        .map(|reference| {
            serde_json::from_value::<TaskItemReference>(reference.clone())
                .expect("validated TASK reference")
        })
        .collect::<Vec<_>>();
    let collection_sha = fingerprint::task_collection(
        index_validation["task_index_sha256"]
            .as_str()
            .expect("validated hash"),
        &references,
    );
    let mut item_hashes = serde_json::Map::new();
    for task_id in &task_ids {
        item_hashes.insert(
            task_id.clone(),
            item_validations[task_id]["task_item_sha256"].clone(),
        );
    }
    Ok(work_model::task::response::typed_response::<
        work_model::task::response::TaskCollectionValidation,
    >(json!({
        "schema": "work-task-collection-validation", "requirement_id": index["requirement_id"], "spec_id": index["spec_id"],
        "task_ids": task_ids, "task_count": task_ids.len(), "task_index_sha256": fingerprint::raw(input.index_raw),
        "task_item_sha256": item_hashes, "task_collection_sha256": collection_sha, "source_sha256": source_sha,
        "instructions_sha256": document_selection["instructions_sha256"], "task_instructions_sha256": semantics["task_instruction_hashes"],
        "task_skill_ids": semantics["task_skill_ids"], "hierarchy_selection_sha256": index["hierarchy_selection"]["selection_sha256"],
        "skill_selection_sha256": index["skill_selection"]["selection_sha256"], "collection_contract": projection,
    })))
}

pub mod assembly;
pub mod create;
pub mod draft;
pub mod semantic_prepare;
pub mod source;
