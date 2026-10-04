//! Immutable requirement source metadata, separate from instruction refresh.

use serde::{Deserialize, Serialize};

use crate::identifiers::{SourceId, path_segment_issue};
use crate::schema::PublicSchema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SourceContentPath(String);

impl SourceContentPath {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SourceContentPath {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if !value.starts_with("source.")
            || value.len() == "source.".len()
            || value.contains(['/', '\\'])
            || path_segment_issue(&value).is_some()
        {
            Err("invalid_source_content_path")
        } else {
            Ok(Self(value))
        }
    }
}

impl From<SourceContentPath> for String {
    fn from(value: SourceContentPath) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnapshotSource {
    GithubIssue {
        repository: String,
        number: u64,
        url: String,
    },
    File {
        path: String,
        media_type: String,
    },
    UserText {},
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotContent {
    pub path: SourceContentPath,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshot {
    pub schema: PublicSchema,
    pub requirement_id: String,
    pub source_id: SourceId,
    pub captured_at: String,
    pub source: SnapshotSource,
    pub content: SnapshotContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRead {
    pub schema: PublicSchema,
    pub manifest: SourceSnapshot,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceValidation {
    pub schema: PublicSchema,
    pub status: String,
    pub requirement_id: String,
    pub source_id: SourceId,
    pub content_sha256: String,
    pub content_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn example() -> serde_json::Value {
        crate::contract_data::registry_value()["items"]["work-source-snapshot/v1"]
            ["description"]["example"]
            .clone()
    }

    #[test]
    fn user_text_rejects_augmented_planning_metadata() {
        assert!(
            serde_json::from_value::<SnapshotSource>(
                json!({"kind":"user_text","hierarchy_selection":{}})
            )
            .is_err()
        );
    }

    #[test]
    fn read_and_validation_examples_match_models_and_binary_transport_is_lossless() {
        let registry = crate::contract_data::registry_value();
        let mut read: SourceRead = serde_json::from_value(
            registry["items"]["work-source-read/v1"]["description"]["example"].clone(),
        )
        .unwrap();
        read.bytes = vec![255, 0, 239, 187, 191, 13, 10];
        assert_eq!(
            serde_json::from_slice::<SourceRead>(&serde_json::to_vec(&read).unwrap()).unwrap(),
            read
        );
        let validation: SourceValidation = serde_json::from_value(
            registry["items"]["work-source-validation/v1"]["description"]["example"].clone(),
        )
        .unwrap();
        assert_eq!(validation.status, "valid");
    }

    #[test]
    fn three_source_kinds_round_trip_without_changing_metadata() {
        for source in [
            json!({"kind": "github_issue", "repository": "owner/repo", "number": 68,
                "url": "https://github.com/owner/repo/issues/68"}),
            json!({"kind": "file", "path": "原始需求.pdf", "media_type": "Application/PDF"}),
            json!({"kind": "user_text"}),
        ] {
            let mut value = example();
            value["source"] = source;
            let manifest: SourceSnapshot = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(manifest).unwrap(), value);
        }
    }

    #[test]
    fn file_media_type_is_required_and_scoped_to_file_sources() {
        for source in [
            json!({"kind":"file","path":"input.pdf"}),
            json!({"kind":"file","path":"input.pdf","media_type":null}),
            json!({"kind":"file","path":"input.pdf","media_type":42}),
            json!({"kind":"user_text","media_type":"text/plain"}),
            json!({"kind":"github_issue","repository":"owner/repo","number":68,
                "url":"https://github.com/owner/repo/issues/68","media_type":"application/json"}),
        ] {
            assert!(serde_json::from_value::<SnapshotSource>(source).is_err());
        }
        let registry = crate::contract_data::registry_value();
        let source = registry["items"]["work-source-snapshot/v1"]["description"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["name"] == "source")
            .unwrap();
        assert_eq!(
            source["constraints"]["variants"]["file"]["required"],
            json!(["kind", "path", "media_type"])
        );
        assert_eq!(
            source["constraints"]["variants"]["file"]["example"]["media_type"],
            "application/pdf"
        );
    }

    #[test]
    fn invalid_source_identifiers_are_rejected_at_the_contract_boundary() {
        for id in [
            "SRC-000",
            "SRC-01",
            "SRC-0001",
            "src-001",
            "SRC-../1",
            "SRC-18446744073709551616",
        ] {
            let mut value = example();
            value["source_id"] = json!(id);
            assert!(
                serde_json::from_value::<SourceSnapshot>(value).is_err(),
                "{id}"
            );
        }
        for id in ["SRC-001", "SRC-999", "SRC-1000"] {
            assert_eq!(id.parse::<SourceId>().unwrap().as_str(), id);
        }
    }

    #[test]
    fn content_is_a_portable_local_source_file() {
        for path in [
            "../source.txt",
            "/source.txt",
            "source/bytes",
            "source.\\bytes",
            "source.",
            "source.txt ",
            "source.txt:",
        ] {
            let mut value = example();
            value["content"]["path"] = json!(path);
            assert!(
                serde_json::from_value::<SourceSnapshot>(value).is_err(),
                "{path}"
            );
        }
        for path in ["source.txt", "source.json", "source.pdf", "source.bin"] {
            assert!(SourceContentPath::try_from(path.to_owned()).is_ok());
        }
    }
}
