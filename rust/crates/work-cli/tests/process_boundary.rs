//! Contracts that require launching the public executable.

use std::path::Path;
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::{fs, path::PathBuf};

use serde_json::Value;
use serde_json::json;
use work_flow::error::ExitCode;
use work_infrastructure::writer_lock::{LocalWriterLock, WriterLock};

fn executable() -> &'static str {
    static EXECUTABLE: OnceLock<String> = OnceLock::new();
    EXECUTABLE
        .get_or_init(|| {
            std::env::var("WORK_TEST_EXECUTABLE")
                .unwrap_or_else(|_| env!("CARGO_BIN_EXE_work").to_owned())
        })
        .as_str()
}

fn project_root() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repository root")
        .to_string_lossy()
        .into_owned()
}

fn installed_executable() -> &'static PathBuf {
    static INSTALLED: OnceLock<PathBuf> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        fn copy_tree(source: &Path, destination: &Path) {
            fs::create_dir_all(destination).unwrap();
            for entry in fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let target = destination.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy_tree(&entry.path(), &target);
                } else {
                    fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        let skill = std::env::temp_dir().join(format!(
            "work-process-installed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let binary = skill.join("scripts/work");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        fs::copy(executable(), &binary).unwrap();
        copy_tree(
            &PathBuf::from(project_root()).join("skills/work/references"),
            &skill.join("references"),
        );
        binary
    })
}

fn run(arguments: &[String]) -> Output {
    Command::new(installed_executable())
        .args(arguments)
        .output()
        .expect("launch work")
}

