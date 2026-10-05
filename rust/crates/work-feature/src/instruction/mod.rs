//! Instruction resolution and selection validation over source repositories.

pub mod refresh;
pub mod refresh_build;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use work_operations::derivation::fingerprint;
use work_operations::hierarchy::{CrossModeCatalog, Hierarchy, build_hierarchy, sorted_paths};
use work_operations::instruction::{
    InstructionSelection, ModeCatalog, SourceSet, SourceSummary, selection as source_selection,
};
use work_operations::protocol::{INVALID_SHA256_ERROR_CODE, WORKFLOW_MODES, valid_sha256};
use work_operations::routing::RoutingRequest;

use crate::error::{ExitCode, WorkError};
use crate::hierarchy::HierarchyCatalogRepository;
use crate::workflow::WorkflowRoutingRepository;

pub trait InstructionCatalogRepository: HierarchyCatalogRepository {
    fn scan_mode_metadata(&self, mode: &str) -> Result<BTreeMap<String, Value>, WorkError>;
}

pub fn mode_catalog(
    repository: &impl InstructionCatalogRepository,
    mode: &str,
) -> Result<ModeCatalog, WorkError> {
    if !matches!(mode, "task" | "execute") {
        return Err(WorkError::new(
            ExitCode::Contract,
            "invalid_instruction_mode",
            "The instruction mode must be task or execute.",
            json!({"mode":mode}),
        ));
    }
    let metadata = repository.scan_mode_metadata(mode)?;
    if !metadata.contains_key("general") {
        return Err(WorkError::new(
            ExitCode::Contract,
            "general_instruction_not_unique",
            "The instruction mode must contain exactly one general entrypoint.",
            json!({"mode":mode,"count":0}),
        ));
    }
    for path in metadata.keys().filter(|path| path.as_str() != "general") {
        let parts: Vec<_> = path.split('/').collect();
        for depth in 1..parts.len() {
            let ancestor = parts[..depth].join("/");
            if !metadata.contains_key(&ancestor) {
                return Err(WorkError::new(
                    ExitCode::Contract,
                    "instruction_ancestor_missing",
                    "Every instruction hierarchy path must have all ancestor entrypoints.",
                    json!({"mode":mode,"path":path,"missing_ancestor":ancestor}),
                ));
            }
        }
    }
    let paths = sorted_paths(metadata.keys().cloned().collect());
    Ok(ModeCatalog { paths, metadata })
}

pub fn cross_mode_catalog(
    repository: &impl InstructionCatalogRepository,
) -> Result<CrossModeCatalog, WorkError> {
    let modes = WORKFLOW_MODES;
    let catalogs: Vec<_> = modes
        .iter()
        .map(|mode| mode_catalog(repository, mode))
        .collect::<Result<_, _>>()?;
    let paths = sorted_paths(
        catalogs
            .iter()
            .flat_map(|catalog| catalog.paths.iter().cloned())
            .collect::<BTreeSet<_>>(),
    );
    let mut children: BTreeMap<String, Vec<String>> = paths
        .iter()
        .map(|path| (path.clone(), Vec::new()))
        .collect();
    for path in paths.iter().filter(|path| path.as_str() != "general") {
        let (parent, child) = path
            .rsplit_once('/')
            .map_or(("general", path.as_str()), |(parent, child)| {
                (parent, child)
            });
        children
            .get_mut(parent)
            .expect("catalog ancestors are validated")
            .push(child.to_owned());
    }
    for values in children.values_mut() {
        values.sort();
    }
    let mut metadata = BTreeMap::new();
    for path in &paths {
        let mode_support: Vec<_> = modes
            .iter()
            .enumerate()
            .filter_map(|(index, mode)| {
                catalogs[index].metadata.contains_key(path).then_some(*mode)
            })
            .collect();
        let mode_metadata: BTreeMap<_, _> = modes
            .iter()
            .enumerate()
            .filter_map(|(index, mode)| {
                catalogs[index]
                    .metadata
                    .get(path)
                    .map(|value| ((*mode).to_owned(), value.clone()))
            })
            .collect();
        metadata.insert(
            path.clone(),
            json!({"mode_support":mode_support,"modes":mode_metadata}),
        );
    }
    let value = json!({"schema":"work-instruction-catalog","mode":"all",
        "paths":paths,"children":children,"metadata":metadata});
    let catalog_sha256 =
        fingerprint::structured(&value).expect("catalog contains only JSON values");
    Ok(CrossModeCatalog {
        paths,
        children,
        metadata,
        catalog_sha256,
    })
}

