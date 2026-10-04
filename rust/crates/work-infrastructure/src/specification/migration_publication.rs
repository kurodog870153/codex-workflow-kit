//! Publication and recovery of a complete, independently validated migration.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::{ArtifactStore, WriterLock};
use work_feature::specification::migration_publication::{
    publication_paths, require_approved_preview,
};
use work_operations::canonical::parse_json_contract;
use work_operations::derivation::fingerprint;
use work_operations::specification::transaction::{render_transaction, validate_transaction};

use crate::files::LocalFiles;
use crate::skill_catalog::SkillRootConfig;
use crate::specification::migration::preview_migration;
use crate::specification::storage::{
    execution_history_bytes, publish_journal, require_no_spec_update, storage_path, write_journal,
};
use crate::writer_lock::LocalWriterLock;

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

use work_feature::specification::migration_transaction::{
    MigrationTransactionRepository, migration_transaction_with_additional as prepare_transaction,
};

struct LocalMigrationTransaction<'a> {
    root: &'a Path,
}
impl MigrationTransactionRepository for LocalMigrationTransaction<'_> {
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        LocalFiles.read_raw(&storage_path(self.root, relative)?)
    }
    fn exists(&self, relative: &str) -> Result<bool, WorkError> {
        Ok(storage_path(self.root, relative)?.is_file())
    }
    fn history_bytes(&self, execution_dir: &str) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
        execution_history_bytes(self.root, execution_dir)
    }
}

struct RecoveryBaseline<'a> {
    root: &'a Path,
    sources: BTreeMap<String, Vec<u8>>,
    additional: &'a BTreeMap<String, Vec<u8>>,
}
impl MigrationTransactionRepository for RecoveryBaseline<'_> {
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        self.sources.get(relative).cloned().ok_or_else(|| {
            fail(
                "migration_recovery_request_changed",
                "The approved original source snapshot is missing.",
            )
        })
    }
    fn exists(&self, relative: &str) -> Result<bool, WorkError> {
        Ok(self.sources.contains_key(relative))
    }
    fn history_bytes(&self, execution: &str) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
        let mut history = execution_history_bytes(self.root, execution)?;
        for (path, after) in self.additional {
            if let Some(current) = history.remove(path) {
                if &current != after && self.sources.get(path) != Some(&current) {
                    return Err(fail(
                        "migration_source_changed",
                        "An approved additional artifact changed during recovery.",
                    ));
                }
            }
            if path.starts_with(&format!("{execution}/")) {
                if let Some(before) = self.sources.get(path) {
                    history.insert(path.clone(), before.clone());
                }
            }
        }
        Ok(history)
    }
}

fn verify_recovery_candidates(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
    approved: &str,
    journal: &Value,
    additional: &BTreeMap<String, Vec<u8>>,
) -> Result<(), WorkError> {
    let mut sources = BTreeMap::new();
    for file in journal["files"]
        .as_array()
        .expect("validated journal files")
    {
        if !file["before"].is_null() {
            let raw = work_operations::derivation::snapshot::decode_snapshot(&file["before"])
                .map_err(|_| {
                    fail(
                        "migration_recovery_request_changed",
                        "The original source snapshot is invalid.",
                    )
                })?;
            sources.insert(file["path"].as_str().expect("validated path").into(), raw);
        }
    }
    let preview = crate::specification::migration::preview_migration_with_baseline(
        root,
        skill_root,
        configs,
        request,
        Some(&sources),
    )?;
    require_approved_preview(&preview, approved)?;
    let mut expected = prepare_transaction(
        &RecoveryBaseline {
            root,
            sources,
            additional,
        },
        request,
        &preview,
        additional,
    )?;
    expected["state"] = journal["state"].clone();
    expected["published_count"] = journal["published_count"].clone();
    if &expected != journal {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "migration_recovery_request_changed",
            "Recovery requires exact approved candidates, original evidence and unchanged history.",
            json!({"changed_fields":expected.as_object().unwrap().keys().filter(|key| expected[*key] != journal[*key]).collect::<Vec<_>>()}),
        ));
    }
    Ok(())
}