#[test]
fn invocation_confirmation_is_bound_read_only_and_cannot_forge_explicit_origin() {
    let root = std::env::temp_dir().join(format!(
        "work-invocation-confirm-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("request.json");
    let request = "  完整需求 e\u{0301}\r\n$HOME $(command) -- \"literal\"\n";
    let value = json!({"mode":"task","request":request,"confirmation":{"mode":"task","request":request,"confirmed":true,"evidence":"User approved task mode and this exact request."}});
    let invoke = || {
        Command::new(executable())
            .args([
                "--project-root",
                root.to_str().unwrap(),
                "--verbose",
                "invocation",
                "confirm",
                "--input-file",
                input.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let raw = serde_json::to_vec(&value).unwrap();
    fs::write(&input, &raw).unwrap();
    let output = invoke();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["data"]["origin"], "implicit_confirmed");
    assert_eq!(response["data"]["request"], request);
    assert_eq!(response["data"]["confirmation"], value["confirmation"]);
    assert_eq!(fs::read(&input).unwrap(), raw);
    for request in [
        "example",
        "resume example",
        "討論需求",
        "$work execute -- opaque",
    ] {
        let value = json!({"mode":"task","request":request,"confirmation":{
            "mode":"task","request":request,"confirmed":true,"evidence":"Approved exact input."}});
        fs::write(&input, serde_json::to_vec(&value).unwrap()).unwrap();
        let output = invoke();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["data"]["request"], request);
        assert_eq!(result["data"]["origin"], "implicit_confirmed");
    }
    for case in [
        "unconfirmed",
        "changed_request",
        "forged_origin",
        "missing_confirmation",
        "removed_mode",
    ] {
        let mut invalid = value.clone();
        match case {
            "unconfirmed" => invalid["confirmation"]["confirmed"] = json!(false),
            "changed_request" => invalid["confirmation"]["request"] = json!("Changed"),
            "forged_origin" => invalid["origin"] = json!("explicit"),
            "missing_confirmation" => {
                invalid.as_object_mut().unwrap().remove("confirmation");
            }
            "removed_mode" => invalid["mode"] = json!("plan"),
            _ => unreachable!(),
        }
        let raw = serde_json::to_vec(&invalid).unwrap();
        fs::write(&input, &raw).unwrap();
        assert!(!invoke().status.success(), "{case}");
        assert_eq!(fs::read(&input).unwrap(), raw);
    }
    let duplicate = br#"{"mode":"task","mode":"execute","request":"Request","confirmation":{"mode":"execute","request":"Request","confirmed":true,"evidence":"Approved"}}"#;
    fs::write(&input, duplicate).unwrap();
    assert!(!invoke().status.success());
    assert_eq!(fs::read(&input).unwrap(), duplicate);
    let raw = b"$work task --  original request\n";
    fs::write(&input, raw).unwrap();
    let explicit = Command::new(executable())
        .args([
            "--project-root",
            root.to_str().unwrap(),
            "--verbose",
            "invocation",
            "parse",
            "--input-file",
            input.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(explicit.status.success());
    let response: Value = serde_json::from_slice(&explicit.stdout).unwrap();
    assert_eq!(response["data"]["origin"], "explicit");
    assert_eq!(response["data"]["request"], "  original request\n");
    assert_eq!(fs::read(&input).unwrap(), raw);
    fs::write(&input, b"$work plan -- removed").unwrap();
    let removed = Command::new(executable())
        .args([
            "--project-root",
            root.to_str().unwrap(),
            "invocation",
            "parse",
            "--input-file",
            input.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!removed.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&removed.stdout).unwrap()["reason_code"],
        "work_invocation_mode_invalid"
    );
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn attempt_cli_preserves_verified_acceptance_evidence_and_rejects_unexecuted_val() {
    let root = std::env::temp_dir().join(format!(
        "work-acceptance-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("attempt.json");
    let mut attempt=work_model::contract_data::registry_value()["items"]["work-attempt"]["description"]["example"].clone();
    attempt["authorization"]["validations"] = json!([{"id":"VAL-001","kind":"manual","confirmer":"user","criteria":"Actual result verified.","acceptance_ids":["ACCEPTANCE-001"]}]);
    attempt["authorization_sha256"] = json!(
        work_infrastructure::fixture_support::structured_sha256(&attempt["authorization"])
    );
    attempt["records"] = json!([{"id":"VAL-001","kind":"validation","outcome":"passed","evidence":"Actual result verified."}]);
    attempt["acceptance_results"] = json!([{"id":"ACCEPTANCE-001","status":"completed","evidence":[{"task_id":attempt["task_id"],"attempt_id":attempt["attempt_id"],"validation_id":"VAL-001","record_id":"VAL-001","outcome":"passed","evidence":"Actual result verified.","task_item_sha256":attempt["task_item_sha256"],"task_instructions_sha256":attempt["task_instructions_sha256"]}]}]);
    fs::write(&input, serde_json::to_vec(&attempt).unwrap()).unwrap();
    let invoke = || {
        Command::new(executable())
            .args([
                "--project-root",
                root.to_str().unwrap(),
                "--verbose",
                "attempt",
                "render",
                "--input-file",
                input.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };
    let output = invoke();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["data"], attempt);
    let rendered = String::from_utf8(output.stdout).unwrap();
    assert!(
        rendered.find("\"records\"").unwrap() < rendered.find("\"acceptance_results\"").unwrap()
    );
    attempt["records"] = json!([]);
    let raw = serde_json::to_vec(&attempt).unwrap();
    fs::write(&input, &raw).unwrap();
    let rejected = invoke();
    assert!(!rejected.status.success());
    let response: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(response["reason_code"], "acceptance_evidence_not_recorded");
    assert_eq!(fs::read(&input).unwrap(), raw);
    assert!(!root.join("outputs/work/executions").exists());
}

#[test]
fn public_paths_resolve_exposes_only_source_task_and_execution() {
    let output = run(&[
        "--project-root".into(),
        project_root(),
        "paths".into(),
        "resolve".into(),
        "--requirement-id".into(),
        "example".into(),
    ]);
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        response["data"]["paths"],
        json!({
            "source": "outputs/work/sources/example",
            "task": "outputs/work/tasks/example/index.json",
            "execution": "outputs/work/executions/example"
        })
    );
    assert!(response["data"]["paths"].get("plan").is_none());
}

#[test]
fn artifact_render_data_round_trips_without_cli_envelope() {
    let registry: Value = work_model::contract_data::registry_value();
    let base = std::env::temp_dir().join(format!(
        "work-render-round-trip-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&base).unwrap();
    for (command, contract_id, ordered) in [
        ("attempt", "work-attempt", Some("attempt_id")),
        ("correction", "work-correction", Some("correction_id")),
        ("handoff", "work-handoff", None),
    ] {
        let fixture_root = PathBuf::from(project_root())
            .join("rust/crates/work-infrastructure/fixtures/handoff-closed/stopped");
        let example: Value = if command == "handoff" {
            serde_json::from_slice(
                &fs::read(fixture_root.join("execute_to_task-expected.json")).unwrap(),
            )
            .unwrap()
        } else {
            registry["items"][contract_id]["description"]["example"].clone()
        };
        let input = base.join(format!("{command}.json"));
        fs::write(&input, serde_json::to_vec(&example).unwrap()).unwrap();
        let args = |action: &str| {
            vec![
                "--project-root".to_owned(),
                if command == "handoff" {
                    fixture_root.to_string_lossy().into_owned()
                } else {
                    project_root()
                },
                "--verbose".to_owned(),
                command.to_owned(),
                action.to_owned(),
                "--input-file".to_owned(),
                input.to_string_lossy().into_owned(),
            ]
        };
        let rendered = run(&args("render"));
        assert!(
            rendered.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&rendered.stdout)
        );
        assert!(rendered.stderr.is_empty(), "{command}");
        let response: Value = serde_json::from_slice(&rendered.stdout).unwrap();
        assert_eq!(response["data"], example, "{command}");
        let text = String::from_utf8(rendered.stdout).unwrap();
        if let Some(field) = ordered {
            let data = text.split_once("\"data\":").unwrap().1;
            assert!(
                data.find("\"schema\"").unwrap() < data.find(&format!("\"{field}\"")).unwrap(),
                "{command}: canonical leading fields"
            );
        }
        fs::write(
            &input,
            work_infrastructure::codec::canonical_json(&response["data"]).unwrap(),
        )
        .unwrap();
        let validated = run(&args("validate"));
        assert!(
            validated.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&validated.stdout)
        );
        assert!(validated.stderr.is_empty(), "{command}");
    }
}

#[test]
fn task_write_commands_reject_legacy_single_file_before_publication() {
    let registry: Value = work_model::contract_data::registry_value();
    let base = std::env::temp_dir().join(format!(
        "work-legacy-task-write-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let project = base.join("project");
    let plan_path = "outputs/work/plans/example.json";
    let task_path = "outputs/work/tasks/example/task.json";
    let execution = "outputs/work/executions/example";
    let artifacts = json!({"plan":plan_path,"task":task_path,"execution":execution});
    let plan = project.join(plan_path);
    fs::create_dir_all(plan.parent().unwrap()).unwrap();
    let plan_raw = serde_json::to_vec(&json!({"schema":"work-plan/v1",
        "requirement_id":"example","artifacts":artifacts}))
    .unwrap();
    fs::write(&plan, &plan_raw).unwrap();
    let cases = [
        (
            "prepare",
            registry["items"]["work-spec-prepare-request"]["description"]["example"].clone(),
            vec![],
        ),
        (
            "preview",
            json!({"task_index":{"artifacts":{"task":task_path}}}),
            vec![],
        ),
    ];
    let input = base.join("request.json");
    for (command, request, flags) in cases {
        fs::write(&input, serde_json::to_vec(&request).unwrap()).unwrap();
        let mut args = vec![
            "--project-root".to_owned(),
            project.to_string_lossy().into_owned(),
            (if matches!(command, "prepare" | "preview") {
                "specification"
            } else {
                "task"
            })
            .to_owned(),
            command.to_owned(),
            "--input-file".to_owned(),
            input.to_string_lossy().into_owned(),
            "--user-config-root".to_owned(),
            project.to_string_lossy().into_owned(),
        ];
        args.extend(flags.into_iter().map(str::to_owned));
        let output = run(&args);
        assert_eq!(
            output.status.code(),
            Some(if command == "prepare" { 5 } else { 6 }),
            "{command}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stderr.is_empty(), "{command}");
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            response["reason_code"],
            if command == "prepare" {
                "spec_task_source_missing"
            } else {
                "task_collection_required"
            },
            "{command}"
        );
        assert_eq!(fs::read(&plan).unwrap(), plan_raw, "{command}");
        assert!(!project.join("outputs/work/tasks").exists(), "{command}");
    }
}

#[test]
fn removed_task_commands_return_cli_usage_error() {
    for command in [
        "prepare",
        "status",
        "save",
        "draft-init",
        "semantic-prepare",
        "draft-save",
        "draft-recover",
        "draft-read",
        "draft-status",
        "draft-check",
        "draft-save-request",
        "draft-recover-request",
        "draft-list-update",
        "draft-list-recover",
        "draft-source-update",
        "draft-source-recover",
        "draft-assemble",
        "draft-create",
        "create",
        "recover-create",
    ] {
        let output = run(&[
            "--project-root".into(),
            project_root(),
            "--verbose".into(),
            "task".into(),
            command.into(),
        ]);
        assert_eq!(output.status.code(), Some(2), "{command}");
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["reason_code"], "cli_usage_error", "{command}");
    }
}

#[test]
fn removed_migration_semantic_commands_return_cli_usage_error() {
    for command in [
        "semantic-prepare",
        "semantic-preview",
        "semantic-apply",
        "semantic-recover",
    ] {
        let output = run(&[
            "--project-root".into(),
            project_root(),
            "migration".into(),
            command.into(),
        ]);
        assert_eq!(output.status.code(), Some(2), "{command}");
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["reason_code"], "cli_usage_error", "{command}");
    }
}
#[test]
fn removed_task_diagnose_is_rejected() {
    let output = run(&[
        "--project-root".into(),
        project_root(),
        "task".into(),
        "diagnose".into(),
    ]);
    assert_ne!(output.status.code(), Some(0));
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["reason_code"], "cli_usage_error");
}

#[test]
fn unsupported_stdin_options_reject_before_publication() {
    let root = std::env::temp_dir().join(format!(
        "work-unknown-stdin-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let sentinel = root.join("existing.txt");
    fs::write(
        &sentinel,
        b"preserved
",
    )
    .unwrap();
    for option in ["--stdin", "--stdin=true"] {
        let actual = Command::new(installed_executable())
            .args([
                "--project-root",
                root.to_str().unwrap(),
                "paths",
                "resolve",
                "--requirement-id",
                "example",
                option,
            ])
            .stdin(std::process::Stdio::piped())
            .output()
            .unwrap();
        assert_eq!(actual.status.code(), Some(2));
        assert!(actual.stderr.is_empty());
        let response: Value = serde_json::from_slice(&actual.stdout).unwrap();
        assert_eq!(response["reason_code"], "cli_usage_error");
        assert!(response["data"].get("replacement").is_none());
        assert_eq!(
            fs::read(&sentinel).unwrap(),
            b"preserved
"
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }
}

#[test]
fn frozen_cli_responses_match_at_process_boundary() {
    let fixtures: Value =
        serde_json::from_str(include_str!("process_baseline.json")).expect("frozen fixtures");
    let root = project_root();
    let encoded_root = serde_json::to_string(&root).expect("JSON root");
    let encoded_placeholder = serde_json::to_string("<PROJECT_ROOT>").expect("JSON placeholder");
    for case in fixtures["cli_cases"].as_array().expect("CLI cases") {
        let arguments = case["argv"]
            .as_array()
            .expect("argv")
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .expect("argument")
                    .replace("<PROJECT_ROOT>", &root)
            })
            .collect::<Vec<_>>();
        let actual = run(&arguments);
        let name = case["name"].as_str().expect("name");
        assert_eq!(
            actual.status.code(),
            case["exit_code"].as_i64().map(|v| v as i32),
            "{name}: exit code"
        );
        assert_eq!(
            String::from_utf8(actual.stdout)
                .expect("UTF-8 stdout")
                .replace(&encoded_root, &encoded_placeholder),
            case["stdout"].as_str().expect("stdout"),
            "{name}: stdout"
        );
        assert_eq!(
            actual.stderr,
            case["stderr"].as_str().expect("stderr").as_bytes(),
            "{name}: stderr"
        );
    }
}

fn check_help(node: &Value, path: &mut Vec<String>, count: &mut usize) {
    let children = node["children"].as_array().expect("children");
    if children.is_empty() {
        let mut arguments = path.clone();
        arguments.push("--help".to_owned());
        let actual = run(&arguments);
        assert_eq!(actual.status.code(), Some(0), "{path:?}: exit code");
        assert!(actual.stderr.is_empty(), "{path:?}: stderr");
        let response: Value = serde_json::from_slice(&actual.stdout).expect("help response");
        assert_eq!(response["data"]["help"], node["help"], "{path:?}: help");
        *count += 1;
    }
    for child in children {
        path.push(child["name"].as_str().expect("name").to_owned());
        check_help(child, path, count);
        path.pop();
    }
}

#[test]
fn every_public_leaf_has_frozen_help_at_process_boundary() {
    let manifest: Value = serde_json::from_str(include_str!("../src/parser/commands.json"))
        .expect("command manifest");
    let mut count = 0;
    check_help(&manifest["root"], &mut Vec::new(), &mut count);
    assert_eq!(count, 82);
}

#[test]
fn public_command_tree_matches_frozen_baseline() {
    fn collect(node: &Value, path: &mut Vec<String>, commands: &mut Vec<String>) {
        let children = node["children"].as_array().unwrap();
        if children.is_empty() {
            commands.push(path.join(" "));
        }
        for child in children {
            path.push(child["name"].as_str().unwrap().to_owned());
            collect(child, path, commands);
            path.pop();
        }
    }

    let manifest: Value =
        serde_json::from_str(include_str!("../src/parser/commands.json")).unwrap();
    let mut commands = Vec::new();
    collect(&manifest["root"], &mut Vec::new(), &mut commands);
    commands.sort();
    assert_eq!(commands.len(), 82);
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.starts_with("task "))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "task apply",
            "task preview",
            "task recover",
            "task validate"
        ]
    );
    let migration = [
        "migration analyze",
        "migration prepare",
        "migration preview",
        "migration apply",
        "migration verify",
        "migration recover",
    ];
    for new_command in migration {
        assert!(commands.contains(&new_command.to_owned()));
    }
    for command in [
        "task preview",
        "task apply",
        "task recover",
        "specification prepare",
        "specification preview",
        "specification apply",
        "specification verify",
        "specification recover",
        "specification reconciliation-prepare",
        "specification reconciliation-preview",
        "specification reconciliation-apply",
        "specification reconciliation-recover",
    ] {
        assert!(commands.contains(&command.to_owned()));
    }
    for removed in [
        "task prepare",
        "task save",
        "task status",
        "progress prepare",
        "progress read",
        "progress save",
        "progress preview",
        "task spec-prepare",
        "task spec-validate",
        "task spec-update",
        "task spec-verify",
        "task spec-recover",
        "task reconciliation-prepare",
        "task reconciliation-preview",
        "task reconciliation-apply",
        "task reconciliation-recover",
        "migration semantic-prepare",
        "migration semantic-preview",
        "migration semantic-apply",
        "migration semantic-recover",
    ] {
        assert!(!commands.contains(&removed.to_owned()));
    }
    for command in [
        "source capture",
        "source read",
        "source validate",
        "invocation confirm",
        "instructions recover",
    ] {
        assert!(commands.contains(&command.to_owned()));
    }
    let legacy = commands
        .into_iter()
        .filter(|command| {
            !migration.contains(&command.as_str())
                && !command.starts_with("source ")
                && command != "invocation confirm"
                && command != "specification reconciliation-recover"
                && command != "instructions recover"
        })
        .collect::<Vec<_>>();
    assert_eq!(legacy.len(), 70);
    let raw = format!("{}\n", legacy.join("\n"));
    assert_eq!(
        work_infrastructure::fixture_support::raw_sha256(raw.as_bytes()),
        "786b0d4857be2f2c699d78539d92a43b5d59dadc59899eb204a6c98b7b16b474"
    );
}

#[test]
fn specification_recover_dispatches_to_specification_validation() {
    let project = std::env::temp_dir().join(format!(
        "work-specification-recover-cli-{}",
        std::process::id()
    ));
    fs::create_dir_all(&project).unwrap();
    let input = project.join("request.json");
    fs::write(&input, b"{}").unwrap();
    let output = run(&[
        "--project-root".into(),
        project.to_string_lossy().into_owned(),
        "specification".into(),
        "recover".into(),
        "--input-file".into(),
        input.to_string_lossy().into_owned(),
        "--user-config-root".into(),
        project.to_string_lossy().into_owned(),
        "--approved-sha256".into(),
        "a".repeat(64),
    ]);
    assert_eq!(output.status.code(), Some(ExitCode::WorkflowState as i32));
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["reason_code"], "task_collection_required");
}

#[test]
fn migration_preview_uses_public_mode_and_rejects_task_entry() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../work-infrastructure/fixtures/specification-migration");
    let project = std::env::temp_dir().join(format!(
        "work-semantic-migration-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for relative in [
        "outputs/work/plans/example.json",
        "outputs/work/tasks/example/index.json",
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/executions/example/index.json",
    ] {
        let destination = project.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(fixture.join(relative), destination).unwrap();
    }
    let project_arg = project.to_string_lossy().into_owned();
    let request_arg = fixture.join("request.json").to_string_lossy().into_owned();
    let arguments = [
        "--project-root",
        project_arg.as_str(),
        "migration",
        "preview",
        "--input-file",
        request_arg.as_str(),
        "--user-config-root",
        project_arg.as_str(),
    ]
    .map(str::to_owned);
    let output = run(&arguments);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["data"]["schema"], "work-spec-migration-preview");

    let request: Value = serde_json::from_slice(&fs::read(&request_arg).unwrap()).unwrap();
    assert!(
        request["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["kind"] != "plan")
    );
    let original_plan = fs::read(project.join("outputs/work/plans/example.json")).unwrap();
    for (fixture_name, status) in [
        ("incomplete-candidate-set-request.json", Some("blocked")),
        ("invalid-execution-binding-request.json", Some("blocked")),
        ("invalid-plan-request.json", None),
    ] {
        let mut negative = arguments.to_vec();
        negative[5] = fixture.join(fixture_name).to_string_lossy().into_owned();
        let output = run(&negative);
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        if let Some(status) = status {
            assert!(output.status.success());
            assert_eq!(response["data"]["status"], status);
        } else {
            assert_eq!(output.status.code(), Some(ExitCode::Contract as i32));
            assert_eq!(response["reason_code"], "invalid_contract_value");
        }
    }
    assert_eq!(
        fs::read(project.join("outputs/work/plans/example.json")).unwrap(),
        original_plan
    );

    let mut legacy = arguments.to_vec();
    legacy[2] = "task".into();
    legacy[3] = "migration-preview".into();
    let rejected = run(&legacy);
    assert_eq!(rejected.status.code(), Some(ExitCode::CliUsage as i32));
}

#[test]
fn migration_public_lifecycle_handles_revision_and_reconstruction() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../work-infrastructure/fixtures");
    for (name, relative, has_execution) in [
        ("revision", "specification-update/revision-migration", true),
        (
            "reconstruction",
            "specification-migration/reconstruction",
            false,
        ),
    ] {
        let fixture = fixtures.join(relative);
        let project = std::env::temp_dir().join(format!(
            "work-migration-public-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut sources = vec![
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
        ];
        if has_execution {
            sources.push("outputs/work/executions/example/index.json");
        }
        for relative in sources {
            let destination = project.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), destination).unwrap();
        }
        if has_execution {
            fs::write(project.join("src.txt"), b"source\n").unwrap();
        }
        let project_arg = project.to_string_lossy().into_owned();
        let semantic_arg = fixture
            .join("semantic-request.json")
            .to_string_lossy()
            .into_owned();
        let prepared_path = project.join("migration-request.json");
        let prepared_arg = prepared_path.to_string_lossy().into_owned();
        let semantic: Value = serde_json::from_slice(&fs::read(&semantic_arg).unwrap()).unwrap();
        let original_sources: Vec<_> = semantic["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| {
                let path = project.join(source["path"].as_str().unwrap());
                let raw = fs::read(&path).unwrap();
                (path, raw)
            })
            .collect();
        for (case, expected_code, expected_reason) in [
            ("missing", ExitCode::Contract, "invalid_contract_value"),
            ("plan", ExitCode::Contract, "invalid_contract_value"),
            (
                "unresolved",
                ExitCode::ArtifactIntegrity,
                "migration_semantic_decisions_unresolved",
            ),
            (
                "drift",
                ExitCode::ArtifactIntegrity,
                "migration_source_changed",
            ),
        ] {
            let mut invalid = semantic.clone();
            match case {
                "missing" => {
                    invalid.as_object_mut().unwrap().remove("sources");
                }
                "plan" => {
                    invalid["plan"] = json!({});
                }
                "unresolved" => {
                    invalid["semantic_decisions"] =
                        json!([{"id":"D-1","question":"Choose meaning","resolution":null}]);
                }
                "drift" => {
                    invalid["sources"][0]["raw_sha256"] = json!("0".repeat(64));
                }
                _ => unreachable!(),
            }
            let negative_path = project.join(format!("{case}-semantic.json"));
            fs::write(&negative_path, serde_json::to_vec(&invalid).unwrap()).unwrap();
            let output = run(&[
                "--project-root".into(),
                project_arg.clone(),
                "migration".into(),
                "prepare".into(),
                "--input-file".into(),
                negative_path.to_string_lossy().into_owned(),
                "--output-file".into(),
                prepared_arg.clone(),
                "--user-config-root".into(),
                project_arg.clone(),
            ]);
            let response: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(
                output.status.code(),
                Some(expected_code as i32),
                "{case}: {response}"
            );
            assert_eq!(response["reason_code"], expected_reason);
            assert!(!prepared_path.exists());
            for (path, raw) in &original_sources {
                assert_eq!(fs::read(path).unwrap(), *raw);
            }
        }
        let invoke = |command: &str, input_file: &str, approval: Option<&str>| {
            let mut args = vec![
                "--project-root".into(),
                project_arg.clone(),
                "--verbose".into(),
                "migration".into(),
                command.into(),
                "--input-file".into(),
                input_file.into(),
                "--user-config-root".into(),
                project_arg.clone(),
            ];
            if command == "prepare" {
                args.extend(["--output-file".into(), prepared_arg.clone()]);
            }
            if let Some(approved) = approval {
                args.extend(["--approved-sha256".into(), approved.into()]);
            }
            let output = run(&args);
            let response: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(output.status.success(), "{name} {command}: {response}");
            response["data"].clone()
        };
        let prepared = invoke("prepare", &semantic_arg, None);
        assert_eq!(
            prepared["request"]["schema"],
            "work-spec-migration-preview-request"
        );
        assert_eq!(prepared["preview"]["status"], "ready");
        let preview = invoke("preview", &prepared_arg, None);
        assert_eq!(preview, prepared["preview"]);
        let approved = preview["fingerprint"].as_str().unwrap();
        let approval_rejected = run(&[
            "--project-root".into(),
            project_arg.clone(),
            "migration".into(),
            "apply".into(),
            "--input-file".into(),
            prepared_arg.clone(),
            "--approved-sha256".into(),
            "0".repeat(64),
            "--user-config-root".into(),
            project_arg.clone(),
        ]);
        let rejected: Value = serde_json::from_slice(&approval_rejected.stdout).unwrap();
        assert_eq!(
            approval_rejected.status.code(),
            Some(ExitCode::ArtifactIntegrity as i32)
        );
        assert_eq!(rejected["reason_code"], "migration_approval_changed");
        for (path, raw) in &original_sources {
            assert_eq!(fs::read(path).unwrap(), *raw);
        }
        if name == "reconstruction" {
            let retained = preview["diffs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["path"] == "outputs/work/plans/example.json")
                .unwrap();
            assert_eq!(retained["operation"], "replace");
            assert_eq!(retained["unified_diff"], "");
        }
        let unpublished = run(&[
            "--project-root".into(),
            project_arg.clone(),
            "migration".into(),
            "verify".into(),
            "--input-file".into(),
            prepared_arg.clone(),
            "--approved-sha256".into(),
            approved.into(),
        ]);
        let unpublished_error: Value = serde_json::from_slice(&unpublished.stdout).unwrap();
        assert_eq!(
            unpublished_error["reason_code"],
            "migration_verify_result_missing"
        );
        let applied = invoke("apply", &prepared_arg, Some(approved));
        assert_eq!(applied["publication_status"], "published");
        let journal_path = project.join(applied["journal"].as_str().unwrap());
        let journal_before = fs::read(&journal_path).unwrap();
        let plan_path = project.join("outputs/work/plans/example.json");
        let plan_before = fs::read(&plan_path).unwrap();
        let verified = invoke("verify", &prepared_arg, Some(approved));
        assert_eq!(verified["schema"], "work-spec-migration-verification");
        assert_eq!(verified["status"], "valid");
        assert_eq!(verified["mode"], "semantic");
        assert_eq!(fs::read(&journal_path).unwrap(), journal_before);
        assert_eq!(fs::read(&plan_path).unwrap(), plan_before);
        let recovered = invoke("recover", &prepared_arg, Some(approved));
        assert_eq!(recovered["publication_status"], "already_published");
        let damaged_path = project.join("outputs/work/tasks/example/index.json");
        fs::write(&damaged_path, b"damaged\n").unwrap();
        let invalid = run(&[
            "--project-root".into(),
            project_arg.clone(),
            "migration".into(),
            "verify".into(),
            "--input-file".into(),
            prepared_arg.clone(),
            "--approved-sha256".into(),
            approved.into(),
        ]);
        let error: Value = serde_json::from_slice(&invalid.stdout).unwrap();
        assert_eq!(error["reason_code"], "migration_verify_installed_mismatch");
        assert_eq!(fs::read(&damaged_path).unwrap(), b"damaged\n");
        assert_eq!(fs::read(&journal_path).unwrap(), journal_before);
        let marker_path = project.join(applied["completion_marker"].as_str().unwrap());
        fs::write(&marker_path, b"bad marker\n").unwrap();
        let marker_result = run(&[
            "--project-root".into(),
            project_arg.clone(),
            "migration".into(),
            "verify".into(),
            "--input-file".into(),
            prepared_arg.clone(),
            "--approved-sha256".into(),
            approved.into(),
        ]);
        let marker_error: Value = serde_json::from_slice(&marker_result.stdout).unwrap();
        assert_eq!(
            marker_error["reason_code"],
            "migration_verify_marker_mismatch"
        );
        assert_eq!(fs::read(&marker_path).unwrap(), b"bad marker\n");
        fs::write(&journal_path, b"not json\n").unwrap();
        let corrupt_result = run(&[
            "--project-root".into(),
            project_arg.clone(),
            "migration".into(),
            "verify".into(),
            "--input-file".into(),
            prepared_arg.clone(),
            "--approved-sha256".into(),
            approved.into(),
        ]);
        let corrupt_error: Value = serde_json::from_slice(&corrupt_result.stdout).unwrap();
        assert_eq!(
            corrupt_error["reason_code"],
            "migration_verify_result_invalid"
        );
        assert_eq!(fs::read(&journal_path).unwrap(), b"not json\n");
    }
}

#[test]
fn removed_rules_command_returns_public_usage_error() {
    let root = project_root();
    let result = run(&[
        "--project-root".into(),
        root.clone(),
        "rules".into(),
        "resolve".into(),
        "--user-config-root".into(),
        root,
        "--work-directory".into(),
        "task".into(),
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stderr.is_empty());
    let response: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(response["schema"], "work-cli-result");
    assert_eq!(response["reason_code"], "cli_usage_error");
    assert!(
        response["data"]["reason"]
            .as_str()
            .unwrap()
            .contains("rules")
    );
}

#[test]
fn handoff_invalid_json_fails_before_source_lookup() {
    let root = project_root();
    let base = std::env::temp_dir().join(format!(
        "work-handoff-invalid-json-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&base).unwrap();
    let input = base.join("request.json");
    fs::write(&input, "{").unwrap();
    for command in ["validate", "build-task-to-execute"] {
        let mut arguments = vec![
            "--project-root".into(),
            root.clone(),
            "handoff".into(),
            command.into(),
            "--input-file".into(),
            input.to_string_lossy().into_owned(),
        ];
        if command == "build-task-to-execute" {
            arguments.extend([
                "--task-path".into(),
                "outputs/work/tasks/missing/index.json".into(),
                "--task-id".into(),
                "TASK-001".into(),
                "--user-config-root".into(),
                root.clone(),
            ]);
        }
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(3), "{command}");
        assert!(output.stderr.is_empty(), "{command}");
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["schema"], "work-cli-result", "{command}");
        assert_eq!(
            response["reason_code"], "invalid_json_contract",
            "{command}"
        );
    }
}

#[test]
fn handoff_build_output_validates_across_installed_processes() {
    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let destination = target.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &destination);
            } else {
                fs::copy(entry.path(), destination).unwrap();
            }
        }
    }
    let repo = PathBuf::from(project_root());
    let base = std::env::temp_dir().join(format!(
        "work-handoff-process-t25-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let skill = base.join("work");
    let installed = skill.join("scripts/work");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::copy(executable(), &installed).unwrap();
    copy_tree(
        &repo.join("skills/work/references"),
        &skill.join("references"),
    );
    let fixture = repo.join("rust/crates/work-infrastructure/fixtures/task-diagnostics");
    let project = base.join("project");
    let task_path = "outputs/work/tasks/example/index.json";
    for relative in [task_path, "outputs/work/tasks/example/tasks/TASK-001.json"] {
        let target = project.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(fixture.join(relative), target).unwrap();
    }
    work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &project).unwrap();
    let original = fs::read(project.join(task_path)).unwrap();
    let request = base.join("request.json");
    fs::write(&request, r#"{"summary":"Start execution."}"#).unwrap();
    let built = Command::new(&installed)
        .args([
            "--project-root",
            project.to_str().unwrap(),
            "--verbose",
            "handoff",
            "build-task-to-execute",
            "--input-file",
            request.to_str().unwrap(),
            "--task-path",
            task_path,
            "--task-id",
            "TASK-001",
            "--user-config-root",
            project.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(built.status.code(), Some(0));
    assert!(built.stderr.is_empty());
    let response: Value = serde_json::from_slice(&built.stdout).unwrap();
    assert_eq!(response["schema"], "work-cli-result");
    assert_eq!(response["data"]["schema"], "work-handoff");
    let handoff = base.join("handoff.json");
    fs::write(&handoff, serde_json::to_vec(&response["data"]).unwrap()).unwrap();
    let validated = Command::new(&installed)
        .args([
            "--project-root",
            project.to_str().unwrap(),
            "handoff",
            "validate",
            "--input-file",
            handoff.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(validated.status.code(), Some(0));
    assert!(validated.stderr.is_empty());
    let checked: Value = serde_json::from_slice(&validated.stdout).unwrap();
    assert_eq!(checked["data"]["status"], "valid");
    assert_eq!(fs::read(project.join(task_path)).unwrap(), original);
    assert!(!project.join("outputs/work/plans").exists());
    assert!(!project.join("outputs/work/executions").exists());

    for relative in [
        task_path,
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/executions/example/index.json",
    ] {
        let destination = project.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(
            repo.join("rust/crates/work-infrastructure/fixtures/task-diagnostics")
                .join(relative),
            &destination,
        )
        .unwrap();
    }
    work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &project).unwrap();
    let run_handoff = |root: &Path, command: &str, input: &Path, flags: &[&str]| -> Value {
        let mut arguments = vec![
            "--project-root",
            root.to_str().unwrap(),
            "--verbose",
            "handoff",
            command,
            "--input-file",
            input.to_str().unwrap(),
            "--user-config-root",
            root.to_str().unwrap(),
        ];
        arguments.extend_from_slice(flags);
        let output = Command::new(&installed).args(arguments).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{command}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stderr.is_empty(), "{command}");
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        response["data"].clone()
    };
    let return_request = json!({"summary":"Review specification.",
        "confirmed_approach":"Retain the interface.",
        "requested_changes":["Clarify scope."],"preserve":["Current behavior."],
        "affected_ids":["TASK-001"],"validation_requirements":["Review criteria."]});
    let mut preflight_request = return_request.clone();
    preflight_request["reason"] = json!("Specification defect.");
    for (build, verify, semantic, build_flags, verify_flags) in [
        (
            "build-task-to-execute",
            "verify-task-to-execute",
            json!({"summary":"Start execution."}),
            vec!["--task-path", task_path, "--task-id", "TASK-001"],
            vec!["--task-path", task_path, "--task-id", "TASK-001"],
        ),
        (
            "build-execute-to-task",
            "verify-execute-to-task",
            preflight_request.clone(),
            vec![
                "--task-path",
                task_path,
                "--task-id",
                "TASK-001",
                "--preflight",
            ],
            vec![
                "--task-path",
                task_path,
                "--task-id",
                "TASK-001",
                "--preflight",
            ],
        ),
    ] {
        fs::write(&request, serde_json::to_vec(&semantic).unwrap()).unwrap();
        let built = run_handoff(&project, build, &request, &build_flags);
        assert_eq!(built["schema"], "work-handoff", "{build}");
        assert_eq!(built["artifacts"]["task"], task_path, "{build}");
        fs::write(&handoff, serde_json::to_vec(&built).unwrap()).unwrap();
        let checked = run_handoff(&project, verify, &handoff, &verify_flags);
        assert_eq!(checked["status"], "valid", "{verify}");
        assert_eq!(checked["source"], built["source"], "{verify}");
    }
    assert_eq!(fs::read(project.join(task_path)).unwrap(), original);

    let custom_project = base.join("custom-project");
    let custom_task_path = "custom/tasks/example/index.json";
    let custom_artifacts = json!({"source":"custom/sources/example","task":custom_task_path,"execution":"custom/executions/example"});
    let mut custom_index: Value =
        serde_json::from_slice(&fs::read(project.join(task_path)).unwrap()).unwrap();
    custom_index["artifacts"] = json!({"source":"custom/sources/example","task":custom_task_path,"execution":"custom/executions/example"});
    work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &custom_project).unwrap();
    fs::create_dir_all(custom_project.join("custom/sources")).unwrap();
    fs::rename(
        custom_project.join("outputs/work/sources/example"),
        custom_project.join("custom/sources/example"),
    )
    .unwrap();
    let custom_index_raw =
        work_infrastructure::fixture_support::render_task_index(&custom_index).unwrap();
    let custom_index_file = custom_project.join(custom_task_path);
    fs::create_dir_all(custom_index_file.parent().unwrap()).unwrap();
    fs::write(&custom_index_file, &custom_index_raw).unwrap();
    let item_file = custom_index_file
        .parent()
        .unwrap()
        .join("tasks/TASK-001.json");
    fs::create_dir_all(item_file.parent().unwrap()).unwrap();
    fs::copy(
        project.join("outputs/work/tasks/example/tasks/TASK-001.json"),
        &item_file,
    )
    .unwrap();
    {
        let (build, verify, semantic, build_flags, verify_flags) = (
            "build-task-to-execute",
            "verify-task-to-execute",
            json!({"summary":"Start execution."}),
            vec!["--task-path", custom_task_path, "--task-id", "TASK-001"],
            vec!["--task-path", custom_task_path, "--task-id", "TASK-001"],
        );
        fs::write(&request, serde_json::to_vec(&semantic).unwrap()).unwrap();
        let built = run_handoff(&custom_project, build, &request, &build_flags);
        assert_eq!(built["artifacts"], custom_artifacts, "{build}");
        fs::write(&handoff, serde_json::to_vec(&built).unwrap()).unwrap();
        let checked = run_handoff(&custom_project, verify, &handoff, &verify_flags);
        assert_eq!(checked["status"], "valid", "{verify}");
    }
    assert_eq!(fs::read(&custom_index_file).unwrap(), custom_index_raw);

    let closed_project = base.join("closed-project");
    let closed_fixture =
        repo.join("rust/crates/work-infrastructure/fixtures/handoff-closed/stopped");
    for relative in [
        task_path,
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/tasks/example/tasks/TASK-002.json",
        "outputs/work/executions/example/index.json",
        "outputs/work/executions/example/TASK-001/ATTEMPT-004/attempt.json",
    ] {
        let target = closed_project.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(closed_fixture.join(relative), target).unwrap();
        work_infrastructure::fixture_support::copy_fixture_sources(
            &closed_fixture,
            &closed_project,
        )
        .unwrap();
    }
    let closed_request = json!({"summary":"調整已確認範圍", "reason":"Clarify specification",
        "confirmed_approach":"保留既有介面", "requested_changes":["新增驗收條件"],
        "preserve":["既有功能"], "affected_ids":["ACCEPTANCE-001","TASK-001"],
        "validation_requirements":["重新確認驗收條件"]});
    fs::write(&request, serde_json::to_vec(&closed_request).unwrap()).unwrap();
    let drifted = Command::new(&installed)
        .args([
            "--project-root",
            closed_project.to_str().unwrap(),
            "handoff",
            "build-execute-to-task",
            "--input-file",
            request.to_str().unwrap(),
            "--task-path",
            task_path,
            "--task-id",
            "TASK-001",
            "--attempt-id",
            "ATTEMPT-004",
            "--user-config-root",
            closed_project.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(drifted.status.code(), Some(5));
    let drift_response: Value = serde_json::from_slice(&drifted.stdout).unwrap();
    assert_eq!(
        drift_response["reason_code"],
        "handoff_execute_instructions_changed"
    );
    work_infrastructure::fixture_support::restore_historical_execute_instructions(&skill).unwrap();

    {
        let (build, verify) = ("build-execute-to-task", "verify-execute-to-task");
        let built = run_handoff(
            &closed_project,
            build,
            &request,
            &[
                "--task-path",
                task_path,
                "--task-id",
                "TASK-001",
                "--attempt-id",
                "ATTEMPT-004",
            ],
        );
        assert_eq!(
            built["source"]["execution_context"]["phase"], "execution",
            "{build}"
        );
        fs::write(&handoff, serde_json::to_vec(&built).unwrap()).unwrap();
        let checked = run_handoff(
            &closed_project,
            verify,
            &handoff,
            &[
                "--task-path",
                task_path,
                "--task-id",
                "TASK-001",
                "--attempt-id",
                "ATTEMPT-004",
            ],
        );
        assert_eq!(checked["status"], "valid", "{verify}");
        assert_eq!(checked["source"], built["source"], "{verify}");
    }
    for command in [
        "build-plan-to-task",
        "verify-plan-to-task",
        "build-task-to-plan",
        "verify-task-to-plan",
        "build-execute-to-plan",
        "verify-execute-to-plan",
    ] {
        let output = Command::new(&installed)
            .args([
                "--project-root",
                project.to_str().unwrap(),
                "handoff",
                command,
                "--input-file",
                request.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{command}");
    }
    for command in [
        "build-task-to-execute",
        "verify-task-to-execute",
        "build-execute-to-task",
        "verify-execute-to-task",
    ] {
        let mut invocation = Command::new(&installed);
        invocation.args([
            "--project-root",
            project.to_str().unwrap(),
            "handoff",
            command,
            "--input-file",
            request.to_str().unwrap(),
            "--user-config-root",
            project.to_str().unwrap(),
            "--task-path",
            task_path,
            "--task-id",
            "TASK-001",
            "--plan-path",
            "legacy.json",
        ]);
        if command.ends_with("-to-task") {
            invocation.arg("--preflight");
        }
        let output = invocation.output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{command}");
    }
    assert!(!project.join("outputs/work/plans").exists());
}
#[test]
fn task_delegation_uses_fixed_source_without_plan_and_rejects_legacy_context() {
    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let destination = target.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &destination);
            } else {
                fs::copy(entry.path(), destination).unwrap();
            }
        }
    }
    let repo = PathBuf::from(project_root());
    let base = std::env::temp_dir().join(format!(
        "work-task-delegation-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for role in ["task-coordinator", "task-skill"] {
        let fixture = repo.join(if role == "task-skill" {
            "rust/crates/work-infrastructure/fixtures/delegation-role/task-skill"
        } else {
            "rust/crates/work-infrastructure/fixtures/task-diagnostics"
        });
        let project = base.join(role);
        copy_tree(
            &fixture.join("outputs/work/sources"),
            &project.join("outputs/work/sources"),
        );
        let index: Value = serde_json::from_slice(
            &fs::read(fixture.join("outputs/work/tasks/example/index.json")).unwrap(),
        )
        .unwrap();
        if role == "task-skill" {
            copy_tree(
                &fixture.join("outputs/work/tasks"),
                &project.join("outputs/work/tasks"),
            );
            copy_tree(&fixture.join("skills"), &project.join("skills"));
        }
        let mut request = json!({"schema":"work-delegation-build-request","role":role,"request":"Preserve the confirmed requirement.","repository_evidence":["Reviewed src.txt."],"saved_discussion":["Preserve the interface."]});
        if role == "task-coordinator" {
            request["planning_source"] = json!({"snapshot":index["source"]["manifest"],"artifacts":index["artifacts"],"hierarchy_selection":index["hierarchy_selection"],"skill_selection":index["skill_selection"],"acceptance_criteria":index["acceptance_criteria"]});
        } else {
            request["task_path"] = json!("outputs/work/tasks/example/index.json");
            request["task_id"] = json!("TASK-001");
        }
        let input = project.join("request.json");
        let envelope_file = project.join("envelope.json");
        let source_dir = project.join("outputs/work/sources/example/SRC-001");
        let original_source = ["manifest.json", "manifest.json.done", "source.txt"]
            .map(|name| (name, fs::read(source_dir.join(name)).unwrap()));
        let skill_root = format!(
            "repo:delegation-fixture={}",
            project.join("skills").display()
        );
        let build = |value: &Value| {
            fs::write(&input, serde_json::to_vec(value).unwrap()).unwrap();
            let mut command = Command::new(installed_executable());
            command.args([
                "--project-root",
                project.to_str().unwrap(),
                "--verbose",
                "delegation",
                "build",
                "--input-file",
                input.to_str().unwrap(),
            ]);
            if role == "task-skill" {
                command.args(["--skill-root", &skill_root]);
            }
            let output = command.output().unwrap();
            let response: Value = serde_json::from_slice(&output.stdout).unwrap();
            (output.status.code(), response)
        };
        let (code, response) = build(&request);
        assert_eq!(code, Some(0), "{role}: {response}");
        let envelope = &response["data"];
        assert!(envelope["context"].get("source_plan").is_none());
        assert_eq!(
            envelope["context"]["task_source"]["source"],
            index["source"]
        );
        assert_eq!(
            envelope["context"]["repository_evidence"],
            request["repository_evidence"]
        );
        assert_eq!(
            envelope["context"]["saved_discussion"],
            request["saved_discussion"]
        );
        assert_eq!(
            envelope["context"]["task_source"]["source_bytes"],
            json!(original_source[2].1)
        );
        fs::write(&envelope_file, serde_json::to_vec(envelope).unwrap()).unwrap();
        let sender = if role == "task-skill" {
            "task-coordinator"
        } else {
            "parent"
        };
        let validate = |value: &Value| {
            fs::write(&envelope_file, serde_json::to_vec(value).unwrap()).unwrap();
            let output = Command::new(installed_executable())
                .args([
                    "--project-root",
                    project.to_str().unwrap(),
                    "--verbose",
                    "delegation",
                    "validate",
                    "--input-file",
                    envelope_file.to_str().unwrap(),
                    "--role",
                    role,
                    "--sender",
                    sender,
                ])
                .output()
                .unwrap();
            (
                output.status.code(),
                serde_json::from_slice::<Value>(&output.stdout).unwrap(),
            )
        };
        let (code, validated) = validate(envelope);
        assert_eq!(code, Some(0), "{role}: {validated}");
        assert_eq!(validated["data"]["grants_authorization"], false);
        let mut legacy = request.clone();
        legacy["source_plan_path"] = json!("missing-plan.json");
        assert_eq!(build(&legacy).0, Some(ExitCode::Contract as i32));
        let mut mixed = envelope.clone();
        mixed["context"]["source_plan"] = json!({});
        assert_eq!(validate(&mixed).0, Some(ExitCode::Contract as i32));
        let mut drifted = envelope.clone();
        drifted["context"]["task_source"]["source_bytes"][0] = json!(0);
        assert_eq!(validate(&drifted).0, Some(ExitCode::Contract as i32));
        let mut changed = original_source[2].1.clone();
        changed[0] ^= 1;
        fs::write(source_dir.join("source.txt"), changed).unwrap();
        assert_eq!(build(&request).0, Some(ExitCode::ArtifactIntegrity as i32));
        fs::write(source_dir.join("source.txt"), &original_source[2].1).unwrap();
        for (name, raw) in original_source {
            assert_eq!(fs::read(source_dir.join(name)).unwrap(), raw);
        }
        assert!(!project.join("outputs/work/plans").exists());
        assert!(!project.join("outputs/work/executions").exists());
        if role == "task-coordinator" {
            assert!(!project.join("outputs/work/tasks").exists());
        }
    }
}

#[test]
fn specification_task_boundary_continuation_across_installed_processes() {
    use work_flow::task::validate_collection;
    use work_infrastructure::fixture_support::{
        build_initial_execution_index, render_execution_index, render_task_index,
    };
    use work_infrastructure::hierarchy_catalog::LocalHierarchyCatalog;
    use work_infrastructure::skill_catalog::LocalSkillCatalog;
    use work_infrastructure::task::storage::LocalTaskStorage;

    let repo = PathBuf::from(project_root());
    let fixture =
        repo.join("rust/crates/work-infrastructure/fixtures/specification-update/task-summary");
    for drift in [false, true] {
        let base = std::env::temp_dir().join(format!(
            "work-spec-continuation-t25-{}-{}-{drift}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let project = base.join("project");
        for relative in [
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
        ] {
            let target = project.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), target).unwrap();
            work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &project).unwrap();
        }
        fs::write(project.join("src.txt"), b"source\n").unwrap();
        let item_path = project.join("outputs/work/tasks/example/tasks/TASK-001.json");
        let source_path = project.join("outputs/work/sources/example/SRC-001/source.txt");
        let source_raw = fs::read(&source_path).unwrap();
        let index_path = project.join("outputs/work/tasks/example/index.json");
        let index: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
        fs::write(&index_path, render_task_index(&index).unwrap()).unwrap();
        let skill = repo.join("skills/work");
        let collection = validate_collection(
            &LocalHierarchyCatalog {
                skill_root: skill.clone(),
            },
            &LocalSkillCatalog { roots: vec![] },
            &work_infrastructure::artifact_paths::LocalArtifactPaths {
                project_root: project.clone(),
            },
            &LocalTaskStorage {
                project_root: project.clone(),
            },
            &[],
            "outputs/work/tasks/example/index.json",
        )
        .unwrap();
        let execution =
            build_initial_execution_index(&collection["collection_contract"], &collection).unwrap();
        let execution_path = project.join("outputs/work/executions/example/index.json");
        fs::create_dir_all(execution_path.parent().unwrap()).unwrap();
        fs::write(execution_path, render_execution_index(&execution).unwrap()).unwrap();
        let request_path = base.join("semantic.json");
        fs::write(
            &request_path,
            serde_json::to_vec(&json!({
                "schema":"work-spec-prepare-request","requirement_id":"example",
                "reason":"Confirm the constraint wording and save progress.",
                "edits":[{"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"goal",
                    "after":"Confirmed boundary"}]
            }))
            .unwrap(),
        )
        .unwrap();
        let prepared_path = base.join("prepared.json");
        let invoke = |arguments: &[&str], expected: i32| -> Value {
            let output = Command::new(installed_executable())
                .args(["--project-root", project.to_str().unwrap(), "--verbose"])
                .args(arguments)
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(expected),
                "{arguments:?}: {output:?}"
            );
            assert!(output.stderr.is_empty());
            serde_json::from_slice::<Value>(&output.stdout).unwrap()
        };
        let prepared = invoke(
            &[
                "specification",
                "prepare",
                "--input-file",
                request_path.to_str().unwrap(),
                "--output-file",
                prepared_path.to_str().unwrap(),
                "--summary",
                "--user-config-root",
                project.to_str().unwrap(),
            ],
            0,
        );
        assert_eq!(
            prepared["data"]["changed_fields"],
            json!(["/task_items/TASK-001/goal"])
        );
        assert!(prepared["data"].get("request").is_none());
        let validated = invoke(
            &[
                "specification",
                "preview",
                "--input-file",
                prepared_path.to_str().unwrap(),
                "--summary",
                "--user-config-root",
                project.to_str().unwrap(),
            ],
            0,
        );
        let spec_approval = validated["data"]["approved_sha256"]
            .as_str()
            .unwrap()
            .to_owned();
        let published = invoke(
            &[
                "specification",
                "apply",
                "--input-file",
                prepared_path.to_str().unwrap(),
                "--approved-sha256",
                &spec_approval,
                "--user-config-root",
                project.to_str().unwrap(),
            ],
            0,
        );
        assert_eq!(published["data"]["status"], "updated");
        let verify_path = base.join("verify.json");
        fs::write(
            &verify_path,
            serde_json::to_vec(&published["data"]["verification_request"]).unwrap(),
        )
        .unwrap();
        if drift {
            let mut changed: Value =
                serde_json::from_slice(&fs::read(&item_path).unwrap()).unwrap();
            changed["goal"] = json!("Unexpected drift after publication.");
            fs::write(&item_path, serde_json::to_vec_pretty(&changed).unwrap()).unwrap();
            let rejected = invoke(
                &[
                    "specification",
                    "verify",
                    "--input-file",
                    verify_path.to_str().unwrap(),
                    "--user-config-root",
                    project.to_str().unwrap(),
                ],
                ExitCode::ArtifactIntegrity as i32,
            );
            assert_eq!(rejected["reason_code"], "spec_verify_state_changed");
            assert!(
                !project
                    .join("outputs/work/progress/example/task/progress.json")
                    .exists()
            );
        } else {
            let verified = invoke(
                &[
                    "specification",
                    "verify",
                    "--input-file",
                    verify_path.to_str().unwrap(),
                    "--user-config-root",
                    project.to_str().unwrap(),
                ],
                0,
            );
            assert_eq!(verified["data"]["verified"], true);
            assert!(!project.join("outputs/work/plans").exists());
            for field in ["requirement_id", "source", "task"] {
                let mut wrong = published["data"]["verification_request"].clone();
                if field == "requirement_id" {
                    wrong[field] = json!("other");
                } else {
                    wrong["artifacts"][field] =
                        json!(format!("outputs/work/other/{field}/example"));
                }
                fs::write(&verify_path, serde_json::to_vec(&wrong).unwrap()).unwrap();
                let rejected = invoke(
                    &[
                        "specification",
                        "verify",
                        "--input-file",
                        verify_path.to_str().unwrap(),
                        "--user-config-root",
                        project.to_str().unwrap(),
                    ],
                    5,
                );
                assert_eq!(rejected["reason_code"], "spec_verify_record_mismatch");
            }

            let discussion_fixture =
                repo.join("rust/crates/work-infrastructure/fixtures/discussion-assembly");
            let session = work_infrastructure::fixture_support::prepare_discussion_fixture(
                &discussion_fixture,
                &project,
                &skill,
            )
            .unwrap();
            let request = base.join("session-init.json");
            fs::write(&request,serde_json::to_vec(&json!({"schema":"work-discussion-request","requirement_id":"example","command":{"kind":"init","session":session}})).unwrap()).unwrap();
            let saved = invoke(
                &[
                    "discussion",
                    "init",
                    "--input-file",
                    request.to_str().unwrap(),
                    "--user-config-root",
                    project.to_str().unwrap(),
                ],
                0,
            );
            fs::write(&request,serde_json::to_vec(&json!({"schema":"work-discussion-request","requirement_id":"example","command":{"kind":"read"}})).unwrap()).unwrap();
            let restored = invoke(
                &[
                    "discussion",
                    "read",
                    "--input-file",
                    request.to_str().unwrap(),
                    "--user-config-root",
                    project.to_str().unwrap(),
                ],
                0,
            );
            assert_eq!(
                saved["data"]["data"]["view"]["content_sha256"],
                restored["data"]["data"]["content_sha256"]
            );
            assert_eq!(restored["data"]["data"]["revision"], 1);
            assert_eq!(fs::read(&source_path).unwrap(), source_raw);
            let installed: Value = serde_json::from_slice(&fs::read(&item_path).unwrap()).unwrap();
            assert_eq!(installed["goal"], "Confirmed boundary");
        }
    }
}

#[test]
fn every_public_command_help_returns_one_json_response() {
    fn check(node: &Value, path: &mut Vec<String>, count: &mut usize) {
        let mut arguments = path.clone();
        arguments.push("--help".into());
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(0), "{path:?}");
        assert!(output.stderr.is_empty(), "{path:?}");
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["schema"], "work-cli-result");
        assert!(
            response["data"]["help"]
                .as_str()
                .unwrap()
                .contains("usage:")
        );
        *count += 1;
        for child in node["children"].as_array().unwrap() {
            path.push(child["name"].as_str().unwrap().to_owned());
            check(child, path, count);
            path.pop();
        }
    }

    let manifest: Value =
        serde_json::from_str(include_str!("../src/parser/commands.json")).unwrap();
    let mut count = 0;
    check(&manifest["root"], &mut Vec::new(), &mut count);
    assert_eq!(count, 102);
}

