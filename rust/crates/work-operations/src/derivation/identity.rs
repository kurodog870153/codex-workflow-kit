//! Typed identities derived from approved content.

use serde_json::{Value, json};

use crate::execution::ExecutionIssue;
use crate::identifiers::{IdentifierIssue, TransactionId};
use crate::protocol::ATTEMPT_ID_PREFIX;
use crate::protocol::valid_sha256;
use crate::specification::transaction::TransactionIssue;

/// Bind a runtime transaction to the complete approved context, never a short digest.
/// The canonical root is supplied by Infrastructure after filesystem validation.
pub fn runtime_transaction_identity(
    canonical_root: &str,
    requirement: &crate::identifiers::RequirementId,
    operation: &str,
    approval: &str,
    business_identity: &Value,
    targets: &[String],
) -> Result<String, TransactionIssue> {
    let valid_operation = matches!(
        operation,
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
            | "source-capture"
    );
    let mut unique = std::collections::BTreeSet::new();
    if canonical_root.is_empty()
        || !valid_operation
        || !valid_sha256(approval)
        || !business_identity.is_object()
        || targets.is_empty()
        || targets.iter().any(|target| {
            !runtime_relative_path(target)
                || !unique.insert(crate::canonical::portable_path_identity(target))
        })
    {
        return Err(runtime_identity_issue());
    }
    crate::derivation::fingerprint::structured(&json!({
        "domain":"WORK-RUNTIME-IDENTITY-V1",
        "canonical_root":canonical_root,
        "requirement_id":requirement.as_str(),
        "operation":operation,
        "approval_sha256":approval,
        "business_identity":business_identity,
        "targets":targets,
    }))
    .map_err(|_| runtime_identity_issue())
}

/// Portable relative representations are canonical before they enter identity evidence.
pub fn runtime_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && path
            .split('/')
            .all(|part| crate::identifiers::path_segment_issue(part).is_none())
}

/// A fresh owner nonce distinguishes successive acquisitions in the same requirement.
pub fn runtime_owner_identity(
    canonical_root: &str,
    requirement: &crate::identifiers::RequirementId,
    class: &str,
    instance_nonce: &str,
) -> Result<String, TransactionIssue> {
    if canonical_root.is_empty()
        || !matches!(class, "source" | "discussion" | "execution")
        || !valid_sha256(instance_nonce)
    {
        return Err(runtime_identity_issue());
    }
    crate::derivation::fingerprint::structured(&json!({
        "domain":"WORK-RUNTIME-OWNER-V1",
        "canonical_root":canonical_root,
        "requirement_id":requirement.as_str(),
        "class":class,
        "instance_nonce":instance_nonce,
    }))
    .map_err(|_| runtime_identity_issue())
}

fn runtime_identity_issue() -> TransactionIssue {
    TransactionIssue {
        reason_code: "runtime_transaction_identity",
        message: "Runtime identity requires the complete validated project, approval and inventory context.",
        details: json!({}),
    }
}

