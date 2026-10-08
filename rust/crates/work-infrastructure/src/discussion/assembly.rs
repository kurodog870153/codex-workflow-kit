//! Lock-held Session revalidation and exact formal publication.

use super::storage::LocalDiscussionStorage;
use crate::artifact_paths::LocalArtifactPaths;
use crate::files::LocalFiles;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::storage_path;
use crate::writer_lock::LocalWriterLock;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
pub use work_feature::discussion::assembly::PublicationRequest;
use work_feature::discussion::assembly::{self, PreparedDiscussion};
use work_feature::discussion::repository::DiscussionRepository;
use work_feature::error::WorkError;
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use work_model::discussion::DiscussionSession;

    pub(super) fn setup() -> (LocalDiscussionAssembly, DiscussionSession, Value) {
        setup_with_protocol(false)
    }

    fn setup_runtime() -> (LocalDiscussionAssembly, DiscussionSession, Value) {
        setup_with_protocol(true)
    }

    fn setup_with_protocol(runtime: bool) -> (LocalDiscussionAssembly, DiscussionSession, Value) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/cases/discussion/assembly/valid/input");
        let root = std::env::temp_dir().join(format!(
            "work-discussion-assembly-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        crate::fixture_support::copy_fixture_sources(&fixture.join("../project"), &root).unwrap();
        let session = crate::fixture_support::prepare_discussion_fixture(
            &fixture,
            &root,
            &repo.join("../skills/work"),
        )
        .unwrap();
        let storage = LocalDiscussionStorage {
            project_root: root.clone(),
            files: LocalFiles,
        };
        if runtime {
            storage.initialize_with_runtime(&session, false).unwrap();
        } else {
            storage.initialize(&session, false).unwrap();
        }
        let metadata =
            serde_json::from_slice(&fs::read(fixture.join("metadata.json")).unwrap()).unwrap();
        (
            LocalDiscussionAssembly {
                project_root: root,
                skill_root: repo.join("../skills/work"),
                skill_configs: vec![],
            },
            session,
            metadata,
        )
    }

    #[test]
    fn runtime_publication_orders_locks_and_releases_second_lock_failure() {
        use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};
        let (assembly, session, _) = setup_runtime();
        let context = work_feature::ports::RequirementWriterContext {
            canonical_project_root: assembly.project_root.clone(),
            requirement_id: session.requirement_id.parse().unwrap(),
        };
        for exclude in [false, true] {
            assembly
                .with_runtime_publication(&session, exclude, |discussion, execution| {
                    assert_eq!(discussion.class, work_model::runtime::LockClass::Discussion);
                    assert_eq!(execution.is_some(), exclude);
                    assert!(
                        LocalWriterLock
                            .require_runtime_idle(
                                &context,
                                work_model::runtime::LockClass::Discussion
                            )
                            .is_err()
                    );
                    if exclude {
                        assert!(
                            LocalWriterLock
                                .require_runtime_idle(
                                    &context,
                                    work_model::runtime::LockClass::Execution
                                )
                                .is_err()
                        );
                    }
                    Ok(())
                })
                .unwrap();
        }
        let execution = LocalWriterLock
            .acquire_runtime(&context, work_model::runtime::LockClass::Execution)
            .unwrap();
        let mut called = false;
        assert!(
            assembly
                .with_runtime_publication(&session, true, |_, _| {
                    called = true;
                    Ok(())
                })
                .is_err()
        );
        assert!(!called);
        LocalWriterLock
            .require_runtime_idle(&context, work_model::runtime::LockClass::Discussion)
            .unwrap();
        execution.release().unwrap();
        assert!(
            assembly
                .with_runtime_publication::<()>(&session, true, |_, _| {
                    Err(assembly::error("injected_publication_failure"))
                })
                .is_err()
        );
        for class in [
            work_model::runtime::LockClass::Discussion,
            work_model::runtime::LockClass::Execution,
        ] {
            LocalWriterLock
                .require_runtime_idle(&context, class)
                .unwrap();
        }
    }

    #[test]
    fn runtime_publication_excludes_task_create_and_recovers_exact_targets() {
        use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};
        let (adapter, session, metadata) = setup_runtime();
        let preview = adapter.preview("example", &metadata).unwrap();
        let approval = preview["approval_sha256"].as_str().unwrap();
        let request = |recovery| PublicationRequest {
            requirement_id: "example",
            expected_revision: session.revision,
            session_sha256: &session.commit.content_sha256,
            metadata: &metadata,
            approved_sha256: approval,
            publication_evidence: "Approved full target preview",
            recovery,
        };
        let context = work_feature::ports::RequirementWriterContext {
            canonical_project_root: adapter.project_root.clone(),
            requirement_id: "example".parse().unwrap(),
        };
        let held = LocalWriterLock
            .acquire_runtime(&context, work_model::runtime::LockClass::Execution)
            .unwrap();
        assert!(adapter.publish_with_runtime(request(false)).is_err());
        assert!(
            !adapter
                .project_root
                .join("outputs/work/tasks/example/index.json")
                .exists()
        );
        LocalWriterLock
            .require_runtime_idle(&context, work_model::runtime::LockClass::Discussion)
            .unwrap();
        held.release().unwrap();
        let published = adapter.publish_with_runtime(request(false)).unwrap();
        assert_eq!(published["published"], true);
        assert_eq!(
            adapter.publish_with_runtime(request(true)).unwrap()["changed"],
            false
        );
        for class in [
            work_model::runtime::LockClass::Discussion,
            work_model::runtime::LockClass::Execution,
        ] {
            LocalWriterLock
                .require_runtime_idle(&context, class)
                .unwrap();
        }
        assert!(
            !adapter
                .project_root
                .join("outputs/work/discussions/example/.task-publication.lock")
                .exists()
        );
        assert!(
            !adapter
                .project_root
                .join("outputs/work/executions/example/.work-state-writer.lock")
                .exists()
        );
    }

    #[test]
    fn runtime_publication_rejects_declared_custom_execution_legacy_lock() {
        let (adapter, mut session, _) = setup_runtime();
        let custom = "custom executions/example";
        match &mut session.context.confirmed_source {
            work_model::common::Nullable::Value(source) => {
                source.artifacts.execution = custom.into()
            }
            work_model::common::Nullable::Null => panic!("fixture requires confirmed Source"),
        }
        session.commit.content_sha256 =
            work_operations::derivation::fingerprint::discussion_session(&session);
        let legacy = adapter
            .project_root
            .join(format!("{custom}/.work-state-writer.lock"));
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, b"legacy owner evidence").unwrap();
        let error = adapter
            .with_runtime_publication(&session, true, |_, _| Ok(()))
            .unwrap_err();
        assert_eq!(error.reason_code, "legacy_writer_lock_present");
        assert_eq!(
            error.details["legacy_locks"],
            json!([format!("{custom}/.work-state-writer.lock")])
        );
        assert_eq!(fs::read(legacy).unwrap(), b"legacy owner evidence");
        assert!(
            !adapter
                .project_root
                .join("outputs/work/runtime/locks/example/discussion.lock")
                .exists()
        );
    }

    pub(super) fn publish(
        adapter: &LocalDiscussionAssembly,
        session: &DiscussionSession,
        metadata: &Value,
        approval: &str,
        recovery: bool,
    ) -> Result<Value, WorkError> {
        adapter.publish(PublicationRequest {
            requirement_id: "example",
            expected_revision: session.revision,
            session_sha256: &session.commit.content_sha256,
            metadata,
            approved_sha256: approval,
            publication_evidence: "User approved full target preview",
            recovery,
        })
    }

    #[test]
    fn complete_preview_publication_and_exact_recovery_preserve_pending_and_trace() {
        let (adapter, session, metadata) = setup();
        let preview = adapter.preview("example", &metadata).unwrap();
        assert_eq!(preview["targets"].as_object().unwrap().len(), 3);
        assert_eq!(preview["execution_index"]["overall_status"], "pending");
        assert_eq!(
            preview["contract"]["discussion"]["content_sha256"],
            session.commit.content_sha256
        );
        let approval = preview["approval_sha256"].as_str().unwrap();
        assert_eq!(
            publish(&adapter, &session, &metadata, approval, false).unwrap()["changed"],
            true
        );
        assert_eq!(
            publish(&adapter, &session, &metadata, approval, true).unwrap()["changed"],
            false
        );
        assert_eq!(
            publish(&adapter, &session, &metadata, approval, false)
                .unwrap_err()
                .reason_code,
            "task_create_target_exists"
        );
        for (path, raw) in preview["targets"].as_object().unwrap() {
            assert_eq!(
                fs::read(adapter.project_root.join(path)).unwrap(),
                raw.as_str().unwrap().as_bytes()
            );
        }
    }

    #[test]
    fn same_revision_different_bytes_source_drift_and_missing_authorization_block_publication() {
        let (adapter, session, metadata) = setup();
        let preview = adapter.preview("example", &metadata).unwrap();
        let approval = preview["approval_sha256"].as_str().unwrap();
        let mut changed = metadata.clone();
        changed["summary"] = json!("Changed reviewed content");
        assert_eq!(
            publish(&adapter, &session, &changed, approval, false)
                .unwrap_err()
                .reason_code,
            "discussion_approval_mismatch"
        );
        assert!(!adapter.project_root.join("outputs/work/tasks").exists());
        let denied = adapter.publish(PublicationRequest {
            requirement_id: "example",
            expected_revision: 1,
            session_sha256: &session.commit.content_sha256,
            metadata: &metadata,
            approved_sha256: approval,
            publication_evidence: "",
            recovery: false,
        });
        assert_eq!(
            denied.unwrap_err().reason_code,
            "discussion_publication_not_authorized"
        );
        fs::write(
            adapter
                .project_root
                .join("outputs/work/sources/example/SRC-001/source.txt"),
            b"drift",
        )
        .unwrap();
        assert!(publish(&adapter, &session, &metadata, approval, false).is_err());
        assert!(!adapter.project_root.join("outputs/work/tasks").exists());
    }

    #[test]
    fn stale_session_and_session_writer_lock_prevent_publication() {
        let (adapter, session, metadata) = setup();
        let preview = adapter.preview("example", &metadata).unwrap();
        let approval = preview["approval_sha256"].as_str().unwrap();
        use work_feature::ports::{RuntimeWriterGuard, RuntimeWriterLock};

        let context = work_feature::ports::RequirementWriterContext {
            canonical_project_root: adapter.project_root.canonicalize().unwrap(),
            requirement_id: "example".parse().unwrap(),
        };
        let runtime_guard = {
            LocalWriterLock
                .acquire_runtime(&context, work_model::runtime::LockClass::Discussion)
                .unwrap()
        };

        assert_eq!(
            publish(&adapter, &session, &metadata, approval, false)
                .unwrap_err()
                .reason_code,
            "work_state_writer_busy"
        );
        {
            let guard = runtime_guard;
            guard.release().unwrap();
        }

        let operation = work_operations::discussion::DiscussionOperation {
            operation_id: "advance".into(),
            expected_revision: 1,
            previous_sha256: session.commit.content_sha256.clone(),
            change: work_operations::discussion::DiscussionChange::UpdateContinuation {
                current_task_id: work_model::common::Nullable::Null,
            },
        };
        LocalDiscussionStorage {
            project_root: adapter.project_root.clone(),
            files: LocalFiles,
        }
        .submit("example", &operation, false)
        .unwrap();
        assert_eq!(
            publish(&adapter, &session, &metadata, approval, false)
                .unwrap_err()
                .reason_code,
            "stale_discussion_revision"
        );
        assert!(!adapter.project_root.join("outputs/work/tasks").exists());
    }
}

