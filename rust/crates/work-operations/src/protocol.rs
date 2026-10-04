//! Shared public Work protocol values.

pub const INVALID_SHA256_ERROR_CODE: &str = "invalid_sha256";
pub const SHA256_HEX_LENGTH: usize = 64;
pub const TASK_ID_PREFIX: &str = "TASK-";
pub const ATTEMPT_ID_PREFIX: &str = "ATTEMPT-";
pub const INVOCATION_MODES: [&str; 4] = ["task", "revise", "migration", "execute"];
pub const WORKFLOW_MODES: [&str; 2] = ["task", "execute"];
pub const PLANNING_STATUSES: [&str; 4] = ["planned", "in_progress", "refined", "needs_review"];
pub const BLOCKING_STOPPED_TYPES: [&str; 3] = [
    "external_operation_failed",
    "instructions_changed",
    "specification_defect",
];

pub fn valid_sha256(value: &str) -> bool {
    value.len() == SHA256_HEX_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_values_and_sha256_validation_match_current_contract_protocol() {
        assert_eq!(INVALID_SHA256_ERROR_CODE, "invalid_sha256");
        assert_eq!(TASK_ID_PREFIX, "TASK-");
        assert_eq!(ATTEMPT_ID_PREFIX, "ATTEMPT-");
        assert_eq!(WORKFLOW_MODES, ["task", "execute"]);
        assert_eq!(
            PLANNING_STATUSES,
            ["planned", "in_progress", "refined", "needs_review"]
        );
        assert_eq!(
            BLOCKING_STOPPED_TYPES,
            [
                "external_operation_failed",
                "instructions_changed",
                "specification_defect"
            ]
        );
        assert!(valid_sha256(&"a".repeat(SHA256_HEX_LENGTH)));
        assert!(!valid_sha256(&"A".repeat(SHA256_HEX_LENGTH)));
        assert!(!valid_sha256(&"a".repeat(SHA256_HEX_LENGTH - 1)));
    }
}
