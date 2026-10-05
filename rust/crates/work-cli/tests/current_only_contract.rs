//! Local current-contract guards; historical evidence is allowed only by exact identity.
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use work_model::schema::PublicSchema;

const OLD_FIELDS: &[&str] = &[
    "compatibility_revision",
    "router_revision",
    "task_rules_sha256",
    "execute_rules_sha256",
    "task_sha256",
    "rule_selection",
    "source_plan_path",
    "draft_ref",
    "saved_progress",
    "source_progress_path",
];
const MARKERS: &[&str] = &[
    "WORK_DELEGATION",
    "WORK_TASK_SKILL",
    "WORK_ARTIFACT_EDIT",
    "WORK_PROGRESS_SAVE",
];
const RETIRED_SCHEMAS: &[&str] = &[
    "work-discussion-progress",
    "work-progress-prepare",
    "work-progress-preview",
    "work-progress-read",
    "work-progress-save-request",
    "work-progress-save",
    "work-task-draft-prepare",
    "work-task-draft-recovery",
    "work-task-draft-save",
    "work-task-draft-source-check",
    "work-task-draft-validation",
    "work-task-draft",
    "work-task-planning-index-validation",
    "work-task-planning-index",
    "work-task-semantic-request",
];
const RETIRED_EVIDENCE: &[(&str, &str)] = &[
    (
        "rust/crates/work-infrastructure/fixtures/delegation-role/progress-saver-expected.json",
        "7227c8da01d21946ee677b7c07d034f569d2855724add9e878ee60cbe0827166",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-assembly/draft.json",
        "83e1c9a4c677b18014e9dd3788f2c5c1b53fa5d69f9cbdff0399e72bb8a19f58",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-assembly/index.json",
        "7fbd7294fca31e21b226ea8ec25e2fa70ba3c92f5513a765a742c0f5d98160a3",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/valid/expected.json",
        "7ea4e4acfb5dea3b25b5dd7f145ba2c9840fd6d637d5db0b7c1a9e043934b3f4",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/source-drift/outputs/work/tasks/example/drafts/index.json",
        "a1d599387c37e1ff72d61186b56ad28edaea689a5d10e5da27f09859ecde4242",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/source-drift/outputs/work/tasks/example/drafts/history/1/index.json",
        "a1d599387c37e1ff72d61186b56ad28edaea689a5d10e5da27f09859ecde4242",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/missing-selection/outputs/work/tasks/example/drafts/index.json",
        "8a9139b81004b37e1b38a1894b3bdb34564f5e0f21f3b3106d2131320dbdbedb",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/missing-selection/outputs/work/tasks/example/drafts/history/1/index.json",
        "8a9139b81004b37e1b38a1894b3bdb34564f5e0f21f3b3106d2131320dbdbedb",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/valid/outputs/work/tasks/example/drafts/index.json",
        "d4e4f775a41e316f1955bf337b2d29cb963e07d7268755672039169e4d921764",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/valid/outputs/work/tasks/example/drafts/history/1/index.json",
        "d4e4f775a41e316f1955bf337b2d29cb963e07d7268755672039169e4d921764",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/delegation-role/outputs/work/progress/example/task/progress.json",
        "3dc23b07b4e169fabee7bb0fff9af7efea22964ee86e0bac204bb3e1f5bd2497",
    ),
];

