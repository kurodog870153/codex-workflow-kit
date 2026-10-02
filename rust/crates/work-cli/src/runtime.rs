//! Process bootstrap and command composition.

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_flow::delegation::{build as build_delegation, validate as validate_delegation};
use work_flow::error::{ExitCode, WorkError};
use work_flow::execution::{
    CommandProjectRequest, CommandProjectSources, ExecutionProjectTarget, attempt_render,
    attempt_validate, correction_render, correction_validate,
};
use work_flow::fingerprint::text as fingerprint_text;
use work_flow::hierarchy::{
    resolve as resolve_hierarchy_flow, selection_build as build_selection,
    selection_validate as validate_selection,
};
use work_flow::instruction::{
    catalog as instruction_catalog, load as load_instructions, resolve as resolve_instructions,
    select as select_instructions,
};
use work_flow::invocation::parse as parse_invocation_flow;
use work_flow::paths::resolve as resolve_artifact_paths;
use work_flow::plan::{
    create as create_plan_bytes, semantic_prepare as prepare_semantic,
    validate_bytes as validate_plan_bytes, validate_file as validate_plan_file,
};
use work_flow::progress::{
    prepare as prepare_progress_raw, read as read_progress, save as save_progress_raw,
    validate as preview_progress_raw,
};
use work_flow::skill::SkillRoot;
use work_flow::skill::{
    catalog as skill_catalog, selection_build as build_skill_selection,
    selection_validate as validate_skill_selection, snapshot as skill_snapshot,
};
use work_flow::specification::{
    apply_reconciliation, prepare_reconciliation, preview_reconciliation,
};
use work_flow::task::{
    DraftCreatePorts, ProjectAssemblyInput, TaskCollectionRepository, assemble_task,
    create_from_drafts as create_draft_task, validate_collection as validate_task_collection,
};
use work_flow::workflow::{
    OperationContextRequest, build_operation_context, validate_operation_context,
};
use work_infrastructure::clock_workspace::{LocalWorkspaceAllocator, local_date, local_timestamp};
use work_infrastructure::codec::{canonical_json, decode_utf8, fingerprint, parse_json_contract};
use work_infrastructure::delegation_storage::LocalDelegationStorage;
use work_infrastructure::execution_storage::{AttemptStartRecoveryRequest, LocalExecutionStorage};
use work_infrastructure::files::resolve_project_path;
use work_infrastructure::handoff_storage::LocalHandoffStorage;
use work_infrastructure::hierarchy_catalog::LocalHierarchyCatalog;
use work_infrastructure::instruction::migration::{apply_migration, build_migration};
use work_infrastructure::instruction::refresh::{
    apply_source_refresh, apply_source_refresh_all, build_refresh, preview_source_refresh_all,
    source_impact,
};
use work_infrastructure::plan_storage::LocalPlanStorage;
use work_infrastructure::progress_storage::LocalProgressStorage;
use work_infrastructure::routing_sources::RoutingSourceSession;
use work_infrastructure::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use work_infrastructure::specification::artifact_migration::{
    analyze as analyze_artifact_migration, execute as execute_artifact_migration,
    prepare_request as prepare_artifact_migration, preview as preview_artifact_migration,
    recover as recover_artifact_migration, verify as verify_artifact_migration,
};
use work_infrastructure::specification::migration::{prepare_revision_request, preview_migration};
use work_infrastructure::specification::migration_publication::publish_migration;
use work_infrastructure::specification::migration_verification::verify_semantic_migration;
use work_infrastructure::specification::reconciliation_storage::{
    LocalReconciliationArtifacts, LocalSemanticReconciliation, publish_ledger_only,
    publish_with_migration,
};
use work_infrastructure::specification::reconstruction::prepare_reconstruction_request;
use work_infrastructure::specification::storage::require_no_spec_update;
use work_infrastructure::specification::workflow_storage::{
    SpecOperation, SpecificationPrepareInput, SpecificationProjectRequest, prepare_simple_update,
    update_from_project, verify_from_project,
};
use work_infrastructure::task::assembly_storage::LocalTaskAssembly;
use work_infrastructure::task::create_storage::LocalTaskCreation;
use work_infrastructure::task::draft_storage::{
    LocalTaskDraftStorage, TaskSourceCheckRequest, TaskSourceUpdateProjectRequest,
};
use work_infrastructure::task::semantic_prepare::{
    prepare_semantic_task_request, save_prepared_initial_task, save_prepared_list_task,
    save_prepared_source_task,
};
use work_infrastructure::task::storage::LocalTaskStorage;
use work_infrastructure::workflow_storage::{WorkflowStateRequest, load_workflow_snapshot};

use crate::CliResult;
use crate::contract;
use crate::parser::{ParseOutcome, ParsedCommand, parse_tokens};

#[derive(Debug, Clone)]
pub struct FileInput {
    pub raw: Vec<u8>,
    pub source: String,
    pub source_raw_sha256: String,
}

struct TaskInputOverlay<'a> {
    repository: LocalTaskStorage,
    index_path: &'a str,
    input_raw: &'a [u8],
}

impl TaskCollectionRepository for TaskInputOverlay<'_> {
    fn read_task_file(&self, relative_path: &str) -> Result<Vec<u8>, WorkError> {
        if relative_path == self.index_path {
            Ok(self.input_raw.to_vec())
        } else {
            self.repository.read_task_file(relative_path)
        }
    }

    fn item_names(&self, index_path: &str) -> Result<Vec<String>, WorkError> {
        self.repository.item_names(index_path)
    }
}

fn root_path(raw: &str) -> Result<PathBuf, WorkError> {
    let path = Path::new(raw).canonicalize().map_err(|_| {
        WorkError::new(
            ExitCode::IoFailure,
            "root_resolution_failed",
            "The project root could not be resolved.",
            json!({"path":raw}),
        )
    })?;
    if !path.is_dir() {
        return Err(WorkError::new(
            ExitCode::IoFailure,
            "root_not_directory",
            "The project root is not a directory.",
            json!({"path":path}),
        ));
    }
    Ok(path)
}

fn read_input_file(raw_path: &str) -> Result<FileInput, WorkError> {
    let path = Path::new(raw_path).canonicalize().map_err(|_| {
        WorkError::new(
            ExitCode::IoFailure,
            "input_file_read_failed",
            "The request file could not be read as a regular file.",
            json!({"path":raw_path}),
        )
    })?;
    let raw = fs::read(&path).map_err(|_| {
        WorkError::new(
            ExitCode::IoFailure,
            "input_file_read_failed",
            "The request file could not be read as a regular file.",
            json!({"path":raw_path}),
        )
    })?;
    if !path.is_file() {
        return Err(WorkError::new(
            ExitCode::IoFailure,
            "input_file_read_failed",
            "The request file could not be read as a regular file.",
            json!({"path":raw_path}),
        ));
    }
    let text = decode_utf8(&raw).map_err(|error| {
        WorkError::new(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source":path,"byte_offset":error.valid_up_to()}),
        )
    })?;
    if text.starts_with('\u{feff}') {
        return Err(WorkError::new(
            ExitCode::InputFormat,
            "input_file_multiple_bom",
            "The request file may contain at most one leading UTF-8 BOM.",
            json!({"source":path}),
        ));
    }
    Ok(FileInput {
        raw: text.as_bytes().to_vec(),
        source: path.to_string_lossy().into_owned(),
        source_raw_sha256: fingerprint::raw(&raw),
    })
}

fn operation_artifacts(
    parsed: &ParsedCommand,
    root: &Path,
    input: Option<&FileInput>,
    bind_pre_read_input: bool,
) -> Result<Value, WorkError> {
    let mut artifacts = serde_json::Map::new();
    for (name, value) in &parsed.arguments {
        let Some(raw) = value.as_str() else { continue };
        if !(name.ends_with("_path") || name.ends_with("_file") || name.ends_with("_dir")) {
            continue;
        }
        let path = if matches!(name.as_str(), "input_file" | "output_file") {
            let candidate = Path::new(raw);
            let absolute = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                std::env::current_dir()
                    .map_err(|_| {
                        WorkError::new(
                            ExitCode::IoFailure,
                            "path_resolution_failed",
                            "The path could not be resolved.",
                            json!({"path":raw}),
                        )
                    })?
                    .join(candidate)
            };
            absolute.canonicalize().unwrap_or(absolute)
        } else {
            resolve_project_path(root, raw)?.1
        };
        let state = if name == "input_file" && bind_pre_read_input {
            let input = input.expect("input_file has already been read");
            if path.to_string_lossy() != input.source {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "operation_artifact_drift",
                    "A bound operation artifact changed before the worker started.",
                    json!({}),
                ));
            }
            input.source_raw_sha256.clone()
        } else if path.is_file() {
            fingerprint::raw(&fs::read(&path).map_err(|_| {
                WorkError::new(
                    ExitCode::IoFailure,
                    "operation_artifact_read_failed",
                    "The operation artifact could not be read.",
                    json!({"path":path}),
                )
            })?)
        } else if path.is_dir() {
            "directory".into()
        } else {
            "missing".into()
        };
        artifacts.insert(
            name.clone(),
            json!({"path":path.to_string_lossy(),"raw_sha256":state}),
        );
    }
    Ok(Value::Object(artifacts))
}

fn dispatch_with_operation_context(
    parsed: &ParsedCommand,
    root: &Path,
    input: Option<&FileInput>,
    skill_root: &Path,
) -> Result<Value, WorkError> {
    let Some(command) = parsed.path.first().map(String::as_str) else {
        return dispatch(parsed, root, input, skill_root);
    };
    if !matches!(
        command,
        "plan" | "task" | "execute" | "delegation" | "progress" | "handoff"
    ) {
        return dispatch(parsed, root, input, skill_root);
    }
    let operation = parsed.path.get(1).map(String::as_str).unwrap_or(command);
    let artifacts = operation_artifacts(parsed, root, input, true)?;
    let mut routing = RoutingSourceSession::new(skill_root.to_path_buf());
    let (envelope, selection) = build_operation_context(
        &mut routing,
        &OperationContextRequest {
            command,
            operation,
            delegated_role: parsed.arguments.get("role").and_then(Value::as_str),
            artifacts: &artifacts,
            project_root: &root.to_string_lossy(),
            approval_sha256: parsed
                .arguments
                .get("approved_sha256")
                .and_then(Value::as_str),
            transaction_workspace: parsed
                .arguments
                .get("execution_dir")
                .and_then(Value::as_str),
        },
    )?;
    let current = operation_artifacts(parsed, root, input, false)?;
    validate_operation_context(&envelope, &selection, &current)?;
    let result = dispatch(parsed, root, input, skill_root)?;
    if result["schema"].as_str().is_none_or(str::is_empty) {
        return Err(WorkError::new(
            ExitCode::Contract,
            "operation_result_contract_missing",
            "The worker result does not declare its result contract.",
            json!({}),
        ));
    }
    Ok(result)
}

fn argument<'a>(parsed: &'a ParsedCommand, name: &str) -> Result<&'a str, WorkError> {
    parsed
        .arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| {
            WorkError::new(
                ExitCode::InternalError,
                "unreachable_command",
                "The parsed command could not be dispatched.",
                json!({}),
            )
        })
}

fn string_list(parsed: &ParsedCommand, name: &str) -> Vec<String> {
    parsed
        .arguments
        .get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn required_input(input: Option<&FileInput>) -> Result<&FileInput, WorkError> {
    input.ok_or_else(|| {
        WorkError::new(
            ExitCode::InternalError,
            "unreachable_command",
            "The parsed command could not be dispatched.",
            json!({}),
        )
    })
}

fn input_json(input: Option<&FileInput>) -> Result<Value, WorkError> {
    let input = required_input(input)?;
    parse_json_contract(&input.raw).map_err(|issue| match issue {
        work_infrastructure::codec::JsonContractIssue::DuplicateKey(key) => WorkError::new(
            ExitCode::InputFormat,
            "duplicate_json_key",
            "The JSON contract contains a duplicate key.",
            json!({"key":key}),
        ),
        work_infrastructure::codec::JsonContractIssue::InvalidConstant(value) => WorkError::new(
            ExitCode::InputFormat,
            "invalid_json_constant",
            "The JSON contract contains a non-standard numeric constant.",
            json!({"value":value}),
        ),
        work_infrastructure::codec::JsonContractIssue::NotObject => WorkError::new(
            ExitCode::Contract,
            "json_contract_not_object",
            "The JSON contract root must be an object.",
            json!({}),
        ),
        work_infrastructure::codec::JsonContractIssue::InvalidJson { line, column } => {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The JSON contract is invalid.",
                json!({"line":line,"column":column}),
            )
        }
        _ => WorkError::new(
            ExitCode::InputFormat,
            "invalid_json_contract",
            "The JSON input is invalid.",
            json!({"source":input.source}),
        ),
    })
}

fn progress_contract_input(input: &FileInput) -> Result<(), WorkError> {
    parse_json_contract(&input.raw).map_err(|issue| {
        use work_infrastructure::codec::JsonContractIssue;
        match issue {
            JsonContractIssue::InvalidUtf8(byte_offset) => WorkError::new(
                ExitCode::InputFormat,
                "invalid_utf8",
                "The input is not valid UTF-8.",
                json!({"source":input.source,"byte_offset":byte_offset}),
            ),
            JsonContractIssue::DuplicateKey(key) => WorkError::new(
                ExitCode::InputFormat,
                "duplicate_json_key",
                "The JSON contract contains a duplicate key.",
                json!({"key":key}),
            ),
            JsonContractIssue::InvalidConstant(value) => WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_constant",
                "The JSON contract contains a non-standard numeric constant.",
                json!({"value":value}),
            ),
            JsonContractIssue::NotObject => WorkError::new(
                ExitCode::Contract,
                "json_contract_not_object",
                "The JSON contract root must be an object.",
                json!({}),
            ),
            JsonContractIssue::InvalidJson { line, column } => WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The JSON contract is invalid.",
                json!({"line":line,"column":column}),
            ),
            JsonContractIssue::MultipleBom => WorkError::new(
                ExitCode::InputFormat,
                "invalid_json_contract",
                "The JSON contract is invalid.",
                json!({"line":1,"column":1}),
            ),
        }
    })?;
    Ok(())
}

