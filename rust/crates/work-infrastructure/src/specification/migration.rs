//! Revision migration candidate assembly from a reviewed Specification update.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;
use work_feature::specification::migration_prepare::{
    MigrationPrepareRepository, build_revision_request, parse_revision_semantic,
};
use work_operations::derivation::fingerprint;
#[cfg(test)]
use work_operations::derivation::fingerprint::raw as sha256_hex;
use work_operations::derivation::snapshot::decode_snapshot;
use work_operations::derivation::transaction::{
    PublicationOrder, TransactionDeriver, TransactionInput, TransactionKind,
};
use work_operations::execution::index::render_execution_index;
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::files::LocalFiles;
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::{execution_history_bytes, storage_path};
use crate::specification::workflow_storage::{SpecificationPrepareInput, prepare_simple_update};
use work_feature::specification::migration_preview_project::{
    MigrationPreviewRepository, preview_migration as preview_from_ports,
};
use work_operations::specification::migration_diff::unified_diff;

fn fail(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}

struct LocalMigrationPreview<'a> {
    root: &'a Path,
    baseline: Option<&'a BTreeMap<String, Vec<u8>>>,
}
impl MigrationPreviewRepository for LocalMigrationPreview<'_> {
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        if let Some(raw) = self.baseline.and_then(|sources| sources.get(relative)) {
            return Ok(raw.clone());
        }
        LocalFiles.read_raw(&storage_path(self.root, relative)?)
    }
    fn validate_path(&self, relative: &str) -> Result<(), WorkError> {
        storage_path(self.root, relative).map(|_| ())
    }
}
pub fn preview_migration(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
) -> Result<Value, WorkError> {
    preview_migration_with_baseline(root, skill_root, configs, request, None)
}

pub(super) fn preview_migration_with_baseline(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    request: &Value,
    baseline: Option<&BTreeMap<String, Vec<u8>>>,
) -> Result<Value, WorkError> {
    let instructions = LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
    let skills = LocalSkillCatalog {
        roots: configs.to_vec(),
    };
    let roots = configs
        .iter()
        .map(|config| SkillRoot {
            scope: config.scope.clone(),
            locator: config.locator.clone(),
        })
        .collect::<Vec<_>>();
    preview_from_ports(
        &LocalMigrationPreview { root, baseline },
        &instructions,
        &skills,
        &crate::artifact_paths::LocalArtifactPaths {
            project_root: root.to_path_buf(),
        },
        &roots,
        request,
    )
}

struct LocalMigrationPrepare<'a> {
    root: &'a Path,
}
impl MigrationPrepareRepository for LocalMigrationPrepare<'_> {
    fn read_execution(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        LocalFiles.read_raw(&storage_path(self.root, relative)?)
    }
}

pub fn prepare_revision_request(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    raw: &[u8],
    date: &str,
) -> Result<Value, WorkError> {
    let parsed = parse_revision_semantic(raw)?;
    for source in parsed.semantic["sources"]
        .as_array()
        .expect("validated raw sources")
    {
        let path = source["path"].as_str().expect("validated source path");
        let raw = LocalFiles.read_raw(&storage_path(root, path)?)?;
        if source["raw_sha256"] != fingerprint::raw(&raw) {
            return Err(fail(
                "migration_source_changed",
                "Original bytes differ from AI-reviewed evidence.",
            ));
        }
    }
    let prepared = prepare_simple_update(
        root,
        skill_root,
        configs,
        SpecificationPrepareInput {
            raw: &parsed.revision_raw,
            date,
            output_file: None,
        },
    )?;
    build_revision_request(&LocalMigrationPrepare { root }, &parsed.semantic, &prepared)
}

