//! Filesystem instruction source loader.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use work_feature::error::{ExitCode, WorkError};
use work_feature::instruction::InstructionSourceRepository;
use work_operations::canonical::canonical_bytes;
use work_operations::hierarchy::Hierarchy;
use work_operations::instruction::{LoadedSource, SourceSet, from_sources, source_summary};

use crate::hierarchy_catalog::LocalHierarchyCatalog;

fn error(code: ExitCode, reason: &str, message: &str, details: serde_json::Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn load_file(
    root: &Path,
    path: PathBuf,
    kind: &str,
    logical_name: &str,
    boundary: Option<&Path>,
) -> Result<LoadedSource, WorkError> {
    let resolved = path.canonicalize().map_err(|_| {
        error(
            ExitCode::ArtifactIntegrity,
            "instruction_source_missing",
            "A required instruction source does not exist.",
            json!({"logical_name": logical_name, "path": path}),
        )
    })?;
    if !resolved.is_file() {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "instruction_source_not_file",
            "A required instruction source is not a regular file.",
            json!({"logical_name": logical_name, "path": path}),
        ));
    }
    if !resolved.starts_with(root) {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "instruction_source_escapes_skill_root",
            "An instruction source resolves outside the Work skill root.",
            json!({"logical_name": logical_name, "declared_path": path, "resolved_path": resolved}),
        ));
    }
    if boundary.is_some_and(|boundary| !resolved.starts_with(boundary)) {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "instruction_reference_escapes_hierarchy",
            "An instruction reference resolves outside its declaring hierarchy.",
            json!({"logical_name": logical_name, "declared_path": path, "resolved_path": resolved}),
        ));
    }
    let raw = fs::read(&path).map_err(|_| {
        error(
            ExitCode::IoFailure,
            "file_read_failed",
            "The file could not be read.",
            json!({"path": path}),
        )
    })?;
    let canonical_content = canonical_bytes(&raw).map_err(|invalid| {
        error(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source": path, "byte_offset": invalid.valid_up_to()}),
        )
    })?;
    let summary = source_summary(kind, logical_name, &canonical_content).map_err(|reason| {
        error(
            ExitCode::InputFormat,
            reason,
            "An instruction source may declare at most one compatibility revision.",
            json!({}),
        )
    })?;
    Ok(LoadedSource {
        summary,
        canonical_content,
    })
}

fn valid_reference_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn route(
    mode: &str,
    hierarchy: &Hierarchy,
    references: &[String],
) -> Result<BTreeMap<String, Vec<(String, String)>>, WorkError> {
    let mut seen = HashSet::new();
    let mut routes: BTreeMap<_, Vec<_>> = hierarchy
        .resolved_paths
        .iter()
        .map(|path| (path.clone(), Vec::new()))
        .collect();
    let mut candidates = hierarchy.resolved_paths.clone();
    candidates.sort_by_key(|path| std::cmp::Reverse(path.split('/').count()));
    for name in references {
        if name.is_empty() {
            return Err(error(
                ExitCode::Contract,
                "invalid_instruction_reference",
                "An instruction reference logical name must be a non-empty string.",
                json!({}),
            ));
        }
        if !seen.insert(name) {
            return Err(error(
                ExitCode::Contract,
                "duplicate_instruction_reference",
                "Instruction reference logical names must be unique.",
                json!({}),
            ));
        }
        let Some((path, suffix)) = candidates.iter().find_map(|path| {
            let prefix = format!("{mode}.{}.", path.replace('/', "."));
            name.strip_prefix(&prefix).map(|suffix| (path, suffix))
        }) else {
            return Err(error(
                ExitCode::Contract,
                "unroutable_instruction_reference",
                "The instruction reference does not belong to a selected hierarchy.",
                json!({"logical_name": name}),
            ));
        };
        if !valid_reference_name(suffix) {
            return Err(error(
                ExitCode::Contract,
                "invalid_instruction_reference",
                "The instruction reference name must be lowercase kebab-case.",
                json!({"logical_name": name}),
            ));
        }
        routes
            .get_mut(path)
            .expect("candidate is a route")
            .push((name.clone(), suffix.into()));
    }
    Ok(routes)
}