pub fn catalog(
    repository: &impl InstructionCatalogRepository,
    mode: &str,
) -> Result<Value, WorkError> {
    if mode == "all" {
        let catalog = cross_mode_catalog(repository)?;
        return Ok(work_model::instruction::verified::<
            work_model::instruction::InstructionCatalog,
        >(
            json!({"schema":"work-instruction-catalog","mode":"all",
            "paths":catalog.paths,"children":catalog.children,"metadata":catalog.metadata,
            "catalog_sha256":catalog.catalog_sha256})
        ));
    }
    let catalog = mode_catalog(repository, mode)?;
    let mut children: BTreeMap<String, Vec<String>> = catalog
        .paths
        .iter()
        .map(|path| (path.clone(), Vec::new()))
        .collect();
    for path in catalog
        .paths
        .iter()
        .filter(|path| path.as_str() != "general")
    {
        let (parent, child) = path
            .rsplit_once('/')
            .map_or(("general", path.as_str()), |(parent, child)| {
                (parent, child)
            });
        children
            .get_mut(parent)
            .expect("catalog ancestors are validated")
            .push(child.to_owned());
    }
    for names in children.values_mut() {
        names.sort();
    }
    let value = json!({"schema":"work-instruction-catalog","mode":mode,
        "paths":catalog.paths,"children":children,"metadata":catalog.metadata});
    let catalog_sha256 = fingerprint::structured(&value).expect("catalog JSON serializes");
    let mut result = value;
    result["catalog_sha256"] = json!(catalog_sha256);
    Ok(work_model::instruction::verified::<
        work_model::instruction::InstructionCatalog,
    >(result))
}

pub trait InstructionSourceRepository: HierarchyCatalogRepository {
    fn load_sources(
        &self,
        mode: &str,
        hierarchy: &Hierarchy,
        references: &[String],
    ) -> Result<SourceSet, WorkError>;
}

pub fn migration_manifest(
    routing: &mut impl WorkflowRoutingRepository,
    mode: &str,
    status: &str,
    operation: &str,
    artifact: &Value,
) -> Result<Value, WorkError> {
    let mut state = serde_json::Map::new();
    for field in [
        "schema",
        "requirement_id",
        "spec_id",
        "task_spec_id",
        "id",
        "status",
        "overall_status",
    ] {
        if let Some(value) = artifact.get(field) {
            state.insert(field.into(), value.clone());
        }
    }
    let state_sha = fingerprint::structured(&Value::Object(state)).map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "instruction_migration_state_invalid",
            "The migration routing state is invalid.",
            json!({}),
        )
    })?;
    let request = RoutingRequest {
        status,
        operation,
        confirmation: false,
        mode: Some(mode),
        artifact_lifecycle: "confirmed",
        formal_events: &[],
        role: "main",
        authorization_state: Some("authorized"),
        verified_state_sha256: &state_sha,
    };
    let selected = routing.route(&request)?;
    if selected["routing_status"] != "VALID" {
        return Err(WorkError::new(
            ExitCode::ArtifactIntegrity,
            "instruction_migration_routing_review_required",
            "The new router could not build a valid manifest.",
            json!({"status":status,"operation":operation}),
        ));
    }
    Ok(selected["selection_manifest"].clone())
}

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

