//! Formal and discussion handoff formats.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::common::{Nullable, deserialize_optional_nullable};
use crate::schema::PublicSchema;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionHandoffRequest {
    pub schema: PublicSchema,
    #[serde(deserialize_with = "deserialize_direction")]
    pub direction: String,
    pub requirement_id: String,
    pub summary: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub confirmed_approach: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub requested_changes: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub preserve: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub affected_ids: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub validation_requirements: Option<Nullable<Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscussionHandoff {
    pub schema: PublicSchema,
    pub marker: String,
    #[serde(deserialize_with = "deserialize_direction")]
    pub direction: String,
    pub requirement_id: String,
    pub source_stage: String,
    pub target_stage: String,
    pub source_status: String,
    pub source_validation: String,
    pub grants_authorization: bool,
    pub summary: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub confirmed_approach: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub requested_changes: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub preserve: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub affected_ids: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub validation_requirements: Option<Nullable<Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormalHandoff {
    pub schema: PublicSchema,
    pub marker: String,
    #[serde(deserialize_with = "deserialize_direction")]
    pub direction: String,
    pub requirement_id: String,
    pub artifacts: BTreeMap<String, String>,
    pub source: Value,
    pub target: Value,
    pub summary: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub confirmed_approach: Option<Nullable<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub requested_changes: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub preserve: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub affected_ids: Option<Nullable<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub validation_requirements: Option<Nullable<Vec<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffValidation {
    pub schema: PublicSchema,
    pub marker: String,
    #[serde(deserialize_with = "deserialize_direction")]
    pub direction: String,
    pub requirement_id: String,
    pub source_stage: String,
    pub target_stage: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffSourceValidation {
    pub schema: PublicSchema,
    pub marker: String,
    #[serde(deserialize_with = "deserialize_direction")]
    pub direction: String,
    pub requirement_id: String,
    pub source_stage: String,
    pub target_stage: String,
    pub status: String,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub task_path: Option<Nullable<String>>,
    pub source: Value,
}

fn deserialize_direction<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let direction = String::deserialize(deserializer)?;
    if matches!(direction.as_str(), "task_to_execute" | "execute_to_task") {
        Ok(direction)
    } else {
        Err(serde::de::Error::custom(
            "only Task/Execute handoff directions are supported",
        ))
    }
}
