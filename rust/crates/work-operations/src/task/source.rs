//! Pure validation of exact requirement bytes and Task-owned planning decisions.
use crate::derivation::fingerprint;
use crate::protocol::valid_sha256;
use crate::task::TaskIssue;
use serde_json::{Value, json};
use std::collections::HashSet;
use work_model::task::planning::PlanningSource;
use work_model::task::source::{TaskAcceptance, TaskArtifactPaths};
fn issue(code: &'static str, message: &'static str, location: &str) -> TaskIssue {
    TaskIssue {
        reason_code: code,
        message,
        details: json!({"location":location}),
    }
}
pub fn valid_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.starts_with('/')
        && path
            .split('/')
            .all(|part| work_model::identifiers::path_segment_issue(part).is_none())
}
pub fn validate_artifacts(paths: &TaskArtifactPaths, requirement: &str) -> Result<(), TaskIssue> {
    for (name, path) in [
        ("source", &paths.source),
        ("task", &paths.task),
        ("execution", &paths.execution),
    ] {
        if !valid_relative_path(path) {
            return Err(issue(
                "invalid_artifact_path",
                "Artifact paths must be portable project-relative paths.",
                name,
            ));
        }
        let ending = if name == "task" {
            format!("/{requirement}/index.json")
        } else {
            format!("/{requirement}")
        };
        if !path.ends_with(&ending) {
            return Err(issue(
                "artifact_requirement_mismatch",
                "Artifacts must identify the same requirement.",
                name,
            ));
        }
    }
    let roots = [
        paths.source.as_str(),
        paths
            .task
            .strip_suffix("/index.json")
            .expect("validated suffix"),
        paths.execution.as_str(),
    ];
    for (n, left) in roots.iter().enumerate() {
        for right in &roots[n + 1..] {
            let left = crate::canonical::portable_path_identity(left);
            let right = crate::canonical::portable_path_identity(right);
            if left == right
                || left.starts_with(&format!("{right}/"))
                || right.starts_with(&format!("{left}/"))
            {
                return Err(issue(
                    "artifact_path_overlap",
                    "Source, Task and Execution roots must be separate.",
                    "artifacts",
                ));
            }
        }
    }
    Ok(())
}
pub fn validate_acceptance(criteria: &[TaskAcceptance], prefix: &str) -> Result<(), TaskIssue> {
    if criteria.is_empty() {
        return Err(issue(
            "missing_task_acceptance",
            "Task acceptance criteria are required.",
            "acceptance_criteria",
        ));
    }
    let mut ids = HashSet::new();
    for row in criteria {
        let suffix = row.id.strip_prefix(prefix).unwrap_or("");
        if suffix.len() < 3
            || !suffix.bytes().all(|b| b.is_ascii_digit())
            || suffix
                .parse::<u64>()
                .ok()
                .is_none_or(|n| n == 0 || format!("{n:03}") != suffix)
            || !ids.insert(&row.id)
            || row.criterion.trim().is_empty()
        {
            return Err(issue(
                "invalid_task_acceptance",
                "Acceptance IDs must be stable, unique and have explicit criteria.",
                "acceptance_criteria",
            ));
        }
    }
    Ok(())
}
pub fn validate_planning_source(
    value: &Value,
    requirement: &str,
) -> Result<PlanningSource, TaskIssue> {
    let source: PlanningSource = serde_json::from_value(value.clone()).map_err(|_| {
        issue(
            "invalid_planning_source",
            "A complete Snapshot and Task-owned planning context are required.",
            "source",
        )
    })?;
    if value["snapshot"] != serde_json::to_value(&source.snapshot).expect("Snapshot serializes") {
        return Err(issue(
            "invalid_planning_source",
            "Snapshot metadata must remain exact and unaugmented.",
            "source.snapshot",
        ));
    }
    crate::source_snapshot::validate_metadata(&source.snapshot)
        .map_err(|error| issue(error.reason_code, error.message, "source.snapshot"))?;
    if source.snapshot.requirement_id != requirement {
        return Err(issue(
            "source_requirement_mismatch",
            "Source and Task must identify the same requirement.",
            "source.snapshot.requirement_id",
        ));
    }
    validate_artifacts(&source.artifacts, requirement)?;
    validate_acceptance(&source.acceptance_criteria, "ACCEPTANCE-")?;
    validate_choices(value)?;
    Ok(source)
}

