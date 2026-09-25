//! Instruction entrypoint catalog used by hierarchy selection.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::hierarchy::HierarchyCatalogRepository;
use work_feature::instruction::InstructionCatalogRepository;
use work_operations::hierarchy::{CrossModeCatalog, valid_segment};

#[derive(Debug, Clone)]
pub struct LocalHierarchyCatalog {
    pub skill_root: PathBuf,
}

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

impl LocalHierarchyCatalog {
    fn scan_mode_metadata(&self, mode: &str) -> Result<BTreeMap<String, Value>, WorkError> {
        let skill_root = self.skill_root.canonicalize().map_err(|_| {
            error(
                ExitCode::IoFailure,
                "skill_root_missing",
                "The Work skill root does not exist or is not a directory.",
                json!({"path": self.skill_root}),
            )
        })?;
        if !skill_root.is_dir() {
            return Err(error(
                ExitCode::IoFailure,
                "skill_root_missing",
                "The Work skill root does not exist or is not a directory.",
                json!({"path": self.skill_root}),
            ));
        }
        let mode_dir = self.skill_root.join("references/instructions").join(mode);
        let resolved_mode = mode_dir.canonicalize().map_err(|_| {
            error(
                ExitCode::IoFailure,
                "instruction_root_missing",
                "The instruction mode directory does not exist or is not a directory.",
                json!({"path": mode_dir}),
            )
        })?;
        if !resolved_mode.starts_with(&skill_root) {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "instruction_root_escapes_skill_root",
                "The instruction mode directory resolves outside the Work skill root.",
                json!({"mode": mode, "path": mode_dir}),
            ));
        }
        let mut metadata = BTreeMap::new();
        visit(&mode_dir, &mode_dir, &resolved_mode, mode, &mut metadata)?;
        Ok(metadata)
    }
}

fn visit(
    current: &Path,
    mode_dir: &Path,
    resolved_mode: &Path,
    mode: &str,
    output: &mut BTreeMap<String, Value>,
) -> Result<(), WorkError> {
    let entries = fs::read_dir(current).map_err(|_| {
        error(
            ExitCode::IoFailure,
            "instruction_entrypoint_unreadable",
            "An instruction entrypoint cannot be resolved.",
            json!({"path": current}),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|_| {
            error(
                ExitCode::IoFailure,
                "instruction_entrypoint_unreadable",
                "An instruction entrypoint cannot be resolved.",
                json!({"path": current}),
            )
        })?;
        let path = entry.path();
        if path.is_dir() {
            visit(&path, mode_dir, resolved_mode, mode, output)?;
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) != Some("instructions.md") {
            continue;
        }
        let resolved = path.canonicalize().map_err(|_| {
            error(
                ExitCode::IoFailure,
                "instruction_entrypoint_unreadable",
                "An instruction entrypoint cannot be resolved.",
                json!({"path": path}),
            )
        })?;
        if !resolved.is_file() || !resolved.starts_with(resolved_mode) {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "instruction_path_escapes_skill_root",
                "An instruction entrypoint resolves outside its instruction directory.",
                json!({"mode": mode, "path": path}),
            ));
        }
        let relative = path
            .parent()
            .unwrap()
            .strip_prefix(mode_dir)
            .expect("visited under mode directory");
        let parts: Vec<_> = relative
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect();
        let hierarchy_path = parts.join("/");
        if parts.is_empty() || parts.iter().any(|part| !valid_segment(part)) {
            return Err(error(
                ExitCode::Contract,
                "invalid_instruction_hierarchy_path",
                "An instruction hierarchy path is invalid.",
                json!({"mode": mode, "path": hierarchy_path}),
            ));
        }
        if parts.iter().any(|part| part == "general") && hierarchy_path != "general" {
            return Err(error(
                ExitCode::Contract,
                "invalid_general_instruction_path",
                "The general instruction entrypoint must be a root hierarchy path.",
                json!({"mode": mode, "path": hierarchy_path}),
            ));
        }
        output.insert(hierarchy_path, read_metadata(&path)?);
    }
    Ok(())
}

