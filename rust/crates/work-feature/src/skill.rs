//! Skill selection use case over current catalog snapshots.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_operations::derivation::fingerprint;
use work_operations::protocol::{INVALID_SHA256_ERROR_CODE, WORKFLOW_MODES, valid_sha256};

use crate::error::{ExitCode, WorkError};

#[derive(Debug, Clone)]
pub struct SkillRoot {
    pub scope: String,
    pub locator: String,
}

pub trait SkillSnapshotRepository {
    fn snapshot(&self, scope: &str, root: &str, source: &str) -> Result<Value, WorkError>;
}

pub trait SkillCatalogRepository {
    fn roots(&self) -> Vec<SkillRoot>;
    fn list_directories(&self, root_index: usize) -> Result<Vec<PathBuf>, WorkError>;
    fn has_entrypoint(&self, directory: &Path) -> bool;
    fn resolve_directory(&self, directory: &Path) -> Option<PathBuf>;
    fn summary(&self, root_index: usize, resolved: &Path, source: &str)
    -> Result<Value, WorkError>;
}

pub fn catalog(
    repository: &impl SkillCatalogRepository,
    disabled_sources: &HashSet<String>,
    excluded_names: &HashSet<String>,
) -> Result<Value, WorkError> {
    let mut skills = Vec::new();
    let mut unavailable = Vec::new();
    let mut seen_paths = HashSet::new();
    for (index, root) in repository.roots().iter().enumerate() {
        for directory in repository.list_directories(index)? {
            if !repository.has_entrypoint(&directory) {
                continue;
            }
            let source = format!(
                "{}/SKILL.md",
                directory.file_name().unwrap_or_default().to_string_lossy()
            );
            let resolved = match repository.resolve_directory(&directory) {
                Some(resolved) => resolved,
                None => {
                    unavailable.push(json!({"scope": root.scope, "root": root.locator, "source": source, "code": "skill_path_resolution_failed"}));
                    continue;
                }
            };
            if !seen_paths.insert(resolved.to_string_lossy().to_lowercase())
                || disabled_sources.contains(&source)
            {
                continue;
            }
            match repository.summary(index, &resolved, &source) {
                Ok(value) if !excluded_names.contains(value["name"].as_str().unwrap_or("")) => {
                    skills.push(value)
                }
                Ok(_) => {}
                Err(failure) => unavailable.push(json!({"scope": root.scope, "root": root.locator, "source": source, "code": failure.reason_code, "message": failure.message})),
            }
        }
    }
    Ok(
        work_model::skill::verified::<work_model::skill::SkillCatalog>(
            json!({"schema": "work-skill-catalog", "skills": skills, "unavailable": unavailable}),
        ),
    )
}

pub fn validate_root(scope: &str, locator: &str) -> Result<(), WorkError> {
    if !["repo", "user", "admin", "system"].contains(&scope) {
        return Err(error(
            ExitCode::CliUsage,
            "invalid_skill_scope",
            "The skill root scope is invalid.",
            json!({"scope": scope}),
        ));
    }
    if locator.is_empty()
        || locator.starts_with('/')
        || locator
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(error(
            ExitCode::CliUsage,
            "invalid_skill_root_locator",
            "A skill root locator must be a normalized scope-relative path.",
            json!({"locator": locator}),
        ));
    }
    Ok(())
}

pub fn validate_source(source: &str) -> Result<&str, WorkError> {
    let parts: Vec<_> = source.split('/').collect();
    if parts.len() != 2
        || parts[0].is_empty()
        || parts[0] == "."
        || parts[0] == ".."
        || parts[1] != "SKILL.md"
    {
        return Err(error(
            ExitCode::CliUsage,
            "invalid_skill_source",
            "A skill source must use <folder>/SKILL.md relative to its catalog root.",
            json!({"source": source}),
        ));
    }
    Ok(parts[0])
}

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn fields<'a>(
    value: &'a Value,
    location: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, WorkError> {
    let object = value.as_object().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "expected_object",
            "A JSON object is required.",
            json!({"location": location}),
        )
    })?;
    let mut missing: Vec<_> = required
        .iter()
        .filter(|field| !object.contains_key(**field))
        .copied()
        .collect();
    let mut unknown: Vec<_> = object
        .keys()
        .filter(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
        .cloned()
        .collect();
    missing.sort_unstable();
    unknown.sort();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(error(
            ExitCode::Contract,
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location": location, "missing": missing, "unknown": unknown}),
        ));
    }
    Ok(object)
}