pub fn validate_choices(value: &Value) -> Result<(), TaskIssue> {
    let hierarchy_typed: work_model::hierarchy::HierarchySelection =
        serde_json::from_value(value["hierarchy_selection"].clone()).map_err(|_| {
            issue(
                "invalid_task_hierarchy_selection",
                "A confirmed hierarchy decision is required.",
                "hierarchy_selection",
            )
        })?;
    let skills_typed: work_model::skill::SkillSelection =
        serde_json::from_value(value["skill_selection"].clone()).map_err(|_| {
            issue(
                "invalid_task_skill_selection",
                "A confirmed Skill decision is required.",
                "skill_selection",
            )
        })?;
    let hierarchy = &value["hierarchy_selection"];
    let decision = hierarchy["decision"].as_str().unwrap_or("");
    let selected = hierarchy_typed.selected_paths.clone();
    let entries = hierarchy_typed.entries.clone();
    let catalog = &hierarchy_typed.catalog_sha256;
    if hierarchy_typed.schema.as_str() != "work-hierarchy-selection"
        || !valid_sha256(catalog)
        || hierarchy_typed.selection_sha256
            != fingerprint::hierarchy_selection(decision, &selected, &entries, catalog)
        || (decision == "general_only" && (!selected.is_empty() || !entries.is_empty()))
        || (decision == "instruction_paths"
            && (selected.is_empty() || selected.len() != entries.len()))
        || !matches!(decision, "general_only" | "instruction_paths")
    {
        return Err(issue(
            "invalid_task_hierarchy_selection",
            "Task hierarchy decisions must match their complete fingerprints.",
            "hierarchy_selection",
        ));
    }
    let mut selected_seen = HashSet::new();
    for (path, entry) in selected.iter().zip(&entries) {
        if !valid_relative_path(path)
            || !selected_seen.insert(path)
            || entry["path"] != *path
            || entry.as_object().is_none_or(|row| {
                row.len() != 4
                    || ![
                        "path",
                        "mode_support",
                        "mode_metadata",
                        "recommendation_reason",
                    ]
                    .iter()
                    .all(|key| row.contains_key(*key))
            })
            || entry["recommendation_reason"]
                .as_str()
                .is_none_or(|text| text.trim().is_empty())
        {
            return Err(issue(
                "invalid_task_hierarchy_selection",
                "Hierarchy entries must uniquely match their confirmed paths and complete metadata.",
                "hierarchy_selection",
            ));
        }
    }
    let skill = &value["skill_selection"];
    let decision = skill["decision"].as_str().unwrap_or("");
    let rows = skill["skills"].as_array().expect("typed skills");
    if skills_typed.schema.as_str() != "work-skill-selection"
        || skills_typed.selection_sha256 != fingerprint::skill_selection(decision, rows)
        || (decision == "base_only" && !rows.is_empty())
        || (decision == "external_skills" && rows.is_empty())
        || !matches!(decision, "base_only" | "external_skills")
        || rows
            .iter()
            .any(|row| row["mode_support"]["task"] == "unsupported" || row["available"] == false)
    {
        return Err(issue(
            "invalid_task_skill_selection",
            "Task Skill decisions must match their complete fingerprints.",
            "skill_selection",
        ));
    }
    let mut skill_ids = HashSet::new();
    for row in rows {
        let skill: work_model::skill::SelectedSkill =
            serde_json::from_value(row.clone()).map_err(|_| {
                issue(
                    "invalid_task_skill_selection",
                    "Selected Skills must retain their complete confirmed definitions.",
                    "skill_selection.skills",
                )
            })?;
        if !skill_ids.insert(skill.id.clone())
            || [
                &skill.id,
                &skill.name,
                &skill.scope,
                &skill.root,
                &skill.source,
                &skill.description,
                &skill.recommendation_reason,
            ]
            .iter()
            .any(|text| text.trim().is_empty())
            || !valid_sha256(&skill.summary_sha256)
            || !valid_sha256(&skill.bundle_sha256)
            || skill.dependency_status != "available"
            || skill.mode_support.task == work_model::skill::SkillModeSupport::Unsupported
        {
            return Err(issue(
                "invalid_task_skill_selection",
                "Selected Skills must be unique, available and support Task.",
                "skill_selection.skills",
            ));
        }
    }
    Ok(())
}
pub fn validate_formal_context(value: &Value, requirement: &str) -> Result<(), TaskIssue> {
    let source: work_model::task::source::TaskProvenance = serde_json::from_value(
        value["source"].clone(),
    )
    .map_err(|_| {
        issue(
            "invalid_task_source",
            "Task provenance requires an exact Snapshot or retained approved migration evidence.",
            "source",
        )
    })?;
    match source {
        work_model::task::source::TaskProvenance::Snapshot { manifest } => {
            crate::source_snapshot::validate_metadata(&manifest)
                .map_err(|e| issue(e.reason_code, e.message, "source.manifest"))?;
            if manifest.requirement_id != requirement
                || value["source"]["manifest"]
                    != serde_json::to_value(&manifest).expect("manifest serializes")
            {
                return Err(issue(
                    "source_requirement_mismatch",
                    "Task must bind the exact same-requirement Snapshot.",
                    "source.manifest",
                ));
            }
        }
        work_model::task::source::TaskProvenance::Migration {
            sources,
            approval_sha256,
        } => {
            if sources.is_empty()
                || approval_sha256 != fingerprint::migration_source_approval(requirement, &sources)
            {
                return Err(issue(
                    "migration_source_approval_mismatch",
                    "The complete original evidence set must match its reviewed binding.",
                    "source",
                ));
            }
            let mut seen = HashSet::new();
            for source in sources {
                if !valid_relative_path(&source.path)
                    || !seen.insert(crate::canonical::portable_path_identity(&source.path))
                    || source.raw.len() as u64 != source.size
                    || !fingerprint::verify_raw(&source.raw, &source.raw_sha256)
                {
                    return Err(issue(
                        "invalid_migration_source",
                        "Migration requires distinct portable origins and exact retained raw bytes.",
                        "source.sources",
                    ));
                }
            }
        }
    }
    let artifacts: TaskArtifactPaths =
        serde_json::from_value(value["artifacts"].clone()).map_err(|_| {
            issue(
                "invalid_artifact_path",
                "Source, Task and Execution paths are required.",
                "artifacts",
            )
        })?;
    validate_artifacts(&artifacts, requirement)?;
    let acceptance: Vec<TaskAcceptance> =
        serde_json::from_value(value["acceptance_criteria"].clone()).map_err(|_| {
            issue(
                "invalid_task_acceptance",
                "Task-owned main acceptance definitions are required.",
                "acceptance_criteria",
            )
        })?;
    validate_acceptance(&acceptance, "ACCEPTANCE-")?;
    validate_choices(value)
}