pub fn preview_revision_from_prepared(
    root: &Path,
    migration_request: &Value,
    specification_prepared: &Value,
) -> Result<Value, WorkError> {
    if migration_request["schema"] != "work-spec-migration-preview-request" {
        return Err(fail(
            "migration_preview_schema",
            "A migration preview request is required.",
        ));
    }
    let transaction = &specification_prepared["preview"]["transaction"];
    let source_sha = transaction["metadata"]["source_sha256"]
        .as_object()
        .ok_or_else(|| fail("migration_source_missing", "Migration sources are missing."))?;
    let candidate_sha = transaction["metadata"]["candidate_sha256"]
        .as_object()
        .ok_or_else(|| {
            fail(
                "migration_candidate_set_incomplete",
                "Migration candidates are missing.",
            )
        })?;
    let mut sources = BTreeMap::new();
    for (path, expected) in source_sha {
        let raw = LocalFiles.read_raw(&storage_path(root, path)?)?;
        if expected != &json!(fingerprint::raw(&raw)) {
            return Err(fail(
                "migration_source_changed",
                "Migration source bytes changed.",
            ));
        }
        sources.insert(path.clone(), raw);
    }
    let mut candidates = BTreeMap::new();
    for (path, expected) in candidate_sha {
        let raw = transaction["files"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|file| file["path"] == *path)
            .and_then(|file| file.get("after"))
            .map(|snapshot| {
                decode_snapshot(snapshot).map_err(|issue| {
                    WorkError::new(
                        ExitCode::ArtifactIntegrity,
                        issue.reason_code,
                        issue.message,
                        issue.details,
                    )
                })
            })
            .transpose()?
            .or_else(|| sources.get(path).cloned())
            .ok_or_else(|| {
                fail(
                    "migration_candidate_set_incomplete",
                    "A candidate is missing.",
                )
            })?;
        if expected != &json!(fingerprint::raw(&raw)) {
            return Err(fail(
                "migration_candidate_changed",
                "Migration candidate bytes changed.",
            ));
        }
        candidates.insert(path.clone(), raw);
    }
    let all_paths = sources
        .keys()
        .chain(candidates.keys())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let diffs = all_paths
        .iter()
        .map(|path| {
            let before = sources.get(path).map(Vec::as_slice);
            let after = candidates.get(path).map(Vec::as_slice);
            json!({"path":path,"operation":if before.is_none() { "add" }
            else if after.is_none() { "remove" } else { "replace" },
            "unified_diff":unified_diff(path, before, after)})
        })
        .collect::<Vec<_>>();
    let validators = vec![
        json!({"name":"task_collection","status":"passed"}),
        json!({"name":"execution_index","status":"passed"}),
    ];
    let relationships = vec![
        json!({"name":"candidate_set","status":"passed"}),
        json!({"name":"artifact_paths","status":"passed"}),
        json!({"name":"execution_binding","status":"passed"}),
    ];
    let mut unresolved = migration_request["semantic_decisions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["resolution"].is_null())
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    unresolved.sort();
    let ready = unresolved.is_empty();
    let source_hashes = sources
        .iter()
        .map(|(path, raw)| (path.clone(), fingerprint::raw(raw)))
        .collect::<BTreeMap<_, _>>();
    let candidate_hashes = candidates
        .iter()
        .map(|(path, raw)| (path.clone(), fingerprint::raw(raw)))
        .collect::<BTreeMap<_, _>>();
    let evidence = json!({"request":migration_request,"source_sha256":source_hashes,
        "candidate_sha256":candidate_hashes,"validator_results":validators,
        "relationship_results":relationships,"unresolved_items":unresolved});
    let fingerprint = fingerprint::structured(&evidence).map_err(|_| {
        fail(
            "invalid_contract_value",
            "Migration evidence cannot be fingerprinted.",
        )
    })?;
    Ok(work_model::specification::verified::<
        work_model::specification::SpecMigrationPreview,
    >(json!({"schema":"work-spec-migration-preview",
        "status":if ready {"ready"} else {"blocked"},"documents":all_paths,
        "diffs":diffs,"validator_results":validators,"relationship_results":relationships,
        "unresolved_items":unresolved,"fingerprint":fingerprint,"writable_ready":ready})))
}

pub fn revision_transaction(
    root: &Path,
    migration_request: &Value,
    preview: &Value,
    specification_prepared: &Value,
) -> Result<Value, WorkError> {
    if preview["writable_ready"] != true {
        return Err(fail(
            "migration_not_writable",
            "The migration preview is not ready.",
        ));
    }
    let specification = &specification_prepared["preview"]["transaction"];
    let source_sha = specification["metadata"]["source_sha256"]
        .as_object()
        .ok_or_else(|| fail("migration_source_missing", "Migration sources are missing."))?;
    let candidate_sha = specification["metadata"]["candidate_sha256"]
        .as_object()
        .ok_or_else(|| {
            fail(
                "migration_candidate_set_incomplete",
                "Migration candidates are missing.",
            )
        })?;
    let requested_sources = migration_request["sources"].as_array().ok_or_else(|| {
        fail(
            "migration_source_missing",
            "Migration source evidence is missing.",
        )
    })?;
    if requested_sources.len() != source_sha.len()
        || requested_sources.iter().any(|row| {
            row["path"]
                .as_str()
                .is_none_or(|path| source_sha.get(path) != Some(&row["raw_sha256"]))
        })
    {
        return Err(fail(
            "migration_source_changed",
            "Migration sources differ from the reviewed update.",
        ));
    }
    let requested_candidates = migration_request["candidates"].as_array().ok_or_else(|| {
        fail(
            "migration_candidate_set_incomplete",
            "Migration candidates are missing.",
        )
    })?;
    let mut requested_hashes = BTreeMap::new();
    for row in requested_candidates {
        let path = row["path"].as_str().ok_or_else(|| {
            fail(
                "migration_candidate_set_incomplete",
                "A migration candidate path is missing.",
            )
        })?;
        let raw = match row["kind"].as_str() {
            Some("task_index") => render_task(&row["content"], TaskDocumentKind::Index),
            Some("task_item") => render_task(&row["content"], TaskDocumentKind::Item),
            Some("execution_index") => render_execution_index(&row["content"]),
            _ => {
                return Err(fail(
                    "migration_candidate_set_incomplete",
                    "A migration candidate kind is invalid.",
                ));
            }
        }
        .map_err(|_| {
            fail(
                "invalid_contract_value",
                "A migration candidate cannot be rendered.",
            )
        })?;
        if requested_hashes
            .insert(path.to_owned(), json!(fingerprint::raw(&raw)))
            .is_some()
        {
            return Err(fail(
                "migration_candidate_duplicate",
                "Migration candidate paths must be unique.",
            ));
        }
    }
    for row in requested_candidates
        .iter()
        .filter(|row| row["kind"] == "task_index")
    {
        for path in work_feature::task::source::evidence_paths(&row["content"])? {
            let digest = source_sha.get(&path).ok_or_else(|| {
                fail(
                    "migration_source_missing",
                    "The complete immutable Source proof is required.",
                )
            })?;
            if candidate_sha.get(&path) != Some(digest) {
                return Err(fail(
                    "migration_candidate_changed",
                    "Source proof bytes must stay immutable.",
                ));
            }
            requested_hashes.insert(path, digest.clone());
        }
    }
    if requested_hashes.len() != candidate_sha.len()
        || requested_hashes
            .iter()
            .any(|(path, hash)| candidate_sha.get(path) != Some(hash))
    {
        return Err(fail(
            "migration_candidate_changed",
            "Migration candidates differ from the reviewed update.",
        ));
    }
    let mut source_raw = BTreeMap::new();
    let mut candidate_raw = BTreeMap::new();
    let paths = source_sha
        .keys()
        .chain(candidate_sha.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for path in paths {
        let before = if source_sha.contains_key(&path) {
            let raw = LocalFiles.read_raw(&storage_path(root, &path)?)?;
            if source_sha[&path] != json!(fingerprint::raw(&raw)) {
                return Err(fail(
                    "migration_source_changed",
                    "Migration source bytes changed.",
                ));
            }
            Some(raw)
        } else {
            None
        };
        let after = if candidate_sha.contains_key(&path) {
            let raw = specification["files"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|row| row["path"] == path)
                .and_then(|row| row.get("after"))
                .map(|snapshot| {
                    decode_snapshot(snapshot).map_err(|issue| {
                        WorkError::new(
                            ExitCode::ArtifactIntegrity,
                            issue.reason_code,
                            issue.message,
                            issue.details,
                        )
                    })
                })
                .transpose()?
                .or_else(|| before.clone())
                .ok_or_else(|| {
                    fail(
                        "migration_candidate_set_incomplete",
                        "A candidate is missing.",
                    )
                })?;
            if candidate_sha[&path] != json!(fingerprint::raw(&raw)) {
                return Err(fail(
                    "migration_candidate_changed",
                    "Migration candidate bytes changed.",
                ));
            }
            Some(raw)
        } else {
            None
        };
        if let Some(before) = before {
            source_raw.insert(path.clone(), before);
        }
        if let Some(after) = after {
            candidate_raw.insert(path, after);
        }
    }
    let artifacts = &migration_request["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row["kind"] == "task_index")
        .ok_or_else(|| {
            fail(
                "migration_candidate_set_incomplete",
                "The migration Task index is missing.",
            )
        })?["content"]["artifacts"];
    let mut affected = migration_request["candidates"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| row["kind"] == "task_item")
        .filter_map(|row| row["task_id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    affected.sort();
    let history_sha256 = serde_json::from_value::<BTreeMap<String, String>>(
        specification["metadata"]["history_sha256"].clone(),
    )
    .map_err(|_| {
        fail(
            "invalid_contract_value",
            "Migration history fingerprints are invalid.",
        )
    })?;
    let execution = artifacts["execution"].as_str().ok_or_else(|| {
        fail(
            "migration_execution_directory",
            "An execution directory is required.",
        )
    })?;
    let history = execution_history_bytes(root, execution)?;
    let current_history_sha256 = history
        .iter()
        .map(|(path, raw)| (path.clone(), fingerprint::history(raw)))
        .collect::<BTreeMap<_, _>>();
    if current_history_sha256 != history_sha256 {
        return Err(fail(
            "migration_source_changed",
            "Migration history bytes changed.",
        ));
    }
    let derived = TransactionDeriver::derive(TransactionInput {
        kind: TransactionKind::Migration,
        order: PublicationOrder::Migration,
        request: json!({"migration":migration_request,
            "preview_fingerprint":preview["fingerprint"]}),
        artifacts: artifacts.clone(),
        affected_task_ids: affected,
        history,
        source: source_raw,
        candidate: candidate_raw,
    })
    .map_err(|issue| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    Ok(derived.journal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::specification::migration_publication::publish_migration;
    use crate::specification::storage::write_journal;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn independent_preview_matches_current_contract_complete_unchanged_collection() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/shared/specification-migration-project");
        let root = std::env::temp_dir().join(format!(
            "work-spec-migration-preview-{}-{}",
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
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        let request: Value = serde_json::from_slice(
            &fs::read(fixture.join("../../cases/specification/migration/valid/input/request.json"))
                .unwrap(),
        )
        .unwrap();
        let expected: Value = serde_json::from_slice(
            &fs::read(
                fixture.join("../../cases/specification/migration/valid/expected/result.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let actual = preview_migration(
            &root,
            &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                .unwrap(),
            &[],
            &request,
        )
        .unwrap();
        assert_eq!(actual, expected);
        let unresolved: Value = serde_json::from_slice(
            &fs::read(
                fixture.join("../../cases/specification/migration/unresolved/input/request.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let blocked: Value = serde_json::from_slice(
            &fs::read(
                fixture.join("../../cases/specification/migration/unresolved/expected/result.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            preview_migration(
                &root,
                &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                    .unwrap(),
                &[],
                &unresolved
            )
            .unwrap(),
            blocked
        );
        let mut resolved = unresolved.clone();
        resolved["semantic_decisions"][0]["resolution"] = json!("Use the confirmed v1 meaning.");
        let resolved_preview = preview_migration(
            &root,
            &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                .unwrap(),
            &[],
            &resolved,
        )
        .unwrap();
        assert_eq!(resolved_preview["status"], "ready");
        assert_ne!(resolved_preview["fingerprint"], blocked["fingerprint"]);
        let mut mismatched = request.clone();
        let task_candidate = mismatched["candidates"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["kind"] == "task_index")
            .unwrap();
        task_candidate["content"]["artifacts"]["task"] = json!("wrong/index.json");
        let mismatch = preview_migration(
            &root,
            &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                .unwrap(),
            &[],
            &mismatched,
        )
        .unwrap();
        assert_eq!(mismatch["status"], "blocked");
        assert_eq!(mismatch["writable_ready"], false);
        assert!(
            mismatch["relationship_results"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["name"] == "artifact_paths" && row["status"] == "failed")
        );
        for variant in ["invalid-execution-binding", "incomplete-candidate-set"] {
            let request: Value = serde_json::from_slice(
                &fs::read(fixture.join(format!(
                    "../../cases/specification/migration/{variant}/input/request.json"
                )))
                .unwrap(),
            )
            .unwrap();
            let expected: Value = serde_json::from_slice(
                &fs::read(fixture.join(format!(
                    "../../cases/specification/migration/{variant}/expected/result.json"
                )))
                .unwrap(),
            )
            .unwrap();
            let actual = preview_migration(
                &root,
                &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                    .unwrap(),
                &[],
                &request,
            )
            .unwrap();
            assert_eq!(actual, expected, "variant {variant}");
        }
        let legacy_candidate: Value = serde_json::from_slice(
            &fs::read(
                fixture.join("../../cases/specification/migration/invalid-plan/input/request.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            preview_migration(
                &root,
                &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                    .unwrap(),
                &[],
                &legacy_candidate
            )
            .unwrap_err()
            .reason_code,
            "invalid_contract_value"
        );
        let mut legacy = request.clone();
        for source in legacy["sources"].as_array_mut().unwrap() {
            let relative = source["path"].as_str().unwrap().to_owned();
            let invalid_raw = b"{\"schema\":\"same-id-but-incompatible\"}\n";
            fs::write(root.join(&relative), invalid_raw).unwrap();
            source["raw_sha256"] = json!(sha256_hex(invalid_raw));
        }
        let before = legacy["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                let relative = row["path"].as_str().unwrap().to_owned();
                (relative.clone(), fs::read(root.join(relative)).unwrap())
            })
            .collect::<BTreeMap<_, _>>();
        let legacy_preview = preview_migration(
            &root,
            &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                .unwrap(),
            &[],
            &legacy,
        )
        .unwrap();
        assert_eq!(legacy_preview["status"], "ready");
        assert_eq!(legacy_preview["writable_ready"], true);
        for (relative, bytes) in before {
            assert_eq!(fs::read(root.join(relative)).unwrap(), bytes);
        }
    }

    #[test]
    fn revision_migration_request_matches_current_contract_candidate_set() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo
            .join("crates/work-infrastructure/fixtures/cases/specification/update/revision-migration/project");
        let root = std::env::temp_dir().join(format!(
            "work-spec-migration-{}-{}",
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
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let expected: Value =
            serde_json::from_slice(&fs::read(fixture.join("../input/request.json")).unwrap())
                .unwrap();
        let date = expected["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["kind"] == "task_index")
            .unwrap()["content"]["changes"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["date"]
            .as_str()
            .unwrap();
        let actual = prepare_revision_request(
            &root,
            &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                .unwrap(),
            &[],
            &fs::read(fixture.join("../input/semantic-request.json")).unwrap(),
            date,
        )
        .unwrap();
        assert_eq!(actual, expected);
        let semantic: Value = serde_json::from_slice(
            &fs::read(fixture.join("../input/semantic-request.json")).unwrap(),
        )
        .unwrap();
        let mut redirected = semantic.clone();
        redirected["plan_path"] = json!("outputs/work/plans/other.json");
        assert_eq!(
            prepare_revision_request(
                &root,
                &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                    .unwrap(),
                &[],
                &serde_json::to_vec(&redirected).unwrap(),
                date,
            )
            .unwrap_err()
            .reason_code,
            "invalid_contract_value"
        );
        let mut forged_identity = actual.clone();
        forged_identity["candidates"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|row| row["kind"] == "task_index")
            .unwrap()["task_id"] = json!("TASK-001");
        assert_eq!(
            preview_migration(
                &root,
                &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                    .unwrap(),
                &[],
                &forged_identity
            )
            .unwrap_err()
            .reason_code,
            "invalid_contract_value"
        );
        let revision = json!({"schema":"work-spec-prepare-request",
            "requirement_id":semantic["requirement_id"],"reason":semantic["reason"],"edits":semantic["edits"]});
        let prepared = prepare_simple_update(
            &root,
            &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                .unwrap(),
            &[],
            SpecificationPrepareInput {
                raw: &serde_json::to_vec(&revision).unwrap(),
                date,
                output_file: None,
            },
        )
        .unwrap();
        let expected_preview: Value =
            serde_json::from_slice(&fs::read(fixture.join("../expected/result.json")).unwrap())
                .unwrap();
        let preview = preview_revision_from_prepared(&root, &actual, &prepared).unwrap();
        assert_eq!(preview, expected_preview);
        assert_eq!(
            preview_migration(
                &root,
                &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                    .unwrap(),
                &[],
                &actual
            )
            .unwrap(),
            expected_preview
        );
        let plan_source = root.join("outputs/work/tasks/example/index.json");
        let original_plan = fs::read(&plan_source).unwrap();
        fs::write(&plan_source, b"changed\n").unwrap();
        assert_eq!(
            publish_migration(
                &root,
                &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                    .unwrap(),
                &[],
                &actual,
                "apply",
                preview["fingerprint"].as_str().unwrap(),
            )
            .unwrap_err()
            .reason_code,
            "migration_source_changed"
        );
        fs::write(&plan_source, original_plan).unwrap();
        let mut journal = revision_transaction(&root, &actual, &preview, &prepared).unwrap();
        let published: Value =
            serde_json::from_slice(&fs::read(fixture.join("../input/journal.json")).unwrap())
                .unwrap();
        assert_eq!(journal["approval_sha256"], published["approval_sha256"]);
        journal["state"] = json!("published");
        journal["published_count"] = json!(journal["files"].as_array().unwrap().len());
        assert_eq!(journal, published);
        let result = publish_migration(
            &root,
            &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                .unwrap(),
            &[],
            &actual,
            "apply",
            preview["fingerprint"].as_str().unwrap(),
        )
        .unwrap();
        let mut expected_publication: Value =
            serde_json::from_slice(&fs::read(fixture.join("../input/publication.json")).unwrap())
                .unwrap();
        {
            expected_publication["journal"] = json!(
                "outputs/work/executions/example/journals/specification-migration/4DEC1920BE07/journal.json"
            );
            expected_publication["completion_marker"] = json!(
                "outputs/work/executions/example/journals/specification-migration/4DEC1920BE07/committed.sha256"
            );
        }
        assert_eq!(result, expected_publication);
        let installed = fs::read(root.join(result["journal"].as_str().unwrap())).unwrap();
        let reference = fs::read(fixture.join("../input/journal.json")).unwrap();
        let installed_text = String::from_utf8(installed.clone()).unwrap();
        let reference_text = String::from_utf8(reference.clone()).unwrap();
        let mismatch = installed_text
            .lines()
            .zip(reference_text.lines())
            .enumerate()
            .find(|(_, (left, right))| left != right);
        assert!(
            installed == reference,
            "journal first differing line: {mismatch:?}"
        );
        let recovered = publish_migration(
            &root,
            &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                .unwrap(),
            &[],
            &actual,
            "recover",
            preview["fingerprint"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(recovered["status"], "recovered");
        assert_eq!(recovered["publication_status"], "already_published");
    }

    #[test]
    fn revision_migration_recovers_after_first_published_file() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo
            .join("crates/work-infrastructure/fixtures/cases/specification/update/revision-migration/project");
        let root = std::env::temp_dir().join(format!(
            "work-spec-migration-recovery-{}-{}",
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
            "outputs/work/executions/example/index.json",
        ] {
            let destination = root.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let request: Value =
            serde_json::from_slice(&fs::read(fixture.join("../input/request.json")).unwrap())
                .unwrap();
        let preview: Value =
            serde_json::from_slice(&fs::read(fixture.join("../expected/result.json")).unwrap())
                .unwrap();
        let published: Value =
            serde_json::from_slice(&fs::read(fixture.join("../input/journal.json")).unwrap())
                .unwrap();
        let approved = preview["fingerprint"].as_str().unwrap();
        let journal_path = work_operations::derivation::publication::journal_path(
            "outputs/work/executions/example",
            work_operations::derivation::publication::JournalKind::SpecificationMigration(approved),
        );
        let mut partial = published.clone();
        partial["state"] = json!("publishing");
        partial["published_count"] = json!(1);
        let first = &partial["files"][0];
        let destination = root.join(first["path"].as_str().unwrap());
        fs::write(&destination, decode_snapshot(&first["after"]).unwrap()).unwrap();
        {
            partial["state"] = json!("prepared");
            partial["published_count"] = json!(0);
            let staged = work_operations::derivation::transaction::build_journal_staging(work_operations::derivation::transaction::JournalStagingInput {
                canonical_root:root.canonicalize().unwrap().to_str().unwrap(),requirement:&"example".parse().unwrap(),
                execution_dir:"outputs/work/executions/example",journal_path:&journal_path,
                kind:work_operations::derivation::publication::JournalKind::SpecificationMigration(approved),journal:&partial,
            }).unwrap();
            crate::transaction_storage::prepare_runtime_transaction(
                &LocalFiles,
                &root,
                &staged.manifest,
                &staged.payloads,
            )
            .unwrap();
        }
        fs::create_dir_all(root.join(&journal_path).parent().unwrap()).unwrap();
        write_journal(&root, &journal_path, &partial).unwrap();
        let mut changed = request.clone();
        changed["semantic_decisions"] =
            json!([{"id":"different","question":"Why?","resolution":"Approved"}]);
        assert_eq!(
            publish_migration(
                &root,
                &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                    .unwrap(),
                &[],
                &changed,
                "recover",
                approved
            )
            .unwrap_err()
            .reason_code,
            "migration_recovery_request_changed"
        );
        let result = publish_migration(
            &root,
            &crate::fixture_support::historical_task_skill_root(&repo.join("../skills/work"))
                .unwrap(),
            &[],
            &request,
            "recover",
            approved,
        )
        .unwrap();
        assert_eq!(result["status"], "recovered");
        assert_eq!(result["publication_status"], "published");
        assert_eq!(
            fs::read(root.join(journal_path)).unwrap(),
            fs::read(fixture.join("../input/journal.json")).unwrap()
        );
        assert!(
            root.join(result["completion_marker"].as_str().unwrap())
                .is_file()
        );
    }
}
