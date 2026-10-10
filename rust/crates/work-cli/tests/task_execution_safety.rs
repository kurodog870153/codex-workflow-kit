//! Public project-file preview/approval/closure and independent DAG acceptance.
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

fn setup() -> (PathBuf, PathBuf) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "work-file-cli-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    let root = root.canonicalize().unwrap();
    let skills = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"))
        .canonicalize()
        .unwrap();
    work_infrastructure::fixture_support::file_transaction_fixture(&root, &skills).unwrap();
    (root, skills)
}
fn invoke(
    root: &Path,
    skills: &Path,
    command: &str,
    task: &str,
    request: Option<Value>,
) -> (i32, Value) {
    let mut args = vec![
        "--project-root".into(),
        root.to_string_lossy().into_owned(),
        "--verbose".into(),
        "execute".into(),
        command.into(),
        "--task-path".into(),
        "outputs/work/tasks/example/index.json".into(),
        "--execution-dir".into(),
        "outputs/work/executions/example".into(),
        "--task-id".into(),
        task.into(),
        "--user-config-root".into(),
        root.to_string_lossy().into_owned(),
    ];
    if let Some(request) = request {
        let input = root.join("outputs/work/transactions/example/file-test/request.json");
        fs::write(&input, serde_json::to_vec(&request).unwrap()).unwrap();
        args.extend(["--input-file".into(), input.to_string_lossy().into_owned()]);
    }
    let (code, result) = work_cli::runtime::run_with_skill_root(&args, skills);
    (code, serde_json::to_value(result).unwrap())
}
fn begin_validation(root: &Path, skills: &Path) {
    let args = vec![
        "--project-root".into(),
        root.to_string_lossy().into_owned(),
        "execute".into(),
        "record-begin".into(),
        "--task-path".into(),
        "outputs/work/tasks/example/index.json".into(),
        "--execution-dir".into(),
        "outputs/work/executions/example".into(),
        "--task-id".into(),
        "TASK-001".into(),
        "--user-config-root".into(),
        root.to_string_lossy().into_owned(),
        "--record-id".into(),
        "VAL-001".into(),
    ];
    let (code, result) = work_cli::runtime::run_with_skill_root(&args, skills);
    assert_eq!(code, 0, "{result:?}");
}
#[test]
fn exact_file_preview_publishes_then_acceptance_enables_downstream_task() {
    let (root, skills) = setup();
    let staged: std::collections::BTreeMap<_, _> = ["b-moved.txt", "c-existing.txt", "d-new.txt"]
        .into_iter()
        .map(|p| {
            (
                p,
                format!("outputs/work/transactions/example/file-test/staging/{p}"),
            )
        })
        .collect();
    let request = json!({"schema":"work-file-transaction-request","command":{"kind":"prepare","attempt_id":"ATTEMPT-001","staged_files":staged}});
    let (code, response) = invoke(
        &root,
        &skills,
        "file-prepare",
        "TASK-001",
        Some(request.clone()),
    );
    assert_eq!(code, 0, "{response}");
    let preview = response["data"].clone();
    assert_eq!(preview["manifest"]["targets"].as_array().unwrap().len(), 4);
    assert!(
        !root
            .join("outputs/work/runtime/staging/example/project-files")
            .exists()
    );
    assert_eq!(
        fs::read(root.join("a-old.txt")).unwrap(),
        b"original move\r\n"
    );
    assert_ne!(invoke(&root, &skills, "preflight", "TASK-002", None).0, 0);
    let mut wrong = json!({"schema":"work-file-transaction-request","command":{"kind":"apply","preview":preview,"approved_sha256":"0".repeat(64),"authorization_evidence":"Approve wrong preview"}});
    assert_ne!(
        invoke(
            &root,
            &skills,
            "file-apply",
            "TASK-001",
            Some(wrong.clone())
        )
        .0,
        0
    );
    assert!(!root.join("d-new.txt").exists());
    wrong["command"]["approved_sha256"] = preview["manifest"]["approval_sha256"].clone();
    let (code, response) = invoke(&root, &skills, "file-apply", "TASK-001", Some(wrong));
    assert_eq!(code, 0, "{response}");
    assert_eq!(response["data"]["phase"], "published-verified");
    assert!(!root.join("a-old.txt").exists());
    assert!(root.join("d-new.txt").exists());
    let close = json!({"schema":"work-attempt-close-request","status":"completed"});
    assert_ne!(
        invoke(
            &root,
            &skills,
            "attempt-close",
            "TASK-001",
            Some(close.clone())
        )
        .0,
        0
    );
    begin_validation(&root, &skills);
    let (code, response) = invoke(
        &root,
        &skills,
        "record-finish",
        "TASK-001",
        Some(
            json!({"schema":"work-record-finish-request","record":{"outcome":"passed","evidence":"User verified all published create/modify/move paths and complete bytes"},"modified_files":["a-old.txt","b-moved.txt","c-existing.txt","d-new.txt"]}),
        ),
    );
    assert_eq!(code, 0, "{response}");
    let (code, response) = invoke(&root, &skills, "attempt-close", "TASK-001", Some(close));
    assert_eq!(code, 0, "{response}");
    assert_eq!(response["data"]["task_status"], "completed");
    let (code, response) = invoke(&root, &skills, "preflight", "TASK-002", None);
    assert_eq!(code, 0, "{response}");
    assert_eq!(response["data"]["dependencies"], json!(["TASK-001"]));
}