fn text<'a>(value: &'a Value, location: &str) -> Result<&'a str, WorkError> {
    value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "empty_text_value",
                "A non-empty string is required.",
                json!({"location": location}),
            )
        })
}

fn roots_by_identity(roots: &[SkillRoot]) -> Result<HashSet<(&str, &str)>, WorkError> {
    let mut seen = HashSet::new();
    for root in roots {
        if !seen.insert((root.scope.as_str(), root.locator.as_str())) {
            return Err(error(
                ExitCode::CliUsage,
                "duplicate_skill_root",
                "Skill root scope and locator pairs must be unique.",
                json!({"scope": root.scope, "root": root.locator}),
            ));
        }
    }
    Ok(seen)
}

fn declared_modes(
    summary: &Value,
    choice: &serde_json::Map<String, Value>,
) -> Result<Value, WorkError> {
    let declared = summary["work_modes"].as_array().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "invalid_skill_mode_support",
            "Skill mode support is invalid.",
            json!({}),
        )
    })?;
    if declared.is_empty() {
        return choice.get("mode_support").cloned().ok_or_else(|| {
            error(
                ExitCode::Contract,
                "inferred_skill_modes_required",
                "Supply confirmed mode support for a skill without declared modes.",
                json!({}),
            )
        });
    }
    let modes: serde_json::Map<_, _> = WORKFLOW_MODES
        .iter()
        .map(|mode| {
            (
                mode.to_string(),
                json!(if declared.contains(&json!(mode)) {
                    "declared"
                } else {
                    "unsupported"
                }),
            )
        })
        .collect();
    let modes = Value::Object(modes);
    if choice
        .get("mode_support")
        .is_some_and(|value| value != &modes)
    {
        return Err(error(
            ExitCode::Contract,
            "declared_skill_modes_mismatch",
            "Confirmed modes differ from the declared modes.",
            json!({}),
        ));
    }
    Ok(modes)
}

fn string_list(
    value: Option<&Value>,
    field: &str,
    allowed: Option<&[&str]>,
) -> Result<Vec<String>, WorkError> {
    let mut items = Vec::new();
    match value {
        None | Some(Value::Null) => {}
        Some(Value::String(text)) => items.extend(
            text.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned),
        ),
        Some(Value::Array(values)) if values.iter().all(Value::is_string) => items.extend(
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_owned),
        ),
        _ => {
            return Err(error(
                ExitCode::InputFormat,
                "invalid_skill_metadata_field",
                "The skill metadata field must be a string or string array.",
                json!({"field": field}),
            ));
        }
    }
    if items.iter().collect::<HashSet<_>>().len() != items.len()
        || allowed.is_some_and(|allowed| items.iter().any(|item| !allowed.contains(&item.as_str())))
    {
        return Err(error(
            ExitCode::InputFormat,
            "invalid_skill_metadata_value",
            "The skill metadata field contains duplicate or unsupported values.",
            json!({"field": field, "values": items}),
        ));
    }
    Ok(items)
}

