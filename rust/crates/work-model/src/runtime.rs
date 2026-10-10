//! Internal runtime evidence; these shapes do not add public CLI schemas.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimePhase {
    Prepared,
    Publishing,
    PublishedVerified,
    Cleaning,
    Restoring,
    Restored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LockClass {
    Source,
    Discussion,
    Execution,
}

impl LockClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Discussion => "discussion",
            Self::Execution => "execution",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeOwner {
    pub canonical_root: String,
    pub requirement_id: String,
    pub class: LockClass,
    pub instance_nonce: String,
    pub owner_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeBytes {
    pub bytes: Vec<u8>,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTarget {
    /// Canonical project-relative formal location, never an inventory-relative path.
    pub path: String,
    pub before: Option<RuntimeBytes>,
    pub after: Option<RuntimeBytes>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeFile {
    /// Relative to this transaction directory; transaction.json is implicit.
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    pub schema: String,
    pub canonical_root: String,
    pub requirement_id: String,
    pub execution_dir: String,
    pub operation: String,
    pub transaction_identity: String,
    pub approval_sha256: String,
    pub business_identity: Value,
    pub targets: Vec<RuntimeTarget>,
    pub inventory: Vec<RuntimeFile>,
    pub published_count: usize,
    pub phase: RuntimePhase,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && path
            .split('/')
            .all(|part| crate::identifiers::path_segment_issue(part).is_none())
}

impl RuntimeOwner {
    /// Hash equality and physical canonical-root binding are checked by Operations/Infrastructure.
    pub fn validate_shape(&self) -> Result<(), &'static str> {
        if self.canonical_root.is_empty()
            || self
                .requirement_id
                .parse::<crate::identifiers::RequirementId>()
                .is_err()
            || !digest(&self.instance_nonce)
            || !digest(&self.owner_identity)
        {
            return Err("runtime_owner_invalid");
        }
        Ok(())
    }
}

impl RuntimeManifest {
    /// Single-document invariants precede cross-document identity and raw-byte verification.
    pub fn validate_shape(&self) -> Result<(), &'static str> {
        let operation = matches!(
            self.operation.as_str(),
            "project-files"
                | "attempt-start"
                | "record-begin"
                | "command-correction"
                | "record-finish"
                | "deviation-record"
                | "attempt-close"
                | "correction"
                | "specification-update"
                | "specification-migration"
                | "specification-migration-item"
                | "specification-migration-reconcile"
                | "instruction-migration"
                | "source-refresh"
        );
        if self.schema != "work-runtime-transaction"
            || self.canonical_root.is_empty()
            || self
                .requirement_id
                .parse::<crate::identifiers::RequirementId>()
                .is_err()
            || !relative(&self.execution_dir)
            || !operation
            || !digest(&self.transaction_identity)
            || !digest(&self.approval_sha256)
            || !self.business_identity.is_object()
            || self.targets.is_empty()
            || self.inventory.is_empty()
            || self.published_count > self.targets.len()
            || (self.phase == RuntimePhase::Prepared && self.published_count != 0)
            || (matches!(self.phase, RuntimePhase::Restoring | RuntimePhase::Restored)
                && self.operation != "project-files")
            || (matches!(
                self.phase,
                RuntimePhase::PublishedVerified | RuntimePhase::Cleaning
            ) && self.published_count != self.targets.len())
        {
            return Err("runtime_manifest_invalid");
        }
        let mut targets = BTreeSet::new();
        for target in &self.targets {
            if !relative(&target.path)
                || !targets.insert(&target.path)
                || target.path.starts_with("outputs/work/runtime/")
                || (target.before.is_none() && target.after.is_none())
                || target
                    .before
                    .iter()
                    .chain(target.after.iter())
                    .any(|bytes| !digest(&bytes.sha256))
            {
                return Err("runtime_target_invalid");
            }
        }
        let mut inventory = BTreeSet::new();
        for file in &self.inventory {
            if !relative(&file.path)
                || file.path == "transaction.json"
                || !inventory.insert(&file.path)
                || !digest(&file.sha256)
            {
                return Err("runtime_inventory_invalid");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest() -> RuntimeManifest {
        serde_json::from_value(json!({
            "schema":"work-runtime-transaction","canonical_root":"/專案 空白",
            "requirement_id":"example","execution_dir":"custom/execution",
            "operation":"record-begin","transaction_identity":"a".repeat(64),
            "approval_sha256":"b".repeat(64),"business_identity":{"task_id":"TASK-001"},
            "targets":[{"path":"custom/execution/index.json","before":null,
                "after":{"bytes":[10],"sha256":"c".repeat(64)}}],
            "inventory":[{"path":"index.json.tmp","sha256":"c".repeat(64),"size_bytes":1}],
            "published_count":0,"phase":"prepared"
        }))
        .unwrap()
    }

    #[test]
    fn unknown_incomplete_or_conflicting_manifests_are_rejected() {
        let valid = manifest();
        valid.validate_shape().unwrap();
        let mut value = serde_json::to_value(&valid).unwrap();
        value["unknown"] = json!(true);
        assert!(serde_json::from_value::<RuntimeManifest>(value).is_err());
        let mut missing = serde_json::to_value(&valid).unwrap();
        missing.as_object_mut().unwrap().remove("approval_sha256");
        assert!(serde_json::from_value::<RuntimeManifest>(missing).is_err());
        for field in ["approval_sha256", "transaction_identity"] {
            let mut damaged = serde_json::to_value(&valid).unwrap();
            damaged[field] = json!("short");
            assert!(
                serde_json::from_value::<RuntimeManifest>(damaged)
                    .unwrap()
                    .validate_shape()
                    .is_err()
            );
        }
        for path in ["../outside", "outputs/work/runtime/foreign/index.json.tmp"] {
            let mut damaged = valid.clone();
            damaged.targets[0].path = path.into();
            assert!(damaged.validate_shape().is_err());
        }
        let mut damaged = valid.clone();
        damaged.inventory.push(damaged.inventory[0].clone());
        assert!(damaged.validate_shape().is_err());
        damaged = valid.clone();
        damaged.inventory[0].path = "transaction.json".into();
        assert!(damaged.validate_shape().is_err());
        damaged = valid.clone();
        damaged.targets.push(damaged.targets[0].clone());
        assert!(damaged.validate_shape().is_err());
    }

    #[test]
    fn cleanup_requires_complete_publication_and_retains_identity_evidence() {
        let mut manifest = manifest();
        manifest.phase = RuntimePhase::Cleaning;
        assert!(manifest.validate_shape().is_err());
        manifest.published_count = 1;
        manifest.validate_shape().unwrap();
        let round_trip: RuntimeManifest =
            serde_json::from_slice(&serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert_eq!(round_trip, manifest);
        let owner = RuntimeOwner {
            canonical_root: "/專案 空白".into(),
            requirement_id: "example".into(),
            class: LockClass::Execution,
            instance_nonce: "a".repeat(64),
            owner_identity: "b".repeat(64),
        };
        owner.validate_shape().unwrap();
        let mut damaged = owner;
        damaged.requirement_id = "../other".into();
        assert!(damaged.validate_shape().is_err());
    }
}