fn skill_selection_validation_input(input: &FileInput) -> Result<Value, WorkError> {
    let text = decode_utf8(&input.raw).map_err(|error| {
        WorkError::new(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source":"stdin","byte_offset":error.valid_up_to()}),
        )
    })?;
    serde_json::from_str(text).map_err(|error| {
        WorkError::new(
            ExitCode::InputFormat,
            "invalid_json",
            "The input is not valid JSON.",
            json!({"line":error.line(),"column":error.column()}),
        )
    })
}

fn write_prepared_output(path: &str, value: &Value) -> Result<(), WorkError> {
    let raw = canonical_json(value).map_err(|_| {
        WorkError::new(
            ExitCode::Contract,
            "invalid_json_contract",
            "The prepared request cannot be rendered.",
            json!({}),
        )
    })?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "prepared_output_write_failed",
                "The prepared request output could not be created.",
                json!({"path":path}),
            )
        })?;
    file.write_all(&raw)
        .and_then(|_| file.sync_all())
        .map_err(|_| {
            WorkError::new(
                ExitCode::IoFailure,
                "prepared_output_write_failed",
                "The prepared request output could not be written.",
                json!({"path":path}),
            )
        })
}

fn skill_configs(values: &[String]) -> Result<Vec<SkillRootConfig>, WorkError> {
    values
        .iter()
        .map(|value| {
            let (identity, path) = value.split_once('=').ok_or_else(|| {
                WorkError::new(
                    ExitCode::CliUsage,
                    "invalid_skill_root_argument",
                    "Skill roots must use scope:locator=path syntax.",
                    json!({"value":value}),
                )
            })?;
            let (scope, locator) = identity.split_once(':').ok_or_else(|| {
                WorkError::new(
                    ExitCode::CliUsage,
                    "invalid_skill_root_argument",
                    "Skill roots must use scope:locator=path syntax.",
                    json!({"value":value}),
                )
            })?;
            if path.is_empty() || locator.is_empty() {
                return Err(WorkError::new(
                    ExitCode::CliUsage,
                    "invalid_skill_root_argument",
                    "Skill roots must use scope:locator=path syntax.",
                    json!({"value":value}),
                ));
            }
            Ok(SkillRootConfig {
                scope: scope.to_owned(),
                locator: locator.to_owned(),
                path: PathBuf::from(path),
            })
        })
        .collect()
}

fn skill_roots(configs: &[SkillRootConfig]) -> Vec<SkillRoot> {
    configs
        .iter()
        .map(|config| SkillRoot {
            scope: config.scope.clone(),
            locator: config.locator.clone(),
        })
        .collect()
}

fn unsigned_argument(parsed: &ParsedCommand, name: &str) -> Result<u64, WorkError> {
    argument(parsed, name)?.parse().map_err(|_| {
        WorkError::new(
            ExitCode::CliUsage,
            "cli_usage_error",
            "The CLI arguments are invalid.",
            json!({"argument":name}),
        )
    })
}

fn require_collection_path(path: &str) -> Result<(), WorkError> {
    if path.ends_with("/index.json") {
        Ok(())
    } else {
        Err(WorkError::new(
            ExitCode::WorkflowState,
            "task_collection_required",
            "TASK writes require a collection index.json artifact.",
            json!({}),
        ))
    }
}

fn specification_summary(result: &Value) -> Value {
    let source = result.get("preview").unwrap_or(result);
    let mut summary = json!({
        "schema":"work-specification-summary/v1",
        "status":source["status"],"record_id":source["record_id"],
        "approved_sha256":source["approved_sha256"],
        "affected_task_ids":source["affected_task_ids"],
        "changed_fields":source["changed_fields"],
        "file_readiness":source["file_readiness"],
    });
    if result.get("preview").is_some() {
        summary["output_file"] = result.get("output_file").cloned().unwrap_or(Value::Null);
        summary["transport"] = result["transport"].clone();
        summary["next_step"] = result["next_step"].clone();
    } else if let Some(next) = source.get("next_step") {
        summary["next_step"] = next.clone();
    }
    if let Some(verification) = source.get("verification_request") {
        summary["verification_request"] = verification.clone();
    }
    summary
}