pub fn resolve_hierarchy(
    repository: &impl HierarchyCatalogRepository,
    mode: &str,
    selected_paths: &[String],
) -> Result<Hierarchy, WorkError> {
    let hierarchy = build_hierarchy(mode, selected_paths).map_err(|issue| {
        error(
            ExitCode::Contract,
            issue.reason_code,
            issue.message,
            issue.details,
        )
    })?;
    let mode_paths = repository.mode_paths(mode)?;
    for path in &hierarchy.resolved_paths {
        if !mode_paths.contains(path) {
            let parent = path
                .rsplit_once('/')
                .map_or("general", |(parent, _)| parent);
            let choices: Vec<_> = mode_paths
                .iter()
                .filter_map(|candidate| candidate.strip_prefix(&format!("{parent}/")))
                .filter(|rest| !rest.contains('/'))
                .collect();
            return Err(error(
                ExitCode::Contract,
                "instruction_hierarchy_path_missing",
                "A selected instruction hierarchy path does not exist in the catalog.",
                json!({"mode": mode, "path": path, "parent": parent, "valid_choices": choices}),
            ));
        }
    }
    Ok(hierarchy)
}

pub fn load(
    repository: &impl InstructionSourceRepository,
    mode: &str,
    selected_paths: &[String],
    references: &[String],
) -> Result<SourceSet, WorkError> {
    let hierarchy = resolve_hierarchy(repository, mode, selected_paths)?;
    repository.load_sources(mode, &hierarchy, references)
}

pub fn select(
    repository: &impl InstructionSourceRepository,
    mode: &str,
    selected_paths: &[String],
    references: &[String],
) -> Result<InstructionSelection, WorkError> {
    Ok(source_selection(&load(
        repository,
        mode,
        selected_paths,
        references,
    )?))
}

pub fn select_task(
    repository: &impl InstructionSourceRepository,
    confirmed_hierarchy: &Value,
    selected_paths: &[String],
    references: &[String],
) -> Result<InstructionSelection, WorkError> {
    crate::hierarchy::validate_selection(repository, confirmed_hierarchy)?;
    crate::hierarchy::validate_task_paths(
        repository,
        selected_paths,
        confirmed_hierarchy,
        "instruction_selection",
    )?;
    let loaded = load(repository, "task", selected_paths, references)?;
    require_task_sources(&loaded)?;
    Ok(source_selection(&loaded))
}

pub fn validate_task_selection(
    repository: &impl InstructionSourceRepository,
    confirmed_hierarchy: &Value,
    stored: &InstructionSelection,
) -> Result<SourceSet, WorkError> {
    crate::hierarchy::validate_selection(repository, confirmed_hierarchy)?;
    crate::hierarchy::validate_task_paths(
        repository,
        &stored.selected_paths,
        confirmed_hierarchy,
        "instruction_selection",
    )?;
    let loaded = validate_selection(repository, "task", stored)?;
    require_task_sources(&loaded)?;
    Ok(loaded)
}

fn require_task_sources(set: &SourceSet) -> Result<(), WorkError> {
    if set.mode != "task"
        || set.sources.iter().any(|source| {
            let name = source.summary.logical_name.as_str();
            match source.summary.kind.as_str() {
                "instruction" | "reference" => !name.starts_with("task."),
                "workflow" => {
                    name != "work.instruction-loading"
                        && !name.starts_with("work.shared.")
                        && name != "work.workflow.task"
                        && !name.starts_with("work.workflow.task.")
                }
                _ => true,
            }
        })
    {
        return Err(error(
            ExitCode::Contract,
            "task_instruction_source_mode_mismatch",
            "Task instruction selections must contain only Task and shared sources.",
            json!({"mode": set.mode}),
        ));
    }
    Ok(())
}

