//! Local timestamps and exclusive transaction workspace allocation.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use chrono::{Local, Utc};
use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::workspace::WorkspaceAllocator;

pub fn local_timestamp() -> String {
    Local::now().format("%Y-%m-%dT%H:%M%:z").to_string()
}

pub fn local_date() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub struct LocalWorkspaceAllocator<'a> {
    pub project_root: &'a Path,
}

pub fn create_transaction_workspace(
    project_root: &Path,
    requirement_id: Option<&str>,
    workflow_id: &str,
) -> Result<Value, WorkError> {
    work_feature::workspace::create(
        &LocalWorkspaceAllocator { project_root },
        requirement_id,
        workflow_id,
    )
}

impl WorkspaceAllocator for LocalWorkspaceAllocator<'_> {
    fn random_suffix(&self) -> Result<String, WorkError> {
        let mut random = [0u8; 4];
        getrandom::fill(&mut random).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "transaction_workspace_create_failed",
                "The transaction workspace could not be created.",
                json!({}),
            )
        })?;
        let mut suffix = String::with_capacity(8);
        for byte in random {
            write!(&mut suffix, "{byte:02x}").expect("writing to a String cannot fail");
        }
        Ok(suffix)
    }

    fn utc_stamp(&self) -> String {
        Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
    }

    fn allocate(&self, relative: &str) -> Result<String, WorkError> {
        let project_root = self.project_root;
        let (relative, absolute) = {
            if !relative.starts_with("outputs/work/transactions/") {
                return Err(WorkError::new(
                    ExitCode::Contract,
                    "transaction_workspace_scope",
                    "The workspace requires the canonical transport namespace.",
                    json!({"path":relative}),
                ));
            }
            (
                relative.to_owned(),
                crate::files::resolve_runtime_path(project_root, relative)?,
            )
        };
        if absolute.symlink_metadata().is_ok() {
            return Err(WorkError::new(
                ExitCode::WorkflowState,
                "transaction_workspace_exists",
                "A generated transaction workspace already exists.",
                json!({"path": relative}),
            ));
        }
        fs::create_dir_all(absolute.parent().expect("workspace has parent")).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "transaction_workspace_create_failed",
                "The transaction workspace could not be created.",
                json!({"path": relative}),
            )
        })?;
        fs::create_dir(&absolute).map_err(|error| {
            let exists = error.kind() == std::io::ErrorKind::AlreadyExists;
            WorkError::new(
                if exists {
                    ExitCode::WorkflowState
                } else {
                    ExitCode::IoFailure
                },
                if exists {
                    "transaction_workspace_exists"
                } else {
                    "transaction_workspace_create_failed"
                },
                if exists {
                    "A generated transaction workspace already exists."
                } else {
                    "The transaction workspace could not be created."
                },
                json!({"path": relative}),
            )
        })?;
        {
            for directory in ["inputs", "requests", "responses", "envelopes"] {
                let path = crate::files::resolve_runtime_path(
                    project_root,
                    &format!("{relative}/{directory}"),
                )?;
                fs::create_dir(&path).map_err(|_| {
                    WorkError::new(
                        ExitCode::IoFailure,
                        "transaction_workspace_create_failed",
                        "The transport workspace is incomplete; preserve its allocation evidence.",
                        json!({"path":relative,"directory":directory}),
                    )
                })?;
                crate::files::resolve_runtime_path(
                    project_root,
                    &format!("{relative}/{directory}"),
                )?;
            }
        }
        Ok(relative)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_workspace_allocates_all_directories_and_preserves_old_inputs_and_steps() {
        use work_feature::ports::ArtifactStore;
        use work_operations::derivation::publication::{self, WorkspaceEntry};
        let root = std::env::temp_dir().join(format!(
            "work-grouped-workspace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let old = root.join(".work/transactions/pending/invocation/retained/input.pdf");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        let binary = b"%PDF original\0\xff\n";
        fs::write(&old, binary).unwrap();
        let pending = create_transaction_workspace(&root, None, "invocation").unwrap();
        assert!(
            pending["path"]
                .as_str()
                .unwrap()
                .starts_with("outputs/work/transactions/pending/invocation/")
        );
        for directory in ["inputs", "requests", "responses", "envelopes"] {
            assert_eq!(
                pending["paths"][directory],
                format!("{}/{directory}", pending["path"].as_str().unwrap())
            );
            assert!(
                root.join(pending["paths"][directory].as_str().unwrap())
                    .is_dir()
            );
        }
        let workspace = pending["path"].as_str().unwrap();
        let input = publication::workspace_input_path(workspace, "原始 輸入.pdf").unwrap();
        crate::files::LocalFiles
            .create_new(&root.join(&input), binary)
            .unwrap();
        assert!(
            crate::files::LocalFiles
                .create_new(&root.join(&input), b"replacement")
                .is_err()
        );
        for kind in [
            WorkspaceEntry::Request,
            WorkspaceEntry::Response,
            WorkspaceEntry::Envelope,
        ] {
            let first = publication::workspace_entry_path(workspace, kind, 1, "capture").unwrap();
            let second = publication::workspace_entry_path(workspace, kind, 2, "capture").unwrap();
            crate::files::LocalFiles
                .create_new(&root.join(&first), b"first exact bytes")
                .unwrap();
            crate::files::LocalFiles
                .create_new(&root.join(&second), b"second exact bytes")
                .unwrap();
            assert!(
                crate::files::LocalFiles
                    .create_new(&root.join(&first), b"replacement")
                    .is_err()
            );
            assert_eq!(fs::read(root.join(first)).unwrap(), b"first exact bytes");
        }
        assert_eq!(
            LocalWorkspaceAllocator {
                project_root: &root
            }
            .allocate(workspace)
            .unwrap_err()
            .reason_code,
            "transaction_workspace_exists"
        );
        let confirmed = create_transaction_workspace(&root, Some("example"), "invocation").unwrap();
        assert!(
            confirmed["path"]
                .as_str()
                .unwrap()
                .starts_with("outputs/work/transactions/example/invocation/")
        );
        assert_ne!(pending["path"], confirmed["path"]);
        assert_eq!(fs::read(root.join(input)).unwrap(), binary);
        assert_eq!(fs::read(&old).unwrap(), binary);
        #[cfg(unix)]
        {
            let linked = root.join("linked project");
            fs::create_dir_all(&linked).unwrap();
            std::os::unix::fs::symlink(root.join("outputs"), linked.join("outputs")).unwrap();
            assert!(create_transaction_workspace(&linked, None, "invocation").is_err());
            assert_eq!(fs::read(&old).unwrap(), binary);
        }
    }

    #[test]
    fn generated_timestamp_formats_match_current_contract_contract() {
        let stamp = local_timestamp();
        assert_eq!(stamp.len(), 22);
        assert!(matches!(stamp.as_bytes()[16], b'+' | b'-'));
        assert_eq!(local_date().len(), 10);
    }

    #[test]
    fn transaction_workspace_uses_requirement_or_pending_owner_and_rejects_unsafe_segments() {
        let root = std::env::temp_dir().join(format!(
            "work-transaction-paths-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let confirmed =
            create_transaction_workspace(&root, Some("feature-1"), "specification").unwrap();
        let confirmed_id = confirmed["transaction_id"].as_str().unwrap();
        assert_eq!(
            confirmed["path"],
            format!("outputs/work/transactions/feature-1/specification/{confirmed_id}")
        );
        assert!(root.join(confirmed["path"].as_str().unwrap()).is_dir());
        let first_evidence = root
            .join(confirmed["path"].as_str().unwrap())
            .join("request.json");
        fs::write(&first_evidence, b"evidence").unwrap();
        let second =
            create_transaction_workspace(&root, Some("feature-1"), "specification").unwrap();
        assert_ne!(confirmed["transaction_id"], second["transaction_id"]);
        assert!(root.join(second["path"].as_str().unwrap()).is_dir());
        assert_eq!(fs::read(&first_evidence).unwrap(), b"evidence");
        assert_eq!(
            crate::transaction_storage::prepare_transaction_directory(
                &root.join(confirmed["path"].as_str().unwrap())
            )
            .unwrap_err()
            .reason_code,
            "transaction_workspace_exists"
        );
        let pending = create_transaction_workspace(&root, None, "invocation").unwrap();
        let pending_id = pending["transaction_id"].as_str().unwrap();
        assert_eq!(pending["path"], {
            format!("outputs/work/transactions/pending/invocation/{pending_id}")
        });
        assert!(root.join(pending["path"].as_str().unwrap()).is_dir());
        for (owner, workflow, reason) in [
            (
                Some("pending"),
                "specification",
                "reserved_transaction_owner",
            ),
            (Some("feature-1"), "../specification", "invalid_workflow_id"),
        ] {
            assert_eq!(
                create_transaction_workspace(&root, owner, workflow)
                    .unwrap_err()
                    .reason_code,
                reason
            );
        }
    }
}
