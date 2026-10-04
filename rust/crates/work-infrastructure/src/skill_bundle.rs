//! Filesystem skill bundle snapshots.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_operations::derivation::fingerprint;
use work_operations::skill::BundleEntry;

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn ignored(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str() == "__pycache__")
        || path.file_name().is_some_and(|name| name == ".DS_Store")
        || path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pyc"))
}

fn collect(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), WorkError> {
    let entries = fs::read_dir(directory).map_err(|_| {
        error(
            ExitCode::IoFailure,
            "skill_bundle_path_resolution_failed",
            "A skill bundle path could not be resolved.",
            json!({"path": directory}),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|_| {
            error(
                ExitCode::IoFailure,
                "skill_bundle_path_resolution_failed",
                "A skill bundle path could not be resolved.",
                json!({"path": directory}),
            )
        })?;
        let path = entry.path();
        if ignored(&path) {
            continue;
        }
        if path.is_file() {
            output.push(path);
        } else if path.is_dir() {
            collect(&path, output)?;
        }
    }
    Ok(())
}

pub fn snapshot_skill_bundle(skill_root: &Path) -> Result<Value, WorkError> {
    let root = skill_root.canonicalize().map_err(|_| {
        error(
            ExitCode::IoFailure,
            "skill_root_resolution_failed",
            "The skill root could not be resolved.",
            json!({"path": skill_root}),
        )
    })?;
    if !root.is_dir() {
        return Err(error(
            ExitCode::IoFailure,
            "skill_root_not_directory",
            "The skill root is not a directory.",
            json!({"path": root}),
        ));
    }
    let mut paths = vec![root.join("SKILL.md")];
    for name in ["agents", "assets", "references", "scripts"] {
        let directory = root.join(name);
        if directory.is_dir() {
            collect(&directory, &mut paths)?;
        }
    }
    paths.retain(|path| !ignored(path));
    paths.sort_by_key(|path| {
        path.strip_prefix(&root)
            .expect("collected beneath root")
            .to_string_lossy()
            .to_lowercase()
    });
    let mut files = Vec::new();
    for path in paths {
        let relative = path
            .strip_prefix(&root)
            .expect("collected beneath root")
            .to_string_lossy()
            .replace('\\', "/");
        let resolved = path.canonicalize().map_err(|_| {
            error(
                ExitCode::IoFailure,
                "skill_bundle_path_resolution_failed",
                "A skill bundle path could not be resolved.",
                json!({"path": relative}),
            )
        })?;
        if !resolved.starts_with(&root) {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "skill_bundle_path_escapes_root",
                "A skill bundle path resolves outside the skill root.",
                json!({"path": relative, "resolved_path": resolved}),
            ));
        }
        let raw = fs::read(&resolved).map_err(|_| {
            error(
                ExitCode::IoFailure,
                "file_read_failed",
                "The file could not be read.",
                json!({"path": resolved}),
            )
        })?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_text = [
            "bat", "command", "json", "md", "ps1", "py", "sh", "toml", "txt", "yaml", "yml",
        ]
        .contains(&extension.as_str())
            || relative == "SKILL.md";
        let normalization = if is_text { "canonical-text" } else { "raw" };
        let hash = if is_text {
            fingerprint::canonical(&raw).map_err(|invalid| {
                error(
                    ExitCode::InputFormat,
                    "invalid_utf8",
                    "The input is not valid UTF-8.",
                    json!({"source": resolved, "byte_offset": invalid.valid_up_to()}),
                )
            })?
        } else {
            fingerprint::raw(&raw)
        };
        files.push(
            json!({"path": relative, "normalization": normalization, "content_sha256": hash}),
        );
    }
    let entries: Vec<_> = files
        .iter()
        .map(|entry| BundleEntry {
            path: entry["path"].as_str().unwrap(),
            normalization: entry["normalization"].as_str().unwrap(),
            content_sha256: entry["content_sha256"].as_str().unwrap(),
        })
        .collect();
    Ok(
        work_model::skill::verified::<work_model::skill::SkillBundle>(
            json!({"schema": "work-skill-bundle", "files": files, "bundle_sha256": fingerprint::skill_bundle(&entries)}),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_content_changes_bundle_fingerprint() {
        let root = std::env::temp_dir().join(format!(
            "work-skill-bundle-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("references")).unwrap();
        fs::write(
            root.join("SKILL.md"),
            b"---\nname: frontend\ndescription: Frontend skill.\n---\n",
        )
        .unwrap();
        let reference = root.join("references/guide.md");
        fs::write(&reference, b"Version one\n").unwrap();
        let first = snapshot_skill_bundle(&root).unwrap();
        assert_eq!(
            first["bundle_sha256"],
            "e30ac49a17fcd6c9974890c9cac67eee80b5b200e3a6db4caaed5d1d4982388d"
        );
        fs::write(&reference, b"Version two\n").unwrap();
        let second = snapshot_skill_bundle(&root).unwrap();
        assert_ne!(first["bundle_sha256"], second["bundle_sha256"]);
    }

    #[test]
    fn current_rust_skill_bundle_has_expected_contents() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        let bundle = snapshot_skill_bundle(root).unwrap();
        assert_eq!(bundle["schema"], "work-skill-bundle");
        let files = bundle["files"].as_array().unwrap();
        assert!(files.iter().any(|file| file["path"] == "SKILL.md"));
        assert!(
            files
                .iter()
                .any(|file| file["path"] == "references/instruction-loading/invocation.md")
        );
        assert!(work_operations::protocol::valid_sha256(
            bundle["bundle_sha256"].as_str().unwrap()
        ));
    }
}
