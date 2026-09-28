//! Immutable TASK planning history reads and status projection.

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::plan::{PlanPathRepository, validate_plan_bytes};
use work_feature::skill::SkillRoot;
use work_feature::task::draft::{
    self as task_draft, DraftSourceCheck, PreparedListUpdate, SourceUpdateRequest,
    TaskDraftHistoryRepository, ValidatedSourceRefresh, check_validated_sources,
    prepare_list_update, prepare_save, prepare_source_update, prepare_validated_source_refresh,
};
use work_model::task::draft::{TaskDraft, TaskPlanningIndex};
use work_operations::canonical::{canonical_json, parse_json_contract};
use work_operations::identifiers::RequirementId;

use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};

#[derive(Debug, Clone)]
pub struct LocalTaskDraftStorage {
    pub project_root: PathBuf,
}

pub struct TaskSourceCheckRequest<'a> {
    pub requirement_id: &'a str,
    pub task_id: &'a str,
    pub expected_revision: u64,
    pub plan_path: &'a str,
    pub skill_root: &'a Path,
    pub skill_configs: &'a [SkillRootConfig],
    pub selected_paths: Option<&'a [String]>,
    pub reference_names: Option<&'a [String]>,
}

pub struct TaskSourceUpdateProjectRequest<'a> {
    pub requirement_id: &'a str,
    pub raw_request: &'a Value,
    pub expected_revision: u64,
    pub plan_path: &'a str,
    pub skill_root: &'a Path,
    pub skill_configs: &'a [SkillRootConfig],
    pub recover: bool,
}

