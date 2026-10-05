//! In-memory preparation of exact formal TASK collection creation targets.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use work_operations::canonical::{JsonContractIssue, parse_json_contract};
use work_operations::derivation::fingerprint;
use work_operations::execution::index::{
    build_initial_execution_index, render_execution_index, validate_execution_index,
};
use work_operations::task::create::prepare_collection;

use crate::artifact_paths::ArtifactPathRepository;
use crate::error::{ExitCode, WorkError};
use crate::instruction::InstructionSourceRepository;
use crate::ports::SourceSnapshotReader;
use crate::skill::{SkillRoot, SkillSnapshotRepository};
use crate::task::{CollectionInput, validate_collection};

pub struct PreparedTaskCreate {
    pub index_raw: Vec<u8>,
    pub items: BTreeMap<String, Vec<u8>>,
    pub execution_raw: Vec<u8>,
    pub approval_bytes: Vec<u8>,
    pub validation: Value,
    pub initial_execution: Value,
}

pub struct TaskCreateInput<'a> {
    pub raw: &'a [u8],
    pub index_path: &'a str,
    pub source_root: &'a str,
    pub execution_dir: &'a str,
}

pub struct TaskCreateProjectInput<'a> {
    pub raw: &'a [u8],
    pub source_root: &'a str,
    pub task_path: &'a str,
    pub execution_dir: &'a str,
    pub recovery: bool,
}

pub trait TaskCreationRepository {
    fn publish(
        &self,
        task_path: &str,
        execution_dir: &str,
        prepared: &PreparedTaskCreate,
        recovery: bool,
    ) -> Result<bool, WorkError>;
    fn read_execution_index(&self, execution_dir: &str) -> Result<Vec<u8>, WorkError>;
}

fn contract(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::Contract, reason, message, json!({}))
}

pub fn parse_create_input(raw: &[u8]) -> Result<Value, WorkError> {
    let projection = parse_json_contract(raw).map_err(|issue| match issue {
        JsonContractIssue::InvalidConstant(value) => WorkError::new(
            ExitCode::InputFormat,
            "invalid_json_constant",
            "The JSON contract contains a non-standard numeric constant.",
            json!({"value": value}),
        ),
        other => {
            let reason = match other {
                JsonContractIssue::DuplicateKey(_) => "duplicate_json_key",
                JsonContractIssue::InvalidUtf8(_) => "invalid_utf8",
                _ => "invalid_json_contract",
            };
            contract(reason, "TASK create input is not valid JSON.")
        }
    })?;
    if projection["schema"] != "work-task-collection-projection" {
        return Err(contract(
            "task_create_schema",
            "TASK create input must be a complete work-task-collection-projection contract.",
        ));
    }
    let object = projection
        .as_object()
        .expect("schema check requires an object");
    let required = [
        "requirement_id",
        "spec_id",
        "status",
        "title",
        "summary",
        "artifacts",
        "source",
        "hierarchy_selection",
        "skill_selection",
        "acceptance_criteria",
        "instruction_selection",
        "tasks",
        "readiness",
    ];
    let allowed = [
        "schema",
        "requirement_id",
        "spec_id",
        "status",
        "title",
        "summary",
        "artifacts",
        "source",
        "hierarchy_selection",
        "skill_selection",
        "acceptance_criteria",
        "instruction_selection",
        "execution_defaults",
        "discussion",
        "decisions",
        "tasks",
        "changes",
        "readiness",
    ];
    let mut missing = required
        .iter()
        .filter(|field| !object.contains_key(**field))
        .map(|field| (*field).to_owned())
        .collect::<Vec<_>>();
    let mut unknown = object
        .keys()
        .filter(|field| !allowed.contains(&field.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() || !unknown.is_empty() {
        missing.sort();
        unknown.sort();
        let mut issues = missing
            .iter()
            .map(|field| json!({"location":field,"validation_type":"missing"}))
            .chain(
                unknown
                    .iter()
                    .map(|field| json!({"location":field,"validation_type":"extra_forbidden"})),
            )
            .collect::<Vec<_>>();
        issues.sort_by(|left, right| left["location"].as_str().cmp(&right["location"].as_str()));
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location":"contract","missing":missing,"unknown":unknown,"issues":issues}),
        ));
    }
    Ok(projection)
}

