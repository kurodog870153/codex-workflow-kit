//! Read-only offline layout review evidence for the existing Migration workflow.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::ArtifactStore;
use work_operations::derivation::{fingerprint, legacy_layout};
use work_operations::identifiers::RequirementId;

use crate::files::{LocalFiles, resolve_runtime_path};

fn fail(path: &str) -> WorkError {
    WorkError::new(
        ExitCode::ArtifactIntegrity,
        "migration_layout_inventory_invalid",
        "Layout evidence cannot be inspected safely; preserve the original tree.",
        json!({"path":path}),
    )
}

fn collect(
    root: &Path,
    relative: &str,
    files: &mut BTreeMap<String, Value>,
    directories: &mut BTreeSet<String>,
) -> Result<(), WorkError> {
    let absolute = resolve_runtime_path(root, relative)?;
    let metadata = match fs::symlink_metadata(&absolute) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(fail(relative)),
    };
    if metadata.is_dir() {
        directories.insert(relative.to_owned());
        let mut entries = fs::read_dir(absolute)
            .map_err(|_| fail(relative))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| fail(relative))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| fail(relative))?;
            collect(root, &format!("{relative}/{name}"), files, directories)?;
        }
    } else if metadata.is_file() {
        let raw = LocalFiles.read_raw(&absolute)?;
        // Recheck physical identity after reading; no unsafe alias becomes review evidence.
        resolve_runtime_path(root, relative)?;
        files.insert(
            relative.to_owned(),
            json!({"path":relative,"raw_sha256":fingerprint::raw(&raw),"size":raw.len(),"raw":raw}),
        );
    } else {
        return Err(fail(relative));
    }
    Ok(())
}

