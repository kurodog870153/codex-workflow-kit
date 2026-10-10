//! Formal TASK item and index filesystem repository.

use std::fs;
use std::path::PathBuf;

use serde_json::json;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_feature::task::TaskCollectionRepository;

use crate::files::{LocalFiles, resolve_project_path};

#[derive(Debug, Clone)]
pub struct LocalTaskStorage {
    pub project_root: PathBuf,
}

impl TaskCollectionRepository for LocalTaskStorage {
    fn read_task_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        let (_, path) = resolve_project_path(&self.project_root, relative_path)?;
        if let Some((collection, _)) = relative_path.rsplit_once("/tasks/") {
            let (_, collection_path) = resolve_project_path(&self.project_root, collection)?;
            if let (Ok(parent), Ok(collection)) = (
                path.parent().expect("item path has parent").canonicalize(),
                collection_path.canonicalize(),
            ) {
                if !parent.starts_with(&collection) {
                    return Err(WorkError::new(
                        ExitCode::Contract,
                        "task_item_path_escapes_collection",
                        "The resolved TASK item path escapes the formal TASK directory.",
                        json!({"path": relative_path}),
                    ));
                }
            }
        }
        LocalFiles.read_raw(&path)
    }

    fn item_names(&self, index_path: &str) -> Result<Vec<String>, WorkError> {
        let (_, path) = resolve_project_path(&self.project_root, index_path)?;
        let directory = path.parent().expect("index path has parent").join("tasks");
        if !directory.is_dir() {
            return Ok(Vec::new());
        }
        let entries = fs::read_dir(&directory).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "task_collection_directory_read_failed",
                "The TASK item directory could not be inspected.",
                json!({"path": directory.to_string_lossy()}),
            )
        })?;
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "task_collection_directory_read_failed",
                    "The TASK item directory could not be inspected.",
                    json!({"path": directory.to_string_lossy()}),
                )
            })?;
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
                && path.is_file()
            {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        names.sort();
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    use crate::writer_lock::WriterLock;
    use serde_json::{Value, json};
    use work_feature::execution::{
        AttemptStartPublication, AttemptStartRepository, CommandProjectRequest,
        CommandProjectSources, DeviationProjectRequest, ExecutionIndexRepository,
        ExecutionProjectTarget, RecoveryPrepareRepository, begin_record_from_project,
        close_attempt_from_project, create_correction_from_project, finish_record_from_project,
        inspect_worktree_from_project, prepare_attempt_start_from_project,
        prepare_command_from_project, prepare_deviation_from_project,
        prepare_execute_preflight_from_project,
        prepare_legacy_recovery_from_project as prepare_recovery_from_project,
        prepare_semantic_deviation_from_project, record_command_correction_from_project,
        record_deviation_from_project, start_attempt_from_project,
    };
    use work_feature::instruction::{load, select, task_document_selection};
    use work_feature::ports::Git;
    use work_feature::task::{
        CollectionInput, load_collection, load_task_execution_context,
        recheck_task_execution_context, validate_collection,
    };
    use work_operations::derivation::fingerprint::skill_selection as skill_selection_sha256;
    use work_operations::derivation::fingerprint::{
        raw as sha256_hex, structured as canonical_json_sha256,
    };
    use work_operations::execution::attempt::render_attempt;
    use work_operations::execution::build_execution_lock;
    use work_operations::execution::correction::{build_correction_candidates, render_correction};
    use work_operations::execution::index::{
        build_initial_execution_index, render_execution_index,
    };
    use work_operations::execution::preflight::FileState;
    use work_operations::task::ordering::{TaskDocumentKind, render_task};

    use crate::execution::storage::{
        AttemptStartRecoveryRequest, CorrectionRecoveryInput, LocalExecutionStorage,
    };
    use crate::git::LocalGit;
    use crate::hierarchy_catalog::LocalHierarchyCatalog;
    use crate::skill_catalog::LocalSkillCatalog;
    use crate::writer_lock::LocalWriterLock;

    struct CountedTaskStorage<'a> {
        inner: &'a LocalTaskStorage,
        reads: RefCell<Vec<String>>,
    }

    impl TaskCollectionRepository for CountedTaskStorage<'_> {
        fn read_task_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
            self.reads.borrow_mut().push(relative_path.to_owned());
            self.inner.read_task_file(relative_path)
        }

        fn item_names(&self, index_path: &str) -> Result<Vec<String>, WorkError> {
            self.inner.item_names(index_path)
        }
    }

    struct ChangingTaskStorage<'a> {
        inner: &'a LocalTaskStorage,
        target_reads: Cell<usize>,
    }

    #[derive(Clone, Copy)]
    enum StagingEvidenceChange {
        Add,
        Delete,
        Bytes,
    }

    struct ChangingRecoveryStorage<'a> {
        inner: &'a LocalExecutionStorage,
        inventory_calls: Cell<usize>,
        staged_reads: Cell<usize>,
        change_inventory: bool,
        staging_change: Option<StagingEvidenceChange>,
    }

    impl ExecutionIndexRepository for ChangingRecoveryStorage<'_> {
        fn read_index(&self, path: &str) -> Result<Vec<u8>, WorkError> {
            self.inner.read_index(path)
        }
        fn inspect_path(&self, path: &str) -> Result<FileState, WorkError> {
            self.inner.inspect_path(path)
        }
        fn check_execution_ready(&self, task: &str, execution: &str) -> Result<(), WorkError> {
            self.inner.check_execution_ready(task, execution)
        }
        fn check_execution_task_layout(
            &self,
            execution: &str,
            task: &str,
        ) -> Result<(), WorkError> {
            self.inner.check_execution_task_layout(execution, task)
        }
    }

    impl AttemptStartRepository for ChangingRecoveryStorage<'_> {
        fn attempt_names(&self, execution: &str, task: &str) -> Result<Vec<String>, WorkError> {
            self.inner.attempt_names(execution, task)
        }
        fn read_attempt(&self, path: &str) -> Result<Vec<u8>, WorkError> {
            self.inner.read_attempt(path)
        }
        fn publish_attempt_start(
            &self,
            publication: &AttemptStartPublication<'_>,
        ) -> Result<(), WorkError> {
            self.inner.publish_attempt_start(publication)
        }
    }

    impl RecoveryPrepareRepository for ChangingRecoveryStorage<'_> {
        fn staging_transactions(
            &self,
            context: &work_feature::execution::ExecutionWriterContext,
        ) -> Result<Vec<work_feature::execution::StagingRecoverySnapshot>, WorkError> {
            let mut transactions = self.inner.staging_transactions(context)?;
            self.inventory_calls.set(self.inventory_calls.get() + 1);
            if self.inventory_calls.get() == 2 {
                if let Some(change) = self.staging_change {
                    let files = &mut transactions[0].files;
                    match change {
                        StagingEvidenceChange::Add => {
                            files.insert("foreign.tmp".into(), b"foreign".to_vec());
                        }
                        StagingEvidenceChange::Delete => {
                            files.remove("index.json.tmp");
                        }
                        StagingEvidenceChange::Bytes => {
                            files.get_mut("index.json.tmp").unwrap().push(0);
                        }
                    }
                }
            }
            Ok(transactions)
        }

        fn check_recovery_idle(&self, execution: &str) -> Result<(), WorkError> {
            self.inner.check_recovery_idle(execution)
        }
        fn temporary_names(&self, execution: &str) -> Result<Vec<String>, WorkError> {
            let mut names = self.inner.temporary_names(execution)?;
            self.inventory_calls.set(self.inventory_calls.get() + 1);
            if self.change_inventory && self.inventory_calls.get() == 2 {
                names.push(".work-added-evidence.tmp".into());
            }
            Ok(names)
        }
        fn recovery_source_exists(&self, path: &str) -> Result<bool, WorkError> {
            self.inner.recovery_source_exists(path)
        }
        fn read_recovery_source(&self, path: &str) -> Result<Vec<u8>, WorkError> {
            let raw = self.inner.read_recovery_source(path)?;
            if !self.change_inventory && path.ends_with("-attempt.tmp") {
                self.staged_reads.set(self.staged_reads.get() + 1);
                if self.staged_reads.get() == 2 {
                    return Ok([raw, b"\n".to_vec()].concat());
                }
            }
            Ok(raw)
        }
    }

    impl TaskCollectionRepository for ChangingTaskStorage<'_> {
        fn read_task_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
            if relative_path.ends_with("/tasks/TASK-001.json") {
                self.target_reads.set(self.target_reads.get() + 1);
                if self.target_reads.get() > 1 {
                    return Ok(b"changed after preflight".to_vec());
                }
            }
            self.inner.read_task_file(relative_path)
        }

        fn item_names(&self, index_path: &str) -> Result<Vec<String>, WorkError> {
            self.inner.item_names(index_path)
        }
    }

    #[test]
    fn staging_recovery_fake_detects_added_deleted_or_changed_bytes_without_touching_other_requirement()
     {
        use work_model::runtime::*;
        use work_operations::execution::recovery::*;
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/shared/task-diagnostics-project");
        let root = std::env::temp_dir().join(format!(
            "work-task-staging-fake-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let task_path = "outputs/work/tasks/example/index.json";
        for relative in [task_path, "outputs/work/tasks/example/tasks/TASK-001.json"] {
            let target = root.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), target).unwrap();
        }
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        fs::write(root.join("src.txt"), b"original\n").unwrap();
        let work = LocalHierarchyCatalog {
            skill_root: crate::fixture_support::historical_task_skill_root(
                &repo.join("../skills/work"),
            )
            .unwrap(),
        };
        let skills = LocalSkillCatalog { roots: vec![] };
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: root.clone(),
        };
        let tasks = LocalTaskStorage {
            project_root: root.clone(),
        };
        let sources = CommandProjectSources {
            instructions: &work,
            skills: &skills,
            paths: &paths,
            task_repository: &tasks,
            skill_roots: &[],
        };
        let full = load_collection(&work, &skills, &paths, &tasks, &[], task_path).unwrap();
        let execution = full["collection_contract"]["artifacts"]["execution"]
            .as_str()
            .unwrap();
        let index = build_initial_execution_index(&full["collection_contract"], &full).unwrap();
        let raw = render_execution_index(&index).unwrap();
        let target = format!("{execution}/index.json");
        fs::create_dir_all(root.join(execution)).unwrap();
        fs::write(root.join(&target), &raw).unwrap();
        let context = work_feature::execution::load_execution_writer_context(
            &sources,
            ExecutionProjectTarget {
                task_path,
                execution_dir: execution,
                task_id: "TASK-001",
            },
        )
        .unwrap();
        let payloads = BTreeMap::from([("index.json.tmp".into(), raw.clone())]);
        let evidence = RuntimeBytes {
            sha256: sha256_hex(&raw),
            bytes: raw.clone(),
        };
        let manifest=build_execution_staging_manifest(ExecutionStagingInput {
            canonical_root:context.writer().canonical_project_root.to_str().unwrap(),requirement:&context.writer().requirement_id,
            execution_dir:execution,operation:work_operations::derivation::publication::RuntimeOperation::RecordBegin,
            approval_sha256:&sha256_hex(&raw),business_identity:json!({"task_id":"TASK-001","attempt_id":"ATTEMPT-001","record_id":"CMD-001"}),
            targets:vec![RuntimeTarget {path:target.clone(),before:Some(evidence.clone()),after:Some(evidence)}],payloads:&payloads,
        }).unwrap();
        let directory = crate::transaction_storage::prepare_runtime_transaction(
            &LocalFiles,
            &context.writer().canonical_project_root,
            &manifest,
            &payloads,
        )
        .unwrap();
        let other = root.join("outputs/work/runtime/staging/other/unknown/bad");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("transaction.json"), b"unknown other requirement").unwrap();
        let storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        for change in [
            StagingEvidenceChange::Add,
            StagingEvidenceChange::Delete,
            StagingEvidenceChange::Bytes,
        ] {
            let fake = ChangingRecoveryStorage {
                inner: &storage,
                inventory_calls: Cell::new(0),
                staged_reads: Cell::new(0),
                change_inventory: false,
                staging_change: Some(change),
            };
            let before = fake.staging_transactions(&context).unwrap();
            let after = fake.staging_transactions(&context).unwrap();
            let binding = |snap: &work_feature::execution::StagingRecoverySnapshot| {
                execution_staging_binding(
                    &snap.manifest,
                    context.writer().canonical_project_root.to_str().unwrap(),
                    &context.writer().requirement_id,
                    execution,
                    "TASK-001",
                    &snap.files,
                )
            };
            let reviewed = binding(&before[0]).unwrap();
            match binding(&after[0]) {
                Ok(current) => assert_eq!(
                    work_feature::execution::recovery::require_staging_recovery_evidence(
                        &reviewed, &current
                    )
                    .unwrap_err()
                    .reason_code,
                    "execution_recovery_evidence_changed"
                ),
                Err(issue) => assert!(matches!(
                    issue.reason_code,
                    "execution_staging_inventory" | "execution_staging_evidence_changed"
                )),
            }
        }
        assert_eq!(fs::read(directory.join("index.json.tmp")).unwrap(), raw);
        assert_eq!(fs::read(root.join(target)).unwrap(), raw);
        assert_eq!(
            fs::read(other.join("transaction.json")).unwrap(),
            b"unknown other requirement"
        );
    }

    #[test]
    fn task_item_read_stays_within_formal_collection() {
        let root = std::env::temp_dir().join(format!(
            "work-task-item-path-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let items = root.join("outputs/work/tasks/feature-1/tasks");
        fs::create_dir_all(&items).unwrap();
        fs::write(items.join("TASK-001.json"), b"item").unwrap();
        let storage = LocalTaskStorage {
            project_root: root.clone(),
        };
        assert_eq!(
            storage
                .read_task_file("outputs/work/tasks/feature-1/tasks/TASK-001.json")
                .unwrap(),
            b"item"
        );
        #[cfg(unix)]
        {
            let collection = root.join("outputs/work/tasks/feature-2");
            let elsewhere = root.join("elsewhere");
            fs::create_dir_all(&collection).unwrap();
            fs::create_dir_all(&elsewhere).unwrap();
            std::os::unix::fs::symlink(&elsewhere, collection.join("tasks")).unwrap();
            assert_eq!(
                storage
                    .read_task_file("outputs/work/tasks/feature-2/tasks/TASK-001.json")
                    .unwrap_err()
                    .reason_code,
                "task_item_path_escapes_collection"
            );
        }
    }

    #[test]
    fn independent_collection_accepts_both_provenances_and_rejects_missing_evidence() {
        for fixture_name in [
            "shared/task-diagnostics-project",
            "shared/specification-migration-project",
        ] {
            let fixture =
                PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures")).join(fixture_name);
            let root = std::env::temp_dir().join(format!(
                "work-formal-provenance-{}-{}-{}",
                std::process::id(),
                fixture_name,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let index_path = "outputs/work/tasks/example/index.json";
            let index_raw = fs::read(fixture.join(index_path)).unwrap();
            let mut index: Value = serde_json::from_slice(&index_raw).unwrap();
            let index_file = root.join(index_path);
            fs::create_dir_all(index_file.parent().unwrap()).unwrap();
            fs::write(&index_file, &index_raw).unwrap();
            for row in index["tasks"].as_array().unwrap() {
                let relative = format!(
                    "outputs/work/tasks/example/{}",
                    row["path"].as_str().unwrap()
                );
                let target = root.join(&relative);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::write(target, fs::read(fixture.join(relative)).unwrap()).unwrap();
            }
            if fixture_name == "shared/task-diagnostics-project" {
                crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
            }
            let work = LocalHierarchyCatalog {
                skill_root: PathBuf::from(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../../skills/work"
                )),
            };
            // Confirm current Task instructions without changing retained migration evidence.
            let mut sets = Vec::new();
            for row in index["tasks"].as_array_mut().unwrap() {
                let relative = format!(
                    "outputs/work/tasks/example/{}",
                    row["path"].as_str().unwrap()
                );
                let item_file = root.join(&relative);
                let mut item: Value =
                    serde_json::from_slice(&fs::read(&item_file).unwrap()).unwrap();
                let selected: Vec<String> =
                    serde_json::from_value(item["instruction_selection"]["selected_paths"].clone())
                        .unwrap();
                let refs: Vec<String> =
                    serde_json::from_value(item["instruction_selection"]["references"].clone())
                        .unwrap();
                item["instruction_selection"] =
                    serde_json::to_value(select(&work, "task", &selected, &refs).unwrap()).unwrap();
                sets.push(load(&work, "task", &selected, &refs).unwrap());
                let raw = render_task(&item, TaskDocumentKind::Item).unwrap();
                row["canonical_sha256"] = json!(sha256_hex(&raw));
                fs::write(item_file, raw).unwrap();
            }
            index["instruction_selection"] =
                serde_json::to_value(task_document_selection(&sets).unwrap()).unwrap();
            let index_raw = render_task(&index, TaskDocumentKind::Index).unwrap();
            fs::write(&index_file, &index_raw).unwrap();
            let skills = LocalSkillCatalog { roots: vec![] };
            let paths = crate::artifact_paths::LocalArtifactPaths {
                project_root: root.clone(),
            };
            let repository = LocalTaskStorage {
                project_root: root.clone(),
            };
            let loaded =
                load_collection(&work, &skills, &paths, &repository, &[], index_path).unwrap();
            assert!(loaded["task_count"].as_u64().unwrap() > 0);
            assert!(!root.join("outputs/work/plans").exists());
            let mut missing = index.clone();
            missing.as_object_mut().unwrap().remove("source");
            fs::write(&index_file, serde_json::to_vec(&missing).unwrap()).unwrap();
            assert!(load_collection(&work, &skills, &paths, &repository, &[], index_path).is_err());
            if index["source"]["kind"] == "migration" {
                let mut unapproved = index.clone();
                unapproved["source"]["approval_sha256"] = json!("0".repeat(64));
                fs::write(&index_file, serde_json::to_vec(&unapproved).unwrap()).unwrap();
                assert_eq!(
                    load_collection(&work, &skills, &paths, &repository, &[], index_path)
                        .unwrap_err()
                        .reason_code,
                    "migration_source_approval_mismatch"
                );
            } else {
                fs::write(&index_file, &index_raw).unwrap();
                let source_path = root.join(format!(
                    "outputs/work/sources/example/{}/source.txt",
                    index["source"]["manifest"]["source_id"].as_str().unwrap()
                ));
                let mut raw = fs::read(&source_path).unwrap();
                raw[0] ^= 1;
                fs::write(source_path, raw).unwrap();
                assert_eq!(
                    load_collection(&work, &skills, &paths, &repository, &[], index_path)
                        .unwrap_err()
                        .reason_code,
                    "source_hash_mismatch"
                );
            }
        }
    }

    #[test]
    fn collection_validates_without_plan_items_sources_and_fingerprints() {
        let root = std::env::temp_dir().join(format!(
            "work-task-execution-context-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        LocalGit
            .mutate(&root, &["init".into(), "-q".into()])
            .unwrap();
        let work = LocalHierarchyCatalog {
            skill_root: crate::fixture_support::historical_task_skill_root(&PathBuf::from(
                concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"),
            ))
            .unwrap(),
        };
        let skills = LocalSkillCatalog { roots: vec![] };
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: root.clone(),
        };
        let hierarchy = work_feature::hierarchy::build_selection(
            &work,
            &json!({"decision":"general_only","selections":[]}),
        )
        .unwrap();
        let selected_skills = work_feature::skill::build_selection(
            &skills,
            &[],
            &json!({"decision":"base_only","skills":[]}),
        )
        .unwrap();
        let task_path = "outputs/work/tasks/issue55-task-parity/index.json";
        let references = vec!["task.general.task-records".into()];
        let selection = select(&work, "task", &[], &references).unwrap();
        let set = load(&work, "task", &[], &references).unwrap();
        let document = task_document_selection(&[set]).unwrap();
        let source = crate::fixture_support::capture_planning_context(
            &root,
            "issue55-task-parity",
            b"Deliver the result artifact and verify it.",
            &hierarchy,
            &selected_skills,
            &json!([{"id":"ACCEPTANCE-001","criterion":"Result is verified."}]),
        )
        .unwrap();
        let item = json!({"schema":"work-task-item","id":"TASK-001","title":"Implement result","skill_id":null,
            "instruction_selection":selection,"traceability":{"acceptance_ids":["ACCEPTANCE-001"]},"acceptance_criteria":[{"id":"TASK-001-ACCEPTANCE-001","criterion":"Result is verified."}],
            "goal":"Deliver result.","steps":[{"id":"STEP-001","action":"Verify result.","references":["VAL-001"]}],
            "validations":[{"id":"VAL-001","kind":"manual","confirmer":"user","criteria":"Result is verified.","acceptance_ids":["ACCEPTANCE-001","TASK-001-ACCEPTANCE-001"]}]});
        let item_raw = render_task(&item, TaskDocumentKind::Item).unwrap();
        let index = json!({"schema":"work-task-index","requirement_id":"issue55-task-parity","spec_id":"TASK-SPEC-001","status":"confirmed",
            "title":"Task parity","summary":"Verify collection.","artifacts":source["artifacts"],
            "source":{"kind":"snapshot","manifest":source["snapshot"]},"hierarchy_selection":source["hierarchy_selection"],"skill_selection":source["skill_selection"],"acceptance_criteria":source["acceptance_criteria"],
            "instruction_selection":document,"tasks":[{"id":"TASK-001","path":"tasks/TASK-001.json","canonical_sha256":sha256_hex(&item_raw)}],
            "readiness":{"status":"passed","spec_id":"TASK-SPEC-001"}});
        let index_raw = render_task(&index, TaskDocumentKind::Index).unwrap();
        let items = BTreeMap::from([("TASK-001".into(), item_raw.clone())]);
        let result = validate_collection(
            &work,
            &skills,
            &paths,
            &[],
            CollectionInput {
                index_raw: &index_raw,
                item_raw: &items,
                index_path: task_path,
            },
        )
        .unwrap();
        assert_eq!(result["task_ids"], json!(["TASK-001"]));
        assert_eq!(
            result["instructions_sha256"],
            document["instructions_sha256"]
        );
        assert_eq!(
            result["task_instructions_sha256"]["TASK-001"],
            selection.instructions_sha256
        );
        assert_eq!(result["task_skill_ids"]["TASK-001"], json!(null));
        assert_eq!(
            result["hierarchy_selection_sha256"],
            hierarchy["selection_sha256"]
        );
        assert!(result.get("rules_sha256").is_none());
        assert!(result.get("task_rules_sha256").is_none());
        assert_eq!(
            result["task_item_sha256"]["TASK-001"],
            sha256_hex(&item_raw)
        );
        let mut stale_item = item.clone();
        stale_item["instruction_selection"]["instructions_sha256"] = json!("0".repeat(64));
        let stale_raw = render_task(&stale_item, TaskDocumentKind::Item).unwrap();
        let mut stale_index = index.clone();
        stale_index["tasks"][0]["canonical_sha256"] = json!(sha256_hex(&stale_raw));
        let stale_index_raw = render_task(&stale_index, TaskDocumentKind::Index).unwrap();
        assert_eq!(
            validate_collection(
                &work,
                &skills,
                &paths,
                &[],
                CollectionInput {
                    index_raw: &stale_index_raw,
                    item_raw: &BTreeMap::from([("TASK-001".into(), stale_raw)]),
                    index_path: task_path,
                },
            )
            .unwrap_err()
            .reason_code,
            "instructions_fingerprint_mismatch"
        );
        let index_file = root.join(task_path);
        let item_path = index_file.parent().unwrap().join("tasks/TASK-001.json");
        fs::create_dir_all(item_path.parent().unwrap()).unwrap();
        fs::write(&index_file, &index_raw).unwrap();
        fs::write(&item_path, &item_raw).unwrap();
        let repository = LocalTaskStorage {
            project_root: root.clone(),
        };
        let context = load_task_execution_context(
            &work,
            &skills,
            &paths,
            &repository,
            &[],
            task_path,
            "TASK-001",
        )
        .unwrap();
        assert_eq!(context.contract["schema"], "work-task-execution-view");
        assert_eq!(context.contract["tasks"][0]["id"], "TASK-001");
        assert_eq!(
            canonical_json_sha256(&context.contract).unwrap(),
            "f13613cbc40ccc684ce4404570d19fd8ab735470bdf1d0af2f070c18432f2197"
        );
        assert_eq!(
            canonical_json_sha256(&context.validation).unwrap(),
            "30c3235d6c071d9baf8d262d293a7cded10055360d89cbabb6c896898cc70892"
        );
        assert_eq!(
            context.validation["task_collection_sha256"],
            result["task_collection_sha256"]
        );
        assert_eq!(context.sources[task_path], index_raw);
        assert_eq!(
            context.sources[&format!(
                "{}/tasks/TASK-001.json",
                task_path.rsplit_once('/').unwrap().0
            )],
            item_raw
        );
        assert!(!root.join("outputs/work/plans").exists());
        let loaded = load_collection(&work, &skills, &paths, &repository, &[], task_path).unwrap();
        assert_eq!(loaded["task_ids"], json!(["TASK-001"]));
        assert_eq!(loaded["task_count"], 1);
        assert_eq!(
            loaded["task_collection_sha256"],
            result["task_collection_sha256"]
        );
        let legacy_path = index_file.with_file_name("task.json");
        fs::write(&legacy_path, &index_raw).unwrap();
        assert!(
            load_collection(
                &work,
                &skills,
                &paths,
                &repository,
                &[],
                &format!("{}/task.json", task_path.rsplit_once('/').unwrap().0),
            )
            .is_err()
        );
        fs::remove_file(&legacy_path).unwrap();
        fs::remove_file(&item_path).unwrap();
        assert_eq!(
            load_collection(&work, &skills, &paths, &repository, &[], task_path)
                .unwrap_err()
                .reason_code,
            "file_not_found"
        );
        fs::write(&item_path, &item_raw).unwrap();
        let orphan = item_path.with_file_name("TASK-999.json");
        fs::write(&orphan, &item_raw).unwrap();
        let error =
            load_collection(&work, &skills, &paths, &repository, &[], task_path).unwrap_err();
        assert_eq!(error.reason_code, "task_collection_directory_mismatch");
        assert_eq!(error.details["orphan"], json!(["TASK-999.json"]));
        fs::remove_file(&orphan).unwrap();
        let mut wrong_hash = index.clone();
        wrong_hash["tasks"][0]["canonical_sha256"] = json!("0".repeat(64));
        fs::write(
            &index_file,
            render_task(&wrong_hash, TaskDocumentKind::Index).unwrap(),
        )
        .unwrap();
        assert_eq!(
            load_collection(&work, &skills, &paths, &repository, &[], task_path)
                .unwrap_err()
                .reason_code,
            "task_item_fingerprint_mismatch"
        );
        fs::write(&index_file, &index_raw).unwrap();
        let mut unknown_dependency = item.clone();
        unknown_dependency["dependencies"] = json!(["TASK-999"]);
        let unknown_raw = render_task(&unknown_dependency, TaskDocumentKind::Item).unwrap();
        let mut unknown_index = index.clone();
        unknown_index["tasks"][0]["canonical_sha256"] = json!(sha256_hex(&unknown_raw));
        fs::write(&item_path, &unknown_raw).unwrap();
        fs::write(
            &index_file,
            render_task(&unknown_index, TaskDocumentKind::Index).unwrap(),
        )
        .unwrap();
        assert_eq!(
            load_collection(&work, &skills, &paths, &repository, &[], task_path)
                .unwrap_err()
                .reason_code,
            "invalid_task_dependency"
        );
        fs::write(&item_path, &item_raw).unwrap();
        fs::write(&index_file, &index_raw).unwrap();
        let mut second_item = item.clone();
        second_item["id"] = json!("TASK-002");
        second_item["acceptance_criteria"][0]["id"] = json!("TASK-002-ACCEPTANCE-001");
        second_item["validations"][0]["acceptance_ids"][1] = json!("TASK-002-ACCEPTANCE-001");
        let second_raw = render_task(&second_item, TaskDocumentKind::Item).unwrap();
        let mut second_index = index.clone();
        second_index["tasks"].as_array_mut().unwrap().push(json!({
            "id":"TASK-002","path":"tasks/TASK-002.json",
            "canonical_sha256":sha256_hex(&second_raw)}));
        let second_path = item_path.with_file_name("TASK-002.json");
        fs::write(&second_path, &second_raw).unwrap();
        fs::write(
            &index_file,
            render_task(&second_index, TaskDocumentKind::Index).unwrap(),
        )
        .unwrap();
        let counted = CountedTaskStorage {
            inner: &repository,
            reads: RefCell::new(Vec::new()),
        };
        let selected = load_task_execution_context(
            &work,
            &skills,
            &paths,
            &counted,
            &[],
            task_path,
            "TASK-001",
        )
        .unwrap();
        assert_eq!(
            *counted.reads.borrow(),
            [
                task_path.to_owned(),
                format!(
                    "{}/tasks/TASK-001.json",
                    task_path.rsplit_once('/').unwrap().0
                ),
                format!(
                    "{}/tasks/TASK-002.json",
                    task_path.rsplit_once('/').unwrap().0
                ),
            ]
        );
        assert_eq!(selected.contract["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(selected.contract["tasks"][0]["id"], "TASK-001");
        assert_eq!(
            selected.validation["task_ids"],
            json!(["TASK-001", "TASK-002"])
        );
        assert_eq!(
            selected.validation["task_item_sha256"]
                .as_object()
                .unwrap()
                .len(),
            2
        );
        let mut large_index = second_index.clone();
        for number in 3..=100 {
            let task_id = format!("TASK-{number:03}");
            let mut next_item = item.clone();
            next_item["id"] = json!(task_id);
            let technical = format!("{task_id}-ACCEPTANCE-001");
            next_item["acceptance_criteria"][0]["id"] = json!(technical);
            next_item["validations"][0]["acceptance_ids"][1] = json!(technical);
            next_item["title"] = json!(format!("Validate item {number}"));
            if number == 100 {
                next_item["dependencies"] = json!(["TASK-001"]);
            }
            let raw = render_task(&next_item, TaskDocumentKind::Item).unwrap();
            large_index["tasks"].as_array_mut().unwrap().push(json!({
                "id":task_id,"path":format!("tasks/{task_id}.json"),
                "canonical_sha256":sha256_hex(&raw)}));
            fs::write(item_path.with_file_name(format!("{task_id}.json")), raw).unwrap();
        }
        fs::write(
            &index_file,
            render_task(&large_index, TaskDocumentKind::Index).unwrap(),
        )
        .unwrap();
        let counted_large = CountedTaskStorage {
            inner: &repository,
            reads: RefCell::new(Vec::new()),
        };
        let large = load_task_execution_context(
            &work,
            &skills,
            &paths,
            &counted_large,
            &[],
            task_path,
            "TASK-100",
        )
        .unwrap();
        assert_eq!(counted_large.reads.borrow().len(), 101);
        assert_eq!(large.validation["task_ids"].as_array().unwrap().len(), 100);
        assert_eq!(
            large.contract["tasks"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["TASK-001", "TASK-100"]
        );
        recheck_task_execution_context(&paths, &repository, &large).unwrap();
        let middle = item_path.with_file_name("TASK-050.json");
        let middle_raw = fs::read(&middle).unwrap();
        fs::write(&middle, [middle_raw.as_slice(), b" "].concat()).unwrap();
        assert_eq!(
            recheck_task_execution_context(&paths, &repository, &large)
                .unwrap_err()
                .reason_code,
            "execute_worktree_task_changed"
        );
        for number in 3..=100 {
            fs::remove_file(item_path.with_file_name(format!("TASK-{number:03}.json"))).unwrap();
        }
        fs::write(
            &index_file,
            render_task(&second_index, TaskDocumentKind::Index).unwrap(),
        )
        .unwrap();
        fs::write(&second_path, b"broken unrelated item").unwrap();
        assert_eq!(
            recheck_task_execution_context(&paths, &repository, &selected)
                .unwrap_err()
                .reason_code,
            "execute_worktree_task_changed"
        );
        assert!(
            load_task_execution_context(
                &work,
                &skills,
                &paths,
                &repository,
                &[],
                task_path,
                "TASK-001",
            )
            .is_err()
        );
        fs::write(&second_path, &second_raw).unwrap();
        let snapshot_path =
            root.join("outputs/work/sources/issue55-task-parity/SRC-001/source.txt");
        let original_source = fs::read(&snapshot_path).unwrap();
        let mut changed_source = original_source.clone();
        changed_source[0] ^= 1;
        fs::write(&snapshot_path, &changed_source).unwrap();
        assert_eq!(
            recheck_task_execution_context(&paths, &repository, &selected)
                .unwrap_err()
                .reason_code,
            "source_hash_mismatch"
        );
        fs::write(&snapshot_path, original_source).unwrap();
        fs::write(&index_file, &index_raw).unwrap();
        fs::remove_file(&second_path).unwrap();
        let execute = select(
            &work,
            "execute",
            &[],
            &["execute.general.execution-records".into()],
        )
        .unwrap();
        let execute_sha = execute.instructions_sha256.clone();
        let skill_sha = skill_selection_sha256("base_only", &[]);
        let authorization = json!({"schema":"work-attempt-authorization",
            "task_id":"TASK-001","commands":[],"validations":[item["validations"][0]],
            "modifiable_files":[],"working_directories":[],"external_operations":[],
            "allowed_deviations":[],"reapproval_conditions":["scope_expansion",
                "source_or_worktree_drift","failure_divergence","retry","recovery","unknown_result"],
            "authorization_evidence":"Approved"});
        let attempt = json!({"acceptance_results":work_operations::execution::acceptance::pending(work_operations::execution::acceptance::task_ids(&item)),"schema":"work-attempt","attempt_id":"ATTEMPT-001",
            "task_spec_id":"TASK-SPEC-001","task_id":"TASK-001","skill_id":null,
            "status":"in_progress","task_collection_sha256":context.validation["task_collection_sha256"],
            "task_index_sha256":context.validation["task_index_sha256"],
            "task_item_sha256":context.validation["task_item_sha256"]["TASK-001"],
            "task_instructions_sha256":context.validation["task_instructions_sha256"]["TASK-001"],
            "execute_instructions_sha256":execute_sha,
            "hierarchy_selection_sha256":context.validation["hierarchy_selection_sha256"],
            "execute_skill_selection_sha256":skill_sha,
            "authorization_sha256":canonical_json_sha256(&authorization).unwrap(),
            "authorization":authorization,"started_at":"2026-09-01T10:00+08:00","records":[]});
        let execution_dir = context.contract["artifacts"]["execution"].as_str().unwrap();
        let initial_index =
            build_initial_execution_index(&result["collection_contract"], &result).unwrap();
        let execution_index_file = root.join(format!("{execution_dir}/index.json"));
        fs::create_dir_all(execution_index_file.parent().unwrap()).unwrap();
        fs::write(
            &execution_index_file,
            render_execution_index(&initial_index).unwrap(),
        )
        .unwrap();
        let start_storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let start_sources = CommandProjectSources {
            instructions: &work,
            skills: &skills,
            paths: &paths,
            task_repository: &repository,
            skill_roots: &[],
        };
        let start_target = ExecutionProjectTarget {
            task_path,
            execution_dir,
            task_id: "TASK-001",
        };
        let initial_preflight = prepare_execute_preflight_from_project(
            &start_sources,
            &start_storage,
            start_target,
            &[],
        )
        .unwrap();
        assert_eq!(initial_preflight["task_status"], "pending");
        let worktree =
            inspect_worktree_from_project(&start_sources, &start_storage, start_target, &[])
                .unwrap();
        let start_choice = json!({"command_positions":[],"validation_positions":[1],
            "modifiable_files":[],"external_operation_positions":[],"allowed_deviations":[],
            "authorization_evidence":"Approved","carried_records":[]});
        let invalid_choice = prepare_attempt_start_from_project(
            &start_sources,
            &start_storage,
            ExecutionProjectTarget {
                task_path: "missing.json",
                ..start_target
            },
            &[],
            &json!({"authorization":{}}),
        )
        .unwrap_err();
        assert_eq!(invalid_choice.reason_code, "invalid_object_fields");
        let counted_start_tasks = CountedTaskStorage {
            inner: &repository,
            reads: RefCell::new(Vec::new()),
        };
        let counted_start_sources = CommandProjectSources {
            instructions: &work,
            skills: &skills,
            paths: &paths,
            task_repository: &counted_start_tasks,
            skill_roots: &[],
        };
        let prepared_start = prepare_attempt_start_from_project(
            &counted_start_sources,
            &start_storage,
            start_target,
            &[],
            &start_choice,
        )
        .unwrap();
        // Include the post-worktree and final preparation collection rechecks.
        assert_eq!(counted_start_tasks.reads.borrow().len(), 16);
        assert_eq!(prepared_start["request"]["authorization"], authorization);
        assert_eq!(
            prepared_start["request"]["worktree_snapshot_sha256"],
            worktree["snapshot_sha256"]
        );
        let mut expanded = authorization.clone();
        expanded["validations"] = json!([{"id":"VAL-002"}]);
        let changing = ChangingTaskStorage {
            inner: &repository,
            target_reads: Cell::new(0),
        };
        let changing_sources = CommandProjectSources {
            instructions: &work,
            skills: &skills,
            paths: &paths,
            task_repository: &changing,
            skill_roots: &[],
        };
        let drift_before_authorization = start_attempt_from_project(
            &changing_sources,
            &start_storage,
            start_target,
            &[],
            &json!({"schema":"work-attempt-start-request",
                "worktree_snapshot_sha256":worktree["snapshot_sha256"],
                "authorization":expanded.clone()}),
            "2026-09-01T10:00+08:00",
        )
        .unwrap_err();
        assert_eq!(
            drift_before_authorization.reason_code,
            "execute_worktree_task_changed"
        );
        assert!(changing.target_reads.get() > 1);
        let rejected_scope = start_attempt_from_project(
            &start_sources,
            &start_storage,
            start_target,
            &[],
            &json!({"schema":"work-attempt-start-request",
                "worktree_snapshot_sha256":worktree["snapshot_sha256"],
                "authorization":expanded}),
            "2026-09-01T10:00+08:00",
        )
        .unwrap_err();
        assert_eq!(
            rejected_scope.reason_code,
            "attempt_authorization_scope_expansion"
        );
        assert_eq!(
            fs::read(&execution_index_file).unwrap(),
            render_execution_index(&initial_index).unwrap()
        );
        let start_request = json!({"schema":"work-attempt-start-request",
            "worktree_snapshot_sha256":worktree["snapshot_sha256"],
            "authorization":authorization});
        let started = start_attempt_from_project(
            &start_sources,
            &start_storage,
            start_target,
            &[],
            &start_request,
            "2026-09-01T10:00+08:00",
        )
        .unwrap();
        assert_eq!(started["attempt_id"], "ATTEMPT-001");
        let started_index_bytes = fs::read(&execution_index_file).unwrap();
        let mut interrupted_index = initial_index.clone();
        interrupted_index["lock"] = build_execution_lock("TASK-001", "ATTEMPT-001", &execute_sha);
        fs::write(
            &execution_index_file,
            render_execution_index(&interrupted_index).unwrap(),
        )
        .unwrap();
        let changing_recovery = ChangingTaskStorage {
            inner: &repository,
            target_reads: Cell::new(0),
        };
        let changing_recovery_sources = CommandProjectSources {
            instructions: &work,
            skills: &skills,
            paths: &paths,
            task_repository: &changing_recovery,
            skill_roots: &[],
        };
        let recovery_drift = start_storage
            .recover_attempt_start_from_project(
                &changing_recovery_sources,
                AttemptStartRecoveryRequest {
                    target: start_target,
                    request: &start_request,
                    confirmed_inputs: &[],
                    started_at: "2026-09-01T10:00+08:00",
                },
            )
            .unwrap_err();
        assert_eq!(recovery_drift.reason_code, "execute_worktree_task_changed");
        assert!(changing_recovery.target_reads.get() > 1);
        let started_attempt_path =
            root.join(format!("{execution_dir}/TASK-001/ATTEMPT-001/attempt.json"));
        let started_attempt_bytes = fs::read(&started_attempt_path).unwrap();
        fs::remove_file(&started_attempt_path).unwrap();
        fs::remove_dir(started_attempt_path.parent().unwrap()).unwrap();
        let recovered_missing_directory = start_storage
            .recover_attempt_start_from_project(
                &start_sources,
                AttemptStartRecoveryRequest {
                    target: start_target,
                    request: &start_request,
                    confirmed_inputs: &[],
                    started_at: "2026-09-01T10:00+08:00",
                },
            )
            .unwrap();
        assert_eq!(recovered_missing_directory["status"], "recovered");
        assert_eq!(
            fs::read(&started_attempt_path).unwrap(),
            started_attempt_bytes
        );
        fs::write(
            &execution_index_file,
            render_execution_index(&interrupted_index).unwrap(),
        )
        .unwrap();
        fs::remove_file(&started_attempt_path).unwrap();
        let recovered_empty_directory = start_storage
            .recover_attempt_start_from_project(
                &start_sources,
                AttemptStartRecoveryRequest {
                    target: start_target,
                    request: &start_request,
                    confirmed_inputs: &[],
                    started_at: "2026-09-01T10:00+08:00",
                },
            )
            .unwrap();
        assert_eq!(recovered_empty_directory["status"], "recovered");
        assert_eq!(
            fs::read(&started_attempt_path).unwrap(),
            started_attempt_bytes
        );
        fs::write(
            &execution_index_file,
            render_execution_index(&interrupted_index).unwrap(),
        )
        .unwrap();
        let start_recovery = start_storage
            .recover_attempt_start_from_project(
                &start_sources,
                AttemptStartRecoveryRequest {
                    target: start_target,
                    request: &start_request,
                    confirmed_inputs: &[],
                    started_at: "2026-09-01T10:00+08:00",
                },
            )
            .unwrap();
        assert_eq!(start_recovery["status"], "recovered");
        assert_eq!(
            fs::read(&execution_index_file).unwrap(),
            started_index_bytes
        );
        let mut execution_index = json!({"acceptance_results":initial_index["acceptance_results"],"schema":"work-execution-index",
            "requirement_id":"issue55-task-parity","title":"Execution",
            "task_spec_id":"TASK-SPEC-001",
            "task_collection_sha256":context.validation["task_collection_sha256"],
            "task_index_sha256":context.validation["task_index_sha256"],
            "task_instructions_sha256":context.validation["instructions_sha256"],
            "hierarchy_selection_sha256":context.validation["hierarchy_selection_sha256"],
            "skill_selection_sha256":skill_sha,"overall_status":"in_progress",
            "lock":build_execution_lock("TASK-001","ATTEMPT-001",&execute_sha),
            "tasks":[{"id":"TASK-001","status":"in_progress","skill_id":null,
                "task_item_sha256":context.validation["task_item_sha256"]["TASK-001"],
                "acceptance_results":initial_index["tasks"][0]["acceptance_results"],"instructions_sha256":context.validation["task_instructions_sha256"]["TASK-001"],
                "latest_attempt":"ATTEMPT-001"}]});
        execution_index["lock"]["record_id"] = json!("VAL-001");
        let attempt_path = root.join(format!("{execution_dir}/TASK-001/ATTEMPT-001/attempt.json"));
        fs::create_dir_all(attempt_path.parent().unwrap()).unwrap();
        fs::write(
            root.join(format!("{execution_dir}/index.json")),
            render_execution_index(&execution_index).unwrap(),
        )
        .unwrap();
        fs::write(&attempt_path, render_attempt(&attempt).unwrap()).unwrap();
        let execution_storage = LocalExecutionStorage {
            project_root: root.clone(),
        };
        let sources = CommandProjectSources {
            instructions: &work,
            skills: &skills,
            paths: &paths,
            task_repository: &repository,
            skill_roots: &[],
        };
        let missing_request = json!({"schema":"work-command-run-request","timeout_seconds":60});
        let input = CommandProjectRequest {
            task_path,
            execution_dir,
            task_id: "TASK-001",
            request: &missing_request,
        };
        let rejected =
            prepare_command_from_project(&sources, &execution_storage, input).unwrap_err();
        assert_eq!(rejected.reason_code, "command_correction_command_not_found");
        let (os, shell, original_argv, reviewed_argv) = if cfg!(windows) {
            (
                "windows",
                "powershell",
                json!([
                    "powershell.exe",
                    "-NoProfile",
                    "-Command",
                    "[Console]::Write('verified')"
                ]),
                json!([
                    "powershell.exe",
                    "-NoProfile",
                    "-Command",
                    "[Console]::Write('reviewed')"
                ]),
            )
        } else {
            (
                "macos",
                "sh",
                json!(["/usr/bin/printf", "verified"]),
                json!(["/usr/bin/printf", "reviewed"]),
            )
        };
        let mut command_item = item.clone();
        command_item["commands"] = json!([{"id":"CMD-001","mode":"argv",
            "argv":original_argv}]);
        command_item["steps"][0]["references"] = json!(["CMD-001", "VAL-001"]);
        let command_item_raw = render_task(&command_item, TaskDocumentKind::Item).unwrap();
        let mut command_index = index.clone();
        command_index["tasks"][0]["canonical_sha256"] = json!(sha256_hex(&command_item_raw));
        command_index["execution_defaults"] = json!({"working_directory":".",
            "os":os,"shell":shell});
        fs::write(&item_path, &command_item_raw).unwrap();
        fs::write(
            &index_file,
            render_task(&command_index, TaskDocumentKind::Index).unwrap(),
        )
        .unwrap();
        let command_context = load_task_execution_context(
            &work,
            &skills,
            &paths,
            &repository,
            &[],
            task_path,
            "TASK-001",
        )
        .unwrap();
        let mut command_authorization = authorization.clone();
        command_authorization["commands"] = json!([command_item["commands"][0]]);
        command_authorization["working_directories"] = json!(["."]);
        let deviation_action = json!({"kind":"replace_command","record_id":"CMD-001",
            "replacement":{"mode":"argv","argv":reviewed_argv}});
        command_authorization["allowed_deviations"] = json!([deviation_action]);
        let mut command_attempt = attempt.clone();
        command_attempt["task_collection_sha256"] =
            command_context.validation["task_collection_sha256"].clone();
        command_attempt["task_index_sha256"] =
            command_context.validation["task_index_sha256"].clone();
        command_attempt["task_item_sha256"] =
            command_context.validation["task_item_sha256"]["TASK-001"].clone();
        command_attempt["authorization_sha256"] =
            json!(canonical_json_sha256(&command_authorization).unwrap());
        command_attempt["authorization"] = command_authorization;
        let mut command_execution_index = execution_index.clone();
        command_execution_index["task_collection_sha256"] =
            command_context.validation["task_collection_sha256"].clone();
        command_execution_index["task_index_sha256"] =
            command_context.validation["task_index_sha256"].clone();
        command_execution_index["tasks"][0]["task_item_sha256"] =
            command_context.validation["task_item_sha256"]["TASK-001"].clone();
        command_execution_index["lock"]["record_id"] = json!("CMD-001");
        fs::write(
            root.join(format!("{execution_dir}/index.json")),
            render_execution_index(&command_execution_index).unwrap(),
        )
        .unwrap();
        fs::write(&attempt_path, render_attempt(&command_attempt).unwrap()).unwrap();
        let command_request = json!({"schema":"work-command-run-request","timeout_seconds":60});
        let input = CommandProjectRequest {
            task_path,
            execution_dir,
            task_id: "TASK-001",
            request: &command_request,
        };
        let command_index_before = fs::read(&execution_index_file).unwrap();
        let command_attempt_before = fs::read(&attempt_path).unwrap();
        let command_lock = root.join(format!("{execution_dir}/.work-state-writer.lock"));
        // The isolated actor owns this released legacy kernel-lock test artifact.
        // Native production gates never remove it or infer a dead owner.
        if command_lock.exists() {
            fs::remove_file(&command_lock).unwrap();
        }
        let load_command_context = || {
            work_feature::execution::load_execution_writer_context(
                &sources,
                ExecutionProjectTarget {
                    task_path,
                    execution_dir,
                    task_id: "TASK-001",
                },
            )
        };
        let command_storage = crate::execution::storage::RuntimeExecutionSession {
            storage: &execution_storage,
            load_context: &load_command_context,
        };
        let command_lock_before = fs::read(&command_lock).ok();
        // A repository without a trusted executor still cannot self-grant a boundary.
        let rejected =
            prepare_command_from_project(&sources, &execution_storage, input).unwrap_err();
        assert_eq!(
            rejected.reason_code,
            "file_transaction_effect_boundary_unverified"
        );
        let approval_rejection = if cfg!(any(target_os = "macos", windows)) {
            let isolated = prepare_command_from_project(&sources, &command_storage, input).unwrap();
            let policy = &isolated["execution"]["work_isolation"];
            assert_eq!(policy["network_access"], false);
            assert_eq!(
                policy["read_only_project_root"],
                root.canonicalize().unwrap().to_string_lossy().as_ref()
            );
            assert!(policy["read_only_project_view"].as_str().is_some());
            assert!(!std::path::Path::new(policy["writable_directory"].as_str().unwrap()).exists());
            "command_run_approval_changed"
        } else {
            assert_eq!(
                prepare_command_from_project(&sources, &command_storage, input)
                    .unwrap_err()
                    .reason_code,
                "file_transaction_effect_boundary_unverified"
            );
            "file_transaction_effect_boundary_unverified"
        };
        assert_eq!(
            fs::read(&execution_index_file).unwrap(),
            command_index_before
        );
        assert_eq!(fs::read(&attempt_path).unwrap(), command_attempt_before);
        assert_eq!(fs::read(&command_lock).ok(), command_lock_before);
        // A fabricated preview hash or repeated request cannot grant a capability.
        let approved = "a".repeat(64);
        let rejected = command_storage
            .run_command_from_project(&sources, input, &approved)
            .unwrap_err();
        assert_eq!(rejected.reason_code, approval_rejection);
        assert!(
            !root
                .join(format!("{execution_dir}/TASK-001/ATTEMPT-001/receipts"))
                .exists()
        );
        assert_eq!(
            fs::read(&execution_index_file).unwrap(),
            command_index_before
        );
        assert_eq!(fs::read(&attempt_path).unwrap(), command_attempt_before);
        assert_eq!(
            command_storage
                .run_command_from_project(&sources, input, &approved)
                .unwrap_err()
                .reason_code,
            approval_rejection
        );
        let index_before_correction = fs::read(&execution_index_file).unwrap();
        let rejected_correction = record_command_correction_from_project(
            &sources,
            &execution_storage,
            ExecutionProjectTarget {
                task_path,
                execution_dir,
                task_id: "TASK-001",
            },
            &json!({"schema":"wrong","actual_command":{"mode":"argv",
                "argv":reviewed_argv},"reason":"Review"}),
        )
        .unwrap_err();
        assert_eq!(
            rejected_correction.reason_code,
            "command_correction_invalid_schema"
        );
        assert_eq!(
            fs::read(&execution_index_file).unwrap(),
            index_before_correction
        );
        let corrected = record_command_correction_from_project(
            &sources,
            &execution_storage,
            ExecutionProjectTarget {
                task_path,
                execution_dir,
                task_id: "TASK-001",
            },
            &json!({"schema":"work-command-correction-request",
                "actual_command":{"mode":"argv","argv":reviewed_argv},
                "reason":"Use reviewed command."}),
        )
        .unwrap();
        assert_eq!(corrected["correction_status"], "recorded");
        let proposal = json!({"schema":"work-execution-deviation-proposal",
            "task_id":"TASK-001","attempt_id":"ATTEMPT-001","anchor_record_id":"CMD-001",
            "task_basis":["CMD-001"],"gap":"An equivalent command was approved.",
            "action":deviation_action,"modifiable_files":[],
            "impact":{"summary":"Use the reviewed command.","requirement_changed":false,
                "scope_changed":false,"acceptance_criteria_changed":false,
                "deliverables_changed":false,"safety_boundary_changed":false,
                "external_side_effect_boundary_changed":false},
            "side_effects":["Runs the reviewed command."]});
        let index_before_preview = fs::read(&execution_index_file).unwrap();
        let attempt_before_preview = fs::read(&attempt_path).unwrap();
        let semantic_request = json!({"gap":proposal["gap"],
            "action":{"kind":"replace_command","replacement":proposal["action"]["replacement"]},
            "modifiable_files":proposal["modifiable_files"],
            "impact":proposal["impact"],"side_effects":proposal["side_effects"]});
        let semantic_preview = prepare_semantic_deviation_from_project(
            &sources,
            &execution_storage,
            ExecutionProjectTarget {
                task_path,
                execution_dir,
                task_id: "TASK-001",
            },
            &semantic_request,
        )
        .unwrap();
        assert_eq!(semantic_preview["proposal"]["task_id"], "TASK-001");
        assert_eq!(semantic_preview["proposal"]["attempt_id"], "ATTEMPT-001");
        assert_eq!(semantic_preview["proposal"]["anchor_record_id"], "CMD-001");
        assert_eq!(
            semantic_preview["proposal"]["action"]["record_id"],
            "CMD-001"
        );
        assert_eq!(
            semantic_preview["proposal"]["task_basis"],
            json!(["CMD-001", "STEP-001"])
        );
        assert_eq!(
            fs::read(&execution_index_file).unwrap(),
            index_before_preview
        );
        assert_eq!(fs::read(&attempt_path).unwrap(), attempt_before_preview);
        let mut stale_index = command_execution_index.clone();
        stale_index["lock"]["task_id"] = json!("TASK-002");
        fs::write(
            &execution_index_file,
            render_execution_index(&stale_index).unwrap(),
        )
        .unwrap();
        let stale = prepare_semantic_deviation_from_project(
            &sources,
            &execution_storage,
            ExecutionProjectTarget {
                task_path,
                execution_dir,
                task_id: "TASK-001",
            },
            &semantic_request,
        )
        .unwrap_err();
        assert_eq!(stale.reason_code, "deviation_active_record");
        fs::write(&execution_index_file, &index_before_preview).unwrap();
        let deviation_input = DeviationProjectRequest {
            task_path,
            execution_dir,
            task_id: "TASK-001",
            proposal: &proposal,
        };
        let deviation_preview =
            prepare_deviation_from_project(&sources, &execution_storage, deviation_input).unwrap();
        assert_eq!(deviation_preview["action_validation"], "passed");
        assert_eq!(deviation_preview["semantic_review"], "required");
        assert_eq!(deviation_preview["classification"], "task_only");
        assert_eq!(deviation_preview["blocking"], false);
        assert_eq!(
            deviation_preview["preview_sha256"].as_str().unwrap().len(),
            64
        );
        assert_eq!(
            fs::read(&execution_index_file).unwrap(),
            index_before_preview
        );
        assert_eq!(fs::read(&attempt_path).unwrap(), attempt_before_preview);
        let mut wrong_action = proposal.clone();
        wrong_action["action"]["record_id"] = json!("CMD-002");
        assert_eq!(
            prepare_deviation_from_project(
                &sources,
                &execution_storage,
                DeviationProjectRequest {
                    task_path,
                    execution_dir,
                    task_id: "TASK-001",
                    proposal: &wrong_action,
                },
            )
            .unwrap_err()
            .reason_code,
            "invalid_contract_value"
        );
        let mut different_lock = command_execution_index.clone();
        different_lock["lock"]["record_id"] = json!("CMD-002");
        fs::write(
            &execution_index_file,
            render_execution_index(&different_lock).unwrap(),
        )
        .unwrap();
        assert_eq!(
            prepare_deviation_from_project(
                &sources,
                &execution_storage,
                DeviationProjectRequest {
                    task_path,
                    execution_dir,
                    task_id: "TASK-001",
                    proposal: &proposal,
                },
            )
            .unwrap_err()
            .reason_code,
            "deviation_active_record"
        );
        fs::write(&execution_index_file, &index_before_preview).unwrap();
        let mut changed_gap = proposal.clone();
        changed_gap["gap"] = json!("A different reviewed gap.");
        let changed_preview = prepare_deviation_from_project(
            &sources,
            &execution_storage,
            DeviationProjectRequest {
                task_path,
                execution_dir,
                task_id: "TASK-001",
                proposal: &changed_gap,
            },
        )
        .unwrap();
        assert_ne!(
            changed_preview["preview_sha256"],
            deviation_preview["preview_sha256"]
        );
        let approved_deviation = deviation_preview["preview_sha256"].as_str().unwrap();
        let deviation_record = record_deviation_from_project(
            &sources,
            &execution_storage,
            deviation_input,
            approved_deviation,
            "Fresh approval",
        )
        .unwrap();
        assert_eq!(deviation_record["deviation_id"], "DEVIATION-001");
        assert_eq!(deviation_record["record_status"], "recorded");
        let committed_raw = fs::read(&attempt_path).unwrap();
        let committed: serde_json::Value = serde_json::from_slice(&committed_raw).unwrap();
        let mut prepared = committed.clone();
        let mut second = committed["execution_deviations"][0].clone();
        second["deviation_id"] = json!("DEVIATION-002");
        second["approved_preview_sha256"] = json!("b".repeat(64));
        second["supplemental_authorization"]["preview_sha256"] = json!("b".repeat(64));
        let added_validation = json!({"kind":"add_validation","validation":{
            "id":"VAL-002","kind":"manual","confirmer":"user","criteria":"Reviewed."}});
        second["proposal"]["action"] = added_validation.clone();
        second["supplemental_authorization"]["action"] = added_validation;
        prepared["execution_deviations"]
            .as_array_mut()
            .unwrap()
            .push(second);
        let temporary = root.join(format!(
            "{execution_dir}/.work-deviation-record-TASK-001-ATTEMPT-001-CMD-001-attempt.tmp"
        ));
        fs::write(&temporary, render_attempt(&prepared).unwrap()).unwrap();
        let foreign = root.join(format!(
            "{execution_dir}/.work-attempt-start-TASK-001-ATTEMPT-001-lock.tmp"
        ));
        fs::write(&foreign, b"preserved").unwrap();
        let mixed = prepare_recovery_from_project(
            &sources,
            &execution_storage,
            ExecutionProjectTarget {
                task_path,
                execution_dir,
                task_id: "TASK-001",
            },
            &json!({"schema":"work-execution-recovery-prepare-request",
                "transaction":"deviation_record","attempt_id":"ATTEMPT-001"}),
        )
        .unwrap_err();
        assert_eq!(mixed.reason_code, "recovery_prepare_mixed_transactions");
        assert_eq!(fs::read(&foreign).unwrap(), b"preserved");
        fs::remove_file(&foreign).unwrap();
        let recovery_request = json!({"schema":"work-execution-recovery-prepare-request",
            "transaction":"deviation_record","attempt_id":"ATTEMPT-001"});
        assert!(
            prepare_recovery_from_project(
                &sources,
                &execution_storage,
                ExecutionProjectTarget {
                    task_path,
                    execution_dir,
                    task_id: "TASK-999"
                },
                &recovery_request,
            )
            .is_err()
        );
        let actual_index_raw = fs::read(&execution_index_file).unwrap();
        let mut wrong_lock: serde_json::Value = serde_json::from_slice(&actual_index_raw).unwrap();
        wrong_lock["lock"]["execute_instructions_sha256"] = json!("d".repeat(64));
        fs::write(
            &execution_index_file,
            render_execution_index(&wrong_lock).unwrap(),
        )
        .unwrap();
        assert_eq!(
            prepare_recovery_from_project(
                &sources,
                &execution_storage,
                ExecutionProjectTarget {
                    task_path,
                    execution_dir,
                    task_id: "TASK-001"
                },
                &recovery_request,
            )
            .unwrap_err()
            .reason_code,
            "recovery_prepare_lock"
        );
        fs::write(&execution_index_file, &actual_index_raw).unwrap();
        let preserved_temporary = fs::read(&temporary).unwrap();
        fs::write(&temporary, b"{\"partial\":").unwrap();
        assert!(
            prepare_recovery_from_project(
                &sources,
                &execution_storage,
                ExecutionProjectTarget {
                    task_path,
                    execution_dir,
                    task_id: "TASK-001"
                },
                &recovery_request,
            )
            .is_err()
        );
        assert_eq!(fs::read(&temporary).unwrap(), b"{\"partial\":");
        fs::write(&temporary, &preserved_temporary).unwrap();
        let guard = LocalWriterLock
            .acquire(&root.join(format!("{execution_dir}/.work-state-writer.lock")))
            .unwrap();
        assert_eq!(
            prepare_recovery_from_project(
                &sources,
                &execution_storage,
                ExecutionProjectTarget {
                    task_path,
                    execution_dir,
                    task_id: "TASK-001"
                },
                &recovery_request,
            )
            .unwrap_err()
            .reason_code,
            "work_state_writer_busy"
        );
        drop(guard);
        assert_eq!(fs::read(&temporary).unwrap(), preserved_temporary);
        for change_inventory in [false, true] {
            let changing_recovery = ChangingRecoveryStorage {
                inner: &execution_storage,
                inventory_calls: Cell::new(0),
                staged_reads: Cell::new(0),
                change_inventory,
                staging_change: None,
            };
            assert_eq!(
                prepare_recovery_from_project(
                    &sources,
                    &changing_recovery,
                    ExecutionProjectTarget {
                        task_path,
                        execution_dir,
                        task_id: "TASK-001"
                    },
                    &recovery_request,
                )
                .unwrap_err()
                .reason_code,
                "recovery_prepare_source_changed"
            );
            assert_eq!(fs::read(&temporary).unwrap(), preserved_temporary);
            assert_eq!(fs::read(&execution_index_file).unwrap(), actual_index_raw);
        }
        let before_recovery_index = fs::read(&execution_index_file).unwrap();
        let before_recovery_attempt = fs::read(&attempt_path).unwrap();
        let before_recovery_stage = fs::read(&temporary).unwrap();
        let recovery_lock = root.join(format!("{execution_dir}/.work-state-writer.lock"));
        let before_recovery_lock = fs::read(&recovery_lock).ok();
        let deviation_recovery_preview = prepare_recovery_from_project(
            &sources,
            &execution_storage,
            ExecutionProjectTarget {
                task_path,
                execution_dir,
                task_id: "TASK-001",
            },
            &json!({"schema":"work-execution-recovery-prepare-request",
                "transaction":"deviation_record","attempt_id":"ATTEMPT-001"}),
        )
        .unwrap();
        assert_eq!(
            deviation_recovery_preview["request"]["transaction_files"],
            json!([temporary.file_name().unwrap().to_string_lossy().as_ref()])
        );
        assert_eq!(deviation_recovery_preview["recovery_authorized"], false);
        assert_eq!(
            deviation_recovery_preview["recovery_validation"],
            "requires_authorized_recover"
        );
        assert!(
            deviation_recovery_preview["evidence"]
                .as_object()
                .unwrap()
                .contains_key(task_path)
        );
        assert_eq!(
            fs::read(&execution_index_file).unwrap(),
            before_recovery_index
        );
        assert_eq!(fs::read(&attempt_path).unwrap(), before_recovery_attempt);
        assert_eq!(fs::read(&temporary).unwrap(), before_recovery_stage);
        assert_eq!(fs::read(&recovery_lock).ok(), before_recovery_lock);
        let recovered = execution_storage
            .recover_legacy_execution_from_project(
                &sources,
                ExecutionProjectTarget {
                    task_path,
                    execution_dir,
                    task_id: "TASK-001",
                },
                &deviation_recovery_preview["request"],
            )
            .unwrap();
        assert_eq!(recovered["transaction"], "deviation_record");
        let recovered_raw = fs::read(&attempt_path).unwrap();
        assert_eq!(recovered_raw, render_attempt(&prepared).unwrap());
        let target = ExecutionProjectTarget {
            task_path,
            execution_dir,
            task_id: "TASK-001",
        };
        let finished = finish_record_from_project(
            &sources,
            &execution_storage,
            target,
            &json!({"schema":"work-record-finish-request",
                "record":{"exit_code":0,"result":"Command verified."}}),
        )
        .unwrap();
        assert_eq!(finished["record_status"], "recorded");
        let reserved =
            begin_record_from_project(&sources, &execution_storage, target, "VAL-001", None)
                .unwrap();
        assert_eq!(reserved["record_id"], "VAL-001");
        let validated = finish_record_from_project(
            &sources,
            &execution_storage,
            target,
            &json!({"schema":"work-record-finish-request",
                "record":{"outcome":"passed","evidence":"Reviewed."}}),
        )
        .unwrap();
        assert_eq!(validated["record_kind"], "validation");
        let validation_attempt_raw = fs::read(&attempt_path).unwrap();
        let validation_attempt: Value = serde_json::from_slice(&validation_attempt_raw).unwrap();
        assert!(
            validation_attempt["acceptance_results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["status"] == "completed"
                    && row["evidence"][0]["record_id"] == "VAL-001")
        );
        let validation_index: Value =
            serde_json::from_slice(&fs::read(&execution_index_file).unwrap()).unwrap();
        assert_eq!(
            validation_index["tasks"][0]["acceptance_results"],
            validation_attempt["acceptance_results"]
        );

        let closed = close_attempt_from_project(
            &sources,
            &execution_storage,
            target,
            &json!({"schema":"work-attempt-close-request","status":"stopped",
                "final_type":"user_stopped","reason":"Stop after review.",
                "authorization_evidence":"Fresh close approval"}),
            "2026-09-01T11:00+08:00",
        )
        .unwrap();
        assert_eq!(closed["attempt_status"], "stopped");
        let stopped_attempt_raw = fs::read(&attempt_path).unwrap();
        let stopped_attempt: Value = serde_json::from_slice(&stopped_attempt_raw).unwrap();
        assert_eq!(stopped_attempt["records"], validation_attempt["records"]);
        assert!(
            stopped_attempt["acceptance_results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["status"] == "pending"
                    && row["evidence"].as_array().unwrap().is_empty())
        );
        let stopped_index: Value =
            serde_json::from_slice(&fs::read(&execution_index_file).unwrap()).unwrap();
        assert_eq!(
            stopped_index["tasks"][0]["acceptance_results"],
            stopped_attempt["acceptance_results"]
        );

        let correction = create_correction_from_project(
            &sources,
            &execution_storage,
            target,
            &json!({"schema":"work-correction-create-request",
                "target_attempt_id":"ATTEMPT-001","field":"records[0].result",
                "correct_value":"Reviewed","reason":"Correct the record narrative.",
                "invalidates_completion":false}),
            "2026-09-01T11:05+08:00",
        )
        .unwrap();
        assert_eq!(correction["correction_id"], "ATTEMPT-001-CORRECTION-001");
        assert_eq!(correction["lock_status"], "released");
        let first_correction_path = root.join(correction["correction_path"].as_str().unwrap());
        let first_correction_raw = fs::read(&first_correction_path).unwrap();
        let first_correction: serde_json::Value =
            serde_json::from_slice(&first_correction_raw).unwrap();
        assert_eq!(
            first_correction["task_collection_sha256"],
            command_context.validation["task_collection_sha256"]
        );
        assert_eq!(
            first_correction["task_index_sha256"],
            command_context.validation["task_index_sha256"]
        );
        assert_eq!(
            first_correction["task_item_sha256"],
            command_context.validation["task_item_sha256"]["TASK-001"]
        );
        let correction_index_before = fs::read(&execution_index_file).unwrap();
        let correction_attempt_before = fs::read(&attempt_path).unwrap();
        let closed_attempt: serde_json::Value =
            serde_json::from_slice(&correction_attempt_before).unwrap();
        assert_eq!(
            closed_attempt["task_collection_sha256"],
            command_context.validation["task_collection_sha256"]
        );
        assert_eq!(
            closed_attempt["task_index_sha256"],
            command_context.validation["task_index_sha256"]
        );
        assert_eq!(
            closed_attempt["task_item_sha256"],
            command_context.validation["task_item_sha256"]["TASK-001"]
        );
        let correction_index: serde_json::Value =
            serde_json::from_slice(&correction_index_before).unwrap();
        let correction_attempt: serde_json::Value =
            serde_json::from_slice(&correction_attempt_before).unwrap();
        let retry_prepared = prepare_attempt_start_from_project(
            &sources,
            &execution_storage,
            target,
            &[],
            &json!({"command_positions":[1],"validation_positions":[1],
                "modifiable_files":[],"external_operation_positions":[],
                "allowed_deviations":[],"authorization_evidence":"Fresh retry approval",
                "carried_records":[{"position":2,"evidence":"Review remains valid."}]}),
        )
        .unwrap();
        assert_eq!(
            retry_prepared["request"]["continuation"]["source_attempt_id"],
            "ATTEMPT-001"
        );
        assert_eq!(
            retry_prepared["request"]["continuation"]["carried_records"][0]["record_id"],
            "VAL-001"
        );
        let second_request = json!({"schema":"work-correction-create-request",
            "target_attempt_id":"ATTEMPT-001","field":"records[1].evidence",
            "correct_value":"Reviewed again","reason":"Correct validation narrative.",
            "invalidates_completion":false});
        let second_id = "ATTEMPT-001-CORRECTION-002";
        let second_candidates = build_correction_candidates(
            &result["collection_contract"],
            &correction_index,
            &correction_attempt,
            "TASK-001",
            &second_request,
            second_id,
            "2026-09-01T11:06+08:00",
        )
        .unwrap();
        let prefix = format!("{execution_dir}/.work-correction-TASK-001-{second_id}");
        fs::write(
            root.join(format!("{prefix}-artifact.tmp")),
            render_correction(&second_candidates.artifact).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(format!("{prefix}-lock.tmp")),
            render_execution_index(&second_candidates.locked_index).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(format!("{prefix}-index.tmp")),
            render_execution_index(&second_candidates.final_index).unwrap(),
        )
        .unwrap();
        let second_preview = prepare_recovery_from_project(
            &sources,
            &execution_storage,
            target,
            &json!({"schema":"work-execution-recovery-prepare-request",
                "transaction":"correction","attempt_id":"ATTEMPT-001"}),
        )
        .unwrap();
        let recovered_correction = execution_storage
            .recover_legacy_execution_from_project(&sources, target, &second_preview["request"])
            .unwrap();
        assert_eq!(recovered_correction["status"], "recovered");
        assert_eq!(
            fs::read(&execution_index_file).unwrap(),
            render_execution_index(&second_candidates.final_index).unwrap()
        );
        assert_eq!(fs::read(&attempt_path).unwrap(), correction_attempt_before);
        assert_eq!(
            fs::read(&first_correction_path).unwrap(),
            first_correction_raw
        );
        let second_correction_path = root.join(format!(
            "{execution_dir}/TASK-001/ATTEMPT-001/corrections/{second_id}.json"
        ));
        let second_correction_raw = fs::read(&second_correction_path).unwrap();
        let second_correction: serde_json::Value =
            serde_json::from_slice(&second_correction_raw).unwrap();
        assert_eq!(second_correction["correction_id"], second_id);
        assert_eq!(
            second_correction["task_collection_sha256"],
            command_context.validation["task_collection_sha256"]
        );
        assert_eq!(
            second_correction["task_index_sha256"],
            command_context.validation["task_index_sha256"]
        );
        assert_eq!(
            second_correction["task_item_sha256"],
            command_context.validation["task_item_sha256"]["TASK-001"]
        );
        assert!(!root.join(format!("{prefix}-artifact.tmp")).exists());
        assert!(!root.join(format!("{prefix}-lock.tmp")).exists());
        assert!(!root.join(format!("{prefix}-index.tmp")).exists());
        let third_index_raw = fs::read(&execution_index_file).unwrap();
        let third_index: serde_json::Value = serde_json::from_slice(&third_index_raw).unwrap();
        let third_id = "ATTEMPT-001-CORRECTION-003";
        let third_candidates = build_correction_candidates(
            &result["collection_contract"],
            &third_index,
            &correction_attempt,
            "TASK-001",
            &second_request,
            third_id,
            "2026-09-01T11:07+08:00",
        )
        .unwrap();
        let third_prefix = format!("{execution_dir}/.work-correction-TASK-001-{third_id}");
        fs::write(
            root.join(format!("{third_prefix}-artifact.tmp")),
            render_correction(&third_candidates.artifact).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(format!("{third_prefix}-lock.tmp")),
            render_execution_index(&third_candidates.locked_index).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join(format!("{third_prefix}-index.tmp")),
            render_execution_index(&third_candidates.final_index).unwrap(),
        )
        .unwrap();
        let recovery_preview = prepare_recovery_from_project(
            &sources,
            &execution_storage,
            target,
            &json!({"schema":"work-execution-recovery-prepare-request",
                "transaction":"correction","attempt_id":"ATTEMPT-001"}),
        )
        .unwrap();
        assert_eq!(recovery_preview["status"], "prepared");
        assert_eq!(
            recovery_preview["request"]["transaction_files"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        fs::write(root.join(format!("{third_prefix}-index.tmp")), b"{}\n").unwrap();
        let rejected = execution_storage
            .recover_correction(CorrectionRecoveryInput {
                execution_dir,
                task_id: "TASK-001",
                attempt_id: "ATTEMPT-001",
                correction_id: third_id,
                collection: &result["collection_contract"],
                index_before: &third_index_raw,
                attempt_before: &correction_attempt_before,
            })
            .unwrap_err();
        assert_eq!(
            rejected.reason_code,
            "correction_recovery_index_target_mismatch"
        );
        assert_eq!(fs::read(&execution_index_file).unwrap(), third_index_raw);
        let mut broken = index;
        broken["tasks"][0]["canonical_sha256"] = json!("0".repeat(64));
        let broken_raw = render_task(&broken, TaskDocumentKind::Index).unwrap();
        assert_eq!(
            validate_collection(
                &work,
                &skills,
                &paths,
                &[],
                CollectionInput {
                    index_raw: &broken_raw,
                    item_raw: &items,
                    index_path: task_path,
                }
            )
            .unwrap_err()
            .reason_code,
            "task_item_fingerprint_mismatch"
        );
    }
}
