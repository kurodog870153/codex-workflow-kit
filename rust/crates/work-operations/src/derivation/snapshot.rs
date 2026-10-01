//! Canonical raw-byte snapshots for transaction evidence.

use serde_json::{Value, json};

use crate::canonical::sha256_hex;
use crate::protocol::valid_sha256;
use crate::specification::transaction::TransactionIssue;

fn issue(reason_code: &'static str, message: &'static str) -> TransactionIssue {
    TransactionIssue {
        reason_code,
        message,
        details: json!({}),
    }
}

fn strict(value: &Value, required: &[&str], optional: &[&str]) -> Result<(), TransactionIssue> {
    let Some(object) = value.as_object() else {
        return Err(issue(
            "invalid_contract_value",
            "The JSON contract contains an invalid value.",
        ));
    };
    if required.iter().any(|field| !object.contains_key(*field))
        || object
            .keys()
            .any(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
    {
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
        ));
    }
    Ok(())
}

fn sha(value: &Value) -> bool {
    value.as_str().is_some_and(valid_sha256)
}

fn base64_encode(raw: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(raw.len().div_ceil(3) * 4);
    for chunk in raw.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        result.push(ALPHABET[(first >> 2) as usize] as char);
        result.push(ALPHABET[(((first & 3) << 4) | (second >> 4)) as usize] as char);
        result.push(if chunk.len() > 1 {
            ALPHABET[(((second & 15) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            ALPHABET[(third & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

fn base64_decode(value: &str) -> Option<Vec<u8>> {
    if value.len() % 4 != 0 || !value.is_ascii() {
        return None;
    }
    let mut result = Vec::with_capacity(value.len() / 4 * 3);
    for (index, chunk) in value.as_bytes().chunks_exact(4).enumerate() {
        let last = index + 1 == value.len() / 4;
        let digit = |byte: u8| -> Option<u8> {
            match byte {
                b'A'..=b'Z' => Some(byte - b'A'),
                b'a'..=b'z' => Some(byte - b'a' + 26),
                b'0'..=b'9' => Some(byte - b'0' + 52),
                b'+' => Some(62),
                b'/' => Some(63),
                _ => None,
            }
        };
        let a = digit(chunk[0])?;
        let b = digit(chunk[1])?;
        let c = if chunk[2] == b'=' && last {
            0
        } else {
            digit(chunk[2])?
        };
        let d = if chunk[3] == b'=' && last {
            0
        } else {
            digit(chunk[3])?
        };
        if chunk[2] == b'=' && chunk[3] != b'=' {
            return None;
        }
        result.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            result.push((b << 4) | (c >> 2));
        }
        if chunk[3] != b'=' {
            result.push((c << 6) | d);
        }
    }
    (base64_encode(&result) == value).then_some(result)
}

pub fn encode_snapshot(raw: &[u8]) -> Value {
    json!({"raw_sha256": sha256_hex(raw), "base64": base64_encode(raw)})
}

pub fn decode_snapshot(value: &Value) -> Result<Vec<u8>, TransactionIssue> {
    strict(value, &["raw_sha256", "base64"], &[])?;
    let raw = base64_decode(value["base64"].as_str().ok_or_else(|| {
        issue(
            "invalid_contract_value",
            "Transaction bytes must use canonical base64.",
        )
    })?)
    .ok_or_else(|| {
        issue(
            "invalid_contract_value",
            "Transaction bytes must use canonical base64.",
        )
    })?;
    if !sha(&value["raw_sha256"]) || value["raw_sha256"] != sha256_hex(&raw) {
        return Err(issue(
            "invalid_contract_value",
            "Transaction bytes do not match their fingerprint.",
        ));
    }
    Ok(raw)
}
