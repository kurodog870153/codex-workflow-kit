//! Review-only legacy path mappings. No historical DTO or writer compatibility path.

use super::identity::runtime_relative_path;
use super::publication::{JournalKind, command_receipt_directory, command_receipt_instance};

/// Historical spellings are evidence addresses, never current publication destinations.
pub fn legacy_completion_marker_path(journal_path: &str) -> String {
    format!("{journal_path}.done")
}

pub fn legacy_journal_path(execution_dir: &str, kind: JournalKind<'_>) -> String {
    let short = |digest: &str| {
        digest
            .chars()
            .take(12)
            .collect::<String>()
            .to_ascii_uppercase()
    };
    let name = match kind {
        JournalKind::InstructionMigration(approved) => {
            format!(".work-instruction-migration-{}.json", short(approved))
        }
        JournalKind::SourceRefresh(approved) => {
            format!(".work-source-refresh-{}.json", short(approved))
        }
        JournalKind::SpecificationUpdate(id) => format!(".work-spec-update-{id}.json"),
        JournalKind::SpecificationMigration(approved) => {
            format!(".work-spec-migration-{}.json", short(approved))
        }
        JournalKind::SpecificationMigrationItem { approved, position } => format!(
            ".work-spec-migration-{}-{:03}.json",
            short(approved),
            position + 1
        ),
        JournalKind::SpecificationMigrationReconcile(request) => {
            format!(".work-spec-migration-{}-reconcile.json", short(request))
        }
    };
    format!("{execution_dir}/{name}")
}

