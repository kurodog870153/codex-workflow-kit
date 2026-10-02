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
    text.split("#[cfg(test)]\nmod ")
        .next()
        .expect("source has a prefix")
}

fn contains_identifier(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        let before = text.as_bytes()[..at].last();
        let after = text.as_bytes()[at + name.len()..].first();
        let identifier_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
        !before.is_some_and(|byte| identifier_byte(*byte))
            && !after.is_some_and(|byte| identifier_byte(*byte))
    })
}

fn imports_low_level_hash(production: &str) -> bool {
    production
        .match_indices("use work_operations::")
        .any(|(at, _)| {
            if production[..at].trim_end().ends_with("#[cfg(test)]") {
                return false;
            }
            let statement = production[at..].split(';').next().unwrap_or("");
            let compact: String = statement.chars().filter(|ch| !ch.is_whitespace()).collect();
            let module_imports = [
                (
                    "canonical::",
                    &[
                        "sha256_hex",
                        "canonical_sha256",
                        "canonical_json_sha256",
                        "instructions_sha256",
                    ][..],
                ),
                ("task::collection::", &["collection_fingerprint_sha256"][..]),
                ("hierarchy::", &["selection_sha256"][..]),
            ];
            module_imports.iter().any(|(module, names)| {
                compact.contains(module)
                    && names
                        .iter()
                        .any(|name| contains_identifier(statement, name))
            })
        })
}

