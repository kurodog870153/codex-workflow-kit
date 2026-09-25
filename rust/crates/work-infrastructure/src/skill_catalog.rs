//! Skill catalog and snapshot filesystem adapter.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::skill::{SkillCatalogRepository, SkillRoot, SkillSnapshotRepository};

use crate::skill_bundle::snapshot_skill_bundle;

#[derive(Debug, Clone)]
pub struct SkillRootConfig {
    pub scope: String,
    pub locator: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct LocalSkillCatalog {
    pub roots: Vec<SkillRootConfig>,
}

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn root_path(root: &SkillRootConfig) -> Result<PathBuf, WorkError> {
    work_feature::skill::validate_root(&root.scope, &root.locator)?;
    let resolved = root.path.canonicalize().map_err(|_| {
        error(
            ExitCode::IoFailure,
            "skill_catalog_root_resolution_failed",
            "A skill catalog root could not be resolved.",
            json!({"scope": root.scope, "path": root.path}),
        )
    })?;
    if !resolved.is_dir() {
        return Err(error(
            ExitCode::IoFailure,
            "skill_catalog_root_not_directory",
            "A skill catalog root is not a directory.",
            json!({"scope": root.scope, "path": resolved}),
        ));
    }
    Ok(resolved)
}

fn yaml(path: &Path) -> Result<Value, WorkError> {
    let raw = fs::read(path).map_err(|_| {
        error(
            ExitCode::IoFailure,
            "skill_summary_read_failed",
            "The skill summary could not be read.",
            json!({"source": path}),
        )
    })?;
    let raw = raw.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&raw);
    let lines = raw.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    let delimiter = |line: &[u8]| line.strip_suffix(b"\r").unwrap_or(line) == b"---";
    if !lines.first().is_some_and(|line| delimiter(line)) {
        return Err(error(
            ExitCode::InputFormat,
            "skill_frontmatter_missing",
            "SKILL.md must begin with YAML frontmatter.",
            json!({"source": path}),
        ));
    }
    let end = lines
        .iter()
        .skip(1)
        .position(|line| delimiter(line))
        .ok_or_else(|| {
            error(
                ExitCode::InputFormat,
                "skill_frontmatter_unterminated",
                "SKILL.md YAML frontmatter is not terminated.",
                json!({"source": path}),
            )
        })?
        + 1;
    let frontmatter = lines[1..end].join(&b'\n');
    let text = std::str::from_utf8(&frontmatter).map_err(|_| {
        error(
            ExitCode::InputFormat,
            "invalid_skill_frontmatter",
            "SKILL.md frontmatter is not valid UTF-8 YAML.",
            json!({"source": path}),
        )
    })?;
    serde_yaml_ng::from_str::<Value>(text).map_err(|_| {
        error(
            ExitCode::InputFormat,
            "invalid_skill_frontmatter",
            "SKILL.md frontmatter is not valid UTF-8 YAML.",
            json!({"source": path}),
        )
    })
}

fn metadata_file(path: &Path) -> Result<Value, WorkError> {
    let raw = fs::read(path).map_err(|_| {
        error(
            ExitCode::InputFormat,
            "invalid_skill_metadata",
            "The skill metadata file is not valid UTF-8 YAML.",
            json!({"source": path}),
        )
    })?;
    let text = std::str::from_utf8(raw.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&raw)).map_err(
        |_| {
            error(
                ExitCode::InputFormat,
                "invalid_skill_metadata",
                "The skill metadata file is not valid UTF-8 YAML.",
                json!({"source": path}),
            )
        },
    )?;
    serde_yaml_ng::from_str(text).map_err(|_| {
        error(
            ExitCode::InputFormat,
            "invalid_skill_metadata",
            "The skill metadata file is not valid UTF-8 YAML.",
            json!({"source": path}),
        )
    })
}