pub struct LocalDiscussionAssembly {
    pub project_root: PathBuf,
    pub skill_root: PathBuf,
    pub skill_configs: Vec<SkillRootConfig>,
}

impl assembly::DiscussionPublicationRepository for LocalDiscussionAssembly {
    fn preview(&self, requirement: &str, metadata: &Value) -> Result<Value, WorkError> {
        LocalDiscussionAssembly::preview(self, requirement, metadata)
    }
    fn publish(&self, request: PublicationRequest<'_>) -> Result<Value, WorkError> {
        LocalDiscussionAssembly::publish(self, request)
    }
}

impl LocalDiscussionAssembly {
    /// Publication scope: one Session owner, then an optional Execute owner.
    /// The publication closure receives the held owners and must not reacquire Session.
    pub fn with_runtime_publication<T>(
        &self,
        session: &work_model::discussion::DiscussionSession,
        exclude_execution: bool,
        publish: impl FnOnce(
            &work_model::runtime::RuntimeOwner,
            Option<&work_model::runtime::RuntimeOwner>,
        ) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        let storage = LocalDiscussionStorage {
            project_root: self.project_root.clone(),
            files: LocalFiles,
        };
        storage.with_runtime_session_writer(session, |discussion_owner| {
            if exclude_execution {
                work_feature::ports::with_runtime_writer(
                    &LocalWriterLock,
                    &work_feature::ports::RequirementWriterContext {
                        canonical_project_root: self
                            .project_root
                            .canonicalize()
                            .map_err(|_| assembly::error("discussion_project_root"))?,
                        requirement_id: session
                            .requirement_id
                            .parse()
                            .map_err(|_| assembly::error("discussion_snapshot_identity"))?,
                    },
                    work_model::runtime::LockClass::Execution,
                    |execution_owner| publish(discussion_owner, Some(execution_owner)),
                )
            } else {
                publish(discussion_owner, None)
            }
        })
    }