fn direct_contract_hash(relative: &str, text: &str) -> bool {
    let production = production_source(text);
    let forbidden_hash = [
        "sha256_hex(",
        "canonical_sha256(",
        "canonical_json_sha256(",
        "collection_fingerprint_sha256(",
        "instructions_sha256(",
    ]
    .iter()
    .any(|pattern| production.contains(pattern));
    let bare_hierarchy_call = production.match_indices("selection_sha256(").any(|(at, _)| {
        let is_identifier_suffix = at > 0
            && matches!(production.as_bytes()[at - 1], b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9');
        let allowed_fixture_definition = relative == "work-infrastructure/src/fixture_support.rs"
            && production[..at].ends_with("pub fn ");
        !is_identifier_suffix && !allowed_fixture_definition
    });
    forbidden_hash
        || production.contains("work_operations::hierarchy::selection_sha256")
        || bare_hierarchy_call
        || imports_low_level_hash(production)
}

fn has_free_call(production: &str, name: &str) -> bool {
    production
        .match_indices(&format!("{name}("))
        .any(|(at, _)| {
            at == 0
                || !matches!(
                    production.as_bytes()[at - 1],
                    b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.'
                )
        })
}

fn caller_local_transaction_chain(relative: &str, text: &str) -> bool {
    let production = production_source(text);
    if [
        "fn derived_transaction_id(",
        "fn encode_snapshot(",
        "fn approval_sha256(",
        "encode_snapshot(",
    ]
    .iter()
    .any(|pattern| production.contains(pattern))
    {
        return true;
    }
    let checked = if relative == "work-infrastructure/src/specification/workflow_storage.rs"
        && production.contains("use work_operations::derivation::identity::derived_transaction_id;")
    {
        production.replacen(
            "let id = derived_transaction_id(\"UPDATE\", approval)",
            "let id = shared_transaction_id(\"UPDATE\", approval)",
            1,
        )
    } else if relative == "work-feature/src/progress.rs"
        && production.contains("use work_operations::progress::{")
        && production
            .split("use work_operations::progress::{")
            .nth(1)
            .and_then(|imports| imports.split(';').next())
            .is_some_and(|imports| contains_identifier(imports, "approval_sha256"))
    {
        production.replacen(
            "let approved = approval_sha256(",
            "let approved = shared_progress_approval(",
            1,
        )
    } else {
        production.to_owned()
    };
    has_free_call(&checked, "derived_transaction_id") || has_free_call(&checked, "approval_sha256")
}

fn caller_local_propagation(text: &str) -> bool {
    let production = production_source(text);
    [
        "fn propagate_artifact(",
        "fn propagate_task_collection(",
        "fn sync_task_collection(",
        "fn update_task_collection_sha256(",
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
fn feature_and_infrastructure_cannot_bypass_derivation_facade() {
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
        if relative.starts_with("work-feature/src/")
            || relative.starts_with("work-infrastructure/src/")
        {
            assert!(
                !direct_contract_hash(&relative, &text),
                "caller-local contract digest in {relative}"
            );
            assert!(
                !caller_local_transaction_chain(&relative, &text),
                "caller-local transaction chain in {relative}"
            );
            assert!(
                !caller_local_propagation(&text),
                "caller-local artifact propagation in {relative}"
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
            !caller_local_transaction_chain(file, &text),
            "local transaction chain in {file}"
        );
        assert!(!direct_contract_hash(file, &text), "local digest in {file}");
    }
    assert!(
        !crates_root()
            .join("work-infrastructure/src/specification/artifact_reconciliation.rs")
            .exists()
    );
    assert!(
        source("work-infrastructure/src/codec.rs")
            .contains("pub use work_operations::derivation::fingerprint;"),
        "process adapter must expose the derivation facade"
    );
    assert!(!source("work-infrastructure/src/codec.rs").contains("pub fn sha256_hex("));
    assert!(
        !source("work-operations/src/skill.rs").contains("pub fn selection_sha256("),
        "legacy skill selection digest entry must stay removed"
    );
    for (file, functions) in [
        (
            "work-operations/src/canonical.rs",
            &[
                "sha256_hex",
                "canonical_sha256",
                "canonical_json_sha256",
                "instructions_sha256",
            ][..],
        ),
        (
            "work-operations/src/task/collection.rs",
            &["collection_fingerprint_sha256"][..],
        ),
        (
            "work-operations/src/hierarchy.rs",
            &["selection_sha256"][..],
        ),
    ] {
        let text = source(file);
        for function in functions {
            assert!(
                text.contains(&format!("pub(crate) fn {function}(")),
                "{function} must stay crate-private in {file}"
            );
        }
    }
}

#[test]
fn guard_detects_forbidden_production_fixtures() {
    assert!(duplicated_marker_algorithm(
        "work-infrastructure/src/transaction_storage.rs",
        "fn completion_marker(raw: &[u8]) -> Vec<u8> { raw.to_vec() }"
    ));
    for pattern in [
        "sha256_hex(raw)",
        "canonical_sha256(raw)",
        "canonical_json_sha256(value)",
        "collection_fingerprint_sha256(index, items)",
        "instructions_sha256(mode, sources)",
        "work_operations::hierarchy::selection_sha256(decision, paths, entries, catalog)",
        "selection_sha256(decision, paths, entries, catalog)",
    ] {
        assert!(
            direct_contract_hash(
                "work-feature/src/local.rs",
                &format!("fn local() {{ {pattern}; }}")
            ),
            "{pattern}"
        );
    }
    assert!(direct_contract_hash(
        "work-feature/src/local.rs",
        "use work_operations::hierarchy::{selection_sha256 as local}; fn local() { local(); }"
    ));
    for import in [
        "use work_operations::canonical::sha256_hex as digest;",
        "use work_operations::canonical::{canonical_sha256 as digest};",
        "use work_operations::{canonical::instructions_sha256 as digest};",
        "use work_operations::task::collection::collection_fingerprint_sha256 as digest;",
        "use work_operations::hierarchy::selection_sha256 as digest;",
    ] {
        assert!(
            direct_contract_hash(
                "work-feature/src/local.rs",
                &format!("{import} fn local() {{ digest(raw); }}")
            ),
            "{import}"
        );
    }
    assert!(!direct_contract_hash(
        "work-infrastructure/src/fixture_support.rs",
        "pub fn selection_sha256() {}"
    ));
    assert!(direct_contract_hash(
        "work-infrastructure/src/fixture_support.rs",
        "pub fn selection_sha256() { selection_sha256(raw); }"
    ));
    assert!(caller_local_transaction_chain(
        "work-feature/src/local.rs",
        "fn local(files: &Value, meta: &Value) { approval_sha256(files, meta); }"
    ));
    assert!(caller_local_transaction_chain(
        "work-infrastructure/src/local.rs",
        "fn local() { encode_snapshot(raw); derived_transaction_id(kind, approval); }"
    ));
    assert!(caller_local_propagation(
        "fn propagate_task_collection() {}"
    ));
    assert!(caller_local_propagation(
        "fn update_task_collection_sha256() {}"
    ));
    assert!(caller_local_transaction_chain(
        "work-feature/src/progress.rs",
        "let approved = approval_sha256(raw); approval_sha256(other);"
    ));
    assert!(caller_local_transaction_chain(
        "work-feature/src/progress.rs",
        "let approved = approval_sha256(raw);"
    ));
    assert!(caller_local_transaction_chain(
        "work-infrastructure/src/specification/workflow_storage.rs",
        "let id = derived_transaction_id(\"UPDATE\", approval); derived_transaction_id(other, approval);"
    ));
    assert!(!direct_contract_hash(
        "work-feature/src/local.rs",
        "#[cfg(test)]\nmod tests { sha256_hex(raw); }"
    ));
}

#[test]
fn guard_accepts_facade_usage() {
    let facade_usage = "use work_operations::derivation::fingerprint as hashes;
        fn local(raw: &[u8], value: &Value) {
            hashes::raw(raw);
            hashes::structured(value);
            hashes::hierarchy_selection(decision, paths, entries, catalog);
        }";
    assert!(!direct_contract_hash(
        "work-feature/src/local.rs",
        facade_usage
    ));
    assert!(!caller_local_transaction_chain(
        "work-feature/src/local.rs",
        facade_usage
    ));
    assert!(!caller_local_propagation(facade_usage));
}
