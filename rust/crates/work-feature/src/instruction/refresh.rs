//! Instruction refresh eligibility and impact decisions.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use work_operations::derivation::fingerprint;

use crate::error::{ExitCode, WorkError};
use crate::instruction::refresh_build::RefreshCandidate;

pub struct RefreshDecision {
    pub preview: Value,
    pub before: BTreeMap<String, Vec<u8>>,
    pub after: BTreeMap<String, Vec<u8>>,
}

pub trait RefreshImpactRepository {
    fn requirement_ids(&self) -> Result<Vec<String>, WorkError>;
    fn build_candidate(&self, requirement_id: &str) -> Result<RefreshCandidate, WorkError>;
}

pub fn source_impact(repository: &impl RefreshImpactRepository) -> Result<Value, WorkError> {
    let mut requirements = Vec::new();
    let mut changed = BTreeSet::new();
    for requirement_id in repository.requirement_ids()? {
        let candidate = repository.build_candidate(&requirement_id)?;
        changed.extend(candidate.changed_source_names);
        requirements.push(candidate.preview);
    }
    Ok(impact_summary(requirements, &changed))
}

pub fn decide_refresh(
    requirement_id: &str,
    mut before: BTreeMap<String, Vec<u8>>,
    mut after: BTreeMap<String, Vec<u8>>,
    blocked: Vec<Value>,
    changed_sources: &BTreeSet<String>,
    mut counts: Value,
) -> Result<RefreshDecision, WorkError> {
    if !blocked.is_empty() {
        before.clear();
        after.clear();
        counts = json!({"plans":0,"task_items":0,"task_indexes":0,"execution_indexes":0});
    }
    let files = after
        .iter()
        .map(|(path, raw)| {
            json!({"path":path,
                "before_sha256":fingerprint::raw(&before[path]),"after_sha256":fingerprint::raw(raw)})
        })
        .collect::<Vec<_>>();
    let evidence = json!({"requirement_id":requirement_id,"changed_sources":changed_sources,
        "blocked":blocked,"files":files});
    let approval = fingerprint::structured(&evidence).map_err(|_| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            "invalid_contract_value",
            "The refresh preview cannot be fingerprinted.",
            json!({}),
        )
    })?;
    let status = if !blocked.is_empty() {
        "review_required"
    } else if !files.is_empty() {
        "refreshable"
    } else {
        "valid"
    };
    let preview = json!({"schema":"work-source-refresh-preview/v1","status":status,
        "requirement_id":requirement_id,"changed_sources":changed_sources.len(),
        "affected":counts,"blocked":blocked,"files":files,"approved_sha256":approval});
    let _: work_model::source::SourceRefreshPreview =
        serde_json::from_value(preview.clone()).expect("source refresh preview matches its model");
    Ok(RefreshDecision {
        preview,
        before,
        after,
    })
}

pub fn impact_summary(requirements: Vec<Value>, changed_sources: &BTreeSet<String>) -> Value {
    let affected_files: usize = requirements
        .iter()
        .map(|row| row["files"].as_array().unwrap().len())
        .sum();
    let affected_requirements = requirements
        .iter()
        .filter(|row| row["status"] != "valid")
        .count();
    let blocked: usize = requirements
        .iter()
        .map(|row| row["blocked"].as_array().unwrap().len())
        .sum();
    let result = json!({"schema":"work-source-impact/v1",
        "status":if blocked > 0 {"review_required"} else if affected_files > 0 {"changes_detected"} else {"valid"},
        "refreshable":blocked == 0,"changed_sources":changed_sources.len(),
        "affected_requirements":affected_requirements,"affected_files":affected_files,
        "blocked":blocked,"requirements":requirements});
    let _: work_model::source::SourceImpact =
        serde_json::from_value(result.clone()).expect("source impact matches its model");
    result
}

pub fn batch_preview(impact: &Value) -> Result<Value, WorkError> {
    let requirements = impact["requirements"].as_array().unwrap();
    let status = if requirements
        .iter()
        .any(|row| row["status"] == "review_required")
    {
        "review_required"
    } else if requirements
        .iter()
        .any(|row| row["status"] == "refreshable")
    {
        "refreshable"
    } else {
        "valid"
    };
    let evidence = requirements
        .iter()
        .map(|row| json!([row["requirement_id"], row["approved_sha256"]]))
        .collect::<Vec<_>>();
    let approved = fingerprint::structured(&json!(evidence)).map_err(|_| {
        WorkError::new(
            ExitCode::ArtifactIntegrity,
            "invalid_contract_value",
            "The batch preview cannot be fingerprinted.",
            json!({}),
        )
    })?;
    Ok(
        json!({"schema":"work-source-refresh-batch-preview/v1","status":status,
        "requirements":requirements,"approved_sha256":approved}),
    )
}

pub fn require_apply_operation(operation: &str) -> Result<(), WorkError> {
    if operation != "apply" {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "source_refresh_operation",
            "Use apply or recover for source refresh.",
            json!({}),
        ));
    }
    Ok(())
}

pub fn require_writable_approval(
    candidate: &RefreshCandidate,
    approved_sha256: &str,
) -> Result<(), WorkError> {
    if candidate.preview["status"] != "refreshable" {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "source_refresh_not_writable",
            "The source refresh preview is not writable.",
            json!({}),
        ));
    }
    if candidate.preview["approved_sha256"] != approved_sha256 {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "source_refresh_approval_changed",
            "The approved source refresh fingerprint changed.",
            json!({}),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_refresh_drops_all_writes_and_impact_reports_review() {
        let before = BTreeMap::from([("plan.json".into(), b"before".to_vec())]);
        let after = BTreeMap::from([("plan.json".into(), b"after".to_vec())]);
        let changed = BTreeSet::from(["workflow".into()]);
        let decision = decide_refresh(
            "example",
            before,
            after,
            vec![json!({"path":"plan.json","reason":"compatibility_revision_changed"})],
            &changed,
            json!({"plans":1,"task_items":0,"task_indexes":0,"execution_indexes":0}),
        )
        .unwrap();
        assert!(decision.before.is_empty());
        assert!(decision.after.is_empty());
        assert_eq!(decision.preview["status"], "review_required");
        assert_eq!(
            impact_summary(vec![decision.preview], &changed)["refreshable"],
            false
        );
    }

    #[test]
    fn batch_preview_prioritizes_review_required() {
        let impact = json!({"requirements":[
            {"requirement_id":"a","status":"refreshable","approved_sha256":"a"},
            {"requirement_id":"b","status":"review_required","approved_sha256":"b"}
        ]});
        assert_eq!(batch_preview(&impact).unwrap()["status"], "review_required");
    }

    struct EmptyImpact;

    impl RefreshImpactRepository for EmptyImpact {
        fn requirement_ids(&self) -> Result<Vec<String>, WorkError> {
            Ok(vec![])
        }
        fn build_candidate(&self, _: &str) -> Result<RefreshCandidate, WorkError> {
            panic!("empty requirement set must not build candidates")
        }
    }

    #[test]
    fn empty_source_impact_is_valid() {
        let result = source_impact(&EmptyImpact).unwrap();
        assert_eq!(result["status"], "valid");
        assert_eq!(result["affected_requirements"], 0);
    }
}
