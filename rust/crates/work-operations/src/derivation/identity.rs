//! Typed identities derived from approved content.

use serde_json::{Value, json};

use crate::execution::ExecutionIssue;
use crate::identifiers::{IdentifierIssue, TransactionId};
use crate::protocol::ATTEMPT_ID_PREFIX;
use crate::protocol::valid_sha256;
use crate::specification::transaction::TransactionIssue;

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

/// Workspace allocation combines external clock and random inputs into a distinct ID type.
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