pub fn summary_from_metadata(
    frontmatter: &Value,
    openai: impl FnOnce() -> Result<Option<Value>, WorkError>,
    scope: &str,
    locator: &str,
    source: &str,
    source_path: &Path,
    openai_path: &Path,
) -> Result<Value, WorkError> {
    let object = frontmatter.as_object().ok_or_else(|| {
        error(
            ExitCode::InputFormat,
            "invalid_skill_yaml_object",
            "Skill YAML must decode to an object with string keys.",
            json!({"source": source_path}),
        )
    })?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let (Some(name), Some(description)) = (name, description) else {
        return Err(error(
            ExitCode::InputFormat,
            "invalid_skill_identity",
            "Skill frontmatter requires non-empty name and description strings.",
            json!({"source": source}),
        ));
    };
    let metadata = object
        .get("metadata")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let metadata = metadata.as_object().ok_or_else(|| {
        error(
            ExitCode::InputFormat,
            "invalid_skill_yaml_object",
            "Skill YAML must decode to an object with string keys.",
            json!({"source": format!("{source}:metadata")}),
        )
    })?;
    let modes = string_list(
        metadata.get("work-modes"),
        "metadata.work-modes",
        Some(&WORKFLOW_MODES),
    )?;
    let tags = string_list(metadata.get("work-tags"), "metadata.work-tags", None)?;
    let mut allow_implicit = true;
    let mut dependencies = Vec::new();
    if let Some(openai) = openai()? {
        let policy = openai
            .get("policy")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let policy = policy.as_object().ok_or_else(|| {
            error(
                ExitCode::InputFormat,
                "invalid_skill_yaml_object",
                "Skill YAML must decode to an object with string keys.",
                json!({"source": format!("{source}:policy")}),
            )
        })?;
        if let Some(value) = policy.get("allow_implicit_invocation") {
            allow_implicit = value.as_bool().ok_or_else(|| {
                error(
                    ExitCode::InputFormat,
                    "invalid_skill_invocation_policy",
                    "allow_implicit_invocation must be a boolean.",
                    json!({"source": source}),
                )
            })?;
        }
        if let Some(tools) = openai
            .get("dependencies")
            .and_then(|value| value.get("tools"))
        {
            let tools = tools.as_array().ok_or_else(|| {
                error(
                    ExitCode::InputFormat,
                    "invalid_skill_dependencies",
                    "Skill tool dependencies must be an array.",
                    json!({"source": openai_path}),
                )
            })?;
            for (index, tool) in tools.iter().enumerate() {
                let kind = tool.get("type").and_then(Value::as_str);
                let value = tool.get("value").and_then(Value::as_str);
                let (Some(kind), Some(value)) = (kind, value) else {
                    return Err(error(
                        ExitCode::InputFormat,
                        "invalid_skill_dependency",
                        "Each skill tool dependency requires string type and value fields.",
                        json!({"source": openai_path, "index": index}),
                    ));
                };
                let mut normalized = json!({"type": kind, "value": value});
                for field in ["description", "transport", "url"] {
                    if let Some(field_value) = tool.get(field) {
                        let text = field_value.as_str().ok_or_else(|| {
                            error(
                                ExitCode::InputFormat,
                                "invalid_skill_dependency",
                                "Known skill dependency fields must be strings.",
                                json!({"source": openai_path, "index": index, "field": field}),
                            )
                        })?;
                        normalized[field] = json!(text);
                    }
                }
                dependencies.push(normalized);
            }
        }
    }
    let name = name.trim();
    let fields = json!({"name": name, "description": description.trim(), "work_modes": modes, "work_tags": tags, "allow_implicit_invocation": allow_implicit, "dependencies": dependencies});
    let digest = fingerprint::structured(&fields).expect("JSON values serialize");
    let mut result = json!({"id": fingerprint::skill_identity(name, scope, locator, source), "scope": scope, "root": locator, "source": source, "summary_sha256": digest});
    for (key, value) in fields.as_object().unwrap() {
        result[key] = value.clone();
    }
    Ok(result)
}

pub fn build_selection(
    repository: &impl SkillSnapshotRepository,
    roots: &[SkillRoot],
    request: &Value,
) -> Result<Value, WorkError> {
    let request = fields(
        request,
        "skill_selection_request",
        &["decision", "skills"],
        &[],
    )?;
    let decision = request["decision"]
        .as_str()
        .filter(|decision| matches!(*decision, "external_skills" | "base_only"))
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_skill_selection_decision",
                "An explicit selection decision is required.",
                json!({}),
            )
        })?;
    let choices = request["skills"].as_array().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "invalid_skill_selection_skills",
            "Skill choices must be an array.",
            json!({}),
        )
    })?;
    if (decision == "base_only") != choices.is_empty() {
        return Err(error(
            ExitCode::Contract,
            "skill_selection_decision_mismatch",
            "The decision does not match the choices.",
            json!({}),
        ));
    }
    let root_identities = roots_by_identity(roots)?;
    let mut skills = Vec::new();
    let mut seen_ids = HashSet::new();
    for (index, value) in choices.iter().enumerate() {
        let location = format!("skill_selection_request.skills[{index}]");
        let choice = fields(
            value,
            &location,
            &[
                "scope",
                "root",
                "source",
                "recommendation_reason",
                "dependency_status",
            ],
            &["mode_support"],
        )?;
        for field in [
            "scope",
            "root",
            "source",
            "recommendation_reason",
            "dependency_status",
        ] {
            text(&choice[field], &format!("{location}.{field}"))?;
        }
        if choice["dependency_status"] != "available" {
            return Err(error(
                ExitCode::Contract,
                "skill_dependencies_unavailable",
                "Confirm dependency availability before building a selection.",
                json!({}),
            ));
        }
        let scope = choice["scope"].as_str().unwrap();
        let root = choice["root"].as_str().unwrap();
        let source = choice["source"].as_str().unwrap();
        if !root_identities.contains(&(scope, root)) {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "selected_skill_root_missing",
                "The selected skill root is not available.",
                json!({}),
            ));
        }
        let snapshot = repository.snapshot(scope, root, source)?;
        let summary = &snapshot["skill"];
        let mode_support = declared_modes(summary, choice)?;
        let mut selected = serde_json::Map::new();
        for field in [
            "id",
            "name",
            "scope",
            "root",
            "source",
            "description",
            "allow_implicit_invocation",
            "summary_sha256",
        ] {
            selected.insert(field.into(), summary[field].clone());
        }
        selected.insert("mode_support".into(), mode_support);
        selected.insert("dependency_status".into(), json!("available"));
        selected.insert(
            "recommendation_reason".into(),
            choice["recommendation_reason"].clone(),
        );
        selected.insert(
            "bundle_sha256".into(),
            snapshot["bundle"]["bundle_sha256"].clone(),
        );
        if !seen_ids.insert(selected["id"].as_str().unwrap_or("").to_owned()) {
            return Err(error(
                ExitCode::Contract,
                "duplicate_selected_skill",
                "A skill may appear only once in a selection.",
                json!({"id": selected["id"]}),
            ));
        }
        skills.push(Value::Object(selected));
    }
    let _: work_model::skill::SkillSelectionRequest =
        serde_json::from_value(Value::Object(request.clone()))
            .expect("validated Skill request matches its model");
    let digest = fingerprint::skill_selection(decision, &skills);
    let selection = json!({"schema": "work-skill-selection", "decision": decision, "skills": skills, "selection_sha256": digest});
    validate_selection(repository, roots, &selection)?;
    Ok(selection)
}