fn summary(
    skill_root: &Path,
    scope: &str,
    locator: &str,
    source: &str,
) -> Result<Value, WorkError> {
    let source_path = skill_root.join("SKILL.md");
    let frontmatter = yaml(&source_path)?;
    let openai_path = skill_root.join("agents/openai.yaml");
    work_feature::skill::summary_from_metadata(
        &frontmatter,
        || {
            if openai_path.is_file() {
                Ok(Some(metadata_file(&openai_path)?))
            } else {
                Ok(None)
            }
        },
        scope,
        locator,
        source,
        &source_path,
        &openai_path,
    )
}

impl LocalSkillCatalog {
    pub fn catalog(
        &self,
        disabled_sources: &HashSet<String>,
        excluded_names: &HashSet<String>,
    ) -> Result<Value, WorkError> {
        work_feature::skill::catalog(self, disabled_sources, excluded_names)
    }

    pub fn snapshot(&self, scope: &str, locator: &str, source: &str) -> Result<Value, WorkError> {
        <Self as SkillSnapshotRepository>::snapshot(self, scope, locator, source)
    }
}

impl SkillCatalogRepository for LocalSkillCatalog {
    fn roots(&self) -> Vec<SkillRoot> {
        self.roots
            .iter()
            .map(|root| SkillRoot {
                scope: root.scope.clone(),
                locator: root.locator.clone(),
            })
            .collect()
    }

    fn list_directories(&self, root_index: usize) -> Result<Vec<PathBuf>, WorkError> {
        let path = root_path(&self.roots[root_index])?;
        let mut directories: Vec<_> = fs::read_dir(&path)
            .map_err(|_| {
                error(
                    ExitCode::IoFailure,
                    "skill_catalog_root_resolution_failed",
                    "A skill catalog root could not be resolved.",
                    json!({"path": path}),
                )
            })?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        directories.sort_by_key(|path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase()
        });
        Ok(directories)
    }

    fn has_entrypoint(&self, directory: &Path) -> bool {
        directory.join("SKILL.md").is_file()
    }

    fn resolve_directory(&self, directory: &Path) -> Option<PathBuf> {
        directory.canonicalize().ok()
    }

    fn summary(
        &self,
        root_index: usize,
        resolved: &Path,
        source: &str,
    ) -> Result<Value, WorkError> {
        let root = &self.roots[root_index];
        summary(resolved, &root.scope, &root.locator, source)
    }
}

