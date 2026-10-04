//! Discovery of formal Task collections for source-impact and batch refresh previews.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_operations::canonical::parse_json_contract;

pub fn discover_requirements(root: &Path) -> Result<BTreeMap<String, Value>, WorkError> {
    let start = root.join("outputs/work");
    if !start.is_dir() {
        return Ok(BTreeMap::new());
    }
    let mut pending = vec![start];
    let mut paths = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "source_impact_directory_read_failed",
                "The Work output directory cannot be scanned.",
                json!({"path":directory}),
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "source_impact_directory_read_failed",
                    "The Work output directory cannot be scanned.",
                    json!({"path":directory}),
                )
            })?;
            let kind = entry.file_type().map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "source_impact_directory_read_failed",
                    "A Work output entry cannot be inspected.",
                    json!({"path":entry.path()}),
                )
            })?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json")
            {
                paths.push(entry.path());
            }
        }
    }
    paths.sort();
    let mut discovered = BTreeMap::new();
    for path in paths {
        let Ok(raw) = fs::read(&path) else {
            continue;
        };
        let Ok(value) = parse_json_contract(&raw) else {
            continue;
        };
        if value["schema"] != "work-task-index/v1" {
            continue;
        }
        let Some(requirement_id) = value["requirement_id"].as_str() else {
            continue;
        };
        if ["source", "task", "execution"]
            .iter()
            .any(|field| !value["artifacts"][field].is_string())
        {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if value["artifacts"]["task"] != relative {
            continue;
        }
        let artifacts = json!({"source":value["artifacts"]["source"],
            "task":value["artifacts"]["task"],
            "execution":value["artifacts"]["execution"]});
        if discovered
            .insert(requirement_id.to_owned(), artifacts)
            .is_some()
        {
            return Err(WorkError::new(
                ExitCode::ArtifactIntegrity,
                "source_impact_duplicate_requirement",
                "Multiple Task collections declare the same requirement ID.",
                json!({"requirement_id":requirement_id}),
            ));
        }
    }
    Ok(discovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-instruction-discovery-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn discovers_non_default_task_path_and_rejects_duplicate_requirement() {
        let root = root();
        let path = root.join("outputs/work/custom/tasks/index.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{\"schema\":\"work-task-index/v1\",\"requirement_id\":\"custom\",\"artifacts\":{\"source\":\"outputs/work/custom/sources\",\"task\":\"outputs/work/custom/tasks/index.json\",\"execution\":\"outputs/work/custom/execution\"}}\n").unwrap();
        let found = discover_requirements(&root).unwrap();
        assert_eq!(
            found["custom"]["task"],
            "outputs/work/custom/tasks/index.json"
        );
        let duplicate = root.join("outputs/work/other/index.json");
        fs::create_dir_all(duplicate.parent().unwrap()).unwrap();
        fs::write(&duplicate, b"{\"schema\":\"work-task-index/v1\",\"requirement_id\":\"custom\",\"artifacts\":{\"source\":\"outputs/work/other/sources\",\"task\":\"outputs/work/other/index.json\",\"execution\":\"execution\"}}\n").unwrap();
        assert_eq!(
            discover_requirements(&root).unwrap_err().reason_code,
            "source_impact_duplicate_requirement"
        );
    }
}