fn failure(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn ensure_status_index_unchanged(expected: &Value, latest: &Value) -> Result<(), WorkError> {
    if latest != expected {
        return Err(failure(
            ExitCode::WorkflowState,
            "draft_revision_conflict",
            "The planning index changed during status inspection.",
            json!({}),
        ));
    }
    Ok(())
}

impl LocalTaskDraftStorage {
    pub fn update_sources_from_project(
        &self,
        request: TaskSourceUpdateProjectRequest<'_>,
    ) -> Result<Value, WorkError> {
        let TaskSourceUpdateProjectRequest {
            requirement_id,
            raw_request,
            expected_revision,
            plan_path,
            skill_root,
            skill_configs,
            recover,
        } = request;
        let reason = task_draft::source_update_request(raw_request, expected_revision)?;
        let current = self.read_planning_index(requirement_id)?;
        let previous = if recover {
            self.read_planning_revision(requirement_id, expected_revision)?
        } else {
            current.clone()
        };
        let selections = task_draft::source_update_selections(
            raw_request,
            &previous,
            requirement_id,
            expected_revision,
        )?;
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        let raw = paths.read(plan_path)?;
        let instructions = LocalHierarchyCatalog {
            skill_root: skill_root.to_path_buf(),
        };
        let skills = LocalSkillCatalog {
            roots: skill_configs.to_vec(),
        };
        let roots: Vec<SkillRoot> = skill_configs
            .iter()
            .map(|config| SkillRoot {
                scope: config.scope.clone(),
                locator: config.locator.clone(),
            })
            .collect();
        let validation =
            validate_plan_bytes(&instructions, &skills, &paths, &roots, &raw, plan_path)?;
        let plan = parse_json_contract(&raw).map_err(|_| {
            failure(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The Plan JSON is invalid.",
                json!({}),
            )
        })?;
        let prepared = prepare_validated_source_refresh(
            &instructions,
            self,
            &ValidatedSourceRefresh {
                previous: &previous,
                plan: &plan,
                plan_validation: &validation,
                selections: &selections,
                expected_revision,
                reason: &reason,
                plan_path,
            },
        )?;
        if paths.read(plan_path)? != raw || self.read_planning_index(requirement_id)? != current {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_revision_conflict",
                "The planning sources changed during preparation.",
                json!({}),
            ));
        }
        self.publish_source_update(requirement_id, &previous, &current, &prepared, recover)
    }

    pub fn save_discussion_request(
        &self,
        request: &Value,
        context: &TaskSourceCheckRequest<'_>,
        recover: bool,
    ) -> Result<Value, WorkError> {
        let expected = context.expected_revision;
        task_draft::validate_save_request(request, expected)?;
        let current = self.read_planning_index(context.requirement_id)?;
        let revision = task_draft::validate_save_revision(&current, expected, recover)?;
        let previous = if recover {
            self.read_planning_revision(context.requirement_id, expected)?
        } else {
            current.clone()
        };
        let (entry, paths, references) = task_draft::save_selection(
            &previous,
            context.task_id,
            context.selected_paths,
            context.reference_names,
        )?;
        let checked = self.check_sources(&TaskSourceCheckRequest {
            expected_revision: revision,
            requirement_id: context.requirement_id,
            task_id: context.task_id,
            plan_path: context.plan_path,
            skill_root: context.skill_root,
            skill_configs: context.skill_configs,
            selected_paths: Some(&paths),
            reference_names: Some(&references),
        })?;
        task_draft::validate_save_sources(&previous, entry, &checked)?;
        if self.read_planning_index(context.requirement_id)? != current {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_revision_conflict",
                "The planning index changed while preparing the save.",
                json!({}),
            ));
        }
        let (proposed, draft) = task_draft::build_save_documents(
            &previous,
            entry,
            &checked,
            request,
            context.requirement_id,
            context.task_id,
            expected,
        );
        if recover {
            self.recover_planning(&proposed, expected, Some(&draft))
        } else {
            self.save_planning(&proposed, expected, Some(&draft))
        }
    }

    fn root(&self, requirement_id: &str) -> Result<PathBuf, WorkError> {
        let id: RequirementId = requirement_id.parse().map_err(
            |issue: work_operations::identifiers::IdentifierIssue| {
                failure(
                    ExitCode::Contract,
                    issue.reason_code(),
                    "The requirement ID is invalid.",
                    json!({}),
                )
            },
        )?;
        let root = self.project_root.canonicalize().map_err(|_| {
            failure(
                ExitCode::IoFailure,
                "draft_path_resolution_failed",
                "The project root could not be resolved.",
                json!({}),
            )
        })?;
        Ok(root
            .join("outputs/work/tasks")
            .join(id.as_str())
            .join("drafts"))
    }

    fn path(&self, requirement_id: &str, suffix: &str) -> Result<PathBuf, WorkError> {
        let root = self.root(requirement_id)?;
        let path = root.join(suffix);
        let base = self.project_root.canonicalize().map_err(|_| {
            failure(
                ExitCode::IoFailure,
                "draft_path_resolution_failed",
                "The project root could not be resolved.",
                json!({}),
            )
        })?;
        let mut cursor = base.clone();
        for component in path
            .strip_prefix(&base)
            .expect("draft path is under project root")
            .components()
        {
            cursor.push(component);
            if cursor
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_path_link",
                    "Draft storage paths cannot contain links or junctions.",
                    json!({}),
                ));
            }
        }
        if !path.starts_with(&root) {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_path_escape",
                "The draft path escapes its requirement directory.",
                json!({}),
            ));
        }
        Ok(path)
    }

    pub(crate) fn require_initial_storage_free(
        &self,
        requirement_id: &str,
    ) -> Result<(), WorkError> {
        let index = self.path(requirement_id, "index.json")?;
        let directory = index.parent().expect("index has parent");
        let occupied = directory.exists()
            && fs::read_dir(directory).map_or(true, |mut entries| entries.next().is_some());
        if occupied {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_initial_storage_exists",
                "Existing planning storage requires review, not initialization.",
                json!({}),
            ));
        }
        Ok(())
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, WorkError> {
        fs::read(path).map_err(|_| {
            failure(
                ExitCode::IoFailure,
                "draft_read_failed",
                "The draft artifact could not be read.",
                json!({"path": path.to_string_lossy()}),
            )
        })
    }

    fn decode(&self, raw: &[u8]) -> Result<Value, WorkError> {
        let value = parse_json_contract(raw).map_err(|_| {
            failure(
                ExitCode::Contract,
                "invalid_json_contract",
                "The draft storage JSON is invalid.",
                json!({}),
            )
        })?;
        let rendered = canonical_json(&value).expect("JSON value serializes");
        if rendered != raw {
            return Err(failure(
                ExitCode::WorkflowState,
                "noncanonical_draft_storage",
                "The stored JSON is not canonical.",
                json!({}),
            ));
        }
        Ok(value)
    }

    fn write_new(path: &Path, raw: &[u8]) -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(raw)?;
        file.sync_all()
    }

    pub fn read_planning_revision(
        &self,
        requirement_id: &str,
        revision: u64,
    ) -> Result<Value, WorkError> {
        task_draft::require_existing_revision(revision)?;
        let raw =
            self.read(&self.path(requirement_id, &format!("history/{revision}/index.json"))?)?;
        let index = self.decode(&raw)?;
        task_draft::validate_historical_index(&index, requirement_id, revision)?;
        let index: TaskPlanningIndex =
            serde_json::from_value(index).expect("validated planning index matches Model");
        Ok(serde_json::to_value(index).expect("planning index serializes"))
    }

    pub fn read_planning_index(&self, requirement_id: &str) -> Result<Value, WorkError> {
        let raw = self.read(&self.path(requirement_id, "index.json")?)?;
        let index = self.decode(&raw)?;
        let revision = task_draft::validate_current_index(&index, requirement_id)?;
        let history =
            self.read(&self.path(requirement_id, &format!("history/{revision}/index.json"))?)?;
        task_draft::validate_current_index_history(&raw, &history)?;
        let index: TaskPlanningIndex =
            serde_json::from_value(index).expect("validated planning index matches Model");
        Ok(serde_json::to_value(index).expect("planning index serializes"))
    }

    pub fn read_draft(&self, index: &Value, task_id: &str) -> Result<Value, WorkError> {
        let requirement_id = index["requirement_id"]
            .as_str()
            .expect("validated requirement");
        let (save_revision, expected_sha256) = task_draft::draft_reference(index, task_id)?;
        let raw = self.read(&self.path(
            requirement_id,
            &format!("history/{save_revision}/{task_id}.json"),
        )?)?;
        task_draft::validate_draft_fingerprint(&raw, &expected_sha256)?;
        let draft = self.decode(&raw)?;
        task_draft::validate_stored_draft(&draft, index, task_id)?;
        let draft: TaskDraft =
            serde_json::from_value(draft).expect("validated TASK draft matches Model");
        Ok(serde_json::to_value(draft).expect("TASK draft serializes"))
    }

    pub fn status(&self, requirement_id: &str, task_id: Option<&str>) -> Result<Value, WorkError> {
        let current = self.path(requirement_id, "index.json")?;
        let inspect_error = || {
            failure(
                ExitCode::IoFailure,
                "draft_status_read_failed",
                "Planning storage could not be inspected.",
                json!({}),
            )
        };
        if !current.try_exists().map_err(|_| inspect_error())? {
            let root = self.root(requirement_id)?;
            let residue = root.try_exists().map_err(|_| inspect_error())?
                && fs::read_dir(&root)
                    .map_err(|_| inspect_error())?
                    .next()
                    .is_some();
            if current.try_exists().map_err(|_| inspect_error())? {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_revision_conflict",
                    "Planning was initialized during inspection; reload progress.",
                    json!({}),
                ));
            }
            return task_draft::status(requirement_id, None, task_id, None, residue);
        }
        let index = self.read_planning_index(requirement_id)?;
        let selected = task_id.or_else(|| index["current_task_id"].as_str());
        let next = index["revision"].as_u64().expect("validated revision") + 1;
        let recovery = self
            .path(requirement_id, &format!("history/{next}"))?
            .try_exists()
            .map_err(|_| inspect_error())?;
        let discussion = if recovery {
            None
        } else if let Some(id) = selected {
            if index["tasks"]
                .as_array()
                .expect("validated tasks")
                .iter()
                .any(|entry| entry["id"] == id && entry.get("draft_ref").is_some())
            {
                Some(self.read_draft(&index, id)?)
            } else {
                None
            }
        } else {
            None
        };
        let result = task_draft::status(
            requirement_id,
            Some(&index),
            task_id,
            discussion.as_ref(),
            recovery,
        )?;
        ensure_status_index_unchanged(&index, &self.read_planning_index(requirement_id)?)?;
        Ok(result)
    }

    pub fn check_sources(&self, request: &TaskSourceCheckRequest<'_>) -> Result<Value, WorkError> {
        let TaskSourceCheckRequest {
            requirement_id,
            task_id,
            expected_revision,
            plan_path,
            skill_root,
            skill_configs,
            selected_paths,
            reference_names,
        } = *request;
        let index = self.read_planning_index(requirement_id)?;
        let entry = task_draft::source_check_entry(&index, expected_revision, task_id)?;
        if entry.get("draft_ref").is_some() {
            self.read_draft(&index, task_id)?;
        }
        task_draft::resolve_instruction_selection(entry, selected_paths, reference_names)?;
        let paths = LocalPlanStorage {
            project_root: self.project_root.clone(),
        };
        let raw = paths.read(plan_path)?;
        let instructions = LocalHierarchyCatalog {
            skill_root: skill_root.to_path_buf(),
        };
        let skills = LocalSkillCatalog {
            roots: skill_configs.to_vec(),
        };
        let roots: Vec<SkillRoot> = skill_configs
            .iter()
            .map(|config| SkillRoot {
                scope: config.scope.clone(),
                locator: config.locator.clone(),
            })
            .collect();
        let validation =
            validate_plan_bytes(&instructions, &skills, &paths, &roots, &raw, plan_path)?;
        let plan = parse_json_contract(&raw).map_err(|_| {
            failure(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The Plan JSON is invalid.",
                json!({}),
            )
        })?;
        let result = check_validated_sources(
            &instructions,
            &DraftSourceCheck {
                index: &index,
                task_id,
                expected_revision,
                plan: &plan,
                plan_validation: &validation,
                selected_paths,
                reference_names,
            },
        )?;
        if self.read_planning_index(requirement_id)? != index {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_revision_conflict",
                "The planning index changed while its sources were checked.",
                json!({}),
            ));
        }
        Ok(result)
    }

    pub fn save_planning(
        &self,
        index: &Value,
        expected_revision: u64,
        draft: Option<&Value>,
    ) -> Result<Value, WorkError> {
        let requirement_id = index["requirement_id"].as_str().ok_or_else(|| {
            failure(
                ExitCode::Contract,
                "invalid_draft_schema",
                "A planning requirement is required.",
                json!({}),
            )
        })?;
        if expected_revision == 0 {
            self.require_initial_storage_free(requirement_id)?;
        }
        let current = self.path(requirement_id, "index.json")?;
        let previous = if current.exists() {
            Some(self.read_planning_index(requirement_id)?)
        } else {
            None
        };
        let prepared = prepare_save(index, expected_revision, previous.as_ref(), draft)?;
        let prefix = format!("history/{}", prepared.revision);
        let history = self.path(requirement_id, &prefix)?;
        let write_result = (|| -> Result<(), WorkError> {
            fs::create_dir_all(history.parent().expect("history has parent")).map_err(|_| failure(ExitCode::IoFailure, "draft_save_interrupted", "Save did not commit. Preserve history for recovery; the prior index remains authoritative.", json!({"revision": prepared.revision, "committed": false, "recovery_required": true})))?;
            fs::create_dir(&history).map_err(|_| failure(ExitCode::IoFailure, "draft_save_interrupted", "Save did not commit. Preserve history for recovery; the prior index remains authoritative.", json!({"revision": prepared.revision, "committed": false, "recovery_required": true})))?;
            if let (Some(id), Some(raw)) = (&prepared.task_id, &prepared.draft_raw) {
                Self::write_new(&history.join(format!("{id}.json")), raw).map_err(|_| failure(ExitCode::IoFailure, "draft_save_interrupted", "Save did not commit. Preserve history for recovery; the prior index remains authoritative.", json!({"revision": prepared.revision, "committed": false, "recovery_required": true})))?;
            }
            Self::write_new(&history.join("index.json"), &prepared.index_raw).map_err(|_| failure(ExitCode::IoFailure, "draft_save_interrupted", "Save did not commit. Preserve history for recovery; the prior index remains authoritative.", json!({"revision": prepared.revision, "committed": false, "recovery_required": true})))?;
            Self::write_new(&history.join("index-current.tmp"), &prepared.index_raw).map_err(|_| failure(ExitCode::IoFailure, "draft_save_interrupted", "Save did not commit. Preserve history for recovery; the prior index remains authoritative.", json!({"revision": prepared.revision, "committed": false, "recovery_required": true})))?;
            if let Some(previous) = &previous {
                if self.read_planning_index(requirement_id)? != *previous {
                    return Err(failure(
                        ExitCode::WorkflowState,
                        "draft_revision_conflict",
                        "The committed index changed during save.",
                        json!({}),
                    ));
                }
            } else if current.exists() {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_revision_conflict",
                    "Another initial index appeared during save.",
                    json!({}),
                ));
            }
            fs::rename(history.join("index-current.tmp"), &current).map_err(|_| failure(ExitCode::IoFailure, "draft_save_interrupted", "Save did not commit. Preserve history for recovery; the prior index remains authoritative.", json!({"revision": prepared.revision, "committed": false, "recovery_required": true})))?;
            Ok(())
        })();
        write_result?;
        let mirror = if let (Some(id), Some(raw)) = (&prepared.task_id, &prepared.draft_raw) {
            let mirror_result = (|| -> Result<&'static str, WorkError> {
                let staged = history.join(format!("{id}-latest.tmp"));
                Self::write_new(&staged, raw).map_err(|_| {
                    failure(
                        ExitCode::IoFailure,
                        "draft_mirror_failed",
                        "The display copy could not be staged.",
                        json!({}),
                    )
                })?;
                if self.read_planning_index(requirement_id)?["revision"].as_u64()
                    != Some(prepared.revision)
                {
                    return Ok("superseded");
                }
                fs::rename(staged, self.path(requirement_id, &format!("{id}.json"))?).map_err(
                    |_| {
                        failure(
                            ExitCode::IoFailure,
                            "draft_mirror_failed",
                            "The display copy could not be updated.",
                            json!({}),
                        )
                    },
                )?;
                Ok("updated")
            })();
            mirror_result.unwrap_or("stale")
        } else {
            "not_applicable"
        };
        Ok(work_model::task::response::typed_response::<
            work_model::task::response::TaskDraftSave,
        >(
            json!({"schema":"work-task-draft-save/v1","requirement_id":requirement_id,"revision":prepared.revision,"status":"saved","mirror_status":mirror}),
        ))
    }

    pub fn update_planning_list(
        &self,
        proposed: &Value,
        expected_revision: u64,
        reason: &str,
        recover: bool,
    ) -> Result<Value, WorkError> {
        let requirement_id = proposed["requirement_id"].as_str().ok_or_else(|| {
            failure(
                ExitCode::Contract,
                "invalid_draft_schema",
                "A planning requirement is required.",
                json!({}),
            )
        })?;
        let current = self.read_planning_index(requirement_id)?;
        let previous = if recover {
            self.read_planning_revision(requirement_id, expected_revision)?
        } else {
            current.clone()
        };
        let prepared = prepare_list_update(self, &previous, proposed, expected_revision, reason)?;
        let history = self.path(requirement_id, &format!("history/{}", prepared.revision))?;
        let current_path = self.path(requirement_id, "index.json")?;
        let expected: BTreeSet<String> = prepared.files.keys().cloned().collect();
        if recover {
            if current != previous && current != prepared.index {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_revision_conflict",
                    "Recovery cannot replace a newer or different index.",
                    json!({}),
                ));
            }
            if !history.is_dir() {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_recovery_incomplete",
                    "The prepared list update is missing.",
                    json!({}),
                ));
            }
            let observed = Self::history_names(&history)?;
            let mut required = expected.clone();
            if current == prepared.index {
                required.remove("index-current.tmp");
            }
            if !observed.is_subset(&expected) || !required.is_subset(&observed) {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_recovery_incomplete",
                    "Recovery requires the exact complete list-update file set.",
                    json!({}),
                ));
            }
            for name in &observed {
                if self.read(&history.join(name))? != prepared.files[name] {
                    return Err(failure(
                        ExitCode::WorkflowState,
                        "draft_recovery_conflict",
                        "Preserved content differs from the authorized list update.",
                        json!({}),
                    ));
                }
            }
            if current == prepared.index {
                return Ok(
                    json!({"schema":"work-task-draft-list-update/v1","status":"already_completed","revision":prepared.revision,"affected_task_ids":prepared.affected_task_ids}),
                );
            }
        } else {
            let write_result = (|| -> std::io::Result<()> {
                fs::create_dir(&history)?;
                for (name, content) in &prepared.files {
                    Self::write_new(&history.join(name), content)?;
                }
                Ok(())
            })();
            write_result.map_err(|_| {
                failure(
                    ExitCode::IoFailure,
                    "draft_list_update_interrupted",
                    "The list update was not committed; preserve its history for recovery.",
                    json!({"recovery_required":true}),
                )
            })?;
        }
        if self.read_planning_index(requirement_id)? != previous {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_revision_conflict",
                "The current index changed before list publication.",
                json!({}),
            ));
        }
        if Self::history_names(&history)? != expected {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_recovery_conflict",
                "The prepared file set changed before list publication.",
                json!({}),
            ));
        }
        for (name, content) in &prepared.files {
            if self.read(&history.join(name))? != *content {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_recovery_conflict",
                    "Prepared content changed before list publication.",
                    json!({}),
                ));
            }
        }
        fs::rename(history.join("index-current.tmp"), &current_path).map_err(|_| {
            failure(
                ExitCode::IoFailure,
                "draft_list_update_interrupted",
                "The prepared list update could not be published.",
                json!({"recovery_required":true}),
            )
        })?;
        Ok(
            json!({"schema":"work-task-draft-list-update/v1","status":if recover {"recovered"} else {"saved"},
            "requirement_id":requirement_id,"revision":prepared.revision,"affected_task_ids":prepared.affected_task_ids,"display_copies":"not_updated"}),
        )
    }

    pub fn update_planning_sources(
        &self,
        requirement_id: &str,
        request: &SourceUpdateRequest<'_>,
        recover: bool,
    ) -> Result<Value, WorkError> {
        let current = self.read_planning_index(requirement_id)?;
        let previous = if recover {
            self.read_planning_revision(requirement_id, request.expected_revision)?
        } else {
            current.clone()
        };
        let prepared = prepare_source_update(self, &previous, request)?;
        self.publish_source_update(requirement_id, &previous, &current, &prepared, recover)
    }

    fn publish_source_update(
        &self,
        requirement_id: &str,
        previous: &Value,
        current: &Value,
        prepared: &PreparedListUpdate,
        recover: bool,
    ) -> Result<Value, WorkError> {
        let history = self.path(requirement_id, &format!("history/{}", prepared.revision))?;
        let current_path = self.path(requirement_id, "index.json")?;
        let expected: BTreeSet<String> = prepared.files.keys().cloned().collect();
        if recover {
            if current != previous && *current != prepared.index {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_revision_conflict",
                    "Source recovery cannot replace a changed index.",
                    json!({}),
                ));
            }
            if !history.is_dir() {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_recovery_incomplete",
                    "Prepared source update is missing.",
                    json!({}),
                ));
            }
            let observed = Self::history_names(&history)?;
            let mut required = expected.clone();
            if *current == prepared.index {
                required.remove("index-current.tmp");
            }
            if !observed.is_subset(&expected) || !required.is_subset(&observed) {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_recovery_incomplete",
                    "The complete source-update file set is required.",
                    json!({}),
                ));
            }
            for name in &observed {
                if self.read(&history.join(name))? != prepared.files[name] {
                    return Err(failure(
                        ExitCode::WorkflowState,
                        "draft_recovery_conflict",
                        "Preserved content differs from the revalidated source update.",
                        json!({}),
                    ));
                }
            }
            if *current == prepared.index {
                return Ok(
                    json!({"schema":"work-task-draft-source-update/v1","status":"already_completed","revision":prepared.revision,"affected_task_ids":prepared.affected_task_ids}),
                );
            }
        } else {
            let write_result = (|| -> std::io::Result<()> {
                fs::create_dir(&history)?;
                for (name, content) in &prepared.files {
                    Self::write_new(&history.join(name), content)?;
                }
                Ok(())
            })();
            write_result.map_err(|_| {
                failure(
                    ExitCode::IoFailure,
                    "draft_source_update_interrupted",
                    "Preserve the uncommitted source update for recovery.",
                    json!({"recovery_required":true}),
                )
            })?;
        }
        if self.read_planning_index(requirement_id)? != *previous {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_revision_conflict",
                "The planning index changed before source publication.",
                json!({}),
            ));
        }
        if Self::history_names(&history)? != expected {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_recovery_conflict",
                "The prepared file set changed before publication.",
                json!({}),
            ));
        }
        for (name, content) in &prepared.files {
            if self.read(&history.join(name))? != *content {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_recovery_conflict",
                    "The prepared source-update bytes changed.",
                    json!({}),
                ));
            }
        }
        fs::rename(history.join("index-current.tmp"), current_path).map_err(|_| {
            failure(
                ExitCode::IoFailure,
                "draft_source_update_interrupted",
                "The prepared source update could not be published.",
                json!({"recovery_required":true}),
            )
        })?;
        Ok(
            json!({"schema":"work-task-draft-source-update/v1","status":if recover {"recovered"} else {"saved"},
            "requirement_id":requirement_id,"revision":prepared.revision,"affected_task_ids":prepared.affected_task_ids,"display_copies":"not_updated"}),
        )
    }

    fn history_names(history: &Path) -> Result<BTreeSet<String>, WorkError> {
        fs::read_dir(history)
            .map_err(|_| {
                failure(
                    ExitCode::IoFailure,
                    "draft_read_failed",
                    "The draft history could not be inspected.",
                    json!({}),
                )
            })?
            .map(|entry| entry.map(|item| item.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()
            .map_err(|_| {
                failure(
                    ExitCode::IoFailure,
                    "draft_read_failed",
                    "The draft history could not be inspected.",
                    json!({}),
                )
            })
    }

    pub fn recover_planning(
        &self,
        index: &Value,
        expected_revision: u64,
        draft: Option<&Value>,
    ) -> Result<Value, WorkError> {
        let requirement_id = index["requirement_id"].as_str().ok_or_else(|| {
            failure(
                ExitCode::Contract,
                "invalid_draft_schema",
                "A planning requirement is required.",
                json!({}),
            )
        })?;
        let previous = if expected_revision == 0 {
            None
        } else {
            Some(self.read_planning_revision(requirement_id, expected_revision)?)
        };
        let prepared = prepare_save(index, expected_revision, previous.as_ref(), draft)?;
        let history = self.path(requirement_id, &format!("history/{}", prepared.revision))?;
        if !history.is_dir() {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_recovery_incomplete",
                "The prepared history directory is missing.",
                json!({}),
            ));
        }
        let mut expected = std::collections::BTreeMap::from([
            ("index.json".to_owned(), prepared.index_raw.as_slice()),
            (
                "index-current.tmp".to_owned(),
                prepared.index_raw.as_slice(),
            ),
        ]);
        if let (Some(id), Some(raw)) = (&prepared.task_id, &prepared.draft_raw) {
            expected.insert(format!("{id}.json"), raw);
            expected.insert(format!("{id}-latest.tmp"), raw);
        }
        let observed: std::collections::BTreeSet<String> = fs::read_dir(&history)
            .map_err(|_| {
                failure(
                    ExitCode::IoFailure,
                    "draft_read_failed",
                    "The draft artifact could not be read.",
                    json!({}),
                )
            })?
            .map(|entry| entry.map(|item| item.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()
            .map_err(|_| {
                failure(
                    ExitCode::IoFailure,
                    "draft_read_failed",
                    "The draft artifact could not be read.",
                    json!({}),
                )
            })?;
        if observed.iter().any(|name| !expected.contains_key(name)) {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_recovery_conflict",
                "The history directory contains unknown files.",
                json!({}),
            ));
        }
        let mut required = std::collections::BTreeSet::from(["index.json".to_owned()]);
        if let Some(id) = &prepared.task_id {
            required.insert(format!("{id}.json"));
        }
        if !required.is_subset(&observed) {
            return Err(failure(
                ExitCode::WorkflowState,
                "draft_recovery_incomplete",
                "The historical index or draft is incomplete.",
                json!({}),
            ));
        }
        for name in &observed {
            if self.read(&history.join(name))? != expected[name] {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_recovery_conflict",
                    "Preserved bytes differ from the original save request.",
                    json!({}),
                ));
            }
        }
        let current_path = self.path(requirement_id, "index.json")?;
        let current = if current_path.exists() {
            Some(self.read_planning_index(requirement_id)?)
        } else {
            None
        };
        let status = if current.as_ref() == Some(&prepared.index) {
            "already_completed"
        } else {
            if current != previous {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_revision_conflict",
                    "Recovery cannot replace a changed or newer index.",
                    json!({}),
                ));
            }
            if !observed.contains("index-current.tmp") {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_recovery_incomplete",
                    "The complete prepared index is required for recovery.",
                    json!({}),
                ));
            }
            let again: std::collections::BTreeSet<String> = fs::read_dir(&history)
                .map_err(|_| {
                    failure(
                        ExitCode::IoFailure,
                        "draft_read_failed",
                        "The draft artifact could not be read.",
                        json!({}),
                    )
                })?
                .map(|entry| entry.map(|item| item.file_name().to_string_lossy().into_owned()))
                .collect::<Result<_, _>>()
                .map_err(|_| {
                    failure(
                        ExitCode::IoFailure,
                        "draft_read_failed",
                        "The draft artifact could not be read.",
                        json!({}),
                    )
                })?;
            if again != observed
                || observed.iter().any(|name| {
                    self.read(&history.join(name))
                        .map_or(true, |raw| raw != expected[name])
                })
            {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_recovery_conflict",
                    "Prepared content changed during recovery.",
                    json!({}),
                ));
            }
            let latest = if current_path.exists() {
                Some(self.read_planning_index(requirement_id)?)
            } else {
                None
            };
            if latest != previous {
                return Err(failure(
                    ExitCode::WorkflowState,
                    "draft_revision_conflict",
                    "The current index changed during recovery.",
                    json!({}),
                ));
            }
            fs::rename(history.join("index-current.tmp"), &current_path).map_err(|_| failure(ExitCode::IoFailure, "draft_recovery_interrupted", "Recovery could not publish the index; preserve the current state for inspection.", json!({"revision": prepared.revision, "recovery_required": true})))?;
            "recovered"
        };
        Ok(work_model::task::response::typed_response::<
            work_model::task::response::TaskDraftRecovery,
        >(
            json!({"schema":"work-task-draft-recovery/v1","requirement_id":requirement_id,"revision":prepared.revision,"status":status,"display_copy":if draft.is_some() {"not_updated"} else {"not_applicable"}}),
        ))
    }
}