pub fn validate_selection(
    repository: &impl InstructionSourceRepository,
    mode: &str,
    stored: &InstructionSelection,
) -> Result<SourceSet, WorkError> {
    let current = load(repository, mode, &stored.selected_paths, &stored.references)?;
    let expected = source_selection(&current);
    if stored.selected_paths != expected.selected_paths
        || stored.resolved_paths != expected.resolved_paths
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "instruction_selection_hierarchy_mismatch",
            "The stored instruction hierarchy does not match the current resolution.",
            json!({"location": "instruction_selection"}),
        ));
    }
    if stored.references != expected.references {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "instruction_selection_references_mismatch",
            "The stored instruction references are not in actual load order.",
            json!({"location": "instruction_selection.references"}),
        ));
    }
    if stored.sources != expected.sources {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "instruction_selection_sources_mismatch",
            "The stored instruction sources do not match the current sources.",
            json!({"location": "instruction_selection.sources"}),
        ));
    }
    if stored.instructions_sha256 != expected.instructions_sha256 {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "instructions_fingerprint_mismatch",
            "The stored instruction fingerprint does not match the current sources.",
            json!({"location": "instruction_selection.instructions_sha256"}),
        ));
    }
    Ok(current)
}

fn strict_fields<'a>(
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

fn strings(value: &Value, location: &str, allow_empty: bool) -> Result<Vec<String>, WorkError> {
    let values = value
        .as_array()
        .filter(|values| allow_empty || !values.is_empty())
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_string_array",
                "A string array with the required cardinality is required.",
                json!({"location": location}),
            )
        })?;
    let mut result = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let Some(item) = value.as_str().filter(|item| !item.is_empty()) else {
            return Err(error(
                ExitCode::Contract,
                "invalid_string_array",
                "Every array item must be a non-empty string.",
                json!({"location": format!("{location}[{index}]")}),
            ));
        };
        result.push(item.to_owned());
    }
    Ok(result)
}

fn sha(value: &Value, location: &str) -> Result<String, WorkError> {
    let valid = value.as_str().filter(|hash| valid_sha256(hash));
    valid.map(str::to_owned).ok_or_else(|| {
        error(
            ExitCode::Contract,
            INVALID_SHA256_ERROR_CODE,
            "A SHA-256 value must contain 64 lowercase hexadecimal characters.",
            json!({"location": location}),
        )
    })
}

fn validate_optional_routing_manifest(value: &Value) -> Result<(), WorkError> {
    if let Some(manifest) = value.get("routing_manifest") {
        serde_json::from_value::<work_model::instruction::InstructionSelectionManifest>(
            manifest.clone(),
        )
        .map_err(|cause| {
            error(
                ExitCode::Contract,
                "invalid_instruction_routing_manifest",
                "Stored routing metadata must match the instruction selection manifest contract.",
                json!({"cause":cause.to_string()}),
            )
        })?;
    }
    Ok(())
}

pub fn parse_selection(value: &Value, location: &str) -> Result<InstructionSelection, WorkError> {
    validate_optional_routing_manifest(value)?;
    let selection = strict_fields(
        value,
        location,
        &[
            "selected_paths",
            "resolved_paths",
            "sources",
            "references",
            "instructions_sha256",
        ],
        &["routing_manifest"],
    )?;
    let selected_paths = strings(
        &selection["selected_paths"],
        &format!("{location}.selected_paths"),
        true,
    )?;
    let resolved_paths = strings(
        &selection["resolved_paths"],
        &format!("{location}.resolved_paths"),
        false,
    )?;
    let references = strings(
        &selection["references"],
        &format!("{location}.references"),
        true,
    )?;
    if references
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
        != references.len()
    {
        return Err(error(
            ExitCode::Contract,
            "duplicate_instruction_reference",
            "Instruction reference logical names must be unique.",
            json!({"location": format!("{location}.references")}),
        ));
    }
    let raw_sources = selection["sources"]
        .as_array()
        .filter(|sources| !sources.is_empty())
        .ok_or_else(|| {
            error(
                ExitCode::Contract,
                "invalid_instruction_sources",
                "Instruction sources must be a non-empty array.",
                json!({"location": format!("{location}.sources")}),
            )
        })?;
    let mut sources = Vec::new();
    for (index, value) in raw_sources.iter().enumerate() {
        let source_location = format!("{location}.sources[{index}]");
        let source = strict_fields(
            value,
            &source_location,
            &["kind", "logical_name", "canonical_sha256"],
            &[],
        )?;
        let kind = source["kind"]
            .as_str()
            .filter(|kind| matches!(*kind, "workflow" | "instruction" | "reference"))
            .ok_or_else(|| {
                error(
                    ExitCode::Contract,
                    "invalid_instruction_kind",
                    "The instruction source kind is invalid.",
                    json!({"location": format!("{source_location}.kind"), "kind": source["kind"]}),
                )
            })?;
        let logical_name = source["logical_name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                error(
                    ExitCode::Contract,
                    "invalid_instruction_logical_name",
                    "The instruction source logical name must be a non-empty string.",
                    json!({"location": format!("{source_location}.logical_name")}),
                )
            })?;
        sources.push(SourceSummary {
            kind: kind.into(),
            logical_name: logical_name.into(),
            canonical_sha256: sha(
                &source["canonical_sha256"],
                &format!("{source_location}.canonical_sha256"),
            )?,
        });
    }
    Ok(InstructionSelection {
        selected_paths,
        resolved_paths,
        sources,
        references,
        instructions_sha256: sha(
            &selection["instructions_sha256"],
            &format!("{location}.instructions_sha256"),
        )?,
    })
}

