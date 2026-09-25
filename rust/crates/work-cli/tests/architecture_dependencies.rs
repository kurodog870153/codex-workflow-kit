//! Enforce the six Work crate edges for normal, build, and dev dependencies.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde_json::Value;

const CRATES: [&str; 6] = [
    "work-cli",
    "work-feature",
    "work-flow",
    "work-infrastructure",
    "work-model",
    "work-operations",
];

fn allowed() -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    BTreeMap::from([
        ("work-model", BTreeSet::new()),
        ("work-operations", BTreeSet::from(["work-model"])),
        (
            "work-feature",
            BTreeSet::from(["work-model", "work-operations"]),
        ),
        ("work-flow", BTreeSet::from(["work-feature", "work-model"])),
        (
            "work-infrastructure",
            BTreeSet::from(["work-feature", "work-model", "work-operations"]),
        ),
        (
            "work-cli",
            BTreeSet::from(["work-flow", "work-infrastructure", "work-model"]),
        ),
    ])
}

fn check_edge(source: &str, destination: &str, kind: &str) -> Result<(), String> {
    if !["normal", "build", "dev"].contains(&kind) {
        return Err(format!("unknown dependency kind: {kind}"));
    }
    let table = allowed();
    let Some(destinations) = table.get(source) else {
        return Err(format!("unknown Work source: {source}"));
    };
    if destinations.contains(destination) {
        Ok(())
    } else {
        Err(format!(
            "forbidden {kind} Work edge: {source} -> {destination}"
        ))
    }
}

#[test]
fn workspace_has_only_six_work_crates_and_allowed_direct_edges() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let output = Command::new(env!("CARGO"))
        .env("RUSTUP_TOOLCHAIN", "1.85.0")
        .args([
            "metadata",
            "--locked",
            "--offline",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .expect("cargo metadata starts");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Value = serde_json::from_slice(&output.stdout).unwrap();
    let members: BTreeSet<_> = metadata["workspace_members"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let packages = metadata["packages"].as_array().unwrap();
    let work_packages: Vec<_> = packages
        .iter()
        .filter(|package| {
            package["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("work-"))
        })
        .collect();
    let actual: BTreeSet<_> = work_packages
        .iter()
        .filter(|package| members.contains(package["id"].as_str().unwrap()))
        .map(|package| package["name"].as_str().unwrap())
        .collect();
    assert_eq!(actual, BTreeSet::from(CRATES));
    for package in work_packages {
        let source = package["name"].as_str().unwrap();
        for dependency in package["dependencies"].as_array().unwrap() {
            let destination = dependency["name"].as_str().unwrap();
            if !destination.starts_with("work-") {
                continue;
            }
            let kind = dependency["kind"].as_str().unwrap_or("normal");
            check_edge(source, destination, kind).unwrap();
        }
    }
}

#[test]
fn every_forbidden_direction_is_rejected_for_all_dependency_kinds() {
    let table = allowed();
    for source in CRATES {
        for destination in CRATES {
            for kind in ["normal", "build", "dev"] {
                let result = check_edge(source, destination, kind);
                assert_eq!(
                    result.is_ok(),
                    table[source].contains(destination),
                    "{kind} {source} -> {destination}"
                );
            }
        }
    }
    assert!(check_edge("work-cli", "work-unlisted", "normal").is_err());
}