fn read_metadata(path: &Path) -> Result<Value, WorkError> {
    let raw = fs::read(path).map_err(|_| {
        error(
            ExitCode::IoFailure,
            "file_read_failed",
            "The file could not be read.",
            json!({"path": path}),
        )
    })?;
    let text = std::str::from_utf8(raw.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&raw)).map_err(
        |invalid| {
            error(
                ExitCode::InputFormat,
                "invalid_utf8",
                "The input is not valid UTF-8.",
                json!({"source": path, "byte_offset": invalid.valid_up_to()}),
            )
        },
    )?;
    let lines: Vec<_> = text.lines().collect();
    if lines.first() != Some(&"---") {
        return Err(error(
            ExitCode::InputFormat,
            "instruction_frontmatter_missing",
            "instructions.md must begin with YAML frontmatter.",
            json!({"source": path}),
        ));
    }
    let end = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| **line == "---")
        .map(|(index, _)| index)
        .ok_or_else(|| {
            error(
                ExitCode::InputFormat,
                "instruction_frontmatter_unterminated",
                "instructions.md YAML frontmatter is not terminated.",
                json!({"source": path}),
            )
        })?;
    let loaded: Value = serde_yaml_ng::from_str(&lines[1..end].join("\n")).map_err(|_| {
        error(
            ExitCode::InputFormat,
            "invalid_instruction_frontmatter",
            "instructions.md frontmatter is not valid YAML.",
            json!({"source": path}),
        )
    })?;
    let object = loaded.as_object().ok_or_else(|| {
        error(
            ExitCode::InputFormat,
            "invalid_instruction_frontmatter",
            "instructions.md frontmatter must be a YAML object with string keys.",
            json!({"source": path}),
        )
    })?;
    if object.len() != 3
        || !["name", "description", "metadata"]
            .iter()
            .all(|key| object.contains_key(*key))
    {
        return Err(error(
            ExitCode::InputFormat,
            "invalid_instruction_metadata_fields",
            "Instruction frontmatter has missing or unknown fields.",
            json!({"source": path}),
        ));
    }
    let name = object["name"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            error(
                ExitCode::InputFormat,
                "invalid_instruction_metadata_value",
                "Instruction name and description must be non-empty strings.",
                json!({"source": path, "field": "name"}),
            )
        })?
        .trim();
    let description = object["description"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            error(
                ExitCode::InputFormat,
                "invalid_instruction_metadata_value",
                "Instruction name and description must be non-empty strings.",
                json!({"source": path, "field": "description"}),
            )
        })?
        .trim();
    let metadata = object["metadata"].as_object().ok_or_else(|| {
        error(
            ExitCode::InputFormat,
            "invalid_instruction_metadata",
            "Instruction metadata must be a YAML object with string keys.",
            json!({"source": path}),
        )
    })?;
    if metadata.len() != 1 || !metadata.contains_key("work-tags") {
        return Err(error(
            ExitCode::InputFormat,
            "invalid_instruction_metadata_fields",
            "Instruction metadata has missing or unknown fields.",
            json!({"source": path}),
        ));
    }
    let tags = metadata["work-tags"].as_array().filter(|tags| !tags.is_empty() && tags.iter().all(|tag| tag.as_str().is_some_and(valid_segment))).ok_or_else(|| error(ExitCode::InputFormat, "invalid_instruction_work_tags", "Instruction work-tags must be a non-empty array of unique lowercase kebab-case strings.", json!({"source": path})))?;
    let unique: BTreeSet<_> = tags.iter().map(|tag| tag.as_str().unwrap()).collect();
    if unique.len() != tags.len() {
        return Err(error(
            ExitCode::InputFormat,
            "invalid_instruction_work_tags",
            "Instruction work-tags must be a non-empty array of unique lowercase kebab-case strings.",
            json!({"source": path}),
        ));
    }
    Ok(json!({"name": name, "description": description, "work_tags": tags}))
}

impl LocalHierarchyCatalog {
    pub fn catalog(&self, mode: &str) -> Result<Value, WorkError> {
        work_feature::instruction::catalog(self, mode)
    }
}

