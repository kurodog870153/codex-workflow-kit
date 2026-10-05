//! Shared source replacement review for Task and Specification.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::task::TaskIssue;

fn issue(reason_code: &'static str, message: &'static str) -> TaskIssue {
    TaskIssue {
        reason_code,
        message,
        details: json!({}),
    }
}

/// Shared full-requirement review used by planning and formal Specification replacement.
pub fn validate_context_confirmation(
    previous: &Value,
    old_source: &Value,
    new_source: &Value,
    affected: &BTreeSet<String>,
    value: &Value,
) -> Result<(), TaskIssue> {
    let requirement = previous["requirement_id"].as_str().ok_or_else(|| {
        issue(
            "invalid_requirement_id",
            "The review baseline needs a requirement ID.",
        )
    })?;
    crate::task::source::validate_planning_source(new_source, requirement)?;
    let criteria: Vec<work_model::task::source::TaskAcceptance> =
        serde_json::from_value(old_source["acceptance_criteria"].clone()).map_err(|_| {
            issue(
                "invalid_task_acceptance",
                "The review baseline needs complete acceptance criteria.",
            )
        })?;
    crate::task::source::validate_acceptance(&criteria, "ACCEPTANCE-")?;
    let confirmation: work_model::task::request::SourceReplacementConfirmation =
        serde_json::from_value(value.clone()).map_err(|_| {
            issue(
                "source_confirmation_required",
                "Confirm the complete requirement coverage and every affected Task aspect.",
            )
        })?;
    if !confirmation.complete_requirement_review {
        return Err(issue(
            "source_requirement_review_incomplete",
            "The replacement must cover the complete retained requirement set.",
        ));
    }
    if confirmation.previous_planning_sha256
        != crate::derivation::fingerprint::structured(previous).expect("planning serializes")
        || confirmation.new_source_sha256
            != crate::derivation::fingerprint::structured(new_source).expect("Source serializes")
        || confirmation.requirement_sha256 != new_source["snapshot"]["content"]["sha256"]
    {
        return Err(issue(
            "source_confirmation_drift",
            "Confirmation must bind the exact planning baseline, replacement context and requirement bytes.",
        ));
    }
    let ids = |source: &Value| {
        source["acceptance_criteria"]
            .as_array()
            .expect("validated criteria")
            .iter()
            .map(|row| row["id"].as_str().expect("validated ID").to_owned())
            .collect::<BTreeSet<_>>()
    };
    let old = ids(old_source);
    let new = ids(new_source);
    let retained: BTreeSet<_> = confirmation
        .retained_acceptance_ids
        .iter()
        .cloned()
        .collect();
    let removed: BTreeSet<_> = confirmation
        .removed_acceptance
        .iter()
        .map(|row| row.id.clone())
        .collect();
    let added: BTreeSet<_> = confirmation.added_acceptance_ids.iter().cloned().collect();
    if retained.len() != confirmation.retained_acceptance_ids.len()
        || removed.len() != confirmation.removed_acceptance.len()
        || added.len() != confirmation.added_acceptance_ids.len()
        || retained != old.intersection(&new).cloned().collect()
        || removed != old.difference(&new).cloned().collect()
        || added != new.difference(&old).cloned().collect()
        || confirmation
            .removed_acceptance
            .iter()
            .any(|row| row.reason.trim().is_empty())
    {
        return Err(issue(
            "source_requirement_coverage_incomplete",
            "Every retained, explicitly removed and newly added acceptance must be reviewed exactly once.",
        ));
    }
    if confirmation
        .task_reviews
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        != *affected
        || confirmation.task_reviews.values().any(|review| {
            [
                &review.outcome_decisions,
                &review.technical_decisions,
                &review.boundary,
                &review.acceptance,
                &review.skills,
                &review.hierarchy,
                &review.instructions,
            ]
            .iter()
            .any(|text| text.trim().is_empty())
        })
    {
        return Err(issue(
            "source_task_review_incomplete",
            "Review outcome decisions, technical decisions, boundaries, acceptance, Skills, hierarchy and instructions for every affected Task.",
        ));
    }
    Ok(())
}
