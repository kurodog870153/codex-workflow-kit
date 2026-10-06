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
use work_feature::ports::{ArtifactStore, WriterLock};
use work_feature::skill::SkillRoot;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use work_model::discussion::DiscussionSession;

    pub(super) fn setup() -> (LocalDiscussionAssembly, DiscussionSession, Value) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/discussion-assembly");
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
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        let session = crate::fixture_support::prepare_discussion_fixture(
            &fixture,
            &root,
            &repo.join("../skills/work"),
        )
        .unwrap();
        LocalDiscussionStorage {
            project_root: root.clone(),
            files: LocalFiles,
        }
        .initialize(&session, false)
        .unwrap();
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
        let lock = adapter
            .project_root
            .join("outputs/work/discussions/example/.work-state-writer.lock");
        let guard = LocalWriterLock.acquire(&lock).unwrap();
        assert_eq!(
            publish(&adapter, &session, &metadata, approval, false)
                .unwrap_err()
                .reason_code,
            "work_state_writer_busy"
        );
        drop(guard);
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
        // Both apply and recovery always acquire Session before publication.
        let dir = format!("outputs/work/discussions/{}", session.requirement_id);
        let _session = LocalWriterLock.acquire(&storage_path(
            &self.project_root,
            &format!("{dir}/.work-state-writer.lock"),
        )?)?;
        let _publication = LocalWriterLock.acquire(&storage_path(
            &self.project_root,
            &format!("{dir}/.task-publication.lock"),
        )?)?;
        let current = self.prepare(request.requirement_id)?;
        if current.revision != request.expected_revision
            || current.commit.content_sha256 != request.session_sha256
        {
            return Err(assembly::error("stale_discussion_revision"));
        }
        let execution = match &current.context.confirmed_source {
            work_model::common::Nullable::Value(source) => {
                storage_path(&self.project_root, &source.artifacts.execution)?
            }
            work_model::common::Nullable::Null => {
                return Err(assembly::error("discussion_planning_incomplete"));
            }
        };
        // Recovery also excludes Execute writers once execution storage exists.
        let _execution = if request.recovery && execution.exists() {
            let lock = match &current.context.confirmed_source {
                work_model::common::Nullable::Value(source) => storage_path(
                    &self.project_root,
                    &format!("{}/.work-state-writer.lock", source.artifacts.execution),
                )?,
                work_model::common::Nullable::Null => unreachable!("checked source"),
            };
            Some(LocalWriterLock.acquire(&lock)?)
        } else {
            None
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
