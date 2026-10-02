//! Public delegation request, envelope, and validation formats.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_optional_nullable};
use crate::schema::PublicSchema;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationBuildRequest {
    pub schema: PublicSchema,
    pub role: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub mode: Option<Nullable<String>>,
    pub request: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub planning_source: Option<Nullable<crate::task::draft::PlanningSource>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub task_path: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub task_id: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_progress_path: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub content: Option<Nullable<Value>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub confirmed_request: Option<Nullable<crate::specification::SpecPrepareRequest>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub decisions: Option<Nullable<Vec<Value>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub affected_task_ids: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub repository_evidence: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub saved_discussion: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub continuation_point: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub save_approval: Option<Nullable<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationEnvelope {
    pub schema: PublicSchema,
    pub marker: String,
    pub skill: String,
    pub role: String,
    pub sender: String,
    pub mode: String,
    pub project_root: String,
    pub skill_root: String,
    pub request: String,
    pub context: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegationValidation {
    pub schema: PublicSchema,
    pub status: String,
    pub role: String,
    pub mode: String,
    pub scope: String,
    pub source_validation: String,
    pub sender_authentication: String,
    pub grants_authorization: bool,
}