#[cfg(test)]
pub(crate) fn fixture_context() -> Value {
    let index=work_model::contract_data::registry_value()["items"]["work-task-index"]["description"]["example"].clone();
    json!({"snapshot":index["source"]["manifest"],"artifacts":index["artifacts"],"hierarchy_selection":index["hierarchy_selection"],"skill_selection":index["skill_selection"],"acceptance_criteria":index["acceptance_criteria"]})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formal_provenance_requires_exact_snapshot_or_complete_approved_raw_evidence() {
        let mut context = fixture_context();
        context["source"] = json!({"kind":"snapshot","manifest":context["snapshot"]});
        validate_formal_context(&context, "example").unwrap();
        let mut missing = context.clone();
        missing.as_object_mut().unwrap().remove("source");
        assert_eq!(
            validate_formal_context(&missing, "example")
                .unwrap_err()
                .reason_code,
            "invalid_task_source"
        );
        let mut wrong = context.clone();
        wrong["source"]["manifest"]["requirement_id"] = json!("other");
        assert_eq!(
            validate_formal_context(&wrong, "example")
                .unwrap_err()
                .reason_code,
            "source_requirement_mismatch"
        );
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../work-infrastructure/fixtures/shared/specification-migration-project/outputs/work/tasks/example/index.json"
        ));
        let migrated: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        context["source"] = migrated["source"].clone();
        validate_formal_context(&context, "example").unwrap();
        let mut unapproved = context.clone();
        unapproved["source"]["approval_sha256"] = json!("0".repeat(64));
        assert_eq!(
            validate_formal_context(&unapproved, "example")
                .unwrap_err()
                .reason_code,
            "migration_source_approval_mismatch"
        );
        let mut changed = context.clone();
        changed["source"]["sources"][0]["raw"][0] = json!(0);
        assert!(validate_formal_context(&changed, "example").is_err());
        let mut incomplete = context.clone();
        incomplete["source"]["sources"]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(validate_formal_context(&incomplete, "example").is_err());
        let mut masquerade = context.clone();
        masquerade["source"]["kind"] = json!("snapshot");
        assert_eq!(
            validate_formal_context(&masquerade, "example")
                .unwrap_err()
                .reason_code,
            "invalid_task_source"
        );
        for id in [
            "GOAL-001",
            "ACCEPTANCE-000",
            "ACCEPTANCE-01",
            "TASK-001-ACCEPTANCE-001",
        ] {
            let mut invalid = context.clone();
            invalid["acceptance_criteria"][0]["id"] = json!(id);
            assert!(
                validate_formal_context(&invalid, "example").is_err(),
                "{id}"
            );
        }
    }

    #[test]
    fn retained_origins_and_artifact_paths_reject_nonportable_segments() {
        for path in [
            "CON/source.txt",
            "a?b/source.txt",
            "a\u{1}/source.txt",
            "a//source.txt",
            "../source.txt",
            "a\\source.txt",
            "/source.txt",
            "a./source.txt",
        ] {
            assert!(!valid_relative_path(path), "{path}");
        }
        assert!(valid_relative_path("原始來源/需求.txt"));
    }
    #[test]
    fn freshly_fingerprinted_incomplete_skill_is_rejected() {
        let mut context = fixture_context();
        let rows = vec![json!({"id":"skill","mode_support":{"task":"declared"}})];
        context["skill_selection"] = json!({"schema":"work-skill-selection","decision":"external_skills","selection_sha256":fingerprint::skill_selection("external_skills",&rows),"skills":rows});
        assert_eq!(
            validate_choices(&context).unwrap_err().reason_code,
            "invalid_task_skill_selection"
        );
    }
    #[test]
    fn freshly_fingerprinted_misaligned_hierarchy_is_rejected() {
        let mut context = fixture_context();
        let paths = vec!["web".to_owned()];
        let entries = vec![
            json!({"path":"other","mode_support":{},"mode_metadata":{},"recommendation_reason":"Required."}),
        ];
        let catalog = "a".repeat(64);
        context["hierarchy_selection"] = json!({"schema":"work-hierarchy-selection","decision":"instruction_paths","selected_paths":paths,"entries":entries,"catalog_sha256":catalog,"selection_sha256":fingerprint::hierarchy_selection("instruction_paths",&paths,&entries,&catalog)});
        assert_eq!(
            validate_choices(&context).unwrap_err().reason_code,
            "invalid_task_hierarchy_selection"
        );
    }
}
