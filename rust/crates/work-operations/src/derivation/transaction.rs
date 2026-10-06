//! Derive a complete transaction candidate from semantic file states.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::canonical::canonical_json_sha256;
use crate::derivation::fingerprint;
use crate::derivation::identity::{
    PreviewTransactionKind, derived_transaction_id, preview_transaction_id,
};
use crate::derivation::snapshot::encode_snapshot;
use crate::specification::transaction::{TransactionIssue, validate_transaction};

pub fn approval_sha256(files: &Value, metadata: &Value) -> String {
    canonical_json_sha256(&json!({"files":files,"metadata":metadata}))
        .expect("JSON value serializes")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionKind {
    Update,
    Migration,
    Reconciliation,
    InstructionMigration,
    SourceRefresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationOrder {
    Flat,
    Migration,
    Artifact { task_index_path: String },
    FinalReconciliation { task_index_path: String },
}

pub struct TransactionInput {
    pub kind: TransactionKind,
    pub order: PublicationOrder,
    pub request: Value,
    pub artifacts: Value,
    pub affected_task_ids: Vec<String>,
    pub history: BTreeMap<String, Vec<u8>>,
    pub source: BTreeMap<String, Vec<u8>>,
    pub candidate: BTreeMap<String, Vec<u8>>,
}

pub struct DerivedTransaction {
    pub journal: Value,
    pub approval_sha256: String,
}

pub struct TransactionDeriver;

pub struct JournalStagingInput<'a> {
    pub canonical_root: &'a str,
    pub requirement: &'a crate::identifiers::RequirementId,
    pub execution_dir: &'a str,
    pub journal_path: &'a str,
    pub kind: crate::derivation::publication::JournalKind<'a>,
    pub journal: &'a Value,
}

pub struct PreparedJournalStaging {
    pub manifest: work_model::runtime::RuntimeManifest,
    pub payloads: BTreeMap<String, Vec<u8>>,
    pub prepared_journal: Vec<u8>,
    pub published_journal: Vec<u8>,
}

/// Freeze original approval and final journal/marker bytes without changing the public journal.
pub fn build_journal_staging(
    input: JournalStagingInput<'_>,
) -> Result<PreparedJournalStaging, TransactionIssue> {
    use crate::derivation::{identity, publication, snapshot};
    use work_model::runtime::{
        RuntimeBytes, RuntimeFile, RuntimeManifest, RuntimePhase, RuntimeTarget,
    };
    let invalid = || TransactionIssue {
        reason_code: "journal_staging_identity",
        message: "Journal staging requires the complete original approved context.",
        details: json!({}),
    };
    validate_transaction(input.journal)?;
    if input.journal["state"] != "prepared"
        || input.journal["published_count"] != 0
        || input.journal["metadata"]["artifacts"]
            .get("execution")
            .is_some_and(|execution| execution != input.execution_dir)
        || input.canonical_root.is_empty()
    {
        return Err(invalid());
    }
    let address = publication::bind_retained_journal_path(
        input.execution_dir,
        input.journal_path,
        input.kind,
    )?;
    for path in input.journal["metadata"]["history_sha256"]
        .as_object()
        .expect("validated history map")
        .keys()
    {
        if path.starts_with(&format!("{}/journals/", input.execution_dir)) {
            let journal = if let Some(directory) = path.strip_suffix("/committed.sha256") {
                format!("{directory}/journal.json")
            } else {
                path.clone()
            };
            if !publication::retained_journal_history_includes(
                input.execution_dir,
                &journal,
                Some(input.journal_path),
            )? {
                return Err(invalid());
            }
        }
    }
    let prepared_journal = crate::specification::transaction::render_transaction(input.journal)?;
    let rows = input.journal["files"]
        .as_array()
        .expect("validated file rows");
    let mut published = input.journal.clone();
    published["state"] = json!("published");
    published["published_count"] = json!(rows.len());
    let published_journal = crate::specification::transaction::render_transaction(&published)?;
    let evidence = |bytes: Vec<u8>| RuntimeBytes {
        sha256: fingerprint::raw(&bytes),
        bytes,
    };
    let mut targets = rows
        .iter()
        .map(|row| {
            Ok(RuntimeTarget {
                path: row["path"].as_str().expect("validated path").to_owned(),
                before: row
                    .get("before")
                    .map(snapshot::decode_snapshot)
                    .transpose()?
                    .map(evidence),
                after: row
                    .get("after")
                    .map(snapshot::decode_snapshot)
                    .transpose()?
                    .map(evidence),
            })
        })
        .collect::<Result<Vec<_>, TransactionIssue>>()?;
    targets.push(RuntimeTarget {
        path: input.journal_path.to_owned(),
        before: None,
        after: Some(evidence(published_journal.clone())),
    });
    targets.push(RuntimeTarget {
        path: address.marker,
        before: None,
        after: Some(evidence(publication::completion_marker(&published_journal))),
    });
    let mut payloads = BTreeMap::from([("journal.json.tmp".to_owned(), published_journal.clone())]);
    for (position, target) in targets.iter().enumerate() {
        let bytes = target
            .after
            .as_ref()
            .or(target.before.as_ref())
            .ok_or_else(invalid)?;
        payloads.insert(format!("targets/{position}.tmp"), bytes.bytes.clone());
    }
    let inventory = payloads
        .iter()
        .map(|(path, bytes)| RuntimeFile {
            path: path.clone(),
            sha256: fingerprint::raw(bytes),
            size_bytes: bytes.len() as u64,
        })
        .collect();
    let layout_identity = match input.kind {
        publication::JournalKind::SpecificationUpdate(id) => id,
        publication::JournalKind::SpecificationMigration(id)
        | publication::JournalKind::SpecificationMigrationReconcile(id)
        | publication::JournalKind::InstructionMigration(id)
        | publication::JournalKind::SourceRefresh(id) => id,
        publication::JournalKind::SpecificationMigrationItem { approved, .. } => approved,
    };
    let business_identity = json!({"domain":"WORK-JOURNAL-PREPARED-V1","layout_identity":layout_identity,
        "item_position":address.item_position,"journal_path":input.journal_path,"original_journal":input.journal,
        "frozen_targets":targets,"runtime_payloads":payloads});
    let approval = input.journal["approval_sha256"]
        .as_str()
        .expect("validated approval");
    let transaction_identity = identity::runtime_transaction_identity(
        input.canonical_root,
        input.requirement,
        address.operation.as_str(),
        approval,
        &business_identity,
        &targets
            .iter()
            .map(|target| target.path.clone())
            .collect::<Vec<_>>(),
    )?;
    let manifest = RuntimeManifest {
        schema: "work-runtime-transaction".to_owned(),
        canonical_root: input.canonical_root.to_owned(),
        requirement_id: input.requirement.as_str().to_owned(),
        execution_dir: input.execution_dir.to_owned(),
        operation: address.operation.as_str().to_owned(),
        transaction_identity,
        approval_sha256: approval.to_owned(),
        business_identity,
        targets,
        inventory,
        published_count: 0,
        phase: RuntimePhase::Prepared,
    };
    manifest.validate_shape().map_err(|_| invalid())?;
    Ok(PreparedJournalStaging {
        manifest,
        payloads,
        prepared_journal,
        published_journal,
    })
}

/// Rebuild immutable staging proof; progress alone may differ from the frozen preparation.
pub fn restore_journal_staging(
    manifest: &work_model::runtime::RuntimeManifest,
) -> Result<PreparedJournalStaging, TransactionIssue> {
    use crate::derivation::publication::JournalKind;
    let invalid = || TransactionIssue {
        reason_code: "journal_staging_identity",
        message: "Journal staging requires the complete original approved context.",
        details: json!({}),
    };
    manifest.validate_shape().map_err(|_| invalid())?;
    let business = &manifest.business_identity;
    let layout = business["layout_identity"].as_str().ok_or_else(invalid)?;
    let kind = match manifest.operation.as_str() {
        "specification-update" => JournalKind::SpecificationUpdate(layout),
        "specification-migration" => JournalKind::SpecificationMigration(layout),
        "specification-migration-item" => JournalKind::SpecificationMigrationItem {
            approved: layout,
            position: usize::try_from(business["item_position"].as_u64().ok_or_else(invalid)?)
                .map_err(|_| invalid())?,
        },
        "specification-migration-reconcile" => JournalKind::SpecificationMigrationReconcile(layout),
        "instruction-migration" => JournalKind::InstructionMigration(layout),
        "source-refresh" => JournalKind::SourceRefresh(layout),
        _ => return Err(invalid()),
    };
    let requirement = manifest.requirement_id.parse().map_err(|_| invalid())?;
    let prepared = build_journal_staging(JournalStagingInput {
        canonical_root: &manifest.canonical_root,
        requirement: &requirement,
        execution_dir: &manifest.execution_dir,
        journal_path: business["journal_path"].as_str().ok_or_else(invalid)?,
        kind,
        journal: &business["original_journal"],
    })?;
    let mut expected = prepared.manifest.clone();
    expected.phase = manifest.phase;
    expected.published_count = manifest.published_count;
    if expected != *manifest {
        return Err(invalid());
    }
    Ok(prepared)
}

pub fn journal_control_candidates(
    manifest: &work_model::runtime::RuntimeManifest,
    raw: &[u8],
) -> Result<Vec<work_model::runtime::RuntimeManifest>, TransactionIssue> {
    use work_model::runtime::RuntimePhase as P;
    restore_journal_staging(manifest)?;
    let phases = [
        P::Prepared,
        P::Publishing,
        P::PublishedVerified,
        P::Cleaning,
    ];
    let start = phases
        .iter()
        .position(|phase| *phase == manifest.phase)
        .expect("typed phase");
    let mut candidates = Vec::new();
    for phase in phases.into_iter().skip(start) {
        for count in manifest.published_count..=manifest.targets.len() {
            let mut candidate = manifest.clone();
            candidate.phase = phase;
            candidate.published_count = count;
            if candidate.validate_shape().is_ok()
                && serde_json::to_vec(&candidate)
                    .expect("typed manifest")
                    .starts_with(raw)
            {
                candidates.push(candidate);
            }
        }
    }
    if candidates.is_empty() {
        return Err(TransactionIssue {
            reason_code: "journal_control_identity",
            message: "Journal control bytes are not a monotonic state of the frozen transaction.",
            details: json!({}),
        });
    }
    Ok(candidates)
}

impl TransactionDeriver {
    pub fn derive(input: TransactionInput) -> Result<DerivedTransaction, TransactionIssue> {
        let mut files = Vec::new();
        for path in input
            .source
            .keys()
            .chain(input.candidate.keys())
            .collect::<BTreeSet<_>>()
        {
            let before = input.source.get(path);
            let after = input.candidate.get(path);
            if before == after && !matches!(input.order, PublicationOrder::Migration) {
                continue;
            }
            let phase = phase(&input.order, path, after.is_some());
            let mut row = json!({"phase":phase,"path":path,"operation":
                if before.is_none() {"add"} else if after.is_none() {"remove"} else {"replace"}});
            if let Some(raw) = before {
                row["before"] = encode_snapshot(raw);
            }
            if let Some(raw) = after {
                row["after"] = encode_snapshot(raw);
            }
            files.push(row);
        }
        files.sort_by_key(|row| {
            (
                row["phase"].as_u64().expect("derived phase"),
                row["path"].as_str().expect("source path").to_owned(),
            )
        });
        let source_sha256 = input
            .source
            .iter()
            .map(|(path, raw)| (path.clone(), fingerprint::raw(raw)))
            .collect::<BTreeMap<_, _>>();
        let candidate_sha256 = input
            .candidate
            .iter()
            .map(|(path, raw)| (path.clone(), fingerprint::raw(raw)))
            .collect::<BTreeMap<_, _>>();
        let history_sha256 = input
            .history
            .iter()
            .map(|(path, raw)| (path.clone(), fingerprint::history(raw)))
            .collect::<BTreeMap<_, _>>();
        let files = Value::Array(files);
        let metadata = json!({"request":input.request,"artifacts":input.artifacts,
            "affected_task_ids":input.affected_task_ids,
            "history_sha256":history_sha256,
            "source_sha256":source_sha256,"candidate_sha256":candidate_sha256});
        let approval = approval_sha256(&files, &metadata);
        let id = match input.kind {
            TransactionKind::Update => derived_transaction_id("UPDATE", &approval)?,
            TransactionKind::Migration => derived_transaction_id("MIGRATION", &approval)?,
            TransactionKind::Reconciliation => derived_transaction_id("RECONCILIATION", &approval)?,
            TransactionKind::InstructionMigration => preview_transaction_id(
                PreviewTransactionKind::InstructionMigration,
                metadata["request"]["preview_fingerprint"]
                    .as_str()
                    .unwrap_or(""),
            )?,
            TransactionKind::SourceRefresh => preview_transaction_id(
                PreviewTransactionKind::SourceRefresh,
                metadata["request"]["preview_fingerprint"]
                    .as_str()
                    .unwrap_or(""),
            )?,
        };
        let journal = json!({"schema":"work-spec-transaction","transaction_id":id,
            "approval_sha256":approval,"state":"prepared","published_count":0,
            "metadata":metadata,"files":files});
        validate_transaction(&journal)?;
        Ok(DerivedTransaction {
            journal,
            approval_sha256: approval,
        })
    }
}

fn phase(order: &PublicationOrder, path: &str, has_after: bool) -> u64 {
    match order {
        PublicationOrder::Flat => 10,
        PublicationOrder::Migration => {
            if has_after {
                10
            } else {
                50
            }
        }
        PublicationOrder::Artifact { task_index_path } => {
            if path == task_index_path {
                30
            } else if path.contains("/tasks/") {
                20
            } else {
                40
            }
        }
        PublicationOrder::FinalReconciliation { task_index_path } => {
            if path == task_index_path {
                30
            } else {
                40
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_staging_freezes_all_six_layouts_and_final_evidence_without_reapproval() {
        use crate::derivation::publication::{JournalKind, RuntimeOperation};
        let execution = "自訂 execution/example";
        let result = TransactionDeriver::derive(TransactionInput {
            kind: TransactionKind::Update,
            order: PublicationOrder::Flat,
            request: json!({"requirement_id":"example"}),
            artifacts: json!({"execution":execution}),
            affected_task_ids: vec![],
            history: BTreeMap::new(),
            source: BTreeMap::from([
                ("task/index.json".into(), b"old\r\n".to_vec()),
                ("task/retired.json".into(), b"retained old bytes".to_vec()),
            ]),
            candidate: BTreeMap::from([
                ("task/index.json".into(), b"new\r\n".to_vec()),
                ("task/new.json".into(), b"new artifact".to_vec()),
            ]),
        })
        .unwrap();
        let original =
            crate::specification::transaction::render_transaction(&result.journal).unwrap();
        let approved = "ab".repeat(32);
        let requirement = "example".parse().unwrap();
        let other_requirement = "other".parse().unwrap();
        let root = if cfg!(windows) {
            "C:/project"
        } else {
            "/project"
        };
        for (kind, operation) in [
            (
                JournalKind::SpecificationUpdate(
                    result.journal["transaction_id"].as_str().unwrap(),
                ),
                RuntimeOperation::SpecificationUpdate,
            ),
            (
                JournalKind::SpecificationMigration(&approved),
                RuntimeOperation::SpecificationMigration,
            ),
            (
                JournalKind::SpecificationMigrationItem {
                    approved: &approved,
                    position: 2,
                },
                RuntimeOperation::SpecificationMigrationItem,
            ),
            (
                JournalKind::SpecificationMigrationReconcile(&approved),
                RuntimeOperation::SpecificationMigrationReconcile,
            ),
            (
                JournalKind::InstructionMigration(&approved),
                RuntimeOperation::InstructionMigration,
            ),
            (
                JournalKind::SourceRefresh(&approved),
                RuntimeOperation::SourceRefresh,
            ),
        ] {
            let path =
                crate::derivation::publication::retained_journal_path(execution, kind).unwrap();
            let build = |canonical_root, requirement, journal| {
                build_journal_staging(JournalStagingInput {
                    canonical_root,
                    requirement,
                    execution_dir: execution,
                    journal_path: &path,
                    kind,
                    journal,
                })
            };
            let prepared = build(root, &requirement, &result.journal).unwrap();
            assert_eq!(prepared.prepared_journal, original);
            assert_eq!(prepared.manifest.approval_sha256, result.approval_sha256);
            assert_eq!(prepared.manifest.operation, operation.as_str());
            assert_eq!(
                prepared.payloads["journal.json.tmp"],
                prepared.published_journal
            );
            assert_eq!(
                restore_journal_staging(&prepared.manifest)
                    .unwrap()
                    .payloads,
                prepared.payloads
            );
            assert_eq!(prepared.manifest.targets.len(), 5);
            let mut progress = prepared.manifest.clone();
            progress.phase = work_model::runtime::RuntimePhase::Publishing;
            progress.published_count = 1;
            let control = serde_json::to_vec(&progress).unwrap();
            assert_eq!(
                journal_control_candidates(&prepared.manifest, &control).unwrap(),
                vec![progress.clone()]
            );
            assert!(
                !journal_control_candidates(&prepared.manifest, &control[..control.len() - 5])
                    .unwrap()
                    .is_empty()
            );
            assert!(
                journal_control_candidates(
                    &progress,
                    &serde_json::to_vec(&prepared.manifest).unwrap()
                )
                .is_err()
            );
            assert!(journal_control_candidates(&prepared.manifest, b"foreign").is_err());
            assert_eq!(
                prepared.payloads["targets/3.tmp"],
                prepared.published_journal
            );
            assert_eq!(
                prepared.payloads["targets/4.tmp"],
                crate::derivation::publication::completion_marker(&prepared.published_journal)
            );
            assert_eq!(
                prepared.payloads.keys().cloned().collect::<BTreeSet<_>>(),
                crate::derivation::publication::runtime_inventory(operation, 5)
                    .into_iter()
                    .filter(|path| path != "transaction.json")
                    .collect()
            );
            assert_ne!(
                build("/other root", &requirement, &result.journal)
                    .unwrap()
                    .manifest
                    .transaction_identity,
                prepared.manifest.transaction_identity
            );
            assert_ne!(
                build(root, &other_requirement, &result.journal)
                    .unwrap()
                    .manifest
                    .transaction_identity,
                prepared.manifest.transaction_identity
            );
            let mut published = result.journal.clone();
            published["state"] = json!("published");
            published["published_count"] = json!(3);
            assert!(build(root, &requirement, &published).is_err());
            let mut foreign = result.journal.clone();
            foreign["metadata"]["artifacts"]["execution"] = json!("foreign");
            foreign["approval_sha256"] =
                json!(approval_sha256(&foreign["files"], &foreign["metadata"]));
            assert!(build(root, &requirement, &foreign).is_err());
            let mut tampered = prepared.manifest.clone();
            tampered.targets[0].after.as_mut().unwrap().bytes.push(0);
            assert!(restore_journal_staging(&tampered).is_err());
            let mut tampered = prepared.manifest.clone();
            tampered.business_identity["layout_identity"] = json!("foreign");
            assert!(restore_journal_staging(&tampered).is_err());
        }
        assert_eq!(
            crate::specification::transaction::render_transaction(&result.journal).unwrap(),
            original
        );
    }

    #[test]
    fn snapshot_approval_and_identity_derive_from_one_candidate() {
        let input = TransactionInput {
            kind: TransactionKind::Update,
            order: PublicationOrder::Artifact {
                task_index_path: "task/index.json".into(),
            },
            request: json!({}),
            artifacts: json!({}),
            affected_task_ids: Vec::new(),
            history: BTreeMap::new(),
            source: BTreeMap::from([("task/index.json".into(), b"old".to_vec())]),
            candidate: BTreeMap::from([("task/index.json".into(), b"new".to_vec())]),
        };
        let result = TransactionDeriver::derive(input).unwrap();
        let journal = &result.journal;
        assert_eq!(journal["files"][0]["phase"], 30);
        assert_eq!(
            journal["files"][0]["before"]["raw_sha256"],
            fingerprint::raw(b"old")
        );
        assert_eq!(
            journal["files"][0]["after"]["raw_sha256"],
            fingerprint::raw(b"new")
        );
        assert_eq!(
            journal["metadata"]["source_sha256"]["task/index.json"],
            fingerprint::raw(b"old")
        );
        assert_eq!(
            journal["metadata"]["candidate_sha256"]["task/index.json"],
            fingerprint::raw(b"new")
        );
        assert_eq!(journal["approval_sha256"], result.approval_sha256);
        assert_eq!(
            journal["transaction_id"],
            derived_transaction_id("UPDATE", &result.approval_sha256).unwrap()
        );
    }

    #[test]
    fn unchanged_source_evidence_binds_approval_without_a_publication_phase() {
        let derive = |evidence: &[u8], after: &[u8]| {
            TransactionDeriver::derive(TransactionInput {
                kind: TransactionKind::Update,
                order: PublicationOrder::Artifact {
                    task_index_path: "outputs/work/tasks/example/index.json".into(),
                },
                request: json!({"schema":"review/v1"}),
                artifacts: json!({}),
                affected_task_ids: vec![],
                history: BTreeMap::new(),
                source: BTreeMap::from([
                    (
                        "outputs/work/tasks/example/index.json".into(),
                        b"old".to_vec(),
                    ),
                    (
                        "outputs/work/sources/example/SRC-001/source.txt".into(),
                        evidence.to_vec(),
                    ),
                ]),
                candidate: BTreeMap::from([
                    (
                        "outputs/work/tasks/example/index.json".into(),
                        after.to_vec(),
                    ),
                    (
                        "outputs/work/sources/example/SRC-001/source.txt".into(),
                        evidence.to_vec(),
                    ),
                ]),
            })
            .unwrap()
        };
        let reviewed = derive(b"requirement", b"candidate");
        assert_eq!(
            reviewed.approval_sha256,
            derive(b"requirement", b"candidate").approval_sha256
        );
        assert_ne!(
            reviewed.approval_sha256,
            derive(b"different requirement", b"candidate").approval_sha256
        );
        assert_ne!(
            reviewed.approval_sha256,
            derive(b"requirement", b"different candidate").approval_sha256
        );
        assert_eq!(reviewed.journal["files"].as_array().unwrap().len(), 1);
        assert_eq!(reviewed.journal["files"][0]["phase"], 30);
    }

    #[test]
    fn history_bytes_are_fingerprinted_inside_the_transaction_boundary() {
        let build = |raw: &[u8]| {
            TransactionDeriver::derive(TransactionInput {
                kind: TransactionKind::Update,
                order: PublicationOrder::Flat,
                request: json!({}),
                artifacts: json!({}),
                affected_task_ids: Vec::new(),
                history: BTreeMap::from([("execution/TASK-001/attempt.json".into(), raw.to_vec())]),
                source: BTreeMap::from([("task/index.json".into(), b"before".to_vec())]),
                candidate: BTreeMap::from([("task/index.json".into(), b"after".to_vec())]),
            })
            .unwrap()
        };
        let original = build(b"original history");
        let changed = build(b"changed history");
        assert_eq!(
            original.journal["metadata"]["history_sha256"]["execution/TASK-001/attempt.json"],
            fingerprint::history(b"original history")
        );
        assert_ne!(original.approval_sha256, changed.approval_sha256);
        assert_ne!(
            original.journal["transaction_id"],
            changed.journal["transaction_id"]
        );
    }
}