#[test]
fn changed_staging_invalidates_approval_without_any_target_publication() {
    let (root, skills) = setup();
    let staged: std::collections::BTreeMap<_, _> = ["b-moved.txt", "c-existing.txt", "d-new.txt"]
        .into_iter()
        .map(|p| {
            (
                p,
                format!("outputs/work/transactions/example/file-test/staging/{p}"),
            )
        })
        .collect();
    let (code, response) = invoke(
        &root,
        &skills,
        "file-prepare",
        "TASK-001",
        Some(
            json!({"schema":"work-file-transaction-request","command":{"kind":"prepare","attempt_id":"ATTEMPT-001","staged_files":staged}}),
        ),
    );
    assert_eq!(code, 0, "{response}");
    let preview = response["data"].clone();
    fs::write(
        root.join("outputs/work/transactions/example/file-test/staging/d-new.txt"),
        b"changed",
    )
    .unwrap();
    let (code, response) = invoke(
        &root,
        &skills,
        "file-apply",
        "TASK-001",
        Some(
            json!({"schema":"work-file-transaction-request","command":{"kind":"apply","approved_sha256":preview["manifest"]["approval_sha256"],"preview":preview,"authorization_evidence":"Old approval"}}),
        ),
    );
    assert_ne!(code, 0);
    assert_eq!(response["reason_code"], "file_transaction_approval_stale");
    assert_eq!(
        fs::read(root.join("c-existing.txt")).unwrap(),
        b"original modify\0bytes\n"
    );
    assert!(!root.join("d-new.txt").exists());
}