pub fn validate_selection_value(
    repository: &impl InstructionSourceRepository,
    mode: &str,
    value: &Value,
    location: &str,
) -> Result<SourceSet, WorkError> {
    let stored = parse_selection(value, location)?;
    validate_selection(repository, mode, &stored)
}

pub fn validate_work_selection_value(
    repository: &impl InstructionSourceRepository,
    mode: &str,
    value: &Value,
    selected_paths: &[String],
    location: &str,
) -> Result<SourceSet, WorkError> {
    let stored = parse_selection(value, location)?;
    if stored.selected_paths != selected_paths {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "work_instruction_selection_selected_paths_mismatch",
            "The stored Work selected paths do not match the confirmed hierarchy selection.",
            json!({"location": format!("{location}.selected_paths")}),
        ));
    }
    let current = load(repository, mode, selected_paths, &stored.references)?;
    let expected = source_selection(&current);
    if stored.resolved_paths != expected.resolved_paths {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "work_instruction_selection_hierarchy_mismatch",
            "The stored Work hierarchy does not match the current mode resolution.",
            json!({"location": location}),
        ));
    }
    if stored.references != expected.references {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "work_instruction_selection_references_mismatch",
            "The stored Work instruction references are not in actual load order.",
            json!({"location": format!("{location}.references")}),
        ));
    }
    if stored.sources != expected.sources {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "work_instruction_selection_sources_mismatch",
            "The stored Work instruction sources do not match the current sources.",
            json!({"location": format!("{location}.sources")}),
        ));
    }
    if stored.instructions_sha256 != expected.instructions_sha256 {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "work_instructions_fingerprint_mismatch",
            "The stored Work instruction fingerprint does not match the current sources.",
            json!({"location": format!("{location}.instructions_sha256")}),
        ));
    }
    Ok(current)
}

pub fn task_document_selection(task_sources: &[SourceSet]) -> Result<Value, WorkError> {
    if task_sources.is_empty() {
        return Err(error(
            ExitCode::Contract,
            "task_instruction_selections_required",
            "At least one TASK instruction selection is required.",
            json!({}),
        ));
    }
    let mut union = Vec::new();
    let mut identities = std::collections::HashMap::new();
    let mut references = Vec::new();
    for set in task_sources {
        require_task_sources(set)?;
        for source in &set.sources {
            let key = (
                source.summary.kind.clone(),
                source.summary.logical_name.clone(),
            );
            if let Some(existing) = identities.get(&key) {
                if existing != &source.canonical_content {
                    return Err(error(
                        ExitCode::ArtifactIntegrity,
                        "instruction_source_identity_conflict",
                        "One instruction source identity resolved to different content.",
                        json!({"kind": key.0, "logical_name": key.1}),
                    ));
                }
            } else {
                identities.insert(key, source.canonical_content.clone());
                union.push(source.clone());
            }
        }
        for reference in &set.references {
            if !references.contains(reference) {
                references.push(reference.clone());
            }
        }
    }
    let fingerprint_sources: Vec<_> = union
        .iter()
        .map(|source| work_operations::canonical::InstructionSource {
            kind: &source.summary.kind,
            logical_name: &source.summary.logical_name,
            content: &source.canonical_content,
        })
        .collect();
    let digest = fingerprint::instruction_selection("task", &fingerprint_sources);
    let summaries: Vec<_> = union.iter().map(|source| source.summary.clone()).collect();
    Ok(json!({"sources": summaries, "references": references, "instructions_sha256": digest}))
}