#[test]
fn relative_unicode_request_path_uses_child_cwd_and_utf8_stdout() {
    let base = std::env::temp_dir().join(format!(
        "work-process-cwd-t25-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root = base.join("專案 workspace");
    fs::create_dir_all(&root).unwrap();
    let input = base.join("需求.txt");
    fs::write(&input, "$work task -- 需求").unwrap();
    let arguments = [
        "--project-root",
        root.to_str().unwrap(),
        "--verbose",
        "invocation",
        "parse",
        "--input-file",
        "需求.txt",
    ];
    let invoke = |request_path: &str| {
        Command::new(installed_executable())
            .args(&arguments[..arguments.len() - 1])
            .arg(request_path)
            .current_dir(&base)
            .output()
            .unwrap()
    };
    let success = invoke("需求.txt");
    assert_eq!(success.status.code(), Some(0));
    assert!(success.stderr.is_empty());
    let response: Value = serde_json::from_slice(&success.stdout).unwrap();
    assert_eq!(response["schema"], "work-cli-result");
    assert_eq!(response["data"]["request"], " 需求");
    let missing = invoke("找不到 missing.json");
    assert_eq!(missing.status.code(), Some(8));
    assert!(missing.stderr.is_empty());
    let response: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(response["reason_code"], "input_file_read_failed");
    assert_eq!(fs::read_dir(PathBuf::from(&root)).unwrap().count(), 0);
}

#[test]
fn source_capture_requires_bound_approval_and_preserves_all_host_payloads() {
    let base = std::env::temp_dir().join(format!(
        "work-source-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&base).unwrap();
    let metadata_path = base.join("metadata.json");
    let payload_path = base.join("payload.bin");
    let cases = [
        (json!({"kind":"file","path":"original.pdf","media_type":"Application/PDF"}), "source.pdf", b"%PDF-1.7\r\n\xff\x00\xef\xbb\xbf".to_vec(), None),
        (json!({"kind":"user_text"}), "source.txt", "\u{feff}  原樣\r\n尾端 \n".as_bytes().to_vec(), None),
        (json!({"kind":"github_issue","repository":"owner/repo","number":68,"url":"https://github.com/owner/repo/issues/68"}),
            "source.json", br#"{ "title":"Issue", "body":"https://example.com/raw", "comments":[{"body":"first"},{"body":"second"}] }"#.to_vec(), Some(2)),
    ];
    for (index, (source, content_path, bytes, comment_count)) in cases.into_iter().enumerate() {
        let mut metadata = json!({"requirement_id":"example","captured_at":"2026-10-03T00:00:00Z","source":source,"content_path":content_path});
        if let Some(count) = comment_count {
            metadata["issue_comment_count"] = json!(count);
        }
        fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        fs::write(&payload_path, &bytes).unwrap();
        let args = vec![
            "--project-root".into(),
            base.to_string_lossy().into_owned(),
            "--verbose".into(),
            "source".into(),
            "capture".into(),
            "--input-file".into(),
            metadata_path.to_string_lossy().into_owned(),
            "--payload-file".into(),
            payload_path.to_string_lossy().into_owned(),
        ];
        if index == 0 {
            let mut no_id = metadata.clone();
            no_id.as_object_mut().unwrap().remove("requirement_id");
            fs::write(&metadata_path, serde_json::to_vec(&no_id).unwrap()).unwrap();
            let output = run(&args);
            assert_eq!(output.status.code(), Some(4));
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["reason_code"], "invalid_source_metadata");
            assert!(!base.join("outputs/work/sources").exists());
            let mut missing = metadata.clone();
            missing["source"]
                .as_object_mut()
                .unwrap()
                .remove("media_type");
            fs::write(&metadata_path, serde_json::to_vec(&missing).unwrap()).unwrap();
            let output = run(&args);
            assert_eq!(output.status.code(), Some(4));
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["reason_code"], "invalid_source_metadata");
            assert!(!base.join("outputs/work/sources/example").exists());
            let mut malformed = metadata.clone();
            malformed["source"]["media_type"] = json!("application/pdf\r\n");
            fs::write(&metadata_path, serde_json::to_vec(&malformed).unwrap()).unwrap();
            let preview = run(&args);
            assert_eq!(preview.status.code(), Some(6));
            let preview: Value = serde_json::from_slice(&preview.stdout).unwrap();
            let mut invalid_approved = args.clone();
            invalid_approved.extend([
                "--approved-sha256".into(),
                preview["data"]["approval_sha256"].as_str().unwrap().into(),
            ]);
            let output = run(&invalid_approved);
            assert_eq!(output.status.code(), Some(4));
            let result: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["reason_code"], "invalid_source_media_type");
            assert!(!base.join("outputs/work/sources/example").exists());
            fs::write(&metadata_path, serde_json::to_vec(&metadata).unwrap()).unwrap();
        }
        let rejected = run(&args);
        assert_eq!(rejected.status.code(), Some(6));
        let rejected: Value = serde_json::from_slice(&rejected.stdout).unwrap();
        assert_eq!(rejected["reason_code"], "source_capture_approval_required");
        let root = base.join("outputs/work/sources/example");
        assert!(!root.join(format!("SRC-{:03}", index + 1)).exists());
        let mut approved = args.clone();
        approved.extend([
            "--approved-sha256".into(),
            rejected["data"]["approval_sha256"].as_str().unwrap().into(),
        ]);
        let output = run(&approved);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["data"]["source_id"], format!("SRC-{:03}", index + 1));
        assert_eq!(result["data"]["source"], metadata["source"]);
        assert_eq!(
            fs::read(root.join(format!("SRC-{:03}/{content_path}", index + 1))).unwrap(),
            bytes
        );
        let read_args = |action: &str, source_id: &str| {
            vec![
                "--project-root".into(),
                base.to_string_lossy().into_owned(),
                "source".into(),
                action.into(),
                "--requirement-id".into(),
                "example".into(),
                "--source-id".into(),
                source_id.into(),
            ]
        };
        let source_id = format!("SRC-{:03}", index + 1);
        let output = run(&read_args("read", &source_id));
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        let restored: Vec<u8> = serde_json::from_value(result["data"]["bytes"].clone()).unwrap();
        assert_eq!(restored, bytes);
        assert_eq!(result["data"]["schema"], "work-source-read");
        assert_eq!(result["data"]["manifest"]["source"], metadata["source"]);
        let output = run(&read_args("validate", &source_id));
        assert!(output.status.success());
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["data"]["schema"], "work-source-validation");
        assert_eq!(result["data"]["content_size"], bytes.len());
        assert!(result["data"].get("bytes").is_none());
        if index == 0 {
            let content = root.join("SRC-001/source.pdf");
            let mut drift = bytes.clone();
            drift[0] ^= 1;
            fs::write(&content, &drift).unwrap();
            for action in ["read", "validate"] {
                let output = run(&read_args(action, "SRC-001"));
                assert_eq!(output.status.code(), Some(5));
                let result: Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(result["reason_code"], "source_hash_mismatch");
                assert!(result["data"].get("bytes").is_none());
                let missing = run(&read_args(action, "SRC-999"));
                assert_eq!(missing.status.code(), Some(5));
                let missing: Value = serde_json::from_slice(&missing.stdout).unwrap();
                assert_eq!(missing["reason_code"], "source_snapshot_missing");
            }
            fs::write(&content, &bytes).unwrap();
        }
        if index == 2 {
            let old = fs::read(root.join("SRC-003/manifest.json")).unwrap();
            let output = run(&approved);
            assert!(output.status.success());
            assert_eq!(fs::read(root.join("SRC-004/source.json")).unwrap(), bytes);
            assert_eq!(fs::read(root.join("SRC-003/manifest.json")).unwrap(), old);
            fs::write(&payload_path, b"changed").unwrap();
            let drift = run(&approved);
            assert_eq!(drift.status.code(), Some(6));
            assert!(!root.join("SRC-005").exists());
        }
    }
}