#[test]
fn interrupted_publication_blocks_closure_and_downstream_until_fresh_authorized_restore() {
    let (root, skills) = setup();
    let target = work_flow::execution::ExecutionProjectTarget {
        task_path: "outputs/work/tasks/example/index.json",
        execution_dir: "outputs/work/executions/example",
        task_id: "TASK-001",
    };
    let context =
        work_cli::runtime::prepare_execution_writer_context(&root, &skills, &[], target).unwrap();
    let identity = work_infrastructure::fixture_support::interrupt_file_transaction_fixture(
        &context, &skills, 2,
    )
    .unwrap();
    assert_ne!(
        invoke(
            &root,
            &skills,
            "attempt-close",
            "TASK-001",
            Some(json!({"schema":"work-attempt-close-request","status":"completed"}))
        )
        .0,
        0
    );
    assert_ne!(invoke(&root, &skills, "preflight", "TASK-002", None).0, 0);
    let request = json!({"schema":"work-file-transaction-request","command":{"kind":"recovery-prepare","transaction_identity":identity}});
    let (code, response) = invoke(
        &root,
        &skills,
        "file-recovery-prepare",
        "TASK-001",
        Some(request),
    );
    assert_eq!(code, 0, "{response}");
    let recovery = response["data"].clone();
    let mut restore = json!({"schema":"work-file-transaction-request","command":{"kind":"restore","transaction_identity":identity,"approved_sha256":recovery["approval_sha256"],"authorization_evidence":""}});
    assert_ne!(
        invoke(
            &root,
            &skills,
            "file-restore",
            "TASK-001",
            Some(restore.clone())
        )
        .0,
        0
    );
    restore["command"]["authorization_evidence"] =
        json!("User freshly approved restore of this exact before/actual set");
    let (code, response) = invoke(&root, &skills, "file-restore", "TASK-001", Some(restore));
    assert_eq!(code, 0, "{response}");
    assert_eq!(response["data"]["phase"], "restored");
    assert_eq!(
        fs::read(root.join("a-old.txt")).unwrap(),
        b"original move\r\n"
    );
    assert_eq!(
        fs::read(root.join("c-existing.txt")).unwrap(),
        b"original modify\0bytes\n"
    );
    assert!(!root.join("b-moved.txt").exists());
    assert!(!root.join("d-new.txt").exists());
    assert_ne!(invoke(&root, &skills, "preflight", "TASK-002", None).0, 0);
    begin_validation(&root, &skills);
    let (code, response) = invoke(
        &root,
        &skills,
        "record-finish",
        "TASK-001",
        Some(
            json!({"schema":"work-record-finish-request","record":{"outcome":"passed","evidence":"Manual record cannot substitute for missing published outcome"}}),
        ),
    );
    assert_eq!(code, 0, "{response}");
    let (code, response) = invoke(
        &root,
        &skills,
        "attempt-close",
        "TASK-001",
        Some(json!({"schema":"work-attempt-close-request","status":"completed"})),
    );
    assert_ne!(code, 0);
    assert_eq!(response["reason_code"], "file_transaction_restored_attempt");
    let (code, response) = invoke(
        &root,
        &skills,
        "attempt-close",
        "TASK-001",
        Some(
            json!({"schema":"work-attempt-close-request","status":"stopped","final_type":"other","reason":"Publication restored; authorize fresh execution","authorization_evidence":"Approve closing this restored attempt"}),
        ),
    );
    assert_eq!(code, 0, "{response}");
    let choice = json!({"command_positions":[],"validation_positions":[1],"modifiable_files":["a-old.txt","b-moved.txt","c-existing.txt","d-new.txt"],"external_operation_positions":[],"allowed_deviations":[],"authorization_evidence":"Fresh approval to retry restored publication"});
    let (code, response) = invoke(
        &root,
        &skills,
        "attempt-start-prepare",
        "TASK-001",
        Some(choice),
    );
    assert_eq!(code, 0, "{response}");
    let (code, response) = invoke(
        &root,
        &skills,
        "attempt-start",
        "TASK-001",
        Some(response["data"]["request"].clone()),
    );
    assert_eq!(code, 0, "{response}");
    let staged: std::collections::BTreeMap<_, _> = ["b-moved.txt", "c-existing.txt", "d-new.txt"]
        .into_iter()
        .map(|p| {
            (
                p,
                format!("outputs/work/transactions/example/file-test/staging/{p}"),
            )
        })
        .collect();
    let (code, response) = invoke(
        &root,
        &skills,
        "file-prepare",
        "TASK-001",
        Some(
            json!({"schema":"work-file-transaction-request","command":{"kind":"prepare","attempt_id":"ATTEMPT-002","staged_files":staged}}),
        ),
    );
    assert_eq!(code, 0, "{response}");
    let preview = response["data"].clone();
    let (code, response) = invoke(
        &root,
        &skills,
        "file-apply",
        "TASK-001",
        Some(
            json!({"schema":"work-file-transaction-request","command":{"kind":"apply","approved_sha256":preview["manifest"]["approval_sha256"],"preview":preview,"authorization_evidence":"Approve complete retry publication"}}),
        ),
    );
    assert_eq!(code, 0, "{response}");
    begin_validation(&root, &skills);
    let (code, response) = invoke(
        &root,
        &skills,
        "record-finish",
        "TASK-001",
        Some(
            json!({"schema":"work-record-finish-request","record":{"outcome":"passed","evidence":"Fresh verification of retry publication"},"modified_files":["a-old.txt","b-moved.txt","c-existing.txt","d-new.txt"]}),
        ),
    );
    assert_eq!(code, 0, "{response}");
    let (code, response) = invoke(
        &root,
        &skills,
        "attempt-close",
        "TASK-001",
        Some(json!({"schema":"work-attempt-close-request","status":"completed"})),
    );
    assert_eq!(code, 0, "{response}");
    assert_eq!(invoke(&root, &skills, "preflight", "TASK-002", None).0, 0);
}
