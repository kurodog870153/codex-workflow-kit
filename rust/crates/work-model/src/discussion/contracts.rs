//! Public Session contract records and command definitions.

pub const COMMANDS: &[&str] = &[
    "init",
    "read",
    "status",
    "update",
    "question",
    "answer",
    "history",
    "recover",
    "preview",
    "apply",
    "recover-publication",
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_records_round_trip_with_exact_schema() {
        let records = crate::contract::data::registry().items;
        serde_json::from_value::<super::super::DiscussionSession>(
            records["work-discussion-session"]
                .description
                .example
                .clone(),
        )
        .unwrap();
        serde_json::from_value::<super::super::request::DiscussionRequest>(
            records["work-discussion-request"]
                .description
                .example
                .clone(),
        )
        .unwrap();
        assert_eq!(records.len(), 100);
        serde_json::from_value::<super::super::response::DiscussionResult>(
            records["work-discussion-result"]
                .description
                .example
                .clone(),
        )
        .unwrap();
        assert!(records["work-discussion-request"].scaffold.is_some());
        assert_eq!(COMMANDS.len(), 11);
        let request = &records["work-discussion-request"].description;
        let command = request
            .fields
            .iter()
            .find(|field| field.name == "command")
            .unwrap();
        assert_eq!(command.constraints["variants"], serde_json::json!(COMMANDS));
    }
}
