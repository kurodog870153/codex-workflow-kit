//! Read-only TASK collection diagnosis; this report never authorizes repair.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::plan::{PlanValidationInput, validate_plan};
use work_feature::ports::ArtifactStore;
use work_feature::skill::SkillRoot;
use work_feature::task::load_collection;
use work_operations::canonical::{canonical_text, decode_utf8, parse_json_contract, sha256_hex};
use work_operations::execution::index::{build_initial_execution_index, validate_execution_index};
use work_operations::task::index::validate_task_index;
use work_operations::task::item::validate_task_item;
use work_operations::task::ordering::{TaskDocumentKind, render_task};

use crate::files::{LocalFiles, resolve_project_path};
use crate::hierarchy_catalog::LocalHierarchyCatalog;
use crate::plan_storage::LocalPlanStorage;
use crate::skill_catalog::{LocalSkillCatalog, SkillRootConfig};
use crate::specification::storage::{require_no_spec_update, storage_path};
use crate::task::storage::LocalTaskStorage;

#[derive(Default)]
struct Report {
    checks: Vec<Value>,
    issues: Vec<Value>,
}

impl Report {
    fn passed(&mut self, name: &str) {
        self.checks.push(json!({"name":name,"status":"passed"}));
    }
    fn skip(&mut self, name: &str, dependencies: &[&str]) {
        self.checks
            .push(json!({"name":name,"status":"not_checked","requires":dependencies}));
    }
    fn failure(&mut self, name: &str, error: WorkError, location: &str) {
        self.checks.push(json!({"name":name,"status":"failed"}));
        let leaf = name.rsplit(':').next().unwrap_or(name);
        let (category, suggestion) = if matches!(leaf, "encoding" | "json") {
            (
                "user_decision",
                "Preserve the original bytes; ask the user to resolve any ambiguous decoding or JSON interpretation.",
            )
        } else if matches!(leaf, "normalization" | "canonical") {
            (
                "format_repair",
                "Preview a lossless canonical UTF-8 rendering before requesting write approval.",
            )
        } else if name.starts_with("instructions") {
            (
                "source_review",
                "Review the evidence before preparing a repair; do not change source fingerprints alone.",
            )
        } else {
            (
                "review_required",
                "Review the evidence before preparing a repair; do not change source fingerprints alone.",
            )
        };
        self.issues
            .push(json!({"stage":name,"code":error.reason_code,
            "location":location,"category":category,"message":error.message,
            "suggestion":suggestion,"details":error.details}));
    }
    fn check<T>(&mut self, name: &str, location: &str, result: Result<T, WorkError>) -> Option<T> {
        match result {
            Ok(value) => {
                self.passed(name);
                Some(value)
            }
            Err(error) => {
                self.failure(name, error, location);
                None
            }
        }
    }
    fn status(&self, name: &str) -> &'static str {
        self.checks
            .iter()
            .find(|row| row["name"] == name)
            .and_then(|row| row["status"].as_str())
            .map_or("not_checked", |status| match status {
                "passed" => "passed",
                "failed" => "failed",
                _ => "not_checked",
            })
    }
}

fn error(code: ExitCode, reason: &str, message: &str, details: Value) -> WorkError {
    WorkError::new(code, reason, message, details)
}

fn parse(raw: &[u8]) -> Result<Value, WorkError> {
    parse_json_contract(raw).map_err(|issue| {
        let (reason, message, details) = match issue {
            work_operations::canonical::JsonContractIssue::DuplicateKey(key) => (
                "duplicate_json_key",
                "Duplicate keys are ambiguous; no parsed document will be used.",
                json!({"keys":[key]}),
            ),
            work_operations::canonical::JsonContractIssue::NotObject => (
                "json_contract_not_object",
                "The TASK document must be a JSON object.",
                json!({}),
            ),
            work_operations::canonical::JsonContractIssue::InvalidConstant(_) => (
                "invalid_json_constant",
                "JSON cannot contain non-standard numeric constants.",
                json!({}),
            ),
            work_operations::canonical::JsonContractIssue::InvalidJson { line, column } => {
                let eof = serde_json::from_slice::<Value>(raw)
                    .err()
                    .is_some_and(|error| error.is_eof());
                let text = decode_utf8(raw).unwrap_or("");
                let (line, column) = if eof {
                    (
                        text.bytes().filter(|byte| *byte == b'\n').count() + 1,
                        text.rsplit('\n').next().unwrap_or("").chars().count() + 1,
                    )
                } else {
                    (line, column)
                };
                let byte_offset = if eof {
                    raw.len()
                } else {
                    let mut start = 0;
                    for part in raw
                        .split(|byte| *byte == b'\n')
                        .take(line.saturating_sub(1))
                    {
                        start += part.len() + 1;
                    }
                    start + column.saturating_sub(1)
                };
                (
                    "invalid_json_contract",
                    "The TASK JSON is invalid.",
                    json!({"line":line,"column":column,
                        "byte_offset":byte_offset}),
                )
            }
            _ => (
                "invalid_json_contract",
                "The TASK JSON is invalid.",
                json!({}),
            ),
        };
        error(ExitCode::Contract, reason, message, details)
    })
}