pub fn prepare_task_create<H, S, P>(
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    input: TaskCreateInput<'_>,
) -> Result<PreparedTaskCreate, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
{
    let projection = parse_create_input(input.raw)?;
    let artifacts = &projection["artifacts"];
    if artifacts["execution"] != input.execution_dir {
        return Err(contract(
            "task_create_path_mismatch",
            "The explicit execution path must match the TASK index.",
        ));
    }
    let source = serde_json::from_value(projection["source"].clone()).map_err(|_| {
        contract(
            "invalid_task_source",
            "Creation requires explicit Task provenance.",
        )
    })?;
    let artifact_paths = serde_json::from_value(artifacts.clone()).map_err(|_| {
        contract(
            "invalid_artifact_path",
            "Creation requires explicit artifact roots.",
        )
    })?;
    crate::task::source::verify_provenance(
        paths,
        projection["requirement_id"].as_str().ok_or_else(|| {
            contract(
                "invalid_requirement_id",
                "Creation requires a requirement ID.",
            )
        })?,
        &source,
        &artifact_paths,
    )?;
    let prepared =
        prepare_collection(&projection, input.index_path, input.source_root).map_err(|issue| {
            WorkError::new(
                ExitCode::Contract,
                issue.reason_code,
                issue.message,
                issue.details,
            )
        })?;
    let validation = validate_collection(
        instructions,
        skills,
        paths,
        skill_roots,
        CollectionInput {
            index_raw: &prepared.index_raw,
            item_raw: &prepared.items,
            index_path: input.index_path,
        },
    )?;
    let initial_execution =
        build_initial_execution_index(&validation["collection_contract"], &validation).map_err(
            |issue| {
                WorkError::new(
                    ExitCode::Contract,
                    issue.reason_code,
                    issue.message,
                    issue.details,
                )
            },
        )?;
    let execution_raw = render_execution_index(&initial_execution).map_err(|_| {
        contract(
            "invalid_contract_value",
            "The execution index cannot be rendered.",
        )
    })?;
    validate_execution_index(&initial_execution, &execution_raw).map_err(|issue| {
        WorkError::new(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let approval_bytes = work_operations::task::create::approval_with_execution(
        &prepared.approval_bytes,
        &format!("{}/index.json", input.execution_dir),
        &execution_raw,
    );
    Ok(PreparedTaskCreate {
        index_raw: prepared.index_raw,
        items: prepared.items,
        execution_raw,
        approval_bytes,
        validation,
        initial_execution,
    })
}

pub fn create_task_from_project<H, S, P, T>(
    instructions: &H,
    skills: &S,
    paths: &P,
    storage: &T,
    skill_roots: &[SkillRoot],
    request: TaskCreateProjectInput<'_>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: ArtifactPathRepository + SourceSnapshotReader,
    T: TaskCreationRepository,
{
    parse_create_input(request.raw)?;
    let prepared = prepare_task_create(
        instructions,
        skills,
        paths,
        skill_roots,
        TaskCreateInput {
            raw: request.raw,
            index_path: request.task_path,
            source_root: request.source_root,
            execution_dir: request.execution_dir,
        },
    )?;
    let changed = storage.publish(
        request.task_path,
        request.execution_dir,
        &prepared,
        request.recovery,
    )?;
    let stored_index = storage.read_execution_index(request.execution_dir)?;
    let stored: Value = parse_json_contract(&stored_index).map_err(|_| {
        WorkError::new(
            ExitCode::WorkflowState,
            "unrecoverable_task_create_state",
            "The installed execution index is invalid.",
            json!({}),
        )
    })?;
    validate_execution_index(&stored, &stored_index).map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let status = if request.recovery {
        if changed {
            "recovered"
        } else {
            "already_completed"
        }
    } else {
        "created"
    };
    let mut result = json!({"schema":if request.recovery {"work-task-create-recovery"}
        else {"work-task-create"},
        "requirement_id":prepared.validation["requirement_id"],
        "spec_id":prepared.validation["spec_id"],"task_path":request.task_path,
        "execution_dir":request.execution_dir,
        "task_collection_sha256":prepared.validation["task_collection_sha256"],
        "task_index_sha256":prepared.validation["task_index_sha256"],
        "task_item_sha256":prepared.validation["task_item_sha256"],
        "index_sha256":fingerprint::raw(&stored_index),"status":status});
    if request.recovery {
        result["recovered"] = json!(changed);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::hierarchy::HierarchyCatalogRepository;
    use work_operations::hierarchy::{CrossModeCatalog, Hierarchy};

    struct UnusedSources;

    impl HierarchyCatalogRepository for UnusedSources {
        fn cross_mode_catalog(&self) -> Result<CrossModeCatalog, WorkError> {
            panic!("source lookup must follow plan read")
        }

        fn mode_paths(&self, _: &str) -> Result<Vec<String>, WorkError> {
            panic!("source lookup must follow plan read")
        }
    }

    impl InstructionSourceRepository for UnusedSources {
        fn load_sources(
            &self,
            _: &str,
            _: &Hierarchy,
            _: &[String],
        ) -> Result<work_operations::instruction::SourceSet, WorkError> {
            panic!("source lookup must follow plan read")
        }
    }

    impl SkillSnapshotRepository for UnusedSources {
        fn snapshot(&self, _: &str, _: &str, _: &str) -> Result<Value, WorkError> {
            panic!("skill lookup must follow plan read")
        }
    }

    impl ArtifactPathRepository for UnusedSources {
        fn resolve(&self, _: &str) -> Result<std::path::PathBuf, WorkError> {
            panic!("unexpected path resolution")
        }
        fn exists(&self, _: &str) -> Result<bool, WorkError> {
            panic!("unexpected existence check")
        }
        fn read_raw(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("unexpected raw read")
        }
        fn create_new(&self, _: &str, _: &[u8]) -> Result<(), WorkError> {
            panic!("no writes before validation")
        }
        fn validate_paths(
            &self,
            _: &work_model::identifiers::RequirementId,
            _: &crate::artifact_paths::ArtifactPaths,
        ) -> Result<(), WorkError> {
            Ok(())
        }
    }
    impl SourceSnapshotReader for UnusedSources {
        fn read_snapshot(
            &self,
            id: &work_model::identifiers::RequirementId,
            source: &work_model::identifiers::SourceId,
        ) -> Result<crate::ports::SnapshotBytes, WorkError> {
            self.read_snapshot_at(id, source, "outputs/work/sources/example")
        }
        fn read_snapshot_at(
            &self,
            _: &work_model::identifiers::RequirementId,
            _: &work_model::identifiers::SourceId,
            root: &str,
        ) -> Result<crate::ports::SnapshotBytes, WorkError> {
            assert_eq!(root, "outputs/work/sources/example");
            Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "source_snapshot_missing",
                "The fixed Snapshot is missing.",
                json!({}),
            ))
        }
    }
    struct FailingStorage(Cell<bool>);

    impl TaskCreationRepository for FailingStorage {
        fn publish(
            &self,
            _: &str,
            _: &str,
            _: &PreparedTaskCreate,
            _: bool,
        ) -> Result<bool, WorkError> {
            panic!("publication must not start after plan read fails")
        }

        fn read_execution_index(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("execution read must follow publication")
        }
    }

    #[test]
    fn create_stops_at_missing_snapshot_before_any_publication() {
        let storage = FailingStorage(Cell::new(false));
        let raw = serde_json::to_vec(&json!({
            "schema":"work-task-collection-projection",
            "requirement_id":"example","spec_id":null,"status":null,"title":null,
            "summary":null,"artifacts":{"source":"outputs/work/sources/example","task":"outputs/work/tasks/example/index.json","execution":"outputs/work/executions/example"},"source":{"kind":"snapshot","manifest":work_model::contract_data::registry_value()["items"]["work-source-snapshot"]["description"]["example"]},"hierarchy_selection":null,"skill_selection":null,"acceptance_criteria":null,
            "instruction_selection":null,"tasks":null,"readiness":null
        }))
        .unwrap();
        let error = create_task_from_project(
            &UnusedSources,
            &UnusedSources,
            &UnusedSources,
            &storage,
            &[],
            TaskCreateProjectInput {
                raw: &raw,
                source_root: "outputs/work/sources/example",
                task_path: "outputs/work/tasks/example/index.json",
                execution_dir: "outputs/work/executions/example",
                recovery: false,
            },
        )
        .unwrap_err();
        assert!(!storage.0.get());
        assert_eq!(error.reason_code, "source_snapshot_missing");
    }
}
