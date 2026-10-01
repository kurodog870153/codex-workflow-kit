//! Assemble reviewed TASK discussions into one complete formal collection.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use work_operations::canonical::{canonical_json, parse_json_contract};
use work_operations::derivation::fingerprint;
use work_operations::task::TaskIssue;
use work_operations::task::candidate::{build_semantic_candidate, validate_semantic_candidate};
use work_operations::task::draft::{validate_planning_index, validate_task_draft};
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::error::{ExitCode, WorkError};
use crate::instruction::{InstructionSourceRepository, load, select, task_document_selection};
use crate::plan::{PlanPathRepository, PlanValidationInput, validate_plan};
use crate::skill::{SkillRoot, SkillSnapshotRepository};
use crate::task::create::{TaskCreateInput, prepare_task_create};

pub struct AssemblyInput<'a> {
    pub index: &'a Value,
    pub drafts: &'a BTreeMap<String, Value>,
    pub plan_raw: &'a [u8],
    pub metadata: &'a Value,
    pub expected_revision: u64,
    pub plan_path: &'a str,
}

pub struct ProjectAssemblyInput<'a> {
    pub requirement_id: &'a str,
    pub metadata: &'a Value,
    pub expected_revision: u64,
    pub plan_path: &'a str,
}

pub trait TaskAssemblyRepository {
    fn read_planning_index(&self, requirement_id: &str) -> Result<Value, WorkError>;
    fn read_plan_raw(&self, plan_path: &str) -> Result<Vec<u8>, WorkError>;
    fn read_draft(&self, index: &Value, task_id: &str) -> Result<Value, WorkError>;
}

pub fn assemble_from_repository<R, H, S, P>(
    repository: &R,
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    input: ProjectAssemblyInput<'_>,
) -> Result<Value, WorkError>
where
    R: TaskAssemblyRepository,
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    let index = repository.read_planning_index(input.requirement_id)?;
    let plan_raw = repository.read_plan_raw(input.plan_path)?;
    let mut drafts = BTreeMap::new();
    for entry in index["tasks"].as_array().expect("validated planning tasks") {
        if entry["status"] != "planned" {
            let id = entry["id"].as_str().expect("validated ID");
            drafts.insert(id.to_owned(), repository.read_draft(&index, id)?);
        }
    }
    let assembled = assemble_task_drafts(
        instructions,
        skills,
        paths,
        skill_roots,
        AssemblyInput {
            index: &index,
            drafts: &drafts,
            plan_raw: &plan_raw,
            metadata: input.metadata,
            expected_revision: input.expected_revision,
            plan_path: input.plan_path,
        },
    )?;
    if repository.read_planning_index(input.requirement_id)? != index {
        return Err(failure(
            ExitCode::WorkflowState,
            "draft_revision_conflict",
            "The planning index changed during assembly.",
            json!({}),
        ));
    }
    Ok(assembled)
}

pub fn approved_contract<'a>(
    assembled: &'a Value,
    approved_sha256: &str,
) -> Result<&'a Value, WorkError> {
    if assembled["approval_sha256"] != approved_sha256 {
        return Err(failure(
            ExitCode::ArtifactIntegrity,
            "draft_approval_mismatch",
            "The assembled content differs from the reviewed fingerprint.",
            json!({}),
        ));
    }
    Ok(&assembled["contract"])
}

pub fn render_approved_contract(
    assembled: &Value,
    approved_sha256: &str,
) -> Result<Vec<u8>, WorkError> {
    let contract = approved_contract(assembled, approved_sha256)?;
    render_task(contract, TaskDocumentKind::Collection).map_err(|_| {
        failure(
            ExitCode::Contract,
            "invalid_contract_value",
            "The TASK contract cannot be rendered.",
            json!({}),
        )
    })
}

fn domain(issue: TaskIssue) -> WorkError {
    WorkError::new(
        ExitCode::Contract,
        issue.reason_code,
        issue.message,
        issue.details,
    )
}

fn failure(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn identifiers(plan: &Value, field: &str) -> Vec<Value> {
    plan[field]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| row["id"].clone())
        .collect()
}

