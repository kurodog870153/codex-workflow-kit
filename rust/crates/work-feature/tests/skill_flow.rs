//! Public skill selection flow over memory-only snapshot repositories.

use serde_json::{Value, json};
use std::cell::Cell;
use work_feature::error::WorkError;
use work_feature::skill::{
    SkillRoot, SkillSnapshotRepository, build_selection, validate_selection,
};
use work_operations::skill::selection_sha256;

struct Empty;
impl SkillSnapshotRepository for Empty {
    fn snapshot(&self, _: &str, _: &str, _: &str) -> Result<Value, WorkError> {
        unreachable!()
    }
}

#[test]
fn base_only_round_trip_and_mismatch() {
    let selection =
        build_selection(&Empty, &[], &json!({"decision": "base_only", "skills": []})).unwrap();
    assert_eq!(
        validate_selection(&Empty, &[], &selection).unwrap()["status"],
        "valid"
    );
    assert_eq!(
        build_selection(
            &Empty,
            &[],
            &json!({"decision": "external_skills", "skills": []})
        )
        .unwrap_err()
        .reason_code,
        "skill_selection_decision_mismatch"
    );
}

struct OneSkill;
impl SkillSnapshotRepository for OneSkill {
    fn snapshot(&self, _: &str, _: &str, _: &str) -> Result<Value, WorkError> {
        Ok(json!({"skill": {
            "id": "a".repeat(64), "name": "frontend", "scope": "repo", "root": "skills", "source": "frontend/SKILL.md",
            "description": "Build frontends.", "work_modes": ["plan", "task", "execute"],
            "allow_implicit_invocation": true, "summary_sha256": "b".repeat(64)
        }, "bundle": {"bundle_sha256": "c".repeat(64)}}))
    }
}

struct ChangedSkill;
impl SkillSnapshotRepository for ChangedSkill {
    fn snapshot(&self, scope: &str, root: &str, source: &str) -> Result<Value, WorkError> {
        let mut snapshot = OneSkill.snapshot(scope, root, source)?;
        snapshot["bundle"]["bundle_sha256"] = json!("d".repeat(64));
        Ok(snapshot)
    }
}

#[test]
fn declared_skill_selection_detects_drift_and_duplicates() {
    let roots = [SkillRoot {
        scope: "repo".into(),
        locator: "skills".into(),
    }];
    let choice = json!({"scope": "repo", "root": "skills", "source": "frontend/SKILL.md", "recommendation_reason": "Needed.", "dependency_status": "available"});
    let request = json!({"decision": "external_skills", "skills": [choice.clone()]});
    let selected = build_selection(&OneSkill, &roots, &request).unwrap();
    assert_eq!(
        validate_selection(&OneSkill, &roots, &selected).unwrap()["status"],
        "valid"
    );
    assert_eq!(selected["skills"][0]["mode_support"]["plan"], "declared");
    let duplicated = json!({"decision": "external_skills", "skills": [choice.clone(), choice]});
    assert_eq!(
        build_selection(&OneSkill, &roots, &duplicated)
            .unwrap_err()
            .reason_code,
        "duplicate_selected_skill"
    );
    let mut duplicate_selection = selected.clone();
    duplicate_selection["skills"] = json!([selected["skills"][0], selected["skills"][0]]);
    duplicate_selection["selection_sha256"] = json!(selection_sha256(
        "external_skills",
        duplicate_selection["skills"].as_array().unwrap(),
    ));
    assert_eq!(
        validate_selection(&OneSkill, &roots, &duplicate_selection)
            .unwrap_err()
            .reason_code,
        "duplicate_selected_skill"
    );
    assert_eq!(
        validate_selection(&ChangedSkill, &roots, &selected)
            .unwrap_err()
            .reason_code,
        "selected_skill_snapshot_mismatch"
    );
    let mut stale = selected;
    stale["skills"][0]["bundle_sha256"] = json!("d".repeat(64));
    assert_eq!(
        validate_selection(&OneSkill, &roots, &stale)
            .unwrap_err()
            .reason_code,
        "selected_skill_snapshot_mismatch"
    );
}

struct DriftingSkill {
    reads: Cell<usize>,
}

impl SkillSnapshotRepository for DriftingSkill {
    fn snapshot(&self, scope: &str, root: &str, source: &str) -> Result<Value, WorkError> {
        let reads = self.reads.get() + 1;
        self.reads.set(reads);
        let mut snapshot = OneSkill.snapshot(scope, root, source)?;
        if reads > 1 {
            snapshot["bundle"]["bundle_sha256"] = json!("d".repeat(64));
        }
        Ok(snapshot)
    }
}

#[test]
fn build_resnapshots_and_rejects_source_drift() {
    let repository = DriftingSkill {
        reads: Cell::new(0),
    };
    let roots = [SkillRoot {
        scope: "repo".into(),
        locator: "skills".into(),
    }];
    let choice = json!({"scope":"repo","root":"skills","source":"frontend/SKILL.md",
        "recommendation_reason":"Needed.","dependency_status":"available"});
    let request = json!({"decision":"external_skills","skills":[choice]});
    assert_eq!(
        build_selection(&repository, &roots, &request)
            .unwrap_err()
            .reason_code,
        "selected_skill_snapshot_mismatch"
    );
    assert_eq!(repository.reads.get(), 2);
}

