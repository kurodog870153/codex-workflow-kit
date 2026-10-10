//! Public project-file command records.
use crate::contract::ContractRecord;
use serde_json::json;
use std::collections::BTreeMap;

pub fn records() -> BTreeMap<String, ContractRecord> {
    let manifest = json!({"schema":"work-runtime-transaction","canonical_root":"/project","requirement_id":"example",
        "execution_dir":"outputs/work/executions/example","operation":"project-files","transaction_identity":"a".repeat(64),
        "approval_sha256":"b".repeat(64),"business_identity":{"task_id":"TASK-001","attempt_id":"ATTEMPT-001",
            "source_sha256":{},"staged_files":{},"before_metadata":{"result.txt":null},"after_metadata":{"result.txt":{"readonly":false,"unix_mode":null}}},
        "targets":[{"path":"result.txt","before":null,"after":{"bytes":[10],"sha256":"c".repeat(64)}}],
        "inventory":[{"path":"candidate.json","sha256":"d".repeat(64),"size_bytes":1}],"published_count":0,"phase":"prepared"});
    let mut result = BTreeMap::new();
    for (id, kind, constructible, example, order) in [
        (
            "work-file-transaction-request",
            "semantic_request",
            true,
            json!({"schema":"work-file-transaction-request","command":{"kind":"prepare","attempt_id":"ATTEMPT-001","staged_files":{"result.txt":"outputs/work/transactions/example/task/staging/result.txt"}}}),
            vec!["schema", "command"],
        ),
        (
            "work-file-transaction-preview",
            "response",
            false,
            json!({"schema":"work-file-transaction-preview","manifest":manifest}),
            vec!["schema", "manifest"],
        ),
        (
            "work-file-recovery-preview",
            "response",
            false,
            json!({"schema":"work-file-recovery-preview","manifest":manifest,"observed":{"result.txt":null},"approval_sha256":"e".repeat(64)}),
            vec!["schema", "manifest", "observed", "approval_sha256"],
        ),
        (
            "work-file-transaction-result",
            "response",
            false,
            json!({"schema":"work-file-transaction-result","transaction_identity":"a".repeat(64),"phase":"published-verified"}),
            vec!["schema", "transaction_identity", "phase"],
        ),
    ] {
        let fields:Vec<_>=order.iter().map(|name|json!({"name":name,"type":if *name=="manifest" || *name=="command" || *name=="observed" {"object"} else {"string"},"required":true,"reference":null,
            "constraints":if *name=="command" {json!({"discriminator":"kind","unknown_fields":"reject","variants":{
                "prepare":{"required":["kind","attempt_id","staged_files"]},
                "apply":{"required":["kind","preview","approved_sha256","authorization_evidence"]},
                "recovery-prepare":{"required":["kind","transaction_identity"]},
                "restore":{"required":["kind","transaction_identity","approved_sha256","authorization_evidence"]}}})} else {json!({})}})).collect();
        let description = json!({"schema":"work-contract-description","id":id,"kind":kind,"caller_constructible":constructible,
            "required":order,"optional":[],"canonical_order":order,"fields":fields,"example":example});
        let record = if constructible {
            json!({"description":description,"scaffold":{"schema":"work-contract-scaffold","id":id,"canonical_order":order,"scaffold":example,"example":example}})
        } else {
            json!({"description":description,"scaffold_error":{"reason_code":"contract_scaffold_requires_request","message":"Only semantic requests are caller constructible.","details":{"contract_id":id,"kind":kind}}})
        };
        result.insert(
            id.into(),
            serde_json::from_value(record).expect("file transaction contract matches model"),
        );
    }
    result
}