pub fn assemble_task_drafts<H, S, P>(
    instructions: &H,
    skills: &S,
    paths: &P,
    skill_roots: &[SkillRoot],
    input: AssemblyInput<'_>,
) -> Result<Value, WorkError>
where
    H: InstructionSourceRepository,
    S: SkillSnapshotRepository,
    P: PlanPathRepository,
{
    let metadata = input.metadata.as_object().ok_or_else(|| {
        failure(
            ExitCode::Contract,
            "expected_object",
            "Assembly metadata must be an object.",
            json!({"location":"assembly"}),
        )
    })?;
    if !metadata.contains_key("title")
        || !metadata.contains_key("summary")
        || metadata.keys().any(|field| {
            !["title", "summary", "execution_defaults", "decisions"].contains(&field.as_str())
        })
    {
        return Err(failure(
            ExitCode::Contract,
            "invalid_object_fields",
            "Assembly metadata has missing or unknown fields.",
            json!({"location":"assembly"}),
        ));
    }
    validate_planning_index(input.index).map_err(domain)?;
    if input.index["revision"].as_u64() != Some(input.expected_revision) {
        return Err(failure(
            ExitCode::WorkflowState,
            "draft_revision_conflict",
            "The reviewed planning revision is no longer current.",
            json!({}),
        ));
    }
    let plan = parse_json_contract(input.plan_raw).map_err(|_| {
        failure(
            ExitCode::Contract,
            "invalid_json_contract",
            "The source Plan is invalid.",
            json!({}),
        )
    })?;
    let plan_validation = validate_plan(
        instructions,
        skills,
        paths,
        skill_roots,
        &plan,
        PlanValidationInput {
            raw: input.plan_raw,
            actual_plan_path: input.plan_path,
            allow_task_index: true,
        },
    )?;
    if input.index["requirement_id"] != plan_validation["requirement_id"]
        || [
            "plan_sha256",
            "hierarchy_selection_sha256",
            "skill_selection_sha256",
        ]
        .iter()
        .any(|field| input.index["source"][*field] != plan_validation[*field])
    {
        return Err(failure(
            ExitCode::ArtifactIntegrity,
            "draft_source_drift",
            "The planning source differs from the current validated Plan.",
            json!({}),
        ));
    }
    let entries = input.index["tasks"]
        .as_array()
        .expect("validated planning tasks");
    let mut dependency_files = BTreeMap::new();
    for entry in entries {
        let id = entry["id"].as_str().unwrap();
        if let Some(draft) = input.drafts.get(id) {
            validate_task_draft(draft, input.index).map_err(domain)?;
            if let Some(candidate) = draft.get("task_candidate") {
                validate_semantic_candidate(candidate, true).map_err(domain)?;
                let files = candidate["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter_map(|(position, row)| {
                        Some((
                            row["key"].as_str()?.to_owned(),
                            format!("FILE-{:03}", position + 1),
                        ))
                    })
                    .collect::<BTreeMap<_, _>>();
                dependency_files.insert(id.to_owned(), files);
            }
        }
    }
    let mut tasks = Vec::new();
    let mut source_sets = Vec::new();
    let mut sorted = entries.clone();
    sorted.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    for entry in &sorted {
        let task_id = entry["id"].as_str().unwrap();
        if entry["status"] != "refined" {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_not_refined",
                "Every active TASK must complete discussion before assembly.",
                json!({"task_id":task_id}),
            ));
        }
        let draft = input.drafts.get(task_id).ok_or_else(|| {
            failure(
                ExitCode::WorkflowState,
                "draft_not_saved",
                "The TASK discussion is missing.",
                json!({"task_id":task_id}),
            )
        })?;
        let candidate = draft.get("task_candidate").filter(|value| value.is_object()).ok_or_else(||
            failure(ExitCode::Contract, "task_candidate_required",
                "A structured candidate is required; discussion notes cannot be inferred into a TASK.",
                json!({"task_id":task_id})))?;
        let dependencies = entry["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let acceptance = identifiers(&plan, "acceptance_criteria")
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let semantic =
            build_semantic_candidate(candidate, &acceptance, &dependencies, &dependency_files)
                .map_err(domain)?;
        let selection = entry.get("instruction_selection").ok_or_else(|| {
            failure(
                ExitCode::WorkflowState,
                "draft_selection_required",
                "Confirm instruction paths and references for this TASK.",
                json!({}),
            )
        })?;
        let paths_selected = selection["selected_paths"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let refs = selection["references"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let loaded = load(instructions, "task", &paths_selected, &refs)?;
        let selected = select(instructions, "task", &paths_selected, &refs)?;
        let selected = serde_json::to_value(selected).map_err(|_| {
            failure(
                ExitCode::Contract,
                "invalid_contract_value",
                "The instruction selection cannot be serialized.",
                json!({}),
            )
        })?;
        if selected["instructions_sha256"] != entry["instructions_sha256"] {
            return Err(failure(
                ExitCode::ArtifactIntegrity,
                "draft_instruction_drift",
                "The TASK instruction selection differs from its confirmed boundary.",
                json!({}),
            ));
        }
        source_sets.push(loaded);
        let mut task = semantic.as_object().expect("semantic result").clone();
        for field in ["id", "title", "goal", "skill_id", "dependencies"] {
            task.insert(field.into(), entry[field].clone());
        }
        task.insert("instruction_selection".into(), selected);
        task.insert(
            "traceability".into(),
            json!({"goal_ids":identifiers(&plan,"goals"),
            "deliverable_ids":identifiers(&plan,"deliverables"),
            "acceptance_ids":identifiers(&plan,"acceptance_criteria")}),
        );
        tasks.push(Value::Object(task));
    }
    let mut contract = metadata.clone();
    contract.insert("schema".into(), json!("work-task-collection-projection/v1"));
    contract.insert(
        "requirement_id".into(),
        input.index["requirement_id"].clone(),
    );
    contract.insert("spec_id".into(), json!("TASK-SPEC-001"));
    contract.insert("status".into(), json!("confirmed"));
    contract.insert("artifacts".into(), plan["artifacts"].clone());
    contract.insert(
        "source_plan".into(),
        json!({
        "canonical_sha256":input.index["source"]["plan_sha256"],
        "hierarchy_selection_sha256":input.index["source"]["hierarchy_selection_sha256"]}),
    );
    contract.insert(
        "instruction_selection".into(),
        task_document_selection(&source_sets)?,
    );
    contract.insert("tasks".into(), Value::Array(tasks));
    contract.insert(
        "readiness".into(),
        json!({"status":"passed","spec_id":"TASK-SPEC-001"}),
    );
    let contract = Value::Object(contract);
    let raw = render_task(&contract, TaskDocumentKind::Collection).map_err(|_| {
        failure(
            ExitCode::Contract,
            "invalid_contract_value",
            "The TASK contract cannot be rendered.",
            json!({}),
        )
    })?;
    let artifacts = &contract["artifacts"];
    let prepared = prepare_task_create(
        instructions,
        skills,
        paths,
        skill_roots,
        TaskCreateInput {
            raw: &raw,
            index_path: artifacts["task"].as_str().unwrap(),
            plan_path: artifacts["plan"].as_str().unwrap(),
            execution_dir: artifacts["execution"].as_str().unwrap(),
            plan_raw: input.plan_raw,
        },
    )?;
    let index_raw = canonical_json(input.index).expect("JSON value serializes");
    Ok(
        json!({"schema":"work-task-draft-assembly/v1","status":"valid",
        "requirement_id":input.index["requirement_id"],"revision":input.expected_revision,
        "approval_sha256":fingerprint::task_draft_approval(&index_raw, &prepared.approval_bytes),
        "task_collection_sha256":prepared.validation["task_collection_sha256"],
        "task_index_sha256":prepared.validation["task_index_sha256"],
        "task_item_sha256":prepared.validation["task_item_sha256"],"contract":contract}),
    )
}

#[cfg(test)]
mod project_tests {
    use super::*;
    use std::cell::RefCell;

    struct FailingRepository {
        reads: RefCell<Vec<&'static str>>,
    }

    impl TaskAssemblyRepository for FailingRepository {
        fn read_planning_index(&self, _: &str) -> Result<Value, WorkError> {
            self.reads.borrow_mut().push("index");
            Ok(json!({"tasks": []}))
        }

        fn read_plan_raw(&self, _: &str) -> Result<Vec<u8>, WorkError> {
            self.reads.borrow_mut().push("plan");
            Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "plan_read_failed",
                "The source Plan cannot be read.",
                json!({}),
            ))
        }

        fn read_draft(&self, _: &Value, _: &str) -> Result<Value, WorkError> {
            panic!("draft read must follow a successful Plan read")
        }
    }

    #[test]
    fn project_assembly_stops_after_plan_port_failure() {
        struct Unused;
        impl crate::hierarchy::HierarchyCatalogRepository for Unused {
            fn cross_mode_catalog(
                &self,
            ) -> Result<work_operations::hierarchy::CrossModeCatalog, WorkError> {
                panic!("hierarchy must not be used")
            }
            fn mode_paths(&self, _: &str) -> Result<Vec<String>, WorkError> {
                panic!("hierarchy must not be used")
            }
        }
        impl InstructionSourceRepository for Unused {
            fn load_sources(
                &self,
                _: &str,
                _: &work_operations::hierarchy::Hierarchy,
                _: &[String],
            ) -> Result<work_operations::instruction::SourceSet, WorkError> {
                panic!("instruction source must not be used")
            }
        }
        impl SkillSnapshotRepository for Unused {
            fn snapshot(&self, _: &str, _: &str, _: &str) -> Result<Value, WorkError> {
                panic!("skill must not be used")
            }
        }
        impl PlanPathRepository for Unused {
            fn default_paths(
                &self,
                _: &work_operations::identifiers::RequirementId,
            ) -> Result<Value, WorkError> {
                panic!("paths must not be used")
            }
            fn validate_paths(
                &self,
                _: &work_operations::identifiers::RequirementId,
                _: &Value,
                _: &str,
                _: bool,
            ) -> Result<(), WorkError> {
                panic!("paths must not be used")
            }
            fn exists(&self, _: &str) -> Result<bool, WorkError> {
                panic!("paths must not be used")
            }
            fn create_exclusive(&self, _: &str, _: &[u8]) -> Result<(), WorkError> {
                panic!("paths must not be used")
            }
            fn read(&self, _: &str) -> Result<Vec<u8>, WorkError> {
                panic!("paths must not be used")
            }
        }
        let repository = FailingRepository {
            reads: RefCell::new(Vec::new()),
        };
        let error = assemble_from_repository(
            &repository,
            &Unused,
            &Unused,
            &Unused,
            &[],
            ProjectAssemblyInput {
                requirement_id: "example",
                metadata: &json!({}),
                expected_revision: 1,
                plan_path: "plan.json",
            },
        )
        .unwrap_err();
        assert_eq!(error.reason_code, "plan_read_failed");
        assert_eq!(*repository.reads.borrow(), ["index", "plan"]);
    }
}