/// Includes the complete scoped inventory when any old layout is present. It grants no write permission.
pub fn diagnostics(
    root: &Path,
    requirement: &RequirementId,
    source: &str,
    execution: &str,
) -> Result<Vec<Value>, WorkError> {
    let mut files = BTreeMap::new();
    let mut directories = BTreeSet::new();
    let requirement_name = requirement.as_str();
    for scope in [
        source.to_owned(),
        execution.to_owned(),
        format!("outputs/work/discussions/{requirement_name}"),
        format!("outputs/work/runtime/staging/{requirement_name}"),
        format!("outputs/work/runtime/locks/{requirement_name}"),
        ".work/transactions/pending".to_owned(),
    ] {
        collect(root, &scope, &mut files, &mut directories)?;
    }
    let mut mappings = Vec::new();
    for path in files.keys().chain(directories.iter()) {
        if let Some(mapping) = legacy_layout::classify(path, execution, source) {
            mappings.push(json!({"original_path":path,"class":mapping.class,"candidate_path":mapping.candidate_path,"disposition":mapping.disposition}));
        } else if path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with(".work-") || name.starts_with(".capture-"))
        {
            mappings.push(json!({"original_path":path,"class":"unknown_legacy","candidate_path":null,"disposition":"blocked_preserve_unknown_effects"}));
        }
    }
    if mappings.is_empty() {
        return Ok(Vec::new());
    }
    mappings.sort_by(|a, b| {
        a["original_path"]
            .as_str()
            .cmp(&b["original_path"].as_str())
    });
    let canonical_root = root.canonicalize().map_err(|_| fail(execution))?;
    let canonical_root = canonical_root.to_str().ok_or_else(|| fail(execution))?;
    let inventory = json!({"canonical_project_root":canonical_root,"requirement_id":requirement.as_str(),"source":source,"execution":execution,"files":files.values().collect::<Vec<_>>(),"directories":directories,"mappings":mappings});
    let digest = fingerprint::structured(&inventory).map_err(|_| fail(execution))?;
    Ok(vec![
        json!({"code":"legacy_layout_review_required","path":execution,"mode":"blocked","next_command":"migration semantic prepare after offline review","inventory_sha256":digest,"inventory":inventory,"offline_requirements":["Deployment owner explicitly excludes every old and current writer; PID, timeout and absence of a new lock are insufficient.","Recover pending transactions in their original environment and resolve unknown command effects before candidate preparation.","Retain exact original bytes and mappings, create an isolated current candidate, regenerate and approve its preview.","Never copy legacy locks or partial runtime files into current runtime; never reuse old approval or rerun a recorded command."]}),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-layout-review-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn all_legacy_classes_mixed_partial_and_binary_inventory_are_exact_and_read_only() {
        let root = root();
        let execution = "自訂 空白/執行";
        let source = "outputs/work/sources/example";
        let relative = [
            format!("{execution}/.work-state-writer.lock"),
            format!("{source}/.capture-SRC-001/capture.json"),
            format!("{execution}/.work-record-begin-CMD-001.tmp"),
            format!("{execution}/TASK-001/ATTEMPT-001/.work-command-CMD-001.started.json"),
            format!("{execution}/TASK-001/ATTEMPT-001/.work-command-CMD-001.finished.json"),
            format!("{execution}/.work-spec-update-SPEC-UPDATE-ABCDEF123456.json"),
            format!("{execution}/.work-spec-migration-ABCDEF123456.json"),
            format!("{execution}/.work-spec-migration-ABCDEF123456-001.json"),
            format!("{execution}/.work-spec-migration-ABCDEF123456-reconcile.json"),
            format!("{execution}/.work-instruction-migration-ABCDEF123456.json"),
            format!("{execution}/.work-source-refresh-ABCDEF123456.json.done"),
            ".work/transactions/pending/invocation/old/input.pdf".into(),
            format!("{execution}/.work-unknown.tmpx"),
            format!("{execution}/journals/specification-migration/ABCDEF123456/journal.json"),
            "outputs/work/runtime/staging/example/record-begin/tx/transaction.json".into(),
        ];
        let raw = b"%PDF-1.7\r\n\0\xff{ partial";
        for path in &relative {
            let target = root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, raw).unwrap();
        }
        let before = diagnostics(&root, &"example".parse().unwrap(), source, execution).unwrap();
        let report = &before[0];
        assert_eq!(report["mode"], "blocked");
        assert_eq!(
            report["inventory"]["files"].as_array().unwrap().len(),
            relative.len()
        );
        for file in report["inventory"]["files"].as_array().unwrap() {
            assert_eq!(file["raw"], json!(raw.as_slice()));
            assert_eq!(file["raw_sha256"], fingerprint::raw(raw));
            assert_eq!(file["size"], raw.len());
        }
        for class in [
            "writer_lock",
            "source_capture",
            "execute_temporary",
            "command_receipt",
            "journal",
            "journal_marker",
            "pending_workspace",
            "unknown_legacy",
        ] {
            assert!(
                report["inventory"]["mappings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["class"] == class),
                "{class}"
            );
        }
        assert_eq!(
            diagnostics(&root, &"example".parse().unwrap(), source, execution).unwrap(),
            before
        );
        for path in &relative {
            assert_eq!(fs::read(root.join(path)).unwrap(), raw);
        }
        fs::write(root.join(&relative[0]), b"changed").unwrap();
        assert_ne!(
            diagnostics(&root, &"example".parse().unwrap(), source, execution).unwrap()[0]["inventory_sha256"],
            report["inventory_sha256"]
        );
        assert!(!root.join("outputs/work/migrations").exists());
    }

    #[test]
    fn offline_candidate_uses_existing_prepare_preview_apply_recover_verify_and_retains_originals()
    {
        use crate::specification::{
            migration::preview_migration, migration_publication::publish_migration,
            migration_verification::verify_semantic_migration,
            reconstruction::prepare_reconstruction_request,
        };
        for interrupted in [false, true] {
            let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
            let fixture = repo.join(
                "crates/work-infrastructure/fixtures/cases/specification/migration/reconstruction",
            );
            let old = root();
            let candidate = root();
            let paths = [
                "outputs/work/plans/example.json",
                "outputs/work/tasks/example/index.json",
                "outputs/work/tasks/example/tasks/TASK-001.json",
            ];
            let mut originals = BTreeMap::new();
            let mut sources = Vec::new();
            for path in paths {
                let raw = fs::read(fixture.join("project").join(path)).unwrap();
                for tree in [&old, &candidate] {
                    let target = tree.join(path);
                    fs::create_dir_all(target.parent().unwrap()).unwrap();
                    fs::write(target, &raw).unwrap();
                }
                sources.push(json!({"path":path,"raw_sha256":fingerprint::raw(&raw)}));
                originals.insert(path.to_owned(), raw);
            }
            let execution = "自訂 空白/執行/example";
            let lock = format!("{execution}/.work-state-writer.lock");
            fs::create_dir_all(old.join(execution)).unwrap();
            let lock_raw = b"retained offline legacy owner\r\n";
            fs::write(old.join(&lock), lock_raw).unwrap();
            let report = diagnostics(
                &old,
                &"example".parse().unwrap(),
                "outputs/work/sources/example",
                execution,
            )
            .unwrap()
            .remove(0);
            let workspace = crate::clock_workspace::create_transaction_workspace(
                &candidate,
                Some("example"),
                "migration",
            )
            .unwrap();
            let evidence = format!(
                "{}/original-tree/{lock}",
                workspace["paths"]["inputs"].as_str().unwrap()
            );
            let archived = candidate.join(&evidence);
            fs::create_dir_all(archived.parent().unwrap()).unwrap();
            LocalFiles.create_new(&archived, lock_raw).unwrap();
            sources.push(json!({"path":evidence,"raw_sha256":fingerprint::raw(lock_raw)}));
            let mut request: Value = serde_json::from_slice(
                &fs::read(fixture.join("input/semantic-request.json")).unwrap(),
            )
            .unwrap();
            request["task_context"]["artifacts"]["execution"] = json!(execution);
            request["sources"] = json!(sources);
            request["semantic_decisions"].as_array_mut().unwrap().push(json!({"id":"offline-layout","resolution":{"all_writers_stopped":true,"old_binaries_disabled":true,"pending_resolved":true,"command_effects_known":true,"deployment_evidence":"Isolated test deployment: no old or new process is started; original lock is inert retained evidence and no command has run.","inventory":report["inventory"],"inventory_sha256":report["inventory_sha256"],"evidence_mapping":[{"original_path":lock,"evidence_path":evidence,"raw_sha256":fingerprint::raw(lock_raw)}]}}));
            let skill = repo.join("../skills/work");
            let mut refused = request.clone();
            refused["semantic_decisions"]
                .as_array_mut()
                .unwrap()
                .last_mut()
                .unwrap()["resolution"]["all_writers_stopped"] = json!(false);
            assert_eq!(
                prepare_reconstruction_request(
                    &candidate,
                    &skill,
                    &[],
                    &serde_json::to_vec(&refused).unwrap()
                )
                .unwrap_err()
                .reason_code,
                "migration_offline_review_required"
            );
            let prepared = prepare_reconstruction_request(
                &candidate,
                &skill,
                &[],
                &serde_json::to_vec(&request).unwrap(),
            )
            .unwrap();
            let preview = preview_migration(&candidate, &skill, &[], &prepared).unwrap();
            assert_eq!(preview["status"], "ready");
            let approval = preview["fingerprint"].as_str().unwrap();
            if interrupted {
                let transaction =
                    crate::specification::migration_publication::migration_transaction(
                        &candidate, &prepared, &preview,
                    )
                    .unwrap();
                let paths = work_feature::specification::migration_publication::publication_paths(
                    &prepared, approval,
                )
                .unwrap();
                let context = work_feature::ports::RequirementWriterContext {
                    canonical_project_root: candidate.canonicalize().unwrap(),
                    requirement_id: "example".parse().unwrap(),
                };
                let failure = work_feature::ports::with_runtime_writer(
                    &crate::writer_lock::LocalWriterLock,
                    &context,
                    work_model::runtime::LockClass::Execution,
                    |owner| {
                        crate::specification::storage::publish_retained_journal_with_owner(
                            &crate::specification::storage::RetainedJournalRuntimeInput {
                                context: &context,
                                execution,
                                relative: &paths.journal,
                                prepared_journal: &transaction,
                                recover: false,
                            },
                            owner,
                            || Ok(()),
                            |stage| {
                                if matches!(
                                    stage,
                                    crate::transaction_storage::JournalRuntimeStage::TargetWritten(
                                        _
                                    )
                                ) {
                                    Err(WorkError::new(
                                        ExitCode::IoFailure,
                                        "injected_offline_publication",
                                        "Retain the approved partial transaction.",
                                        json!({}),
                                    ))
                                } else {
                                    Ok(())
                                }
                            },
                        )
                    },
                )
                .unwrap_err();
                assert_eq!(failure.reason_code, "injected_offline_publication");
                publish_migration(&candidate, &skill, &[], &prepared, "recover", approval).unwrap();
            } else {
                assert_eq!(
                    publish_migration(&candidate, &skill, &[], &prepared, "apply", approval)
                        .unwrap()["status"],
                    "updated"
                );
            }
            verify_semantic_migration(&candidate, &skill, &[], &prepared, approval).unwrap();
            let installed = fs::read(candidate.join(format!("{execution}/index.json"))).unwrap();
            fs::write(&archived, b"evidence drift").unwrap();
            assert!(
                publish_migration(&candidate, &skill, &[], &prepared, "recover", approval).is_err()
            );
            assert_eq!(
                fs::read(candidate.join(format!("{execution}/index.json"))).unwrap(),
                installed
            );
            fs::write(&archived, lock_raw).unwrap();
            publish_migration(&candidate, &skill, &[], &prepared, "recover", approval).unwrap();
            verify_semantic_migration(&candidate, &skill, &[], &prepared, approval).unwrap();
            assert_eq!(
                fs::read(candidate.join(format!("{execution}/index.json"))).unwrap(),
                installed
            );
            for (path, raw) in originals {
                assert_eq!(fs::read(old.join(path)).unwrap(), raw);
            }
            assert_eq!(fs::read(old.join(&lock)).unwrap(), lock_raw);
            assert_eq!(fs::read(archived).unwrap(), lock_raw);
            assert!(
                !candidate
                    .join(format!("{execution}/.work-state-writer.lock"))
                    .exists()
            );
            assert!(!candidate.join("outputs/work/executions/example").exists());
            assert!(
                !candidate
                    .join(format!("{execution}/TASK-001/ATTEMPT-001"))
                    .exists()
            );
            assert!(candidate.join(workspace["path"].as_str().unwrap()).is_dir());
        }
    }

    #[test]
    fn layout_inventory_rejects_hard_links_and_preserves_every_alias_byte() {
        let root = root();
        let execution = "outputs/work/executions/example";
        let source = "outputs/work/sources/example";
        let original = root.join(format!("{execution}/.work-state-writer.lock"));
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        fs::write(&original, b"retained owner").unwrap();
        let alias = root.join("alias.lock");
        fs::hard_link(&original, &alias).unwrap();
        assert!(diagnostics(&root, &"example".parse().unwrap(), source, execution).is_err());
        assert_eq!(fs::read(original).unwrap(), b"retained owner");
        assert_eq!(fs::read(alias).unwrap(), b"retained owner");
    }

    #[cfg(unix)]
    #[test]
    fn layout_inventory_rejects_linked_capture_without_reading_foreign_tree() {
        let root = root();
        let execution = "outputs/work/executions/example";
        let source = "outputs/work/sources/example";
        fs::create_dir_all(root.join(source)).unwrap();
        fs::create_dir(root.join("foreign")).unwrap();
        fs::write(root.join("foreign/capture.json"), b"foreign evidence").unwrap();
        std::os::unix::fs::symlink(
            root.join("foreign"),
            root.join(format!("{source}/.capture-SRC-001")),
        )
        .unwrap();
        assert!(diagnostics(&root, &"example".parse().unwrap(), source, execution).is_err());
        assert_eq!(
            fs::read(root.join("foreign/capture.json")).unwrap(),
            b"foreign evidence"
        );
    }

    #[test]
    fn empty_capture_and_lock_are_evidence_and_foreign_requirement_is_not_read() {
        let root = root();
        let execution = "outputs/work/executions/example";
        let source = "outputs/work/sources/example";
        fs::create_dir_all(root.join(format!("{source}/.capture-SRC-001"))).unwrap();
        fs::create_dir_all(root.join("outputs/work/executions/other")).unwrap();
        fs::write(
            root.join("outputs/work/executions/other/.work-state-writer.lock"),
            b"foreign",
        )
        .unwrap();
        let report = diagnostics(&root, &"example".parse().unwrap(), source, execution).unwrap();
        assert!(
            report[0]["inventory"]["files"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            report[0]["inventory"]["mappings"][0]["class"],
            "source_capture"
        );
        assert_eq!(
            fs::read(root.join("outputs/work/executions/other/.work-state-writer.lock")).unwrap(),
            b"foreign"
        );
    }
}