pub fn validate_task_document_selection(
    value: &Value,
    expected: &Value,
) -> Result<Value, WorkError> {
    validate_optional_routing_manifest(value)?;
    let location = "instruction_selection";
    let selection = strict_fields(
        value,
        location,
        &["sources", "references", "instructions_sha256"],
        &["routing_manifest"],
    )?;
    let parsed = json!({
        "selected_paths": [],
        "resolved_paths": ["general"],
        "sources": selection["sources"],
        "references": selection["references"],
        "instructions_sha256": selection["instructions_sha256"]
    });
    parse_selection(&parsed, location)?;
    for (field, reason, message) in [
        (
            "sources",
            "task_document_instruction_sources_mismatch",
            "Document instruction sources do not match the TASK source union.",
        ),
        (
            "references",
            "task_document_instruction_references_mismatch",
            "Document instruction references do not match the TASK reference union.",
        ),
        (
            "instructions_sha256",
            "task_document_instructions_fingerprint_mismatch",
            "The document instruction fingerprint does not match its source union.",
        ),
    ] {
        if selection[field] != expected[field] {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                reason,
                message,
                json!({"location": format!("{location}.{field}")}),
            ));
        }
    }
    Ok(expected.clone())
}

/// Validate persisted instruction metadata without loading its original source files.
pub fn stored_selection(
    value: &Value,
    selected_paths: Option<&[String]>,
) -> Result<Value, WorkError> {
    strict_fields(
        value,
        "historical instruction selection",
        &[
            "selected_paths",
            "resolved_paths",
            "sources",
            "references",
            "instructions_sha256",
        ],
        &["routing_manifest"],
    )?;
    let parsed = parse_selection(value, "historical instruction selection")?;
    for paths in [&parsed.selected_paths, &parsed.resolved_paths] {
        if paths.iter().collect::<std::collections::HashSet<_>>().len() != paths.len() {
            return Err(error(
                ExitCode::Contract,
                "duplicate_instruction_path",
                "Stored paths must be unique.",
                json!({}),
            ));
        }
    }
    if selected_paths.is_some_and(|selected| selected != parsed.selected_paths) {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "work_instruction_selection_selected_paths_mismatch",
            "Stored Work paths differ from the confirmed selection.",
            json!({}),
        ));
    }
    let mut identities = std::collections::HashSet::new();
    for source in &parsed.sources {
        if !identities.insert((&source.kind, &source.logical_name)) {
            return Err(error(
                ExitCode::Contract,
                "duplicate_instruction_source",
                "Stored source identities must be unique.",
                json!({}),
            ));
        }
    }
    Ok(value.clone())
}

