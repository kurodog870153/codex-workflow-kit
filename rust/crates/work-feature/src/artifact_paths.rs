//! Shared artifact locations independent of planning use cases.

use std::path::PathBuf;

use work_model::identifiers::RequirementId;

use crate::error::WorkError;

pub use work_model::task::source::TaskArtifactPaths as ArtifactPaths;

pub fn default_artifact_paths(requirement_id: &RequirementId) -> ArtifactPaths {
    let id = requirement_id.as_str();
    ArtifactPaths {
        source: format!("outputs/work/sources/{id}"),
        task: format!("outputs/work/tasks/{id}/index.json"),
        execution: format!("outputs/work/executions/{id}"),
    }
}

pub trait ArtifactPathRepository {
    fn resolve(&self, relative: &str) -> Result<PathBuf, WorkError>;
    fn exists(&self, relative: &str) -> Result<bool, WorkError>;
    fn read_raw(&self, relative: &str) -> Result<Vec<u8>, WorkError>;
    fn create_new(&self, relative: &str, bytes: &[u8]) -> Result<(), WorkError>;
    fn validate_paths(
        &self,
        requirement_id: &RequirementId,
        paths: &ArtifactPaths,
    ) -> Result<(), WorkError>;

    fn default_paths(&self, requirement_id: &RequirementId) -> Result<ArtifactPaths, WorkError> {
        let paths = default_artifact_paths(requirement_id);
        self.validate_paths(requirement_id, &paths)?;
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_only_the_three_active_artifact_locations() {
        let paths = default_artifact_paths(&"example".parse().unwrap());
        assert_eq!(
            serde_json::to_value(paths).unwrap(),
            serde_json::json!({
                "source": "outputs/work/sources/example",
                "task": "outputs/work/tasks/example/index.json",
                "execution": "outputs/work/executions/example"
            })
        );
    }
}
