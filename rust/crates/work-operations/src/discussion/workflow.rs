//! Staged workflow and effect rules derived only from verified committed Session state.

use super::{Result, ready_to_generate, view};
use serde_json::{Value, json};
use work_model::common::Nullable;
use work_model::discussion::DiscussionSession;

pub fn resume(session: &DiscussionSession) -> Result<Value> {
    let view = view(session)?;
    let ready = ready_to_generate(session).is_ok();
    let action = if ready {
        "preview"
    } else if view.continuation.question != Nullable::Null {
        "answer"
    } else {
        "update"
    };
    Ok(
        json!({"view":view,"next_action":action,"requires_user_confirmation":true,
        "planning_ready":ready,"preview_required":true,"save_authorization_is_publication_authorization":false}),
    )
}

pub fn command_definition(name: &str) -> Option<Value> {
    if !work_model::discussion::contracts::COMMANDS.contains(&name) {
        return None;
    }
    let read = matches!(name, "read" | "status" | "history" | "preview");
    let publication = matches!(name, "apply" | "recover-publication");
    let formal = publication || name == "preview";
    let group = if formal { "task" } else { "discussion" };
    let operation = if name == "recover-publication" {
        "recover"
    } else {
        name
    };
    Some(
        json!({"command":name,"request_contract":"work-discussion-request","result_contract":"work-discussion-result",
        "effect":if read { "read_only" } else { "write" },
        "authorization":if read {"read_only"} else if publication {"exact_preview_publication"} else {"bounded_session_save"},
        "group":group,"operation":operation,
        "route":"task.general.task-records","help":format!("{group} {operation} --input-file <work-discussion-request.json>"),
        "execute_authorized":false}),
    )
}