    fn prepare(
        &self,
        requirement: &str,
    ) -> Result<work_model::discussion::DiscussionSession, WorkError> {
        LocalDiscussionStorage {
            project_root: self.project_root.clone(),
            files: LocalFiles,
        }
        .read_current(requirement)?
        .ok_or_else(|| assembly::error("discussion_not_initialized"))
    }

    fn candidate(
        &self,
        session: &work_model::discussion::DiscussionSession,
        metadata: &Value,
    ) -> Result<PreparedDiscussion, WorkError> {
        let instructions = LocalHierarchyCatalog {
            skill_root: self.skill_root.clone(),
        };
        let skills = LocalSkillCatalog {
            roots: self.skill_configs.clone(),
        };
        let paths = LocalArtifactPaths {
            project_root: self.project_root.clone(),
        };
        let roots: Vec<_> = self
            .skill_configs
            .iter()
            .map(|r| SkillRoot {
                scope: r.scope.clone(),
                locator: r.locator.clone(),
            })
            .collect();
        assembly::prepare(&instructions, &skills, &paths, &roots, session, metadata)
    }

    pub fn preview(&self, requirement: &str, metadata: &Value) -> Result<Value, WorkError> {
        let session = self.prepare(requirement)?;
        let candidate = self.candidate(&session, metadata)?;
        if self.prepare(requirement)? != session {
            return Err(assembly::error("stale_discussion_revision"));
        }
        Ok(candidate.preview)
    }