pub fn command_receipt_prefix(
    execution_dir: &str,
    task_id: &str,
    attempt_id: &str,
    record_id: &str,
) -> String {
    format!(
        "{execution_dir}/{task_id}/{attempt_id}/.work-command-{}",
        record_id.replace('#', "-retry-")
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyLayout {
    pub class: &'static str,
    pub candidate_path: Option<String>,
    pub disposition: &'static str,
}

/// A mapping proposes a location only; it never approves bytes, completion or a move.
pub fn classify(path: &str, execution: &str, source: &str) -> Option<LegacyLayout> {
    if !runtime_relative_path(path)
        || !runtime_relative_path(execution)
        || !runtime_relative_path(source)
    {
        return None;
    }
    let name = path.rsplit('/').next()?;
    let result = |class, candidate_path, disposition| {
        Some(LegacyLayout {
            class,
            candidate_path,
            disposition,
        })
    };
    if path.starts_with(".work/transactions/pending/") {
        return result("pending_workspace", None, "retain_original_workspace");
    }
    if matches!(
        name,
        ".work-source-writer.lock" | ".work-state-writer.lock" | ".task-publication.lock"
    ) {
        return result(
            "writer_lock",
            None,
            "exclude_all_writers_before_reviewed_retirement",
        );
    }
    if path
        .strip_prefix(&format!("{source}/"))
        .is_some_and(|tail| {
            tail.split('/')
                .next()
                .is_some_and(|part| part.starts_with(".capture-"))
        })
    {
        return result("source_capture", None, "recover_in_original_environment");
    }
    if name.starts_with(".work-") && name.ends_with(".tmp") {
        return result("execute_temporary", None, "recover_in_original_environment");
    }
    let relative = path.strip_prefix(&format!("{execution}/"))?;
    let parts: Vec<_> = relative.split('/').collect();
    if parts.len() == 3 {
        if let Some(receipt) = name.strip_prefix(".work-command-") {
            for (suffix, filename) in [
                (".started.json", "started.json"),
                (".finished.json", "finished.json"),
            ] {
                if let Some(instance) = receipt.strip_suffix(suffix) {
                    let record = command_receipt_instance(instance).ok()?;
                    let directory =
                        command_receipt_directory(execution, parts[0], parts[1], &record).ok()?;
                    return result(
                        "command_receipt",
                        Some(format!("{directory}/{filename}")),
                        "preserve_raw_and_require_current_attempt_proof_never_rerun",
                    );
                }
            }
        }
    }
    if parts.len() != 1 {
        return None;
    }
    let (journal, marker) = name
        .strip_suffix(".done")
        .map_or((name, false), |journal| (journal, true));
    let stem = journal.strip_suffix(".json")?;
    let hex = |value: &str| {
        value.len() == 12
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_lowercase())
    };
    let leaf = if let Some(id) = stem.strip_prefix(".work-spec-update-") {
        if !id.starts_with("SPEC-UPDATE-") || !hex(id.strip_prefix("SPEC-UPDATE-")?) {
            return None;
        }
        format!("specification-update/{id}")
    } else if let Some(id) = stem.strip_prefix(".work-spec-migration-") {
        if hex(id) {
            format!("specification-migration/{id}")
        } else if let Some(id) = id.strip_suffix("-reconcile").filter(|id| hex(id)) {
            format!("specification-migration/{id}/reconcile")
        } else {
            let (id, item) = id.rsplit_once('-')?;
            if !hex(id)
                || item.len() != 3
                || !item.bytes().all(|b| b.is_ascii_digit())
                || item == "000"
            {
                return None;
            }
            format!("specification-migration/{id}/items/{item}")
        }
    } else if let Some(id) = stem
        .strip_prefix(".work-instruction-migration-")
        .filter(|id| hex(id))
    {
        format!("instruction-migration/{id}")
    } else {
        let id = stem
            .strip_prefix(".work-source-refresh-")
            .filter(|id| hex(id))?;
        format!("source-refresh/{id}")
    };
    result(
        if marker { "journal_marker" } else { "journal" },
        Some(format!(
            "{execution}/journals/{leaf}/{}",
            if marker {
                "committed.sha256"
            } else {
                "journal.json"
            }
        )),
        "preserve_raw_rebuild_and_approve_current_transaction",
    )
}

/// The existing semantic decision carries explicit deployment review and exact archived sources.
/// This is evidence validation, never a claim that a process probe can prove quiescence.
pub fn validate_offline_review(request: &serde_json::Value) -> Result<(), &'static str> {
    use std::collections::BTreeSet;
    let decisions = request["semantic_decisions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let reviews: Vec<_> = decisions
        .iter()
        .filter(|row| row["id"] == "offline-layout")
        .collect();
    let legacy_sources = request["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|row| {
            row["path"].as_str().is_some_and(|path| {
                path.split('/').any(|part| {
                    part.starts_with(".work-") || part.starts_with(".capture-") || part == ".work"
                })
            })
        });
    if reviews.is_empty() && !legacy_sources {
        return Ok(());
    }
    if reviews.len() != 1 {
        return Err("migration_offline_review_missing");
    }
    let review = &reviews[0]["resolution"];
    for field in [
        "all_writers_stopped",
        "old_binaries_disabled",
        "pending_resolved",
        "command_effects_known",
    ] {
        if review[field] != true {
            return Err("migration_offline_review_required");
        }
    }
    if review["deployment_evidence"]
        .as_str()
        .is_none_or(|text| text.trim().is_empty())
    {
        return Err("migration_offline_review_required");
    }
    let inventory = &review["inventory"];
    if super::fingerprint::structured(inventory)
        .map_err(|_| "migration_offline_inventory_invalid")?
        != review["inventory_sha256"]
    {
        return Err("migration_offline_inventory_invalid");
    }
    let requirement = request["requirement_id"].as_str().or_else(|| {
        request["candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["kind"] == "task_index")
            .and_then(|row| row["content"]["requirement_id"].as_str())
    });
    if requirement.is_none() || inventory["requirement_id"].as_str() != requirement {
        return Err("migration_offline_inventory_invalid");
    }
    let files = inventory["files"]
        .as_array()
        .ok_or("migration_offline_inventory_invalid")?;
    let mappings = review["evidence_mapping"]
        .as_array()
        .ok_or("migration_offline_inventory_invalid")?;
    let sources = request["sources"]
        .as_array()
        .ok_or("migration_offline_inventory_invalid")?;
    let history = |path: &str| {
        path.split('/')
            .any(|part| part.starts_with("ATTEMPT-") || part.starts_with("CORRECTION-"))
    };
    if request["mode"] == "reconstruction"
        && files
            .iter()
            .any(|row| row["path"].as_str().is_some_and(history))
    {
        return Err("migration_offline_history_unsupported");
    }
    for file in files {
        let path = file["path"]
            .as_str()
            .ok_or("migration_offline_inventory_invalid")?;
        if history(path) {
            if path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with(".work-command-"))
            {
                let parent = path
                    .rsplit_once('/')
                    .ok_or("migration_offline_inventory_invalid")?
                    .0;
                if !files
                    .iter()
                    .any(|row| row["path"] == format!("{parent}/attempt.json"))
                {
                    return Err("migration_offline_history_unsupported");
                }
            } else if (path.ends_with("/attempt.json") || path.contains("/corrections/"))
                && !sources
                    .iter()
                    .any(|row| row["path"] == path && row["raw_sha256"] == file["raw_sha256"])
            {
                return Err("migration_offline_history_unsupported");
            }
        }
    }
    if files.is_empty() || files.len() != mappings.len() {
        return Err("migration_offline_inventory_invalid");
    }
    if inventory["mappings"].as_array().is_none_or(|rows| {
        rows.is_empty() || rows.iter().any(|row| row["class"] == "unknown_legacy")
    }) {
        return Err("migration_offline_unknown_effects");
    }
    let mut original_paths = BTreeSet::new();
    let mut evidence_paths = BTreeSet::new();
    for mapping in mappings {
        let original = mapping["original_path"]
            .as_str()
            .ok_or("migration_offline_inventory_invalid")?;
        let evidence = mapping["evidence_path"]
            .as_str()
            .ok_or("migration_offline_inventory_invalid")?;
        if !runtime_relative_path(original)
            || !runtime_relative_path(evidence)
            || !original_paths.insert(original)
            || !evidence_paths.insert(crate::canonical::portable_path_identity(evidence))
        {
            return Err("migration_offline_inventory_invalid");
        }
        let file = files
            .iter()
            .find(|row| row["path"] == original)
            .ok_or("migration_offline_inventory_invalid")?;
        let raw: Vec<u8> = serde_json::from_value(file["raw"].clone())
            .map_err(|_| "migration_offline_inventory_invalid")?;
        if file["size"] != raw.len()
            || file["raw_sha256"] != super::fingerprint::raw(&raw)
            || mapping["raw_sha256"] != file["raw_sha256"]
            || !sources
                .iter()
                .any(|row| row["path"] == evidence && row["raw_sha256"] == file["raw_sha256"])
        {
            return Err("migration_offline_inventory_invalid");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_evidence_addresses_preserve_exact_spelling() {
        assert_eq!(
            legacy_completion_marker_path("execution/.work-spec-update.json"),
            "execution/.work-spec-update.json.done"
        );
        assert_eq!(
            command_receipt_prefix("execution", "TASK-001", "ATTEMPT-002", "CMD-001#3"),
            "execution/TASK-001/ATTEMPT-002/.work-command-CMD-001-retry-3"
        );
        assert_eq!(
            legacy_journal_path(
                "execution",
                JournalKind::SpecificationMigrationItem {
                    approved: &"a".repeat(64),
                    position: 2,
                }
            ),
            "execution/.work-spec-migration-AAAAAAAAAAAA-003.json"
        );
    }

    #[test]
    fn every_legacy_family_maps_only_reviewed_locations_with_full_custom_context() {
        let execution = "自訂 空白/執行";
        let source = "自訂 來源/example";
        let journals = [
            (
                ".work-spec-update-SPEC-UPDATE-ABCDEF123456.json",
                "specification-update/SPEC-UPDATE-ABCDEF123456",
            ),
            (
                ".work-spec-migration-ABCDEF123456.json",
                "specification-migration/ABCDEF123456",
            ),
            (
                ".work-spec-migration-ABCDEF123456-007.json",
                "specification-migration/ABCDEF123456/items/007",
            ),
            (
                ".work-spec-migration-ABCDEF123456-reconcile.json",
                "specification-migration/ABCDEF123456/reconcile",
            ),
            (
                ".work-instruction-migration-ABCDEF123456.json",
                "instruction-migration/ABCDEF123456",
            ),
            (
                ".work-source-refresh-ABCDEF123456.json",
                "source-refresh/ABCDEF123456",
            ),
        ];
        for (old, leaf) in journals {
            for (suffix, new) in [("", "journal.json"), (".done", "committed.sha256")] {
                let mapping =
                    classify(&format!("{execution}/{old}{suffix}"), execution, source).unwrap();
                assert_eq!(
                    mapping.candidate_path,
                    Some(format!("{execution}/journals/{leaf}/{new}"))
                );
                assert_eq!(
                    mapping.disposition,
                    "preserve_raw_rebuild_and_approve_current_transaction"
                );
            }
        }
        let receipt = classify(
            &format!(
                "{execution}/TASK-003/ATTEMPT-002/.work-command-CMD-005-retry-2.finished.json"
            ),
            execution,
            source,
        )
        .unwrap();
        assert_eq!(
            receipt.candidate_path,
            Some(format!(
                "{execution}/TASK-003/ATTEMPT-002/receipts/CMD-005-retry-2/finished.json"
            ))
        );
        for (path, class) in [
            (
                format!("{execution}/.work-state-writer.lock"),
                "writer_lock",
            ),
            (
                format!("{source}/.capture-SRC-003/capture.json"),
                "source_capture",
            ),
            (
                format!("{execution}/.work-record-finish-CMD-001.tmp"),
                "execute_temporary",
            ),
            (
                ".work/transactions/pending/invocation/old/input.pdf".into(),
                "pending_workspace",
            ),
        ] {
            let mapping = classify(&path, execution, source).unwrap();
            assert_eq!(mapping.class, class);
            assert!(mapping.candidate_path.is_none());
        }
    }

    #[test]
    fn offline_review_requires_complete_exact_evidence_and_explicit_deployment_facts() {
        use serde_json::json;
        let raw = b"retained legacy journal\r\n";
        let sha = super::super::fingerprint::raw(raw);
        let original = "execution/example/.work-spec-migration-ABCDEF123456.json";
        let evidence = "outputs/work/transactions/example/migration/archive/inputs/original.bin";
        let inventory = json!({"requirement_id":"example","files":[{"path":original,"raw":raw.as_slice(),"size":raw.len(),"raw_sha256":sha}],"mappings":[{"original_path":original,"class":"journal"}]});
        let mut request = json!({"requirement_id":"example","sources":[{"path":evidence,"raw_sha256":sha}],"semantic_decisions":[{"id":"offline-layout","resolution":{"all_writers_stopped":true,"old_binaries_disabled":true,"pending_resolved":true,"command_effects_known":true,"deployment_evidence":"Deployment owner stopped old and current binaries and reviewed original recovery.","inventory_sha256":super::super::fingerprint::structured(&inventory).unwrap(),"inventory":inventory,"evidence_mapping":[{"original_path":original,"evidence_path":evidence,"raw_sha256":sha}]}}]});
        assert_eq!(validate_offline_review(&request), Ok(()));
        for field in [
            "all_writers_stopped",
            "old_binaries_disabled",
            "pending_resolved",
            "command_effects_known",
        ] {
            let mut changed = request.clone();
            changed["semantic_decisions"][0]["resolution"][field] = json!(false);
            assert_eq!(
                validate_offline_review(&changed),
                Err("migration_offline_review_required")
            );
        }
        let mut drift = request.clone();
        drift["semantic_decisions"][0]["resolution"]["inventory"]["files"][0]["raw"][0] = json!(0);
        assert_eq!(
            validate_offline_review(&drift),
            Err("migration_offline_inventory_invalid")
        );
        drift["semantic_decisions"][0]["resolution"]["inventory_sha256"] = json!(
            super::super::fingerprint::structured(
                &drift["semantic_decisions"][0]["resolution"]["inventory"]
            )
            .unwrap()
        );
        assert_eq!(
            validate_offline_review(&drift),
            Err("migration_offline_inventory_invalid")
        );
        let mut missing = request.clone();
        missing["sources"] = json!([]);
        assert_eq!(
            validate_offline_review(&missing),
            Err("migration_offline_inventory_invalid")
        );
        let mut unknown = request.clone();
        unknown["semantic_decisions"][0]["resolution"]["inventory"]["mappings"][0]["class"] =
            json!("unknown_legacy");
        unknown["semantic_decisions"][0]["resolution"]["inventory_sha256"] = json!(
            super::super::fingerprint::structured(
                &unknown["semantic_decisions"][0]["resolution"]["inventory"]
            )
            .unwrap()
        );
        assert_eq!(
            validate_offline_review(&unknown),
            Err("migration_offline_unknown_effects")
        );
        request["sources"][0]["path"] = json!(original);
        request["semantic_decisions"] = json!([]);
        assert_eq!(
            validate_offline_review(&request),
            Err("migration_offline_review_missing")
        );
        assert_eq!(validate_offline_review(&json!({"sources":[]})), Ok(()));
    }

    #[test]
    fn archived_attempt_or_receipt_cannot_be_reconstructed_as_an_empty_execution_history() {
        use serde_json::json;
        let raw = b"immutable original evidence";
        let sha = super::super::fingerprint::raw(raw);
        let evidence = "outputs/work/transactions/example/migration/archive/inputs/attempt.json";
        for original in [
            "execution/example/TASK-001/ATTEMPT-001/attempt.json",
            "execution/example/TASK-001/ATTEMPT-001/.work-command-CMD-001.finished.json",
        ] {
            let inventory = json!({"requirement_id":"example","files":[{"path":original,"raw":raw.as_slice(),"size":raw.len(),"raw_sha256":sha}],"mappings":[{"original_path":original,"class":"command_receipt"}]});
            let mut request = json!({"mode":"reconstruction","requirement_id":"example","sources":[{"path":evidence,"raw_sha256":sha}],"semantic_decisions":[{"id":"offline-layout","resolution":{"all_writers_stopped":true,"old_binaries_disabled":true,"pending_resolved":true,"command_effects_known":true,"deployment_evidence":"Explicit reviewed offline deployment.","inventory":inventory,"inventory_sha256":super::super::fingerprint::structured(&inventory).unwrap(),"evidence_mapping":[{"original_path":original,"evidence_path":evidence,"raw_sha256":sha}]}}]});
            assert_eq!(
                validate_offline_review(&request),
                Err("migration_offline_history_unsupported")
            );
            request["mode"] = json!("revision");
            assert_eq!(
                validate_offline_review(&request),
                Err("migration_offline_history_unsupported")
            );
        }
    }

    #[test]
    fn malformed_identity_alias_and_foreign_receipt_never_get_a_mapping() {
        for path in [
            "../execution/.work-spec-migration-ABCDEF123456.json",
            "execution/.work-spec-migration-abcdef123456.json",
            "execution/.work-spec-migration-ABCDEF123456-000.json",
            "execution/TASK-000/ATTEMPT-001/.work-command-CMD-001.started.json",
            "execution/TASK-001/ATTEMPT-001/.work-command-CMD-001-retry-02.started.json",
            "execution/.work-unknown.json",
            "other/.work-spec-migration-ABCDEF123456.json",
        ] {
            assert!(classify(path, "execution", "source").is_none(), "{path}");
        }
    }
}
