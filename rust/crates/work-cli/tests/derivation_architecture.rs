//! Guard the derivation responsibilities already migrated to Operations.

use std::fs;
use std::path::{Path, PathBuf};

fn crates_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn source(relative: &str) -> String {
    fs::read_to_string(crates_root().join(relative)).expect("Rust source exists")
}

fn duplicated_marker_algorithm(relative: &str, text: &str) -> bool {
    relative != "work-operations/src/derivation/publication.rs"
        && (text.contains("fn completion_marker(") || text.contains("pub fn completion_marker("))
}

fn production_source(text: &str) -> &str {
    text.split("#[cfg(test)]\nmod tests")
        .next()
        .expect("source has a prefix")
}

fn direct_contract_hash(text: &str) -> bool {
    let production = production_source(text);
    production.contains("sha256_hex(") || production.contains("canonical_json_sha256(")
}

fn caller_local_transaction_chain(text: &str) -> bool {
    let production = production_source(text);
    [
        "derived_transaction_id(",
        "encode_snapshot(",
        "approval_sha256(",
    ]
    .iter()
    .any(|pattern| production.contains(pattern))
}

fn rust_sources(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("source directory is readable") {
        let path = entry.expect("directory entry is readable").path();
        if path.is_dir() {
            rust_sources(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn migrated_derivation_has_one_marker_and_no_caller_local_transaction_chain() {
    let mut files = Vec::new();
    for crate_name in ["work-operations", "work-feature", "work-infrastructure"] {
        rust_sources(&crates_root().join(crate_name).join("src"), &mut files);
    }
    for path in files {
        let relative = path
            .strip_prefix(crates_root())
            .expect("source is under the workspace")
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&path).expect("Rust source is readable");
        assert!(
            !duplicated_marker_algorithm(&relative, &text),
            "duplicate completion marker in {relative}"
        );
        if relative.starts_with("work-infrastructure/src/")
            && relative != "work-infrastructure/src/codec.rs"
        {
            assert!(
                !direct_contract_hash(&text),
                "caller-local contract digest in {relative}"
            );
        }
    }
    for file in [
        "work-feature/src/instruction/migration_build.rs",
        "work-feature/src/instruction/refresh_build.rs",
    ] {
        let text = source(file);
        assert!(!text.contains("fn hash("), "local hash in {file}");
        assert!(
            !text.contains("task_collection_sha256"),
            "local collection sync in {file}"
        );
    }
    for file in [
        "work-feature/src/instruction/migration_publication.rs",
        "work-feature/src/instruction/refresh_publication.rs",
        "work-feature/src/specification/mod.rs",
        "work-feature/src/specification/migration_transaction.rs",
        "work-feature/src/specification/reconciliation_publication.rs",
    ] {
        let text = source(file);
        assert!(
            !caller_local_transaction_chain(&text),
            "local transaction chain in {file}"
        );
        assert!(!direct_contract_hash(&text), "local digest in {file}");
    }
    assert!(
        !crates_root()
            .join("work-infrastructure/src/specification/artifact_reconciliation.rs")
            .exists()
    );
    assert!(
        source("work-infrastructure/src/codec.rs")
            .contains("work_operations::derivation::fingerprint::raw(raw)"),
        "process adapter must delegate raw digest derivation"
    );
    assert!(
        !source("work-operations/src/skill.rs").contains("pub fn selection_sha256("),
        "legacy skill selection digest entry must stay removed"
    );
}

#[test]
fn guard_detects_duplicate_marker_fixture() {
    assert!(duplicated_marker_algorithm(
        "work-infrastructure/src/transaction_storage.rs",
        "fn completion_marker(raw: &[u8]) -> Vec<u8> { raw.to_vec() }"
    ));
    assert!(direct_contract_hash(
        "fn local(raw: &[u8]) -> String { sha256_hex(raw) }"
    ));
    assert!(caller_local_transaction_chain(
        "fn local(files: &Value, meta: &Value) { approval_sha256(files, meta); }"
    ));
    assert!(!direct_contract_hash(
        "#[cfg(test)]\nmod tests { sha256_hex(raw); }"
    ));
}