    pub fn publish(&self, request: PublicationRequest<'_>) -> Result<Value, WorkError> {
        self.publish_with_runtime(request)
    }

    pub fn publish_with_runtime(
        &self,
        request: PublicationRequest<'_>,
    ) -> Result<Value, WorkError> {
        if request.publication_evidence.trim().is_empty() {
            return Err(assembly::error("discussion_publication_not_authorized"));
        }
        let session = self.prepare(request.requirement_id)?;
        // Initial creation also excludes standalone TASK create, using the same requirement writer.
        self.with_runtime_publication(&session, true, |_, _| self.publish_scoped(request))
    }

    fn publish_scoped(&self, request: PublicationRequest<'_>) -> Result<Value, WorkError> {
        if request.publication_evidence.trim().is_empty() {
            return Err(assembly::error("discussion_publication_not_authorized"));
        }
        let session = self.prepare(request.requirement_id)?;
        if Path::new(&session.authorization.project_root)
            != self
                .project_root
                .canonicalize()
                .map_err(|_| assembly::error("discussion_project_root"))?
        {
            return Err(assembly::error("discussion_save_scope_mismatch"));
        }
        let current = self.prepare(request.requirement_id)?;
        if current.revision != request.expected_revision
            || current.commit.content_sha256 != request.session_sha256
        {
            return Err(assembly::error("stale_discussion_revision"));
        }
        let _execution = match &current.context.confirmed_source {
            work_model::common::Nullable::Value(source) => {
                storage_path(&self.project_root, &source.artifacts.execution)?
            }
            work_model::common::Nullable::Null => {
                return Err(assembly::error("discussion_planning_incomplete"));
            }
        };
        // Resolve all confirmed Source/instruction/skill inputs again under the guards.
        let candidate = self.candidate(&current, request.metadata)?;
        if candidate.approval_sha256 != request.approved_sha256 {
            return Err(assembly::error("discussion_approval_mismatch"));
        }
        for path in candidate.preview["targets"]
            .as_object()
            .expect("derived targets")
            .keys()
        {
            storage_path(&self.project_root, path)?;
        }
        let changed = crate::task::create_storage::targets(
            &self.project_root,
            &candidate.task_path,
            &candidate.execution_dir,
            &candidate.prepared,
            request.recovery,
        )?;
        // A successful write is acknowledged only after every exact target is read back.
        for (path, expected) in candidate.preview["targets"]
            .as_object()
            .expect("derived targets")
        {
            if LocalFiles.read_raw(&storage_path(&self.project_root, path)?)?
                != expected.as_str().expect("UTF-8 target").as_bytes()
            {
                return Err(assembly::error("discussion_publication_readback_mismatch"));
            }
        }
        Ok(
            json!({"published":true,"changed":changed,"requirement_id":current.requirement_id,
            "revision":current.revision,"session_sha256":current.commit.content_sha256,
            "approval_sha256":candidate.approval_sha256,"task_collection_sha256":candidate.prepared.validation["task_collection_sha256"],
            "task_path":candidate.task_path,"execution_dir":candidate.execution_dir}),
        )
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::tests::{publish, setup};
    use super::*;
    use std::fs;
    use work_operations::derivation::fingerprint;

    #[test]
    fn partial_publication_recovers_only_identical_approved_targets() {
        let (adapter, session, metadata) = setup();
        let preview = adapter.preview("example", &metadata).unwrap();
        let approval = preview["approval_sha256"].as_str().unwrap();
        let index = "outputs/work/tasks/example/index.json";
        fs::create_dir_all(adapter.project_root.join("outputs/work/tasks/example")).unwrap();
        fs::write(
            adapter.project_root.join(index),
            preview["targets"][index].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            publish(&adapter, &session, &metadata, approval, true).unwrap()["changed"],
            true
        );
        let before = fs::read(adapter.project_root.join(index)).unwrap();
        let mut changed = metadata.clone();
        changed["summary"] = json!("Other approved set");
        let other = adapter.preview("example", &changed).unwrap();
        assert_eq!(
            publish(
                &adapter,
                &session,
                &changed,
                other["approval_sha256"].as_str().unwrap(),
                true
            )
            .unwrap_err()
            .reason_code,
            "unrecoverable_task_create_state"
        );
        assert_eq!(fs::read(adapter.project_root.join(index)).unwrap(), before);
    }

    #[test]
    fn confirmed_decisions_keep_trace_while_pending_or_missing_planning_blocks() {
        let (adapter, mut session, metadata) = setup();
        session.decisions.push(serde_json::from_value(json!({"id":"D008","question":"Review approach?","question_version":2,
            "options":[{"id":"manual","label":"Manual","explanation":"User verifies result"}],"status":"confirmed",
            "tentative_option_id":null,"resolution":{"option_id":"manual","rationale":"Review is sufficient","confirmation_evidence":"User selected 1"},
            "status_reason":"","status_evidence":"","added_reason":"Choose verification","task_ids":["TASK-001"],"dependencies":[],"source_references":[]})).unwrap());
        session.next_decision_number = 9;
        session.tasks[0].decision_ids = vec!["D008".into()];
        if let work_model::common::Nullable::Value(review) = &mut session.tasks[0].review {
            review.decision_versions.insert("D008".into(), 2);
        }
        session.commit.content_sha256 = fingerprint::discussion_session(&session);
        let prepared = adapter.candidate(&session, &metadata).unwrap();
        assert_eq!(
            prepared.preview["contract"]["discussion"]["decision_versions"]["D008"],
            2
        );
        assert_eq!(
            prepared.preview["contract"]["discussion"]["task_decisions"]["TASK-001"],
            json!(["D008"])
        );
        assert_eq!(
            prepared.preview["contract"]["tasks"][0]["decisions"][0]["rationale"],
            "Review is sufficient"
        );
        session.decisions[0].status = work_model::discussion::DecisionStatus::Pending;
        session.commit.content_sha256 = fingerprint::discussion_session(&session);
        assert_eq!(
            adapter
                .candidate(&session, &metadata)
                .err()
                .unwrap()
                .reason_code,
            "discussion_not_confirmed"
        );
        session.decisions.clear();
        session.tasks[0].decision_ids.clear();
        if let work_model::common::Nullable::Value(review) = &mut session.tasks[0].review {
            review.decision_versions.clear();
        }
        session.tasks[0].steps.clear();
        session.commit.content_sha256 = fingerprint::discussion_session(&session);
        assert_eq!(
            adapter
                .candidate(&session, &metadata)
                .err()
                .unwrap()
                .reason_code,
            "discussion_planning_incomplete"
        );
    }
}