/// A Specification transaction ID is bound to a validated approval digest.
pub fn derived_transaction_id(kind: &str, approval: &str) -> Result<String, TransactionIssue> {
    if !matches!(kind, "UPDATE" | "MIGRATION" | "RECONCILIATION") || !valid_sha256(approval) {
        return Err(TransactionIssue {
            reason_code: "spec_transaction_identity",
            message: "A validated transaction kind and approval fingerprint are required.",
            details: json!({}),
        });
    }
    Ok(format!(
        "SPEC-{kind}-{}",
        approval[..12].to_ascii_uppercase()
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewTransactionKind {
    InstructionMigration,
    SourceRefresh,
}

pub fn preview_transaction_id(
    kind: PreviewTransactionKind,
    approved_preview: &str,
) -> Result<String, TransactionIssue> {
    if !valid_sha256(approved_preview) {
        return Err(TransactionIssue {
            reason_code: "spec_transaction_identity",
            message: "A validated preview fingerprint is required.",
            details: json!({}),
        });
    }
    let prefix = match kind {
        PreviewTransactionKind::InstructionMigration => "INSTRUCTION-MIGRATION",
        PreviewTransactionKind::SourceRefresh => "SOURCE-REFRESH",
    };
    Ok(format!(
        "{prefix}-{}",
        approved_preview[..12].to_ascii_uppercase()
    ))
}

/// Clock and randomness provide a portable identifier; exclusive allocation proves freshness.
pub fn workspace_transaction_id(
    utc_stamp: &str,
    random_suffix: &str,
) -> Result<TransactionId, IdentifierIssue> {
    format!("{utc_stamp}-{random_suffix}").parse()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorrectionIdentityIssue {
    InvalidExistingName(String),
    Exhausted,
}

/// Allocate the next Correction ID from canonical names already in an Attempt.
pub fn next_correction_id(
    attempt_id: &str,
    names: &[String],
) -> Result<String, CorrectionIdentityIssue> {
    let prefix = format!("{attempt_id}-CORRECTION-");
    let mut maximum = 0u16;
    for name in names {
        let number = name
            .strip_prefix(&prefix)
            .and_then(|tail| tail.strip_suffix(".json"));
        let number = number
            .filter(|tail| tail.len() == 3 && tail.bytes().all(|byte| byte.is_ascii_digit()))
            .ok_or_else(|| CorrectionIdentityIssue::InvalidExistingName(name.clone()))?;
        maximum = maximum.max(number.parse::<u16>().expect("ASCII three-digit number"));
    }
    if maximum >= 999 {
        return Err(CorrectionIdentityIssue::Exhausted);
    }
    Ok(format!("{prefix}{:03}", maximum + 1))
}

/// Deviation IDs are contiguous three-digit IDs within an Attempt.
pub fn next_deviation_id(existing_count: usize) -> Option<String> {
    (existing_count < 999).then(|| format!("DEVIATION-{:03}", existing_count + 1))
}

fn execution_issue(
    reason_code: &'static str,
    message: &'static str,
    details: Value,
) -> ExecutionIssue {
    ExecutionIssue {
        reason_code,
        message,
        details,
    }
}

pub fn next_attempt_id(source: Option<&str>) -> Result<String, ExecutionIssue> {
    let Some(source) = source else {
        return Ok("ATTEMPT-001".into());
    };
    let number = source
        .strip_prefix(ATTEMPT_ID_PREFIX)
        .and_then(|value| {
            (value.len() == 3 && value.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| value.parse::<u16>().ok())
                .flatten()
        })
        .filter(|number| *number > 0)
        .ok_or_else(|| {
            execution_issue(
                "attempt_start_invalid_source_attempt",
                "The continuation source Attempt ID is invalid.",
                json!({}),
            )
        })?;
    if number == 999 {
        return Err(execution_issue(
            "attempt_start_id_exhausted",
            "No additional three-digit Attempt ID is available.",
            json!({}),
        ));
    }
    Ok(format!("ATTEMPT-{:03}", number + 1))
}

pub fn next_record_id(base_record_id: &str, attempt: &Value) -> Result<String, ExecutionIssue> {
    let valid = ["CMD-", "OP-", "VAL-"].iter().any(|prefix| {
        base_record_id.strip_prefix(prefix).is_some_and(|digits| {
            digits.len() == 3 && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    });
    if !valid {
        return Err(execution_issue(
            "record_begin_invalid_base_record_id",
            "record_id must be a base CMD-, OP-, or VAL- identifier.",
            json!({"record_id":base_record_id}),
        ));
    }
    let instances = attempt["carried_records"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| &row["record_id"])
        .chain(
            attempt["records"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|row| &row["id"]),
        );
    let maximum = instances
        .filter_map(Value::as_str)
        .filter_map(|id| {
            let (base, retry) = id.split_once('#').unwrap_or((id, "0"));
            (base == base_record_id
                && (retry == "0" && !id.contains('#')
                    || retry.starts_with(|character: char| ('1'..='9').contains(&character))
                        && retry.bytes().all(|byte| byte.is_ascii_digit())))
            .then_some(retry)
        })
        .max_by(|left, right| left.len().cmp(&right.len()).then(left.cmp(right)));
    let Some(maximum) = maximum else {
        return Ok(base_record_id.into());
    };
    let mut digits = maximum.as_bytes().to_vec();
    for digit in digits.iter_mut().rev() {
        if *digit < b'9' {
            *digit += 1;
            break;
        }
        *digit = b'0';
    }
    if digits.first() == Some(&b'0') {
        digits.insert(0, b'1');
    }
    Ok(format!(
        "{base_record_id}#{}",
        String::from_utf8(digits).expect("ASCII digits")
    ))
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    #[test]
    fn complete_identity_separates_prefix_collisions_roots_requirements_and_retry() {
        let requirement = "example".parse().unwrap();
        let targets = vec!["自訂 目錄/execution/index.json".to_owned()];
        let approval = "a".repeat(64);
        let business =
            json!({"task_id":"TASK-001","attempt_id":"ATTEMPT-001","record_id":"CMD-001"});
        let derive = |root: &str,
                      requirement: &crate::identifiers::RequirementId,
                      approval: &str,
                      business: &Value| {
            runtime_transaction_identity(
                root,
                requirement,
                "record-begin",
                approval,
                business,
                &targets,
            )
            .unwrap()
        };
        let identity = derive("/專案 空白", &requirement, &approval, &business);
        assert_eq!(identity.len(), 64);
        assert_eq!(
            identity,
            derive("/專案 空白", &requirement, &approval, &business)
        );
        let mut collision = approval.clone();
        collision.replace_range(63..64, "b");
        assert_eq!(&approval[..12], &collision[..12]);
        assert_ne!(
            identity,
            derive("/專案 空白", &requirement, &collision, &business)
        );
        assert_ne!(
            identity,
            derive("/另一專案", &requirement, &approval, &business)
        );
        assert_ne!(
            identity,
            derive(
                "/專案 空白",
                &"other".parse().unwrap(),
                &approval,
                &business
            )
        );
        let mut retry = business.clone();
        retry["record_id"] = json!("CMD-001#2");
        assert_ne!(
            identity,
            derive("/專案 空白", &requirement, &approval, &retry)
        );
        assert_ne!(
            runtime_owner_identity("/專案 空白", &requirement, "execution", &approval).unwrap(),
            runtime_owner_identity("/專案 空白", &requirement, "execution", &collision).unwrap()
        );
    }

    #[test]
    fn incomplete_or_noncanonical_inventory_and_unknown_owner_class_are_rejected() {
        let requirement = "example".parse().unwrap();
        let approval = "a".repeat(64);
        for target in [
            "",
            "/absolute",
            "../other",
            "a/../other",
            "a//b",
            "a\\b",
            "CON/file",
            "file.",
        ] {
            assert!(
                runtime_transaction_identity(
                    "/project",
                    &requirement,
                    "record-begin",
                    &approval,
                    &json!({}),
                    &[target.into()]
                )
                .is_err(),
                "{target}"
            );
        }
        for targets in [
            vec![],
            vec!["a.json".into(), "a.json".into()],
            vec!["A.json".into(), "a.json".into()],
            vec!["café.json".into(), "cafe\u{301}.json".into()],
        ] {
            assert!(
                runtime_transaction_identity(
                    "/project",
                    &requirement,
                    "record-begin",
                    &approval,
                    &json!({}),
                    &targets
                )
                .is_err()
            );
        }
        for (root, operation, digest, business) in [
            ("", "record-begin", approval.as_str(), json!({})),
            ("/project", "unknown", approval.as_str(), json!({})),
            ("/project", "record-begin", "short", json!({})),
            ("/project", "record-begin", approval.as_str(), json!(null)),
        ] {
            assert!(
                runtime_transaction_identity(
                    root,
                    &requirement,
                    operation,
                    digest,
                    &business,
                    &["a.json".into()]
                )
                .is_err()
            );
        }
        assert!(runtime_owner_identity("/project", &requirement, "unknown", &approval).is_err());
        assert!(runtime_owner_identity("/project", &requirement, "source", "short").is_err());
    }
}