impl InstructionCatalogRepository for LocalHierarchyCatalog {
    fn scan_mode_metadata(&self, mode: &str) -> Result<BTreeMap<String, Value>, WorkError> {
        LocalHierarchyCatalog::scan_mode_metadata(self, mode)
    }
}

impl HierarchyCatalogRepository for LocalHierarchyCatalog {
    fn cross_mode_catalog(&self) -> Result<CrossModeCatalog, WorkError> {
        work_feature::instruction::cross_mode_catalog(self)
    }

    fn mode_paths(&self, mode: &str) -> Result<Vec<String>, WorkError> {
        Ok(work_feature::instruction::mode_catalog(self, mode)?.paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use work_feature::hierarchy::{build_selection, validate_selection, validate_task_paths};
    use work_feature::instruction::resolve_hierarchy;

    #[test]
    fn temporary_catalog_orders_paths_and_exposes_metadata_without_body() {
        let root = std::env::temp_dir().join(format!(
            "work-catalog-t25-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |mode: &str, path: &str, name: &str| {
            let entry = root
                .join("references/instructions")
                .join(mode)
                .join(path)
                .join("instructions.md");
            fs::create_dir_all(entry.parent().unwrap()).unwrap();
            fs::write(
                entry,
                format!("---\nname: {name}\ndescription: {name} instructions.\nmetadata:\n  work-tags:\n    - test-tag\n---\n\n# Private instruction body\n"),
            )
            .unwrap();
        };
        for mode in ["plan", "task", "execute"] {
            for path in ["general", "web", "web/frontend"] {
                write(mode, path, path);
            }
        }
        write("plan", "web/frontend/typescript", "typescript");
        write("task", "web/frontend/css", "css");
        write("execute", "web/frontend/css", "css");
        for path in [
            "web/backend/java/mybatis",
            "web/backend/java",
            "web/backend/java/jpa",
            "web/backend",
        ] {
            write("task", path, path);
        }
        let repository = LocalHierarchyCatalog { skill_root: root };
        let task = repository.catalog("task").unwrap();
        assert_eq!(
            task["paths"],
            json!([
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis",
                "web/frontend",
                "web/frontend/css"
            ])
        );
        assert_eq!(task["children"]["general"], json!(["web"]));
        assert_eq!(
            task["children"]["web/backend/java"],
            json!(["jpa", "mybatis"])
        );
        assert_eq!(task["children"]["web/backend/java/jpa"], json!([]));
        assert_eq!(
            task["metadata"]["web/backend/java/jpa"],
            json!({"name":"web/backend/java/jpa","description":"web/backend/java/jpa instructions.","work_tags":["test-tag"]})
        );
        let plan = repository.catalog("plan").unwrap();
        assert_eq!(plan["metadata"]["general"]["name"], "general");
        assert!(!plan.to_string().contains("Private instruction body"));
        assert!(work_operations::protocol::valid_sha256(
            plan["catalog_sha256"].as_str().unwrap()
        ));

        let all = repository.catalog("all").unwrap();
        assert_eq!(all["mode"], "all");
        assert_eq!(
            all["paths"],
            json!([
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis",
                "web/frontend",
                "web/frontend/css",
                "web/frontend/typescript"
            ])
        );
        assert_eq!(
            all["metadata"]["web/frontend/typescript"]["mode_support"],
            json!(["plan"])
        );
        assert_eq!(
            all["metadata"]["web/frontend/css"]["modes"]
                .as_object()
                .unwrap()
                .keys()
                .collect::<Vec<_>>(),
            vec!["execute", "task"]
        );
        assert!(work_operations::protocol::valid_sha256(
            all["catalog_sha256"].as_str().unwrap()
        ));
    }

    #[test]
    fn temporary_catalog_rejects_bad_entries_and_ignores_nonentrypoint_directories() {
        let root = std::env::temp_dir().join(format!(
            "work-catalog-errors-t25-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let entry = |mode: &str, path: &str| {
            root.join("references/instructions")
                .join(mode)
                .join(path)
                .join("instructions.md")
        };
        let write = |mode: &str, path: &str, body: &str| {
            let path = entry(mode, path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        };
        let good = "---\nname: General\ndescription: General instructions.\nmetadata:\n  work-tags:\n    - test-tag\n---\n\nBody.\n";
        let repository = LocalHierarchyCatalog {
            skill_root: root.clone(),
        };
        assert_eq!(
            repository.catalog("build").unwrap_err().reason_code,
            "invalid_instruction_mode"
        );
        write("plan", "general", "# general\n");
        assert_eq!(
            repository.catalog("plan").unwrap_err().reason_code,
            "instruction_frontmatter_missing"
        );
        write(
            "plan",
            "general",
            &good.replace("    - test-tag", "    - duplicate\n    - duplicate"),
        );
        assert_eq!(
            repository.catalog("plan").unwrap_err().reason_code,
            "invalid_instruction_work_tags"
        );
        write(
            "plan",
            "general",
            &good.replace("---\n\nBody.", "  extra: rejected\n---\n\nBody."),
        );
        assert_eq!(
            repository.catalog("plan").unwrap_err().reason_code,
            "invalid_instruction_metadata_fields"
        );
        write("plan", "general", good);
        let notes = root.join("references/instructions/plan/general/references/notes.md");
        fs::create_dir_all(notes.parent().unwrap()).unwrap();
        fs::write(notes, "notes\n").unwrap();
        let plan = repository.catalog("plan").unwrap();
        assert_eq!(plan["paths"], json!(["general"]));
        assert_eq!(plan["children"], json!({"general": []}));

        write("execute", "web", good);
        assert_eq!(
            repository.catalog("execute").unwrap_err().reason_code,
            "general_instruction_not_unique"
        );
        write("task", "general", good);
        write("task", "web/backend", good);
        let ancestor = repository.catalog("task").unwrap_err();
        assert_eq!(ancestor.reason_code, "instruction_ancestor_missing");
        assert_eq!(ancestor.details["missing_ancestor"], "web");
    }

    #[cfg(unix)]
    #[test]
    fn catalog_entrypoint_symlink_cannot_escape_skill_root() {
        use std::os::unix::fs::symlink;

        let parent = std::env::temp_dir().join(format!(
            "work-catalog-symlink-t25-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = parent.join("work");
        let general = root.join("references/instructions/plan/general/instructions.md");
        fs::create_dir_all(general.parent().unwrap()).unwrap();
        fs::write(&general, "---\nname: General\ndescription: General instructions.\nmetadata:\n  work-tags:\n    - test-tag\n---\n").unwrap();
        let outside = parent.join("outside.md");
        fs::write(&outside, "outside\n").unwrap();
        let escaped = root.join("references/instructions/plan/escaped/instructions.md");
        fs::create_dir_all(escaped.parent().unwrap()).unwrap();
        symlink(outside, escaped).unwrap();
        assert_eq!(
            (LocalHierarchyCatalog { skill_root: root })
                .catalog("plan")
                .unwrap_err()
                .reason_code,
            "instruction_path_escapes_skill_root"
        );
    }

    #[test]
    fn hierarchy_resolution_uses_catalog_paths_and_reports_immediate_choices() {
        let root = std::env::temp_dir().join(format!(
            "work-hierarchy-catalog-t25-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |mode: &str, hierarchy: &str| {
            let path = root
                .join("references/instructions")
                .join(mode)
                .join(hierarchy)
                .join("instructions.md");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "---\nname: Test\ndescription: Test instructions.\nmetadata:\n  work-tags:\n    - test-tag\n---\n\nBody.\n").unwrap();
        };
        for mode in ["plan", "task", "execute"] {
            for path in ["general", "web", "web/frontend", "web/frontend/typescript"] {
                write(mode, path);
            }
        }
        for mode in ["task", "execute"] {
            for path in [
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis",
                "web/frontend/typescript/astro",
            ] {
                write(mode, path);
            }
        }
        let repository = LocalHierarchyCatalog { skill_root: root };
        let selected = resolve_hierarchy(
            &repository,
            "task",
            &[
                "web/backend/java/jpa".into(),
                "web/backend/java/mybatis".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            selected.selected_paths,
            ["web/backend/java/jpa", "web/backend/java/mybatis"]
        );
        assert_eq!(
            selected.resolved_paths,
            [
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis"
            ]
        );
        let general = resolve_hierarchy(&repository, "plan", &[]).unwrap();
        assert!(general.selected_paths.is_empty());
        assert_eq!(general.resolved_paths, ["general"]);
        assert_eq!(general.required_paths, ["general"]);

        let projected = resolve_hierarchy(
            &repository,
            "plan",
            &["web/frontend/typescript/astro".into()],
        )
        .unwrap();
        assert_eq!(projected.selected_paths, ["web/frontend/typescript/astro"]);
        assert_eq!(
            projected.resolved_paths,
            ["general", "web", "web/frontend", "web/frontend/typescript"]
        );
        assert_eq!(
            resolve_hierarchy(&repository, "plan", &["general".into()])
                .unwrap_err()
                .reason_code,
            "invalid_hierarchy_path"
        );
        assert_eq!(
            resolve_hierarchy(&repository, "task", &["web".into(), "web/backend".into()])
                .unwrap_err()
                .reason_code,
            "redundant_hierarchy_path"
        );
        let missing = resolve_hierarchy(
            &repository,
            "execute",
            &["web/backend/java/hibernate".into()],
        )
        .unwrap_err();
        assert_eq!(missing.reason_code, "instruction_hierarchy_path_missing");
        assert_eq!(
            missing.details,
            json!({"mode":"execute","path":"web/backend/java/hibernate","parent":"web/backend/java","valid_choices":["jpa","mybatis"]})
        );
    }

    #[test]
    fn installed_catalog_matches_python_baseline() {
        let repository = LocalHierarchyCatalog {
            skill_root: PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../crates/work-infrastructure/legacy-work-skill"
            )),
        };
        let catalog = repository.cross_mode_catalog().unwrap();
        assert_eq!(
            repository.catalog("task").unwrap()["catalog_sha256"],
            "b53f28ea422329c5109a8da78a61406758e1f67383e817a8a654c429407b570e"
        );
        assert_eq!(
            repository.catalog("all").unwrap()["catalog_sha256"],
            catalog.catalog_sha256
        );
        assert_eq!(
            catalog.catalog_sha256,
            "21bcff3a175d3081a458e9c2e81990c1294ed50fc3217f88be67aa3bef1eeabb"
        );
        let general = build_selection(
            &repository,
            &json!({"decision": "general_only", "selections": []}),
        )
        .unwrap();
        assert_eq!(
            general["selection_sha256"],
            "973f1e1fba10d2e49181e6f55849e8465a20dba6b4ed323547dab13b66d4bf6e"
        );
        let selected = build_selection(&repository, &json!({"decision": "instruction_paths", "selections": [{"path": "web/backend/java", "recommendation_reason": "Uses Java."}]})).unwrap();
        assert_eq!(
            selected["selection_sha256"],
            "999122379153a67a816857ac21b8f7e0a7acc134417d2085a81db5f950f01b54"
        );
        assert_eq!(
            validate_selection(&repository, &selected).unwrap()["status"],
            "valid"
        );
        validate_task_paths(
            &repository,
            &["web/backend/java/jpa".into()],
            &selected,
            "TASK-001",
        )
        .unwrap();
        let frontend = build_selection(
            &repository,
            &json!({"decision":"instruction_paths","selections":[
                {"path":"web/frontend/typescript/astro","recommendation_reason":"Uses Astro."},
                {"path":"web/frontend/css/tailwind","recommendation_reason":"Uses Tailwind."}]}),
        )
        .unwrap();
        assert_eq!(
            frontend["selected_paths"],
            json!(["web/frontend/typescript/astro", "web/frontend/css/tailwind"])
        );
        for entry in frontend["entries"].as_array().unwrap() {
            assert_eq!(entry["mode_support"], json!(["task", "execute"]));
        }
    }
}