pub fn stored_document_selection(
    value: &Value,
    task_selections: &[Value],
) -> Result<Value, WorkError> {
    validate_optional_routing_manifest(value)?;
    strict_fields(
        value,
        "historical instruction selection",
        &["sources", "references", "instructions_sha256"],
        &["routing_manifest"],
    )?;
    let document_as_selection = json!({
        "selected_paths": [], "resolved_paths": ["general"],
        "sources": value["sources"], "references": value["references"],
        "instructions_sha256": value["instructions_sha256"],
    });
    stored_selection(&document_as_selection, None)?;
    let mut sources = Vec::new();
    let mut references = Vec::new();
    let mut identities = std::collections::HashMap::new();
    let mut fingerprints = std::collections::HashMap::new();
    for task in task_selections {
        stored_selection(task, None)?;
        let signature = task["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| {
                (
                    source["kind"].as_str().unwrap().to_owned(),
                    source["logical_name"].as_str().unwrap().to_owned(),
                    source["canonical_sha256"].as_str().unwrap().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let fingerprint = task["instructions_sha256"].as_str().unwrap();
        if fingerprints
            .insert(signature, fingerprint.to_owned())
            .is_some_and(|previous| previous != fingerprint)
        {
            return Err(error(
                ExitCode::ArtifactIntegrity,
                "historical_instruction_fingerprint_conflict",
                "Identical stored source sets have different fingerprints.",
                json!({}),
            ));
        }
        for source in task["sources"].as_array().unwrap() {
            let identity = (
                source["kind"].as_str().unwrap().to_owned(),
                source["logical_name"].as_str().unwrap().to_owned(),
            );
            if let Some(previous) = identities.get(&identity) {
                if previous != source {
                    return Err(error(
                        ExitCode::ArtifactIntegrity,
                        "instruction_source_identity_conflict",
                        "Stored TASK sources conflict.",
                        json!({}),
                    ));
                }
            } else {
                identities.insert(identity, source.clone());
                sources.push(source.clone());
            }
        }
        for reference in task["references"].as_array().unwrap() {
            if !references.contains(reference) {
                references.push(reference.clone());
            }
        }
    }
    if value["sources"] != json!(sources) || value["references"] != json!(references) {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "historical_instruction_union_mismatch",
            "Stored document sources and references must equal the TASK union.",
            json!({}),
        ));
    }
    let signature = sources
        .iter()
        .map(|source| {
            (
                source["kind"].as_str().unwrap().to_owned(),
                source["logical_name"].as_str().unwrap().to_owned(),
                source["canonical_sha256"].as_str().unwrap().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    if fingerprints
        .get(&signature)
        .is_some_and(|fingerprint| fingerprint != value["instructions_sha256"].as_str().unwrap())
    {
        return Err(error(
            ExitCode::ArtifactIntegrity,
            "historical_instruction_fingerprint_conflict",
            "Stored document and TASK fingerprints conflict.",
            json!({}),
        ));
    }
    Ok(value.clone())
}

#[cfg(test)]
mod tests {
    use super::{
        InstructionCatalogRepository, mode_catalog, parse_selection, sha,
        stored_document_selection, stored_selection, strict_fields, strings,
        validate_task_document_selection,
    };
    use crate::error::WorkError;
    use crate::hierarchy::HierarchyCatalogRepository;
    use serde_json::json;
    use std::collections::BTreeMap;
    use work_operations::hierarchy::CrossModeCatalog;

    struct EmptyCatalog;

    impl HierarchyCatalogRepository for EmptyCatalog {
        fn cross_mode_catalog(&self) -> Result<CrossModeCatalog, WorkError> {
            unreachable!("mode catalog does not need cross-mode access")
        }

        fn mode_paths(&self, _: &str) -> Result<Vec<String>, WorkError> {
            unreachable!("mode catalog does not need mode paths")
        }
    }

    impl InstructionCatalogRepository for EmptyCatalog {
        fn scan_mode_metadata(
            &self,
            _: &str,
        ) -> Result<BTreeMap<String, serde_json::Value>, WorkError> {
            Ok(BTreeMap::new())
        }
    }

    #[test]
    fn catalog_rules_are_enforced_with_a_fake_repository() {
        assert_eq!(
            mode_catalog(&EmptyCatalog, "task").unwrap_err().reason_code,
            "general_instruction_not_unique"
        );
        assert_eq!(
            mode_catalog(&EmptyCatalog, "invalid")
                .unwrap_err()
                .reason_code,
            "invalid_instruction_mode"
        );
    }

    #[test]
    fn instruction_validation_primitives_match_current_contract_cases() {
        let value = json!({"name": "work"});
        assert_eq!(
            strict_fields(&value, "selection", &["name"], &[]).unwrap(),
            value.as_object().unwrap()
        );
        let fields =
            strict_fields(&json!({"extra": true}), "selection", &["name"], &[]).unwrap_err();
        assert_eq!(fields.reason_code, "invalid_object_fields");
        assert_eq!(fields.details["missing"], json!(["name"]));
        assert_eq!(fields.details["unknown"], json!(["extra"]));

        let items = strings(&json!(["general", ""]), "paths", true).unwrap_err();
        assert_eq!(items.reason_code, "invalid_string_array");
        assert_eq!(items.details["location"], "paths[1]");

        let lower = "a".repeat(64);
        assert_eq!(sha(&json!(lower), "fingerprint").unwrap(), lower);
        let upper = sha(&json!("A".repeat(64)), "fingerprint").unwrap_err();
        assert_eq!(upper.reason_code, "invalid_sha256");
    }

    #[test]
    fn historical_metadata_validates_without_sources_and_preserves_union_order() {
        let first = json!({"selected_paths":[],"resolved_paths":["general"],
            "references":["first"],"instructions_sha256":"a".repeat(64),
            "sources":[{"kind":"instruction","logical_name":"task.general",
                "canonical_sha256":"b".repeat(64)}]});
        let mut second = first.clone();
        second["sources"][0]["logical_name"] = json!("task.web");
        second["instructions_sha256"] = json!("c".repeat(64));
        second["references"] = json!(["second", "first"]);
        assert_eq!(stored_selection(&first, Some(&[])).unwrap(), first);
        assert_eq!(
            stored_selection(&first, Some(&["web".to_owned()]))
                .unwrap_err()
                .reason_code,
            "work_instruction_selection_selected_paths_mismatch"
        );
        let mut duplicate = first.clone();
        duplicate["sources"]
            .as_array_mut()
            .unwrap()
            .push(first["sources"][0].clone());
        assert_eq!(
            stored_selection(&duplicate, None).unwrap_err().reason_code,
            "duplicate_instruction_source"
        );
        let document = json!({"sources":[first["sources"][0],second["sources"][0]],
            "references":["first","second"],"instructions_sha256":"d".repeat(64)});
        assert_eq!(
            stored_document_selection(&document, &[first.clone(), second.clone(), first.clone()])
                .unwrap(),
            document
        );
        let mut reversed = document.clone();
        reversed["sources"].as_array_mut().unwrap().reverse();
        assert_eq!(
            stored_document_selection(&reversed, &[first.clone(), second.clone()])
                .unwrap_err()
                .reason_code,
            "historical_instruction_union_mismatch"
        );
        let mut other_fingerprint = first.clone();
        other_fingerprint["instructions_sha256"] = json!("c".repeat(64));
        assert_eq!(
            stored_document_selection(&document, &[first.clone(), other_fingerprint.clone()])
                .unwrap_err()
                .reason_code,
            "historical_instruction_fingerprint_conflict"
        );
        other_fingerprint["sources"][0]["canonical_sha256"] = json!("d".repeat(64));
        assert_eq!(
            stored_document_selection(&document, &[first, other_fingerprint])
                .unwrap_err()
                .reason_code,
            "instruction_source_identity_conflict"
        );
    }
    #[test]
    fn malformed_optional_routing_metadata_is_rejected_at_all_selection_entries() {
        let document = json!({"sources":[],"references":[],"instructions_sha256":"a".repeat(64),
            "routing_manifest":{"schema":"work-instruction-selection-manifest"}});
        let mut selection = document.clone();
        selection["selected_paths"] = json!([]);
        selection["resolved_paths"] = json!(["general"]);
        for result in [
            parse_selection(&selection, "instruction_selection").map(|_| ()),
            stored_selection(&selection, None).map(|_| ()),
            validate_task_document_selection(&document, &document).map(|_| ()),
            stored_document_selection(&document, &[]).map(|_| ()),
        ] {
            assert_eq!(
                result.unwrap_err().reason_code,
                "invalid_instruction_routing_manifest"
            );
        }
    }
}