#[test]
fn workflow_uses_fixed_source_then_task_without_plan_and_rejects_plan_paths() {
    let project = std::env::temp_dir().join(format!(
        "work-source-workflow-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&project).unwrap();
    let repo = PathBuf::from(project_root());
    let fixture = repo.join("rust/crates/work-infrastructure/fixtures/task-diagnostics");
    let run = |command: &str, extra: &[&str]| {
        Command::new(installed_executable())
            .args([
                "--project-root",
                project.to_str().unwrap(),
                "--verbose",
                "workflow",
                command,
                "--requirement-id",
                "example",
                "--user-config-root",
                project.to_str().unwrap(),
            ])
            .args(extra)
            .output()
            .unwrap()
    };
    let missing = run("status", &[]);
    assert!(
        missing.status.success(),
        "{}",
        String::from_utf8_lossy(&missing.stdout)
    );
    let missing: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(missing["data"]["next_action"], "capture_source");
    assert_eq!(missing["data"]["command"], "source capture");
    assert_eq!(
        missing["data"]["arguments"]["payload_file"],
        "<original-source-bytes-file>"
    );
    assert!(missing["data"]["arguments"].get("requirement_id").is_none());
    assert_eq!(fs::read_dir(&project).unwrap().count(), 0);
    for command in ["status", "next"] {
        assert_eq!(
            run(command, &["--plan-path", "legacy.json"]).status.code(),
            Some(2)
        );
    }
    work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &project).unwrap();
    let source_path = project.join("outputs/work/sources/example/SRC-001/source.txt");
    let source_raw = fs::read(&source_path).unwrap();
    let available = run("next", &[]);
    assert!(available.status.success());
    let available: Value = serde_json::from_slice(&available.stdout).unwrap();
    assert_eq!(available["data"]["next_action"], "initialize_discussion");
    assert_eq!(available["data"]["command"], "discussion init");
    assert!(available["data"]["arguments"].get("plan_path").is_none());
    let discussion_fixture =
        repo.join("rust/crates/work-infrastructure/fixtures/discussion-assembly");
    let mut session = work_infrastructure::fixture_support::prepare_discussion_fixture(
        &discussion_fixture,
        &project,
        &repo.join("skills/work"),
    )
    .unwrap();
    if let work_model::common::Nullable::Value(source) = &mut session.context.confirmed_source {
        source.snapshot = serde_json::from_slice(
            &fs::read(project.join("outputs/work/sources/example/SRC-001/manifest.json")).unwrap(),
        )
        .unwrap();
    }
    session.commit.content_sha256 =
        work_infrastructure::fixture_support::discussion_sha256(&session);
    let input = project.join("init-session.json");
    fs::write(&input, serde_json::to_vec(&json!({"schema":"work-discussion-request","requirement_id":"example","command":{"kind":"init","session":session}})).unwrap()).unwrap();
    let initialized = Command::new(installed_executable())
        .args(&[
            "--project-root".into(),
            project.to_string_lossy().into_owned(),
            "discussion".into(),
            "init".into(),
            "--input-file".into(),
            input.to_string_lossy().into_owned(),
            "--user-config-root".into(),
            project.to_string_lossy().into_owned(),
        ])
        .output()
        .unwrap();
    assert!(initialized.status.success());
    // An unrelated unfinished newer capture must never replace the saved Source.
    fs::create_dir_all(project.join("outputs/work/sources/example/.capture-SRC-002")).unwrap();
    let resumed = run("status", &[]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stdout)
    );
    let resumed: Value = serde_json::from_slice(&resumed.stdout).unwrap();
    assert_eq!(
        resumed["data"]["details"]["source"]["snapshot"]["source_id"],
        "SRC-001"
    );
    assert_eq!(resumed["data"]["next_action"], "preview_task");
    for relative in [
        "outputs/work/tasks/example/index.json",
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/executions/example/index.json",
    ] {
        let target = project.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(fixture.join(relative), target).unwrap();
    }
    fs::write(project.join("src.txt"), b"original\n").unwrap();
    let formal = run("next", &[]);
    assert!(
        formal.status.success(),
        "{}",
        String::from_utf8_lossy(&formal.stdout)
    );
    let formal: Value = serde_json::from_slice(&formal.stdout).unwrap();
    assert_eq!(formal["data"]["next_action"], "select_task_for_execution");
    assert_eq!(formal["data"]["command"], "execute preflight");
    assert_eq!(fs::read(&source_path).unwrap(), source_raw);
    assert!(!project.join("outputs/work/plans").exists());
    fs::write(&source_path, b"drifted immutable source").unwrap();
    assert!(!run("status", &[]).status.success());
}

