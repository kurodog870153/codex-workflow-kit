//! Exclusive formal TASK creation and exact-byte recovery.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;
use work_feature::task::create::{
    PreparedTaskCreate, TaskCreateProjectInput, TaskCreationRepository, create_task_from_project,
};

use crate::artifact_paths::LocalArtifactPaths;
use crate::files::{LocalFiles, resolve_project_path};
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};

fn failure(reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(ExitCode::WorkflowState, reason, message, details)
}

fn create_target(
    path: &Path,
    bytes: &[u8],
    recovery: bool,
    reason: &str,
) -> Result<bool, WorkError> {
    if path.exists() {
        if !recovery || !path.is_file() || LocalFiles.read_raw(path)? != bytes {
            return Err(failure(
                reason,
                "An existing TASK create target conflicts with approved bytes.",
                json!({"path":path}),
            ));
        }
        return Ok(false);
    }
    LocalFiles.create_new(path, bytes)?;
    Ok(true)
}

fn check_existing_target(path: &Path, bytes: &[u8]) -> Result<(), WorkError> {
    if path.exists() && (!path.is_file() || LocalFiles.read_raw(path)? != bytes) {
        return Err(failure(
            "unrecoverable_task_create_state",
            "An existing TASK create target conflicts with approved bytes.",
            json!({"path":path}),
        ));
    }
    Ok(())
}

fn checked_directory(path: &Path, allowed: &[&str], recovery: bool) -> Result<(), WorkError> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_symlink() || !path.is_dir() {
        return Err(failure(
            "unrecoverable_task_create_state",
            "The TASK create target is not a safe directory.",
            json!({"path":path}),
        ));
    }
    let mut unexpected = Vec::new();
    for entry in fs::read_dir(path).map_err(|_| {
        failure(
            "unrecoverable_task_create_state",
            "The TASK create directory cannot be inspected.",
            json!({"path":path}),
        )
    })? {
        let entry = entry.map_err(|_| {
            failure(
                "unrecoverable_task_create_state",
                "The TASK create directory cannot be inspected.",
                json!({"path":path}),
            )
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !allowed.contains(&name.as_str()) || entry.path().is_symlink() {
            unexpected.push(name);
        }
    }
    if !unexpected.is_empty() {
        unexpected.sort();
        return Err(failure(
            if recovery {
                "unrecoverable_task_create_state"
            } else {
                "task_create_target_exists"
            },
            "The TASK create directory contains unexpected content.",
            json!({"unexpected_collection_entries":unexpected}),
        ));
    }
    Ok(())
}

pub(crate) fn targets(
    root: &Path,
    index_relative: &str,
    execution_relative: &str,
    prepared: &PreparedTaskCreate,
    recovery: bool,
) -> Result<bool, WorkError> {
    let (_, index_path) = resolve_project_path(root, index_relative)?;
    let (_, execution_path) = resolve_project_path(root, execution_relative)?;
    let collection = index_path.parent().expect("TASK index path has parent");
    let items_dir = collection.join("tasks");
    let execution_index = execution_path.join("index.json");
    if !recovery {
        checked_directory(collection, &[], false)?;
        if index_path.exists() || items_dir.exists() || execution_path.exists() {
            return Err(failure(
                "task_create_target_exists",
                "TASK create requires formal collection and execution targets to be absent.",
                json!({"task_index_exists":index_path.exists(),
                    "task_items_directory_exists":items_dir.exists(),
                    "execution_exists":execution_path.exists()}),
            ));
        }
    } else {
        if !collection.exists() {
            return Err(failure(
                "unrecoverable_task_create_state",
                "TASK recovery requires preserved create storage.",
                json!({}),
            ));
        }
        checked_directory(collection, &["index.json", "tasks"], true)?;
        if execution_path.exists() {
            checked_directory(
                &execution_path,
                &["index.json", ".work-state-writer.lock"],
                true,
            )?;
        }
    }
    if items_dir.exists() {
        if !items_dir.is_dir() || items_dir.is_symlink() {
            return Err(failure(
                "unrecoverable_task_create_state",
                "The TASK item target is not a safe directory.",
                json!({}),
            ));
        }
        let expected = prepared
            .items
            .keys()
            .map(|id| format!("{id}.json"))
            .collect::<std::collections::BTreeSet<_>>();
        for entry in fs::read_dir(&items_dir).map_err(|_| {
            failure(
                "unrecoverable_task_create_state",
                "The TASK item directory cannot be inspected.",
                json!({}),
            )
        })? {
            let entry = entry.map_err(|_| {
                failure(
                    "unrecoverable_task_create_state",
                    "The TASK item directory cannot be inspected.",
                    json!({}),
                )
            })?;
            if !expected.contains(&entry.file_name().to_string_lossy().to_string()) {
                return Err(failure(
                    "unrecoverable_task_create_state",
                    "The TASK item directory contains unknown content.",
                    json!({}),
                ));
            }
        }
    }
    if recovery {
        for (task_id, raw) in &prepared.items {
            check_existing_target(&items_dir.join(format!("{task_id}.json")), raw)?;
        }
        check_existing_target(&index_path, &prepared.index_raw)?;
        check_existing_target(&execution_index, &prepared.execution_raw)?;
    }
    fs::create_dir_all(&items_dir).map_err(|_| {
        failure(
            "file_write_failed",
            "The TASK item directory cannot be created.",
            json!({}),
        )
    })?;
    let mut changed = false;
    for (task_id, raw) in &prepared.items {
        changed |= create_target(
            &items_dir.join(format!("{task_id}.json")),
            raw,
            recovery,
            "unrecoverable_task_create_state",
        )?;
    }
    changed |= create_target(
        &index_path,
        &prepared.index_raw,
        recovery,
        "unrecoverable_task_create_state",
    )?;
    fs::create_dir_all(&execution_path).map_err(|_| {
        failure(
            "file_write_failed",
            "The execution directory cannot be created.",
            json!({}),
        )
    })?;
    changed |= create_target(
        &execution_index,
        &prepared.execution_raw,
        recovery,
        "unrecoverable_task_create_state",
    )?;
    Ok(changed)
}

pub struct CreateTaskRequest<'a> {
    pub raw: &'a [u8],
    pub source_root: &'a str,
    pub task_path: &'a str,
    pub execution_dir: &'a str,
    pub recovery: bool,
}

