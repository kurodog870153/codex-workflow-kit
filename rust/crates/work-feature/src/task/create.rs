//! In-memory preparation of exact formal TASK collection creation targets.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use work_operations::canonical::{JsonContractIssue, parse_json_contract};
use work_operations::derivation::fingerprint;
use work_operations::execution::index::{
    build_initial_execution_index, render_execution_index, validate_execution_index,
};
use work_operations::task::create::prepare_collection;

use crate::error::{ExitCode, WorkError};
use crate::instruction::InstructionSourceRepository;
use crate::plan::PlanPathRepository;
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
    pub plan_path: &'a str,
    pub execution_dir: &'a str,
    pub plan_raw: &'a [u8],
}

pub struct TaskCreateProjectInput<'a> {
    pub raw: &'a [u8],
    pub plan_path: &'a str,
    pub task_path: &'a str,
    pub execution_dir: &'a str,
    pub recovery: bool,
}

pub trait TaskCreationRepository {
    fn read_plan(&self, relative_path: &str) -> Result<Vec<u8>, WorkError>;
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
    if projection["schema"] != "work-task-collection-projection/v1" {
        return Err(contract(
            "task_create_schema",
            "TASK create input must be a complete work-task-collection-projection/v1 contract.",
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
        "source_plan",
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
        "source_plan",
        "instruction_selection",
        "execution_defaults",
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
    P: PlanPathRepository,
{
    let projection = parse_create_input(input.raw)?;
    let artifacts = &projection["artifacts"];
    if artifacts["execution"] != input.execution_dir {
        return Err(contract(
            "task_create_path_mismatch",
            "The explicit execution path must match the TASK index.",
        ));
    }
    let prepared =
        prepare_collection(&projection, input.index_path, input.plan_path).map_err(|issue| {
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
            source_plan_raw: input.plan_raw,
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
    Ok(PreparedTaskCreate {
        index_raw: prepared.index_raw,
        items: prepared.items,
        execution_raw,
        approval_bytes: prepared.approval_bytes,
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
    P: PlanPathRepository,
    T: TaskCreationRepository,
{
    parse_create_input(request.raw)?;
    let plan_raw = storage.read_plan(request.plan_path)?;
    let prepared = prepare_task_create(
        instructions,
        skills,
        paths,
        skill_roots,
        TaskCreateInput {
            raw: request.raw,
            index_path: request.task_path,
            plan_path: request.plan_path,
            execution_dir: request.execution_dir,
            plan_raw: &plan_raw,
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
    let mut result = json!({"schema":if request.recovery {"work-task-create-recovery/v1"}
        else {"work-task-create/v1"},
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
    use work_operations::identifiers::RequirementId;

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

    impl PlanPathRepository for UnusedSources {
        fn default_paths(&self, _: &RequirementId) -> Result<Value, WorkError> {
            panic!("path lookup must follow plan read")
        }

        fn validate_paths(
            &self,
            _: &RequirementId,
            _: &Value,
            _: &str,
            _: bool,
        ) -> Result<(), WorkError> {
            panic!("path lookup must follow plan read")
        }

        fn exists(&self, _: &str) -> Result<bool, WorkError> {
            panic!("path lookup must follow plan read")
        }

        fn create_exclusive(&self, _: &str, _: &[u8]) -> Result<(), WorkError> {
            panic!("path lookup must follow plan read")
        }

        fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            panic!("path lookup must follow plan read")
        }
    }

    struct FailingStorage(Cell<bool>);

    impl TaskCreationRepository for FailingStorage {
        fn read_plan(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
            assert_eq!(relative_path, "plans/example.json");
            self.0.set(true);
            Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "plan_missing",
                "The source Plan is missing.",
                json!({}),
            ))
        }

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
    fn create_stops_at_failed_plan_port_before_any_publication() {
        let storage = FailingStorage(Cell::new(false));
        let raw = serde_json::to_vec(&json!({
            "schema":"work-task-collection-projection/v1",
            "requirement_id":null,"spec_id":null,"status":null,"title":null,
            "summary":null,"artifacts":null,"source_plan":null,
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
                plan_path: "plans/example.json",
                task_path: "tasks/example/index.json",
                execution_dir: "execution/example",
                recovery: false,
            },
        )
        .unwrap_err();
        assert!(storage.0.get());
        assert_eq!(error.reason_code, "plan_missing");
    }
}