const IMMUTABLE: &[(&str, &str)] = &[
    // Exact historical/source evidence files and their full SHA-256 are inserted at preparation.
    (
        "rust/crates/work-cli/tests/invalid_json.txt",
        "3c48773b404d850071dff4006d4ef0d7302d1343aefc58fbc84d730753de8831",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/delegation-role/legacy-plan/outputs/work/plans/example.json",
        "6e62f1f148ba58c554362529a568f6dfd1d57983f8ff7aa092baa0c7f7cb3d52",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/delegation-role/outputs/work/sources/example/SRC-001/source.txt",
        "c0025641ad55f45e002ef3307cbe976060548cd4f3a0f068f1bf4d614ea7a9a8",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/delegation-role/plan-expected.json",
        "d59260cad51d5350326077c8d89b735d1cd78aa8b8ff8c550126d39e8b4e36fb",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/delegation-role/task-skill/outputs/work/sources/example/SRC-001/source.txt",
        "c0025641ad55f45e002ef3307cbe976060548cd4f3a0f068f1bf4d614ea7a9a8",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/handoff-closed/blocked/outputs/work/plans/example.json",
        "477a9c931dc3e1aeb41b80d2fa6aa9486453b0aaa4a17e2778025c9236b6f673",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/handoff-closed/blocked/outputs/work/sources/example/SRC-001/source.txt",
        "7207f68cff308aed428d6c1a6b51f48c9778d933fe34dceb8bf26193e4ef87b9",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/handoff-closed/instruction-baseline/instructions.md",
        "4eb8ea4bccf08a049721ab8315b1cb3d992ebdaca2cdce5a8fe1cbd52490bba3",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/handoff-closed/instruction-baseline/references/execution-records.md",
        "fdb3e4239f73887f10244e1f766c99612746ca2164ded2959ea9f7dd58f5e302",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/handoff-closed/instruction-baseline/references/execution-recovery.md",
        "0b10178791148e51d0f1e46f413a6ebb430979d654df6874006b8a9c9b7a0092",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/handoff-closed/stopped/outputs/work/plans/example.json",
        "477a9c931dc3e1aeb41b80d2fa6aa9486453b0aaa4a17e2778025c9236b6f673",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/handoff-closed/stopped/outputs/work/sources/example/SRC-001/source.txt",
        "7207f68cff308aed428d6c1a6b51f48c9778d933fe34dceb8bf26193e4ef87b9",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-migration/outputs/work/plans/example.json",
        "32060424c0c7084b6301982b90f71472ee5abe5cec3f2e9712bfa4e6134e3e5c",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-migration/reconstruction/outputs/work/plans/example.json",
        "06159f62f16157a896a81d7211baf91d7be55cb8fd8e776dda0a9656fd2fac16",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-reconciliation/attempt-raw.txt",
        "1d566b34bf20c67837166d44cfb00f4b9a2a3fbaf3b31d67caf0dffba52f18a7",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-reconciliation/real-flow/with-migration/outputs/work/executions/example/.work-spec-migration-8A3356E4F4D2.json",
        "b1a58d432675b12e1ff986b470b89a658b20882c40feffe73a96ce4486421058",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-reconciliation/real-flow/with-migration/outputs/work/plans/example.json",
        "32060424c0c7084b6301982b90f71472ee5abe5cec3f2e9712bfa4e6134e3e5c",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-update/add-task/outputs/work/sources/example/SRC-001/source.txt",
        "c0025641ad55f45e002ef3307cbe976060548cd4f3a0f068f1bf4d614ea7a9a8",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-update/item-goal/outputs/work/sources/example/SRC-001/source.txt",
        "c0025641ad55f45e002ef3307cbe976060548cd4f3a0f068f1bf4d614ea7a9a8",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-update/outputs/work/sources/example/SRC-001/source.txt",
        "c0025641ad55f45e002ef3307cbe976060548cd4f3a0f068f1bf4d614ea7a9a8",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-update/remove-task/outputs/work/sources/example/SRC-001/source.txt",
        "c0025641ad55f45e002ef3307cbe976060548cd4f3a0f068f1bf4d614ea7a9a8",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-update/revision-migration/outputs/work/plans/example.json",
        "32060424c0c7084b6301982b90f71472ee5abe5cec3f2e9712bfa4e6134e3e5c",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/specification-update/task-summary/outputs/work/sources/example/SRC-001/source.txt",
        "c0025641ad55f45e002ef3307cbe976060548cd4f3a0f068f1bf4d614ea7a9a8",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-assembly/outputs/work/sources/example/SRC-001/source.txt",
        "7207f68cff308aed428d6c1a6b51f48c9778d933fe34dceb8bf26193e4ef87b9",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-assembly/plan.json",
        "ff2063a3da86ffda334b109b7e6afecd4a1ea5dfd172e977ff0742154f5c1c2d",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-diagnostics/outputs/work/sources/example/SRC-001/source.txt",
        "c0025641ad55f45e002ef3307cbe976060548cd4f3a0f068f1bf4d614ea7a9a8",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/missing-selection/outputs/work/sources/example/SRC-001/source.txt",
        "7207f68cff308aed428d6c1a6b51f48c9778d933fe34dceb8bf26193e4ef87b9",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/source-drift/outputs/work/sources/example/SRC-001/source.txt",
        "813cdc534521528983b3ea97ea55cab8f6e7a806ab9ec0e84407a4b6832737e2",
    ),
    (
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/valid/outputs/work/sources/example/SRC-001/source.txt",
        "7207f68cff308aed428d6c1a6b51f48c9778d933fe34dceb8bf26193e4ef87b9",
    ),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn current_schema(value: &str) -> bool {
    (!value.starts_with("work-") || !value.contains('/')) && !RETIRED_SCHEMAS.contains(&value)
}

fn inspect_value(value: &Value, file: &str, pointer: &str) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                let at = format!("{pointer}/{key}");
                if OLD_FIELDS.contains(&key.as_str()) {
                    return Err(format!("{file}:{at}: retired field"));
                }
                if matches!(
                    key.as_str(),
                    "schema" | "id" | "reference" | "$ref" | "target_schema"
                ) {
                    if let Some(schema) = value.as_str() {
                        let rejected_plan = file
                            == "rust/crates/work-infrastructure/fixtures/specification-migration/invalid-plan-request.json"
                            && at == "$/candidates/0/content/schema"
                            && schema == "work-plan/v1";
                        if !current_schema(schema) && !rejected_plan {
                            return Err(format!("{file}:{at}: versioned schema"));
                        }
                    }
                }
                if key == "marker"
                    && value.as_str().is_some_and(|marker| {
                        MARKERS
                            .iter()
                            .any(|current| marker.starts_with(current) && marker != *current)
                    })
                {
                    return Err(format!("{file}:{at}: versioned role marker"));
                }
                if !matches!(key.as_str(), "raw" | "source_bytes" | "base64") {
                    inspect_value(value, file, &at)?;
                }
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                inspect_value(value, file, &format!("{pointer}/{index}"))?;
            }
        }
        Value::String(text)
            if pointer.ends_with("/stdout") && text.trim_start().starts_with('{') =>
        {
            if let Ok(decoded) = serde_json::from_str::<Value>(text) {
                inspect_value(&decoded, file, &format!("{pointer}/decoded"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn files(directory: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            result.extend(files(&path));
        } else if path.is_file() {
            result.push(path);
        }
    }
    result.sort();
    result
}

#[test]
fn registry_all_serde_and_queries_accept_only_one_current_identity() {
    let registry = work_model::contract_data::registry();
    let ids: BTreeSet<_> = registry
        .catalog
        .contracts
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    let all: BTreeSet<_> = PublicSchema::ALL
        .iter()
        .map(|schema| schema.as_str())
        .collect();
    assert_eq!(ids, all);
    assert_eq!(registry.items.len(), ids.len());
    inspect_value(
        &work_model::contract_data::registry_value(),
        "registry",
        "$",
    )
    .unwrap();
    for schema in PublicSchema::ALL {
        let id = schema.as_str();
        assert!(current_schema(id), "{id}");
        assert_eq!(serde_json::to_value(schema).unwrap(), id);
        assert_eq!(
            serde_json::from_value::<PublicSchema>(json!(id)).unwrap(),
            schema
        );
        assert_eq!(work_cli::contract::describe(id).unwrap()["id"], id);
        let entry = registry
            .catalog
            .contracts
            .iter()
            .find(|entry| entry.id == id)
            .unwrap();
        if entry.caller_constructible {
            inspect_value(&work_cli::contract::scaffold(id).unwrap(), id, "$").unwrap();
        }
        for version in [1, 2] {
            let alias = format!("{id}/v{version}");
            assert!(
                serde_json::from_value::<PublicSchema>(json!(alias)).is_err(),
                "{alias}"
            );
            assert!(work_cli::contract::describe(&alias).is_err(), "{alias}");
            assert!(work_cli::contract::scaffold(&alias).is_err(), "{alias}");
        }
    }
}

fn reject_replacement<T: DeserializeOwned>(id: &str, field: &str, retired: &str) {
    let mut value =
        work_model::contract_data::registry_value()["items"][id]["description"]["example"].clone();
    assert!(serde_json::from_value::<T>(value.clone()).is_ok(), "{id}");
    let contents = value.as_object_mut().unwrap().remove(field).unwrap();
    value[retired] = contents;
    assert!(
        serde_json::from_value::<T>(value).is_err(),
        "{id}: {retired}"
    );
}

#[test]
fn artifact_models_have_no_retired_field_alias_or_fallback() {
    reject_replacement::<work_model::task::item::TaskItem>(
        "work-task-item",
        "instruction_selection",
        "rule_selection",
    );
    reject_replacement::<work_model::execution::attempt::Attempt>(
        "work-attempt",
        "task_item_sha256",
        "task_sha256",
    );
    reject_replacement::<work_model::execution::index::ExecutionIndex>(
        "work-execution-index",
        "task_instructions_sha256",
        "task_rules_sha256",
    );
    reject_replacement::<work_model::execution::correction::Correction>(
        "work-correction",
        "execute_instructions_sha256",
        "execute_rules_sha256",
    );
}

#[test]
fn current_fixtures_and_embedded_stdout_are_checked_with_exact_evidence_exceptions() {
    let root = root();
    let immutable: BTreeMap<_, _> = IMMUTABLE.iter().copied().collect();
    for (relative, digest) in &immutable {
        let raw = fs::read(root.join(relative)).unwrap();
        assert_eq!(
            work_infrastructure::fixture_support::raw_sha256(&raw),
            *digest,
            "{relative}"
        );
    }
    let mut checked = 0;
    for path in files(&root.join("rust/crates")) {
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        if let Some((_, digest)) = RETIRED_EVIDENCE.iter().find(|(file, _)| *file == relative) {
            let raw = fs::read(&path).unwrap();
            assert_eq!(
                work_infrastructure::fixture_support::raw_sha256(&raw),
                *digest,
                "{relative}: exact rejected legacy evidence"
            );
            let value: Value = serde_json::from_slice(&raw).unwrap();
            assert!(
                inspect_value(&value, &relative, "$").is_err() || value["role"] == "progress-saver",
                "legacy fixture must be rejected: {relative}"
            );
            checked += 1;
            continue;
        }
        if immutable.contains_key(relative.as_str()) {
            continue;
        }
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap())
            .unwrap_or_else(|error| panic!("{relative}: {error}"));
        inspect_value(&value, &relative, "$").unwrap();
        checked += 1;
    }
    assert!(
        checked >= 182,
        "fixture/config coverage unexpectedly shrank: {checked}"
    );
}

fn production_is_current(text: &str) -> bool {
    let production = text.split("#[cfg(test)]\nmod ").next().unwrap();
    let retired_helpers = [
        "python_missing_reason",
        "python_choice_reason",
        "python_conflict_reason",
        "python_unknown_reason",
        "python_usage_reason",
        "stdin_removed",
        "refresh-preview",
        "refresh-apply",
        "refresh-recover",
    ];
    !OLD_FIELDS
        .iter()
        .chain(retired_helpers.iter())
        .any(|token| production.contains(token))
        && production
            .split('"')
            .all(|token| !token.starts_with("work-") || !token.contains("/v"))
        && MARKERS
            .iter()
            .all(|marker| !production.contains(&format!("{marker}_V")))
}

#[test]
fn production_sources_do_not_reintroduce_old_contract_branches() {
    for path in files(&root().join("rust/crates")) {
        if path.extension().is_some_and(|extension| extension == "rs")
            && path
                .components()
                .any(|component| component.as_os_str() == "src")
        {
            assert!(
                production_is_current(&fs::read_to_string(&path).unwrap()),
                "{}",
                path.display()
            );
        }
    }
}

#[test]
fn instruction_command_tree_has_no_fresh_compatibility_writers() {
    let tree: Value = serde_json::from_str(include_str!("../src/parser/commands.json")).unwrap();
    let node = tree["root"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["name"] == "instructions")
        .unwrap();
    let names: BTreeSet<_> = node["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        BTreeSet::from(["catalog", "resolve", "load", "select", "impact", "recover"])
    );
}

fn clean_installer(text: &str) -> bool {
    !text.contains("refresh_existing_instructions")
        && !text.contains("cp -R -p -- \"$final_work/.\"")
        && !text.contains("xcopy \"!final_work!")
        && !text.contains("Previously installed branches and stale files will be kept")
        && text.contains("prepared")
        && text.contains("previous")
}

#[test]
fn installer_and_negative_probe_guards_detect_regressions_without_mutating_the_repository() {
    for file in [
        "os-scripts/mac/install-work.command",
        "os-scripts/windows/install-work.bat",
    ] {
        let text = fs::read_to_string(root().join(file)).unwrap();
        assert!(clean_installer(&text), "{file}");
        assert!(
            !clean_installer(&format!("{text}\nrefresh_existing_instructions")),
            "{file}"
        );
    }
    for value in [
        json!({"schema":"work-task-index/v1"}),
        json!({"schema":"work-new-producer/v99"}),
        json!({"reference":"work-task-index/v1"}),
        json!({"stdout": "{\"schema\":\"work-cli-result/v1\"}"}),
        json!({"schema":"work-task-index","rule_selection":{}}),
        json!({"marker":"WORK_DELEGATION_V1"}),
        json!({"marker":"WORK_TASK_SKILL_V1"}),
        json!({"marker":"WORK_ARTIFACT_EDIT_V1"}),
        json!({"marker":"WORK_PROGRESS_SAVE_V1"}),
    ] {
        assert!(
            inspect_value(&value, "temporary-probe.json", "$").is_err(),
            "{value}"
        );
    }
    for text in [
        "#[serde(alias = \"work-task-index/v1\")]",
        "value.get(\"rule_selection\").or_else(|| value.get(\"instruction_selection\"))",
        "fn python_missing_reason() {}",
    ] {
        assert!(!production_is_current(text), "{text}");
    }
    assert!(
        inspect_value(
            &json!({"schema":"work-task-index","raw": {"schema":"work-task-index/v1"}}),
            "raw-proof.json",
            "$"
        )
        .is_ok()
    );
    assert!(inspect_value(&json!({"schema":"work-plan/v1"}), "rust/crates/work-infrastructure/fixtures/specification-migration/invalid-plan-request.json", "$/candidates/0/content").is_ok());
    for frame in [
        "WORK-INSTRUCTIONS-SHA-256-V1",
        "WORK-TASK-COLLECTION-CREATE-V1",
        "WORK-SKILL-BUNDLE-SHA-256-V1",
        "WORK-SKILL-IDENTITY-V1",
        "WORK-TASK-DRAFT-APPROVAL-V1",
        "WORK-MIGRATION-SOURCE-APPROVAL-V1",
    ] {
        assert!(inspect_value(&json!({"hash_domain":frame}), "frame.json", "$").is_ok());
    }
    assert!(
        inspect_value(
            &json!({"external_protocol":"Git porcelain v1"}),
            "external.json",
            "$"
        )
        .is_ok()
    );
}

// Rebuilds must not accumulate unreferenced transactions in golden fixtures.
#[test]
fn reconciliation_fixtures_keep_only_reviewed_journals() {
    let base = root()
        .join("rust/crates/work-infrastructure/fixtures/specification-reconciliation/real-flow");
    let historical =
        "with-migration/outputs/work/executions/example/.work-spec-migration-8A3356E4F4D2.json";
    let mut expected = BTreeSet::from([historical.to_owned()]);
    for case in [base.clone(), base.join("with-migration")] {
        let publication: Value =
            serde_json::from_slice(&fs::read(case.join("publication.json")).unwrap()).unwrap();
        let path = case.join(publication["publication"]["journal"].as_str().unwrap());
        let journal: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(journal["schema"], "work-spec-transaction");
        assert_eq!(journal["state"], "published");
        expected.insert(
            path.strip_prefix(&base)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/"),
        );
    }
    let actual: BTreeSet<_> = files(&base)
        .into_iter()
        .filter(|path| {
            let name = path.file_name().unwrap().to_str().unwrap();
            name.starts_with(".work-spec-migration-") && name.ends_with(".json")
        })
        .map(|path| {
            path.strip_prefix(&base)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(actual, expected, "unreviewed fixture journal accumulation");
}

#[test]
fn retired_discussion_lifecycles_have_no_public_schema_or_private_role() {
    for id in RETIRED_SCHEMAS {
        assert!(
            serde_json::from_value::<PublicSchema>(json!(id)).is_err(),
            "{id}"
        );
        assert!(
            !PublicSchema::ALL
                .iter()
                .any(|schema| schema.as_str() == *id)
        );
        let registry = serde_json::to_value(work_model::contract_data::registry()).unwrap();
        assert!(registry["items"].get(*id).is_none(), "{id}");
    }
    let storage = work_infrastructure::delegation_storage::LocalDelegationStorage {
        project_root: root(),
        skill_root: root().join("skills/work"),
        skill_configs: vec![],
    };
    let rejected = work_flow::delegation::build(&storage, &json!({"schema":"work-delegation-build-request","role":"progress-saver","request":"Retired role"})).unwrap_err();
    assert_eq!(rejected.reason_code, "delegation_boundary_mismatch");
}

#[test]
fn active_session_fixture_has_exact_integrity_without_repairing_it() {
    let path =
        root().join("rust/crates/work-infrastructure/fixtures/discussion-assembly/session.json");
    let value: work_model::discussion::DiscussionSession =
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(
        value.commit.content_sha256,
        work_infrastructure::fixture_support::discussion_sha256(&value)
    );
}