pub struct LocalTaskCreation {
    pub project_root: PathBuf,
}

impl TaskCreationRepository for LocalTaskCreation {
    fn publish(
        &self,
        task_path: &str,
        execution_dir: &str,
        prepared: &PreparedTaskCreate,
        recovery: bool,
    ) -> Result<bool, WorkError> {
        targets(
            &self.project_root,
            task_path,
            execution_dir,
            prepared,
            recovery,
        )
    }

    fn read_execution_index(&self, execution_dir: &str) -> Result<Vec<u8>, WorkError> {
        let (_, path) = resolve_project_path(&self.project_root, execution_dir)?;
        LocalFiles.read_raw(&path.join("index.json"))
    }
}

pub fn create_task_artifacts(
    project_root: &Path,
    skill_root: &Path,
    skill_configs: &[SkillRootConfig],
    request: CreateTaskRequest<'_>,
) -> Result<Value, WorkError> {
    let work = LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
    let skill_roots = skill_configs
        .iter()
        .map(|root| SkillRoot {
            scope: root.scope.clone(),
            locator: root.locator.clone(),
        })
        .collect::<Vec<_>>();
    let skills = LocalSkillCatalog {
        roots: skill_configs.to_vec(),
    };
    let paths = LocalArtifactPaths {
        project_root: project_root.to_path_buf(),
    };
    create_task_from_project(
        &work,
        &skills,
        &paths,
        &LocalTaskCreation {
            project_root: project_root.to_path_buf(),
        },
        &skill_roots,
        TaskCreateProjectInput {
            raw: request.raw,
            task_path: request.task_path,
            source_root: request.source_root,
            execution_dir: request.execution_dir,
            recovery: request.recovery,
        },
    )
}
