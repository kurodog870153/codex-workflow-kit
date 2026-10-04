//! Artifact path resolution command flow.

use std::path::Path;

use serde_json::{Value, json};
use work_feature::artifact_paths::ArtifactPathRepository;
use work_feature::error::{ExitCode, WorkError};
use work_model::identifiers::RequirementId;

pub fn resolve(
    raw_id: &str,
    project_root: &Path,
    repository: &impl ArtifactPathRepository,
) -> Result<Value, WorkError> {
    let id: RequirementId = raw_id.parse().map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_requirement_id",
            "The requirement ID must use lowercase letters, digits, dots, underscores, or hyphens.",
            json!({"requirement_id":raw_id}),
        )
    })?;
    let paths = repository.default_paths(&id)?;
    Ok(json!({"schema":"work-paths/v1","project_root":project_root,
        "requirement_id":id.as_str(),"paths":paths}))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use work_feature::artifact_paths::{ArtifactPathRepository, ArtifactPaths};
    use work_feature::error::{ExitCode, WorkError};
    use work_model::identifiers::RequirementId;

    use super::resolve;

    #[derive(Default)]
    struct Paths(RefCell<Vec<String>>);

    impl ArtifactPathRepository for Paths {
        fn resolve(&self, relative: &str) -> Result<PathBuf, WorkError> {
            self.0.borrow_mut().push(relative.into());
            Ok(Path::new("/project").join(relative))
        }
        fn exists(&self, _relative: &str) -> Result<bool, WorkError> {
            Ok(false)
        }
        fn read_raw(&self, _relative: &str) -> Result<Vec<u8>, WorkError> {
            Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "file_not_found",
                "No files in fake repository.",
                json!({}),
            ))
        }
        fn create_new(&self, _relative: &str, _bytes: &[u8]) -> Result<(), WorkError> {
            Err(WorkError::new(
                ExitCode::IoFailure,
                "read_only_repository",
                "Path resolution must not write.",
                json!({}),
            ))
        }
        fn validate_paths(
            &self,
            _id: &RequirementId,
            paths: &ArtifactPaths,
        ) -> Result<(), WorkError> {
            for path in [&paths.source, &paths.task, &paths.execution] {
                self.resolve(path)?;
            }
            Ok(())
        }
    }

    #[test]
    fn flow_checks_each_artifact_path_in_order() {
        let repository = Paths::default();
        let result = resolve("example", Path::new("/project"), &repository).unwrap();
        let observed = repository.0.borrow();
        assert_eq!(
            observed.as_slice(),
            [
                "outputs/work/sources/example",
                "outputs/work/tasks/example/index.json",
                "outputs/work/executions/example",
            ]
        );
        assert_eq!(result["paths"]["task"], json!(observed[1]));
        assert!(result["paths"].get("plan").is_none());
    }
}