pub fn migration_transaction(
    root: &Path,
    request: &Value,
    preview: &Value,
) -> Result<Value, WorkError> {
    migration_transaction_with_additional(root, request, preview, &BTreeMap::new())
}
pub fn migration_transaction_with_additional(
    root: &Path,
    request: &Value,
    preview: &Value,
    additional: &BTreeMap<String, Vec<u8>>,
) -> Result<Value, WorkError> {
    prepare_transaction(
        &LocalMigrationTransaction { root },
        request,
        preview,
        additional,
    )
}

pub fn publish_migration(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
    operation: &str,
    approved_sha256: &str,
) -> Result<Value, WorkError> {
    publish_migration_with_additional(
        root,
        skill_root,
        configs,
        request,
        operation,
        approved_sha256,
        &BTreeMap::new(),
    )
}

pub fn publish_migration_with_additional(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
    operation: &str,
    approved_sha256: &str,
    additional: &BTreeMap<String, Vec<u8>>,
) -> Result<Value, WorkError> {
    publish_migration_with_guard(
        root,
        skill_root,
        configs,
        request,
        operation,
        approved_sha256,
        additional,
        || Ok(()),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn publish_migration_with_guard(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
    operation: &str,
    approved_sha256: &str,
    additional: &BTreeMap<String, Vec<u8>>,
    guard: impl FnOnce() -> Result<(), WorkError>,
) -> Result<Value, WorkError> {
    let paths = publication_paths(request, approved_sha256)?;
    let execution = paths.execution.as_str();
    let journal_path = paths.journal;
    let marker_path = paths.marker;
    let (preview, transaction) = match operation {
        "apply" => {
            let preview = preview_migration(root, skill_root, configs, request)?;
            require_approved_preview(&preview, approved_sha256)?;
            let transaction =
                migration_transaction_with_additional(root, request, &preview, additional)?;
            (preview, transaction)
        }
        "recover" => {
            let raw = LocalFiles.read_raw(&storage_path(root, &journal_path)?)?;
            let journal = parse_json_contract(&raw)
                .map_err(|_| fail("invalid_json_contract", "The migration journal is invalid."))?;
            validate_transaction(&journal).map_err(|issue| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    issue.reason_code,
                    issue.message,
                    issue.details,
                )
            })?;
            if render_transaction(&journal).ok().as_deref() != Some(raw.as_slice())
                || journal["metadata"]["request"]
                    != json!({"migration":request,
                    "preview_fingerprint":approved_sha256})
            {
                return Err(fail(
                    "migration_recovery_request_changed",
                    "Recovery requires the identical approved migration request.",
                ));
            }
            verify_recovery_candidates(
                root,
                skill_root,
                configs,
                request,
                approved_sha256,
                &journal,
                additional,
            )?;
            (json!({"fingerprint":approved_sha256}), journal)
        }
        _ => {
            return Err(fail(
                "migration_operation",
                "Use apply or recover for migration publication.",
            ));
        }
    };
    let directory = storage_path(root, execution)?;
    fs::create_dir_all(&directory).map_err(|_| {
        fail(
            "migration_execution_directory",
            "The migration execution directory cannot be created.",
        )
    })?;
    let ignored = (operation == "recover").then_some(journal_path.as_str());
    require_no_spec_update(root, execution, ignored)?;
    let writer = storage_path(root, &format!("{execution}/.work-state-writer.lock"))?;
    let _guard = LocalWriterLock.acquire(&writer)?;
    require_no_spec_update(root, execution, ignored)?;
    if operation == "recover" {
        verify_recovery_candidates(
            root,
            skill_root,
            configs,
            request,
            approved_sha256,
            &transaction,
            additional,
        )?;
    } else {
        let fresh_preview = preview_migration(root, skill_root, configs, request)?;
        require_approved_preview(&fresh_preview, approved_sha256)?;
        let fresh_transaction =
            migration_transaction_with_additional(root, request, &fresh_preview, additional)?;
        if fresh_transaction != transaction {
            return Err(fail(
                "migration_source_changed",
                "Migration sources or history changed before publication.",
            ));
        }
    }
    guard()?;
    if operation == "apply" {
        write_journal(root, &journal_path, &transaction)?;
    }
    let publication = publish_journal(root, &journal_path, &marker_path).map_err(|_| {
        WorkError::new(
            ExitCode::IoFailure,
            "migration_interrupted",
            "Preserve the migration transaction and obtain recovery authorization.",
            json!({"recovery_required":true,"record":journal_path}),
        )
    })?;
    let source_sha = transaction["metadata"]["source_sha256"]
        .as_object()
        .ok_or_else(|| {
            fail(
                "migration_post_write",
                "Migration source evidence is missing.",
            )
        })?;
    let candidate_sha = transaction["metadata"]["candidate_sha256"]
        .as_object()
        .ok_or_else(|| {
            fail(
                "migration_post_write",
                "Migration candidate evidence is missing.",
            )
        })?;
    for path in source_sha
        .keys()
        .chain(candidate_sha.keys())
        .collect::<BTreeSet<_>>()
    {
        let target = storage_path(root, path)?;
        if let Some(expected) = candidate_sha.get(path) {
            if json!(fingerprint::raw(&LocalFiles.read_raw(&target)?)) != *expected {
                return Err(fail(
                    "migration_post_write",
                    "An installed migration artifact differs from approval.",
                ));
            }
        } else if target.exists() {
            return Err(fail(
                "migration_post_write",
                "A removed source artifact still exists.",
            ));
        }
    }
    let mut post_request = request.clone();
    post_request["sources"] = Value::Array(request["candidates"].as_array().into_iter().flatten()
        .map(|row| json!({"path":row["path"],"raw_sha256":candidate_sha[row["path"].as_str().unwrap()]}))
        .collect());
    let validated = preview_migration(root, skill_root, configs, &post_request)?;
    if validated["writable_ready"] != true {
        return Err(fail(
            "migration_post_validation",
            "Installed migration artifacts failed current validation.",
        ));
    }
    Ok(work_model::specification::verified::<
        work_model::specification::SpecMigrationPublication,
    >(json!({"schema":"work-spec-migration-publication",
        "status":if operation == "recover" {"recovered"} else {"updated"},
        "fingerprint":preview["fingerprint"],
        "transaction_approval_sha256":transaction["approval_sha256"],
        "journal":journal_path,"completion_marker":marker_path,
        "documents":validated["documents"],"publication_status":publication["status"],
        "validator_results":validated["validator_results"],
        "relationship_results":validated["relationship_results"]})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn reconstruction_publication_matches_current_contract_journal_and_response() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-migration/reconstruction");
        let root = std::env::temp_dir().join(format!(
            "work-reconstruction-publication-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let request: Value =
            serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
        let preview: Value =
            serde_json::from_slice(&fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let transaction = migration_transaction(&root, &request, &preview).unwrap();
        let expected_journal: Value =
            serde_json::from_slice(&fs::read(fixture.join("journal.json")).unwrap()).unwrap();
        assert_eq!(
            transaction["approval_sha256"],
            expected_journal["approval_sha256"]
        );
        let result = publish_migration(
            &root,
            &repo.join("../skills/work"),
            &[],
            &request,
            "apply",
            preview["fingerprint"].as_str().unwrap(),
        )
        .unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("publication.json")).unwrap()).unwrap();
        assert_eq!(result, expected);
        let installed = fs::read(root.join(result["journal"].as_str().unwrap())).unwrap();
        let reference = fs::read(fixture.join("journal.json")).unwrap();
        let difference = String::from_utf8(installed.clone())
            .unwrap()
            .lines()
            .zip(String::from_utf8(reference.clone()).unwrap().lines())
            .enumerate()
            .find(|(_, (left, right))| left != right)
            .map(|(position, (left, right))| (position, left.to_owned(), right.to_owned()));
        assert!(
            installed == reference,
            "journal first difference: {difference:?}"
        );
        let recovered = publish_migration(
            &root,
            &repo.join("../skills/work"),
            &[],
            &request,
            "recover",
            preview["fingerprint"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(recovered["status"], "recovered");
        assert_eq!(recovered["publication_status"], "already_published");
    }

    #[test]
    fn migration_journal_resumes_each_published_count() {
        use work_operations::derivation::snapshot::decode_snapshot;

        let fixture = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/work-infrastructure/fixtures/specification-migration/reconstruction"
        ));
        let final_raw = fs::read(fixture.join("journal.json")).unwrap();
        let final_journal: Value = serde_json::from_slice(&final_raw).unwrap();
        let publication: Value =
            serde_json::from_slice(&fs::read(fixture.join("publication.json")).unwrap()).unwrap();
        let journal_path = publication["journal"].as_str().unwrap();
        let marker_path = publication["completion_marker"].as_str().unwrap();
        let files = final_journal["files"].as_array().unwrap();
        for published_count in 0..=files.len() {
            let root = std::env::temp_dir().join(format!(
                "work-migration-phase-{published_count}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&root).unwrap();
            for (index, row) in files.iter().enumerate() {
                let target = root.join(row["path"].as_str().unwrap());
                let side = if index < published_count {
                    "after"
                } else {
                    "before"
                };
                if let Some(snapshot) = row.get(side) {
                    let raw = decode_snapshot(snapshot).unwrap();
                    fs::create_dir_all(target.parent().unwrap()).unwrap();
                    fs::write(target, raw).unwrap();
                }
            }
            let mut interrupted = final_journal.clone();
            interrupted["published_count"] = json!(published_count);
            interrupted["state"] = json!(if published_count == 0 {
                "prepared"
            } else if published_count == files.len() {
                "published"
            } else {
                "publishing"
            });
            fs::create_dir_all(root.join(journal_path).parent().unwrap()).unwrap();
            write_journal(&root, journal_path, &interrupted).unwrap();
            let resumed = publish_journal(&root, journal_path, marker_path).unwrap();
            assert_eq!(resumed["published_count"], files.len());
            assert_eq!(fs::read(root.join(journal_path)).unwrap(), final_raw);
            assert!(root.join(marker_path).is_file());
            for row in files {
                let target = root.join(row["path"].as_str().unwrap());
                match row.get("after") {
                    Some(snapshot) => {
                        assert_eq!(
                            fs::read(target).unwrap(),
                            decode_snapshot(snapshot).unwrap()
                        );
                    }
                    None => assert!(!target.exists()),
                }
            }
        }
    }
    #[test]
    fn recovery_rejects_self_consistent_journal_with_unapproved_candidate_bytes() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-migration/reconstruction");
        let request: Value =
            serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
        let root = std::env::temp_dir().join(format!(
            "work-migration-forged-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut original = BTreeMap::new();
        for source in request["sources"].as_array().unwrap() {
            let path = source["path"].as_str().unwrap();
            let raw = fs::read(fixture.join(path)).unwrap();
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), &raw).unwrap();
            original.insert(path.to_owned(), raw);
        }
        let skill = repo.join("../skills/work");
        let preview = preview_migration(&root, &skill, &[], &request).unwrap();
        let approved = preview["fingerprint"].as_str().unwrap();
        let transaction = migration_transaction(&root, &request, &preview).unwrap();
        let paths = publication_paths(&request, approved).unwrap();
        let mut forged = transaction.clone();
        let target = "outputs/work/tasks/example/index.json";
        let row = forged["files"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["path"] == target)
            .unwrap();
        row["after"] = work_operations::derivation::snapshot::encode_snapshot(b"unapproved bytes");
        forged["metadata"]["candidate_sha256"][target] =
            json!(fingerprint::raw(b"unapproved bytes"));
        let hash = work_operations::derivation::transaction::approval_sha256(
            &forged["files"],
            &forged["metadata"],
        );
        forged["approval_sha256"] = json!(hash);
        forged["transaction_id"] = json!(
            work_operations::derivation::identity::derived_transaction_id("MIGRATION", &hash)
                .unwrap()
        );
        validate_transaction(&forged).unwrap();
        fs::create_dir_all(root.join(&paths.execution)).unwrap();
        write_journal(&root, &paths.journal, &forged).unwrap();
        let journal_before = fs::read(root.join(&paths.journal)).unwrap();
        assert_eq!(
            publish_migration(&root, &skill, &[], &request, "recover", approved)
                .unwrap_err()
                .reason_code,
            "migration_recovery_request_changed"
        );
        assert_eq!(fs::read(root.join(&paths.journal)).unwrap(), journal_before);
        for (path, raw) in &original {
            assert_eq!(fs::read(root.join(path)).unwrap(), *raw);
        }
        assert!(!root.join(&paths.marker).exists());
        fs::write(
            root.join(&paths.journal),
            render_transaction(&transaction).unwrap(),
        )
        .unwrap();
        assert_eq!(
            publish_migration(&root, &skill, &[], &request, "recover", approved).unwrap()["publication_status"],
            "published"
        );
    }
}
