//! Canonical ordering for current embedded routing manifests.

/// Canonical order for an embedded routing manifest in formal artifacts.
pub fn manifest_field_order(path: &[String]) -> Option<&'static [&'static str]> {
    if !path
        .iter()
        .any(|part| part == "routing_manifest" || part == "instruction_selection_manifest")
    {
        return None;
    }
    match path.last().map(String::as_str) {
        Some("routing_manifest" | "instruction_selection_manifest") => Some(&[
            "schema",
            "routing_input",
            "routing_status",
            "sources",
            "confirmation_required",
            "selection_sha256",
        ]),
        Some("routing_input") => Some(&[
            "mode",
            "status",
            "operation",
            "artifact_lifecycle",
            "formal_events",
            "role",
            "authorization_state",
            "verified_state_sha256",
        ]),
        Some("sources") => Some(&["logical_name", "path", "canonical_sha256"]),
        _ => Some(&[]),
    }
}
