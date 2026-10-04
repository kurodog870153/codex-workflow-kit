//! Explicit invocation shape.

use serde::{Deserialize, Serialize};

use crate::schema::PublicSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationMode {
    Task,
    Revise,
    Execute,
    Migration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationOrigin {
    Explicit,
    ImplicitConfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationConfirmation {
    pub mode: InvocationMode,
    pub request: String,
    pub confirmed: bool,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplicitInvocationRequest {
    pub mode: InvocationMode,
    pub request: String,
    pub confirmation: InvocationConfirmation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationEntryKind {
    Workflow,
    ProgressResume,
    TaskPlanning,
    Migration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvocationEntry {
    pub kind: InvocationEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "InvocationWire")]
pub struct Invocation {
    pub schema: PublicSchema,
    pub mode: InvocationMode,
    pub origin: InvocationOrigin,
    pub request: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<InvocationConfirmation>,
    pub entry: InvocationEntry,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InvocationWire {
    schema: PublicSchema,
    mode: InvocationMode,
    origin: InvocationOrigin,
    request: String,
    confirmation: Option<InvocationConfirmation>,
    entry: InvocationEntry,
}

impl TryFrom<InvocationWire> for Invocation {
    type Error = &'static str;
    fn try_from(value: InvocationWire) -> Result<Self, Self::Error> {
        if value.schema != PublicSchema::WorkInvocation || value.request.trim().is_empty() {
            return Err("Invocation requires its exact schema and a nonempty request.");
        }
        match (value.origin, &value.confirmation) {
            (InvocationOrigin::Explicit, None) => {}
            (InvocationOrigin::ImplicitConfirmed, Some(proof))
                if proof.confirmed
                    && !proof.evidence.trim().is_empty()
                    && proof.mode == value.mode
                    && proof.request == value.request => {}
            _ => {
                return Err(
                    "Invocation origin requires exact confirmed mode and request evidence.",
                );
            }
        }
        Ok(Self {
            schema: value.schema,
            mode: value.mode,
            origin: value.origin,
            request: value.request,
            confirmation: value.confirmation,
            entry: value.entry,
        })
    }
}