fn same(actual: &Value, expected: &Value, reason: &str, message: &str) -> Result<(), WorkError> {
    if actual != expected {
        return Err(error(
            ExitCode::Contract,
            reason,
            message,
            json!({"expected":expected,"actual":actual}),
        ));
    }
    Ok(())
}

fn canonical_difference(expected: &[u8], actual: &[u8]) -> Value {
    json!({"expected":String::from_utf8_lossy(expected),
        "actual":String::from_utf8_lossy(actual)})
}

fn decode<'a>(raw: &'a [u8], source: &str) -> Result<&'a str, WorkError> {
    let bom_size = usize::from(raw.starts_with(&[0xef, 0xbb, 0xbf])) * 3;
    let raw = &raw[bom_size..];
    std::str::from_utf8(raw).map_err(|invalid| {
        error(
            ExitCode::InputFormat,
            "invalid_utf8",
            "The input is not valid UTF-8.",
            json!({"source":source,"byte_offset":bom_size + invalid.valid_up_to()}),
        )
    })
}

pub fn diagnose_task_collection(
    root: &Path,
    skill_root: &Path,
    configs: &[SkillRootConfig],
    raw_index_path: &str,
) -> Value {
    let mut report = Report::default();
    let resolved = report.check(
        "index:path",
        raw_index_path,
        resolve_project_path(root, raw_index_path),
    );
    let index_raw = if let Some((_, path)) = &resolved {
        report.check("index:file", raw_index_path, LocalFiles.read_raw(path))
    } else {
        report.skip("index:file", &["index:path"]);
        None
    };
    let text = if let Some(raw) = &index_raw {
        report.check(
            "index:encoding",
            raw_index_path,
            decode(raw, raw_index_path),
        )
    } else {
        report.skip("index:encoding", &["index:file"]);
        None
    };
    let document = if let Some(text) = text {
        let parsed = report.check(
            "index:json",
            raw_index_path,
            parse(index_raw.as_ref().unwrap()),
        );
        if canonical_text(text).as_bytes() == index_raw.as_deref().unwrap() {
            report.passed("index:normalization");
        } else {
            report.failure(
                "index:normalization",
                error(
                    ExitCode::Contract,
                    "noncanonical_task_text",
                    "TASK text must be UTF-8 without BOM, NFC, LF and one trailing LF.",
                    json!({}),
                ),
                raw_index_path,
            );
        }
        parsed
    } else {
        report.skip("index:json", &["index:encoding"]);
        report.skip("index:normalization", &["index:encoding"]);
        None
    };
    let mut index_valid = false;
    let mut item_raw = std::collections::BTreeMap::new();
    if let (Some(raw), Some(value)) = (&index_raw, &document) {
        report.check(
            "index:schema",
            "/schema",
            same(
                &value["schema"],
                &json!("work-task-index/v1"),
                "invalid_task_index_schema",
                "The formal TASK index schema is invalid.",
            ),
        );
        match render_task(value, TaskDocumentKind::Index) {
            Ok(canonical) if &canonical == raw => report.passed("index:canonical"),
            candidate => report.failure(
                "index:canonical",
                error(
                    ExitCode::Contract,
                    "noncanonical_json_contract",
                    "The TASK index does not match canonical field order and serialization.",
                    candidate.map_or_else(
                        |_| json!({}),
                        |canonical| canonical_difference(&canonical, raw),
                    ),
                ),
                raw_index_path,
            ),
        }
        index_valid = report
            .check(
                "index:contract",
                raw_index_path,
                validate_task_index(value, raw, raw_index_path).map_err(|issue| {
                    error(
                        ExitCode::Contract,
                        issue.reason_code,
                        issue.message,
                        issue.details,
                    )
                }),
            )
            .is_some();
    } else {
        for name in ["index:schema", "index:canonical", "index:contract"] {
            report.skip(name, &["index:json"]);
        }
    }
    let references = document
        .as_ref()
        .and_then(|index| index["tasks"].as_array());
    if references.is_none()
        || document
            .as_ref()
            .and_then(|index| index["requirement_id"].as_str())
            .is_none()
    {
        report.skip("items", &["index:json", "index:structure"]);
    }
    if let Some(references) = references {
        let directory = raw_index_path
            .rsplit_once('/')
            .map_or("", |(parent, _)| parent);
        for (position, reference) in references.iter().enumerate() {
            let id = reference["id"].as_str().unwrap_or("");
            let prefix = format!("item:{id}");
            let relative = reference["path"].as_str().unwrap_or("");
            let path = format!("{directory}/{relative}");
            let item_path = report.check(
                &format!("{prefix}:path"),
                &format!("/tasks/{position}/path"),
                storage_path(root, &path),
            );
            let item_location = item_path.as_ref().map_or_else(
                || path.clone(),
                |resolved| resolved.to_string_lossy().into_owned(),
            );
            let raw = if let Some(path) = &item_path {
                report.check(
                    &format!("{prefix}:file"),
                    &path.to_string_lossy(),
                    LocalFiles.read_raw(path),
                )
            } else {
                report.skip(&format!("{prefix}:file"), &[&format!("{prefix}:path")]);
                None
            };
            if let Some(raw) = raw {
                item_raw.insert(id.to_owned(), raw.clone());
                let text = report.check(
                    &format!("{prefix}:encoding"),
                    &item_location,
                    decode(&raw, &item_location),
                );
                if let Some(text) = text {
                    let parsed =
                        report.check(&format!("{prefix}:json"), &item_location, parse(&raw));
                    if canonical_text(text).as_bytes() == raw.as_slice() {
                        report.passed(&format!("{prefix}:normalization"));
                    } else {
                        report.failure(
                            &format!("{prefix}:normalization"),
                            error(
                                ExitCode::Contract,
                                "noncanonical_task_text",
                                "TASK text must be UTF-8 without BOM, NFC, LF and one trailing LF.",
                                json!({}),
                            ),
                            &item_location,
                        );
                    }
                    if let Some(item) = parsed {
                        match render_task(&item, TaskDocumentKind::Item) {
                            Ok(canonical) if canonical == raw => report.passed(&format!("{prefix}:canonical")),
                            candidate => report.failure(&format!("{prefix}:canonical"), error(ExitCode::Contract,
                                "noncanonical_json_contract", "The TASK item does not match canonical field order and serialization.",
                                candidate.map_or_else(|_| json!({}),
                                    |canonical| canonical_difference(&canonical, &raw))), &item_location),
                        }
                        let validated = report.check(
                            &format!("{prefix}:contract"),
                            &item_location,
                            validate_task_item(&item, &raw, id).map_err(|issue| {
                                error(
                                    ExitCode::Contract,
                                    issue.reason_code,
                                    issue.message,
                                    issue.details,
                                )
                            }),
                        );
                        if let Some(validated) = validated {
                            report.check(
                                &format!("{prefix}:fingerprint"),
                                &format!("/tasks/{position}/canonical_sha256"),
                                same(
                                    &validated["task_item_sha256"],
                                    &reference["canonical_sha256"],
                                    "task_item_fingerprint_mismatch",
                                    "The TASK item fingerprint differs from the formal index.",
                                ),
                            );
                        } else {
                            report.skip(
                                &format!("{prefix}:fingerprint"),
                                &[&format!("{prefix}:contract")],
                            );
                        }
                    } else {
                        report.skip(&format!("{prefix}:canonical"), &[&format!("{prefix}:json")]);
                        report.skip(&format!("{prefix}:contract"), &[&format!("{prefix}:json")]);
                        report.skip(
                            &format!("{prefix}:fingerprint"),
                            &[&format!("{prefix}:contract")],
                        );
                    }
                } else {
                    report.skip(&format!("{prefix}:json"), &[&format!("{prefix}:encoding")]);
                    report.skip(
                        &format!("{prefix}:normalization"),
                        &[&format!("{prefix}:encoding")],
                    );
                    report.skip(&format!("{prefix}:contract"), &[&format!("{prefix}:json")]);
                    report.skip(
                        &format!("{prefix}:fingerprint"),
                        &[&format!("{prefix}:contract")],
                    );
                }
            } else {
                report.skip(&format!("{prefix}:encoding"), &[&format!("{prefix}:file")]);
                report.skip(&format!("{prefix}:json"), &[&format!("{prefix}:encoding")]);
                report.skip(&format!("{prefix}:contract"), &[&format!("{prefix}:json")]);
                report.skip(
                    &format!("{prefix}:fingerprint"),
                    &[&format!("{prefix}:contract")],
                );
            }
        }
        let expected = references
            .iter()
            .filter_map(|row| row["path"].as_str())
            .filter_map(|path| path.strip_prefix("tasks/"))
            .collect::<std::collections::BTreeSet<_>>();
        let folder = storage_path(root, &format!("{directory}/tasks"));
        let observed = folder.and_then(|folder| {
            if !folder.exists() {
                return Ok(std::collections::BTreeSet::new());
            }
            fs::read_dir(&folder)
                .map_err(|_| {
                    error(
                        ExitCode::IoFailure,
                        "task_collection_directory_read_failed",
                        "The TASK item directory could not be inspected.",
                        json!({}),
                    )
                })?
                .map(|entry| {
                    entry
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .map_err(|_| {
                            error(
                                ExitCode::IoFailure,
                                "task_collection_directory_read_failed",
                                "The TASK item directory could not be inspected.",
                                json!({}),
                            )
                        })
                })
                .collect::<Result<std::collections::BTreeSet<_>, _>>()
        });
        report.check(
            "items:directory",
            &format!("{directory}/tasks"),
            observed.and_then(|found| {
                let expected = expected
                    .into_iter()
                    .map(str::to_owned)
                    .collect::<std::collections::BTreeSet<_>>();
                same(
                    &json!(found),
                    &json!(expected),
                    "task_collection_directory_mismatch",
                    "The TASK item directory does not exactly match the formal index.",
                )
            }),
        );
    } else {
        report.skip("items:directory", &["index:path", "index:json"]);
    }
    let plan_path = document
        .as_ref()
        .and_then(|value| value["artifacts"]["plan"].as_str());
    let plan_raw = if let Some(plan) = plan_path {
        let path = report.check("plan:path", plan, storage_path(root, plan));
        if let Some(path) = path {
            report.check("plan:file", plan, LocalFiles.read_raw(&path))
        } else {
            report.skip("plan:file", &["plan:path"]);
            None
        }
    } else {
        report.skip("plan:path", &["index:artifacts"]);
        report.skip("plan:file", &["plan:path"]);
        None
    };
    let instructions = LocalHierarchyCatalog {
        skill_root: skill_root.to_path_buf(),
    };
    let skills = LocalSkillCatalog {
        roots: configs.to_vec(),
    };
    let roots = configs
        .iter()
        .map(|config| SkillRoot {
            scope: config.scope.clone(),
            locator: config.locator.clone(),
        })
        .collect::<Vec<_>>();
    let paths = LocalPlanStorage {
        project_root: root.to_path_buf(),
    };
    let repository = LocalTaskStorage {
        project_root: root.to_path_buf(),
    };
    let plan_validation = if let (Some(raw), Some(path)) = (&plan_raw, plan_path) {
        report.check(
            "plan:contract",
            path,
            parse(raw).and_then(|plan| {
                validate_plan(
                    &instructions,
                    &skills,
                    &paths,
                    &roots,
                    &plan,
                    PlanValidationInput {
                        raw,
                        actual_plan_path: path,
                        allow_task_index: true,
                    },
                )
            }),
        )
    } else {
        report.skip("plan:contract", &["plan:file"]);
        None
    };
    if let (Some(validation), Some(value), Some(raw)) = (&plan_validation, &document, &plan_raw) {
        report.check(
            "plan:binding:canonical_sha256",
            "/source_plan/canonical_sha256",
            same(
                &value["source_plan"]["canonical_sha256"],
                &json!(sha256_hex(raw)),
                "source_plan_fingerprint_mismatch",
                "The TASK collection source Plan fingerprint does not match the Plan.",
            ),
        );
        report.check(
            "plan:binding:hierarchy_selection_sha256",
            "/source_plan/hierarchy_selection_sha256",
            same(
                &value["source_plan"]["hierarchy_selection_sha256"],
                &validation["hierarchy_selection_sha256"],
                "source_plan_hierarchy_selection_mismatch",
                "The TASK collection hierarchy selection fingerprint does not match the Plan.",
            ),
        );
    } else if plan_path.is_some() {
        report.skip("plan:binding", &["plan:contract", "index:source_plan"]);
    } else {
        report.skip("plan:binding", &["plan:contract"]);
    }
    let item_contracts_passed = references.is_some_and(|rows| {
        !rows.is_empty()
            && rows.iter().all(|row| {
                row["id"]
                    .as_str()
                    .is_some_and(|id| report.status(&format!("item:{id}:contract")) == "passed")
            })
    });
    let collection = if index_valid
        && item_contracts_passed
        && plan_validation.is_some()
        && report.status("items:directory") == "passed"
    {
        report.check(
            "collection:contract",
            raw_index_path,
            load_collection(
                &instructions,
                &skills,
                &paths,
                &repository,
                &roots,
                raw_index_path,
            ),
        )
    } else {
        report.skip(
            "collection:contract",
            &[
                "index:contract",
                "items:contracts",
                "items:directory",
                "plan:contract",
            ],
        );
        None
    };
    let mut binding = "not_checked";
    if let Some(collection) = &collection {
        let execution = document.as_ref().unwrap()["artifacts"]["execution"]
            .as_str()
            .unwrap_or("");
        let execution_index = format!("{execution}/index.json");
        let transactions = report.check(
            "transactions:path",
            execution,
            storage_path(root, execution),
        );
        if transactions.is_some() {
            report.check(
                "transactions",
                execution,
                require_no_spec_update(root, execution, None).map_err(|_| {
                    error(
                        ExitCode::LockConflict,
                        "task_repair_pending_transaction",
                        "An incomplete transaction permits diagnosis only.",
                        json!({}),
                    )
                }),
            );
        } else {
            report.skip("transactions", &["transactions:path"]);
        }
        let raw = report.check(
            "execution:file",
            &execution_index,
            storage_path(root, &execution_index).and_then(|path| LocalFiles.read_raw(&path)),
        );
        if let Some(raw) = raw {
            let value = parse(&raw);
            if let Ok(value) = value {
                let valid = report.check(
                    "execution:contract",
                    &execution_index,
                    validate_execution_index(&value, &raw).map_err(|issue| {
                        error(
                            ExitCode::Contract,
                            issue.reason_code,
                            issue.message,
                            issue.details,
                        )
                    }),
                );
                if valid.is_some() {
                    if let Ok(expected) = build_initial_execution_index(
                        &collection["collection_contract"],
                        collection,
                    ) {
                        let fields = [
                            "task_spec_id",
                            "task_collection_sha256",
                            "task_index_sha256",
                            "task_instructions_sha256",
                            "hierarchy_selection_sha256",
                            "skill_selection_sha256",
                        ];
                        let mut actual_binding = serde_json::Map::new();
                        let mut expected_binding = serde_json::Map::new();
                        for field in fields {
                            actual_binding.insert(field.into(), value[field].clone());
                            expected_binding.insert(field.into(), expected[field].clone());
                        }
                        for (name, source) in [
                            ("task_item_sha256", &value),
                            ("task_item_sha256", &expected),
                        ] {
                            let map = source["tasks"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|row| {
                                    Some((row["id"].as_str()?.to_owned(), row[name].clone()))
                                })
                                .collect::<serde_json::Map<_, _>>();
                            if std::ptr::eq(source, &value) {
                                actual_binding.insert(name.into(), Value::Object(map));
                            } else {
                                expected_binding.insert(name.into(), Value::Object(map));
                            }
                        }
                        report.check(
                            "execution:binding",
                            &execution_index,
                            same(
                                &Value::Object(actual_binding),
                                &Value::Object(expected_binding),
                                "execution_task_binding_mismatch",
                                "The execution index does not match the TASK collection.",
                            ),
                        );
                        binding = report.status("execution:binding");
                    }
                } else {
                    report.skip("execution:binding", &["execution:contract"]);
                }
            } else {
                report.failure("execution:contract", value.unwrap_err(), &execution_index);
                report.skip("execution:binding", &["execution:contract"]);
            }
        } else {
            report.skip("execution:contract", &["execution:file"]);
            report.skip("execution:binding", &["execution:contract"]);
        }
    } else {
        report.skip("execution:file", &["collection:contract"]);
        report.skip("execution:contract", &["execution:file"]);
        report.skip("execution:binding", &["execution:contract"]);
    }
    report.skip(
        "execution_index_binding",
        &["collection_execution_index_contract"],
    );
    let format_status = if ["index:encoding", "index:json", "index:normalization"]
        .iter()
        .all(|name| report.status(name) == "passed")
    {
        "passed"
    } else if ["index:encoding", "index:json", "index:normalization"]
        .iter()
        .any(|name| report.status(name) == "failed")
    {
        "failed"
    } else {
        "not_checked"
    };
    work_model::task::response::typed_response::<
        work_model::task::response::TaskCollectionDiagnostics,
    >(json!({"schema":"work-task-collection-diagnostics/v1",
        "status":if collection.is_some() {"valid"} else {"blocked"},
        "task_path":raw_index_path,"raw_sha256":index_raw.as_ref().map(|raw| sha256_hex(raw)),
        "format_status":format_status,"contract_status":report.status("collection:contract"),
        "normal_use_allowed":collection.is_some(),"execution_binding_status":binding,
        "repair_mode":"review_required","checks":report.checks,"issues":report.issues}))
}
