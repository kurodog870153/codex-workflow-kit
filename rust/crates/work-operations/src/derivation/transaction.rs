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
    Artifact {
        plan_path: String,
        task_index_path: String,
    },
    FinalReconciliation {
        task_index_path: String,
    },
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
        let journal = json!({"schema":"work-spec-transaction/v1","transaction_id":id,
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
        PublicationOrder::Artifact {
            plan_path,
            task_index_path,
        } => {
            if path.contains("/tasks/") {
                20
            } else if path == plan_path {
                10
            } else if path == task_index_path {
                30
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
    fn snapshot_approval_and_identity_derive_from_one_candidate() {
        let input = TransactionInput {
            kind: TransactionKind::Update,
            order: PublicationOrder::Artifact {
                plan_path: "plan.json".into(),
                task_index_path: "task/index.json".into(),
            },
            request: json!({}),
            artifacts: json!({}),
            affected_task_ids: Vec::new(),
            history: BTreeMap::new(),
            source: BTreeMap::from([("plan.json".into(), b"old".to_vec())]),
            candidate: BTreeMap::from([("plan.json".into(), b"new".to_vec())]),
        };
        let result = TransactionDeriver::derive(input).unwrap();
        let journal = &result.journal;
        assert_eq!(journal["files"][0]["phase"], 10);
        assert_eq!(
            journal["files"][0]["before"]["raw_sha256"],
            fingerprint::raw(b"old")
        );
        assert_eq!(
            journal["files"][0]["after"]["raw_sha256"],
            fingerprint::raw(b"new")
        );
        assert_eq!(
            journal["metadata"]["source_sha256"]["plan.json"],
            fingerprint::raw(b"old")
        );
        assert_eq!(
            journal["metadata"]["candidate_sha256"]["plan.json"],
            fingerprint::raw(b"new")
        );
        assert_eq!(journal["approval_sha256"], result.approval_sha256);
        assert_eq!(
            journal["transaction_id"],
            derived_transaction_id("UPDATE", &result.approval_sha256).unwrap()
        );
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
                source: BTreeMap::from([("plan.json".into(), b"before".to_vec())]),
                candidate: BTreeMap::from([("plan.json".into(), b"after".to_vec())]),
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
