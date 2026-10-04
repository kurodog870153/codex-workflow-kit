//! Read-only current instruction source impact.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::error::WorkError;

pub trait SourceImpactRepository {
    fn requirement_ids(&self) -> Result<Vec<String>, WorkError>;
    fn assess(&self, requirement_id: &str) -> Result<(Value, BTreeSet<String>), WorkError>;
}

pub fn source_impact(repository: &impl SourceImpactRepository) -> Result<Value, WorkError> {
    let mut requirements = Vec::new();
    let mut changed = BTreeSet::new();
    for requirement_id in repository.requirement_ids()? {
        let (assessment, changed_names) = repository.assess(&requirement_id)?;
        changed.extend(changed_names);
        requirements.push(assessment);
    }
    let mut result = impact_summary(requirements, &changed);
    if result["status"] == "changes_detected" {
        result["status"] = json!("stale");
    }
    Ok(result)
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
    let result = json!({"schema":"work-source-impact",
        "status":if blocked > 0 {"review_required"} else if affected_files > 0 {"changes_detected"} else {"valid"},
        "changed_sources":changed_sources.len(),
        "affected_requirements":affected_requirements,"affected_files":affected_files,
        "blocked":blocked,"requirements":requirements});
    let _: work_model::source::SourceImpact =
        serde_json::from_value(result.clone()).expect("source impact matches its model");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptyImpact;

    impl SourceImpactRepository for EmptyImpact {
        fn requirement_ids(&self) -> Result<Vec<String>, WorkError> {
            Ok(vec![])
        }
        fn assess(&self, _: &str) -> Result<(Value, BTreeSet<String>), WorkError> {
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