impl InstructionSourceRepository for LocalHierarchyCatalog {
    fn load_sources(
        &self,
        mode: &str,
        hierarchy: &Hierarchy,
        references: &[String],
    ) -> Result<SourceSet, WorkError> {
        let routes = route(mode, hierarchy, references)?;
        let root = self.skill_root.canonicalize().map_err(|_| {
            error(
                ExitCode::IoFailure,
                "skill_root_missing",
                "The Work skill root does not exist.",
                json!({"path": self.skill_root}),
            )
        })?;
        let mut sources = vec![
            load_file(
                &root,
                self.skill_root.join("references/instruction-loading.md"),
                "workflow",
                "work.instruction-loading",
                None,
            )?,
            load_file(
                &root,
                self.skill_root
                    .join("references/workflows")
                    .join(format!("{mode}.md")),
                "workflow",
                &format!("work.workflow.{mode}"),
                None,
            )?,
        ];
        for path in &hierarchy.resolved_paths {
            let directory = self
                .skill_root
                .join("references/instructions")
                .join(mode)
                .join(path);
            sources.push(load_file(
                &root,
                directory.join("instructions.md"),
                "instruction",
                &format!("{mode}.{}", path.replace('/', ".")),
                None,
            )?);
            if let Some(references) = routes.get(path) {
                let boundary = directory.canonicalize().map_err(|_| {
                    error(
                        ExitCode::ArtifactIntegrity,
                        "instruction_source_missing",
                        "A required instruction source does not exist.",
                        json!({"path": directory}),
                    )
                })?;
                for (logical_name, name) in references {
                    sources.push(load_file(
                        &root,
                        directory.join("references").join(format!("{name}.md")),
                        "reference",
                        logical_name,
                        Some(&boundary),
                    )?);
                }
            }
        }
        Ok(from_sources(mode, hierarchy.clone(), sources))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use work_feature::instruction::{
        load, parse_selection, select, task_document_selection, validate_selection,
        validate_task_document_selection, validate_work_selection_value,
    };
    use work_operations::instruction::selection;

    #[test]
    fn work_instruction_selection_validates_topology_and_fingerprint() {
        let repository = LocalHierarchyCatalog {
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
        };
        let selected = select(&repository, "plan", &[], &[]).unwrap();
        let value = serde_json::to_value(&selected).unwrap();
        assert_eq!(value["selected_paths"], json!([]));
        assert_eq!(value["resolved_paths"], json!(["general"]));
        assert_eq!(
            validate_work_selection_value(
                &repository,
                "plan",
                &value,
                &[],
                "instruction_selection"
            )
            .unwrap()
            .instructions_sha256,
            selected.instructions_sha256
        );
        let mut wrong_paths = value.clone();
        wrong_paths["selected_paths"] = json!(["web"]);
        assert_eq!(
            validate_work_selection_value(
                &repository,
                "plan",
                &wrong_paths,
                &[],
                "instruction_selection"
            )
            .unwrap_err()
            .reason_code,
            "work_instruction_selection_selected_paths_mismatch"
        );
        let mut stale = value;
        stale["instructions_sha256"] = json!("0".repeat(64));
        assert_eq!(
            validate_work_selection_value(
                &repository,
                "plan",
                &stale,
                &[],
                "instruction_selection"
            )
            .unwrap_err()
            .reason_code,
            "work_instructions_fingerprint_mismatch"
        );
    }

    #[test]
    fn installed_sources_match_python_fingerprint_and_order() {
        let repository = LocalHierarchyCatalog {
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
        };
        let loaded = load(&repository, "task", &["web/backend/java".into()], &[]).unwrap();
        assert_eq!(
            loaded.instructions_sha256,
            "479ed501c01988c95d582dc8f64b06ff93e7e93b0391b27f674b9fdc6b52952c"
        );
        assert_eq!(
            loaded
                .sources
                .iter()
                .map(|source| source.summary.logical_name.as_str())
                .collect::<Vec<_>>(),
            [
                "work.instruction-loading",
                "work.workflow.task",
                "task.general",
                "task.web",
                "task.web.backend",
                "task.web.backend.java"
            ]
        );
        assert_eq!(loaded.sources[0].summary.compatibility_revision, 2);
        validate_selection(&repository, "task", &selection(&loaded)).unwrap();
        let mut stale = selection(&loaded);
        stale.sources[0].canonical_sha256 = "0".repeat(64);
        assert_eq!(
            validate_selection(&repository, "task", &stale)
                .unwrap_err()
                .reason_code,
            "instruction_selection_sources_mismatch"
        );
        let parsed = parse_selection(
            &serde_json::to_value(selection(&loaded)).unwrap(),
            "instruction_selection",
        )
        .unwrap();
        assert_eq!(parsed, selection(&loaded));
        let union = task_document_selection(&[loaded.clone(), loaded.clone()]).unwrap();
        assert_eq!(union["instructions_sha256"], loaded.instructions_sha256);
        assert_eq!(
            union["sources"].as_array().unwrap().len(),
            loaded.sources.len()
        );
        assert_eq!(
            load(
                &repository,
                "task",
                &["web/backend/java".into()],
                &["task.web.backend.java.swagger".into()]
            )
            .unwrap()
            .references,
            ["task.web.backend.java.swagger"]
        );
        assert_eq!(
            load(&repository, "task", &[], &["task.unknown.reference".into()])
                .unwrap_err()
                .reason_code,
            "unroutable_instruction_reference"
        );
    }

    #[test]
    fn instruction_selection_round_trips_and_rejects_legacy_or_stale_values() {
        let repository = LocalHierarchyCatalog {
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
        };
        let selected = select(
            &repository,
            "task",
            &["web/backend/java".into()],
            &[
                "task.web.backend.java.swagger".into(),
                "task.general.task-records".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            selected.references,
            ["task.general.task-records", "task.web.backend.java.swagger"]
        );
        assert_eq!(selected.selected_paths, ["web/backend/java"]);
        assert_eq!(
            selected.resolved_paths,
            ["general", "web", "web/backend", "web/backend/java"]
        );
        assert_eq!(
            validate_selection(&repository, "task", &selected)
                .unwrap()
                .instructions_sha256,
            selected.instructions_sha256
        );
        let mut legacy = serde_json::to_value(&selected).unwrap();
        legacy["sources"][0]["layer"] = json!("project");
        let error = parse_selection(&legacy, "instruction_selection").unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["unknown"], json!(["layer"]));
        let mut changed = selected.clone();
        changed.references.reverse();
        assert_eq!(
            validate_selection(&repository, "task", &changed)
                .unwrap_err()
                .reason_code,
            "instruction_selection_references_mismatch"
        );
        let mut changed = selected.clone();
        changed.sources.swap(0, 1);
        assert_eq!(
            validate_selection(&repository, "task", &changed)
                .unwrap_err()
                .reason_code,
            "instruction_selection_sources_mismatch"
        );
        let mut changed = selected;
        changed.instructions_sha256 = "0".repeat(64);
        assert_eq!(
            validate_selection(&repository, "task", &changed)
                .unwrap_err()
                .reason_code,
            "instructions_fingerprint_mismatch"
        );
        let leaves = select(
            &repository,
            "task",
            &[
                "web/backend/java/jpa".into(),
                "web/backend/java/mybatis".into(),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(
            leaves.selected_paths,
            ["web/backend/java/jpa", "web/backend/java/mybatis"]
        );
        assert_eq!(
            leaves.resolved_paths,
            [
                "general",
                "web",
                "web/backend",
                "web/backend/java",
                "web/backend/java/jpa",
                "web/backend/java/mybatis"
            ]
        );
        validate_selection(&repository, "task", &leaves).unwrap();
    }

    #[test]
    fn temporary_sources_preserve_order_and_reject_missing_or_invalid_content() {
        let root = std::env::temp_dir().join(format!(
            "work-instruction-sources-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let write = |relative: &str, content: &[u8]| {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        };
        let metadata = b"---\nname: Test\ndescription: Test instructions.\nmetadata:\n  work-tags:\n    - test-tag\n---\n\nBody.\n";
        write(
            "references/instruction-loading.md",
            b"\xef\xbb\xbfloading\r\n\r\n",
        );
        write("references/workflows/task.md", b"workflow\n");
        for path in ["general", "web", "web/backend"] {
            write(
                &format!("references/instructions/task/{path}/instructions.md"),
                metadata,
            );
        }
        for mode in ["plan", "execute"] {
            write(
                &format!("references/instructions/{mode}/general/instructions.md"),
                metadata,
            );
        }
        for (name, content) in [
            (
                "references/instructions/task/general/references/task-records.md",
                b"records\n".as_slice(),
            ),
            (
                "references/instructions/task/web/backend/references/resilience.md",
                b"resilience\n".as_slice(),
            ),
            (
                "references/instructions/task/web/backend/references/security.md",
                b"security\n".as_slice(),
            ),
        ] {
            write(name, content);
        }
        let repository = LocalHierarchyCatalog {
            skill_root: root.clone(),
        };
        let references = [
            "task.web.backend.resilience".into(),
            "task.general.task-records".into(),
            "task.web.backend.security".into(),
        ];
        let loaded = load(&repository, "task", &["web/backend".into()], &references).unwrap();
        let catalog_selection = work_feature::hierarchy::build_selection(
            &repository,
            &json!({"decision":"general_only","selections":[]}),
        )
        .unwrap();
        assert_eq!(loaded.sources[0].canonical_content, b"loading\n");
        assert_eq!(loaded.sources[0].summary.compatibility_revision, 1);
        assert_eq!(loaded.instructions_sha256.len(), 64);
        assert_eq!(
            loaded
                .sources
                .iter()
                .map(|source| source.summary.logical_name.as_str())
                .collect::<Vec<_>>(),
            [
                "work.instruction-loading",
                "work.workflow.task",
                "task.general",
                "task.general.task-records",
                "task.web",
                "task.web.backend",
                "task.web.backend.resilience",
                "task.web.backend.security"
            ]
        );
        assert_eq!(
            loaded.references,
            [
                "task.general.task-records",
                "task.web.backend.resilience",
                "task.web.backend.security"
            ]
        );
        assert_eq!(
            serde_json::to_value(&loaded.sources[0].summary)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            4
        );
        assert_eq!(
            load(
                &repository,
                "task",
                &[],
                &[
                    "task.general.task-records".into(),
                    "task.general.task-records".into()
                ]
            )
            .unwrap_err()
            .reason_code,
            "duplicate_instruction_reference"
        );
        assert_eq!(
            load(
                &repository,
                "task",
                &[],
                &["task.web.backend.security".into()]
            )
            .unwrap_err()
            .reason_code,
            "unroutable_instruction_reference"
        );
        let missing = load(&repository, "task", &[], &["task.general.missing".into()]).unwrap_err();
        assert_eq!(missing.reason_code, "instruction_source_missing");
        assert_eq!(missing.details["logical_name"], "task.general.missing");
        write(
            "references/instructions/task/general/references/invalid.md",
            b"\xff",
        );
        assert_eq!(
            load(&repository, "task", &[], &["task.general.invalid".into()])
                .unwrap_err()
                .reason_code,
            "invalid_utf8"
        );
        write("references/workflows/task.md", b"changed workflow\n");
        let changed = load(&repository, "task", &["web/backend".into()], &references).unwrap();
        assert_ne!(loaded.instructions_sha256, changed.instructions_sha256);
        assert_eq!(
            validate_selection(&repository, "task", &selection(&loaded))
                .unwrap_err()
                .reason_code,
            "instruction_selection_sources_mismatch"
        );
        write(
            "references/instructions/task/general/instructions.md",
            b"---\nname: Test\ndescription: Changed tasks.\nmetadata:\n  work-tags:\n    - test-tag\n---\n\nBody.\n",
        );
        assert_eq!(
            work_feature::hierarchy::validate_selection(&repository, &catalog_selection)
                .unwrap_err()
                .reason_code,
            "instruction_catalog_snapshot_mismatch"
        );

        write(
            "references/instructions/plan/general/instructions.md",
            metadata,
        );
        let missing = load(&repository, "plan", &[], &[]).unwrap_err();
        assert_eq!(missing.reason_code, "instruction_source_missing");
        assert_eq!(missing.details["logical_name"], "work.workflow.plan");
        write(
            "references/workflows/plan.md",
            b"<!-- work-compatibility-revision: 3 -->\nworkflow\n",
        );
        let plan = load(&repository, "plan", &[], &[]).unwrap();
        assert_eq!(plan.sources[1].summary.compatibility_revision, 3);
        write("references/workflows/plan.md", b"\xff");
        assert_eq!(
            load(&repository, "plan", &[], &[]).unwrap_err().reason_code,
            "invalid_utf8"
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_symlinks_cannot_escape_skill_or_declaring_hierarchy() {
        use std::os::unix::fs::symlink;

        let parent = std::env::temp_dir().join(format!(
            "work-instruction-source-links-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let metadata = b"---\nname: Test\ndescription: Test instructions.\nmetadata:\n  work-tags:\n    - test-tag\n---\n\nBody.\n";
        let create = |label: &str| {
            let root = parent.join(label);
            for (relative, content) in [
                ("references/instruction-loading.md", b"loading\n".as_slice()),
                (
                    "references/instructions/task/general/instructions.md",
                    metadata.as_slice(),
                ),
                (
                    "references/instructions/task/web/instructions.md",
                    metadata.as_slice(),
                ),
            ] {
                let path = root.join(relative);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, content).unwrap();
            }
            fs::create_dir_all(root.join("references/workflows")).unwrap();
            root
        };
        let workflow_root = create("workflow");
        let outside = parent.join("outside.md");
        fs::write(&outside, b"outside\n").unwrap();
        symlink(&outside, workflow_root.join("references/workflows/task.md")).unwrap();
        assert_eq!(
            load(
                &LocalHierarchyCatalog {
                    skill_root: workflow_root,
                },
                "task",
                &[],
                &[]
            )
            .unwrap_err()
            .reason_code,
            "instruction_source_escapes_skill_root"
        );

        let reference_root = create("reference");
        fs::write(
            reference_root.join("references/workflows/task.md"),
            b"workflow\n",
        )
        .unwrap();
        let target =
            reference_root.join("references/instructions/task/web/references/task-records.md");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"outside hierarchy\n").unwrap();
        let source =
            reference_root.join("references/instructions/task/general/references/task-records.md");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        symlink(&target, source).unwrap();
        assert_eq!(
            load(
                &LocalHierarchyCatalog {
                    skill_root: reference_root,
                },
                "task",
                &[],
                &["task.general.task-records".into()]
            )
            .unwrap_err()
            .reason_code,
            "instruction_reference_escapes_hierarchy"
        );
    }

    #[test]
    fn task_document_union_and_validation_match_python_selection_cases() {
        let repository = LocalHierarchyCatalog {
            skill_root: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work")),
        };
        let jpa = load(
            &repository,
            "task",
            &["web/backend/java/jpa".into()],
            &[
                "task.general.task-records".into(),
                "task.web.backend.java.relational-data".into(),
            ],
        )
        .unwrap();
        let mybatis = load(
            &repository,
            "task",
            &["web/backend/java/mybatis".into()],
            &[
                "task.general.task-records".into(),
                "task.web.backend.security".into(),
            ],
        )
        .unwrap();
        let expected = task_document_selection(&[jpa, mybatis]).unwrap();
        assert_eq!(
            expected["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|source| source["logical_name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "work.instruction-loading",
                "work.workflow.task",
                "task.general",
                "task.general.task-records",
                "task.web",
                "task.web.backend",
                "task.web.backend.java",
                "task.web.backend.java.relational-data",
                "task.web.backend.java.jpa",
                "task.web.backend.security",
                "task.web.backend.java.mybatis"
            ]
        );
        assert_eq!(
            expected["references"],
            json!([
                "task.general.task-records",
                "task.web.backend.java.relational-data",
                "task.web.backend.security"
            ])
        );
        assert_eq!(expected["instructions_sha256"].as_str().unwrap().len(), 64);
        assert_eq!(
            validate_task_document_selection(&expected, &expected).unwrap(),
            expected
        );
        let mut invalid = expected.clone();
        invalid["sources"][0]["layer"] = json!("project");
        let error = validate_task_document_selection(&invalid, &expected).unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
        assert_eq!(error.details["unknown"], json!(["layer"]));
        let mut invalid = expected.clone();
        invalid["sources"].as_array_mut().unwrap().swap(0, 1);
        assert_eq!(
            validate_task_document_selection(&invalid, &expected)
                .unwrap_err()
                .reason_code,
            "task_document_instruction_sources_mismatch"
        );
        let mut invalid = expected.clone();
        invalid["references"].as_array_mut().unwrap().reverse();
        assert_eq!(
            validate_task_document_selection(&invalid, &expected)
                .unwrap_err()
                .reason_code,
            "task_document_instruction_references_mismatch"
        );
        let mut invalid = expected.clone();
        invalid["instructions_sha256"] = json!("0".repeat(64));
        assert_eq!(
            validate_task_document_selection(&invalid, &expected)
                .unwrap_err()
                .reason_code,
            "task_document_instructions_fingerprint_mismatch"
        );
    }

    #[test]
    fn routed_reference_documents_keep_small_unlinked_modules() {
        let references = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../skills/work/references"
        ));
        let bootstrap = fs::read_to_string(references.join("instruction-loading.md")).unwrap();
        assert!(bootstrap.lines().count() <= 15);
        assert!(bootstrap.contains("Do not guess sources"));
        assert!(bootstrap.contains("Never fall back"));

        let markdown_files = |directory: &std::path::Path| {
            let mut paths: Vec<_> = fs::read_dir(directory)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
                .collect();
            paths.sort();
            paths
        };
        let has_markdown_link = |content: &str| {
            content.split("](").skip(1).any(|tail| {
                tail.split(')')
                    .next()
                    .is_some_and(|target| target.contains(".md"))
            })
        };
        let common = markdown_files(&references.join("instruction-loading"));
        assert_eq!(common.len(), 18);
        for path in common {
            let content = fs::read_to_string(&path).unwrap();
            assert_eq!(
                content
                    .lines()
                    .filter(|line| line.starts_with("# "))
                    .count(),
                1
            );
            assert!(!has_markdown_link(&content), "{}", path.display());
        }
        for entry in markdown_files(&references.join("workflows")) {
            let content = fs::read_to_string(&entry).unwrap();
            assert!(content.lines().count() <= 15, "{}", entry.display());
            let modules = markdown_files(&entry.with_extension(""));
            assert!(!modules.is_empty(), "{}", entry.display());
            for module in modules {
                let content = fs::read_to_string(&module).unwrap();
                assert!(!has_markdown_link(&content), "{}", module.display());
            }
        }
    }
}