impl TaskDraftHistoryRepository for LocalTaskDraftStorage {
    fn read_historical_draft(
        &self,
        requirement_id: &str,
        save_revision: u64,
        task_id: &str,
    ) -> Result<Vec<u8>, WorkError> {
        self.read(&self.path(
            requirement_id,
            &format!("history/{save_revision}/{task_id}.json"),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use work_operations::canonical::sha256_hex;

    #[test]
    fn concurrent_status_index_change_keeps_revision_conflict() {
        let original = json!({"revision":1,"current_task_id":"TASK-001"});
        let changed = json!({"revision":2,"current_task_id":"TASK-001"});
        ensure_status_index_unchanged(&original, &original).unwrap();
        let error = ensure_status_index_unchanged(&original, &changed).unwrap_err();
        assert_eq!(error.exit_code, ExitCode::WorkflowState);
        assert_eq!(error.reason_code, "draft_revision_conflict");
    }

    #[test]
    fn source_update_publishes_and_recovers_approved_file_set() {
        let root = std::env::temp_dir().join(format!(
            "work-draft-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalTaskDraftStorage { project_root: root };
        let source = json!({"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)});
        let previous = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,"source":source,
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Task","goal":"Goal","scope":["Scope"],"skill_id":null,
            "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        storage.save_planning(&previous, 0, None).unwrap();
        let selections = std::collections::BTreeMap::from([(
            "TASK-001".into(),
            json!({"selected_paths":[],"references":[]}),
        )]);
        let hashes = std::collections::BTreeMap::from([("TASK-001".into(), "e".repeat(64))]);
        let request = SourceUpdateRequest {
            new_source: &source,
            selections: &selections,
            instruction_hashes: &hashes,
            expected_revision: 1,
            reason: "Reviewed.",
            plan_path: "outputs/work/plans/example.json",
        };
        let saved = storage
            .update_planning_sources("example", &request, false)
            .unwrap();
        assert_eq!(saved["status"], "saved");
        let saved_index = storage.read_planning_index("example").unwrap();
        assert_eq!(
            saved_index["tasks"][0]["instruction_selection"],
            selections["TASK-001"]
        );
        assert!(
            storage.read_planning_revision("example", 1).unwrap()["tasks"][0]
                .get("instruction_selection")
                .is_none()
        );
        assert_eq!(
            storage
                .update_planning_sources("example", &request, false)
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
        assert_eq!(
            storage.read_planning_index("example").unwrap()["tasks"][0]["boundary_revision"],
            2
        );
        assert_eq!(
            storage
                .update_planning_sources("example", &request, true)
                .unwrap()["status"],
            "already_completed"
        );
        let committed = storage.read_planning_index("example").unwrap();
        let next_hashes = std::collections::BTreeMap::from([("TASK-001".into(), "f".repeat(64))]);
        let staged = prepare_source_update(
            &storage,
            &committed,
            &SourceUpdateRequest {
                new_source: &source,
                selections: &selections,
                instruction_hashes: &next_hashes,
                expected_revision: 2,
                reason: "Reviewed again.",
                plan_path: "outputs/work/plans/example.json",
            },
        )
        .unwrap();
        let history = storage.path("example", "history/3").unwrap();
        fs::create_dir(&history).unwrap();
        for (name, raw) in &staged.files {
            LocalTaskDraftStorage::write_new(&history.join(name), raw).unwrap();
        }
        assert_eq!(
            storage.read_planning_index("example").unwrap()["revision"],
            2
        );
        let changed_source = json!({"plan_sha256":"9".repeat(64),
            "hierarchy_selection_sha256":source["hierarchy_selection_sha256"],
            "skill_selection_sha256":source["skill_selection_sha256"]});
        assert_eq!(
            storage
                .update_planning_sources(
                    "example",
                    &SourceUpdateRequest {
                        new_source: &changed_source,
                        selections: &selections,
                        instruction_hashes: &next_hashes,
                        expected_revision: 2,
                        reason: "Reviewed again.",
                        plan_path: "outputs/work/plans/example.json"
                    },
                    true
                )
                .unwrap_err()
                .reason_code,
            "draft_recovery_conflict"
        );
        assert_eq!(storage.read_planning_index("example").unwrap(), committed);
        assert_eq!(
            storage
                .update_planning_sources(
                    "example",
                    &SourceUpdateRequest {
                        new_source: &source,
                        selections: &selections,
                        instruction_hashes: &next_hashes,
                        expected_revision: 2,
                        reason: "Reviewed again.",
                        plan_path: "outputs/work/plans/example.json"
                    },
                    true
                )
                .unwrap()["status"],
            "recovered"
        );
        assert_eq!(
            storage.read_planning_index("example").unwrap()["revision"],
            3
        );
        assert_eq!(
            storage
                .update_planning_sources(
                    "example",
                    &SourceUpdateRequest {
                        new_source: &source,
                        selections: &selections,
                        instruction_hashes: &next_hashes,
                        expected_revision: 2,
                        reason: "Reviewed again.",
                        plan_path: "outputs/work/plans/example.json"
                    },
                    true
                )
                .unwrap()["status"],
            "already_completed"
        );
        let current = storage.read_planning_index("example").unwrap();
        let instruction_catalog = crate::hierarchy_catalog::LocalHierarchyCatalog {
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
        };
        let plan =
            json!({"skill_selection":{"skills":[]},"hierarchy_selection":{"selected_paths":[]}});
        let validation = json!({"requirement_id":"example","plan_sha256":source["plan_sha256"],
            "hierarchy_selection_sha256":source["hierarchy_selection_sha256"],"skill_selection_sha256":source["skill_selection_sha256"]});
        let refreshed = work_feature::task::draft::prepare_validated_source_refresh(
            &instruction_catalog,
            &storage,
            &work_feature::task::draft::ValidatedSourceRefresh {
                previous: &current,
                plan: &plan,
                plan_validation: &validation,
                selections: &selections,
                expected_revision: 3,
                reason: "Instruction sources reviewed.",
                plan_path: "outputs/work/plans/example.json",
            },
        )
        .unwrap();
        assert_eq!(refreshed.revision, 4);
        assert_ne!(
            refreshed.index["tasks"][0]["instructions_sha256"],
            "f".repeat(64)
        );
        let mut unknown_skill = current.clone();
        unknown_skill["tasks"][0]["skill_id"] = json!("missing");
        assert_eq!(
            work_feature::task::draft::prepare_validated_source_refresh(
                &instruction_catalog,
                &storage,
                &work_feature::task::draft::ValidatedSourceRefresh {
                    previous: &unknown_skill,
                    plan: &plan,
                    plan_validation: &validation,
                    selections: &selections,
                    expected_revision: 3,
                    reason: "Reviewed.",
                    plan_path: "outputs/work/plans/example.json",
                }
            )
            .expect_err("unknown skill must fail")
            .reason_code,
            "draft_skill_not_available"
        );
        assert_eq!(
            work_feature::task::draft::prepare_validated_source_refresh(
                &instruction_catalog,
                &storage,
                &work_feature::task::draft::ValidatedSourceRefresh {
                    previous: &current,
                    plan: &plan,
                    plan_validation: &validation,
                    selections: &BTreeMap::new(),
                    expected_revision: 3,
                    reason: "Reviewed.",
                    plan_path: "outputs/work/plans/example.json",
                }
            )
            .expect_err("missing selection must fail")
            .reason_code,
            "invalid_object_fields"
        );
    }

    #[test]
    fn list_update_publishes_history_and_recovery_recognizes_commit() {
        let root = std::env::temp_dir().join(format!(
            "work-draft-list-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalTaskDraftStorage { project_root: root };
        let previous = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Before","goal":"Goal","scope":["Scope"],"skill_id":null,
            "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        storage.save_planning(&previous, 0, None).unwrap();
        let mut proposed = previous.clone();
        proposed["revision"] = json!(2);
        proposed["tasks"][0]["title"] = json!("After");
        let saved = storage
            .update_planning_list(&proposed, 1, "Title reviewed.", false)
            .unwrap();
        assert_eq!(saved["status"], "saved");
        assert_eq!(saved["affected_task_ids"], json!(["TASK-001"]));
        assert_eq!(
            sha256_hex(
                &storage
                    .read(&storage.path("example", "history/2/index.json").unwrap())
                    .unwrap()
            ),
            "bbc27c8a855a123fa1184dd645f36198976cc24ca3d7f4ecc7efb9b85391ef1a"
        );
        assert_eq!(
            storage.read_planning_index("example").unwrap()["tasks"][0]["boundary_revision"],
            2
        );
        assert_eq!(
            storage
                .update_planning_list(&proposed, 1, "Title reviewed.", true)
                .unwrap()["status"],
            "already_completed"
        );
        let committed = storage.read_planning_index("example").unwrap();
        let mut next = committed.clone();
        next["revision"] = json!(3);
        next["tasks"][0]["title"] = json!("Final title");
        let staged = prepare_list_update(&storage, &committed, &next, 2, "Final review.").unwrap();
        let history = storage.path("example", "history/3").unwrap();
        fs::create_dir(&history).unwrap();
        for (name, raw) in &staged.files {
            LocalTaskDraftStorage::write_new(&history.join(name), raw).unwrap();
        }
        assert_eq!(
            storage.read_planning_index("example").unwrap()["revision"],
            2
        );
        assert_eq!(
            storage
                .update_planning_list(&next, 2, "Final review.", true)
                .unwrap()["status"],
            "recovered"
        );
        assert_eq!(
            storage.read_planning_index("example").unwrap()["revision"],
            3
        );
        assert_eq!(
            storage
                .update_planning_list(&next, 2, "Final review.", false)
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
        assert_eq!(
            storage
                .update_planning_list(&next, 2, "Final review.", true)
                .unwrap()["status"],
            "already_completed"
        );
    }

    #[test]
    fn committed_index_requires_identical_history_and_reports_progress() {
        let root = std::env::temp_dir().join(format!(
            "work-draft-read-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalTaskDraftStorage {
            project_root: root.clone(),
        };
        let absent = storage.path("example", "index.json").unwrap();
        let empty = storage.status("example", None).unwrap();
        assert_eq!(empty["status"], "not_initialized");
        assert_eq!(empty["next_action"], "confirm_task_list");
        assert_eq!(empty["requires_user_confirmation"], true);
        assert!(!absent.exists());
        assert_eq!(
            storage
                .status("example", Some("TASK-001"))
                .unwrap_err()
                .reason_code,
            "draft_task_not_in_index"
        );
        let residue = storage.path("example", "history/1").unwrap();
        fs::create_dir_all(&residue).unwrap();
        let pending = storage.status("example", Some("TASK-001")).unwrap();
        assert_eq!(pending["status"], "recovery_required");
        assert_eq!(pending["next_action"], "inspect_recovery");
        assert!(!absent.exists());
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Task","goal":"Goal","scope":["Scope"],"skill_id":null,
            "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        let raw = canonical_json(&index).unwrap();
        let current = storage.path("example", "index.json").unwrap();
        let history = storage.path("example", "history/1/index.json").unwrap();
        fs::create_dir_all(current.parent().unwrap()).unwrap();
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        fs::write(&current, &raw).unwrap();
        fs::write(&history, &raw).unwrap();
        assert_eq!(
            storage
                .status("example", Some("TASK-999"))
                .unwrap_err()
                .reason_code,
            "draft_task_not_in_index"
        );
        fs::create_dir_all(storage.path("example", "history/2").unwrap()).unwrap();
        let pending = storage.status("example", None).unwrap();
        assert_eq!(pending["status"], "recovery_required");
        assert_eq!(pending["discussion"], Value::Null);
        assert_eq!(
            storage.status("example", None).unwrap()["next_action"],
            "inspect_recovery"
        );
        fs::write(&history, b"different\n").unwrap();
        assert_eq!(
            storage
                .read_planning_index("example")
                .unwrap_err()
                .reason_code,
            "draft_index_integrity"
        );
    }

    #[test]
    fn malformed_index_is_an_error_instead_of_uninitialized_progress() {
        let root = std::env::temp_dir().join(format!(
            "work-draft-invalid-status-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalTaskDraftStorage { project_root: root };
        let index = storage.path("example", "index.json").unwrap();
        fs::create_dir_all(index.parent().unwrap()).unwrap();
        fs::write(&index, b"broken").unwrap();
        assert!(storage.status("example", None).is_err());
        assert_eq!(fs::read(&index).unwrap(), b"broken");
    }

    #[test]
    fn status_reads_selected_history_and_ignores_display_copy() {
        let root = std::env::temp_dir().join(format!(
            "work-draft-selected-status-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalTaskDraftStorage { project_root: root };
        let source = json!({"plan_sha256":"a".repeat(64),
            "hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)});
        let draft = |task_id: &str| {
            json!({"schema":"work-task-draft/v1","requirement_id":"example",
                "task_id":task_id,"revision":1,"boundary_revision":1,"source":source,
                "instructions_sha256":"d".repeat(64),"status":"in_progress","notes":["Discussion"],
                "confirmed_decisions":[],"tentative":[],"open_questions":["Which test?"],
                "next_discussion_point":"Confirm test."})
        };
        let first_raw = canonical_json(&draft("TASK-001")).unwrap();
        let second_raw = canonical_json(&draft("TASK-002")).unwrap();
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":3,
            "source":source,"current_task_id":"TASK-001","tasks":[
                {"id":"TASK-001","title":"Task","goal":"Result","scope":["Source"],"skill_id":null,
                 "dependencies":[],"status":"in_progress","boundary_revision":1,"instructions_sha256":"d".repeat(64),
                 "draft_ref":{"save_revision":2,"revision":1,"sha256":sha256_hex(&first_raw)}},
                {"id":"TASK-002","title":"Task","goal":"Result","scope":["Source"],"skill_id":null,
                 "dependencies":[],"status":"in_progress","boundary_revision":1,"instructions_sha256":"d".repeat(64),
                 "draft_ref":{"save_revision":3,"revision":1,"sha256":sha256_hex(&second_raw)}}]});
        let index_raw = canonical_json(&index).unwrap();
        for (name, raw) in [
            ("index.json", index_raw.as_slice()),
            ("history/3/index.json", index_raw.as_slice()),
            ("history/2/TASK-001.json", first_raw.as_slice()),
            (
                "history/3/TASK-002.json",
                b"unrelated corrupt discussion".as_slice(),
            ),
            ("TASK-001.json", b"stale display copy".as_slice()),
        ] {
            let path = storage.path("example", name).unwrap();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, raw).unwrap();
        }
        let result = storage.status("example", None).unwrap();
        assert_eq!(result["discussion"]["notes"], json!(["Discussion"]));
        assert_eq!(
            storage
                .status("example", Some("TASK-002"))
                .unwrap_err()
                .reason_code,
            "draft_content_integrity"
        );
    }

    #[test]
    fn unreadable_index_is_not_reported_as_missing() {
        let root = std::env::temp_dir().join(format!(
            "work-draft-unreadable-status-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalTaskDraftStorage { project_root: root };
        let index = storage.path("example", "index.json").unwrap();
        fs::create_dir_all(&index).unwrap();
        assert!(storage.status("example", None).is_err());
        assert!(index.is_dir());
    }

    #[test]
    fn exclusive_save_commits_history_then_current_index() {
        let root = std::env::temp_dir().join(format!(
            "work-draft-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalTaskDraftStorage { project_root: root };
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Task","goal":"Goal","scope":["Scope"],"skill_id":null,
            "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        assert_eq!(
            storage.save_planning(&index, 0, None).unwrap()["mirror_status"],
            "not_applicable"
        );
        assert_eq!(
            sha256_hex(
                &storage
                    .read(&storage.path("example", "index.json").unwrap())
                    .unwrap()
            ),
            "4690d87a1d28ac54875218eb02a69841c78ea99fcda22b60882377ce84336d0b"
        );
        let mut next = index.clone();
        next["revision"] = json!(2);
        next["tasks"][0]["status"] = json!("in_progress");
        let draft = json!({"schema":"work-task-draft/v1","requirement_id":"example","task_id":"TASK-001","revision":1,
            "boundary_revision":1,"source":index["source"],"instructions_sha256":"d".repeat(64),"status":"in_progress",
            "notes":[],"confirmed_decisions":[],"tentative":[],"open_questions":[],"next_discussion_point":"Continue discussion."});
        assert_eq!(
            storage.save_planning(&next, 1, Some(&draft)).unwrap()["mirror_status"],
            "updated"
        );
        assert_eq!(
            sha256_hex(
                &storage
                    .read(&storage.path("example", "history/2/TASK-001.json").unwrap())
                    .unwrap()
            ),
            "2853a6cf2b946cb82ec72c27c535f18bef1477895657fece1ca4e0a8fd0379ba"
        );
        assert_eq!(
            storage.status("example", None).unwrap()["next_action"],
            "confirm_resume"
        );
        assert_eq!(
            storage.read_planning_index("example").unwrap()["revision"],
            2
        );
        assert_eq!(storage.read_planning_revision("example", 1).unwrap(), index);
        assert_eq!(
            storage.read_planning_revision("example", 2).unwrap(),
            storage.read_planning_index("example").unwrap()
        );
        assert_eq!(
            storage
                .read_planning_revision("example", 0)
                .unwrap_err()
                .reason_code,
            "invalid_expected_revision"
        );
        let first_history = storage.path("example", "history/1/index.json").unwrap();
        let first_raw = fs::read(&first_history).unwrap();
        for (field, value, reason) in [
            (
                "requirement_id",
                json!("other"),
                "draft_requirement_mismatch",
            ),
            ("revision", json!(2), "draft_revision_conflict"),
        ] {
            let mut changed = index.clone();
            changed[field] = value;
            fs::write(&first_history, canonical_json(&changed).unwrap()).unwrap();
            assert_eq!(
                storage
                    .read_planning_revision("example", 1)
                    .unwrap_err()
                    .reason_code,
                reason
            );
        }
        fs::write(&first_history, first_raw).unwrap();
        let current = storage.path("example", "index.json").unwrap();
        let before = fs::read(&current).unwrap();
        assert_eq!(
            storage
                .save_planning(&next, 1, Some(&draft))
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
        assert_eq!(fs::read(&current).unwrap(), before);
        assert_eq!(
            storage.recover_planning(&next, 1, Some(&draft)).unwrap()["status"],
            "already_completed"
        );
        let draft_path = storage.path("example", "history/2/TASK-001.json").unwrap();
        fs::write(draft_path, b"corrupt draft\n").unwrap();
        fs::create_dir_all(storage.path("example", "history/3").unwrap()).unwrap();
        let pending = storage.status("example", None).unwrap();
        assert_eq!(pending["status"], "recovery_required");
        assert_eq!(pending["discussion"], Value::Null);
        assert_eq!(pending["next_action"], "inspect_recovery");
    }

    #[test]
    fn saved_selection_boundary_and_history_stay_immutable() {
        let root = std::env::temp_dir().join(format!(
            "work-draft-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalTaskDraftStorage { project_root: root };
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Task","goal":"Goal","scope":["Scope"],"skill_id":null,
            "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64),
            "instruction_selection":{"selected_paths":[],"references":[]}}]});
        storage.save_planning(&index, 0, None).unwrap();
        let mut next = index.clone();
        next["revision"] = json!(2);
        next["tasks"][0]["status"] = json!("in_progress");
        let draft = json!({"schema":"work-task-draft/v1","requirement_id":"example","task_id":"TASK-001","revision":1,
            "boundary_revision":1,"source":index["source"],"instructions_sha256":"d".repeat(64),"status":"in_progress",
            "notes":[],"confirmed_decisions":[],"tentative":[],"open_questions":[],"next_discussion_point":"Continue discussion."});
        let current_path = storage.path("example", "index.json").unwrap();
        let original = fs::read(&current_path).unwrap();
        for altered in [
            Value::Null,
            json!({"selected_paths":[],"references":["other"]}),
        ] {
            let mut invalid = next.clone();
            if altered.is_null() {
                invalid["tasks"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("instruction_selection");
            } else {
                invalid["tasks"][0]["instruction_selection"] = altered;
            }
            assert_eq!(
                storage
                    .save_planning(&invalid, 1, Some(&draft))
                    .unwrap_err()
                    .reason_code,
                "draft_selection_mismatch"
            );
        }
        let mut changed_boundary = next.clone();
        changed_boundary["tasks"][0]["goal"] = json!("Changed goal.");
        assert_eq!(
            storage
                .save_planning(&changed_boundary, 1, Some(&draft))
                .unwrap_err()
                .reason_code,
            "draft_boundary_conflict"
        );
        assert_eq!(fs::read(&current_path).unwrap(), original);
        assert!(!storage.path("example", "history/2").unwrap().exists());

        let display = storage.path("example", "TASK-001.json").unwrap();
        fs::create_dir(&display).unwrap();
        assert_eq!(
            storage.save_planning(&next, 1, Some(&draft)).unwrap()["mirror_status"],
            "stale"
        );
        let first_draft = storage.path("example", "history/2/TASK-001.json").unwrap();
        let first_raw = fs::read(&first_draft).unwrap();
        let mut third = storage.read_planning_index("example").unwrap();
        third["revision"] = json!(3);
        let mut second_draft = draft.clone();
        second_draft["revision"] = json!(2);
        second_draft["notes"] = json!(["More detail."]);
        storage
            .save_planning(&third, 2, Some(&second_draft))
            .unwrap();
        assert_eq!(fs::read(&first_draft).unwrap(), first_raw);
        let committed = storage.read_planning_index("example").unwrap();
        assert_eq!(
            storage.read_draft(&committed, "TASK-001").unwrap(),
            second_draft
        );
        assert_eq!(
            storage.read_planning_revision("example", 2).unwrap()["revision"],
            2
        );
        let latest = storage.path("example", "history/3/TASK-001.json").unwrap();
        fs::write(&latest, b"changed").unwrap();
        assert_eq!(
            storage
                .read_draft(&committed, "TASK-001")
                .unwrap_err()
                .reason_code,
            "draft_content_integrity"
        );
        assert_eq!(storage.read_planning_index("example").unwrap(), committed);
    }

    #[cfg(unix)]
    #[test]
    fn invalid_requirement_and_linked_draft_path_are_rejected_before_writes() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "work-draft-link-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalTaskDraftStorage {
            project_root: root.clone(),
        };
        for requirement in ["../escape", "C:/escape", "con"] {
            assert!(storage.read_planning_index(requirement).is_err());
        }
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::create_dir_all(root.join("outputs/work/tasks")).unwrap();
        symlink(&outside, root.join("outputs/work/tasks/example")).unwrap();
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Task","goal":"Goal","scope":["Scope"],"skill_id":null,
            "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        assert_eq!(
            storage
                .save_planning(&index, 0, None)
                .unwrap_err()
                .reason_code,
            "draft_path_link"
        );
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    }

    #[test]
    fn recovery_publishes_only_preserved_approved_bytes() {
        let root = std::env::temp_dir().join(format!(
            "work-draft-recover-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let storage = LocalTaskDraftStorage { project_root: root };
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":"a".repeat(64),"hierarchy_selection_sha256":"b".repeat(64),"skill_selection_sha256":"c".repeat(64)},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Task","goal":"Goal","scope":["Scope"],"skill_id":null,
            "dependencies":[],"status":"planned","boundary_revision":1,"instructions_sha256":"d".repeat(64)}]});
        let history = storage.path("example", "history/1/index.json").unwrap();
        fs::create_dir_all(history.parent().unwrap()).unwrap();
        let raw = canonical_json(&index).unwrap();
        fs::write(&history, &raw).unwrap();
        assert_eq!(
            storage
                .recover_planning(&index, 0, None)
                .unwrap_err()
                .reason_code,
            "draft_recovery_incomplete"
        );
        fs::write(history.parent().unwrap().join("index-current.tmp"), &raw).unwrap();
        let mut changed = index.clone();
        changed["tasks"][0]["title"] = json!("Changed task");
        assert_eq!(
            storage
                .recover_planning(&changed, 0, None)
                .unwrap_err()
                .reason_code,
            "draft_recovery_conflict"
        );
        assert!(!storage.path("example", "index.json").unwrap().exists());
        let temporary = history.parent().unwrap().join("index-current.tmp");
        fs::write(&temporary, b"partial").unwrap();
        assert_eq!(
            storage
                .recover_planning(&index, 0, None)
                .unwrap_err()
                .reason_code,
            "draft_recovery_conflict"
        );
        fs::write(&temporary, &raw).unwrap();
        let unknown = history.parent().unwrap().join("unknown.json");
        fs::write(&unknown, b"unknown").unwrap();
        assert_eq!(
            storage
                .recover_planning(&index, 0, None)
                .unwrap_err()
                .reason_code,
            "draft_recovery_conflict"
        );
        fs::remove_file(unknown).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let directory = history.parent().unwrap();
            let original_permissions = fs::metadata(directory).unwrap().permissions();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o500)).unwrap();
            let failed = storage.recover_planning(&index, 0, None);
            fs::set_permissions(directory, original_permissions).unwrap();
            assert_eq!(
                failed.unwrap_err().reason_code,
                "draft_recovery_interrupted"
            );
            assert_eq!(fs::read(&temporary).unwrap(), raw);
            assert!(!storage.path("example", "index.json").unwrap().exists());
        }
        assert_eq!(
            storage.recover_planning(&index, 0, None).unwrap()["status"],
            "recovered"
        );
        assert_eq!(storage.read_planning_index("example").unwrap(), index);
        let mut next = index.clone();
        next["revision"] = json!(2);
        next["tasks"][0]["status"] = json!("in_progress");
        let draft = json!({"schema":"work-task-draft/v1","requirement_id":"example","task_id":"TASK-001","revision":1,
            "boundary_revision":1,"source":index["source"],"instructions_sha256":"d".repeat(64),"status":"in_progress",
            "notes":[],"confirmed_decisions":[],"tentative":[],"open_questions":[],"next_discussion_point":"Continue discussion."});
        storage.save_planning(&next, 1, Some(&draft)).unwrap();
        assert_eq!(
            storage
                .recover_planning(&index, 0, None)
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
        assert_eq!(
            storage.read_planning_index("example").unwrap()["revision"],
            2
        );
    }

    #[test]
    fn small_discussion_request_saves_selection_and_replaces_optional_candidate() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        let root = std::env::temp_dir().join(format!(
            "work-draft-small-request-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plan_path = "outputs/work/plans/example.json";
        for relative in [
            plan_path,
            "outputs/work/tasks/example/drafts/index.json",
            "outputs/work/tasks/example/drafts/history/1/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let storage = LocalTaskDraftStorage {
            project_root: root.clone(),
        };
        let mut index = storage.read_planning_index("example").unwrap();
        index["tasks"][0]
            .as_object_mut()
            .unwrap()
            .remove("instruction_selection");
        let mut other = index["tasks"][0].clone();
        other["id"] = json!("TASK-002");
        index["tasks"].as_array_mut().unwrap().push(other);
        index["retired_task_ids"] = json!(["TASK-003"]);
        let initial_raw = canonical_json(&index).unwrap();
        for relative in [
            "outputs/work/tasks/example/drafts/index.json",
            "outputs/work/tasks/example/drafts/history/1/index.json",
        ] {
            fs::write(root.join(relative), &initial_raw).unwrap();
        }
        let skill = repo.join("../skills/work");
        let first = json!({"status":"in_progress","notes":["Initial discussion"],
            "confirmed_decisions":[],"tentative":[],"open_questions":["Which test?"],
            "next_discussion_point":"Confirm test.","task_candidate":{"steps":[]}});
        let context = |revision, task_id, selected_paths, reference_names| TaskSourceCheckRequest {
            requirement_id: "example",
            task_id,
            expected_revision: revision,
            plan_path,
            skill_root: &skill,
            skill_configs: &[],
            selected_paths,
            reference_names,
        };
        for field in [
            "index",
            "revision",
            "source",
            "instructions_sha256",
            "task_id",
            "scope",
        ] {
            let mut invalid = first.clone();
            invalid[field] = json!("caller supplied");
            assert_eq!(
                storage
                    .save_discussion_request(
                        &invalid,
                        &context(1, "TASK-001", Some(&[]), Some(&[])),
                        false
                    )
                    .unwrap_err()
                    .reason_code,
                "invalid_object_fields"
            );
            assert_eq!(
                fs::read(root.join("outputs/work/tasks/example/drafts/index.json")).unwrap(),
                initial_raw
            );
        }
        assert_eq!(
            storage
                .save_discussion_request(
                    &first,
                    &context(1, "TASK-999", Some(&[]), Some(&[])),
                    false
                )
                .unwrap_err()
                .reason_code,
            "draft_task_not_in_index"
        );
        assert_eq!(
            storage
                .save_discussion_request(
                    &first,
                    &context(0, "TASK-001", Some(&[]), Some(&[])),
                    false
                )
                .unwrap_err()
                .reason_code,
            "invalid_expected_revision"
        );
        assert_eq!(
            storage
                .save_discussion_request(
                    &first,
                    &context(2, "TASK-001", Some(&[]), Some(&[])),
                    false
                )
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
        let first_saved = storage
            .save_discussion_request(&first, &context(1, "TASK-001", Some(&[]), Some(&[])), false)
            .unwrap();
        assert_eq!(first_saved["revision"], 2);
        let current = storage.read_planning_index("example").unwrap();
        assert_eq!(
            current["tasks"][0]["instruction_selection"],
            json!({"selected_paths":[],"references":[]})
        );
        assert_eq!(
            storage.read_draft(&current, "TASK-001").unwrap()["task_candidate"],
            first["task_candidate"]
        );
        let mut second = first.clone();
        second.as_object_mut().unwrap().remove("task_candidate");
        second["notes"] = json!(["Replacement discussion"]);
        let history1 =
            fs::read(root.join("outputs/work/tasks/example/drafts/history/1/index.json")).unwrap();
        let second_saved = storage
            .save_discussion_request(&second, &context(2, "TASK-001", None, None), false)
            .unwrap();
        assert_eq!(second_saved["revision"], 3);
        assert_eq!(
            storage
                .save_discussion_request(&second, &context(2, "TASK-001", None, None), true)
                .unwrap()["status"],
            "already_completed"
        );
        let current = storage.read_planning_index("example").unwrap();
        let draft = storage.read_draft(&current, "TASK-001").unwrap();
        assert_eq!(draft["revision"], 2);
        assert_eq!(draft["boundary_revision"], 1);
        assert_eq!(draft["notes"], second["notes"]);
        assert!(draft.get("task_candidate").is_none());
        assert_eq!(
            fs::read(root.join("outputs/work/tasks/example/drafts/history/1/index.json")).unwrap(),
            history1
        );
        let history2 =
            fs::read(root.join("outputs/work/tasks/example/drafts/history/2/index.json")).unwrap();
        let history3 =
            fs::read(root.join("outputs/work/tasks/example/drafts/history/3/index.json")).unwrap();
        assert_eq!(
            storage
                .save_discussion_request(
                    &first,
                    &context(3, "TASK-002", Some(&[]), Some(&[])),
                    false
                )
                .unwrap()["revision"],
            4
        );
        let before_other = storage.read_planning_index("example").unwrap()["tasks"][1].clone();
        let mut final_request = second.clone();
        final_request["notes"] = json!(["Later discussion"]);
        assert_eq!(
            storage
                .save_discussion_request(&final_request, &context(4, "TASK-001", None, None), false)
                .unwrap()["revision"],
            5
        );
        let current = storage.read_planning_index("example").unwrap();
        let draft = storage.read_draft(&current, "TASK-001").unwrap();
        assert_eq!(
            (
                current["revision"].as_u64(),
                draft["revision"].as_u64(),
                draft["boundary_revision"].as_u64()
            ),
            (Some(5), Some(3), Some(1))
        );
        assert_eq!(current["tasks"][1], before_other);
        assert_eq!(current["source"], index["source"]);
        assert_eq!(current["retired_task_ids"], json!(["TASK-003"]));
        assert_eq!(current["current_task_id"], "TASK-001");
        assert_eq!(
            storage.read_draft(&current, "TASK-002").unwrap()["notes"],
            first["notes"]
        );
        assert_eq!(draft["notes"], final_request["notes"]);
        for (revision, previous) in [(1, history1), (2, history2), (3, history3)] {
            assert_eq!(
                fs::read(root.join(format!(
                    "outputs/work/tasks/example/drafts/history/{revision}/index.json"
                )))
                .unwrap(),
                previous
            );
        }
        assert!(!root.join("outputs/work/tasks/example/task.json").exists());
        assert!(!root.join("outputs/work/executions/example").exists());
        let mut refined = final_request.clone();
        refined["status"] = json!("refined");
        assert_eq!(
            storage
                .save_discussion_request(&refined, &context(5, "TASK-001", None, None), false)
                .unwrap_err()
                .reason_code,
            "unfinished_refined_draft"
        );
        refined["open_questions"] = json!([]);
        refined["next_discussion_point"] = Value::Null;
        refined["task_candidate"] = json!({"id":"TASK-001"});
        assert_eq!(
            storage
                .save_discussion_request(&refined, &context(5, "TASK-001", None, None), false)
                .unwrap_err()
                .reason_code,
            "invalid_object_fields"
        );
        refined["task_candidate"] = json!({
            "steps":[{"key":"review","action":"Review result.",
                "references":[{"kind":"validations","key":"result"}]}],
            "validations":[{"key":"result","kind":"manual","confirmer":"User",
                "criteria":"Result is observable.","acceptance_positions":[1]}]
        });
        assert_eq!(
            storage
                .save_discussion_request(&refined, &context(5, "TASK-001", None, None), false)
                .unwrap()["revision"],
            6
        );
        let current = storage.read_planning_index("example").unwrap();
        let draft = storage.read_draft(&current, "TASK-001").unwrap();
        assert_eq!(draft["status"], "refined");
        assert_eq!(draft["task_candidate"], refined["task_candidate"]);
        assert_eq!(
            storage
                .save_discussion_request(&second, &context(2, "TASK-001", None, None), true)
                .unwrap_err()
                .reason_code,
            "draft_revision_conflict"
        );
    }

    #[test]
    fn small_discussion_recovery_requires_identical_request_and_sources() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        let root = std::env::temp_dir().join(format!(
            "work-draft-small-recovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plan_path = "outputs/work/plans/example.json";
        for relative in [
            plan_path,
            "outputs/work/tasks/example/drafts/index.json",
            "outputs/work/tasks/example/drafts/history/1/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let storage = LocalTaskDraftStorage {
            project_root: root.clone(),
        };
        let previous = storage.read_planning_index("example").unwrap();
        let request = json!({"status":"in_progress","notes":["Reviewed discussion"],
            "confirmed_decisions":[],"tentative":[],"open_questions":[],
            "next_discussion_point":"Continue discussion."});
        let mut proposed = previous.clone();
        proposed["revision"] = json!(2);
        proposed["current_task_id"] = json!("TASK-001");
        proposed["tasks"][0]["status"] = json!("in_progress");
        proposed["tasks"][0]["instruction_selection"] =
            json!({"selected_paths":[],"references":[]});
        let draft = json!({"schema":"work-task-draft/v1","requirement_id":"example",
            "task_id":"TASK-001","revision":1,
            "boundary_revision":previous["tasks"][0]["boundary_revision"],
            "source":previous["source"],
            "instructions_sha256":previous["tasks"][0]["instructions_sha256"],
            "status":request["status"],"notes":request["notes"],
            "confirmed_decisions":request["confirmed_decisions"],
            "tentative":request["tentative"],"open_questions":request["open_questions"],
            "next_discussion_point":request["next_discussion_point"]});
        let prepared = prepare_save(&proposed, 1, Some(&previous), Some(&draft)).unwrap();
        let history = storage.path("example", "history/2").unwrap();
        fs::create_dir(&history).unwrap();
        fs::write(history.join("index.json"), &prepared.index_raw).unwrap();
        fs::write(history.join("index-current.tmp"), &prepared.index_raw).unwrap();
        fs::write(
            history.join("TASK-001.json"),
            prepared.draft_raw.as_ref().unwrap(),
        )
        .unwrap();
        let skill = repo.join("../skills/work");
        let context = TaskSourceCheckRequest {
            requirement_id: "example",
            task_id: "TASK-001",
            expected_revision: 1,
            plan_path,
            skill_root: &skill,
            skill_configs: &[],
            selected_paths: Some(&[]),
            reference_names: Some(&[]),
        };
        let mut changed = request.clone();
        changed["notes"] = json!(["Different discussion"]);
        assert_eq!(
            storage
                .save_discussion_request(&changed, &context, true)
                .unwrap_err()
                .reason_code,
            "draft_recovery_conflict"
        );
        let plan = root.join(plan_path);
        let original_plan = fs::read(&plan).unwrap();
        let mut altered: Value = serde_json::from_slice(&original_plan).unwrap();
        altered["summary"] = json!("Different result");
        fs::write(
            &plan,
            work_operations::plan::render_plan_value(&altered).unwrap(),
        )
        .unwrap();
        assert_eq!(
            storage
                .save_discussion_request(&request, &context, true)
                .unwrap_err()
                .reason_code,
            "draft_source_drift"
        );
        fs::write(&plan, original_plan).unwrap();
        assert_eq!(storage.read_planning_index("example").unwrap(), previous);
        assert_eq!(
            storage
                .save_discussion_request(&request, &context, true)
                .unwrap()["status"],
            "recovered"
        );
        assert_eq!(
            storage
                .save_discussion_request(&request, &context, true)
                .unwrap()["status"],
            "already_completed"
        );
        assert_eq!(
            storage.read_planning_index("example").unwrap(),
            prepared.index
        );
    }

    #[test]
    fn source_check_validates_formal_plan_and_saved_selection_without_writing() {
        let root = std::env::temp_dir().join(format!(
            "work-draft-check-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let work_root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        let instructions = LocalHierarchyCatalog {
            skill_root: work_root.clone(),
        };
        let skills = LocalSkillCatalog { roots: vec![] };
        let paths = LocalPlanStorage {
            project_root: root.clone(),
        };
        let request = json!({"requirement_id":"example","title":"Plan","summary":"Result",
            "goals":["Result"],"scope":["Source"],"deliverables":["Artifact"],
            "acceptance_criteria":["Observable"],
            "hierarchy_selection_request":{"decision":"general_only","selections":[]},
            "skill_selection_request":{"decision":"base_only","skills":[]},"references":[]});
        let prepared =
            work_feature::plan::prepare_semantic(&instructions, &skills, &paths, &[], &request)
                .unwrap();
        let plan_path = prepared["path"].as_str().unwrap();
        let absolute = root.join(plan_path);
        fs::create_dir_all(absolute.parent().unwrap()).unwrap();
        fs::write(
            &absolute,
            work_operations::plan::render_plan_value(&prepared["plan"]).unwrap(),
        )
        .unwrap();
        let loaded = work_feature::instruction::load(&instructions, "task", &[], &[]).unwrap();
        let index = json!({"schema":"work-task-planning-index/v1","requirement_id":"example","revision":1,
            "source":{"plan_sha256":prepared["validation"]["plan_sha256"],
                "hierarchy_selection_sha256":prepared["validation"]["hierarchy_selection_sha256"],
                "skill_selection_sha256":prepared["validation"]["skill_selection_sha256"]},
            "current_task_id":"TASK-001","tasks":[{"id":"TASK-001","title":"Task","goal":"Result",
                "scope":["Source"],"skill_id":null,"dependencies":[],"status":"planned",
                "boundary_revision":1,"instructions_sha256":loaded.instructions_sha256,
                "instruction_selection":{"selected_paths":[],"references":[]}}]});
        let storage = LocalTaskDraftStorage {
            project_root: root.clone(),
        };
        storage.save_planning(&index, 0, None).unwrap();
        let before = fs::read(storage.path("example", "index.json").unwrap()).unwrap();
        let mut check = TaskSourceCheckRequest {
            requirement_id: "example",
            task_id: "TASK-001",
            expected_revision: 1,
            plan_path,
            skill_root: &work_root,
            skill_configs: &[],
            selected_paths: None,
            reference_names: None,
        };
        let result = storage.check_sources(&check).unwrap();
        assert_eq!(result["status"], "valid");
        assert_eq!(
            result["instruction_selection"],
            index["tasks"][0]["instruction_selection"]
        );
        assert_eq!(
            fs::read(storage.path("example", "index.json").unwrap()).unwrap(),
            before
        );
        check.expected_revision = 2;
        assert_eq!(
            storage.check_sources(&check).unwrap_err().reason_code,
            "draft_revision_conflict"
        );
        check.expected_revision = 1;
        let mut changed = prepared["plan"].clone();
        changed["summary"] = json!("Changed result");
        fs::write(
            &absolute,
            work_operations::plan::render_plan_value(&changed).unwrap(),
        )
        .unwrap();
        assert_eq!(
            storage.check_sources(&check).unwrap_err().reason_code,
            "draft_source_drift"
        );
        fs::write(&absolute, b"broken").unwrap();
        let upstream =
            validate_plan_bytes(&instructions, &skills, &paths, &[], b"broken", plan_path)
                .unwrap_err();
        let propagated = storage.check_sources(&check).unwrap_err();
        assert_eq!(propagated.exit_code, upstream.exit_code);
        assert_eq!(propagated.reason_code, upstream.reason_code);
        assert_eq!(propagated.details, upstream.details);
    }

    #[test]
    fn source_check_matches_python_artifact_and_error_fixtures() {
        let fixtures = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/fixtures/task-draft-sources"
        ));
        let skill_root =
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        for name in ["valid", "plan-drift", "missing-selection"] {
            let fixture = fixtures.join(name);
            let root = std::env::temp_dir().join(format!(
                "work-draft-python-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let relative = [
                "outputs/work/plans/example.json",
                "outputs/work/tasks/example/drafts/index.json",
                "outputs/work/tasks/example/drafts/history/1/index.json",
            ];
            for path in relative {
                let target = root.join(path);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::copy(fixture.join(path), target).unwrap();
            }
            let storage = LocalTaskDraftStorage {
                project_root: root.clone(),
            };
            let request = TaskSourceCheckRequest {
                requirement_id: "example",
                task_id: "TASK-001",
                expected_revision: 1,
                plan_path: "outputs/work/plans/example.json",
                skill_root: &skill_root,
                skill_configs: &[],
                selected_paths: None,
                reference_names: None,
            };
            let expected: Value =
                serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
            let result = match storage.check_sources(&request) {
                Ok(value) => json!({"exit_code":0,"result":value}),
                Err(error) => json!({"exit_code":error.exit_code as i32,"result":{
                    "schema":"work-error/v1","code":error.reason_code,
                    "message":error.message,"details":error.details}}),
            };
            assert_eq!(result, expected, "{name}");
            for path in relative {
                assert_eq!(
                    fs::read(root.join(path)).unwrap(),
                    fs::read(fixture.join(path)).unwrap(),
                    "source check modified {name}/{path}"
                );
            }
        }
    }
}