impl SkillSnapshotRepository for LocalSkillCatalog {
    fn snapshot(&self, scope: &str, locator: &str, source: &str) -> Result<Value, WorkError> {
        let root = self
            .roots
            .iter()
            .find(|root| root.scope == scope && root.locator == locator)
            .ok_or_else(|| {
                error(
                    ExitCode::ArtifactIntegrity,
                    "selected_skill_root_missing",
                    "The selected skill root is not available.",
                    json!({"scope": scope, "root": locator}),
                )
            })?;
        let path = root_path(root)?;
        let folder = work_feature::skill::validate_source(source)?;
        let skill_root = path.join(folder).canonicalize().map_err(|_| {
            error(
                ExitCode::ArtifactIntegrity,
                "selected_skill_root_missing",
                "The selected skill root is not available.",
                json!({"source": source}),
            )
        })?;
        let skill = summary(&skill_root, scope, locator, source)?;
        let bundle = snapshot_skill_bundle(&skill_root)?;
        Ok(work_model::skill::verified::<
            work_model::skill::SkillSnapshot,
        >(
            json!({"schema": "work-skill-snapshot/v1", "skill": skill, "bundle": bundle}),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_catalog_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-skill-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_skill(root: &Path, folder: &str, name: &str) -> PathBuf {
        let skill = root.join(folder);
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: Use {name} for tests.\n---\nInstructions\n"),
        )
        .unwrap();
        skill
    }

    #[test]
    fn catalog_preserves_scope_and_locator_identity() {
        let root = temporary_catalog_root();
        let repo = root.join("repo");
        let user = root.join("user");
        fs::create_dir_all(&repo).unwrap();
        fs::create_dir_all(&user).unwrap();
        write_skill(&repo, "repo-ui", "ui");
        let user_skill = write_skill(&user, "user-ui", "ui");
        fs::create_dir_all(user_skill.join("agents")).unwrap();
        fs::write(user_skill.join("agents/openai.yaml"),
            "policy:\n  allow_implicit_invocation: false\ndependencies:\n  tools:\n    - type: mcp\n      value: designSystem\n      description: Design system server\n").unwrap();
        let catalog = LocalSkillCatalog {
            roots: vec![
                SkillRootConfig {
                    scope: "repo".into(),
                    locator: ".agents/skills".into(),
                    path: repo.clone(),
                },
                SkillRootConfig {
                    scope: "user".into(),
                    locator: ".agents/skills".into(),
                    path: user,
                },
            ],
        }
        .catalog(&HashSet::new(), &HashSet::new())
        .unwrap();
        let skills = catalog["skills"].as_array().unwrap();
        assert_eq!(skills.len(), 2);
        assert_ne!(skills[0]["id"], skills[1]["id"]);
        assert_eq!(skills[1]["allow_implicit_invocation"], false);
        assert_eq!(skills[1]["dependencies"][0]["value"], "designSystem");

        let other = LocalSkillCatalog {
            roots: vec![SkillRootConfig {
                scope: "repo".into(),
                locator: "services/api/.agents/skills".into(),
                path: repo,
            }],
        }
        .catalog(&HashSet::new(), &HashSet::new())
        .unwrap();
        assert_ne!(skills[0]["id"], other["skills"][0]["id"]);
        assert_ne!(skills[0]["root"], other["skills"][0]["root"]);
    }

    #[test]
    fn catalog_reads_only_utf8_frontmatter_and_reports_invalid_yaml() {
        let root = temporary_catalog_root();
        let binary = root.join("binary-body");
        fs::create_dir_all(&binary).unwrap();
        fs::write(
            binary.join("SKILL.md"),
            b"---\nname: binary-body\ndescription: Summary only.\n---\n\xff",
        )
        .unwrap();
        let invalid = root.join("invalid");
        fs::create_dir_all(&invalid).unwrap();
        fs::write(
            invalid.join("SKILL.md"),
            b"---\nname: [invalid\ndescription: broken\n---\n",
        )
        .unwrap();
        let catalog = LocalSkillCatalog {
            roots: vec![SkillRootConfig {
                scope: "repo".into(),
                locator: ".agents/skills".into(),
                path: root,
            }],
        }
        .catalog(&HashSet::new(), &HashSet::new())
        .unwrap();
        assert_eq!(catalog["skills"][0]["name"], "binary-body");
        assert_eq!(catalog["skills"].as_array().unwrap().len(), 1);
        assert_eq!(
            catalog["unavailable"][0]["code"],
            "invalid_skill_frontmatter"
        );
    }

    #[test]
    fn current_rust_skill_snapshot_has_expected_contents() {
        let repository = LocalSkillCatalog {
            roots: vec![SkillRootConfig {
                scope: "repo".into(),
                locator: "skills".into(),
                path: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills")),
            }],
        };
        let snapshot = repository
            .snapshot("repo", "skills", "work/SKILL.md")
            .unwrap();
        assert_eq!(
            snapshot["skill"]["id"],
            "30bd7f92493f1ec33907db1a70ac0d86d74f1d5f43ac08ee8de77294da7dc808"
        );
        assert_eq!(
            snapshot["skill"]["summary_sha256"],
            "c8652a87ff59a9f6233a6372cd6b58e3a5f6c17deae0fd2141418d7ca1cdfb94"
        );
        assert_eq!(
            snapshot["bundle"]["bundle_sha256"],
            "09daeee70687d3bf2871a2bbef90e0bff75809cadc4657f3433c9955a853fbfd"
        );
        assert!(
            repository
                .catalog(&HashSet::new(), &HashSet::from(["work".into()]))
                .unwrap()["skills"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
