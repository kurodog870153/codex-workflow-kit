//! Routed source fingerprints and same-session drift checks.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};
use work_feature::workflow::WorkflowRoutingRepository;
use work_operations::canonical::{canonical_sha256, sha256_hex};
use work_operations::routing::{RoutingRequest, select};

pub struct RoutingSourceSession {
    root: PathBuf,
    sources: BTreeMap<String, (String, String, String)>,
}

impl RoutingSourceSession {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            sources: BTreeMap::new(),
        }
    }

    fn read(&self, logical_name: &str, relative: &str) -> Result<Vec<u8>, WorkError> {
        fs::read(self.root.join(relative)).map_err(|_| {
            WorkError::new(
                ExitCode::ArtifactIntegrity,
                "routed_instruction_source_missing",
                "A routed instruction source is missing or unreadable.",
                json!({"logical_name":logical_name,"path":relative}),
            )
        })
    }

    pub fn fingerprint(&mut self, logical_name: &str, relative: &str) -> Result<String, WorkError> {
        if let Some((stored_path, _, canonical)) = self.sources.get(logical_name) {
            if stored_path == relative {
                return Ok(canonical.clone());
            }
        }
        let raw = self.read(logical_name, relative)?;
        let canonical = canonical_sha256(&raw).map_err(|_| {
            WorkError::new(
                ExitCode::InputFormat,
                "invalid_utf8",
                "The routed instruction source is not valid UTF-8.",
                json!({"logical_name":logical_name,"path":relative}),
            )
        })?;
        self.sources.insert(
            logical_name.into(),
            (relative.into(), sha256_hex(&raw), canonical.clone()),
        );
        Ok(canonical)
    }

    pub fn recheck(&self) -> Result<(), WorkError> {
        for (logical_name, (relative, expected_raw, _)) in &self.sources {
            if sha256_hex(&self.read(logical_name, relative)?) != *expected_raw {
                return Err(WorkError::new(
                    ExitCode::ArtifactIntegrity,
                    "routed_instruction_source_changed",
                    "A routed instruction source changed during routing construction.",
                    json!({"logical_name":logical_name}),
                ));
            }
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub fn build_routing_selection(
    root: &Path,
    request: &RoutingRequest<'_>,
    session: Option<&mut RoutingSourceSession>,
) -> Result<Value, WorkError> {
    let mut local = RoutingSourceSession::new(root.to_path_buf());
    let session = session.unwrap_or(&mut local);
    if session.root() != root {
        return Err(WorkError::new(
            ExitCode::Contract,
            "routed_instruction_root_mismatch",
            "The routing source session belongs to another skill root.",
            json!({}),
        ));
    }
    let result = select(request, |logical_name, relative| {
        session.fingerprint(logical_name, relative)
    })?;
    session.recheck()?;
    Ok(result)
}

impl WorkflowRoutingRepository for RoutingSourceSession {
    fn route(&mut self, request: &RoutingRequest<'_>) -> Result<Value, WorkError> {
        let root = self.root.clone();
        build_routing_selection(&root, request, Some(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use work_feature::workflow::{
        OperationContextRequest, build_operation_context, execution_state, pre_execution_state,
        validate_operation_context,
    };

    #[test]
    fn instruction_migration_manifest_matches_python_example() {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        let mut session = RoutingSourceSession::new(root);
        let manifest = work_feature::instruction::migration_manifest(
            &mut session,
            "plan",
            "plan_confirmed",
            "prepare_plan",
            &json!({"schema":"work-plan/v1","requirement_id":"example"}),
        )
        .unwrap();
        assert_eq!(
            manifest["selection_sha256"],
            "773a632a0498b1f062d31b6bdf6e9cebd92087af44ccf26392d48a4edcc63bae"
        );
    }

    #[test]
    fn source_session_matches_python_identity_and_rejects_drift() {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        let mut session = RoutingSourceSession::new(root.clone());
        let request = RoutingRequest {
            status: "plan_required",
            operation: "prepare_plan",
            confirmation: true,
            mode: None,
            artifact_lifecycle: "missing",
            formal_events: &[],
            role: "main",
            authorization_state: None,
            verified_state_sha256: "",
        };
        let first = build_routing_selection(&root, &request, Some(&mut session)).unwrap();
        let second = build_routing_selection(&root, &request, Some(&mut session)).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first["selection_sha256"],
            "5508e636076b9bbbac126d4ad46106d89cc1ecccf944daf2ddedcc6ea3115c79"
        );

        let temp = std::env::temp_dir().join(format!("work-routing-{}", std::process::id()));
        fs::create_dir(&temp).unwrap();
        let mut absent = RoutingSourceSession::new(temp.clone());
        assert_eq!(
            absent
                .fingerprint("missing", "missing.md")
                .unwrap_err()
                .reason_code,
            "routed_instruction_source_missing"
        );
        let source = temp.join("source.md");
        fs::write(&source, b"first\n").unwrap();
        let mut changed = RoutingSourceSession::new(temp.clone());
        changed.fingerprint("source", "source.md").unwrap();
        fs::write(&source, b"second\n").unwrap();
        assert_eq!(
            changed.recheck().unwrap_err().reason_code,
            "routed_instruction_source_changed"
        );
        fs::remove_file(source).unwrap();
        fs::remove_dir(temp).unwrap();
    }

    #[test]
    fn plan_required_workflow_state_matches_python_selection() {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        let mut session = RoutingSourceSession::new(root);
        let artifacts = json!({"plan":"outputs/work/plans/demo.json","task":"outputs/work/tasks/demo/index.json","execution":"outputs/work/executions/demo"});
        let state = pre_execution_state(&mut session, "demo", &artifacts, None, None, None, false)
            .unwrap()
            .unwrap();
        assert_eq!(
            state["selection_sha256"],
            "0f2047bad1bac4989f150d603e81f19dacb546c42bc5e36beffc1ef92c4a2007"
        );
        assert_eq!(
            state["selection_manifest"]["routing_input"]["verified_state_sha256"],
            "cc6c8b1987d864f7a3706e0c22a066d249917e4541717b4eab6b4887fc023223"
        );
        assert_eq!(state["command"], "plan semantic-prepare");
        assert_eq!(
            state["request_contract_id"],
            "work-plan-semantic-request/v1"
        );
    }

    #[test]
    fn progress_read_operation_context_matches_python_hashes() {
        let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../skills/work"));
        let mut session = RoutingSourceSession::new(root);
        let artifacts = json!({});
        let (envelope, selection) = build_operation_context(
            &mut session,
            &OperationContextRequest {
                command: "progress",
                operation: "read",
                delegated_role: None,
                artifacts: &artifacts,
                project_root: "/project",
                approval_sha256: None,
                transaction_workspace: None,
            },
        )
        .unwrap();
        assert_eq!(
            envelope["verified_state_sha256"],
            "73acc1a5a33836a1f562c2edf6f9bd73d8ec1c50c2c15dba6d5b5cee82288ba8"
        );
        assert_eq!(
            envelope["selection_sha256"],
            "c25076515480d4a225d4f3109fd6f4854f147659766e8fc48b7e158b0becd965"
        );
        assert_eq!(
            envelope["context_sha256"],
            "d84573b287cb98784097e75984a28daf116a55768ef3c2f56e98b7610d4e0aa0"
        );
        validate_operation_context(&envelope, &selection, &artifacts).unwrap();
        let mut drifted = envelope.clone();
        drifted["role"] = json!("worker");
        assert_eq!(
            validate_operation_context(&drifted, &selection, &artifacts)
                .unwrap_err()
                .reason_code,
            "operation_context_identity_mismatch"
        );
    }

    #[test]
    fn execution_workflow_states_match_python_transitions() {
        let repo = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        let fixture = repo.join("crates/work-infrastructure/fixtures/workflow-execution");
        for name in [
            "pending",
            "in-progress",
            "retry",
            "completed",
            "invalid-lock",
            "reconciliation",
            "reconciled",
        ] {
            let input: Value = serde_json::from_slice(
                &fs::read(fixture.join(format!("{name}-input.json"))).unwrap(),
            )
            .unwrap();
            let expected: Value = serde_json::from_slice(
                &fs::read(fixture.join(format!("{name}-expected.json"))).unwrap(),
            )
            .unwrap();
            let mut session = RoutingSourceSession::new(repo.join("../skills/work"));
            let actual = execution_state(
                &mut session,
                input["requirement_id"].as_str().unwrap(),
                &input["artifacts"],
                &input["execution_index"],
                &input["latest_attempts"],
            )
            .unwrap();
            assert_eq!(actual, expected, "workflow state {name}");
        }
    }
}
