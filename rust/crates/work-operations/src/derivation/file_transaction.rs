//! Project transaction derivations share RuntimeManifest identity and inventory conventions.
use super::{fingerprint, identity};
use serde_json::json;
use std::collections::BTreeMap;
use work_model::execution::file_transaction::*;
use work_model::runtime::*;
use work_model::schema::PublicSchema;

pub fn candidate_bytes(binding: &FileTransactionBinding, targets: &[RuntimeTarget]) -> Vec<u8> {
    // Preserve canonical key order without cloning project bytes into a Value tree.
    #[derive(serde::Serialize)]
    struct Target<'a> {
        after: &'a Option<RuntimeBytes>,
        before: &'a Option<RuntimeBytes>,
        path: &'a str,
    }
    #[derive(serde::Serialize)]
    struct Candidate<'a> {
        binding: serde_json::Value,
        targets: Vec<Target<'a>>,
    }
    serde_json::to_vec(&Candidate {
        binding: json!(binding),
        targets: targets
            .iter()
            .map(|target| Target {
                after: &target.after,
                before: &target.before,
                path: &target.path,
            })
            .collect(),
    })
    .expect("candidate serializes")
}

pub fn approval(manifest: &RuntimeManifest) -> String {
    fingerprint::structured(&json!({"domain":"WORK-PROJECT-FILES-V1","root":manifest.canonical_root,
        "requirement_id":manifest.requirement_id,"execution_dir":manifest.execution_dir,
        "binding":manifest.business_identity,"targets":manifest.targets,"inventory":manifest.inventory}))
        .expect("approval serializes")
}

pub fn preview(
    root: &str,
    requirement: &str,
    execution: &str,
    binding: FileTransactionBinding,
    targets: Vec<RuntimeTarget>,
) -> Result<FileTransactionPreview, &'static str> {
    let bytes = candidate_bytes(&binding, &targets);
    let mut manifest = RuntimeManifest {
        schema: "work-runtime-transaction".into(),
        canonical_root: root.into(),
        requirement_id: requirement.into(),
        execution_dir: execution.into(),
        operation: "project-files".into(),
        transaction_identity: String::new(),
        approval_sha256: String::new(),
        business_identity: json!(binding),
        targets,
        inventory: vec![RuntimeFile {
            path: "candidate.json".into(),
            sha256: fingerprint::raw(&bytes),
            size_bytes: bytes.len() as u64,
        }],
        published_count: 0,
        phase: RuntimePhase::Prepared,
    };
    manifest.approval_sha256 = approval(&manifest);
    manifest.transaction_identity = identity::runtime_transaction_identity(
        root,
        &requirement
            .parse()
            .map_err(|_| "file_transaction_requirement")?,
        "project-files",
        &manifest.approval_sha256,
        &manifest.business_identity,
        &manifest
            .targets
            .iter()
            .map(|t| t.path.clone())
            .collect::<Vec<_>>(),
    )
    .map_err(|_| "file_transaction_identity")?;
    manifest.validate_shape()?;
    Ok(FileTransactionPreview {
        schema: PublicSchema::WorkFileTransactionPreview,
        manifest,
    })
}

pub fn recovery_approval(
    manifest: &RuntimeManifest,
    observed: &BTreeMap<String, Option<FileState>>,
) -> String {
    fingerprint::structured(
        &json!({"domain":"WORK-PROJECT-FILES-RESTORE-V1","manifest":manifest,"observed":observed}),
    )
    .expect("recovery serializes")
}

pub fn authorization_path(authorization: &FileAuthorization) -> String {
    let hash = fingerprint::structured(&json!(authorization)).expect("authorization serializes");
    format!("authorization-{hash}.json")
}

pub fn phase_rank(phase: RuntimePhase) -> u8 {
    match phase {
        RuntimePhase::Prepared => 0,
        RuntimePhase::Publishing => 1,
        RuntimePhase::PublishedVerified => 2,
        RuntimePhase::Cleaning => 3,
        RuntimePhase::Restoring => 4,
        RuntimePhase::Restored => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn borrowed_serialization_preserves_existing_candidate_bytes() {
        let metadata = FileMetadata {
            readonly: false,
            unix_mode: Some(420),
            unix_uid: Some(501),
            unix_gid: Some(20),
            windows_security: Some(vec![1, 2, 3]),
        };
        let binding = FileTransactionBinding {
            task_id: "TASK-001".into(),
            attempt_id: "ATTEMPT-001".into(),
            source_sha256: BTreeMap::new(),
            staged_files: BTreeMap::new(),
            before_metadata: BTreeMap::from([("a".into(), Some(metadata.clone()))]),
            after_metadata: BTreeMap::from([("a".into(), Some(metadata))]),
        };
        let targets = vec![RuntimeTarget {
            path: "a".into(),
            before: Some(RuntimeBytes {
                bytes: vec![0, 10, 255],
                sha256: "a".repeat(64),
            }),
            after: None,
        }];
        assert_eq!(
            candidate_bytes(&binding, &targets),
            serde_json::to_vec(&json!({"binding":binding,"targets":targets})).unwrap()
        );
    }
}