#[test]
fn installed_instruction_recovery_uses_only_existing_current_journal() {
    use std::collections::BTreeMap;
    use work_infrastructure::fixture_support::{
        PublicationOrder, TransactionInput, TransactionKind, derive_fixture_transaction,
    };
    for (kind, prefix, transaction_kind) in [
        (
            "source_refresh",
            "source-refresh",
            TransactionKind::SourceRefresh,
        ),
        (
            "instruction_migration",
            "instruction-migration",
            TransactionKind::InstructionMigration,
        ),
    ] {
        let root = std::env::temp_dir().join(format!(
            "work-current-recovery-{kind}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("execution")).unwrap();
        fs::write(root.join("source.txt"), b"immutable source").unwrap();
        fs::write(root.join("target.json"), b"before").unwrap();
        let preview = "a".repeat(64);
        let (raw, approval) = derive_fixture_transaction(TransactionInput {
            kind: transaction_kind, order: PublicationOrder::Flat,
            request: serde_json::json!({"kind":kind,"requirement_id":"example","preview_fingerprint":preview}),
            artifacts: serde_json::json!({"execution":"execution"}), affected_task_ids: vec![],
            history: BTreeMap::new(),
            source: BTreeMap::from([("source.txt".into(),b"immutable source".to_vec()),("target.json".into(),b"before".to_vec())]),
            candidate: BTreeMap::from([("source.txt".into(),b"immutable source".to_vec()),("target.json".into(),b"after".to_vec())]),
        }).unwrap();
        let journal = format!("execution/.work-{prefix}-AAAAAAAAAAAA.json");
        fs::write(root.join(&journal), raw).unwrap();
        let invoke = |status| {
            let output = Command::new(installed_executable())
                .args([
                    "--project-root",
                    root.to_str().unwrap(),
                    "--verbose",
                    "instructions",
                    "recover",
                    "--journal-path",
                    &journal,
                    "--approved-sha256",
                    &approval,
                ])
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(status), "{output:?}");
            assert!(output.stderr.is_empty());
            serde_json::from_slice::<Value>(&output.stdout).unwrap()
        };
        let result = invoke(0);
        assert_eq!(result["data"]["schema"], "work-spec-transaction");
        assert_eq!(result["data"]["state"], "published");
        assert_eq!(fs::read(root.join("target.json")).unwrap(), b"after");
        let published_raw = fs::read(root.join(&journal)).unwrap();
        assert_eq!(invoke(0), result);
        assert_eq!(fs::read(root.join(&journal)).unwrap(), published_raw);
        fs::write(root.join("source.txt"), b"unexpected drift").unwrap();
        assert_eq!(
            invoke(5)["reason_code"],
            "instruction_recovery_source_changed"
        );
    }
}

#[test]
fn installed_spec_preview_rejects_unconfirmed_context_and_plan_candidates() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../work-infrastructure/fixtures/specification-update/item-goal");
    let root = std::env::temp_dir().join(format!(
        "work-spec-preview-task-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for relative in [
        "outputs/work/tasks/example/index.json",
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/executions/example/index.json",
    ] {
        let target = root.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(fixture.join(relative), target).unwrap();
    }
    work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
    fs::write(root.join("src.txt"), b"source\n").unwrap();
    let source = root.join("outputs/work/sources/example/SRC-001/source.txt");
    let original = fs::read(&source).unwrap();
    let invoke = |arguments: &[&str], status: i32| {
        let output = Command::new(installed_executable())
            .args(["--project-root", root.to_str().unwrap(), "--verbose"])
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(status),
            "{arguments:?}: {output:?}"
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let request_path = root.join("request.json");
    let request: Value =
        serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
    let preview = |candidate: &Value, status| {
        fs::write(&request_path, serde_json::to_vec(candidate).unwrap()).unwrap();
        invoke(
            &[
                "specification",
                "preview",
                "--input-file",
                request_path.to_str().unwrap(),
                "--user-config-root",
                root.to_str().unwrap(),
            ],
            status,
        )
    };
    let valid = preview(&request, 0);
    assert_eq!(valid["data"]["status"], "valid");
    assert_eq!(
        valid["data"]["changed_fields"],
        json!(["/task_items/TASK-001/goal"])
    );
    let mut unconfirmed = request.clone();
    unconfirmed["task_index"]["acceptance_criteria"][0]["criterion"] =
        json!("Changed acceptance without impact confirmation.");
    let rejected = preview(&unconfirmed, ExitCode::Contract as i32);
    assert_eq!(rejected["reason_code"], "source_confirmation_required");
    assert!(
        rejected
            .get("data")
            .is_none_or(|v| v.get("approved_sha256").is_none())
    );
    let mut legacy = request.clone();
    legacy["plan"] = json!({"schema":"work-plan/v1"});
    assert_eq!(
        preview(&legacy, ExitCode::Contract as i32)["reason_code"],
        "invalid_object_fields"
    );
    let mut drift = request.clone();
    drift["expected"]["source_sha256"] = json!("0".repeat(64));
    assert_eq!(
        preview(&drift, 5)["reason_code"],
        "spec_update_source_changed"
    );
    assert_eq!(fs::read(&source).unwrap(), original);
    assert_eq!(
        fs::read(root.join("outputs/work/tasks/example/index.json")).unwrap(),
        fs::read(fixture.join("outputs/work/tasks/example/index.json")).unwrap()
    );
    assert!(!root.join("outputs/work/plans").exists());
}

#[test]
fn installed_spec_recover_checks_all_approved_evidence_before_writes() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../work-infrastructure/fixtures/specification-update/item-goal");
    let root = std::env::temp_dir().join(format!(
        "work-spec-recover-task-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for relative in [
        "outputs/work/tasks/example/index.json",
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/executions/example/index.json",
    ] {
        let target = root.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(fixture.join(relative), target).unwrap();
    }
    work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
    fs::write(root.join("src.txt"), b"source\n").unwrap();
    let source = root.join("outputs/work/sources/example/SRC-001/source.txt");
    let original = fs::read(&source).unwrap();
    let invoke = |arguments: &[&str], status: i32| {
        let output = Command::new(installed_executable())
            .args(["--project-root", root.to_str().unwrap(), "--verbose"])
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(status),
            "{arguments:?}: {output:?}"
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let request_path = root.join("request.json");
    let request = fs::read(fixture.join("request.json")).unwrap();
    fs::write(&request_path, &request).unwrap();
    let preview = invoke(
        &[
            "specification",
            "preview",
            "--input-file",
            request_path.to_str().unwrap(),
            "--user-config-root",
            root.to_str().unwrap(),
        ],
        0,
    );
    let approval = preview["data"]["approved_sha256"].as_str().unwrap();
    let mut journal = preview["data"]["transaction"].clone();
    let first = &journal["files"][0];
    fs::write(
        root.join(first["path"].as_str().unwrap()),
        work_infrastructure::fixture_support::decode_transaction_snapshot(&first["after"]).unwrap(),
    )
    .unwrap();
    journal["published_count"] = json!(1);
    journal["state"] = json!("publishing");
    let journal_path = format!(
        "outputs/work/executions/example/.work-spec-update-{}.json",
        journal["transaction_id"].as_str().unwrap()
    );
    work_infrastructure::specification::storage::write_journal(&root, &journal_path, &journal)
        .unwrap();
    let remaining = journal["files"]
        .as_array()
        .unwrap()
        .iter()
        .skip(1)
        .map(|row| {
            let path = row["path"].as_str().unwrap().to_owned();
            let raw = fs::read(root.join(&path)).ok();
            (path, raw)
        })
        .collect::<Vec<_>>();
    let recover = |status| {
        invoke(
            &[
                "specification",
                "recover",
                "--input-file",
                request_path.to_str().unwrap(),
                "--approved-sha256",
                approval,
                "--user-config-root",
                root.to_str().unwrap(),
            ],
            status,
        )
    };
    fs::write(&source, b"unexpected Source drift").unwrap();
    assert_eq!(recover(5)["reason_code"], "spec_update_source_changed");
    for (path, raw) in &remaining {
        assert_eq!(fs::read(root.join(path)).ok(), *raw);
    }
    fs::write(&source, &original).unwrap();
    let recovered = recover(0);
    assert_eq!(recovered["data"]["status"], "recovered");
    assert_eq!(
        recover(0)["data"]["publication_status"],
        "already_published"
    );
    let verify_path = root.join("verify.json");
    fs::write(
        &verify_path,
        serde_json::to_vec(&recovered["data"]["verification_request"]).unwrap(),
    )
    .unwrap();
    let verified = invoke(
        &[
            "specification",
            "verify",
            "--input-file",
            verify_path.to_str().unwrap(),
            "--user-config-root",
            root.to_str().unwrap(),
        ],
        0,
    );
    assert_eq!(verified["data"]["verified"], true);
    assert_eq!(fs::read(&source).unwrap(), original);
    assert!(!root.join("outputs/work/plans").exists());
}

#[test]
fn installed_artifact_editor_uses_fixed_source_and_task_only_requests() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../work-infrastructure/fixtures/specification-update/item-goal");
    let root = std::env::temp_dir().join(format!(
        "work-artifact-editor-task-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for relative in [
        "outputs/work/tasks/example/index.json",
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/executions/example/index.json",
    ] {
        let target = root.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(fixture.join(relative), target).unwrap();
    }
    work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
    fs::write(root.join("src.txt"), b"source\n").unwrap();
    let source = root.join("outputs/work/sources/example/SRC-001/source.txt");
    let original = fs::read(&source).unwrap();
    let invoke = |arguments: &[&str], status: i32| {
        let output = Command::new(installed_executable())
            .args(["--project-root", root.to_str().unwrap(), "--verbose"])
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(status),
            "{arguments:?}: {output:?}"
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let input = root.join("editor.json");
    let request = json!({"schema":"work-delegation-build-request","role":"artifact-editor","mode":"execute","request":"Revise confirmed artifact.","task_path":"outputs/work/tasks/example/index.json","confirmed_request":{"schema":"work-spec-prepare-request","requirement_id":"example","reason":"Reviewed","edits":[{"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"goal","after":"Reviewed goal"}]},"decisions":["Confirmed revision"],"affected_task_ids":["TASK-001"],"continuation_point":"Return to Execute"});
    let build = |value: &Value, status| {
        fs::write(&input, serde_json::to_vec(value).unwrap()).unwrap();
        invoke(
            &[
                "delegation",
                "build",
                "--input-file",
                input.to_str().unwrap(),
            ],
            status,
        )
    };
    let envelope = build(&request, 0)["data"].clone();
    assert_eq!(
        envelope["context"]["task_source"]["source_bytes"],
        json!(original)
    );
    assert!(envelope["context"]["artifacts"].get("plan").is_none());
    fs::write(&input, serde_json::to_vec(&envelope).unwrap()).unwrap();
    let validation = invoke(
        &[
            "delegation",
            "validate",
            "--input-file",
            input.to_str().unwrap(),
            "--role",
            "artifact-editor",
            "--sender",
            "parent",
        ],
        0,
    );
    assert_eq!(validation["data"]["source_validation"], "checked");
    assert_eq!(validation["data"]["grants_authorization"], false);
    for kind in ["origin", "candidate", "scope", "legacy"] {
        let mut wrong = request.clone();
        match kind {
            "origin" => wrong["mode"] = json!("plan"),
            "candidate" => {
                wrong["confirmed_request"]["edits"][0]["target"]["artifact"] = json!("plan")
            }
            "scope" => wrong["affected_task_ids"] = json!(["TASK-999"]),
            _ => wrong["source_plan_path"] = json!("outputs/work/plans/example.json"),
        }
        assert_eq!(
            build(&wrong, ExitCode::Contract as i32)["reason_code"],
            "delegation_boundary_mismatch"
        );
    }
    let mut task_origin = request.clone();
    task_origin["mode"] = json!("task");
    assert_eq!(build(&task_origin, 0)["data"]["mode"], "task");
    fs::write(&source, b"unexpected Source drift").unwrap();
    fs::write(&input, serde_json::to_vec(&envelope).unwrap()).unwrap();
    let rejected = invoke(
        &[
            "delegation",
            "validate",
            "--input-file",
            input.to_str().unwrap(),
            "--role",
            "artifact-editor",
            "--sender",
            "parent",
        ],
        5,
    );
    assert_ne!(rejected["status"], "success");
    assert!(!root.join("outputs/work/plans").exists());
}

#[test]
fn installed_execute_delegation_requires_formal_task_without_plan() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../work-infrastructure/fixtures/task-diagnostics");
    let root = std::env::temp_dir().join(format!(
        "work-execute-delegation-task-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for relative in [
        "outputs/work/tasks/example/index.json",
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/executions/example/index.json",
    ] {
        let target = root.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(fixture.join(relative), target).unwrap();
    }
    work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
    let source = root.join("outputs/work/sources/example/SRC-001/source.txt");
    let original = fs::read(&source).unwrap();
    let execution = root.join("outputs/work/executions/example/index.json");
    let original_execution = fs::read(&execution).unwrap();
    let invoke = |arguments: &[&str], status: i32| {
        let output = Command::new(installed_executable())
            .args(["--project-root", root.to_str().unwrap(), "--verbose"])
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(status),
            "{arguments:?}: {output:?}"
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let input = root.join("delegation.json");
    let request = json!({"schema":"work-delegation-build-request","role":"execute","request":"Execute selected TASK.","task_path":"outputs/work/tasks/example/index.json","task_id":"TASK-001"});
    let build = |value: &Value, status| {
        fs::write(&input, serde_json::to_vec(value).unwrap()).unwrap();
        invoke(
            &[
                "delegation",
                "build",
                "--input-file",
                input.to_str().unwrap(),
            ],
            status,
        )
    };
    let envelope = build(&request, 0)["data"].clone();
    assert_eq!(
        envelope["context"]["task_source"]["source_bytes"],
        json!(original)
    );
    assert_eq!(envelope["context"]["task_boundary"]["id"], "TASK-001");
    assert_eq!(
        envelope["context"]["execution_index"],
        serde_json::from_slice::<Value>(&original_execution).unwrap()
    );
    for kind in ["legacy", "unknown", "plan_role", "plan_mode"] {
        let mut wrong = request.clone();
        match kind {
            "legacy" => wrong["source_plan_path"] = json!("missing.json"),
            "unknown" => wrong["task_id"] = json!("TASK-999"),
            "plan_role" => wrong["role"] = json!("plan"),
            _ => wrong["mode"] = json!("plan"),
        };
        assert_eq!(
            build(&wrong, 4)["reason_code"],
            "delegation_boundary_mismatch"
        );
    }
    let validate = |value: &Value, role: &str, status| {
        fs::write(&input, serde_json::to_vec(value).unwrap()).unwrap();
        invoke(
            &[
                "delegation",
                "validate",
                "--input-file",
                input.to_str().unwrap(),
                "--role",
                role,
                "--sender",
                "parent",
            ],
            status,
        )
    };
    let verified = validate(&envelope, "execute", 0);
    assert_eq!(verified["data"]["source_validation"], "checked");
    assert_eq!(verified["data"]["grants_authorization"], false);
    assert_eq!(
        validate(&envelope, "plan", 2)["reason_code"],
        "cli_usage_error"
    );
    for field in [
        "task_boundary",
        "target_task",
        "task_source",
        "execution_index_sha256",
    ] {
        let mut wrong = envelope.clone();
        wrong["context"][field] = Value::Null;
        assert_eq!(
            validate(&wrong, "execute", 4)["reason_code"],
            "delegation_boundary_mismatch"
        );
    }
    let mut changed_source = original.clone();
    changed_source[0] ^= 1;
    fs::write(&source, changed_source).unwrap();
    assert_eq!(
        validate(&envelope, "execute", 5)["reason_code"],
        "source_hash_mismatch"
    );
    assert_eq!(fs::read(&execution).unwrap(), original_execution);
    assert!(!root.join("outputs/work/plans").exists());
    assert!(
        !root
            .join("outputs/work/executions/example/attempts")
            .exists()
    );
}

#[test]
fn installed_execute_preflight_uses_only_formal_task_and_rejects_drift() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../work-infrastructure/fixtures/task-diagnostics");
    let root = std::env::temp_dir().join(format!(
        "work-execute-preflight-task-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for relative in [
        "outputs/work/tasks/example/index.json",
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/executions/example/index.json",
    ] {
        let target = root.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(fixture.join(relative), target).unwrap();
    }
    work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
    fs::write(root.join("src.txt"), b"source\n").unwrap();
    let source = root.join("outputs/work/sources/example/SRC-001/source.txt");
    let original = fs::read(&source).unwrap();
    let execution = root.join("outputs/work/executions/example/index.json");
    let execution_raw = fs::read(&execution).unwrap();
    let invoke = |task: &str, dir: &str, status: i32| {
        let output = Command::new(installed_executable())
            .args([
                "--project-root",
                root.to_str().unwrap(),
                "--verbose",
                "execute",
                "preflight",
                "--task-path",
                "outputs/work/tasks/example/index.json",
                "--execution-dir",
                dir,
                "--task-id",
                task,
                "--user-config-root",
                root.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(status), "{task}: {output:?}");
        assert!(output.stderr.is_empty());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let result = invoke("TASK-001", "outputs/work/executions/example", 0);
    assert_eq!(result["data"]["eligibility"], "passed");
    assert_eq!(
        result["data"]["execute_skill_selection"]["decision"],
        "base_only"
    );
    assert_eq!(
        result["data"]["task_path"],
        "outputs/work/tasks/example/index.json"
    );
    assert_eq!(
        invoke("TASK-999", "outputs/work/executions/example", 4)["reason_code"],
        "unknown_task_id"
    );
    fs::create_dir_all(root.join("outputs/work/executions/other")).unwrap();
    assert_eq!(
        invoke("TASK-001", "outputs/work/executions/other", 5)["reason_code"],
        "execute_preflight_artifact_path_mismatch"
    );
    let mut stale: Value = serde_json::from_slice(&execution_raw).unwrap();
    stale["task_collection_sha256"] = json!("0".repeat(64));
    fs::write(
        &execution,
        work_infrastructure::fixture_support::render_execution_index(&stale).unwrap(),
    )
    .unwrap();
    assert_eq!(
        invoke("TASK-001", "outputs/work/executions/example", 5)["reason_code"],
        "execute_preflight_index_identity_mismatch"
    );
    fs::write(&execution, &execution_raw).unwrap();
    for field in ["skill_selection_sha256", "task_item_sha256", "skill_id"] {
        let mut stale: Value = serde_json::from_slice(&execution_raw).unwrap();
        if field == "skill_selection_sha256" {
            stale[field] = json!("0".repeat(64));
        } else {
            stale["tasks"][0][field] = if field == "skill_id" {
                json!("repo:unconfirmed")
            } else {
                json!("0".repeat(64))
            };
        }
        fs::write(
            &execution,
            work_infrastructure::fixture_support::render_execution_index(&stale).unwrap(),
        )
        .unwrap();
        assert_eq!(
            invoke("TASK-001", "outputs/work/executions/example", 5)["reason_code"],
            "execute_preflight_task_binding_mismatch",
            "{field}"
        );
    }
    fs::write(&execution, &execution_raw).unwrap();
    let mut changed = original.clone();
    changed[0] ^= 1;
    fs::write(&source, changed).unwrap();
    assert_eq!(
        invoke("TASK-001", "outputs/work/executions/example", 5)["reason_code"],
        "source_hash_mismatch"
    );
    assert_eq!(fs::read(&execution).unwrap(), execution_raw);
    assert!(!root.join("outputs/work/plans").exists());
    assert!(
        !root
            .join("outputs/work/executions/example/attempts")
            .exists()
    );
}

#[test]
fn installed_attempt_render_preserves_derived_acceptance_evidence_and_history() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../work-infrastructure/fixtures/specification-reconciliation/real-flow/with-migration",
    );
    let task: Value = serde_json::from_slice(
        &fs::read(fixture.join("outputs/work/tasks/example/tasks/TASK-001.json")).unwrap(),
    )
    .unwrap();
    let mut attempt: Value = serde_json::from_slice(
        &fs::read(
            fixture.join("outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let previous_records = attempt["records"].clone();
    let previous_deviations = attempt["execution_deviations"].clone();
    attempt["status"] = json!("in_progress");
    attempt.as_object_mut().unwrap().remove("ended_at");
    let mut index: Value = serde_json::from_slice(
        &fs::read(fixture.join("outputs/work/executions/example/index.json")).unwrap(),
    )
    .unwrap();
    // Isolate historical artifact serialization from the fixture's later Task revision.
    index["tasks"][0]["latest_attempt"] = attempt["attempt_id"].clone();
    index["tasks"][0]["task_item_sha256"] = attempt["task_item_sha256"].clone();
    index["tasks"][0]["instructions_sha256"] = attempt["task_instructions_sha256"].clone();
    index["tasks"][0]["status"] = json!("in_progress");
    index["overall_status"] = json!("in_progress");
    index["lock"] = json!({"kind":"execution","task_id":"TASK-001","attempt_id":"ATTEMPT-001","execute_instructions_sha256":attempt["execute_instructions_sha256"],"record_id":"VAL-001#1","retry_authorization_evidence":"Fresh fixture retry approval"});
    let (candidate, _)=work_infrastructure::fixture_support::record_finish_candidates(&task,&attempt,&index,&json!({"schema":"work-record-finish-request","record":{"outcome":"passed","evidence":"Actual fixture VAL reviewed."}})).unwrap();
    let progress = candidate["acceptance_results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "ACCEPTANCE-001")
        .unwrap();
    assert_eq!(progress["status"], "completed");
    assert_eq!(progress["evidence"][0]["record_id"], "VAL-001#1");
    let root = std::env::temp_dir().join(format!(
        "work-attempt-acceptance-render-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("attempt.json");
    fs::write(&input, serde_json::to_vec(&candidate).unwrap()).unwrap();
    let output = Command::new(installed_executable())
        .args([
            "--project-root",
            root.to_str().unwrap(),
            "--verbose",
            "attempt",
            "render",
            "--input-file",
            input.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stderr.is_empty());
    let rendered: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rendered["data"], candidate);
    assert_eq!(
        rendered["data"]["execution_deviations"],
        previous_deviations
    );
    assert_eq!(rendered["data"]["records"][0], previous_records[0]);
    assert_eq!(
        rendered["data"]["acceptance_results"],
        candidate["acceptance_results"]
    );
}

#[test]
fn migration_analysis_preserves_opaque_evidence_and_rejects_plan_selection() {
    let root = std::env::temp_dir().join(format!(
        "work-raw-migration-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let payloads = [
        ("legacy.pdf", b"%PDF-1.7\r\n\xff\x00".as_slice()),
        ("broken.json", b"{invalid".as_slice()),
        (
            "old-task.json",
            b"{\"schema\":\"work-task-index/v1\"}".as_slice(),
        ),
        (
            "unsupported.json",
            b"{\"schema\":\"unsupported/v99\"}".as_slice(),
        ),
        (
            "unknown-field.json",
            b"{\"schema\":\"work-task-index\",\"unknown\":true}".as_slice(),
        ),
        (
            "old-plan.json",
            b"{\"schema\":\"work-plan/v0\",\"unrecognized\":true}".as_slice(),
        ),
    ];
    let mut args = vec![
        "--project-root".to_owned(),
        root.to_string_lossy().into_owned(),
        "migration".into(),
        "analyze".into(),
        "--requirement-id".into(),
        "example".into(),
    ];
    for (name, raw) in payloads {
        fs::write(root.join(name), raw).unwrap();
        args.extend(["--evidence-path".into(), name.into()]);
    }
    let output = run(&args);
    assert_eq!(output.status.code(), Some(0));
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let items = result["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), payloads.len());
    for (item, (name, raw)) in items.iter().zip(payloads) {
        assert_eq!(item["path"], name);
        assert_eq!(item["kind"], "raw_evidence");
        assert_eq!(item["source_size"], raw.len());
        assert_eq!(
            item["source_sha256"],
            work_infrastructure::fixture_support::raw_sha256(raw)
        );
        assert_eq!(
            serde_json::from_value::<Vec<u8>>(item["raw"].clone()).unwrap(),
            raw
        );
        assert_eq!(item["resolution_status"], "needs_review");
        assert!(item.get("proposed_content").is_none());
        assert_eq!(fs::read(root.join(name)).unwrap(), raw);
    }
    assert!(!root.join("outputs").exists());
    let mut legacy = args[..6].to_vec();
    legacy.extend(["--artifact".into(), "plan".into()]);
    let rejected = run(&legacy);
    assert_eq!(rejected.status.code(), Some(2));
    let rejection: Value = serde_json::from_slice(&rejected.stdout).unwrap();
    assert_eq!(rejection["reason_code"], "cli_usage_error");
    assert!(!root.join("outputs").exists());
}

#[test]
fn migration_prepare_requires_complete_reviewed_candidates_and_exact_raw_evidence() {
    let root = std::env::temp_dir().join(format!(
        "work-reviewed-migration-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let fixture = PathBuf::from(project_root())
        .join("rust/crates/work-infrastructure/fixtures/specification-update");
    let task_path = "outputs/work/tasks/example/index.json";
    for path in [
        task_path,
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/executions/example/index.json",
    ] {
        let destination = root.join(path);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(fixture.join(path), destination).unwrap();
    }
    work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
    let original = b"%PDF-1.7\r\n\xff\x00";
    fs::write(root.join(task_path), original).unwrap();
    let base = vec![
        "--project-root".to_owned(),
        root.to_string_lossy().into_owned(),
        "--verbose".into(),
    ];
    let invoke = |tail: Vec<String>| run(&[base.clone(), tail].concat());
    let analyzed = invoke(vec![
        "migration".into(),
        "analyze".into(),
        "--requirement-id".into(),
        "example".into(),
        "--artifact".into(),
        "task".into(),
    ]);
    assert_eq!(analyzed.status.code(), Some(0));
    let analysis = serde_json::from_slice::<Value>(&analyzed.stdout).unwrap()["data"].clone();
    let item = &analysis["items"][0];
    let content: Value =
        serde_json::from_slice(&fs::read(fixture.join(task_path)).unwrap()).unwrap();
    let request = json!({"schema":"work-artifact-migration-decisions","analysis":analysis,"choices":[{"id":item["id"],"action":"modify","content":content}]});
    let input = root.join("reviewed.json");
    let prepare = |value: &Value| {
        fs::write(&input, serde_json::to_vec(value).unwrap()).unwrap();
        invoke(vec![
            "migration".into(),
            "prepare".into(),
            "--input-file".into(),
            input.to_string_lossy().into_owned(),
        ])
    };
    for (value, code, reason) in [
        {
            let mut v = request.clone();
            v["choices"][0].as_object_mut().unwrap().remove("content");
            (v, 5, "migration_candidate_missing")
        },
        {
            let mut v = request.clone();
            v["analysis"]["items"][0]["raw"][0] = json!(0);
            (v, 5, "migration_raw_evidence_invalid")
        },
        {
            let mut v = request.clone();
            v["analysis"]["fingerprint"] = json!("0".repeat(64));
            (v, 5, "migration_analysis_fingerprint_mismatch")
        },
        {
            let mut v = request.clone();
            v["source_plan_path"] = json!("legacy.json");
            (v, 4, "migration_decisions_contract")
        },
        {
            let mut v = request.clone();
            v["choices"][0]["unreviewed"] = json!(true);
            (v, 4, "migration_decisions_contract")
        },
    ] {
        let output = prepare(&value);
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(output.status.code(), Some(code), "{response}");
        assert_eq!(response["reason_code"], reason);
        assert!(!root.join("outputs/work/migrations").exists());
        assert_eq!(fs::read(root.join(task_path)).unwrap(), original);
    }
    let mut changed = original.to_vec();
    changed[0] ^= 1;
    fs::write(root.join(task_path), changed).unwrap();
    let output = prepare(&request);
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(output.status.code(), Some(5));
    assert_eq!(response["reason_code"], "migration_source_changed");
    assert!(!root.join("outputs/work/migrations").exists());
    fs::write(root.join(task_path), original).unwrap();
    let prepared = prepare(&request);
    let response: Value = serde_json::from_slice(&prepared.stdout).unwrap();
    assert_eq!(prepared.status.code(), Some(0), "{response}");
    assert_eq!(response["data"]["executable"], true);
    let raw = fs::read(root.join(response["data"]["request_path"].as_str().unwrap())).unwrap();
    assert_eq!(
        response["data"]["request_sha256"],
        work_infrastructure::fixture_support::raw_sha256(&raw)
    );
    assert_eq!(
        response["data"]["request"]["decisions"][0]["item"]["raw"],
        json!(original.as_slice())
    );
    assert_eq!(fs::read(root.join(task_path)).unwrap(), original);
    assert!(!root.join("outputs/work/plans").exists());
}

#[test]
fn reconciliation_public_commands_complete_both_approved_paths_and_recover() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../work-infrastructure/fixtures/specification-reconciliation/real-flow");
    for migration in [false, true] {
        let fixture = if migration {
            fixture.join("with-migration")
        } else {
            fixture.clone()
        };
        let root = std::env::temp_dir().join(format!(
            "work-cli-reconciliation-{migration}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let semantic: Value =
            serde_json::from_slice(&fs::read(fixture.join("semantic-request.json")).unwrap())
                .unwrap();
        let reference: Value =
            serde_json::from_slice(&fs::read(fixture.join("request.json")).unwrap()).unwrap();
        let attempt = reference["attempt_path"].as_str().unwrap();
        let attempt_raw = fs::read(fixture.join(attempt)).unwrap();
        let mut originals = std::collections::BTreeMap::new();
        let sources = if migration {
            reference["migration"]["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["path"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        } else {
            vec!["outputs/work/executions/example/index.json".to_owned()]
        };
        for path in sources
            .into_iter()
            .chain(std::iter::once(attempt.to_owned()))
        {
            let raw = fs::read(fixture.join(&path)).unwrap();
            fs::create_dir_all(root.join(&path).parent().unwrap()).unwrap();
            fs::write(root.join(&path), &raw).unwrap();
            originals.insert(path, raw);
        }
        fs::write(root.join("src.txt"), b"source\n").unwrap();
        let input = root.join("request.json");
        fs::write(&input, serde_json::to_vec(&semantic).unwrap()).unwrap();
        let invoke =
            |root: &Path, command: &str, input: &Path, approval: Option<&str>, code: i32| {
                let mut args = vec![
                    "--project-root".into(),
                    root.to_string_lossy().into_owned(),
                    "--verbose".into(),
                    "specification".into(),
                    command.into(),
                    "--input-file".into(),
                    input.to_string_lossy().into_owned(),
                    "--user-config-root".into(),
                    root.join("config").to_string_lossy().into_owned(),
                ];
                if let Some(approval) = approval {
                    args.extend(["--approved-sha256".into(), approval.into()]);
                }
                let output = run(&args);
                assert_eq!(
                    output.status.code(),
                    Some(code),
                    "{command}: {}",
                    String::from_utf8_lossy(&output.stdout)
                );
                serde_json::from_slice::<Value>(&output.stdout).unwrap()
            };
        let prepared = invoke(&root, "reconciliation-prepare", &input, None, 0)["data"].clone();
        fs::write(&input, serde_json::to_vec(&prepared["request"]).unwrap()).unwrap();
        let preview = invoke(&root, "reconciliation-preview", &input, None, 0)["data"].clone();
        assert_eq!(preview, prepared["preview"]);
        let approval = preview["fingerprint"].as_str().unwrap();
        assert_eq!(
            invoke(
                &root,
                "reconciliation-apply",
                &input,
                Some(&"0".repeat(64)),
                5
            )["reason_code"],
            "reconciliation_approval_changed"
        );
        for (path, raw) in &originals {
            assert_eq!(fs::read(root.join(path)).unwrap(), *raw);
        }
        let publication =
            invoke(&root, "reconciliation-apply", &input, Some(approval), 0)["data"].clone();
        assert_eq!(fs::read(root.join(attempt)).unwrap(), attempt_raw);
        let journal_path = publication["publication"]["journal"].as_str().unwrap();
        let mut journal: Value =
            serde_json::from_slice(&fs::read(root.join(journal_path)).unwrap()).unwrap();
        let recovery = root.join("recovery-project");
        fs::create_dir_all(&recovery).unwrap();
        for (path, raw) in &originals {
            fs::create_dir_all(recovery.join(path).parent().unwrap()).unwrap();
            fs::write(recovery.join(path), raw).unwrap();
        }
        fs::write(recovery.join("src.txt"), b"source\n").unwrap();
        journal["state"] = json!("prepared");
        journal["published_count"] = json!(0);
        work_infrastructure::specification::storage::write_journal(
            &recovery,
            journal_path,
            &journal,
        )
        .unwrap();
        let last = journal["files"].as_array().unwrap().last().unwrap();
        let path = last["path"].as_str().unwrap();
        fs::create_dir_all(recovery.join(path).parent().unwrap()).unwrap();
        fs::write(
            recovery.join(path),
            work_infrastructure::fixture_support::decode_transaction_snapshot(&last["after"])
                .unwrap(),
        )
        .unwrap();
        let recovered = invoke(
            &recovery,
            "reconciliation-recover",
            &input,
            Some(approval),
            0,
        )["data"]
            .clone();
        assert_eq!(recovered["publication"]["publication_status"], "published");
        assert_eq!(
            invoke(
                &recovery,
                "reconciliation-recover",
                &input,
                Some(approval),
                0
            )["data"]["publication"]["publication_status"],
            "already_published"
        );
        for file in journal["files"].as_array().unwrap() {
            assert_eq!(
                fs::read(recovery.join(file["path"].as_str().unwrap())).unwrap(),
                work_infrastructure::fixture_support::decode_transaction_snapshot(&file["after"])
                    .unwrap()
            );
        }
        assert_eq!(fs::read(recovery.join(attempt)).unwrap(), attempt_raw);
        assert!(!recovery.join("outputs/work/plans").exists());
    }
}

#[test]
fn all_four_public_modes_preserve_both_origins_and_reach_current_commands() {
    let root = std::env::temp_dir().join(format!(
        "work-four-mode-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("invocation-input");
    for (mode, command, action) in [
        ("task", "task", "preview"),
        ("revise", "specification", "prepare"),
        ("migration", "migration", "analyze"),
        ("execute", "execute", "preflight"),
    ] {
        let request = " Confirmed full request e\u{0301}\n";
        for origin in ["explicit", "implicit_confirmed"] {
            let operation = if origin == "explicit" {
                fs::write(&input, format!("$work {mode} --{request}")).unwrap();
                "parse"
            } else {
                fs::write(&input, serde_json::to_vec(&json!({"mode":mode,"request":request,"confirmation":{"mode":mode,"request":request,"confirmed":true,"evidence":"User confirmed this complete request and mode."}})).unwrap()).unwrap();
                "confirm"
            };
            let output = run(&[
                "--project-root".into(),
                root.to_string_lossy().into_owned(),
                "--verbose".into(),
                "invocation".into(),
                operation.into(),
                "--input-file".into(),
                input.to_string_lossy().into_owned(),
            ]);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            let response: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(response["data"]["mode"], mode);
            assert_eq!(response["data"]["origin"], origin);
            assert_eq!(response["data"]["request"], request);
        }
        let output = run(&[
            "--project-root".into(),
            root.to_string_lossy().into_owned(),
            command.into(),
            action.into(),
            "--help".into(),
        ]);
        assert!(output.status.success());
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(
            response["data"]["help"]
                .as_str()
                .unwrap()
                .contains(&format!("work {command} {action}"))
        );
    }
    fs::write(&input, "$work plan -- Removed mode").unwrap();
    let output = run(&[
        "--project-root".into(),
        root.to_string_lossy().into_owned(),
        "invocation".into(),
        "parse".into(),
        "--input-file".into(),
        input.to_string_lossy().into_owned(),
    ]);
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["reason_code"],
        "work_invocation_mode_invalid"
    );
    assert!(!root.join("outputs").exists());
}

#[test]
fn retired_refresh_commands_are_ordinary_usage_errors() {
    for name in [
        "refresh-preview",
        "refresh-apply",
        "refresh-recover",
        "refresh-preview-all",
        "refresh-apply-all",
        "refresh-recover-all",
        "migration-preview",
        "migration-apply",
    ] {
        let output = run(&["instructions".into(), name.into()]);
        assert_eq!(output.status.code(), Some(2), "{name}");
        assert!(output.stderr.is_empty());
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["reason_code"], "cli_usage_error");
    }
}

mod discussion_process_cases {
    //! Session lifecycle through distinct public executable processes.
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use work_model::common::Nullable;
    use work_model::discussion::{DiscussionAction, DiscussionSession};

    fn setup(label: &str) -> (PathBuf, DiscussionSession) {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "work-discussion-cli-{label}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let repo = PathBuf::from(project_root());
        let fixture = repo.join("rust/crates/work-infrastructure/fixtures/discussion-assembly");
        work_infrastructure::fixture_support::copy_fixture_sources(&fixture, &root).unwrap();
        let mut session = work_infrastructure::fixture_support::prepare_discussion_fixture(
            &fixture,
            &root,
            &repo.join("skills/work"),
        )
        .unwrap();
        session.authorization.allowed_actions = vec![
            DiscussionAction::AddDecision,
            DiscussionAction::UpdateDecision,
            DiscussionAction::UpdatePlanning,
            DiscussionAction::PrepareQuestion,
            DiscussionAction::AnswerQuestion,
            DiscussionAction::UpdateContext,
            DiscussionAction::UpdateContinuation,
        ];
        session.commit.content_sha256 =
            work_infrastructure::fixture_support::discussion_sha256(&session);
        (root, session)
    }

    fn invoke(root: &Path, group: &str, operation: &str, command: Value, expected: i32) -> Value {
        let request = root.join(format!("{group}-{operation}-request.json"));
        fs::write(&request,serde_json::to_vec(&json!({"schema":"work-discussion-request","requirement_id":"example","command":command})).unwrap()).unwrap();
        let output = run(&[
            "--project-root".into(),
            root.to_string_lossy().into_owned(),
            group.into(),
            operation.into(),
            "--input-file".into(),
            request.to_string_lossy().into_owned(),
            "--user-config-root".into(),
            root.to_string_lossy().into_owned(),
        ]);
        assert_eq!(
            output.status.code(),
            Some(expected),
            "{group} {operation}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn current(root: &Path) -> Value {
        invoke(root, "discussion", "read", json!({"kind":"read"}), 0)["data"]["data"].clone()
    }

    fn local(view: &Value, id: &str, change: Value) -> Value {
        json!({"operation_id":id,"expected_revision":view["revision"],"previous_sha256":view["content_sha256"],"change":change})
    }

    fn opaque_decision(id: &str, status: &str) -> work_model::discussion::DiscussionDecision {
        serde_json::from_value(json!({"id":id,"question":"Storage format?","question_version":1,
            "options":[{"id":"option_a","label":"JSON","explanation":"Preserve structured fields"},{"id":"option_b","label":"CSV","explanation":"Exchange rows with spreadsheets"}],
            "status":status,"tentative_option_id":null,
            "resolution":if status=="confirmed" {json!({"option_id":"option_b","rationale":"Consumers use spreadsheets","confirmation_evidence":"User selected CSV"})} else {Value::Null},
            "status_reason":"","status_evidence":"","added_reason":"Choose format","task_ids":[],"dependencies":[],"source_references":["source.txt"]})).unwrap()
    }

    #[test]
    fn public_resume_fetches_only_confirmed_dependency_answer_and_rationale() {
        let (root, mut session) = setup("dependency-detail");
        let d001 = opaque_decision("D001", "confirmed");
        let mut d002 = opaque_decision("D002", "pending");
        d002.dependencies = vec!["D001".into()];
        session.decisions = vec![d001, d002];
        session.next_decision_number = 3;
        session.commit.content_sha256 =
            work_infrastructure::fixture_support::discussion_sha256(&session);
        invoke(
            &root,
            "discussion",
            "init",
            json!({"kind":"init","session":session}),
            0,
        );
        // All subsequent reads run in fresh processes, with no chat or Session in memory.
        let view = current(&root);
        assert_eq!(view["pending_decision_ids"], json!(["D002"]));
        let dependent = view["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == "D002")
            .unwrap();
        assert_eq!(dependent["dependencies"], json!(["D001"]));
        let before = fs::read(root.join("outputs/work/discussions/example/session.json")).unwrap();
        let detail = invoke(
            &root,
            "discussion",
            "read",
            json!({"kind":"read","decision_id":"D001"}),
            0,
        )["data"]["data"]
            .clone();
        assert_eq!(detail["id"], "D001");
        assert_eq!(detail["resolution"]["option_id"], "option_b");
        assert_eq!(detail["options"][1]["label"], "CSV");
        assert_eq!(
            detail["resolution"]["rationale"],
            "Consumers use spreadsheets"
        );
        assert_eq!(
            detail["resolution"]["confirmation_evidence"],
            "User selected CSV"
        );
        for field in ["tasks", "decisions", "authorization", "context", "commit"] {
            assert!(detail.get(field).is_none(), "{field}");
        }
        let missing = invoke(
            &root,
            "discussion",
            "read",
            json!({"kind":"read","decision_id":"D999"}),
            4,
        );
        assert_eq!(missing["reason_code"], "unknown_decision");
        assert_eq!(
            fs::read(root.join("outputs/work/discussions/example/session.json")).unwrap(),
            before
        );
    }

    #[test]
    fn swapped_display_mapping_rejects_old_answer_even_with_current_revision() {
        let (root, mut session) = setup("swapped-options");
        session.decisions = vec![opaque_decision("D001", "pending")];
        session.next_decision_number = 2;
        session.commit.content_sha256 =
            work_infrastructure::fixture_support::discussion_sha256(&session);
        invoke(
            &root,
            "discussion",
            "init",
            json!({"kind":"init","session":session}),
            0,
        );
        for (id, version, mapping) in [
            ("ask-first", 1, json!({"1":"option_a","2":"option_b"})),
            ("ask-swapped", 2, json!({"1":"option_b","2":"option_a"})),
        ] {
            let op = local(
                &current(&root),
                id,
                json!({"action":"prepare_question","question":{"decision_id":"D001","question_version":version,"text":"Storage format?","display_options":mapping}}),
            );
            invoke(
                &root,
                "discussion",
                "question",
                json!({"kind":"question","operation":op}),
                0,
            );
        }
        let restored = current(&root);
        assert_eq!(restored["continuation"]["question"]["question_version"], 3);
        let before = fs::read(root.join("outputs/work/discussions/example/session.json")).unwrap();
        let old = local(
            &restored,
            "old-screen-answer",
            json!({"action":"answer_question","decision_id":"D001","question_version":2,"display_number":"1","rationale":"Old screen JSON choice","evidence":"Old answer 1"}),
        );
        assert_eq!(
            invoke(
                &root,
                "discussion",
                "answer",
                json!({"kind":"answer","operation":old}),
                4
            )["reason_code"],
            "stale_discussion_question"
        );
        assert_eq!(
            fs::read(root.join("outputs/work/discussions/example/session.json")).unwrap(),
            before
        );
        assert!(
            !root
                .join("outputs/work/discussions/example/history/4")
                .exists()
        );
        let answer = local(
            &restored,
            "current-screen-answer",
            json!({"action":"answer_question","decision_id":"D001","question_version":3,"display_number":"1","rationale":"CSV for spreadsheets","evidence":"New screen answer 1"}),
        );
        invoke(
            &root,
            "discussion",
            "answer",
            json!({"kind":"answer","operation":answer}),
            0,
        );
        let detail = invoke(
            &root,
            "discussion",
            "read",
            json!({"kind":"read","decision_id":"D001"}),
            0,
        );
        assert_eq!(
            detail["data"]["data"]["resolution"]["option_id"],
            "option_b"
        );
    }

    #[test]
    fn decision_rebinding_persists_old_reviews_and_blocks_preview_until_all_reviewed() {
        let (root, mut session) = setup("rebind-plan");
        let template = serde_json::to_value(&session.tasks[0]).unwrap();
        session.tasks = (1..=5)
            .map(|n| {
                let raw = serde_json::to_string(&template)
                    .unwrap()
                    .replace("TASK-001", &format!("TASK-{n:03}"));
                serde_json::from_str(&raw).unwrap()
            })
            .collect();
        session.tasks[2].dependencies = vec!["TASK-001".into()];
        session.tasks[4].dependencies = vec!["TASK-003".into()];
        session.next_task_number = 6;
        let mut decision = opaque_decision("D001", "confirmed");
        decision.task_ids = vec!["TASK-001".into()];
        session.decisions = vec![decision.clone()];
        session.next_decision_number = 2;
        if let Nullable::Value(r) = &mut session.tasks[0].review {
            r.decision_versions.insert("D001".into(), 1);
        }
        let unrelated = serde_json::to_value(&session.tasks[3]).unwrap();
        session.commit.content_sha256 =
            work_infrastructure::fixture_support::discussion_sha256(&session);
        invoke(
            &root,
            "discussion",
            "init",
            json!({"kind":"init","session":session}),
            0,
        );
        let metadata = json!({"title":"Rebound collection","summary":"Review all affected tasks"});
        invoke(
            &root,
            "task",
            "preview",
            json!({"kind":"preview","metadata":metadata}),
            0,
        );
        decision.task_ids = vec!["TASK-002".into()];
        let op = local(
            &current(&root),
            "rebind",
            json!({"action":"update_decision","decision":decision}),
        );
        invoke(
            &root,
            "discussion",
            "update",
            json!({"kind":"update","operation":op}),
            0,
        );
        // Each history/read/update/preview is a fresh executable process.
        let persisted = invoke(
            &root,
            "discussion",
            "history",
            json!({"kind":"history","revision":2}),
            0,
        )["data"]["data"]
            .clone();
        for i in [0, 1, 2, 4] {
            assert_eq!(persisted["tasks"][i]["review"]["needs_review"], true);
        }
        assert_eq!(persisted["tasks"][3], unrelated);
        for i in [1, 0, 4, 2] {
            let mut task = persisted["tasks"][i].clone();
            task["review"]["needs_review"] = json!(false);
            task["review"]["semantic_consistency_evidence"] =
                json!("Reviewed changed decision association");
            if i == 1 {
                task["review"]["decision_versions"]["D001"] = json!(1);
            }
            let op = local(
                &current(&root),
                &format!("review-{i}"),
                json!({"action":"update_planning","task":task}),
            );
            invoke(
                &root,
                "discussion",
                "update",
                json!({"kind":"update","operation":op}),
                0,
            );
            let expected = if i == 2 { 0 } else { 5 };
            let response = invoke(
                &root,
                "task",
                "preview",
                json!({"kind":"preview","metadata":metadata}),
                expected,
            );
            if expected == 5 {
                assert_eq!(response["reason_code"], "discussion_planning_incomplete");
            }
        }
    }

    #[test]
    fn opaque_option_content_and_withdrawal_review_allow_normal_task_publication() {
        let (root, mut session) = setup("withdraw-plan");
        let mut decision = opaque_decision("D001", "confirmed");
        decision.task_ids = vec!["TASK-001".into()];
        session.decisions = vec![decision];
        session.next_decision_number = 2;
        session.tasks[0].decision_ids = vec!["D001".into()];
        session.tasks[0].steps[0]
            .references
            .push(serde_json::from_value(json!({"kind":"decisions","key":"D001"})).unwrap());
        if let Nullable::Value(review) = &mut session.tasks[0].review {
            review.decision_versions.insert("D001".into(), 1);
        }
        session.commit.content_sha256 =
            work_infrastructure::fixture_support::discussion_sha256(&session);
        invoke(
            &root,
            "discussion",
            "init",
            json!({"kind":"init","session":session}),
            0,
        );
        let metadata =
            json!({"title":"Reviewed collection","summary":"Withdraw superseded choice"});
        let first = invoke(
            &root,
            "task",
            "preview",
            json!({"kind":"preview","metadata":metadata}),
            0,
        )["data"]["data"]
            .clone();
        let statement = first["contract"]["tasks"][0]["decisions"][0]["statement"]
            .as_str()
            .unwrap();
        assert!(
            statement.contains("CSV") && statement.contains("Exchange rows with spreadsheets"),
            "{statement}"
        );
        assert!(!statement.contains("option_b"));
        assert_eq!(
            first["contract"]["tasks"][0]["decisions"][0]["rationale"],
            "Consumers use spreadsheets"
        );
        let mut withdrawn = invoke(
            &root,
            "discussion",
            "read",
            json!({"kind":"read","decision_id":"D001"}),
            0,
        )["data"]["data"]
            .clone();
        withdrawn["status"] = json!("withdrawn");
        withdrawn["status_reason"] = json!("Format no longer required");
        withdrawn["status_evidence"] = json!("User withdrew format choice");
        let op = local(
            &current(&root),
            "withdraw-choice",
            json!({"action":"update_decision","decision":withdrawn}),
        );
        invoke(
            &root,
            "discussion",
            "update",
            json!({"kind":"update","operation":op}),
            0,
        );
        let updated = invoke(
            &root,
            "discussion",
            "history",
            json!({"kind":"history","revision":2}),
            0,
        )["data"]["data"]
            .clone();
        assert_eq!(updated["decisions"][0]["task_ids"], json!([]));
        assert_eq!(updated["tasks"][0]["decision_ids"], json!([]));
        assert_eq!(updated["tasks"][0]["review"]["needs_review"], true);
        assert!(
            updated["tasks"][0]["steps"][0]["references"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["kind"] != "decisions")
        );
        let old = invoke(
            &root,
            "discussion",
            "history",
            json!({"kind":"history","revision":1}),
            0,
        )["data"]["data"]
            .clone();
        assert_eq!(old["decisions"][0]["task_ids"], json!(["TASK-001"]));
        assert_eq!(
            invoke(
                &root,
                "task",
                "preview",
                json!({"kind":"preview","metadata":metadata}),
                5
            )["reason_code"],
            "discussion_planning_incomplete"
        );
        let mut task = updated["tasks"][0].clone();
        task["review"]["needs_review"] = json!(false);
        task["review"]["semantic_consistency_evidence"] =
            json!("Reviewed steps and acceptance after withdrawing the obsolete format constraint");
        let op = local(
            &current(&root),
            "review-withdrawal",
            json!({"action":"update_planning","task":task}),
        );
        invoke(
            &root,
            "discussion",
            "update",
            json!({"kind":"update","operation":op}),
            0,
        );
        let preview = invoke(
            &root,
            "task",
            "preview",
            json!({"kind":"preview","metadata":metadata}),
            0,
        )["data"]["data"]
            .clone();
        assert!(preview["contract"]["tasks"][0].get("decisions").is_none());
        let view = current(&root);
        invoke(
            &root,
            "task",
            "apply",
            json!({"kind":"apply","expected_revision":view["revision"],"session_sha256":view["content_sha256"],"metadata":metadata,"approved_sha256":preview["approval_sha256"],"publication_evidence":"User approved reviewed TASK after withdrawal"}),
            0,
        );
        for (path, raw) in preview["targets"].as_object().unwrap() {
            assert_eq!(
                fs::read(root.join(path)).unwrap(),
                raw.as_str().unwrap().as_bytes()
            );
        }
    }

    #[test]
    fn d008_restores_numbered_question_deduplicates_old_operation_and_preserves_seven_decisions() {
        let (root, mut session) = setup("d008");
        session.decisions=(1..=12).map(|n| {
            let status=match n {1..=7=>"pending",9=>"deferred",10=>"blocked",11=>"needs_review",_=>"pending"};
            serde_json::from_value(json!({"id":format!("D{n:03}"),"question":format!("Question {n}?"),"question_version":1,
                "options":[{"id":"first","label":"First","explanation":"Reason"},{"id":"second","label":"Second","explanation":"Alternative"}],
                "status":status,"tentative_option_id":null,"resolution":if status=="confirmed" || status=="needs_review" {json!({"option_id":"first","rationale":format!("Rationale {n}"),"confirmation_evidence":format!("User {n}")})} else {Value::Null},
                "status_reason":"Explicit reason","status_evidence":"User instruction","added_reason":"Required decision","task_ids":[],"dependencies":[],"source_references":["source.txt"]})).unwrap()
        }).collect();
        session.next_decision_number = 13;
        session.commit.content_sha256 =
            work_infrastructure::fixture_support::discussion_sha256(&session);
        invoke(
            &root,
            "discussion",
            "init",
            json!({"kind":"init","session":session}),
            0,
        );
        // Confirm D001–D007 through public processes; none are seeded as confirmed.
        for number in 1..=7 {
            let view = current(&root);
            let id = format!("D{number:03}");
            let question = local(
                &view,
                &format!("question-{id}"),
                json!({"action":"prepare_question","question":{"decision_id":id,"question_version":1,"text":format!("Question {number}?"),"display_options":{"1":"first","2":"second"}}}),
            );
            invoke(
                &root,
                "discussion",
                "question",
                json!({"kind":"question","operation":question}),
                0,
            );
            let restored = current(&root);
            let answer = local(
                &restored,
                &format!("answer-{id}"),
                json!({"action":"answer_question","decision_id":id,"question_version":restored["continuation"]["question"]["question_version"],"display_number":"1","rationale":format!("Rationale {number}"),"evidence":format!("User confirmed {id}")}),
            );
            invoke(
                &root,
                "discussion",
                "answer",
                json!({"kind":"answer","operation":answer}),
                0,
            );
        }
        let committed = invoke(
            &root,
            "discussion",
            "history",
            json!({"kind":"history","revision":15}),
            0,
        )["data"]["data"]
            .clone();
        let typed: DiscussionSession = serde_json::from_value(committed).unwrap();
        let original = serde_json::to_vec(&typed.decisions[..7]).unwrap();
        let view = current(&root);
        assert_eq!(
            (
                view["progress"]["confirmed"].as_u64(),
                view["progress"]["known_total"].as_u64(),
                view["progress"]["remaining"].as_u64()
            ),
            (Some(7), Some(12), Some(5))
        );
        let question = local(
            &view,
            "question-d008",
            json!({"action":"prepare_question","question":{"decision_id":"D008","question_version":1,"text":"Question 8?","display_options":{"1":"first","2":"second"}}}),
        );
        let saved = invoke(
            &root,
            "discussion",
            "question",
            json!({"kind":"question","operation":question}),
            0,
        );
        assert_eq!(saved["data"]["data"]["saved"], true);
        // This read happens in a new OS process with no previous conversation state.
        let restored = current(&root);
        assert_eq!(
            restored["continuation"]["question"]["display_options"]["1"],
            "first"
        );
        let answer = local(
            &restored,
            "answer-d008",
            json!({"action":"answer_question","decision_id":"D008","question_version":restored["continuation"]["question"]["question_version"],"display_number":"1","rationale":"Chosen stable format","evidence":"User answered 1"}),
        );
        let answered = invoke(
            &root,
            "discussion",
            "answer",
            json!({"kind":"answer","operation":answer}),
            0,
        );
        assert_eq!(answered["data"]["data"]["view"]["progress"]["confirmed"], 8);
        let history = invoke(
            &root,
            "discussion",
            "history",
            json!({"kind":"history","revision":17}),
            0,
        )["data"]["data"]
            .clone();
        let typed: DiscussionSession = serde_json::from_value(history.clone()).unwrap();
        assert_eq!(serde_json::to_vec(&typed.decisions[..7]).unwrap(), original);
        assert!(
            matches!(&typed.decisions[7].resolution,Nullable::Value(resolution) if resolution.option_id=="first" && resolution.rationale=="Chosen stable format")
        );
        let view = current(&root);
        let update = local(
            &view,
            "continuation-after-answer",
            json!({"action":"update_continuation","current_task_id":null}),
        );
        invoke(
            &root,
            "discussion",
            "update",
            json!({"kind":"update","operation":update}),
            0,
        );
        let before = fs::read(root.join("outputs/work/discussions/example/session.json")).unwrap();
        let retried = invoke(
            &root,
            "discussion",
            "answer",
            json!({"kind":"answer","operation":answer}),
            0,
        );
        assert_eq!(retried["data"]["data"]["operation_revision"], 17);
        assert_eq!(retried["data"]["data"]["view"]["revision"], 18);
        assert_eq!(
            fs::read(root.join("outputs/work/discussions/example/session.json")).unwrap(),
            before
        );
        let mut collision = answer.clone();
        collision["change"]["rationale"] = json!("Different payload");
        let rejected = invoke(
            &root,
            "discussion",
            "answer",
            json!({"kind":"answer","operation":collision}),
            5,
        );
        assert_eq!(rejected["data"]["saved"], false);
        assert_eq!(
            fs::read(root.join("outputs/work/discussions/example/session.json")).unwrap(),
            before
        );
        let uncommitted = invoke(
            &root,
            "discussion",
            "history",
            json!({"kind":"history","revision":19}),
            5,
        );
        assert_eq!(
            uncommitted["reason_code"],
            "discussion_history_not_committed"
        );
        assert!(!root.join("outputs/work/tasks").exists());
    }

    #[test]
    fn full_preview_publication_and_recovery_bind_session_source_and_all_target_bytes() {
        let (root, session) = setup("publication");
        invoke(
            &root,
            "discussion",
            "init",
            json!({"kind":"init","session":session}),
            0,
        );
        let source = root.join("outputs/work/sources/example/SRC-001/source.txt");
        let source_before = fs::read(&source).unwrap();
        let metadata = json!({"title":"Approved collection","summary":"Complete preview"});
        let preview = invoke(
            &root,
            "task",
            "preview",
            json!({"kind":"preview","metadata":metadata}),
            0,
        )["data"]["data"]
            .clone();
        assert!(preview["contract"].is_object());
        assert!(preview["execution_index"].is_object());
        assert_eq!(preview["targets"].as_object().unwrap().len(), 3);
        let request = json!({"kind":"apply","expected_revision":1,"session_sha256":session.commit.content_sha256,"metadata":metadata,"approved_sha256":preview["approval_sha256"],"publication_evidence":"User approved the full three-file preview"});
        let mut wrong = request.clone();
        wrong["metadata"]["summary"] = json!("Changed at same revision");
        let output = invoke(&root, "task", "apply", wrong, 5);
        assert_eq!(output["reason_code"], "discussion_approval_mismatch");
        assert!(!root.join("outputs/work/tasks").exists());
        let published = invoke(&root, "task", "apply", request.clone(), 0);
        assert_eq!(published["data"]["data"]["published"], true);
        let before = preview["targets"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(path, target)| {
                let raw = fs::read(root.join(path)).unwrap();
                assert_eq!(raw, target.as_str().unwrap().as_bytes());
                (path.to_owned(), raw)
            })
            .collect::<Vec<_>>();
        let mut recovery = request;
        recovery["kind"] = json!("recover-publication");
        let recovered = invoke(&root, "task", "recover", recovery, 0);
        assert_eq!(recovered["data"]["data"]["changed"], false);
        for (path, raw) in before {
            assert_eq!(fs::read(root.join(path)).unwrap(), raw);
        }
        let execution: Value = serde_json::from_slice(
            &fs::read(root.join("outputs/work/executions/example/index.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(execution["overall_status"], "pending");
        assert!(execution["lock"].is_null());
        assert_eq!(fs::read(source).unwrap(), source_before);
        let collection: Value = serde_json::from_slice(
            &fs::read(root.join("outputs/work/tasks/example/index.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(collection["discussion"]["revision"], 1);
        assert_eq!(
            collection["discussion"]["content_sha256"],
            session.commit.content_sha256
        );
        let public = |tail: Vec<String>| {
            let mut args = vec![
                "--project-root".into(),
                root.to_string_lossy().into_owned(),
                "--verbose".into(),
            ];
            args.extend(tail);
            let output = run(&args);
            assert_eq!(
                output.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            assert!(output.stderr.is_empty());
            serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"].clone()
        };
        let config = root.to_string_lossy().into_owned();
        let task_path = "outputs/work/tasks/example/index.json";
        let validation = public(vec![
            "task".into(),
            "validate".into(),
            "--path".into(),
            task_path.into(),
            "--user-config-root".into(),
            config.clone(),
        ]);
        assert_eq!(validation["task_count"], 1);
        let preflight = public(vec![
            "execute".into(),
            "preflight".into(),
            "--execution-dir".into(),
            "outputs/work/executions/example".into(),
            "--task-path".into(),
            task_path.into(),
            "--task-id".into(),
            "TASK-001".into(),
            "--user-config-root".into(),
            config.clone(),
        ]);
        assert_eq!(preflight["schema"], "work-execute-preflight");
        assert_eq!(preflight["task_status"], "pending");
        let semantic = root.join("confirmed-revision.json");
        fs::write(&semantic, serde_json::to_vec(&json!({"schema":"work-spec-prepare-request","requirement_id":"example","reason":"User confirmed this formal goal revision","edits":[{"target":{"artifact":"task_item","task_id":"TASK-001"},"field":"goal","after":"Reviewed the complete observable result."}]})).unwrap()).unwrap();
        let prepared = public(vec![
            "specification".into(),
            "prepare".into(),
            "--input-file".into(),
            semantic.to_string_lossy().into_owned(),
            "--user-config-root".into(),
            config.clone(),
        ]);
        assert_eq!(
            prepared["request"]["task_index"]["discussion"],
            collection["discussion"]
        );
        let request = root.join("prepared-revision.json");
        fs::write(&request, serde_json::to_vec(&prepared["request"]).unwrap()).unwrap();
        let preview = public(vec![
            "specification".into(),
            "preview".into(),
            "--input-file".into(),
            request.to_string_lossy().into_owned(),
            "--user-config-root".into(),
            config.clone(),
        ]);
        let applied = public(vec![
            "specification".into(),
            "apply".into(),
            "--input-file".into(),
            request.to_string_lossy().into_owned(),
            "--user-config-root".into(),
            config.clone(),
            "--approved-sha256".into(),
            preview["approved_sha256"].as_str().unwrap().into(),
        ]);
        assert_eq!(applied["status"], "updated");
        let revised: Value =
            serde_json::from_slice(&fs::read(root.join(task_path)).unwrap()).unwrap();
        assert_eq!(revised["discussion"], collection["discussion"]);
        let preflight = public(vec![
            "execute".into(),
            "preflight".into(),
            "--execution-dir".into(),
            "outputs/work/executions/example".into(),
            "--task-path".into(),
            task_path.into(),
            "--task-id".into(),
            "TASK-001".into(),
            "--user-config-root".into(),
            config,
        ]);
        assert_eq!(preflight["schema"], "work-execute-preflight");
        assert_eq!(preflight["task_status"], "pending");
        assert_eq!(
            current(&root)["content_sha256"],
            session.commit.content_sha256
        );
    }

    #[test]
    fn writer_lock_failure_blocks_question_until_same_operation_recovers() {
        let (root, session) = setup("lock");
        invoke(
            &root,
            "discussion",
            "init",
            json!({"kind":"init","session":session}),
            0,
        );
        let view = current(&root);
        let operation = local(
            &view,
            "continuation-lock",
            json!({"action":"update_continuation","current_task_id":null}),
        );
        let directory = root.join("outputs/work/discussions/example");
        let guard = LocalWriterLock
            .acquire(&directory.join(".work-state-writer.lock"))
            .unwrap();
        let rejected = invoke(
            &root,
            "discussion",
            "update",
            json!({"kind":"update","operation":operation}),
            7,
        );
        assert_eq!(rejected["data"]["saved"], false);
        assert_eq!(current(&root)["revision"], 1);
        assert!(!directory.join("history/2").exists());
        drop(guard);
        let recovered = invoke(
            &root,
            "discussion",
            "recover",
            json!({"kind":"recover","session":null,"operation":operation}),
            0,
        );
        assert_eq!(recovered["data"]["data"]["view"]["revision"], 2);
    }

    #[test]
    fn large_committed_session_shares_one_decision_across_tasks_and_returns_minimal_views() {
        let (root, mut session) = setup("large");
        session.decisions=(1..=300).map(|number| serde_json::from_value(json!({"id":format!("D{number:03}"),"question":format!("Question {number}?"),"question_version":1,"options":[{"id":"first","label":"First","explanation":"Reason"}],"status":if number==8 {"pending"} else {"confirmed"},"tentative_option_id":null,"resolution":if number==8 {Value::Null} else {json!({"option_id":"first","rationale":"Historical detail ".repeat(200),"confirmation_evidence":"Confirmed"})},"status_reason":"Reason","status_evidence":"Evidence","added_reason":"Explicit scope","task_ids":if number==8 {json!(["TASK-001","TASK-002"])} else {json!([])},"dependencies":if number==8 {json!(["D001"])} else {json!([])},"source_references":["source.txt"]})).unwrap()).collect();
        session.next_decision_number = 301;
        let mut second = session.tasks[0].clone();
        second.id = "TASK-002".into();
        second.title = "Second task".into();
        second.review = Nullable::Null;
        session.tasks[0].review = Nullable::Null;
        session.tasks[0].decision_ids = vec!["D008".into()];
        second.decision_ids = vec!["D008".into()];
        session.tasks.push(second);
        session.next_task_number = 3;
        session.continuation.current_task_id = Nullable::Value("TASK-002".into());
        session.commit.content_sha256 =
            work_infrastructure::fixture_support::discussion_sha256(&session);
        invoke(
            &root,
            "discussion",
            "init",
            json!({"kind":"init","session":session}),
            0,
        );
        let view = current(&root);
        assert_eq!(view["task_count"], 2);
        assert_eq!(view["current_task_position"], 2);
        assert_eq!(view["progress"]["confirmed"], 299);
        assert_eq!(view["progress"]["known_total"], 300);
        assert_eq!(view["progress"]["remaining"], 1);
        assert_eq!(view["decisions"].as_array().unwrap().len(), 2);
        assert_eq!(view["pending_decision_ids"], json!(["D008"]));
        let d008 = view["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == "D008")
            .unwrap();
        assert_eq!(d008["task_ids"], json!(["TASK-001", "TASK-002"]));
        assert!(serde_json::to_vec(&view).unwrap().len() < 5000);
        assert!(
            !serde_json::to_string(&view)
                .unwrap()
                .contains("Historical detail")
        );
        let history = invoke(
            &root,
            "discussion",
            "history",
            json!({"kind":"history","revision":1}),
            0,
        )["data"]["data"]
            .clone();
        assert_eq!(history["decisions"].as_array().unwrap().len(), 300);
        assert!(
            history["decisions"][0]["resolution"]["rationale"]
                .as_str()
                .unwrap()
                .contains("Historical detail")
        );
        assert!(!root.join("outputs/work/tasks/example/index.json").exists());
    }

    #[test]
    fn bounded_authorization_and_source_drift_reject_without_changing_current_or_publishing() {
        let (root, mut session) = setup("authorization");
        session.authorization.allowed_actions = vec![DiscussionAction::UpdateContinuation];
        session.commit.content_sha256 =
            work_infrastructure::fixture_support::discussion_sha256(&session);
        invoke(
            &root,
            "discussion",
            "init",
            json!({"kind":"init","session":session}),
            0,
        );
        let before = fs::read(root.join("outputs/work/discussions/example/session.json")).unwrap();
        let view = current(&root);
        let op = local(
            &view,
            "forbidden-question",
            json!({"action":"prepare_question","question":{"decision_id":"D001","question_version":1,"text":"Unapproved?","display_options":{"1":"first"}}}),
        );
        let denied = invoke(
            &root,
            "discussion",
            "question",
            json!({"kind":"question","operation":op}),
            4,
        );
        assert_eq!(denied["reason_code"], "discussion_save_not_authorized");
        assert_eq!(denied["data"]["saved"], false);
        assert_eq!(denied["data"]["last_verified_revision"], 1);
        assert_eq!(denied["data"]["operation_id"], "forbidden-question");
        assert_eq!(
            fs::read(root.join("outputs/work/discussions/example/session.json")).unwrap(),
            before
        );
        assert!(
            !root
                .join("outputs/work/discussions/example/history/2")
                .exists()
        );
        let metadata = json!({"spec_id":"TASK-SPEC-001","title":"Full review","summary":"Full scope","execution_defaults":{"working_directory":".","os":"macos","shell":"zsh"}});
        fs::write(
            root.join("outputs/work/sources/example/SRC-001/source.txt"),
            b"changed original bytes",
        )
        .unwrap();
        let drift = invoke(
            &root,
            "task",
            "preview",
            json!({"kind":"preview","metadata":metadata}),
            5,
        );
        assert_eq!(drift["status"], "rejected");
        assert_eq!(
            fs::read(root.join("outputs/work/discussions/example/session.json")).unwrap(),
            before
        );
        assert!(!root.join("outputs/work/tasks/example/index.json").exists());
        assert!(
            !root
                .join("outputs/work/executions/example/index.json")
                .exists()
        );
    }
}
