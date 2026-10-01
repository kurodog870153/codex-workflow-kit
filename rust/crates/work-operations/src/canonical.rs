//! Byte-level canonical contracts shared by Work features.

use caseless::default_case_fold_str;
use serde::de::{self, Deserialize, MapAccess, SeqAccess, Visitor};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonContractIssue {
    InvalidUtf8(usize),
    MultipleBom,
    DuplicateKey(String),
    InvalidConstant(String),
    InvalidJson { line: usize, column: usize },
    NotObject,
}

struct StrictValue(serde_json::Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a JSON value")
            }
            fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }
            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }
            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }
            fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
                Ok(StrictValue(
                    serde_json::Number::from_f64(value)
                        .ok_or_else(|| E::custom("invalid_json_constant"))?
                        .into(),
                ))
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }
            fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
                Ok(StrictValue(value.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(serde_json::Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_some<D: de::Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> Result<Self::Value, D::Error> {
                StrictValue::deserialize(deserializer)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<StrictValue>()? {
                    values.push(value.0);
                }
                Ok(StrictValue(values.into()))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, StrictValue>()? {
                    if values.insert(key.clone(), value.0).is_some() {
                        return Err(de::Error::custom(format!("duplicate_json_key:{key}")));
                    }
                }
                Ok(StrictValue(values.into()))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

pub fn parse_json_contract(raw: &[u8]) -> Result<serde_json::Value, JsonContractIssue> {
    let text = decode_utf8(raw)
        .map_err(|invalid| JsonContractIssue::InvalidUtf8(invalid.valid_up_to()))?;
    if text.starts_with('\u{feff}') {
        return Err(JsonContractIssue::MultipleBom);
    }
    let normalized = canonical_text(text);
    let mut deserializer = serde_json::Deserializer::from_str(&normalized);
    let value = StrictValue::deserialize(&mut deserializer).map_err(|invalid| {
        let message = invalid.to_string();
        if let Some(rest) = message.strip_prefix("duplicate_json_key:") {
            let key = rest.split(" at line ").next().unwrap_or(rest);
            JsonContractIssue::DuplicateKey(key.into())
        } else if (message.starts_with("expected value") || message.starts_with("invalid number"))
            && constant_at_error(&normalized, invalid.line(), invalid.column()).is_some()
        {
            JsonContractIssue::InvalidConstant(
                constant_at_error(&normalized, invalid.line(), invalid.column())
                    .unwrap()
                    .into(),
            )
        } else {
            let (line, column) = json_error_position(&normalized, invalid.line(), invalid.column());
            JsonContractIssue::InvalidJson { line, column }
        }
    })?;
    deserializer
        .end()
        .map_err(|invalid| JsonContractIssue::InvalidJson {
            line: invalid.line(),
            column: invalid.column(),
        })?;
    if !value.0.is_object() {
        return Err(JsonContractIssue::NotObject);
    }
    Ok(value.0)
}

fn constant_at_error(text: &str, line: usize, column: usize) -> Option<&'static str> {
    let start = text
        .split_inclusive('\n')
        .take(line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>()
        .checked_add(column.checked_sub(1)?)?;
    let bytes = text.as_bytes();
    for (position, constant) in [
        (start.saturating_sub(1), "-Infinity"),
        (start, "NaN"),
        (start, "Infinity"),
    ] {
        let end = position.checked_add(constant.len())?;
        if bytes.get(position..end) == Some(constant.as_bytes())
            && bytes
                .get(end)
                .is_none_or(|next| b",]} \t\r\n".contains(next))
        {
            return Some(constant);
        }
    }
    None
}

fn json_error_position(text: &str, line: usize, column: usize) -> (usize, usize) {
    if column > 1 {
        if let Some(line_text) = text.lines().nth(line.saturating_sub(1)) {
            let start = column - 2;
            if let Some(suffix) = line_text.get(start..) {
                let invalid_literal = match suffix.as_bytes().first() {
                    Some(b'n') => !suffix.starts_with("null"),
                    Some(b't') => !suffix.starts_with("true"),
                    Some(b'f') => !suffix.starts_with("false"),
                    _ => false,
                };
                if invalid_literal {
                    return (line, column - 1);
                }
            }
        }
    }
    (line, column)
}

pub fn decode_utf8(raw: &[u8]) -> Result<&str, std::str::Utf8Error> {
    let bytes = raw.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(raw);
    std::str::from_utf8(bytes)
}

pub fn canonical_text(text: &str) -> String {
    let normalized: String = text.nfc().collect();
    let normalized = normalized.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim_end_matches('\n');
    format!("{trimmed}\n")
}

pub fn portable_path_identity(path: &str) -> String {
    let normalized: String = path.nfc().collect();
    default_case_fold_str(&normalized)
}

pub fn canonical_bytes(raw: &[u8]) -> Result<Vec<u8>, std::str::Utf8Error> {
    Ok(canonical_text(decode_utf8(raw)?).into_bytes())
}

pub(crate) fn sha256_hex(raw: &[u8]) -> String {
    let digest = Sha256::digest(raw);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut hex, "{byte:02x}").expect("writing to String cannot fail");
    }
    hex
}

pub(crate) fn canonical_sha256(raw: &[u8]) -> Result<String, std::str::Utf8Error> {
    Ok(sha256_hex(&canonical_bytes(raw)?))
}

pub fn canonical_json(value: &serde_json::Value) -> Result<Vec<u8>, serde_json::Error> {
    let encoded = serde_json::to_string_pretty(value)?;
    Ok(canonical_text(&encoded).into_bytes())
}

pub(crate) fn canonical_json_sha256(
    value: &serde_json::Value,
) -> Result<String, serde_json::Error> {
    Ok(sha256_hex(&serde_json::to_vec(value)?))
}

pub struct InstructionSource<'a> {
    pub kind: &'a str,
    pub logical_name: &'a str,
    pub content: &'a [u8],
}

pub(crate) fn instructions_sha256(scope: &str, sources: &[InstructionSource<'_>]) -> String {
    let mut framed = b"WORK-INSTRUCTIONS-SHA-256-V1\n".to_vec();
    for source in sources {
        framed.push(b'S');
        for field in [
            scope.as_bytes(),
            source.kind.as_bytes(),
            source.logical_name.as_bytes(),
            source.content,
        ] {
            framed.extend_from_slice(field.len().to_string().as_bytes());
            framed.push(b':');
            framed.extend_from_slice(field);
        }
        framed.push(b'\n');
    }
    framed.extend_from_slice(b"END\n");
    sha256_hex(&framed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn portable_path_identity_matches_python_unicode_16() {
        assert_eq!(portable_path_identity("Straße/CAFÉ"), "strasse/café");
        assert_eq!(
            portable_path_identity("STRASSE/cafe\u{301}"),
            "strasse/café"
        );
        assert_eq!(portable_path_identity("İstanbul/Σ"), "i\u{307}stanbul/σ");
        assert_eq!(
            portable_path_identity("i\u{307}stanbul/ς"),
            "i\u{307}stanbul/σ"
        );
    }

    fn decode_base64(input: &str) -> Vec<u8> {
        let mut result = Vec::new();
        let mut bits = 0_u32;
        let mut count = 0_u8;
        for byte in input.bytes().take_while(|byte| *byte != b'=') {
            let digit = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => panic!("invalid fixture base64"),
            };
            bits = (bits << 6) | u32::from(digit);
            count += 6;
            if count >= 8 {
                count -= 8;
                result.push((bits >> count) as u8);
                bits &= (1 << count) - 1;
            }
        }
        result
    }

    #[test]
    fn migration_fingerprints_match_python_bytes() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../crates/work-operations/fixtures.json"
        ))
        .unwrap();
        for case in fixtures["canonical_text"].as_array().unwrap() {
            let input = decode_base64(case["input_base64"].as_str().unwrap());
            let expected = decode_base64(case["canonical_base64"].as_str().unwrap());
            assert_eq!(canonical_bytes(&input).unwrap(), expected);
            assert_eq!(
                canonical_sha256(&input).unwrap(),
                case["sha256"].as_str().unwrap()
            );
        }
        let instruction = &fixtures["instruction_fingerprint"];
        let contents: Vec<Vec<u8>> = instruction["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| decode_base64(source["content_base64"].as_str().unwrap()))
            .collect();
        let sources: Vec<_> = instruction["sources"]
            .as_array()
            .unwrap()
            .iter()
            .zip(contents.iter())
            .map(|(source, content)| InstructionSource {
                kind: source["kind"].as_str().unwrap(),
                logical_name: source["logical_name"].as_str().unwrap(),
                content,
            })
            .collect();
        assert_eq!(
            instructions_sha256(instruction["scope"].as_str().unwrap(), &sources),
            instruction["sha256"].as_str().unwrap()
        );
    }

    #[test]
    fn canonical_json_sorts_keys_and_preserves_unicode() {
        let value = serde_json::json!({"z":1,"名稱":"工作"});
        assert_eq!(
            canonical_json(&value).unwrap(),
            "{\n  \"z\": 1,\n  \"名稱\": \"工作\"\n}\n".as_bytes()
        );
    }

    #[test]
    fn json_contract_render_parse_and_canonical_bytes_match_python() {
        let contract = serde_json::json!({"schema":"example/v1","名稱":"工作"});
        let rendered = canonical_json(&contract).unwrap();
        assert_eq!(
            rendered,
            "{\n  \"schema\": \"example/v1\",\n  \"名稱\": \"工作\"\n}\n".as_bytes()
        );
        assert_eq!(parse_json_contract(&rendered).unwrap(), contract);
        let simple = serde_json::json!({"schema":"example/v1"});
        let expected = canonical_json(&simple).unwrap();
        for raw in [
            [expected.as_slice(), b"\n"].concat(),
            String::from_utf8(expected.clone())
                .unwrap()
                .replace('\n', "\r\n")
                .into_bytes(),
            [b"\xef\xbb\xbf".as_slice(), expected.as_slice()].concat(),
            b"{\"schema\":\"example/v1\"}\n".to_vec(),
        ] {
            assert_eq!(parse_json_contract(&raw).unwrap(), simple);
            assert_ne!(raw, expected);
        }
        let normalized = canonical_json(&serde_json::json!({"value":"cafe\u{301}\nline"})).unwrap();
        assert_eq!(
            normalized,
            "{\n  \"value\": \"café\\nline\"\n}\n".as_bytes()
        );
        assert_eq!(
            parse_json_contract(&normalized).unwrap()["value"],
            "café\nline"
        );
    }

    #[test]
    fn json_contract_parser_rejects_python_invalid_inputs() {
        assert_eq!(
            parse_json_contract(br#"{"schema":"one","schema":"two"}"#),
            Err(JsonContractIssue::DuplicateKey("schema".into()))
        );
        for raw in [
            b"# Example\n\n```json\n{\"schema\":\"example/v1\"}\n```\n".as_slice(),
            b"```json\n{\"schema\":\"example/v1\"}\n```\n",
            b"{\"schema\":\"example/v1\"}\nExtra",
            b"{\"schema\":\"example/v1\"}\n{}",
        ] {
            assert!(matches!(
                parse_json_contract(raw),
                Err(JsonContractIssue::InvalidJson { .. })
            ));
        }
        for (raw, constant) in [
            (br#"{"value": NaN}"#.as_slice(), "NaN"),
            (br#"{"value": Infinity}"#.as_slice(), "Infinity"),
        ] {
            assert_eq!(
                parse_json_contract(raw),
                Err(JsonContractIssue::InvalidConstant(constant.into()))
            );
        }
        assert_eq!(
            parse_json_contract(b"[]"),
            Err(JsonContractIssue::NotObject)
        );
    }

    #[test]
    fn instruction_fingerprint_keeps_python_frame_order_and_source_kinds() {
        let sources = [
            InstructionSource {
                kind: "workflow",
                logical_name: "work.instruction-loading",
                content: b"loading\n",
            },
            InstructionSource {
                kind: "workflow",
                logical_name: "work.workflow.task",
                content: b"workflow\n",
            },
            InstructionSource {
                kind: "instruction",
                logical_name: "task.general",
                content: b"general\n",
            },
            InstructionSource {
                kind: "reference",
                logical_name: "task.general.task-records",
                content: b"records\n",
            },
        ];
        assert_eq!(
            instructions_sha256("task", &sources),
            "4e79f44bde6d73dca7be301bfa22ec59cdff0d43553452723a1093eb7b1a134e"
        );
        let reordered = [&sources[0], &sources[2], &sources[1], &sources[3]];
        let reordered: Vec<_> = reordered
            .iter()
            .map(|source| InstructionSource {
                kind: source.kind,
                logical_name: source.logical_name,
                content: source.content,
            })
            .collect();
        assert_eq!(
            instructions_sha256("task", &reordered),
            "8963485dfdc23c8fc5484844a574cb53f6389d312d62bff466d3ecb946d271c0"
        );
        for (kind, expected) in [
            (
                "workflow",
                "ac48980170572797151c3d74b6777879c60302d31d2ae2ecdc6134367d9fcae4",
            ),
            (
                "instruction",
                "8bbd6fdc16492b3da9bef55cea6e852a129ecd787d0a70385fb725813f11d4de",
            ),
            (
                "reference",
                "168df0d09c894c5920c375876de1244692c205e7a0a0eb6688f0ebecf72ab8d6",
            ),
        ] {
            let name = format!("test.{kind}");
            assert_eq!(
                instructions_sha256(
                    "plan",
                    &[InstructionSource {
                        kind,
                        logical_name: &name,
                        content: b"x\n"
                    }]
                ),
                expected
            );
        }
    }

    #[test]
    fn invalid_utf8_and_one_bom_only() {
        assert!(canonical_bytes(b"\xff").is_err());
        assert_eq!(decode_utf8(b"\xff").unwrap_err().valid_up_to(), 0);
        assert_eq!(
            canonical_bytes(b"\xef\xbb\xbfhello\r\n\n").unwrap(),
            b"hello\n"
        );
        assert_eq!(
            canonical_bytes(b"\xef\xbb\xbf\xef\xbb\xbfhello").unwrap(),
            "\u{feff}hello\n".as_bytes()
        );
    }

    #[test]
    fn transaction_approval_uses_sorted_compact_json() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../crates/work-operations/fixtures.json"
        ))
        .unwrap();
        let transaction = &fixtures["transaction"];
        let value = serde_json::json!({
            "files": transaction["files"],
            "metadata": transaction["metadata"],
        });
        assert_eq!(
            canonical_json_sha256(&value).unwrap(),
            transaction["approval_sha256"].as_str().unwrap()
        );
    }

    #[test]
    fn strict_json_rejects_nested_duplicate_keys_and_multiple_bom() {
        assert_eq!(
            parse_json_contract(b"{\"a\": {\"x\": 1, \"x\": 2}}"),
            Err(JsonContractIssue::DuplicateKey("x".into()))
        );
        assert_eq!(
            parse_json_contract(b"[]"),
            Err(JsonContractIssue::NotObject)
        );
        assert_eq!(
            parse_json_contract(b"\xef\xbb\xbf\xef\xbb\xbf{}"),
            Err(JsonContractIssue::MultipleBom)
        );
        assert_eq!(parse_json_contract(b"{\"a\": 1}").unwrap()["a"], 1);
        assert_eq!(
            parse_json_contract(b"not json"),
            Err(JsonContractIssue::InvalidJson { line: 1, column: 1 })
        );
    }

    #[test]
    fn strict_json_reports_nonstandard_constants_only_in_value_position() {
        for (raw, constant) in [
            (br#"{"value": NaN}"#.as_slice(), "NaN"),
            (br#"{"value": Infinity}"#.as_slice(), "Infinity"),
            (br#"{"value": -Infinity}"#.as_slice(), "-Infinity"),
        ] {
            assert_eq!(
                parse_json_contract(raw),
                Err(JsonContractIssue::InvalidConstant(constant.into()))
            );
        }
        assert!(matches!(
            parse_json_contract(br#"{"value": 1}NaN"#),
            Err(JsonContractIssue::InvalidJson { .. })
        ));
        assert!(matches!(
            parse_json_contract(br#"{NaN: 1}"#),
            Err(JsonContractIssue::InvalidJson { .. })
        ));
    }
}
