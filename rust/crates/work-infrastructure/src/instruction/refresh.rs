//! Read-only current instruction and routing impact.

use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::instruction::refresh_storage::discover_requirements;
use crate::routing_sources::RoutingSourceSession;
use crate::specification::storage::storage_path;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use work_feature::artifact_paths::default_artifact_paths;
use work_feature::error::{ExitCode, WorkError};
use work_feature::instruction::refresh::{
    SourceImpactRepository, source_impact as build_source_impact,
};
use work_feature::instruction::refresh_build::{ImpactRoutingRepository, ImpactSnapshotRepository};
use work_operations::identifiers::RequirementId;

struct LocalImpactSnapshot<'a> {
    project_root: &'a Path,
}
impl ImpactSnapshotRepository for LocalImpactSnapshot<'_> {
    fn discover_requirements(&self) -> Result<BTreeMap<String, Value>, WorkError> {
        discover_requirements(self.project_root)
    }
    fn default_paths(&self, id: &RequirementId) -> work_model::task::source::TaskArtifactPaths {
        default_artifact_paths(id)
    }
    fn source_evidence(&self, index: &Value) -> Result<BTreeMap<String, Vec<u8>>, WorkError> {
        let requirement = index["requirement_id"]
            .as_str()
            .ok_or_else(|| failure("invalid_requirement_id", "A Task requirement is required."))?;
        work_operations::task::source::validate_formal_context(index, requirement).map_err(
            |e| {
                WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    e.reason_code,
                    e.message,
                    e.details,
                )
            },
        )?;
        let paths = crate::artifact_paths::LocalArtifactPaths {
            project_root: self.project_root.to_path_buf(),
        };
        work_feature::task::source::verify_provenance(
            &paths,
            requirement,
            &serde_json::from_value(index["source"].clone()).expect("validated Task provenance"),
            &serde_json::from_value(index["artifacts"].clone())
                .expect("validated Task artifact paths"),
        )?;
        let mut evidence = BTreeMap::new();
        for relative in work_feature::task::source::evidence_paths(index)? {
            evidence.insert(relative.clone(), self.read(&relative)?);
        }
        Ok(evidence)
    }
    fn exists(&self, relative: &str) -> Result<bool, WorkError> {
        Ok(storage_path(self.project_root, relative)?.is_file())
    }
    fn read(&self, relative: &str) -> Result<Vec<u8>, WorkError> {
        fs::read(storage_path(self.project_root, relative)?)
            .map_err(|_| failure("file_read_failed", "A formal artifact could not be read."))
    }
}
impl ImpactRoutingRepository for RoutingSourceSession {
    fn recheck_sources(&self) -> Result<(), WorkError> {
        RoutingSourceSession::recheck(self)
    }
}
fn failure(reason: &str, message: &str) -> WorkError {
    WorkError::new(ExitCode::ArtifactIntegrity, reason, message, json!({}))
}
pub fn source_impact(project_root: &Path, skill_root: &Path) -> Result<Value, WorkError> {
    build_source_impact(&LocalSourceImpact {
        project_root,
        skill_root,
    })
}

struct LocalSourceImpact<'a> {
    project_root: &'a Path,
    skill_root: &'a Path,
}

impl SourceImpactRepository for LocalSourceImpact<'_> {
    fn requirement_ids(&self) -> Result<Vec<String>, WorkError> {
        Ok(discover_requirements(self.project_root)?
            .into_keys()
            .collect())
    }

    fn assess(
        &self,
        requirement_id: &str,
    ) -> Result<(Value, std::collections::BTreeSet<String>), WorkError> {
        let source = LocalHierarchyCatalog {
            skill_root: self.skill_root.to_path_buf(),
        };
        let mut routing = RoutingSourceSession::new(self.skill_root.to_path_buf());
        match work_feature::instruction::refresh_build::inspect_source_impact(
            &LocalImpactSnapshot {
                project_root: self.project_root,
            },
            &source,
            &mut routing,
            requirement_id,
        ) {
            Ok(assessment) => Ok(assessment),
            Err(error) => Ok((
                json!({"requirement_id":requirement_id,"status":"review_required",
                "files":[],"blocked":[{"reason":error.reason_code,"required_action":"migration"}]}),
                std::collections::BTreeSet::new(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root(label: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture =
            repo.join("crates/work-infrastructure/fixtures/specification-update/item-goal");
        let root = std::env::temp_dir().join(format!(
            "work-task-refresh-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        for relative in [
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
        ] {
            let target = root.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), target).unwrap();
        }
        crate::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        (root, repo.join("../skills/work"))
    }

    #[test]
    fn source_impact_is_read_only_and_has_no_publication_or_approval() {
        let (root, skill) = fixture_root("read-only-impact");
        let files = [
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
            "outputs/work/executions/example/index.json",
            "outputs/work/sources/example/SRC-001/manifest.json",
            "outputs/work/sources/example/SRC-001/manifest.json.done",
            "outputs/work/sources/example/SRC-001/source.txt",
        ]
        .map(|path| (path, fs::read(root.join(path)).unwrap()));
        let result = source_impact(&root, &skill).unwrap();
        assert!(result.get("refreshable").is_none());
        assert_eq!(result["requirements"][0]["status"], "stale");
        assert!(result["requirements"][0].get("approved_sha256").is_none());
        assert!(result["requirements"][0].get("after").is_none());
        assert_eq!(source_impact(&root, &skill).unwrap(), result);
        for (path, raw) in files {
            assert_eq!(fs::read(root.join(path)).unwrap(), raw);
        }
        assert!(
            !root
                .join("outputs/work/executions/example/.work-state-writer.lock")
                .exists()
        );
    }
}
