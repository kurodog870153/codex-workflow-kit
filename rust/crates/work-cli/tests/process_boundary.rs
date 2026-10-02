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
            "prepare".into(),
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
    assert_eq!(fs::read(&plan_file).unwrap(), plan_raw);
    assert!(!project.join("outputs/work/tasks").exists());
    let approved = project.join("approved-task.json");
    fs::write(&approved, serde_json::to_vec(&response["data"]).unwrap()).unwrap();
    let save_args = [
        "--project-root",
        project.to_str().unwrap(),
        "--verbose",
        "task",
        "save",
        "--input-file",
        approved.to_str().unwrap(),
        "--requirement-id",
        "example",
        "--plan-path",
        plan_path,
        "--user-config-root",
        project.to_str().unwrap(),
    ];
    plan["summary"] = json!("Changed after approval");
    fs::write(
        &plan_file,
        work_infrastructure::fixture_support::render_plan(&plan).unwrap(),
    )
    .unwrap();
    let drift = Command::new(&installed).args(save_args).output().unwrap();
    assert_eq!(
        drift.status.code(),
        Some(6),
        "{}",
        String::from_utf8_lossy(&drift.stdout)
    );
    let drift: Value = serde_json::from_slice(&drift.stdout).unwrap();
    assert_eq!(drift["reason_code"], "draft_source_drift");
    assert!(!project.join("outputs/work/tasks").exists());
    fs::write(&plan_file, &plan_raw).unwrap();
    let saved = Command::new(&installed).args(save_args).output().unwrap();
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stdout)
    );
    let saved: Value = serde_json::from_slice(&saved.stdout).unwrap();
    assert_eq!(saved["data"]["status"], "saved");
    let storage = work_infrastructure::task::draft_storage::LocalTaskDraftStorage {
        project_root: project.clone(),
    };
    assert_eq!(
        storage.read_planning_index("example").unwrap(),
        response["data"]["index"]
    );
    let repeated = Command::new(&installed).args(save_args).output().unwrap();
    assert!(repeated.status.success());
    let repeated: Value = serde_json::from_slice(&repeated.stdout).unwrap();
    assert_eq!(repeated["data"]["status"], "already_completed");
    let revision_request = project.join("revision.json");
    fs::write(
        &revision_request,
        serde_json::to_vec(&json!({
        "upsert":[
            {"existing_task_id":"TASK-001","title":"Task","goal":"Result",
                "scope":["Source"],"skill_id":null,"dependencies":[]},
            {"title":"Follow-up","goal":"Additional result","scope":["Source"],
                "skill_id":null,"instruction_selection":{"selected_paths":[],"references":[]},
                "dependencies":[{"existing_task_id":"TASK-001"}]}
        ],"remove_task_ids":[],"current_task":{"upsert_position":2},
        "reason":"Add confirmed follow-up"}))
        .unwrap(),
    )
    .unwrap();
    let revised = Command::new(&installed)
        .args([
            "--project-root",
            project.to_str().unwrap(),
            "--verbose",
            "task",
            "prepare",
            "--input-file",
            revision_request.to_str().unwrap(),
            "--requirement-id",
            "example",
            "--plan-path",
            plan_path,
            "--user-config-root",
            project.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        revised.status.success(),
        "{}",
        String::from_utf8_lossy(&revised.stdout)
    );
    let revised: Value = serde_json::from_slice(&revised.stdout).unwrap();
    assert_eq!(revised["data"]["index"]["revision"], 2);
    fs::write(&approved, serde_json::to_vec(&revised["data"]).unwrap()).unwrap();
    let rejected_revision = Command::new(&installed)
        .args(save_args)
        .args(["--expected-revision", "2"])
        .output()
        .unwrap();
    assert_eq!(
        rejected_revision.status.code(),
        Some(ExitCode::CliUsage as i32)
    );
    let rejected_revision: Value = serde_json::from_slice(&rejected_revision.stdout).unwrap();
    assert_eq!(
        rejected_revision["reason_code"],
        "invalid_initial_task_save_options"
    );
    assert_eq!(
        storage.read_planning_index("example").unwrap()["revision"],
        1
    );
    let saved_revision = Command::new(&installed).args(save_args).output().unwrap();
    assert!(
        saved_revision.status.success(),
        "{}",
        String::from_utf8_lossy(&saved_revision.stdout)
    );
    let saved_revision: Value = serde_json::from_slice(&saved_revision.stdout).unwrap();
    assert_eq!(saved_revision["data"]["revision"], 2);
    assert_eq!(
        storage.read_planning_index("example").unwrap(),
        revised["data"]["index"]
    );
    let mut changed_plan: Value = serde_json::from_slice(&plan_raw).unwrap();
    changed_plan["summary"] = json!("Confirmed source change");
    let changed_raw = work_infrastructure::fixture_support::render_plan(&changed_plan).unwrap();
    fs::write(&plan_file, &changed_raw).unwrap();
    let status = Command::new(&installed)
        .args([
            "--project-root",
            project.to_str().unwrap(),
            "--verbose",
            "task",
            "status",
            "--requirement-id",
            "example",
            "--plan-path",
            plan_path,
            "--user-config-root",
            project.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["data"]["source_validation"], "review_required");
    assert_eq!(
        status["data"]["required_checks"],
        json!(["plan validate", "task status"])
    );
    let source_request = project.join("source-update.json");
    fs::write(
        &source_request,
        serde_json::to_vec(&json!({
            "reason":"Confirmed Plan change",
            "selections":{
                "TASK-001":{"selected_paths":[],"references":[]},
                "TASK-002":{"selected_paths":[],"references":[]}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    let source_candidate = Command::new(&installed)
        .args([
            "--project-root",
            project.to_str().unwrap(),
            "--verbose",
            "task",
            "prepare",
            "--input-file",
            source_request.to_str().unwrap(),
            "--requirement-id",
            "example",
            "--plan-path",
            plan_path,
            "--user-config-root",
            project.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        source_candidate.status.success(),
        "{}",
        String::from_utf8_lossy(&source_candidate.stdout)
    );
    let source_candidate: Value = serde_json::from_slice(&source_candidate.stdout).unwrap();
    assert_eq!(source_candidate["data"]["index"]["revision"], 3);
    assert_eq!(
        source_candidate["data"]["affected_task_ids"],
        json!(["TASK-001", "TASK-002"])
    );
    fs::write(
        &approved,
        serde_json::to_vec(&source_candidate["data"]).unwrap(),
    )
    .unwrap();
    changed_plan["summary"] = json!("Changed after source approval");
    fs::write(
        &plan_file,
        work_infrastructure::fixture_support::render_plan(&changed_plan).unwrap(),
    )
    .unwrap();
    let stale = Command::new(&installed).args(save_args).output().unwrap();
    assert!(!stale.status.success());
    assert_eq!(
        storage.read_planning_index("example").unwrap()["revision"],
        2
    );
    fs::write(&plan_file, &changed_raw).unwrap();
    let rejected_source_revision = Command::new(&installed)
        .args(save_args)
        .args(["--expected-revision", "2"])
        .output()
        .unwrap();
    assert_eq!(
        rejected_source_revision.status.code(),
        Some(ExitCode::CliUsage as i32)
    );
    let rejected_source_revision: Value =
        serde_json::from_slice(&rejected_source_revision.stdout).unwrap();
    assert_eq!(
        rejected_source_revision["reason_code"],
        "invalid_initial_task_save_options"
    );
    assert_eq!(
        storage.read_planning_index("example").unwrap()["revision"],
        2
    );
    let source_saved = Command::new(&installed).args(save_args).output().unwrap();
    assert!(
        source_saved.status.success(),
        "{}",
        String::from_utf8_lossy(&source_saved.stdout)
    );
    assert_eq!(
        storage.read_planning_index("example").unwrap(),
        source_candidate["data"]["index"]
    );
    let source_repeated = Command::new(&installed).args(save_args).output().unwrap();
    assert!(source_repeated.status.success());
    let source_repeated: Value = serde_json::from_slice(&source_repeated.stdout).unwrap();
    assert_eq!(source_repeated["data"]["status"], "already_completed");
}

#[test]
fn task_preview_and_apply_create_only_the_first_formal_collection() {
    let fixture = PathBuf::from(project_root())
        .join("rust/crates/work-infrastructure/fixtures/task-assembly");
    let root = std::env::temp_dir().join(format!(
        "work-task-public-lifecycle-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let plan = root.join("outputs/work/plans/example.json");
    fs::create_dir_all(plan.parent().unwrap()).unwrap();
    fs::copy(fixture.join("plan.json"), &plan).unwrap();
    let semantic = root.join("semantic.json");
    fs::write(&semantic, serde_json::to_vec(&json!({
        "upsert":[{"title":"Task","goal":"Result","scope":["Source"],"skill_id":null,
            "instruction_selection":{"selected_paths":[],"references":["task.general.task-records"]},
            "dependencies":[]}],"remove_task_ids":[],
        "current_task":{"upsert_position":1},"reason":null
    })).unwrap()).unwrap();
    let public_args = |command: &str, input: &Path, task_id: Option<&str>| {
        let mut args = vec![
            "--project-root".into(),
            root.to_string_lossy().into_owned(),
            "--verbose".into(),
            "task".into(),
            command.into(),
            "--input-file".into(),
            input.to_string_lossy().into_owned(),
            "--requirement-id".into(),
            "example".into(),
            "--plan-path".into(),
            "outputs/work/plans/example.json".into(),
            "--user-config-root".into(),
            root.to_string_lossy().into_owned(),
        ];
        if let Some(task_id) = task_id {
            args.extend(["--task-id".into(), task_id.into()]);
        }
        args
    };
    let prepared = run(&public_args("prepare", &semantic, None));
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stdout)
    );
    let prepared: Value = serde_json::from_slice(&prepared.stdout).unwrap();
    let approved = root.join("approved.json");
    fs::write(&approved, serde_json::to_vec(&prepared["data"]).unwrap()).unwrap();
    let saved = run(&public_args("save", &approved, None));
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stdout)
    );
    let fixture_draft: Value =
        serde_json::from_slice(&fs::read(fixture.join("draft.json")).unwrap()).unwrap();
    let discussion = root.join("discussion.json");
    fs::write(
        &discussion,
        serde_json::to_vec(&json!({
            "status":"refined","notes":[],"confirmed_decisions":[],"tentative":[],
            "open_questions":[],"next_discussion_point":null,
            "task_candidate":fixture_draft["task_candidate"]
        }))
        .unwrap(),
    )
    .unwrap();
    let saved = run(&public_args("save", &discussion, Some("TASK-001")));
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stdout)
    );
    let storage = work_infrastructure::task::draft_storage::LocalTaskDraftStorage {
        project_root: root.clone(),
    };
    let saved_index = storage.read_planning_index("example").unwrap();
    assert_eq!(saved_index["revision"], 2);
    assert_eq!(saved_index["tasks"][0]["status"], "refined");
    assert!(
        saved_index["tasks"][0]["draft_ref"]["sha256"]
            .as_str()
            .is_some()
    );
    let status = run(&[
        "--project-root".into(),
        root.to_string_lossy().into_owned(),
        "--verbose".into(),
        "task".into(),
        "status".into(),
        "--requirement-id".into(),
        "example".into(),
        "--plan-path".into(),
        "outputs/work/plans/example.json".into(),
        "--user-config-root".into(),
        root.to_string_lossy().into_owned(),
    ]);
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["data"]["next_action"], "assemble_for_review");
    assert_eq!(status["data"]["required_checks"], json!(["task preview"]));
    let metadata = root.join("metadata.json");
    fs::write(&metadata, fs::read(fixture.join("metadata.json")).unwrap()).unwrap();
    let args = |command: &str, approval: Option<&str>| {
        let mut args = vec![
            "--project-root".into(),
            root.to_string_lossy().into_owned(),
            "--verbose".into(),
            "task".into(),
            command.into(),
            "--input-file".into(),
            metadata.to_string_lossy().into_owned(),
            "--requirement-id".into(),
            "example".into(),
            "--plan-path".into(),
            "outputs/work/plans/example.json".into(),
            "--user-config-root".into(),
            root.to_string_lossy().into_owned(),
        ];
        if let Some(approval) = approval {
            args.extend(["--approved-sha256".into(), approval.into()]);
        }
        args
    };
    let preview = run(&args("preview", None));
    assert!(
        preview.status.success(),
        "{}",
        String::from_utf8_lossy(&preview.stdout)
    );
    let preview: Value = serde_json::from_slice(&preview.stdout).unwrap();
    let approval = preview["data"]["approval_sha256"].as_str().unwrap();
    assert!(!root.join("outputs/work/tasks/example/index.json").exists());
    let applied = run(&args("apply", Some(approval)));
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stdout)
    );
    let applied: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(applied["data"]["status"], "created");
    assert!(root.join("outputs/work/tasks/example/index.json").is_file());
    assert!(
        root.join("outputs/work/executions/example/index.json")
            .is_file()
    );
    let repeated = run(&args("apply", Some(approval)));
    assert!(!repeated.status.success());
    let execution_index = root.join("outputs/work/executions/example/index.json");
    fs::remove_file(&execution_index).unwrap();
    let recovered = run(&args("recover", Some(approval)));
    assert!(
        recovered.status.success(),
        "{}",
        String::from_utf8_lossy(&recovered.stdout)
    );
    let recovered: Value = serde_json::from_slice(&recovered.stdout).unwrap();
    assert_eq!(recovered["data"]["status"], "recovered");
    assert!(execution_index.is_file());
    let repeated_recovery = run(&args("recover", Some(approval)));
    assert!(repeated_recovery.status.success());
    let repeated_recovery: Value = serde_json::from_slice(&repeated_recovery.stdout).unwrap();
    assert_eq!(repeated_recovery["data"]["status"], "already_completed");
    let wrong = run(&args("recover", Some(&"0".repeat(64))));
    assert!(!wrong.status.success());
    let index_path = root.join("outputs/work/tasks/example/index.json");
    let mut changed = fs::read(&index_path).unwrap();
    changed.push(b' ');
    fs::write(&index_path, changed).unwrap();
    let conflict = run(&args("recover", Some(approval)));
    assert!(!conflict.status.success());
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
            registry["items"]["work-spec-prepare-request/v1"]["description"]["example"].clone(),
            vec![],
        ),
        ("preview", json!({"plan":{"artifacts":artifacts}}), vec![]),
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
fn removed_task_commands_return_cli_usage_error() {
    for command in [
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
            "save".to_owned(),
            "--input-file".to_owned(),
            request_arg.clone(),
            "--requirement-id".to_owned(),
            "example".to_owned(),
            "--task-id".to_owned(),
            "TASK-001".to_owned(),
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
    let status_args = [
        "--project-root",
        root_arg.as_str(),
        "--verbose",
        "task",
        "status",
        "--requirement-id",
        "example",
        "--task-id",
        "TASK-001",
        "--plan-path",
        "outputs/work/plans/example.json",
        "--user-config-root",
        root_arg.as_str(),
    ];
    let status = Command::new(&installed).args(status_args).output().unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["data"]["source_validation"], "valid");
    let overall = Command::new(&installed)
        .args([
            "--project-root",
            root_arg.as_str(),
            "--verbose",
            "task",
            "status",
            "--requirement-id",
            "example",
            "--plan-path",
            "outputs/work/plans/example.json",
            "--user-config-root",
            root_arg.as_str(),
        ])
        .output()
        .unwrap();
    assert!(overall.status.success());
    let overall: Value = serde_json::from_slice(&overall.stdout).unwrap();
    assert_eq!(overall["data"]["source_validation"], "valid");
    let plan_file = root.join("outputs/work/plans/example.json");
    let mut plan: Value = serde_json::from_slice(&fs::read(&plan_file).unwrap()).unwrap();
    plan["summary"] = json!("Changed after planning");
    fs::write(
        &plan_file,
        work_infrastructure::fixture_support::render_plan(&plan).unwrap(),
    )
    .unwrap();
    let drift = Command::new(&installed).args(status_args).output().unwrap();
    assert!(
        drift.status.success(),
        "{}",
        String::from_utf8_lossy(&drift.stdout)
    );
    let drift: Value = serde_json::from_slice(&drift.stdout).unwrap();
    assert_eq!(drift["data"]["source_validation"], "review_required");
    let draft: Value = serde_json::from_slice(
        &fs::read(root.join("outputs/work/tasks/example/drafts/history/3/TASK-001.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(draft["notes"], json!(["Concrete discussion"]));
    assert_eq!(draft["revision"], 2);
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
    assert_eq!(count, 92);
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
    assert_eq!(commands.len(), 92);
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.starts_with("task "))
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "task apply",
            "task prepare",
            "task preview",
            "task recover",
            "task save",
            "task status",
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
        "task prepare",
        "task status",
        "task save",
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
    ] {
        assert!(commands.contains(&command.to_owned()));
    }
    for removed in [
        "task spec-prepare",
        "task spec-validate",
        "task spec-update",
        "task spec-verify",
        "task spec-recover",
        "task reconciliation-prepare",
        "task reconciliation-preview",
        "task reconciliation-apply",
        "migration semantic-prepare",
        "migration semantic-preview",
        "migration semantic-apply",
        "migration semantic-recover",
    ] {
        assert!(!commands.contains(&removed.to_owned()));
    }
    let legacy = commands
        .into_iter()
        .filter(|command| !migration.contains(&command.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(legacy.len(), 86);
    let raw = format!("{}\n", legacy.join("\n"));
    assert_eq!(
        work_infrastructure::fixture_support::raw_sha256(raw.as_bytes()),
        "dd8a8d20bf81d7df11ad974ed402bf647558704d862fdaf029d2f540e215696d"
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
    assert_eq!(response["data"]["schema"], "work-spec-migration-preview/v1");

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
            "work-spec-migration-preview-request/v1"
        );
        assert_eq!(prepared["preview"]["status"], "ready");
        let preview = invoke("preview", &prepared_arg, None);
        assert_eq!(preview, prepared["preview"]);
        let approved = preview["fingerprint"].as_str().unwrap();
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
        assert_eq!(verified["schema"], "work-spec-migration-verification/v1");
        assert_eq!(verified["status"], "valid");
        assert_eq!(verified["mode"], "semantic");
        assert_eq!(fs::read(&journal_path).unwrap(), journal_before);
        assert_eq!(fs::read(&plan_path).unwrap(), plan_before);
        let recovered = invoke("recover", &prepared_arg, Some(approved));
        assert_eq!(recovered["publication_status"], "already_published");
        fs::write(&plan_path, b"damaged\n").unwrap();
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
        assert_eq!(fs::read(&plan_path).unwrap(), b"damaged\n");
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
    custom_index["source_plan"]["canonical_sha256"] = json!(
        work_infrastructure::fixture_support::raw_sha256(&custom_plan_raw)
    );
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
    use work_infrastructure::fixture_support::{
        build_initial_execution_index, raw_sha256, render_execution_index, render_plan,
        render_task_index,
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
        index["source_plan"]["canonical_sha256"] = json!(raw_sha256(&plan_raw));
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
            json!(["/plan/constraints", "/task_index/source_plan"])
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
                serde_json::from_slice(&fs::read(&plan_path).unwrap()).unwrap();
            changed["summary"] = json!("Unexpected drift after publication.");
            fs::write(&plan_path, render_plan(&changed).unwrap()).unwrap();
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
                    .join("outputs/work/progress/example/plan/progress.json")
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