fn dispatch(
    parsed: &ParsedCommand,
    root: &Path,
    input: Option<&FileInput>,
    skill_root: &Path,
) -> Result<Value, WorkError> {
    match parsed
        .path
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["migration", "analyze"] => analyze_artifact_migration(
            root,
            argument(parsed, "requirement_id")?,
            &string_list(parsed, "artifact"),
        ),
        ["migration", "prepare"] => {
            let request = input_json(input)?;
            if request["schema"] == "work-spec-migration-prepare-request/v1" {
                let raw = &required_input(input)?.raw;
                let configs = skill_configs(&string_list(parsed, "skill_root"))?;
                let output = parsed.arguments.get("output_file").and_then(Value::as_str);
                return work_flow::task::migration_prepare(
                    work_flow::task::MigrationPrepareInput {
                        raw,
                        request: &request,
                        date: &local_date(),
                        output,
                    },
                    |raw, date| prepare_revision_request(root, skill_root, &configs, raw, date),
                    |raw| prepare_reconstruction_request(root, skill_root, &configs, raw),
                    |prepared| preview_migration(root, skill_root, &configs, prepared),
                    write_prepared_output,
                );
            }
            if request["schema"] != "work-artifact-migration-decisions/v1" {
                return Err(WorkError::new(
                    ExitCode::Contract,
                    "migration_decisions_schema",
                    "A reviewed artifact migration decision input is required.",
                    json!({}),
                ));
            }
            prepare_artifact_migration(root, &request["analysis"], &request["choices"])
        }
        ["migration", "preview"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            if parsed.arguments.contains_key("input_file") {
                if parsed.arguments.contains_key("request_path") {
                    return Err(WorkError::new(
                        ExitCode::CliUsage,
                        "cli_usage_error",
                        "Choose one migration input.",
                        json!({}),
                    ));
                }
                let request = input_json(input)?;
                return work_flow::specification::migration(
                    "semantic-preview",
                    || preview_migration(root, skill_root, &configs, &request),
                    |_| unreachable!(),
                );
            }
            preview_artifact_migration(
                root,
                skill_root,
                &configs,
                argument(parsed, "request_path")?,
                argument(parsed, "approved_sha256")?,
            )
        }
        ["migration", "apply"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            if parsed.arguments.contains_key("input_file") {
                if parsed.arguments.contains_key("request_path") {
                    return Err(WorkError::new(
                        ExitCode::CliUsage,
                        "cli_usage_error",
                        "Choose one migration input.",
                        json!({}),
                    ));
                }
                let request = input_json(input)?;
                return work_flow::specification::migration(
                    "semantic-apply",
                    || preview_migration(root, skill_root, &configs, &request),
                    |operation| {
                        publish_migration(
                            root,
                            skill_root,
                            &configs,
                            &request,
                            operation,
                            argument(parsed, "approved_sha256")?,
                        )
                    },
                );
            }
            execute_artifact_migration(
                root,
                skill_root,
                &configs,
                argument(parsed, "request_path")?,
                argument(parsed, "approved_sha256")?,
            )
        }
        ["migration", "recover"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            if parsed.arguments.contains_key("input_file") {
                if parsed.arguments.contains_key("request_path") {
                    return Err(WorkError::new(
                        ExitCode::CliUsage,
                        "cli_usage_error",
                        "Choose one migration input.",
                        json!({}),
                    ));
                }
                let request = input_json(input)?;
                return work_flow::specification::migration(
                    "semantic-recover",
                    || preview_migration(root, skill_root, &configs, &request),
                    |operation| {
                        publish_migration(
                            root,
                            skill_root,
                            &configs,
                            &request,
                            operation,
                            argument(parsed, "approved_sha256")?,
                        )
                    },
                );
            }
            recover_artifact_migration(
                root,
                skill_root,
                &configs,
                argument(parsed, "request_path")?,
                argument(parsed, "approved_sha256")?,
            )
        }
        ["migration", "verify"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let approved = argument(parsed, "approved_sha256")?;
            match (
                parsed.arguments.contains_key("input_file"),
                parsed.arguments.get("request_path").and_then(Value::as_str),
            ) {
                (true, None) => verify_semantic_migration(
                    root,
                    skill_root,
                    &configs,
                    &input_json(input)?,
                    approved,
                ),
                (false, Some(request_path)) => {
                    verify_artifact_migration(root, skill_root, &configs, request_path, approved)
                }
                _ => Err(WorkError::new(
                    ExitCode::CliUsage,
                    "cli_usage_error",
                    "Choose exactly one Migration request input.",
                    json!({}),
                )),
            }
        }
        ["paths", "resolve"] => {
            let raw_id = argument(parsed, "requirement_id")?;
            resolve_artifact_paths(raw_id, root, |relative| {
                resolve_project_path(root, relative).map(|_| ())
            })
        }
        ["fingerprint", "text"] => {
            let raw_path = argument(parsed, "path")?;
            fingerprint_text(
                raw_path,
                |path| resolve_project_path(root, path),
                |path| {
                    fs::read(path).map_err(|error| {
                        if error.kind() == std::io::ErrorKind::NotFound || path.is_dir() {
                            WorkError::new(
                                ExitCode::ArtifactIntegrity,
                                "file_not_found",
                                "The required file does not exist or is not a regular file.",
                                json!({"path":path}),
                            )
                        } else {
                            WorkError::new(
                                ExitCode::IoFailure,
                                "file_read_failed",
                                "The file could not be read.",
                                json!({"path":path}),
                            )
                        }
                    })
                },
            )
        }
        ["hierarchy", "resolve"] => {
            let mode = argument(parsed, "work_directory")?;
            let selected = string_list(parsed, "paths");
            resolve_hierarchy_flow(mode, &selected, root)
        }
        ["hierarchy", "selection-build" | "selection-validate"] => {
            let input = input.ok_or_else(|| {
                WorkError::new(
                    ExitCode::InternalError,
                    "unreachable_command",
                    "The parsed command could not be dispatched.",
                    json!({}),
                )
            })?;
            let request = parse_json_contract(&input.raw).map_err(|_| {
                WorkError::new(
                    ExitCode::InputFormat,
                    "invalid_json_contract",
                    "The JSON input is invalid.",
                    json!({"source":input.source}),
                )
            })?;
            let catalog = LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            };
            if parsed.path[1] == "selection-build" {
                build_selection(&catalog, &request)
            } else {
                validate_selection(&catalog, &request)
            }
        }
        ["instructions", "catalog"] => instruction_catalog(
            &LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            },
            argument(parsed, "mode")?,
        ),
        ["instructions", "resolve"] => {
            let catalog = LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            };
            resolve_instructions(
                &catalog,
                argument(parsed, "mode")?,
                &string_list(parsed, "paths"),
                root,
            )
        }
        ["instructions", "load"] => {
            let catalog = LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            };
            load_instructions(
                &catalog,
                argument(parsed, "mode")?,
                &string_list(parsed, "paths"),
                &string_list(parsed, "reference"),
            )
        }
        ["instructions", "select"] => {
            let catalog = LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            };
            let mode = argument(parsed, "mode")?;
            select_instructions(
                &catalog,
                mode,
                &string_list(parsed, "paths"),
                &string_list(parsed, "reference"),
            )
        }
        ["instructions", "impact"] => {
            work_flow::instruction::impact(|| source_impact(root, skill_root))
        }
        ["instructions", "refresh-preview"] => work_flow::instruction::refresh_preview(|| {
            Ok(build_refresh(root, skill_root, argument(parsed, "requirement_id")?)?.preview)
        }),
        ["instructions", "refresh-apply" | "refresh-recover"] => {
            work_flow::instruction::refresh_apply(|| {
                apply_source_refresh(
                    root,
                    skill_root,
                    argument(parsed, "requirement_id")?,
                    argument(parsed, "approved_sha256")?,
                    if parsed.path[1] == "refresh-recover" {
                        "recover"
                    } else {
                        "apply"
                    },
                )
            })
        }
        ["instructions", "refresh-preview-all"] => {
            work_flow::instruction::refresh_preview_all(|| {
                preview_source_refresh_all(root, skill_root)
            })
        }
        ["instructions", "refresh-apply-all" | "refresh-recover-all"] => {
            work_flow::instruction::refresh_apply_all(|| {
                apply_source_refresh_all(
                    root,
                    skill_root,
                    argument(parsed, "approved_sha256")?,
                    if parsed.path[1] == "refresh-recover-all" {
                        "recover"
                    } else {
                        "apply"
                    },
                )
            })
        }
        ["instructions", "migration-preview"] => work_flow::instruction::migration_preview(|| {
            Ok(build_migration(root, skill_root, argument(parsed, "requirement_id")?)?.preview)
        }),
        ["instructions", "migration-apply"] => work_flow::instruction::migration_apply(|| {
            apply_migration(
                root,
                skill_root,
                argument(parsed, "requirement_id")?,
                argument(parsed, "approved_sha256")?,
            )
        }),
        ["skills", "catalog"] => skill_catalog(
            &LocalSkillCatalog {
                roots: skill_configs(&string_list(parsed, "root"))?,
            },
            &string_list(parsed, "disabled_source")
                .into_iter()
                .collect::<HashSet<_>>(),
            &HashSet::from(["work".to_owned()]),
        ),
        ["skills", "snapshot"] => {
            let configs = skill_configs(&[argument(parsed, "root")?.to_owned()])?;
            let scope = configs[0].scope.clone();
            let locator = configs[0].locator.clone();
            skill_snapshot(
                &LocalSkillCatalog { roots: configs },
                &scope,
                &locator,
                argument(parsed, "source")?,
            )
        }
        ["skills", "selection-build" | "selection-validate"] => {
            let configs = skill_configs(&string_list(parsed, "root"))?;
            let roots = skill_roots(&configs);
            let catalog = LocalSkillCatalog { roots: configs };
            if parsed.path[1] == "selection-build" {
                progress_contract_input(required_input(input)?)?;
                let request = input_json(input)?;
                build_skill_selection(&catalog, &roots, &request)
            } else {
                let request = skill_selection_validation_input(required_input(input)?)?;
                validate_skill_selection(&catalog, &roots, &request)
            }
        }
        ["workflow", "status" | "next"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let snapshot = load_workflow_snapshot(&WorkflowStateRequest {
                project_root: root,
                skill_root,
                skill_configs: &configs,
                requirement_id: argument(parsed, "requirement_id")?,
                plan_path: parsed.arguments.get("plan_path").and_then(Value::as_str),
            })?;
            let mut routing = RoutingSourceSession::new(skill_root.to_path_buf());
            let requirement_id = argument(parsed, "requirement_id")?;
            let maintenance = parsed.arguments["instruction_maintenance"] == true;
            let formal_events: &[&str] = if maintenance {
                &["instruction_maintenance"]
            } else {
                &[]
            };
            let state = if parsed.path[1] == "status" {
                work_flow::workflow::status_with_events(
                    &mut routing,
                    requirement_id,
                    snapshot,
                    formal_events,
                )
            } else {
                work_flow::workflow::next_with_events(
                    &mut routing,
                    requirement_id,
                    snapshot,
                    formal_events,
                )
            }?;
            let _: work_model::workflow::WorkflowState = serde_json::from_value(state.clone())
                .expect("routed workflow state matches its model");
            Ok(state)
        }
        ["attempt", "render"] => {
            let request = input_json(input)?;
            attempt_render(request)
        }
        ["attempt", "validate"] => {
            let (value, raw, path) = if let Some(input) = input {
                (input_json(Some(input))?, input.raw.clone(), None)
            } else {
                let (relative, file) = resolve_project_path(root, argument(parsed, "path")?)?;
                if Path::new(&relative)
                    .file_name()
                    .and_then(|name| name.to_str())
                    != Some("attempt.json")
                {
                    return Err(WorkError::new(
                        ExitCode::Contract,
                        "attempt_filename_mismatch",
                        "Attempt files must use <TASK-ID>/<ATTEMPT-ID>/attempt.json; legacy flat paths are unsupported.",
                        json!({}),
                    ));
                }
                let raw = fs::read(&file).map_err(|_| {
                    WorkError::new(
                        ExitCode::IoFailure,
                        "attempt_read_failed",
                        "The Attempt document could not be read.",
                        json!({"path":relative}),
                    )
                })?;
                let value = parse_json_contract(&raw).map_err(|_| {
                    WorkError::new(
                        ExitCode::InputFormat,
                        "invalid_json_contract",
                        "The JSON input is invalid.",
                        json!({"source":relative}),
                    )
                })?;
                (value, raw, Some(relative))
            };
            attempt_validate(
                &value,
                path.as_ref().map(|_| raw.as_slice()),
                path.as_deref(),
            )
        }
        ["correction", "render"] => {
            let request = input_json(input)?;
            correction_render(request)
        }
        ["correction", "validate"] => {
            let (request, path) = if input.is_some() {
                (input_json(input)?, None)
            } else {
                let (relative, file) = resolve_project_path(root, argument(parsed, "path")?)?;
                let raw = fs::read(&file).map_err(|_| {
                    WorkError::new(
                        ExitCode::IoFailure,
                        "correction_read_failed",
                        "The Correction document could not be read.",
                        json!({"path":relative}),
                    )
                })?;
                let value = parse_json_contract(&raw).map_err(|_| {
                    WorkError::new(
                        ExitCode::InputFormat,
                        "invalid_json_contract",
                        "The JSON input is invalid.",
                        json!({"source":relative}),
                    )
                })?;
                (value, Some(relative))
            };
            correction_validate(&request, path.as_deref())
        }
        ["progress", "read"] => read_progress(
            &LocalProgressStorage {
                project_root: root.to_path_buf(),
            },
            argument(parsed, "requirement_id")?,
            argument(parsed, "mode")?,
        ),
        ["progress", "prepare" | "validate" | "save"] => {
            let input = required_input(input)?;
            progress_contract_input(input)?;
            let storage = LocalProgressStorage {
                project_root: root.to_path_buf(),
            };
            let revision = unsigned_argument(parsed, "expected_revision")?;
            match parsed.path[1].as_str() {
                "prepare" => prepare_progress_raw(
                    &storage,
                    &input.raw,
                    argument(parsed, "requirement_id")?,
                    argument(parsed, "mode")?,
                    revision,
                ),
                "validate" => preview_progress_raw(&storage, &input.raw, revision),
                _ => save_progress_raw(
                    &storage,
                    &input.raw,
                    revision,
                    argument(parsed, "approved_sha256")?,
                ),
            }
        }
        ["plan", "semantic-prepare"] => {
            let request = input_json(input)?;
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let roots = skill_roots(&configs);
            let hierarchy = LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            };
            let skills = LocalSkillCatalog { roots: configs };
            let paths = LocalPlanStorage {
                project_root: root.to_path_buf(),
            };
            prepare_semantic(
                &hierarchy,
                &skills,
                &paths,
                &paths,
                &roots,
                &request,
                parsed.arguments.get("output_file").and_then(Value::as_str),
            )
        }
        ["plan", "validate"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let roots = skill_roots(&configs);
            let hierarchy = LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            };
            let skills = LocalSkillCatalog { roots: configs };
            let paths = LocalPlanStorage {
                project_root: root.to_path_buf(),
            };
            if let Some(input) = input {
                let plan_path = parsed
                    .arguments
                    .get("plan_path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        WorkError::new(
                            ExitCode::CliUsage,
                            "plan_path_required",
                            "--plan-path is required with --input-file.",
                            json!({}),
                        )
                    })?;
                let (plan_path, _) = resolve_project_path(root, plan_path)?;
                validate_plan_bytes(&hierarchy, &skills, &paths, &roots, &input.raw, &plan_path)
            } else {
                if parsed.arguments.contains_key("plan_path") {
                    return Err(WorkError::new(
                        ExitCode::CliUsage,
                        "unexpected_plan_path",
                        "--plan-path is only valid with --input-file.",
                        json!({}),
                    ));
                }
                let (plan_path, _) = resolve_project_path(root, argument(parsed, "path")?)?;
                validate_plan_file(&hierarchy, &skills, &paths, &roots, &plan_path)
            }
        }
        ["plan", "create"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let roots = skill_roots(&configs);
            let hierarchy = LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            };
            let skills = LocalSkillCatalog { roots: configs };
            let paths = LocalPlanStorage {
                project_root: root.to_path_buf(),
            };
            let (plan_path, _) = resolve_project_path(root, argument(parsed, "plan_path")?)?;
            create_plan_bytes(
                &hierarchy,
                &skills,
                &paths,
                &roots,
                &required_input(input)?.raw,
                &plan_path,
            )
        }
        ["delegation", "build" | "validate"] => {
            let request = input_json(input)?;
            let storage = LocalDelegationStorage {
                project_root: root.to_path_buf(),
                skill_root: skill_root.to_path_buf(),
                skill_configs: vec![],
            };
            let paths = LocalPlanStorage {
                project_root: root.to_path_buf(),
            };
            if parsed.path[1] == "build" {
                let result = build_delegation(&storage, &paths, &request)?;
                let _: work_model::delegation::DelegationBuildRequest =
                    serde_json::from_value(request)
                        .expect("built delegation request matches its model");
                Ok(result)
            } else {
                validate_delegation(
                    &storage,
                    &paths,
                    &request,
                    argument(parsed, "role")?,
                    argument(parsed, "sender")?,
                )
            }
        }
        ["handoff", command] => {
            let request = input_json(input)?;
            let paths = LocalPlanStorage {
                project_root: root.to_path_buf(),
            };
            work_flow::handoff::run(
                command,
                &paths,
                || {
                    Ok(LocalHandoffStorage {
                        project_root: root.to_path_buf(),
                        skill_root: skill_root.to_path_buf(),
                        skill_configs: skill_configs(&string_list(parsed, "skill_root"))?,
                    })
                },
                work_flow::handoff::HandoffArgs {
                    plan_path: parsed.arguments.get("plan_path").and_then(Value::as_str),
                    task_path: parsed.arguments.get("task_path").and_then(Value::as_str),
                    task_id: parsed.arguments.get("task_id").and_then(Value::as_str),
                    attempt_id: parsed.arguments.get("attempt_id").and_then(Value::as_str),
                    preflight: parsed.arguments.get("preflight") == Some(&json!(true)),
                },
                &request,
            )
        }
        ["invocation", "parse"] => {
            let input = required_input(input)?;
            parse_invocation_flow(&input.raw)
        }
        ["contract", "list"] => Ok(work_flow::contract::list(&contract::EmbeddedRegistry)),
        ["contract", "describe"] => work_flow::contract::describe(
            &contract::EmbeddedRegistry,
            argument(parsed, "contract_id")?,
        ),
        ["contract", "scaffold"] => work_flow::contract::scaffold(
            &contract::EmbeddedRegistry,
            argument(parsed, "contract_id")?,
        ),
        ["task", "status"] => {
            let storage = LocalTaskDraftStorage {
                project_root: root.to_path_buf(),
            };
            let mut result = work_flow::draft::status(|| {
                storage.status(
                    argument(parsed, "requirement_id")?,
                    parsed.arguments.get("task_id").and_then(Value::as_str),
                )
            })?;
            if result["status"] == "saved" {
                let configs = skill_configs(&string_list(parsed, "skill_root"))?;
                let (plan_path, _) = resolve_project_path(root, argument(parsed, "plan_path")?)?;
                let task_ids =
                    if let Some(id) = parsed.arguments.get("task_id").and_then(Value::as_str) {
                        vec![id.to_owned()]
                    } else {
                        result["tasks"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|row| row["id"].as_str().map(str::to_owned))
                            .collect()
                    };
                let revision = result["revision"].as_u64().unwrap_or(0);
                for task_id in task_ids {
                    match storage.check_sources(&TaskSourceCheckRequest {
                        requirement_id: argument(parsed, "requirement_id")?,
                        task_id: &task_id,
                        expected_revision: revision,
                        plan_path: &plan_path,
                        skill_root,
                        skill_configs: &configs,
                        selected_paths: None,
                        reference_names: None,
                    }) {
                        Ok(_) => result["source_validation"] = json!("valid"),
                        Err(error) if error.reason_code == "draft_source_drift" => {
                            result["source_validation"] = json!("review_required");
                            result["next_action"] = json!("review_sources");
                            result["requires_user_confirmation"] = json!(true);
                            break;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            Ok(result)
        }
        ["task", "prepare"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let request = input_json(input)?;
            if request.get("selections").is_some() {
                let (plan_path, _) = resolve_project_path(root, argument(parsed, "plan_path")?)?;
                let storage = LocalTaskDraftStorage {
                    project_root: root.to_path_buf(),
                };
                let revision = if parsed.arguments.contains_key("expected_revision") {
                    unsigned_argument(parsed, "expected_revision")?
                } else {
                    storage.read_planning_index(argument(parsed, "requirement_id")?)?["revision"]
                        .as_u64()
                        .ok_or_else(|| {
                            WorkError::new(
                                ExitCode::Contract,
                                "invalid_draft_revision",
                                "Planning revision is missing.",
                                json!({}),
                            )
                        })?
                };
                return storage.prepare_sources_from_project(TaskSourceUpdateProjectRequest {
                    requirement_id: argument(parsed, "requirement_id")?,
                    raw_request: &request,
                    expected_revision: revision,
                    plan_path: &plan_path,
                    skill_root,
                    skill_configs: &configs,
                    recover: false,
                });
            }
            work_flow::draft::semantic_prepare(|| {
                prepare_semantic_task_request(
                    root,
                    skill_root,
                    &configs,
                    argument(parsed, "requirement_id")?,
                    argument(parsed, "plan_path")?,
                    if parsed.arguments.contains_key("expected_revision") {
                        unsigned_argument(parsed, "expected_revision")?
                    } else {
                        LocalTaskDraftStorage {
                            project_root: root.to_path_buf(),
                        }
                        .status(argument(parsed, "requirement_id")?, None)?["revision"]
                            .as_u64()
                            .unwrap_or(0)
                    },
                    &request,
                )
            })
        }
        ["task", "save"] => {
            let request = input_json(input)?;
            if request["schema"] == "work-task-draft-prepare/v1" {
                if parsed.arguments.contains_key("task_id")
                    || parsed.arguments.get("general_only") == Some(&json!(true))
                    || parsed.arguments.contains_key("instruction_path")
                    || parsed.arguments.contains_key("reference")
                    || (parsed.arguments.contains_key("expected_revision")
                        && unsigned_argument(parsed, "expected_revision")? != 0)
                {
                    return Err(WorkError::new(
                        ExitCode::CliUsage,
                        "invalid_initial_task_save_options",
                        "Initial planning save accepts the prepared candidate without task selection or revision options.",
                        json!({}),
                    ));
                }
                let configs = skill_configs(&string_list(parsed, "skill_root"))?;
                let save = if request["request"]["selections"].is_object() {
                    save_prepared_source_task
                } else if request["index"]["revision"] == 1 {
                    save_prepared_initial_task
                } else {
                    save_prepared_list_task
                };
                return save(
                    root,
                    skill_root,
                    &configs,
                    argument(parsed, "requirement_id")?,
                    argument(parsed, "plan_path")?,
                    &request,
                );
            }
            if !parsed.arguments.contains_key("task_id") {
                return Err(WorkError::new(
                    ExitCode::CliUsage,
                    "task_id_required",
                    "--task-id is required when saving a TASK discussion.",
                    json!({}),
                ));
            }
            let explicit = parsed.arguments.get("general_only") == Some(&json!(true))
                || parsed.arguments.contains_key("instruction_path");
            work_flow::draft::with_source_selection(
                explicit,
                parsed.arguments.contains_key("reference"),
                || {
                    let selected = string_list(parsed, "instruction_path");
                    let references = string_list(parsed, "reference");
                    let configs = skill_configs(&string_list(parsed, "skill_root"))?;
                    let (plan_path, _) =
                        resolve_project_path(root, argument(parsed, "plan_path")?)?;
                    let storage = LocalTaskDraftStorage {
                        project_root: root.to_path_buf(),
                    };
                    let revision = if parsed.arguments.contains_key("expected_revision") {
                        unsigned_argument(parsed, "expected_revision")?
                    } else {
                        storage.read_planning_index(argument(parsed, "requirement_id")?)?["revision"]
                            .as_u64().ok_or_else(|| WorkError::new(ExitCode::Contract,
                                "invalid_draft_revision", "Planning revision is missing.", json!({})))?
                    };
                    storage.save_discussion_request(
                        &request,
                        &TaskSourceCheckRequest {
                            requirement_id: argument(parsed, "requirement_id")?,
                            task_id: argument(parsed, "task_id")?,
                            expected_revision: revision,
                            plan_path: &plan_path,
                            skill_root,
                            skill_configs: &configs,
                            selected_paths: explicit.then_some(selected.as_slice()),
                            reference_names: explicit.then_some(references.as_slice()),
                        },
                        false,
                    )
                },
            )
        }
        ["task", "validate"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let roots = skill_roots(&configs);
            let hierarchy = LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            };
            let skills = LocalSkillCatalog { roots: configs };
            let paths = LocalPlanStorage {
                project_root: root.to_path_buf(),
            };
            let raw_path = if input.is_some() {
                parsed
                    .arguments
                    .get("task_path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        WorkError::new(
                            ExitCode::CliUsage,
                            "task_path_required",
                            "--task-path is required with --input-file.",
                            json!({}),
                        )
                    })?
            } else {
                if parsed.arguments.contains_key("task_path") {
                    return Err(WorkError::new(
                        ExitCode::CliUsage,
                        "unexpected_task_path",
                        "--task-path is only valid with --input-file.",
                        json!({}),
                    ));
                }
                argument(parsed, "path")?
            };
            let (index_path, _) = resolve_project_path(root, raw_path)?;
            let repository = TaskInputOverlay {
                repository: LocalTaskStorage {
                    project_root: root.to_path_buf(),
                },
                index_path: &index_path,
                input_raw: input.map_or(&[], |input| input.raw.as_slice()),
            };
            if input.is_some() {
                validate_task_collection(
                    &hierarchy,
                    &skills,
                    &paths,
                    &repository,
                    &roots,
                    &index_path,
                )
            } else {
                validate_task_collection(
                    &hierarchy,
                    &skills,
                    &paths,
                    &repository.repository,
                    &roots,
                    &index_path,
                )
            }
        }
        ["task", "preview" | "apply" | "recover"] => {
            let metadata = input_json(input)?;
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let (plan_path, _) = resolve_project_path(root, argument(parsed, "plan_path")?)?;
            let request = ProjectAssemblyInput {
                requirement_id: argument(parsed, "requirement_id")?,
                metadata: &metadata,
                expected_revision: if parsed.arguments.contains_key("expected_revision") {
                    unsigned_argument(parsed, "expected_revision")?
                } else {
                    LocalTaskDraftStorage {
                        project_root: root.to_path_buf(),
                    }
                    .read_planning_index(argument(parsed, "requirement_id")?)?["revision"]
                        .as_u64()
                        .ok_or_else(|| {
                            WorkError::new(
                                ExitCode::Contract,
                                "invalid_draft_revision",
                                "Planning revision is missing.",
                                json!({}),
                            )
                        })?
                },
                plan_path: &plan_path,
            };
            let hierarchy = LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            };
            let skills = LocalSkillCatalog {
                roots: configs.clone(),
            };
            let paths = LocalPlanStorage {
                project_root: root.to_path_buf(),
            };
            let roots = skill_roots(&configs);
            let repository = LocalTaskAssembly {
                project_root: root.to_path_buf(),
            };
            if matches!(parsed.path[1].as_str(), "apply" | "recover") {
                create_draft_task(
                    DraftCreatePorts {
                        repository: &repository,
                        instructions: &hierarchy,
                        skills: &skills,
                        paths: &paths,
                        storage: &LocalTaskCreation {
                            project_root: root.to_path_buf(),
                        },
                        skill_roots: &roots,
                    },
                    request,
                    argument(parsed, "approved_sha256")?,
                    parsed.path[1] == "recover",
                )
            } else {
                assemble_task(&repository, &hierarchy, &skills, &paths, &roots, request)
            }
        }
        ["specification", "preview" | "apply" | "recover"] => {
            let input = required_input(input)?;
            let request = input_json(Some(input))?;
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            require_collection_path(request["plan"]["artifacts"]["task"].as_str().unwrap_or(""))?;
            let operation = match parsed.path[1].as_str() {
                "preview" => SpecOperation::Validate,
                "apply" => SpecOperation::Apply,
                _ => SpecOperation::Recover,
            };
            let result = work_flow::specification::update(|| {
                update_from_project(
                    root,
                    skill_root,
                    &configs,
                    SpecificationProjectRequest {
                        raw: &input.raw,
                        operation,
                        approved_sha256: parsed
                            .arguments
                            .get("approved_sha256")
                            .and_then(Value::as_str),
                    },
                )
            })?;
            if parsed.arguments.get("summary") == Some(&json!(true)) {
                Ok(specification_summary(&result))
            } else {
                Ok(result)
            }
        }
        ["specification", "verify"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let report =
                verify_from_project(root, skill_root, &configs, &required_input(input)?.raw)?;
            work_flow::specification::verify(report)
        }
        ["specification", "prepare"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let date = local_date();
            let output = parsed
                .arguments
                .get("output_file")
                .and_then(Value::as_str)
                .map(Path::new);
            let result = work_flow::specification::prepare(|| {
                prepare_simple_update(
                    root,
                    skill_root,
                    &configs,
                    SpecificationPrepareInput {
                        raw: &required_input(input)?.raw,
                        date: &date,
                        output_file: output,
                    },
                )
            })?;
            if parsed.arguments.get("summary") == Some(&json!(true)) {
                Ok(specification_summary(&result))
            } else {
                Ok(result)
            }
        }
        [
            "specification",
            "reconciliation-preview" | "reconciliation-apply",
        ] => {
            let request = input_json(input)?;
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            if parsed.path[1] == "reconciliation-preview" {
                preview_reconciliation(
                    &LocalReconciliationArtifacts { root },
                    &request,
                    |migration| preview_migration(root, skill_root, &configs, migration),
                )
            } else {
                apply_reconciliation(
                    &request,
                    argument(parsed, "approved_sha256")?,
                    |request, approval| {
                        publish_ledger_only(root, skill_root, &configs, request, approval)
                    },
                    |request, approval| {
                        publish_with_migration(root, skill_root, &configs, request, approval)
                    },
                )
            }
        }
        ["specification", "reconciliation-prepare"] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let mut prepared = prepare_reconciliation(
                &LocalSemanticReconciliation { root },
                &input_json(input)?,
                &local_date(),
                |raw, date| prepare_revision_request(root, skill_root, &configs, raw, date),
                |request| {
                    preview_reconciliation(
                        &LocalReconciliationArtifacts { root },
                        request,
                        |migration| preview_migration(root, skill_root, &configs, migration),
                    )
                },
            )?;
            if let Some(path) = parsed.arguments.get("output_file").and_then(Value::as_str) {
                write_prepared_output(path, &prepared["request"])?;
                prepared["output_file"] = json!(path);
            }
            Ok(prepared)
        }
        ["workspace", "create"] => work_flow::workspace::create(
            &LocalWorkspaceAllocator { project_root: root },
            parsed
                .arguments
                .get("requirement_id")
                .and_then(Value::as_str),
            argument(parsed, "workflow_id")?,
        ),
        [
            "execute",
            "preflight"
            | "worktree"
            | "attempt-start"
            | "attempt-start-prepare"
            | "recover-attempt-start"
            | "recovery-prepare"
            | "command-prepare"
            | "deviation-prepare-semantic"
            | "record-begin"
            | "record-finish"
            | "attempt-close"
            | "correction-create"
            | "command-correction"
            | "recover"
            | "command-run"
            | "deviation-record",
        ] => {
            let configs = skill_configs(&string_list(parsed, "skill_root"))?;
            let roots = skill_roots(&configs);
            let (task_path, _) = resolve_project_path(root, argument(parsed, "task_path")?)?;
            let (execution_dir, _) =
                resolve_project_path(root, argument(parsed, "execution_dir")?)?;
            require_no_spec_update(root, &execution_dir, None)?;
            let hierarchy = LocalHierarchyCatalog {
                skill_root: skill_root.to_path_buf(),
            };
            let skills = LocalSkillCatalog { roots: configs };
            let paths = LocalPlanStorage {
                project_root: root.to_path_buf(),
            };
            let tasks = LocalTaskStorage {
                project_root: root.to_path_buf(),
            };
            let execution = LocalExecutionStorage {
                project_root: root.to_path_buf(),
            };
            let sources = CommandProjectSources {
                instructions: &hierarchy,
                skills: &skills,
                paths: &paths,
                task_repository: &tasks,
                skill_roots: &roots,
            };
            let target = ExecutionProjectTarget {
                task_path: &task_path,
                execution_dir: &execution_dir,
                task_id: argument(parsed, "task_id")?,
            };
            let confirmed = string_list(parsed, "confirmed_input");
            work_flow::execution::run(
                work_flow::execution::ExecutionCommandInput {
                    command: &parsed.path[1],
                    target,
                    confirmed: &confirmed,
                    record_id: parsed.arguments.get("record_id").and_then(Value::as_str),
                    approved_sha256: parsed
                        .arguments
                        .get("approved_sha256")
                        .and_then(Value::as_str),
                    authorization_evidence: parsed
                        .arguments
                        .get("authorization_evidence")
                        .and_then(Value::as_str),
                },
                &sources,
                &execution,
                || input_json(input),
                local_timestamp,
                work_flow::execution::ExecutionTechnical {
                    recover_start: |request: Value, started_at: String| {
                        execution.recover_attempt_start_from_project(
                            &sources,
                            AttemptStartRecoveryRequest {
                                target,
                                request: &request,
                                confirmed_inputs: &confirmed,
                                started_at: &started_at,
                            },
                        )
                    },
                    recover: |request: Value| {
                        execution.recover_execution_from_project(&sources, target, &request)
                    },
                    command_run: |request: Value, approved_sha256: String| {
                        execution.run_command_from_project(
                            &sources,
                            CommandProjectRequest {
                                task_path: &task_path,
                                execution_dir: &execution_dir,
                                task_id: target.task_id,
                                request: &request,
                            },
                            &approved_sha256,
                        )
                    },
                },
            )
        }
        _ => Err(WorkError::new(
            ExitCode::InternalError,
            "unreachable_command",
            "The parsed command could not be dispatched.",
            json!({}),
        )),
    }
}