pub fn validate_selection(
    repository: &impl SkillSnapshotRepository,
    roots: &[SkillRoot],
    selection: &Value,
) -> Result<Value, WorkError> {
    let object = fields(
        selection,
        "skill_selection",
        &["schema", "decision", "skills", "selection_sha256"],
        &[],
    )?;
    if object["schema"] != "work-skill-selection" {
        return Err(error(
            ExitCode::Contract,
            "invalid_skill_selection_schema",
            "The skill selection schema is invalid.",
            json!({}),
        ));
    }
    let decision = object["decision"]
        .as_str()
        .filter(|decision| matches!(*decision, "external_skills" | "base_only"))
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_skill_selection_decision",
                "The skill selection decision is invalid.",
                json!({"decision": object["decision"]}),
            )
        })?;
    let skills = object["skills"].as_array().ok_or_else(|| {
        error(
            ExitCode::Contract,
            "invalid_skill_selection_skills",
            "Skill selection skills must be an array.",
            json!({}),
        )
    })?;
    if (decision == "base_only") != skills.is_empty() {
        return Err(error(
            ExitCode::Contract,
            "skill_selection_decision_mismatch",
            "The skill selection decision does not match its skills.",
            json!({}),
        ));
    }
    let roots = roots_by_identity(roots)?;
    let mut ids = HashSet::new();
    for (index, value) in skills.iter().enumerate() {
        let location = format!("skill_selection.skills[{index}]");
        let skill = fields(
            value,
            &location,
            &[
                "id",
                "name",
                "scope",
                "root",
                "source",
                "description",
                "mode_support",
                "allow_implicit_invocation",
                "dependency_status",
                "summary_sha256",
                "bundle_sha256",
                "recommendation_reason",
            ],
            &[],
        )?;
        for field in [
            "id",
            "name",
            "scope",
            "root",
            "source",
            "description",
            "recommendation_reason",
        ] {
            text(&skill[field], &format!("{location}.{field}"))?;
        }
        for field in ["summary_sha256", "bundle_sha256"] {
            let valid = skill[field].as_str().is_some_and(valid_sha256);
            if !valid {
                return Err(error(
                    ExitCode::Contract,
                    INVALID_SHA256_ERROR_CODE,
                    "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
                    json!({"location": format!("{location}.{field}")}),
                ));
            }
        }
        if !skill["allow_implicit_invocation"].is_boolean() {
            return Err(error(
                ExitCode::Contract,
                "invalid_skill_invocation_policy",
                "allow_implicit_invocation must be a boolean.",
                json!({"location": format!("{location}.allow_implicit_invocation")}),
            ));
        }
        if skill["dependency_status"] != "available" {
            return Err(error(
                ExitCode::Contract,
                "skill_dependencies_unavailable",
                "A confirmed skill must have available dependencies.",
                json!({"location": format!("{location}.dependency_status")}),
            ));
        }
        let modes = fields(
            &skill["mode_support"],
            &format!("{location}.mode_support"),
            &WORKFLOW_MODES,
            &[],
        )?;
        if modes["task"] == "unsupported"
            || modes.values().any(|value| {
                !matches!(
                    value.as_str(),
                    Some("declared" | "inferred" | "unsupported")
                )
            })
        {
            return Err(error(
                ExitCode::Contract,
                "invalid_skill_mode_support",
                "Confirmed skills must support Task and use valid mode support values.",
                json!({"location": format!("{location}.mode_support")}),
            ));
        }
        let id = skill["id"].as_str().unwrap();
        if !ids.insert(id) {
            return Err(error(
                ExitCode::Contract,
                "duplicate_selected_skill",
                "A skill may appear only once in a selection.",
                json!({"id": id}),
            ));
        }
        let scope = skill["scope"].as_str().unwrap();
        let root = skill["root"].as_str().unwrap();
        if !roots.contains(&(scope, root)) {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "selected_skill_root_missing",
                "The selected skill root is not available.",
                json!({"scope": scope, "root": root}),
            ));
        }
        let snapshot = repository.snapshot(scope, root, skill["source"].as_str().unwrap())?;
        let current = &snapshot["skill"];
        let expected: HashMap<_, _> = [
            "id",
            "name",
            "scope",
            "root",
            "source",
            "description",
            "allow_implicit_invocation",
            "summary_sha256",
        ]
        .iter()
        .map(|field| (*field, &current[*field]))
        .chain([("bundle_sha256", &snapshot["bundle"]["bundle_sha256"])])
        .collect();
        if expected
            .iter()
            .any(|(field, value)| skill[*field] != **value)
        {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "selected_skill_snapshot_mismatch",
                "The selected skill no longer matches its confirmed snapshot.",
                json!({"id": id}),
            ));
        }
        let declared = current["work_modes"].as_array().ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_skill_mode_support",
                "Skill mode support is invalid.",
                json!({}),
            )
        })?;
        if declared.is_empty() {
            if modes.values().any(|value| value == "declared") {
                return Err(error(
                    ExitCode::Contract,
                    "inferred_skill_modes_required",
                    "A skill without declared modes cannot use declared mode support.",
                    json!({"id": id}),
                ));
            }
        } else if declared_modes(current, &serde_json::Map::new())? != skill["mode_support"] {
            return Err(error(
                ExitCode::Contract,
                "declared_skill_modes_mismatch",
                "Selected mode support does not match declared skill modes.",
                json!({"id": id}),
            ));
        }
    }
    let stored = object["selection_sha256"].as_str().unwrap_or("");
    if !valid_sha256(stored) {
        return Err(error(
            ExitCode::Contract,
            INVALID_SHA256_ERROR_CODE,
            "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
            json!({"location": "skill_selection.selection_sha256"}),
        ));
    }
    if stored != fingerprint::skill_selection(decision, skills) {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "skill_selection_fingerprint_mismatch",
            "The skill selection fingerprint does not match its contents.",
            json!({}),
        ));
    }
    Ok(work_model::skill::verified::<
        work_model::skill::SkillSelectionValidation,
    >(
        json!({"schema": "work-skill-selection-validation", "status": "valid", "skill_selection": selection}),
    ))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use super::{SkillCatalogRepository, SkillRoot, catalog};
    use crate::error::WorkError;

    struct FakeCatalog {
        summaries: Cell<usize>,
    }

    impl SkillCatalogRepository for FakeCatalog {
        fn roots(&self) -> Vec<SkillRoot> {
            vec![SkillRoot {
                scope: "repo".into(),
                locator: "skills".into(),
            }]
        }

        fn list_directories(&self, _: usize) -> Result<Vec<PathBuf>, WorkError> {
            Ok(vec![PathBuf::from("first"), PathBuf::from("second")])
        }

        fn has_entrypoint(&self, _: &Path) -> bool {
            true
        }

        fn resolve_directory(&self, directory: &Path) -> Option<PathBuf> {
            Some(directory.to_path_buf())
        }

        fn summary(&self, _: usize, _: &Path, source: &str) -> Result<Value, WorkError> {
            self.summaries.set(self.summaries.get() + 1);
            Ok(json!({"name":source}))
        }
    }

    #[test]
    fn catalog_filters_disabled_sources_before_loading_summaries() {
        let repository = FakeCatalog {
            summaries: Cell::new(0),
        };
        let output = catalog(
            &repository,
            &HashSet::from(["first/SKILL.md".to_owned()]),
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(repository.summaries.get(), 1);
        assert_eq!(output["skills"], json!([{"name":"second/SKILL.md"}]));
        assert_eq!(output["unavailable"], json!([]));
    }
}
