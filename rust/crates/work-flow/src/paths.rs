//! Artifact path resolution command flow.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_model::identifiers::RequirementId;

pub fn resolve(
    raw_id: &str,
    project_root: &Path,
    mut validate_path: impl FnMut(&str) -> Result<(), WorkError>,
) -> Result<Value, WorkError> {
    let id: RequirementId = raw_id.parse().map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_requirement_id",
            "The requirement ID must use lowercase letters, digits, dots, underscores, or hyphens.",
            json!({"requirement_id":raw_id}),
        )
    })?;
    let mut paths = BTreeMap::new();
    for (field, relative) in work_feature::plan::default_artifact_paths(&id) {
        validate_path(&relative)?;
        paths.insert(field, relative);
    }
    Ok(json!({"schema":"work-paths/v1","project_root":project_root,
        "requirement_id":id.as_str(),"paths":paths}))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::resolve;

    #[test]
    fn flow_checks_each_artifact_path_in_order() {
        let mut observed = Vec::new();
        let result = resolve("example", Path::new("/project"), |relative| {
            observed.push(relative.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            observed,
            [
                "outputs/work/plans/example.json",
                "outputs/work/tasks/example/index.json",
                "outputs/work/executions/example",
            ]
        );
        assert_eq!(result["paths"]["task"], json!(observed[1]));
    }
}