fn brief_field(name: &str) -> bool {
    const FIELDS: &[&str] = &[
        "schema",
        "status",
        "next_action",
        "command",
        "arguments",
        "semantic_input_contract",
        "requires_user_confirmation",
        "confirmation_required",
        "recovery_required",
        "required_checks",
        "routing_status",
        "router_compatibility_revision",
        "required_instruction_sources",
        "source_order",
        "routing_reasons",
        "selection_manifest",
        "path",
        "paths",
        "artifact",
        "artifacts",
        "target",
        "targets",
    ];
    FIELDS.contains(&name)
        || ["_id", "_ids", "_sha256", "_path", "_paths"]
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

fn brief(value: &Value) -> Value {
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let mut projected = serde_json::Map::new();
    for (key, item) in object {
        if brief_field(key) {
            projected.insert(key.clone(), item.clone());
        } else if item.is_object() {
            let nested = brief(item);
            if nested.as_object().is_some_and(|value| !value.is_empty()) {
                projected.insert(key.clone(), nested);
            }
        }
    }
    Value::Object(projected)
}

pub fn run(tokens: &[String]) -> (i32, CliResult) {
    let skill_root = std::env::current_exe()
        .ok()
        .and_then(|binary| binary.parent()?.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    run_with_skill_root(tokens, &skill_root)
}

pub fn run_with_skill_root(tokens: &[String], skill_root: &Path) -> (i32, CliResult) {
    let result = (|| -> Result<CliResult, WorkError> {
        let parsed = match parse_tokens(tokens)? {
            ParseOutcome::Help(help) => return Ok(CliResult::success(json!({"help":help}), false)),
            ParseOutcome::Command(command) => command,
        };
        let root = root_path(argument(&parsed, "project_root")?)?;
        let input = parsed
            .arguments
            .get("input_file")
            .and_then(Value::as_str)
            .map(read_input_file)
            .transpose()?;
        let result = dispatch_with_operation_context(&parsed, &root, input.as_ref(), skill_root)?;
        let verbose = parsed.arguments.get("verbose") == Some(&json!(true));
        let completed = result["status"] == "already_completed";
        let full_evidence = parsed.path.last().is_some_and(|operation| {
            operation.contains("recover") || operation.contains("recovery")
        }) || matches!(
            result["status"].as_str(),
            Some("interrupted" | "recovered" | "recovery_required" | "already_completed")
        );
        let mut response = CliResult::success(
            if verbose || full_evidence {
                result
            } else {
                brief(&result)
            },
            completed,
        );
        if parsed.path == ["contract", "scaffold"] {
            response.order_hint = parsed
                .arguments
                .get("contract_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
        } else if parsed.path == ["attempt", "render"] {
            response.order_hint = Some("attempt-render".to_owned());
        } else if parsed.path == ["correction", "render"] {
            response.order_hint = Some("correction-render".to_owned());
        }
        Ok(response)
    })();
    match result {
        Ok(value) => (ExitCode::Success as i32, value),
        Err(error) => (error.exit_code as i32, CliResult::from_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn operation_context_binds_project_paths_and_rechecks_pre_read_input() {
        let root = std::env::temp_dir().join(format!(
            "work-operation-binding-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let task_path = "outputs/work/tasks/example/index.json";
        let task = root.join(task_path);
        fs::create_dir_all(task.parent().unwrap()).unwrap();
        fs::write(&task, b"task\n").unwrap();
        let input_path = root.join("request.json");
        fs::write(&input_path, b"{}\n").unwrap();
        let input = read_input_file(input_path.to_str().unwrap()).unwrap();
        let parsed = ParsedCommand {
            path: vec!["execute".into(), "deviation-record".into()],
            arguments: BTreeMap::from([
                ("task_path".into(), json!(task_path)),
                (
                    "execution_dir".into(),
                    json!("outputs/work/executions/example"),
                ),
                ("input_file".into(), json!(input_path)),
            ]),
        };
        let prepared = operation_artifacts(&parsed, &root, Some(&input), true).unwrap();
        assert_eq!(
            prepared["task_path"]["path"],
            task.to_string_lossy().as_ref()
        );
        assert_eq!(
            prepared["input_file"]["path"],
            input_path.to_string_lossy().as_ref()
        );
        assert_eq!(
            prepared["input_file"]["raw_sha256"],
            input.source_raw_sha256
        );
        let mut routing = RoutingSourceSession::new(PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../skills/work"
        )));
        let (envelope, selection) = build_operation_context(
            &mut routing,
            &OperationContextRequest {
                command: "execute",
                operation: "deviation-record",
                delegated_role: None,
                artifacts: &prepared,
                project_root: root.to_str().unwrap(),
                approval_sha256: Some(&"a".repeat(64)),
                transaction_workspace: Some("outputs/work/executions/example"),
            },
        )
        .unwrap();
        let unchanged = operation_artifacts(&parsed, &root, Some(&input), false).unwrap();
        validate_operation_context(&envelope, &selection, &unchanged).unwrap();
        fs::write(&input_path, b"{\"changed\":true}\n").unwrap();
        let drifted = operation_artifacts(&parsed, &root, Some(&input), false).unwrap();
        assert_eq!(
            validate_operation_context(&envelope, &selection, &drifted)
                .unwrap_err()
                .reason_code,
            "operation_artifact_drift"
        );
    }

    #[test]
    fn root_resolution_requires_existing_directory() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        assert_eq!(
            root_path(repo.to_str().unwrap()).unwrap(),
            repo.canonicalize().unwrap()
        );
        let file = repo.join("Cargo.toml");
        let error = root_path(file.to_str().unwrap()).unwrap_err();
        assert_eq!(error.reason_code, "root_not_directory");
        assert_eq!(error.exit_code, ExitCode::IoFailure);
    }

    #[test]
    fn record_begin_rejects_invalid_id_before_loading_missing_artifacts() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = repo.to_string_lossy().into_owned();
        let args = [
            "--project-root".to_owned(),
            root.clone(),
            "execute".into(),
            "record-begin".into(),
            "--user-config-root".into(),
            root,
            "--task-path".into(),
            "outputs/work/tasks/missing/index.json".into(),
            "--execution-dir".into(),
            "outputs/work/executions/missing".into(),
            "--task-id".into(),
            "TASK-001".into(),
            "--record-id".into(),
            "CMD-1".into(),
        ];
        let (exit, result) = run_with_skill_root(&args, &repo.join("../skills/work"));
        assert_eq!(exit, ExitCode::Contract as i32);
        assert_eq!(result.reason_code, "record_begin_invalid_base_record_id");
        assert_eq!(result.data["record_id"], "CMD-1");
    }

    #[test]
    fn command_run_requires_cli_approval_before_reading_request_or_sources() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = repo.to_string_lossy().into_owned();
        let arguments = vec![
            "--project-root".into(),
            root.clone(),
            "execute".into(),
            "command-run".into(),
            "--user-config-root".into(),
            root,
            "--task-path".into(),
            "outputs/work/tasks/missing/index.json".into(),
            "--execution-dir".into(),
            "outputs/work/executions/missing".into(),
            "--task-id".into(),
            "TASK-001".into(),
            "--input-file".into(),
            "missing-request.json".into(),
        ];
        let (exit, response) = run_with_skill_root(&arguments, &repo.join("../skills/work"));
        assert_eq!(exit, ExitCode::CliUsage as i32);
        assert_eq!(response.reason_code, "cli_usage_error");
        assert!(
            response.data["reason"]
                .as_str()
                .unwrap()
                .contains("--approved-sha256")
        );
    }

    #[test]
    fn execute_preflight_missing_task_keeps_public_io_error() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let project = std::env::temp_dir().join(format!(
            "work-execute-missing-task-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&project).unwrap();
        let root = project.to_string_lossy().into_owned();
        let args = [
            "--project-root".to_owned(),
            root.clone(),
            "execute".into(),
            "preflight".into(),
            "--user-config-root".into(),
            root,
            "--task-path".into(),
            "outputs/work/tasks/missing/task.json".into(),
            "--execution-dir".into(),
            "outputs/work/executions/example".into(),
            "--task-id".into(),
            "TASK-001".into(),
        ];
        let (exit, result) = run_with_skill_root(&args, &repo.join("../skills/work"));
        assert_eq!(exit, ExitCode::IoFailure as i32);
        assert_eq!(result.schema, "work-cli-result/v1");
        assert_eq!(result.reason_code, "execute_preflight_task_missing");
    }

    #[test]
    fn delegation_cli_build_and_validate_keep_formal_context_read_only() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let project = repo.join("crates/work-infrastructure/fixtures/task-diagnostics");
        let skill_root = repo.join("../skills/work");
        let plan = project.join("outputs/work/plans/example.json");
        let before = fs::read(&plan).unwrap();
        let input = std::env::temp_dir().join(format!(
            "work-delegation-cli-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let request = json!({"schema":"work-delegation-build-request/v1",
            "role":"task-coordinator","request":"Coordinate the confirmed TASK work.",
            "source_plan_path":"outputs/work/plans/example.json"});
        fs::write(&input, serde_json::to_vec(&request).unwrap()).unwrap();
        let base = [
            "--project-root".to_owned(),
            project.to_string_lossy().into_owned(),
            "--verbose".to_owned(),
        ];
        let (exit, built) = run_with_skill_root(
            &base
                .iter()
                .cloned()
                .chain([
                    "delegation".into(),
                    "build".into(),
                    "--input-file".into(),
                    input.to_string_lossy().into_owned(),
                ])
                .collect::<Vec<_>>(),
            &skill_root,
        );
        assert_eq!(exit, 0);
        assert_eq!(built.data["sender"], "parent");
        assert_eq!(built.data["marker"], "WORK_DELEGATION_V1");
        assert_eq!(
            built.data["context"]["source_plan"],
            serde_json::from_slice::<Value>(&before).unwrap()
        );
        assert!(built.data.get("authorized").is_none());
        fs::write(&input, serde_json::to_vec(&built.data).unwrap()).unwrap();
        let validate = |role: &str| {
            run_with_skill_root(
                &base
                    .iter()
                    .cloned()
                    .chain([
                        "delegation".into(),
                        "validate".into(),
                        "--role".into(),
                        role.into(),
                        "--sender".into(),
                        "parent".into(),
                        "--input-file".into(),
                        input.to_string_lossy().into_owned(),
                    ])
                    .collect::<Vec<_>>(),
                &skill_root,
            )
        };
        let (valid_exit, valid) = validate("task-coordinator");
        assert_eq!(valid_exit, 0);
        assert_eq!(valid.data["grants_authorization"], false);
        let (wrong_exit, wrong) = validate("execute");
        assert_ne!(wrong_exit, 0);
        assert_eq!(wrong.reason_code, "delegation_boundary_mismatch");
        assert_eq!(fs::read(plan).unwrap(), before);
    }

    #[test]
    fn cli_file_transport_preserves_source_hash_and_rejects_invalid_inputs() {
        let base = std::env::temp_dir().join(format!(
            "work-cli-file-transport-t25-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = base.join("專案 workspace");
        fs::create_dir_all(&project).unwrap();
        let input = base.join("request.json");
        let path = input.to_string_lossy().into_owned();
        let prefix = [
            "--project-root".to_owned(),
            project.to_string_lossy().into_owned(),
        ];
        let command = ["attempt", "render", "--input-file", path.as_str()];
        let invoke = || {
            run_with_skill_root(
                &prefix
                    .iter()
                    .cloned()
                    .chain(command.map(str::to_owned))
                    .collect::<Vec<_>>(),
                &base,
            )
        };

        let raw = b"\xef\xbb\xbf{}";
        fs::write(&input, raw).unwrap();
        let decoded = read_input_file(&path).unwrap();
        assert_eq!(decoded.raw, b"{}");
        assert_eq!(decoded.source_raw_sha256, fingerprint::raw(raw));
        for (raw, reason) in [
            (b"\xff".as_slice(), "invalid_utf8"),
            (b"\xff\xfe{\0}\0".as_slice(), "invalid_utf8"),
            (
                b"\xef\xbb\xbf\xef\xbb\xbf{}".as_slice(),
                "input_file_multiple_bom",
            ),
            (b"".as_slice(), "invalid_json_contract"),
            (b"{".as_slice(), "invalid_json_contract"),
            (b"{\"x\":1,\"x\":2}".as_slice(), "duplicate_json_key"),
        ] {
            fs::write(&input, raw).unwrap();
            let (exit, response) = invoke();
            assert_eq!(exit, ExitCode::InputFormat as i32, "{reason}");
            assert_eq!(response.reason_code, reason);
            assert_eq!(fs::read_dir(&project).unwrap().count(), 0);
        }
        let selection = b"{\"decision\":\"general_only\",\"selections\":[]}";
        let selection_args = prefix
            .iter()
            .cloned()
            .chain([
                "hierarchy".into(),
                "selection-build".into(),
                "--input-file".into(),
                path.clone(),
            ])
            .collect::<Vec<_>>();
        let skill_root =
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        fs::write(&input, selection).unwrap();
        let (plain_exit, plain) = run_with_skill_root(&selection_args, &skill_root);
        fs::write(&input, [b"\xef\xbb\xbf".as_slice(), selection].concat()).unwrap();
        let (bom_exit, bom) = run_with_skill_root(&selection_args, &skill_root);
        assert_eq!((plain_exit, bom_exit), (0, 0));
        assert_eq!(plain.reason_code, bom.reason_code);
        assert_eq!(plain.data, bom.data);

        for unreadable in [base.join("missing.json"), project.clone()] {
            let mut tokens = prefix.to_vec();
            tokens.extend(["attempt".into(), "render".into(), "--input-file".into()]);
            tokens.push(unreadable.to_string_lossy().into_owned());
            let (exit, response) = run_with_skill_root(&tokens, &base);
            assert_eq!(exit, ExitCode::IoFailure as i32);
            assert_eq!(response.reason_code, "input_file_read_failed");
            assert_eq!(fs::read_dir(&project).unwrap().count(), 0);
        }
    }

    #[test]
    fn cli_paths_result_keeps_data_and_verbose_projection() {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
            .canonicalize()
            .unwrap();
        let command = ["paths", "resolve", "--requirement-id", "example"];
        let prefix = [
            "--project-root".to_owned(),
            root.to_string_lossy().into_owned(),
        ];
        let (brief_exit, brief) = run_with_skill_root(
            &prefix
                .iter()
                .cloned()
                .chain(command.map(str::to_owned))
                .collect::<Vec<_>>(),
            &root.join("../skills/work"),
        );
        let (verbose_exit, verbose) = run_with_skill_root(
            &prefix
                .iter()
                .cloned()
                .chain(["--verbose".to_owned()])
                .chain(command.map(str::to_owned))
                .collect::<Vec<_>>(),
            &root.join("../skills/work"),
        );
        assert_eq!((brief_exit, verbose_exit), (0, 0));
        assert_eq!(
            (brief.schema, brief.reason_code.as_str()),
            ("work-cli-result/v1", "ok")
        );
        assert_eq!(brief.data["schema"], "work-paths/v1");
        assert_eq!(brief.data["paths"], verbose.data["paths"]);
        assert_eq!(brief.data["requirement_id"], "example");
        assert_eq!(brief.data.as_object().unwrap().len(), 3);
        assert_eq!(
            verbose.data["project_root"],
            root.to_string_lossy().as_ref()
        );
    }

    #[test]
    fn specification_summary_omits_complete_candidate_documents() {
        let preview = json!({
            "schema":"work-spec-update/v1",
            "status":"valid",
            "record_id":"SPEC-UPDATE-002",
            "approved_sha256":"a".repeat(64),
            "affected_task_ids":["TASK-001"],
            "changed_fields":["/tasks/TASK-001/goal"],
            "file_readiness":"requires_execute_preflight",
            "candidate":{"plan":{},"task":{},"index":{}}
        });
        let result = json!({
            "schema":"work-spec-prepare/v1",
            "request":{},
            "preview":preview,
            "output_file":"prepared.json",
            "transport":{"request_field":"request"},
            "next_step":{"command":"specification preview","input":"request"}
        });
        assert_eq!(
            specification_summary(&result),
            json!({
                "schema":"work-specification-summary/v1",
                "status":"valid",
                "record_id":"SPEC-UPDATE-002",
                "approved_sha256":"a".repeat(64),
                "affected_task_ids":["TASK-001"],
                "changed_fields":["/tasks/TASK-001/goal"],
                "file_readiness":"requires_execute_preflight",
                "output_file":"prepared.json",
                "transport":{"request_field":"request"},
                "next_step":{"command":"specification preview","input":"request"}
            })
        );
    }

    #[test]
    fn task_validate_cli_rejects_invalid_task_path_combinations() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let base = std::env::temp_dir().join(format!(
            "work-cli-task-validate-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&base).unwrap();
        let input = base.join("request.json");
        fs::write(&input, "{}").unwrap();
        let prefix = [
            "--project-root".to_owned(),
            base.to_string_lossy().into_owned(),
            "task".to_owned(),
            "validate".to_owned(),
            "--user-config-root".to_owned(),
            base.to_string_lossy().into_owned(),
        ];
        let invoke = |suffix: Vec<String>| {
            run_with_skill_root(
                &[prefix.as_slice(), suffix.as_slice()].concat(),
                &repo.join("../skills/work"),
            )
        };
        let (exit, required) = invoke(vec![
            "--input-file".into(),
            input.to_string_lossy().into_owned(),
        ]);
        assert_eq!(exit, ExitCode::CliUsage as i32);
        assert_eq!(required.schema, "work-cli-result/v1");
        assert_eq!(required.reason_code, "task_path_required");
        let (exit, unexpected) = invoke(vec![
            "--path".into(),
            "outputs/work/tasks/example/task.json".into(),
            "--task-path".into(),
            "outputs/work/tasks/other/task.json".into(),
        ]);
        assert_eq!(exit, ExitCode::CliUsage as i32);
        assert_eq!(unexpected.schema, "work-cli-result/v1");
        assert_eq!(unexpected.reason_code, "unexpected_task_path");
    }

    #[test]
    fn task_save_requires_explicit_paths_for_references() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = std::env::temp_dir().join(format!(
            "work-cli-task-draft-reference-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let input = root.join("request.json");
        fs::write(&input, "{}").unwrap();
        let command = "save";
        let mut args = vec![
            "--project-root".into(),
            root.to_string_lossy().into_owned(),
            "task".into(),
            command.into(),
            "--requirement-id".into(),
            "example".into(),
            "--task-id".into(),
            "TASK-001".into(),
            "--expected-revision".into(),
            "1".into(),
            "--plan-path".into(),
            "outputs/work/plans/example.json".into(),
            "--user-config-root".into(),
            root.to_string_lossy().into_owned(),
            "--reference".into(),
            "task.general.task-records".into(),
        ];
        args.extend(["--input-file".into(), input.to_string_lossy().into_owned()]);
        let (exit, result) = run_with_skill_root(&args, &repo.join("../skills/work"));
        assert_eq!(exit, ExitCode::CliUsage as i32, "{command}");
        assert_eq!(
            result.reason_code, "draft_selection_incomplete",
            "{command}"
        );
    }

    #[test]
    fn skill_catalog_and_snapshot_cli_match_python_cases() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let base = std::env::temp_dir().join(format!(
            "work-cli-skills-catalog-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = base.join("project");
        let skill = base.join("skills/frontend");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: frontend\ndescription: Build frontends.\n---\nInstructions\n",
        )
        .unwrap();
        let prefix = [
            "--project-root".to_owned(),
            project.to_string_lossy().into_owned(),
            "--verbose".to_owned(),
            "skills".to_owned(),
        ];
        let root = format!("repo:.agents/skills={}", base.join("skills").display());
        let invoke = |suffix: Vec<String>| {
            run_with_skill_root(
                &[prefix.as_slice(), suffix.as_slice()].concat(),
                &repo.join("../skills/work"),
            )
        };
        let (exit, catalog) = invoke(vec!["catalog".into(), "--root".into(), root.clone()]);
        assert_eq!(exit, 0);
        assert_eq!(catalog.data["schema"], "work-skill-catalog/v1");
        assert_eq!(catalog.data["skills"][0]["name"], "frontend");
        let (exit, snapshot) = invoke(vec![
            "snapshot".into(),
            "--root".into(),
            root,
            "--source".into(),
            "frontend/SKILL.md".into(),
        ]);
        assert_eq!(exit, 0);
        assert_eq!(snapshot.data["schema"], "work-skill-snapshot/v1");
        assert_eq!(
            snapshot.data["bundle"]["bundle_sha256"]
                .as_str()
                .unwrap()
                .len(),
            64
        );
        let (exit, invalid) = invoke(vec!["catalog".into(), "--root".into(), "repo".into()]);
        assert_eq!(exit, ExitCode::CliUsage as i32);
        assert_eq!(invalid.schema, "work-cli-result/v1");
        assert_eq!(invalid.reason_code, "invalid_skill_root_argument");
        assert_eq!(fs::read_dir(project).unwrap().count(), 0);
    }

    #[test]
    fn skill_selection_cli_preserves_build_and_validation_parsing() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let base = std::env::temp_dir().join(format!(
            "work-cli-skills-selection-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = base.join("project");
        fs::create_dir_all(&project).unwrap();
        let request = base.join("request.json");
        let prefix = [
            "--project-root".to_owned(),
            project.to_string_lossy().into_owned(),
            "--verbose".to_owned(),
            "skills".to_owned(),
        ];
        let invoke = |operation: &str| {
            run_with_skill_root(
                &[
                    prefix.as_slice(),
                    &[
                        operation.into(),
                        "--root".into(),
                        format!("repo:.agents/skills={}", base.display()),
                        "--input-file".into(),
                        request.to_string_lossy().into_owned(),
                    ],
                ]
                .concat(),
                &repo.join("../skills/work"),
            )
        };
        fs::write(&request, r#"{"decision":"base_only","skills":[]}"#).unwrap();
        let (exit, built) = invoke("selection-build");
        assert_eq!(exit, 0);
        assert_eq!(built.data["schema"], "work-skill-selection/v1");
        assert_eq!(
            built.data["selection_sha256"],
            work_infrastructure::fixture_support::selection_sha256("base_only", &[])
        );
        fs::write(&request, serde_json::to_vec(&built.data).unwrap()).unwrap();
        let (exit, validated) = invoke("selection-validate");
        assert_eq!(exit, 0);
        assert_eq!(
            validated.data["schema"],
            "work-skill-selection-validation/v1"
        );
        assert_eq!(validated.data["status"], "valid");
        fs::write(
            &request,
            r#"{"decision":"base_only","decision":"external_skills","skills":[]}"#,
        )
        .unwrap();
        let (exit, duplicate) = invoke("selection-build");
        assert_eq!(exit, ExitCode::InputFormat as i32);
        assert_eq!(duplicate.reason_code, "duplicate_json_key");
        fs::write(&request, "{").unwrap();
        let (exit, malformed) = invoke("selection-validate");
        assert_eq!(exit, ExitCode::InputFormat as i32);
        assert_eq!(malformed.reason_code, "invalid_json");
        assert_eq!(fs::read_dir(project).unwrap().count(), 0);
    }

    #[test]
    fn attempt_render_invalid_json_keeps_input_format_envelope() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let input = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/invalid_json.txt");
        let (exit, response) = run_with_skill_root(
            &[
                "--project-root".into(),
                repo.to_string_lossy().into_owned(),
                "attempt".into(),
                "render".into(),
                "--input-file".into(),
                input.into(),
            ],
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, ExitCode::InputFormat as i32);
        assert_eq!(response.schema, "work-cli-result/v1");
        assert_eq!(response.reason_code, "invalid_json_contract");
    }

    #[test]
    fn correction_render_invalid_json_keeps_input_format_envelope() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let input = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/invalid_json.txt");
        let (exit, response) = run_with_skill_root(
            &[
                "--project-root".into(),
                repo.to_string_lossy().into_owned(),
                "correction".into(),
                "render".into(),
                "--input-file".into(),
                input.into(),
            ],
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, ExitCode::InputFormat as i32);
        assert_eq!(response.schema, "work-cli-result/v1");
        assert_eq!(response.reason_code, "invalid_json_contract");
    }

    #[test]
    fn contract_commands_dispatch_all_public_catalog_cases() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let skill_root = repo.join("../skills/work");
        let prefix = [
            "--project-root".to_owned(),
            repo.to_string_lossy().into_owned(),
            "--verbose".to_owned(),
            "contract".to_owned(),
        ];
        let invoke = |args: &[&str]| {
            let tokens = [
                prefix.as_slice(),
                &args
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect::<Vec<_>>(),
            ]
            .concat();
            run_with_skill_root(&tokens, &skill_root)
        };
        let (exit, catalog) = invoke(&["list"]);
        assert_eq!(exit, 0);
        assert_eq!(catalog.data["schema"], "work-contract-catalog/v1");
        let ids: Vec<_> = catalog.data["contracts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"work-plan-semantic-request/v1"));
        assert!(!ids.contains(&"work-plan-prepare-request/v1"));
        let (exit, description) = invoke(&["describe", "work-contract-catalog/v1"]);
        assert_eq!(exit, 0);
        assert_eq!(description.data["schema"], "work-contract-description/v1");
        assert_eq!(description.data["id"], "work-contract-catalog/v1");
        let (exit, unknown) = invoke(&["describe", "work-unknown/v1"]);
        assert_eq!(exit, ExitCode::Contract as i32);
        assert_eq!(unknown.reason_code, "unknown_contract_id");
        let (exit, record) = invoke(&["scaffold", "work-record-finish-request/v1"]);
        assert_eq!(exit, 0);
        assert_eq!(record.data["schema"], "work-contract-scaffold/v1");
        assert_eq!(
            record.data["canonical_order"],
            json!([
                "schema",
                "record",
                "modified_files",
                "authorization_evidence"
            ])
        );
        let (exit, nonrequest) = invoke(&["scaffold", "work-contract-catalog/v1"]);
        assert_eq!(exit, ExitCode::Contract as i32);
        assert_eq!(nonrequest.reason_code, "contract_scaffold_requires_request");
        let (exit, generated) = invoke(&["scaffold", "work-spec-update-request/v1"]);
        assert_eq!(exit, ExitCode::Contract as i32);
        assert_eq!(
            generated.reason_code,
            "generated_request_not_caller_constructible"
        );
        let (exit, plan) = invoke(&["scaffold", "work-plan-semantic-request/v1"]);
        assert_eq!(exit, 0);
        assert_eq!(
            plan.data["scaffold"]["hierarchy_selection_request"],
            json!({"decision":"instruction_paths","selections":[]})
        );
        assert_eq!(
            plan.data["scaffold"]["skill_selection_request"],
            json!({"decision":"external_skills","skills":[]})
        );
        assert!(plan.data["scaffold"].get("content").is_none());
        let correction = "work-command-correction-request/v1";
        let (exit, description) = invoke(&["describe", correction]);
        assert_eq!(exit, 0);
        let command = description.data["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["name"] == "actual_command")
            .unwrap();
        assert_eq!(
            command["constraints"]["schema"]["discriminator"]["propertyName"],
            "mode"
        );
        assert_eq!(
            command["constraints"]["schema"]["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let (exit, scaffold) = invoke(&["scaffold", correction]);
        assert_eq!(exit, 0);
        assert_eq!(
            scaffold.data["scaffold"]["actual_command"],
            json!({"mode":"argv","argv":["tool"]})
        );
    }

    #[test]
    fn invocation_parse_preserves_literal_request_without_project_effects() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let base = std::env::temp_dir().join(format!(
            "work-cli-invocation-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = base.join("project");
        fs::create_dir_all(&root).unwrap();
        let input = base.join("invocation.txt");
        let raw = "$work execute -- $(touch should-not-exist) -- \"literal\"\n";
        fs::write(&input, raw).unwrap();
        let arguments = [
            "--project-root".into(),
            root.to_string_lossy().into_owned(),
            "--verbose".into(),
            "invocation".into(),
            "parse".into(),
            "--input-file".into(),
            input.to_string_lossy().into_owned(),
        ];
        let (exit, result) = run_with_skill_root(&arguments, &repo.join("../skills/work"));
        assert_eq!(exit, 0);
        assert_eq!(result.data["request"], raw.split_once("--").unwrap().1);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::write(&input, "$work -- request").unwrap();
        let (exit, result) = run_with_skill_root(&arguments, &repo.join("../skills/work"));
        assert_ne!(exit, 0);
        assert_eq!(result.reason_code, "work_invocation_mode_missing");
    }

    #[test]
    fn invocation_input_decoding_accepts_one_bom_and_rejects_invalid_utf8() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let base = std::env::temp_dir().join(format!(
            "work-cli-invocation-encoding-t25-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = base.join("project");
        fs::create_dir_all(&root).unwrap();
        let input = base.join("invocation.txt");
        let arguments = [
            "--project-root".into(),
            root.to_string_lossy().into_owned(),
            "--verbose".into(),
            "invocation".into(),
            "parse".into(),
            "--input-file".into(),
            input.to_string_lossy().into_owned(),
        ];
        fs::write(&input, "\u{feff} \t$work\tplan\n--\n需求").unwrap();
        let (exit, result) = run_with_skill_root(&arguments, &repo.join("../skills/work"));
        assert_eq!(exit, 0);
        assert_eq!(result.data["request"], "\n需求");
        fs::write(&input, b"$work plan -- \xff").unwrap();
        let (exit, result) = run_with_skill_root(&arguments, &repo.join("../skills/work"));
        assert_eq!(exit, 3);
        assert_eq!(result.reason_code, "invalid_utf8");
    }

    #[test]
    fn progress_invalid_json_is_reported_before_validation() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let skill_root = repo.join("../skills/work");
        let input = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/invalid_json.txt");
        let (exit, response) = run_with_skill_root(
            &[
                "--project-root".into(),
                repo.to_string_lossy().into_owned(),
                "progress".into(),
                "validate".into(),
                "--input-file".into(),
                input.into(),
                "--expected-revision".into(),
                "0".into(),
            ],
            &skill_root,
        );
        assert_eq!(exit, 3);
        assert_eq!(response.reason_code, "invalid_json_contract");
    }

    #[test]
    fn progress_negative_expected_revision_is_cli_usage_error() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let input = repo.join(
            "crates/work-infrastructure/fixtures/delegation-role/outputs/work/progress/example/task/progress.json",
        );
        let (exit, response) = run_with_skill_root(
            &[
                "--project-root".into(),
                repo.to_string_lossy().into_owned(),
                "progress".into(),
                "validate".into(),
                "--input-file".into(),
                input.to_string_lossy().into_owned(),
                "--expected-revision".into(),
                "-1".into(),
            ],
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, ExitCode::CliUsage as i32);
        assert_eq!(response.reason_code, "cli_usage_error");
    }

    #[test]
    fn progress_cli_rejects_duplicate_input_and_missing_arguments() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = std::env::temp_dir().join(format!(
            "work-progress-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let duplicate = root.join("duplicate.json");
        fs::write(&duplicate, br#"{"mode":"plan","mode":"task"}"#).unwrap();
        let valid = repo.join(
            "crates/work-infrastructure/fixtures/delegation-role/outputs/work/progress/example/task/progress.json",
        );
        let prefix = [
            "--project-root".to_owned(),
            root.to_string_lossy().into_owned(),
        ];
        let (exit, response) = run_with_skill_root(
            &prefix
                .iter()
                .cloned()
                .chain([
                    "progress".into(),
                    "validate".into(),
                    "--input-file".into(),
                    duplicate.to_string_lossy().into_owned(),
                    "--expected-revision".into(),
                    "0".into(),
                ])
                .collect::<Vec<_>>(),
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, ExitCode::InputFormat as i32);
        assert_eq!(response.reason_code, "duplicate_json_key");
        for arguments in [
            vec![
                "save",
                "--input-file",
                valid.to_str().unwrap(),
                "--expected-revision",
                "0",
            ],
            vec!["validate", "--input-file", valid.to_str().unwrap()],
            vec!["read", "--requirement-id", "example"],
            vec!["read", "--requirement-id", "example", "--mode", "execute"],
        ] {
            let tokens = prefix
                .iter()
                .cloned()
                .chain(["progress".to_owned()])
                .chain(arguments.into_iter().map(str::to_owned))
                .collect::<Vec<_>>();
            let (exit, response) = run_with_skill_root(&tokens, &repo.join("../skills/work"));
            assert_eq!(exit, ExitCode::CliUsage as i32, "{tokens:?}");
            assert_eq!(response.reason_code, "cli_usage_error", "{tokens:?}");
        }
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    #[test]
    fn hierarchy_selection_build_and_validate_round_trip() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let skill_root = repo.join("../skills/work");
        let prefix = [
            "--project-root".to_owned(),
            repo.to_string_lossy().into_owned(),
            "--verbose".to_owned(),
        ];
        let input = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/hierarchy_request.json");
        let (build_exit, built) = run_with_skill_root(
            &[
                prefix.as_slice(),
                &[
                    "hierarchy".into(),
                    "selection-build".into(),
                    "--input-file".into(),
                    input.into(),
                ],
            ]
            .concat(),
            &skill_root,
        );
        assert_eq!(build_exit, 0);
        let expected: Value =
            serde_json::from_str(include_str!("../tests/hierarchy_selection.json")).unwrap();
        assert_eq!(built.data, expected);
        let validated_input = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/hierarchy_selection.json"
        );
        let (validate_exit, validated) = run_with_skill_root(
            &[
                prefix.as_slice(),
                &[
                    "hierarchy".into(),
                    "selection-validate".into(),
                    "--input-file".into(),
                    validated_input.into(),
                ],
            ]
            .concat(),
            &skill_root,
        );
        assert_eq!(validate_exit, 0);
        assert_eq!(validated.data["status"], "valid");
    }

    #[test]
    fn workspace_create_returns_owned_path_and_directory() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = std::env::temp_dir().join(format!(
            "work-cli-workspace-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let (exit, response) = run_with_skill_root(
            &[
                "--project-root".into(),
                root.to_string_lossy().into_owned(),
                "workspace".into(),
                "create".into(),
                "--requirement-id".into(),
                "example".into(),
                "--workflow-id".into(),
                "specification".into(),
            ],
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, 0);
        assert_eq!(response.data["schema"], "work-transaction-workspace/v1");
        assert_eq!(response.data["requirement_id"], "example");
        assert_eq!(response.data["workflow_id"], "specification");
        let id = response.data["transaction_id"].as_str().unwrap();
        assert_eq!(id.len(), 25);
        assert_eq!(&id[8..9], "T");
        assert_eq!(&id[15..17], "Z-");
        assert!(id[..8].bytes().all(|byte| byte.is_ascii_digit()));
        assert!(id[9..15].bytes().all(|byte| byte.is_ascii_digit()));
        assert!(
            id[17..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        let relative = response.data["path"].as_str().unwrap();
        assert_eq!(
            relative,
            format!("outputs/work/transactions/example/specification/{id}")
        );
        assert!(root.join(relative).is_dir());
    }

    #[test]
    fn paths_and_text_fingerprint_emit_python_brief_fields() {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."));
        let root = root.canonicalize().unwrap();
        let root_string = root.to_string_lossy().into_owned();
        let (exit, paths) = run(&[
            "--project-root".into(),
            root_string.clone(),
            "paths".into(),
            "resolve".into(),
            "--requirement-id".into(),
            "example".into(),
        ]);
        assert_eq!(exit, 0);
        assert_eq!(
            paths.data["paths"]["task"],
            "outputs/work/tasks/example/index.json"
        );
        assert!(paths.data.get("project_root").is_none());
        let (exit, fingerprint) = run(&[
            "--project-root".into(),
            root_string,
            "fingerprint".into(),
            "text".into(),
            "--path".into(),
            "README.md".into(),
        ]);
        assert_eq!(exit, 0);
        assert_eq!(fingerprint.data["path"], "README.md");
        assert_eq!(
            fingerprint.data["raw_sha256"],
            fingerprint.data["canonical_sha256"]
        );
    }

    #[test]
    fn hierarchy_resolve_projects_python_path_fields() {
        let (exit, result) = run(&[
            "--project-root".into(),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../..").into(),
            "hierarchy".into(),
            "resolve".into(),
            "--work-directory".into(),
            "task".into(),
            "web/frontend".into(),
        ]);
        assert_eq!(exit, 0);
        assert_eq!(
            result.data["resolved_paths"],
            json!(["general", "web", "web/frontend"])
        );
        assert!(result.data.get("work_directory").is_none());
    }

    #[test]
    fn hierarchy_selection_uses_installed_skill_catalog() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let skill_root = repo.join("../skills/work");
        let request = std::env::temp_dir().join(format!(
            "work-hierarchy-request-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(
            &request,
            b"{\"decision\":\"general_only\",\"selections\":[]}",
        )
        .unwrap();
        let (exit, response) = run_with_skill_root(
            &[
                "--project-root".into(),
                repo.to_string_lossy().into_owned(),
                "--verbose".into(),
                "hierarchy".into(),
                "selection-build".into(),
                "--input-file".into(),
                request.to_string_lossy().into_owned(),
            ],
            &skill_root,
        );
        assert_eq!(exit, 0);
        assert_eq!(response.data["schema"], "work-hierarchy-selection/v1");
        assert!(work_infrastructure::fixture_support::valid_sha256(
            response.data["selection_sha256"].as_str().unwrap()
        ));
    }

    #[test]
    fn instruction_catalog_and_source_selection_match_installed_reference() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let skill_root = repo.join("../skills/work");
        let prefix = [
            "--project-root".to_owned(),
            repo.to_string_lossy().into_owned(),
            "--verbose".to_owned(),
            "instructions".to_owned(),
        ];
        let (exit, catalog) = run_with_skill_root(
            &[
                prefix.as_slice(),
                &["catalog".into(), "--mode".into(), "task".into()],
            ]
            .concat(),
            &skill_root,
        );
        assert_eq!(exit, 0);
        assert!(work_infrastructure::fixture_support::valid_sha256(
            catalog.data["catalog_sha256"].as_str().unwrap()
        ));
        let (exit, selected) = run_with_skill_root(
            &[
                prefix.as_slice(),
                &["select".into(), "--mode".into(), "task".into()],
            ]
            .concat(),
            &skill_root,
        );
        assert_eq!(exit, 0);
        assert_eq!(selected.data["schema"], "work-instruction-selection/v1");
        assert_eq!(
            selected.data["instruction_selection"]["selected_paths"],
            json!([])
        );
    }

    #[test]
    fn instruction_cli_catalog_resolve_load_select_match_python_cases() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let skill_root = repo.join("../skills/work");
        let prefix = vec![
            "--project-root".to_owned(),
            repo.to_string_lossy().into_owned(),
            "--verbose".to_owned(),
            "instructions".to_owned(),
        ];
        let call = |arguments: &[&str]| {
            let mut args = prefix.clone();
            args.extend(arguments.iter().map(|value| (*value).to_owned()));
            run_with_skill_root(&args, &skill_root)
        };
        let plan_paths = [
            "general",
            "programming-language",
            "programming-language/java",
            "programming-language/typescript",
            "web",
            "web/backend",
            "web/frontend",
            "web/frontend/css",
        ];
        let task_paths = [
            "general",
            "programming-language",
            "programming-language/java",
            "programming-language/java/persistence",
            "programming-language/java/persistence/jpa",
            "programming-language/java/persistence/mybatis",
            "programming-language/java/spring-boot",
            "programming-language/typescript",
            "web",
            "web/backend",
            "web/frontend",
            "web/frontend/astro",
            "web/frontend/css",
            "web/frontend/css/tailwind",
        ];
        for (mode, paths) in [
            ("plan", plan_paths.as_slice()),
            ("task", task_paths.as_slice()),
            ("execute", task_paths.as_slice()),
        ] {
            let (exit, response) = call(&["catalog", "--mode", mode]);
            assert_eq!(exit, 0);
            assert_eq!(response.data["schema"], "work-instruction-catalog/v1");
            assert_eq!(response.data["mode"], mode);
            assert_eq!(response.data["paths"], json!(paths));
            assert_eq!(
                response.data["children"]["general"],
                json!(["programming-language", "web"])
            );
            assert_eq!(
                response.data["metadata"]["general"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                vec!["description", "name", "work_tags"]
            );
            assert!(!response.data["metadata"].to_string().contains("指令邊界"));
        }
        let (exit, invalid) = call(&["catalog", "--mode", "build"]);
        assert_eq!(exit, 2);
        assert_eq!(invalid.reason_code, "cli_usage_error");
        let (exit, all) = call(&["catalog", "--mode", "all"]);
        assert_eq!(exit, 0);
        assert_eq!(all.data["mode"], "all");
        assert_eq!(
            all.data["metadata"]["programming-language/java/persistence/jpa"]["mode_support"],
            json!(["task", "execute"])
        );
        assert!(work_infrastructure::fixture_support::valid_sha256(
            all.data["catalog_sha256"].as_str().unwrap()
        ));

        let leaves = [
            "programming-language/java/persistence/jpa",
            "programming-language/java/persistence/mybatis",
        ];
        let (exit, resolved) = call(&["resolve", "--mode", "task", leaves[0], leaves[1]]);
        assert_eq!(exit, 0);
        assert_eq!(resolved.data["schema"], "work-hierarchy/v1");
        assert_eq!(resolved.data["selected_paths"], json!(leaves));
        assert_eq!(
            resolved.data["resolved_paths"],
            json!([
                "general",
                "programming-language",
                "programming-language/java",
                "programming-language/java/persistence",
                "programming-language/java/persistence/jpa",
                "programming-language/java/persistence/mybatis"
            ])
        );
        let (exit, missing) = call(&[
            "resolve",
            "--mode",
            "task",
            "programming-language/java/persistence/hibernate",
        ]);
        assert_eq!(exit, 4);
        assert_eq!(missing.reason_code, "instruction_hierarchy_path_missing");
        assert_eq!(
            missing.data,
            json!({"mode":"task","parent":"programming-language/java/persistence","path":"programming-language/java/persistence/hibernate","valid_choices":["jpa","mybatis"]})
        );
        let (exit, projected) = call(&[
            "resolve",
            "--mode",
            "plan",
            "programming-language/java/persistence/jpa",
        ]);
        assert_eq!(exit, 0);
        assert_eq!(
            projected.data["selected_paths"],
            json!(["programming-language/java/persistence/jpa"])
        );
        assert_eq!(
            projected.data["resolved_paths"],
            json!([
                "general",
                "programming-language",
                "programming-language/java"
            ])
        );

        let (exit, combined) = call(&[
            "load",
            "--mode",
            "task",
            "--reference",
            "task.web.backend.security",
            "--reference",
            "task.programming-language.java.swagger",
            "--reference",
            "task.programming-language.java.persistence.relational-data",
            "web/backend",
            "programming-language/java/spring-boot",
            "programming-language/java/persistence/jpa",
        ]);
        assert_eq!(exit, 0);
        assert_eq!(
            combined.data["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|source| source["logical_name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "work.instruction-loading",
                "work.workflow.task",
                "task.general",
                "task.web",
                "task.web.backend",
                "task.web.backend.security",
                "task.programming-language",
                "task.programming-language.java",
                "task.programming-language.java.swagger",
                "task.programming-language.java.spring-boot",
                "task.programming-language.java.persistence",
                "task.programming-language.java.persistence.relational-data",
                "task.programming-language.java.persistence.jpa"
            ]
        );

        let source_args = [
            "--mode",
            "task",
            "--reference",
            "task.web.backend.security",
            "--reference",
            "task.general.task-records",
            "web/backend",
        ];
        let load_args = std::iter::once("load")
            .chain(source_args)
            .collect::<Vec<_>>();
        let (exit, loaded) = call(&load_args);
        assert_eq!(exit, 0);
        assert_eq!(loaded.data["schema"], "work-instructions/v1");
        assert_eq!(loaded.data["mode"], "task");
        assert_eq!(
            loaded.data["sources"]
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
                "task.web.backend.security"
            ]
        );
        assert_eq!(
            loaded.data["references"],
            json!(["task.general.task-records", "task.web.backend.security"])
        );
        assert!(work_infrastructure::fixture_support::valid_sha256(
            loaded.data["instructions_sha256"].as_str().unwrap()
        ));
        for source in loaded.data["sources"].as_array().unwrap() {
            assert_eq!(source.as_object().unwrap().len(), 4);
        }
        let select_args = std::iter::once("select")
            .chain(source_args)
            .collect::<Vec<_>>();
        let (exit, selected) = call(&select_args);
        assert_eq!(exit, 0);
        assert_eq!(selected.data["schema"], "work-instruction-selection/v1");
        assert_eq!(selected.data["mode"], "task");
        assert_eq!(
            selected.data["instruction_selection"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "instructions_sha256",
                "references",
                "resolved_paths",
                "selected_paths",
                "sources"
            ]
        );
        assert_eq!(
            selected.data["instruction_selection"]["references"],
            loaded.data["references"]
        );
        assert!(
            selected.data["instruction_selection"]["sources"]
                .as_array()
                .unwrap()
                .iter()
                .all(|source| source.get("layer").is_none())
        );
        let (exit, selected) = call(&["select", "--mode", "task", leaves[0], leaves[1]]);
        assert_eq!(exit, 0);
        assert_eq!(
            selected.data["instruction_selection"]["selected_paths"],
            json!(leaves)
        );
        for command in ["load", "select"] {
            let (exit, error) = call(&[
                command,
                "--mode",
                "task",
                "--reference",
                "task.web.backend.security",
            ]);
            assert_eq!(exit, 4);
            assert_eq!(error.reason_code, "unroutable_instruction_reference");
            assert_eq!(error.data["logical_name"], "task.web.backend.security");
        }
    }

    #[test]
    fn plan_validate_cli_requires_path_for_input_and_rejects_path_for_file() {
        let root = std::env::temp_dir().join(format!(
            "work-plan-cli-t25-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("request.json"), b"{}").unwrap();
        let skill_root =
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        let prefix = vec![
            "--project-root".to_owned(),
            root.to_string_lossy().into_owned(),
            "plan".to_owned(),
            "validate".to_owned(),
            "--user-config-root".to_owned(),
            root.to_string_lossy().into_owned(),
        ];
        let call = |suffix: &[&str]| {
            let mut arguments = prefix.clone();
            arguments.extend(suffix.iter().map(|value| (*value).to_owned()));
            run_with_skill_root(&arguments, &skill_root)
        };
        let input = root.join("request.json");
        let (exit, missing) = call(&["--input-file", input.to_str().unwrap()]);
        assert_eq!(exit, 2);
        assert_eq!(missing.reason_code, "plan_path_required");
        let (exit, unexpected) = call(&[
            "--path",
            "outputs/work/plans/example.json",
            "--plan-path",
            "outputs/work/plans/other.json",
        ]);
        assert_eq!(exit, 2);
        assert_eq!(unexpected.reason_code, "unexpected_plan_path");
    }

    #[test]
    fn plan_semantic_prepare_cli_is_read_only_and_output_file_is_exclusive() {
        let parent = std::env::temp_dir().join(format!(
            "work-plan-semantic-cli-t25-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let root = parent.join("project");
        std::fs::create_dir_all(&root).unwrap();
        let input = parent.join("request.json");
        let request = json!({
            "requirement_id":"example","title":"Plan result","summary":"Prepare a plan.",
            "goals":["Deliver result."],"scope":["Implementation."],
            "deliverables":["Result artifact."],"acceptance_criteria":["Result is verified."],
            "hierarchy_selection_request":{"decision":"general_only","selections":[]},
            "skill_selection_request":{"decision":"base_only","skills":[]},"references":[]
        });
        std::fs::write(&input, serde_json::to_vec(&request).unwrap()).unwrap();
        let skill_root =
            PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        let prefix = vec![
            "--project-root".to_owned(),
            root.to_string_lossy().into_owned(),
            "--verbose".to_owned(),
            "plan".to_owned(),
            "semantic-prepare".to_owned(),
            "--input-file".to_owned(),
            input.to_string_lossy().into_owned(),
            "--user-config-root".to_owned(),
            root.to_string_lossy().into_owned(),
        ];
        let call = |suffix: &[&str]| {
            let mut arguments = prefix.clone();
            arguments.extend(suffix.iter().map(|value| (*value).to_owned()));
            run_with_skill_root(&arguments, &skill_root)
        };
        let (exit, prepared) = call(&[]);
        assert_eq!(exit, 0);
        assert_eq!(prepared.data["schema"], "work-plan-prepare/v1");
        let expected = prepare_semantic(
            &LocalHierarchyCatalog {
                skill_root: skill_root.clone(),
            },
            &LocalSkillCatalog { roots: vec![] },
            &LocalPlanStorage {
                project_root: root.clone(),
            },
            &LocalPlanStorage {
                project_root: root.clone(),
            },
            &[],
            &request,
            None,
        )
        .unwrap();
        assert_eq!(prepared.data["plan"], expected["plan"]);
        assert_eq!(prepared.data["validation"], expected["validation"]);
        assert_eq!(
            prepared.data["plan"]["artifacts"]["task"],
            "outputs/work/tasks/example/index.json"
        );
        assert_eq!(
            prepared.data["validation"]["schema"],
            "work-plan-validation/v1"
        );
        assert!(std::fs::read_dir(&root).unwrap().next().is_none());

        let mut unicode = request;
        unicode["title"] = json!("跨平台計畫");
        std::fs::write(&input, serde_json::to_vec(&unicode).unwrap()).unwrap();
        let output = root.join("transaction/plan-candidate.json");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        let (exit, saved) = call(&["--output-file", output.to_str().unwrap()]);
        assert_eq!(exit, 0);
        let raw = std::fs::read(&output).unwrap();
        assert!(!raw.starts_with(&[0xef, 0xbb, 0xbf]));
        assert!(!raw.windows(2).any(|pair| pair == b"\r\n"));
        assert_eq!(
            serde_json::from_slice::<Value>(&raw).unwrap(),
            saved.data["plan"]
        );
        let (exit, repeated) = call(&["--output-file", output.to_str().unwrap()]);
        assert_eq!(exit, 6);
        assert_eq!(repeated.reason_code, "plan_prepare_output_exists");
        assert_eq!(std::fs::read(&output).unwrap(), raw);
    }

    #[test]
    fn skill_catalog_and_workflow_entrypoints_match_python_reference() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let skill_root = repo.join("../skills/work");
        let prefix = [
            "--project-root".to_owned(),
            repo.to_string_lossy().into_owned(),
            "--verbose".to_owned(),
        ];
        let (exit, catalog) = run_with_skill_root(
            &[
                prefix.as_slice(),
                &[
                    "skills".into(),
                    "catalog".into(),
                    "--root".into(),
                    format!(
                        "repo:skills={}",
                        repo.canonicalize()
                            .unwrap()
                            .parent()
                            .unwrap()
                            .join("skills")
                            .display()
                    ),
                ],
            ]
            .concat(),
            &skill_root,
        );
        assert_eq!(exit, 0);
        assert_eq!(
            catalog.data,
            json!({"schema":"work-skill-catalog/v1","skills":[],"unavailable":[]})
        );
        let (exit, workflow) = run_with_skill_root(
            &[
                prefix.as_slice(),
                &[
                    "workflow".into(),
                    "status".into(),
                    "--requirement-id".into(),
                    "issue55test".into(),
                    "--user-config-root".into(),
                    ".".into(),
                ],
            ]
            .concat(),
            &skill_root,
        );
        assert_eq!(exit, 0);
        assert_eq!(workflow.data["next_action"], "prepare_plan");
        assert_eq!(
            workflow.data["selection_sha256"],
            workflow.data["selection_manifest"]["selection_sha256"]
        );
        let (next_exit, next) = run_with_skill_root(
            &[
                prefix.as_slice(),
                &[
                    "workflow".into(),
                    "next".into(),
                    "--requirement-id".into(),
                    "issue55test".into(),
                    "--user-config-root".into(),
                    ".".into(),
                ],
            ]
            .concat(),
            &skill_root,
        );
        assert_eq!(next_exit, 0);
        assert_eq!(next.data, workflow.data);
        let (maintenance_exit, maintenance) = run_with_skill_root(
            &[
                prefix.as_slice(),
                &[
                    "workflow".into(),
                    "status".into(),
                    "--requirement-id".into(),
                    "issue55test".into(),
                    "--user-config-root".into(),
                    ".".into(),
                    "--instruction-maintenance".into(),
                ],
            ]
            .concat(),
            &skill_root,
        );
        assert_eq!(maintenance_exit, 0);
        assert!(
            maintenance.data["required_instruction_sources"]
                .as_array()
                .unwrap()
                .contains(&json!("work.shared.instruction-maintenance"))
        );
        assert_ne!(
            maintenance.data["selection_sha256"],
            workflow.data["selection_sha256"]
        );
    }

    #[test]
    fn attempt_input_validation_matches_python_reference() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/specification-reconciliation/real-flow/outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json");
        let (exit, result) = run(&[
            "--project-root".into(),
            repo.to_string_lossy().into_owned(),
            "--verbose".into(),
            "attempt".into(),
            "validate".into(),
            "--input-file".into(),
            fixture.to_string_lossy().into_owned(),
        ]);
        assert_eq!(exit, 0);
        assert_eq!(
            result.data,
            json!({"attempt_id":"ATTEMPT-001","record_count":1,"result":"valid",
                "schema":"work-attempt-validation/v1","status":"completed",
                "task_id":"TASK-001","task_spec_id":"TASK-SPEC-001"})
        );
    }

    #[test]
    fn plan_file_validation_matches_python_reference() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture_root =
            repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        let (exit, result) = run_with_skill_root(
            &[
                "--project-root".into(),
                fixture_root.to_string_lossy().into_owned(),
                "--verbose".into(),
                "plan".into(),
                "validate".into(),
                "--path".into(),
                "outputs/work/plans/example.json".into(),
                "--user-config-root".into(),
                ".".into(),
            ],
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, 0);
        assert_eq!(result.data["schema"], "work-plan-validation/v1");
        assert_eq!(
            result.data["plan_sha256"],
            "c1580aa54f48e015ee2efb9b0a5de74122f60839689cc8fd65bbfb16447f20bf"
        );
    }

    #[test]
    fn handoff_validation_matches_python_reference() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/handoff-closed/stopped");
        let (exit, result) = run_with_skill_root(
            &[
                "--project-root".into(),
                fixture.to_string_lossy().into_owned(),
                "--verbose".into(),
                "handoff".into(),
                "validate".into(),
                "--input-file".into(),
                fixture
                    .join("execute_to_task-expected.json")
                    .to_string_lossy()
                    .into_owned(),
            ],
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, 0);
        assert_eq!(
            result.data,
            json!({"direction":"execute_to_task","marker":"WORK-HANDOFF",
            "requirement_id":"example","schema":"work-handoff-validation/v1",
            "source_stage":"execute","status":"valid","target_stage":"task"})
        );
    }

    #[test]
    fn task_status_matches_saved_planning_reference() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        let index = fixture.join("outputs/work/tasks/example/drafts/index.json");
        let history = fixture.join("outputs/work/tasks/example/drafts/history/1/index.json");
        let before_index = fs::read(&index).unwrap();
        let before_history = fs::read(&history).unwrap();
        let (exit, result) = run_with_skill_root(
            &[
                "--project-root".into(),
                fixture.to_string_lossy().into_owned(),
                "--verbose".into(),
                "task".into(),
                "status".into(),
                "--requirement-id".into(),
                "example".into(),
                "--plan-path".into(),
                "outputs/work/plans/example.json".into(),
                "--user-config-root".into(),
                ".".into(),
            ],
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, 0);
        assert_eq!(result.data["status"], "saved");
        assert_eq!(result.data["next_action"], "confirm_start");
        assert_eq!(result.data["counts"]["planned"], 1);
        assert_eq!(
            result.data["tasks"][0]["instructions_sha256"],
            "e4fdfc5254dda6a8522ec33f92f91a788f8ba44d69c89fa62624d7365718eef3"
        );
        let (exit, selected) = run_with_skill_root(
            &[
                "--project-root".into(),
                fixture.to_string_lossy().into_owned(),
                "task".into(),
                "status".into(),
                "--requirement-id".into(),
                "example".into(),
                "--task-id".into(),
                "TASK-001".into(),
                "--plan-path".into(),
                "outputs/work/plans/example.json".into(),
                "--user-config-root".into(),
                ".".into(),
            ],
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, 0);
        assert_eq!(selected.data["status"], "saved");
        assert_eq!(selected.data["selected_task_id"], "TASK-001");
        assert_eq!(fs::read(index).unwrap(), before_index);
        assert_eq!(fs::read(history).unwrap(), before_history);
    }

    #[test]
    fn task_status_checks_saved_sources() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/task-draft-sources/valid");
        let (exit, result) = run_with_skill_root(
            &[
                "--project-root".into(),
                fixture.to_string_lossy().into_owned(),
                "--verbose".into(),
                "task".into(),
                "status".into(),
                "--requirement-id".into(),
                "example".into(),
                "--task-id".into(),
                "TASK-001".into(),
                "--plan-path".into(),
                "outputs/work/plans/example.json".into(),
                "--user-config-root".into(),
                ".".into(),
            ],
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, 0, "{result:?}");
        assert_eq!(result.data["source_validation"], "valid");
        assert_eq!(
            result.data["tasks"][0]["instructions_sha256"],
            "e4fdfc5254dda6a8522ec33f92f91a788f8ba44d69c89fa62624d7365718eef3"
        );
    }

    #[test]
    fn task_collection_validation_matches_python_reference() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/handoff-closed/stopped");
        let (exit, result) = run_with_skill_root(
            &[
                "--project-root".into(),
                fixture.to_string_lossy().into_owned(),
                "--verbose".into(),
                "task".into(),
                "validate".into(),
                "--path".into(),
                "outputs/work/tasks/example/index.json".into(),
                "--user-config-root".into(),
                ".".into(),
            ],
            &repo.join("../skills/work"),
        );
        assert_eq!(exit, 0);
        assert_eq!(result.data["task_count"], 2);
        assert_eq!(
            result.data["task_collection_sha256"],
            "6ad141589d3445cd62715fe46db2d827c0673d0101b1797ace6980a22546139e"
        );
    }

    #[test]
    fn artifact_migration_cli_routes_saved_request_through_publication() {
        let repo = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let root = std::env::temp_dir().join(format!(
            "work-migration-cli-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plan_path = root.join("outputs/work/plans/example.json");
        fs::create_dir_all(plan_path.parent().unwrap()).unwrap();
        let fixture = repo.join("crates/work-infrastructure/fixtures/specification-migration/outputs/work/plans/example.json");
        let mut legacy: Value = serde_json::from_slice(&fs::read(&fixture).unwrap()).unwrap();
        legacy["schema"] = json!("work-plan/v0");
        fs::write(&plan_path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();
        let base = [
            "--project-root".to_owned(),
            root.to_string_lossy().into_owned(),
            "--verbose".to_owned(),
        ];
        let skill_root = repo.join("../skills/work");
        let invoke = |tail: Vec<String>| {
            run_with_skill_root(
                &base.iter().cloned().chain(tail).collect::<Vec<_>>(),
                &skill_root,
            )
        };
        let (exit, analysis) = invoke(vec![
            "migration".into(),
            "analyze".into(),
            "--requirement-id".into(),
            "example".into(),
            "--artifact".into(),
            "plan".into(),
        ]);
        assert_eq!(exit, 0, "{analysis:?}");
        assert_eq!(analysis.data["items"].as_array().unwrap().len(), 1);
        let input = root.join("decisions.json");
        let choices = json!({"schema":"work-artifact-migration-decisions/v1",
            "analysis":analysis.data,"choices":[{"id":analysis.data["items"][0]["id"],"action":"apply"}]});
        fs::write(&input, serde_json::to_vec(&choices).unwrap()).unwrap();
        let (exit, prepared) = invoke(vec![
            "migration".into(),
            "prepare".into(),
            "--input-file".into(),
            input.to_string_lossy().into_owned(),
        ]);
        assert_eq!(exit, 0, "{prepared:?}");
        let request_path = prepared.data["request_path"].as_str().unwrap().to_owned();
        let approved = prepared.data["request_sha256"].as_str().unwrap().to_owned();
        let arguments = vec![
            "--request-path".into(),
            request_path,
            "--approved-sha256".into(),
            approved,
        ];
        let (exit, previewed) = invoke(
            [
                vec!["migration".into(), "preview".into()],
                arguments.clone(),
            ]
            .concat(),
        );
        assert_eq!(exit, 0, "{previewed:?}");
        assert_eq!(previewed.data["status"], "ready");
        let (exit, applied) =
            invoke([vec!["migration".into(), "apply".into()], arguments.clone()].concat());
        assert_eq!(exit, 0, "{applied:?}");
        assert_eq!(applied.data["status"], "completed", "{applied:?}");
        let plan_before = fs::read(&plan_path).unwrap();
        let (exit, verified) =
            invoke([vec!["migration".into(), "verify".into()], arguments.clone()].concat());
        assert_eq!(exit, 0, "{verified:?}");
        assert_eq!(verified.data["status"], "valid");
        assert_eq!(verified.data["mode"], "artifact");
        assert_eq!(fs::read(&plan_path).unwrap(), plan_before);
        let (exit, recovered) =
            invoke([vec!["migration".into(), "recover".into()], arguments].concat());
        assert_eq!(exit, 0, "{recovered:?}");
        assert_eq!(recovered.data["status"], "completed");
    }
}