#[test]
fn build_rejects_malformed_missing_roots_unavailable_and_mode_override() {
    for request in [
        json!({"skills":[]}),
        json!({"decision":"external_skills","skills":[]}),
        json!({"decision":[],"skills":[]}),
        json!({"decision":"base_only","skills":{}}),
        json!({"decision":"base_only","skills":[],"selection_sha256":"invented"}),
    ] {
        assert!(build_selection(&Empty, &[], &request).is_err());
    }
    let roots = [SkillRoot {
        scope: "repo".into(),
        locator: "skills".into(),
    }];
    let choice = json!({"scope":"repo","root":"skills","source":"frontend/SKILL.md",
        "recommendation_reason":"Needed.","dependency_status":"available"});
    let request = json!({"decision":"external_skills","skills":[choice]});
    assert_eq!(
        build_selection(&OneSkill, &[], &request)
            .unwrap_err()
            .reason_code,
        "selected_skill_root_missing"
    );
    let mut unavailable = request.clone();
    unavailable["skills"][0]["dependency_status"] = json!("unavailable");
    assert_eq!(
        build_selection(&OneSkill, &roots, &unavailable)
            .unwrap_err()
            .reason_code,
        "skill_dependencies_unavailable"
    );
    let mut override_modes = request;
    override_modes["skills"][0]["mode_support"] =
        json!({"plan":"inferred","task":"inferred","execute":"unsupported"});
    assert_eq!(
        build_selection(&OneSkill, &roots, &override_modes)
            .unwrap_err()
            .reason_code,
        "declared_skill_modes_mismatch"
    );
}

struct NoDeclaredModes;
impl SkillSnapshotRepository for NoDeclaredModes {
    fn snapshot(&self, scope: &str, root: &str, source: &str) -> Result<Value, WorkError> {
        let mut snapshot = OneSkill.snapshot(scope, root, source)?;
        snapshot["skill"]["work_modes"] = json!([]);
        Ok(snapshot)
    }
}

#[test]
fn inferred_modes_are_required_when_skill_has_no_declared_modes() {
    let roots = [SkillRoot {
        scope: "repo".into(),
        locator: "skills".into(),
    }];
    let choice = json!({"scope":"repo","root":"skills","source":"frontend/SKILL.md",
        "recommendation_reason":"Needed.","dependency_status":"available"});
    let request = json!({"decision":"external_skills","skills":[choice]});
    assert_eq!(
        build_selection(&NoDeclaredModes, &roots, &request)
            .unwrap_err()
            .reason_code,
        "inferred_skill_modes_required"
    );
    let mut confirmed = request;
    confirmed["skills"][0]["mode_support"] =
        json!({"plan":"inferred","task":"inferred","execute":"unsupported"});
    let selected = build_selection(&NoDeclaredModes, &roots, &confirmed).unwrap();
    assert_eq!(
        selected["skills"][0]["mode_support"],
        confirmed["skills"][0]["mode_support"]
    );
}

struct TwoSkills;
impl SkillSnapshotRepository for TwoSkills {
    fn snapshot(&self, scope: &str, root: &str, source: &str) -> Result<Value, WorkError> {
        Ok(json!({"skill": {
            "id": if scope == "repo" {"a".repeat(64)} else {"d".repeat(64)},
            "name": "frontend", "scope": scope, "root": root, "source": source,
            "description": "Build frontends.", "work_modes": ["plan", "task", "execute"],
            "allow_implicit_invocation": true, "summary_sha256": "b".repeat(64)
        }, "bundle": {"bundle_sha256": "c".repeat(64)}}))
    }
}

#[test]
fn build_preserves_choice_order_for_same_named_skills() {
    let roots = [
        SkillRoot {
            scope: "repo".into(),
            locator: "skills".into(),
        },
        SkillRoot {
            scope: "user".into(),
            locator: "personal".into(),
        },
    ];
    let choice = |scope: &str, root: &str| {
        json!({"scope":scope,"root":root,
        "source":"frontend/SKILL.md","recommendation_reason":"Needed.",
        "dependency_status":"available"})
    };
    let user = choice("user", "personal");
    let repo = choice("repo", "skills");
    let request = json!({"decision":"external_skills","skills":[user.clone(),repo.clone()]});
    let reverse = json!({"decision":"external_skills","skills":[repo,user]});
    let selected = build_selection(&TwoSkills, &roots, &request).unwrap();
    let reversed = build_selection(&TwoSkills, &roots, &reverse).unwrap();
    assert_eq!(selected["skills"][0]["scope"], "user");
    assert_eq!(selected["skills"][1]["scope"], "repo");
    assert_ne!(selected["skills"][0]["id"], selected["skills"][1]["id"]);
    assert_ne!(selected["selection_sha256"], reversed["selection_sha256"]);
    assert_eq!(
        validate_selection(&TwoSkills, &roots, &selected).unwrap()["status"],
        "valid"
    );
}
