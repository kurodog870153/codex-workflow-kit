//! Contracts that require launching the public executable.

use std::path::Path;
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::{fs, path::PathBuf};

use serde_json::Value;
use serde_json::json;
use work_flow::error::ExitCode;
use work_flow::progress::preview_value;
use work_infrastructure::progress_storage::LocalProgressStorage;
use work_infrastructure::writer_lock::{LocalWriterLock, WriterLock};

fn executable() -> &'static str {
    env!("CARGO_BIN_EXE_work")
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
        ("attempt", "work-attempt/v1", Some("attempt_id")),
        ("correction", "work-correction/v1", Some("correction_id")),
        ("handoff", "work-handoff/v1", None),
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
fn task_semantic_prepare_reads_file_and_leaves_plan_unchanged() {
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
    let fixture =
        repo.join("rust/crates/work-infrastructure/fixtures/task-draft-sources/valid/outputs/work/plans/example.json");
    let project = std::env::temp_dir().join(format!(
        "work-task-semantic-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let skill = project.join("work");
    let installed = skill.join("scripts/work");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::copy(executable(), &installed).unwrap();
    copy_tree(
        &repo.join("skills/work/references"),
        &skill.join("references"),
    );
    let plan_path = "outputs/work/plans/example.json";
    let plan_file = project.join(plan_path);
    fs::create_dir_all(plan_file.parent().unwrap()).unwrap();
    let mut plan: Value = serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
    plan["artifacts"]["task"] = json!("outputs/work/tasks/example/index.json");
    let plan_raw = work_infrastructure::fixture_support::render_plan(&plan).unwrap();
    fs::write(&plan_file, &plan_raw).unwrap();
    let input = project.join("semantic.json");
    fs::write(
        &input,
        serde_json::to_vec(&json!({"upsert":[{"title":"Task","goal":"Result",
        "scope":["Source"],"skill_id":null,
        "instruction_selection":{"selected_paths":[],"references":[]},"dependencies":[]}],
        "remove_task_ids":[],"current_task":{"upsert_position":1},"reason":null}))
        .unwrap(),
    )
    .unwrap();
    let output = Command::new(&installed)
        .args([
            "--project-root".to_owned(),
            project.to_string_lossy().into_owned(),
            "--verbose".into(),
            "task".into(),
            "semantic-prepare".into(),
            "--input-file".into(),
            input.to_string_lossy().into_owned(),
            "--requirement-id".into(),
            "example".into(),
            "--plan-path".into(),
            plan_path.into(),
            "--user-config-root".into(),
            project.to_string_lossy().into_owned(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(output.stderr.is_empty());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["data"]["status"], "prepared");
    assert_eq!(response["data"]["index"]["tasks"][0]["id"], "TASK-001");
    assert_eq!(fs::read(plan_file).unwrap(), plan_raw);
    assert!(!project.join("outputs/work/tasks").exists());
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
            "create",
            json!({}),
            vec![
                "--plan-path",
                plan_path,
                "--task-path",
                task_path,
                "--execution-dir",
                execution,
            ],
        ),
        (
            "spec-prepare",
            registry["items"]["work-spec-prepare-request/v1"]["description"]["example"].clone(),
            vec![],
        ),
        (
            "repair-prepare",
            registry["items"]["work-task-repair-prepare-request/v1"]["description"]["example"]
                .clone(),
            vec![],
        ),
        (
            "spec-validate",
            json!({"plan":{"artifacts":artifacts}}),
            vec![],
        ),
        ("repair-validate", json!({"artifacts":artifacts}), vec![]),
    ];
    let input = base.join("request.json");
    for (command, request, flags) in cases {
        fs::write(&input, serde_json::to_vec(&request).unwrap()).unwrap();
        let mut args = vec![
            "--project-root".to_owned(),
            project.to_string_lossy().into_owned(),
            "task".to_owned(),
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
            Some(6),
            "{command}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stderr.is_empty(), "{command}");
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(
            response["reason_code"], "task_collection_required",
            "{command}"
        );
        assert_eq!(fs::read(&plan).unwrap(), plan_raw, "{command}");
        assert!(!project.join("outputs/work/tasks").exists(), "{command}");
    }
}

#[test]
fn draft_cli_initial_save_read_and_rejections_use_public_process() {
    let repo = PathBuf::from(project_root());
    let fixture = repo.join(
        "rust/crates/work-infrastructure/fixtures/task-draft-sources/valid/outputs/work/tasks/example/drafts/index.json",
    );
    let base = std::env::temp_dir().join(format!(
        "work-draft-cli-process-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root = base.join("project");
    fs::create_dir_all(&root).unwrap();
    let root_arg = root.to_string_lossy().into_owned();
    let request = base.join("request.json");
    let request_arg = request.to_string_lossy().into_owned();
    let invoke = |suffix: &[String]| {
        let mut args = vec![
            "--project-root".to_owned(),
            root_arg.clone(),
            "--verbose".to_owned(),
            "task".to_owned(),
        ];
        args.extend_from_slice(suffix);
        let output = run(&args);
        assert!(output.stderr.is_empty());
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        (output.status.code(), response)
    };
    assert_eq!(
        invoke(&[
            "draft-read".into(),
            "--requirement-id".into(),
            "example".into()
        ])
        .0,
        Some(8)
    );
    assert!(!root.join("outputs").exists());
    fs::write(&request, b"{").unwrap();
    let init = vec![
        "draft-init".into(),
        "--input-file".into(),
        request_arg.clone(),
    ];
    let (code, response) = invoke(&init);
    assert_eq!(
        (code, response["reason_code"].as_str()),
        (Some(3), Some("invalid_json_contract"))
    );
    assert!(!root.join("outputs").exists());
    for suffix in [
        vec!["draft-init".into()],
        vec![
            "draft-save".into(),
            "--input-file".into(),
            request_arg.clone(),
        ],
        vec![
            "draft-save".into(),
            "--input-file".into(),
            request_arg.clone(),
            "--expected-revision".into(),
            "x".into(),
        ],
        vec!["draft-read".into()],
    ] {
        let (code, response) = invoke(&suffix);
        assert_eq!(
            (code, response["reason_code"].as_str()),
            (Some(2), Some("cli_usage_error")),
            "{suffix:?}"
        );
    }
    let index: Value = serde_json::from_slice(&fs::read(fixture).unwrap()).unwrap();
    let mut proposed = index.clone();
    proposed["revision"] = json!(2);
    proposed["tasks"][0]["status"] = json!("in_progress");
    let draft = json!({"schema":"work-task-draft/v1","requirement_id":"example",
        "task_id":"TASK-001","revision":1,"boundary_revision":1,
        "source":index["source"],"instructions_sha256":index["tasks"][0]["instructions_sha256"],
        "status":"in_progress","notes":["Discussion"],"confirmed_decisions":[],
        "tentative":[],"open_questions":["Which test?"],
        "next_discussion_point":"Confirm test."});
    let save = vec![
        "draft-save".into(),
        "--input-file".into(),
        request_arg.clone(),
        "--expected-revision".into(),
        "1".into(),
    ];
    for malformed in [
        json!({"index":proposed}),
        json!({"index":proposed,"draft":null}),
        json!({"index":proposed,"draft":draft,"approved":true}),
    ] {
        fs::write(&request, serde_json::to_vec(&malformed).unwrap()).unwrap();
        let (code, response) = invoke(&save);
        assert_eq!(code, Some(4), "{response}");
        assert_eq!(response["schema"], "work-cli-result/v1");
        assert!(!root.join("outputs").exists());
    }
    fs::write(&request, serde_json::to_vec(&index).unwrap()).unwrap();
    let (code, response) = invoke(&init);
    assert_eq!(code, Some(0), "{response}");
    let (code, response) = invoke(&[
        "draft-read".into(),
        "--requirement-id".into(),
        "example".into(),
    ]);
    assert_eq!(code, Some(0));
    assert_eq!(response["data"], index);
    fs::write(
        &request,
        serde_json::to_vec(&json!({"index":proposed,"draft":draft})).unwrap(),
    )
    .unwrap();
    let (code, response) = invoke(&save);
    assert_eq!(code, Some(0), "{response}");
    assert_eq!(response["data"]["revision"], 2);
    let (code, response) = invoke(&[
        "draft-read".into(),
        "--requirement-id".into(),
        "example".into(),
        "--task-id".into(),
        "TASK-001".into(),
    ]);
    assert_eq!(code, Some(0), "{response}");
    assert_eq!(response["data"], draft);
    let index_path = root.join("outputs/work/tasks/example/drafts/index.json");
    let committed = fs::read(&index_path).unwrap();
    let (code, response) = invoke(&save);
    assert_eq!(
        (code, response["reason_code"].as_str()),
        (Some(6), Some("draft_revision_conflict"))
    );
    assert_eq!(fs::read(index_path).unwrap(), committed);
    assert!(!root.join("outputs/work/tasks/example/task.json").exists());
    fs::write(&request, serde_json::to_vec(&index).unwrap()).unwrap();
    let (code, response) = invoke(&[
        "draft-recover".into(),
        "--input-file".into(),
        request_arg.clone(),
        "--expected-revision".into(),
        "0".into(),
    ]);
    assert_ne!(code, Some(0));
    assert_ne!(response["reason_code"], "invalid_object_fields");
    fs::write(
        &request,
        serde_json::to_vec(&json!({"index":proposed,"draft":draft})).unwrap(),
    )
    .unwrap();
    let (code, response) = invoke(&[
        "draft-recover".into(),
        "--input-file".into(),
        request_arg,
        "--expected-revision".into(),
        "1".into(),
    ]);
    assert_eq!(code, Some(0));
    assert_eq!(response["data"]["status"], "already_completed");
    let non_verbose = run(&[
        "--project-root".into(),
        root_arg,
        "task".into(),
        "draft-recover".into(),
        "--input-file".into(),
        request.to_string_lossy().into_owned(),
        "--expected-revision".into(),
        "1".into(),
    ]);
    assert_eq!(non_verbose.status.code(), Some(0));
    let full: Value = serde_json::from_slice(&non_verbose.stdout).unwrap();
    assert_eq!(full["data"]["revision"], 2);
    assert_eq!(full["data"]["display_copy"], "not_updated");
}

#[test]
fn draft_request_cli_saves_small_payload_and_reuses_selection() {
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
    let fixture = repo.join("rust/crates/work-infrastructure/fixtures/task-draft-sources/valid");
    let base = std::env::temp_dir().join(format!(
        "work-draft-request-process-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root = base.join("project");
    let skill = base.join("work");
    let installed = skill.join("scripts/work");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::copy(executable(), &installed).unwrap();
    copy_tree(
        &repo.join("skills/work/references"),
        &skill.join("references"),
    );
    for relative in [
        "outputs/work/plans/example.json",
        "outputs/work/tasks/example/drafts/index.json",
        "outputs/work/tasks/example/drafts/history/1/index.json",
    ] {
        let destination = root.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(fixture.join(relative), destination).unwrap();
    }
    let request = base.join("request.json");
    fs::write(&request, r#"{"status":"in_progress","notes":["Concrete discussion"],"confirmed_decisions":[],"tentative":[],"open_questions":["Which test?"],"next_discussion_point":"Confirm test."}"#).unwrap();
    let root_arg = root.to_string_lossy().into_owned();
    let request_arg = request.to_string_lossy().into_owned();
    for (revision, explicit) in [(1, true), (2, false)] {
        let mut arguments = vec![
            "--project-root".to_owned(),
            root_arg.clone(),
            "--verbose".to_owned(),
            "task".to_owned(),
            "draft-save-request".to_owned(),
            "--input-file".to_owned(),
            request_arg.clone(),
            "--requirement-id".to_owned(),
            "example".to_owned(),
            "--task-id".to_owned(),
            "TASK-001".to_owned(),
            "--expected-revision".to_owned(),
            revision.to_string(),
            "--plan-path".to_owned(),
            "outputs/work/plans/example.json".to_owned(),
            "--user-config-root".to_owned(),
            root_arg.clone(),
        ];
        if explicit {
            arguments.push("--general-only".to_owned());
        }
        let output = Command::new(&installed).args(&arguments).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(output.stderr.is_empty());
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["data"]["revision"], revision + 1);
    }
    let draft: Value = serde_json::from_slice(
        &fs::read(root.join("outputs/work/tasks/example/drafts/history/3/TASK-001.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(draft["notes"], json!(["Concrete discussion"]));
    assert_eq!(draft["revision"], 2);
}

#[test]
fn task_diagnostics_cli_is_read_only_and_invalid_item_precedes_writer_lock() {
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
    let fixture = repo.join("rust/crates/work-infrastructure/fixtures/task-diagnostics");
    let base = std::env::temp_dir().join(format!(
        "work-task-diagnostics-process-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root = base.join("project");
    let skill = base.join("work");
    let installed = skill.join("scripts/work");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::copy(executable(), &installed).unwrap();
    copy_tree(
        &repo.join("skills/work/references"),
        &skill.join("references"),
    );
    let paths = [
        "outputs/work/plans/example.json",
        "outputs/work/tasks/example/index.json",
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/executions/example/index.json",
    ];
    for relative in paths {
        let destination = root.join(relative);
        fs::create_dir_all(destination.parent().unwrap()).unwrap();
        fs::copy(fixture.join(relative), destination).unwrap();
    }
    let before = paths
        .iter()
        .map(|path| fs::read(root.join(path)).unwrap())
        .collect::<Vec<_>>();
    let root_arg = root.to_string_lossy().into_owned();
    let task_path = "outputs/work/tasks/example/index.json";
    let diagnose = Command::new(&installed)
        .args([
            "--project-root".into(),
            root_arg.clone(),
            "--verbose".into(),
            "task".into(),
            "diagnose".into(),
            "--path".into(),
            task_path.into(),
            "--plan-path".into(),
            "outputs/work/plans/example.json".into(),
            "--execution-dir".into(),
            "outputs/work/executions/example".into(),
            "--user-config-root".into(),
            root_arg.clone(),
        ])
        .output()
        .unwrap();
    assert_eq!(
        diagnose.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&diagnose.stdout)
    );
    assert!(diagnose.stderr.is_empty());
    let response: Value = serde_json::from_slice(&diagnose.stdout).unwrap();
    let direct = work_infrastructure::task::diagnostics::diagnose_task_collection(
        &root,
        &repo.join("skills/work"),
        &[],
        task_path,
    );
    assert_eq!(response["data"], direct);
    assert_eq!(response["data"]["normal_use_allowed"], true);
    for (path, original) in paths.iter().zip(&before) {
        assert_eq!(fs::read(root.join(path)).unwrap(), *original);
    }
    let item = root.join("outputs/work/tasks/example/tasks/TASK-001.json");
    fs::write(&item, b"{").unwrap();
    let index_before = fs::read(root.join(task_path)).unwrap();
    let execute = Command::new(&installed)
        .args([
            "--project-root".into(),
            root_arg.clone(),
            "--verbose".into(),
            "execute".into(),
            "record-begin".into(),
            "--user-config-root".into(),
            root_arg.clone(),
            "--task-path".into(),
            task_path.into(),
            "--execution-dir".into(),
            "outputs/work/executions/example".into(),
            "--task-id".into(),
            "TASK-001".into(),
            "--record-id".into(),
            "CMD-001".into(),
        ])
        .output()
        .unwrap();
    assert_ne!(execute.status.code(), Some(0));
    let rejected: Value = serde_json::from_slice(&execute.stdout).unwrap();
    assert_eq!(rejected["reason_code"], "invalid_json_contract");
    assert_eq!(fs::read(root.join(task_path)).unwrap(), index_before);
    assert_eq!(fs::read(&item).unwrap(), b"{");
    assert!(
        !root
            .join("outputs/work/executions/example/.work-state-writer.lock")
            .exists()
    );
}

#[test]
fn frozen_cli_responses_match_at_process_boundary() {
    let fixtures: Value =
        serde_json::from_str(include_str!("process_baseline.json")).expect("frozen fixtures");
    let root = project_root();
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
                .replace(&root, "<PROJECT_ROOT>"),
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
    assert_eq!(count, 106);
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
    assert_eq!(commands.len(), 106);
    let raw = format!("{}\n", commands.join("\n"));
    assert_eq!(
        work_infrastructure::codec::sha256_hex(raw.as_bytes()),
        "3b38859c8ede2db6006846f0bd43aa8a97ae8c90f5d59dd84436ea31aa479d54"
    );
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
    assert_eq!(response["schema"], "work-cli-result/v1");
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
    for command in ["validate", "build-plan-to-task"] {
        let mut arguments = vec![
            "--project-root".into(),
            root.clone(),
            "handoff".into(),
            command.into(),
            "--input-file".into(),
            input.to_string_lossy().into_owned(),
        ];
        if command == "build-plan-to-task" {
            arguments.extend([
                "--plan-path".into(),
                "outputs/work/plans/missing.json".into(),
                "--user-config-root".into(),
                root.clone(),
            ]);
        }
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(3), "{command}");
        assert!(output.stderr.is_empty(), "{command}");
        let response: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(response["schema"], "work-cli-result/v1", "{command}");
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
    let project = base.join("project");
    let plan_path = "outputs/work/plans/example.json";
    let plan = project.join(plan_path);
    fs::create_dir_all(plan.parent().unwrap()).unwrap();
    fs::copy(
        repo.join("rust/crates/work-infrastructure/fixtures/task-diagnostics")
            .join(plan_path),
        &plan,
    )
    .unwrap();
    let original = fs::read(&plan).unwrap();
    let request = base.join("request.json");
    fs::write(
        &request,
        r#"{"summary":"Build the tasks.","affected_ids":["GOAL-001"]}"#,
    )
    .unwrap();
    let built = Command::new(&installed)
        .args([
            "--project-root",
            project.to_str().unwrap(),
            "--verbose",
            "handoff",
            "build-plan-to-task",
            "--input-file",
            request.to_str().unwrap(),
            "--plan-path",
            plan_path,
            "--user-config-root",
            project.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(built.status.code(), Some(0));
    assert!(built.stderr.is_empty());
    let response: Value = serde_json::from_slice(&built.stdout).unwrap();
    assert_eq!(response["schema"], "work-cli-result/v1");
    assert_eq!(response["data"]["schema"], "work-handoff/v1");
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
    assert_eq!(fs::read(&plan).unwrap(), original);
    assert!(!project.join("outputs/work/tasks").exists());
    assert!(!project.join("outputs/work/executions").exists());

    let task_path = "outputs/work/tasks/example/index.json";
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
            "build-task-to-plan",
            "verify-task-to-plan",
            return_request,
            vec!["--task-path", task_path],
            vec!["--plan-path", plan_path, "--task-path", task_path],
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
                "--plan-path",
                plan_path,
                "--task-path",
                task_path,
                "--task-id",
                "TASK-001",
                "--preflight",
            ],
        ),
        (
            "build-execute-to-plan",
            "verify-execute-to-plan",
            preflight_request,
            vec![
                "--task-path",
                task_path,
                "--task-id",
                "TASK-001",
                "--preflight",
            ],
            vec![
                "--plan-path",
                plan_path,
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
        assert_eq!(built["schema"], "work-handoff/v1", "{build}");
        assert_eq!(built["artifacts"]["task"], task_path, "{build}");
        fs::write(&handoff, serde_json::to_vec(&built).unwrap()).unwrap();
        let checked = run_handoff(&project, verify, &handoff, &verify_flags);
        assert_eq!(checked["status"], "valid", "{verify}");
        assert_eq!(checked["source"], built["source"], "{verify}");
    }
    assert_eq!(fs::read(plan).unwrap(), original);

    let custom_project = base.join("custom-project");
    let custom_plan_path = "custom/plans/example.json";
    let custom_task_path = "custom/tasks/example/index.json";
    let custom_artifacts = json!({"plan":custom_plan_path,"task":custom_task_path,
        "execution":"custom/executions/example"});
    let mut custom_plan = work_infrastructure::codec::parse_json_contract(&original).unwrap();
    custom_plan["artifacts"] = custom_artifacts.clone();
    let custom_plan_raw = work_infrastructure::fixture_support::render_plan(&custom_plan).unwrap();
    let custom_plan_file = custom_project.join(custom_plan_path);
    fs::create_dir_all(custom_plan_file.parent().unwrap()).unwrap();
    fs::write(&custom_plan_file, &custom_plan_raw).unwrap();
    let mut custom_index: Value =
        serde_json::from_slice(&fs::read(project.join(task_path)).unwrap()).unwrap();
    custom_index["artifacts"] = custom_artifacts.clone();
    custom_index["source_plan"]["canonical_sha256"] =
        json!(work_infrastructure::codec::sha256_hex(&custom_plan_raw));
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
    for (build, verify, semantic, build_flags, verify_flags) in [
        (
            "build-plan-to-task",
            "verify-plan-to-task",
            json!({"summary":"Build the tasks.","affected_ids":["GOAL-001"]}),
            vec!["--plan-path", custom_plan_path],
            vec!["--plan-path", custom_plan_path],
        ),
        (
            "build-task-to-execute",
            "verify-task-to-execute",
            json!({"summary":"Start execution."}),
            vec!["--task-path", custom_task_path, "--task-id", "TASK-001"],
            vec!["--task-path", custom_task_path, "--task-id", "TASK-001"],
        ),
        (
            "build-task-to-plan",
            "verify-task-to-plan",
            json!({"summary":"Review specification.",
            "confirmed_approach":"Retain the interface.","requested_changes":["Clarify scope."],
            "preserve":["Current behavior."],"affected_ids":["TASK-001"],
            "validation_requirements":["Review criteria."]}),
            vec!["--task-path", custom_task_path],
            vec![
                "--plan-path",
                custom_plan_path,
                "--task-path",
                custom_task_path,
            ],
        ),
    ] {
        fs::write(&request, serde_json::to_vec(&semantic).unwrap()).unwrap();
        let built = run_handoff(&custom_project, build, &request, &build_flags);
        assert_eq!(built["artifacts"], custom_artifacts, "{build}");
        fs::write(&handoff, serde_json::to_vec(&built).unwrap()).unwrap();
        let checked = run_handoff(&custom_project, verify, &handoff, &verify_flags);
        assert_eq!(checked["status"], "valid", "{verify}");
    }
    assert_eq!(fs::read(&custom_plan_file).unwrap(), custom_plan_raw);
    assert_eq!(fs::read(&custom_index_file).unwrap(), custom_index_raw);

    let closed_project = base.join("closed-project");
    let closed_fixture =
        repo.join("rust/crates/work-infrastructure/fixtures/handoff-closed/stopped");
    for relative in [
        plan_path,
        task_path,
        "outputs/work/tasks/example/tasks/TASK-001.json",
        "outputs/work/tasks/example/tasks/TASK-002.json",
        "outputs/work/executions/example/index.json",
        "outputs/work/executions/example/TASK-001/ATTEMPT-001/attempt.json",
    ] {
        let target = closed_project.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(closed_fixture.join(relative), target).unwrap();
    }
    let closed_request = json!({"summary":"調整已確認範圍", "reason":"Clarify specification",
        "confirmed_approach":"保留既有介面", "requested_changes":["新增驗收條件"],
        "preserve":["既有功能"], "affected_ids":["GOAL-001","TASK-001"],
        "validation_requirements":["重新確認驗收條件"]});
    fs::write(&request, serde_json::to_vec(&closed_request).unwrap()).unwrap();
    for (build, verify) in [
        ("build-execute-to-task", "verify-execute-to-task"),
        ("build-execute-to-plan", "verify-execute-to-plan"),
    ] {
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
                "ATTEMPT-001",
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
                "--plan-path",
                plan_path,
                "--task-path",
                task_path,
                "--task-id",
                "TASK-001",
                "--attempt-id",
                "ATTEMPT-001",
            ],
        );
        assert_eq!(checked["status"], "valid", "{verify}");
        assert_eq!(checked["source"], built["source"], "{verify}");
    }
}

#[test]
fn progress_prepare_validate_save_and_resume_across_processes() {
    let base = std::env::temp_dir().join(format!(
        "work-progress-process-t25-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let project = base.join("project");
    fs::create_dir_all(&project).unwrap();
    let semantic = base.join("semantic.json");
    fs::write(
        &semantic,
        serde_json::to_vec(
            &json!({"title":"Resume later","request":"Keep the agreed scope.",
            "current_task_id":null,"context":{"scope":["Planning"]},"source_status":[],
            "notes":["Retain evidence"],"confirmed_decisions":[{"statement":"Keep scope"}],
            "tentative":[],"open_questions":["Which outcome?"],
            "next_discussion_point":"Confirm outcome."}),
        )
        .unwrap(),
    )
    .unwrap();
    let invoke = |arguments: &[&str]| {
        let output = Command::new(installed_executable())
            .args(["--project-root", project.to_str().unwrap(), "--verbose"])
            .args(arguments)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{arguments:?}: {output:?}");
        assert!(output.stderr.is_empty());
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["data"].clone()
    };
    let prepared = invoke(&[
        "progress",
        "prepare",
        "--input-file",
        semantic.to_str().unwrap(),
        "--requirement-id",
        "example",
        "--mode",
        "plan",
        "--expected-revision",
        "0",
    ]);
    assert_eq!(prepared["schema"], "work-progress-prepare/v1");
    assert_eq!(prepared["source_validation"], "not_checked");
    assert_eq!(prepared["evidence_trust"], "historical_context_only");
    assert_eq!(prepared["formal_readiness"], "not_established");
    assert!(!project.join("outputs/work").exists());
    let progress_file = base.join("progress.json");
    fs::write(
        &progress_file,
        serde_json::to_vec(&prepared["progress"]).unwrap(),
    )
    .unwrap();
    for (command, path_flag, path) in [
        ("plan", "--plan-path", "outputs/work/plans/example.json"),
        (
            "task",
            "--task-path",
            "outputs/work/tasks/example/index.json",
        ),
    ] {
        let output = Command::new(installed_executable())
            .args([
                "--project-root",
                project.to_str().unwrap(),
                command,
                "validate",
                "--input-file",
                progress_file.to_str().unwrap(),
                path_flag,
                path,
                "--user-config-root",
                project.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(ExitCode::Contract as i32),
            "{command}"
        );
    }
    assert!(!project.join("outputs/work").exists());
    let validated = invoke(&[
        "progress",
        "validate",
        "--input-file",
        progress_file.to_str().unwrap(),
        "--expected-revision",
        "0",
    ]);
    assert_eq!(validated["approved_sha256"], prepared["approved_sha256"]);
    let saved = invoke(&[
        "progress",
        "save",
        "--input-file",
        progress_file.to_str().unwrap(),
        "--expected-revision",
        "0",
        "--approved-sha256",
        prepared["approved_sha256"].as_str().unwrap(),
    ]);
    let resumed = invoke(&[
        "progress",
        "read",
        "--requirement-id",
        "example",
        "--mode",
        "plan",
    ]);
    assert_eq!(resumed["progress"], prepared["progress"]);
    assert_eq!(resumed["sha256"], saved["sha256"]);
    let managed = project.join("outputs/work");
    assert_eq!(fs::read_dir(managed).unwrap().count(), 1);
}

#[test]
fn specification_constraint_continuation_across_installed_processes() {
    use work_flow::task::validate_collection;
    use work_infrastructure::codec::sha256_hex;
    use work_infrastructure::fixture_support::{
        build_initial_execution_index, render_execution_index, render_plan, render_task_index,
    };
    use work_infrastructure::hierarchy_catalog::LocalHierarchyCatalog;
    use work_infrastructure::plan_storage::LocalPlanStorage;
    use work_infrastructure::skill_catalog::LocalSkillCatalog;
    use work_infrastructure::task::storage::LocalTaskStorage;

    let repo = PathBuf::from(project_root());
    let fixture =
        repo.join("rust/crates/work-infrastructure/fixtures/specification-update/plan-summary");
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
            "outputs/work/plans/example.json",
            "outputs/work/tasks/example/index.json",
            "outputs/work/tasks/example/tasks/TASK-001.json",
        ] {
            let target = project.join(relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(relative), target).unwrap();
        }
        fs::write(project.join("src.txt"), b"source\n").unwrap();
        let plan_path = project.join("outputs/work/plans/example.json");
        let mut plan: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
        plan["constraints"] = json!([{"id":"CONSTRAINT-001","statement":"Original boundary",
            "applies_to":["GOAL-001"]}]);
        let plan_raw = render_plan(&plan).unwrap();
        fs::write(&plan_path, &plan_raw).unwrap();
        let index_path = project.join("outputs/work/tasks/example/index.json");
        let mut index: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
        index["source_plan"]["canonical_sha256"] = json!(sha256_hex(&plan_raw));
        fs::write(&index_path, render_task_index(&index).unwrap()).unwrap();
        let skill = repo.join("skills/work");
        let collection = validate_collection(
            &LocalHierarchyCatalog {
                skill_root: skill.clone(),
            },
            &LocalSkillCatalog { roots: vec![] },
            &LocalPlanStorage {
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
        fs::write(&request_path, serde_json::to_vec(&json!({
            "schema":"work-spec-prepare-request/v1","requirement_id":"example",
            "reason":"Confirm the constraint wording and save progress.",
            "edits":[{"target":{"artifact":"plan"},"field":"constraints",
                "semantic_after":[{"key":"boundary","existing_position":1,
                    "statement":"Confirmed boundary","applies_to":[{"collection":"goals","position":1}]}]}]
        })).unwrap()).unwrap();
        let prepared_path = base.join("prepared.json");
        let progress_content_path = base.join("progress-content.json");
        fs::write(
            &progress_content_path,
            serde_json::to_vec(&json!({
                "title":"Specification continuation","request":"Preserve the checkpoint.",
                "current_task_id":null,"context":{"affected_ids":["CONSTRAINT-001"]},
                "source_status":[],"notes":["Candidate reviewed."],
                "confirmed_decisions":[{"statement":"Use confirmed wording."}],
                "tentative":[],"open_questions":[],
                "next_discussion_point":"Continue after verification."
            }))
            .unwrap(),
        )
        .unwrap();
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
                "task",
                "spec-prepare",
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
            json!(["/plan/constraints", "/task_index/source_plan"])
        );
        assert!(prepared["data"].get("request").is_none());
        let validated = invoke(
            &[
                "task",
                "spec-validate",
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
        let progress_prepared = invoke(
            &[
                "progress",
                "prepare",
                "--input-file",
                progress_content_path.to_str().unwrap(),
                "--requirement-id",
                "example",
                "--mode",
                "plan",
                "--expected-revision",
                "0",
            ],
            0,
        );
        let progress_path = base.join("progress.json");
        fs::write(
            &progress_path,
            serde_json::to_vec(&progress_prepared["data"]["progress"]).unwrap(),
        )
        .unwrap();
        let progress_approval = invoke(
            &[
                "progress",
                "validate",
                "--input-file",
                progress_path.to_str().unwrap(),
                "--expected-revision",
                "0",
            ],
            0,
        )["data"]["approved_sha256"]
            .as_str()
            .unwrap()
            .to_owned();
        let published = invoke(
            &[
                "task",
                "spec-update",
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
                serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
            changed["summary"] = json!("Unexpected drift after publication.");
            fs::write(&plan_path, render_plan(&changed).unwrap()).unwrap();
            let rejected = invoke(
                &[
                    "task",
                    "spec-verify",
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
                    .join("outputs/work/progress/example/plan/progress.json")
                    .exists()
            );
        } else {
            let verified = invoke(
                &[
                    "task",
                    "spec-verify",
                    "--input-file",
                    verify_path.to_str().unwrap(),
                    "--user-config-root",
                    project.to_str().unwrap(),
                ],
                0,
            );
            assert_eq!(verified["data"]["verified"], true);
            let saved = invoke(
                &[
                    "progress",
                    "save",
                    "--input-file",
                    progress_path.to_str().unwrap(),
                    "--expected-revision",
                    "0",
                    "--approved-sha256",
                    &progress_approval,
                ],
                0,
            );
            let restored = invoke(
                &[
                    "progress",
                    "read",
                    "--requirement-id",
                    "example",
                    "--mode",
                    "plan",
                ],
                0,
            );
            assert_eq!(saved["data"]["sha256"], restored["data"]["sha256"]);
            assert_eq!(restored["data"]["progress"]["revision"], 1);
            let installed: Value = serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
            assert_eq!(
                installed["constraints"][0]["statement"],
                "Confirmed boundary"
            );
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
        assert_eq!(response["schema"], "work-cli-result/v1");
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
    assert!(count > 106);
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
    fs::write(&input, "$work plan -- 需求").unwrap();
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
    assert_eq!(response["schema"], "work-cli-result/v1");
    assert_eq!(response["data"]["request"], " 需求");
    let missing = invoke("找不到 missing.json");
    assert_eq!(missing.status.code(), Some(8));
    assert!(missing.stderr.is_empty());
    let response: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert_eq!(response["reason_code"], "input_file_read_failed");
    assert_eq!(fs::read_dir(PathBuf::from(&root)).unwrap().count(), 0);
}

#[test]
fn progress_writer_lock_prevents_another_process_from_publishing() {
    let root = std::env::temp_dir().join(format!(
        "work-progress-process-lock-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    let progress = json!({"schema":"work-discussion-progress/v1",
        "requirement_id":"example","mode":"plan","revision":1,
        "status":"discussion_only","title":"Example","request":"Example request.",
        "current_task_id":null,"context":{},"source_status":[],"notes":[],
        "confirmed_decisions":[],"tentative":[],"open_questions":[],
        "next_discussion_point":"Continue."});
    let storage = LocalProgressStorage {
        project_root: root.clone(),
    };
    let approved = preview_value(&storage, &progress, 0).unwrap()["approved_sha256"]
        .as_str()
        .unwrap()
        .to_owned();
    let input = root.join("request.json");
    fs::write(&input, serde_json::to_vec(&progress).unwrap()).unwrap();
    let directory = root.join("outputs/work/progress/example/plan");
    fs::create_dir_all(&directory).unwrap();
    let guard = LocalWriterLock
        .acquire(&directory.join(".work-state-writer.lock"))
        .unwrap();
    let arguments = [
        "--project-root",
        root.to_str().unwrap(),
        "progress",
        "save",
        "--input-file",
        input.to_str().unwrap(),
        "--expected-revision",
        "0",
        "--approved-sha256",
        &approved,
    ];
    let blocked = Command::new(installed_executable())
        .args(arguments)
        .output()
        .unwrap();
    assert_eq!(
        blocked.status.code(),
        Some(work_flow::error::ExitCode::LockConflict as i32)
    );
    assert!(blocked.stderr.is_empty());
    let response: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(response["reason_code"], "work_state_writer_busy");
    assert!(!directory.join("progress.json").exists());
    assert!(!directory.join("history").exists());
    drop(guard);
    let saved = Command::new(installed_executable())
        .args(arguments)
        .output()
        .unwrap();
    assert_eq!(saved.status.code(), Some(0));
    assert!(saved.stderr.is_empty());
    assert!(directory.join("progress.json").is_file());
}
