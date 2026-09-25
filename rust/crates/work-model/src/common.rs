//! Shared representations of public JSON field presence and contract kinds.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A JSON field that may explicitly contain `null`.
///
/// Serde field attributes must use the helpers below so that a missing field
/// does not silently become `null`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nullable<T> {
    Null,
    Value(T),
}

impl<T: Serialize> Serialize for Nullable<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_none(),
            Self::Value(value) => value.serialize(serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Nullable<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(deserializer)? {
            None => Self::Null,
            Some(value) => Self::Value(value),
        })
    }
}

pub fn deserialize_required_nullable<'de, D, T>(deserializer: D) -> Result<Nullable<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Nullable::deserialize(deserializer)
}

pub fn deserialize_optional_nullable<'de, D, T>(
    deserializer: D,
) -> Result<Option<Nullable<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Nullable::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractKind {
    SemanticRequest,
    GeneratedRequest,
    Artifact,
    Response,
    Envelope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliStatus {
    Success,
    AlreadyCompleted,
    Rejected,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Presence {
        #[serde(deserialize_with = "deserialize_required_nullable")]
        required_nullable: Nullable<String>,
        #[serde(
            default,
            deserialize_with = "deserialize_optional_nullable",
            skip_serializing_if = "Option::is_none"
        )]
        optional_nullable: Option<Nullable<String>>,
    }

    #[test]
    fn required_nullable_distinguishes_missing_null_and_value() {
        assert!(serde_json::from_value::<Presence>(json!({})).is_err());
        let null: Presence = serde_json::from_value(json!({"required_nullable":null})).unwrap();
        assert_eq!(null.required_nullable, Nullable::Null);
        assert_eq!(null.optional_nullable, None);
        assert_eq!(
            serde_json::to_value(&null).unwrap(),
            json!({"required_nullable":null})
        );
        let explicit_null: Presence =
            serde_json::from_value(json!({"required_nullable":"x","optional_nullable":null}))
                .unwrap();
        assert_eq!(explicit_null.optional_nullable, Some(Nullable::Null));
        assert_eq!(
            serde_json::to_value(&explicit_null).unwrap()["optional_nullable"],
            Value::Null
        );
        let value: Presence =
            serde_json::from_value(json!({"required_nullable":"x","optional_nullable":"y"}))
                .unwrap();
        assert_eq!(value.optional_nullable, Some(Nullable::Value("y".into())));
        assert_eq!(
            serde_json::to_value(&value).unwrap()["optional_nullable"],
            "y"
        );
        assert!(
            serde_json::from_value::<Presence>(json!({"required_nullable":"x","extra":1})).is_err()
        );
    }

    #[test]
    fn kind_literals_are_exact() {
        let kinds = [
            (ContractKind::SemanticRequest, "semantic_request"),
            (ContractKind::GeneratedRequest, "generated_request"),
            (ContractKind::Artifact, "artifact"),
            (ContractKind::Response, "response"),
            (ContractKind::Envelope, "envelope"),
        ];
        for (kind, literal) in kinds {
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                Value::String(literal.into())
            );
            assert_eq!(
                serde_json::from_value::<ContractKind>(json!(literal)).unwrap(),
                kind
            );
        }
    }

    #[test]
    fn cli_status_literals_are_exact() {
        for (status, literal) in [
            (CliStatus::Success, "success"),
            (CliStatus::AlreadyCompleted, "already_completed"),
            (CliStatus::Rejected, "rejected"),
            (CliStatus::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), json!(literal));
            assert_eq!(
                serde_json::from_value::<CliStatus>(json!(literal)).unwrap(),
                status
            );
        }
    }
}
