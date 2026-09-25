//! Portable Work identifier and path-segment validation.

use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierIssue {
    InvalidRequirementId,
    InvalidWorkflowId,
    InvalidTransactionId,
    UnsafePathSegment,
    WindowsDeviceName,
}

impl IdentifierIssue {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidRequirementId => "invalid_requirement_id",
            Self::InvalidWorkflowId => "invalid_workflow_id",
            Self::InvalidTransactionId => "invalid_transaction_id",
            Self::UnsafePathSegment => "unsafe_path_segment",
            Self::WindowsDeviceName => "windows_device_name",
        }
    }
}

pub fn path_segment_issue(segment: &str) -> Option<IdentifierIssue> {
    if segment.is_empty()
        || segment == "."
        || segment == ".."
        || segment.ends_with([' ', '.'])
        || segment.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c))
    {
        return Some(IdentifierIssue::UnsafePathSegment);
    }
    let stem = segment.split('.').next().unwrap().to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
    {
        return Some(IdentifierIssue::WindowsDeviceName);
    }
    None
}

macro_rules! identifier {
    ($name:ident, $validator:expr) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = IdentifierIssue;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                ($validator)(value)?;
                Ok(Self(value.to_owned()))
            }
        }
    };
}

identifier!(RequirementId, |value: &str| {
    if value.is_empty()
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
    {
        return Err(IdentifierIssue::InvalidRequirementId);
    }
    path_segment_issue(value).map_or(Ok(()), Err)
});

identifier!(WorkflowId, |value: &str| {
    if value.is_empty()
        || value.starts_with('-')
        || value.ends_with('-')
        || value.contains("--")
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        Err(IdentifierIssue::InvalidWorkflowId)
    } else {
        Ok(())
    }
});

identifier!(TransactionId, |value: &str| {
    let b = value.as_bytes();
    if b.len() == 25
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'T'
        && b[9..15].iter().all(u8::is_ascii_digit)
        && b[15] == b'Z'
        && b[16] == b'-'
        && b[17..]
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    {
        Ok(())
    } else {
        Err(IdentifierIssue::InvalidTransactionId)
    }
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_policy_matches_python() {
        assert!("example_1".parse::<RequirementId>().is_ok());
        assert_eq!(
            "feature-1.2".parse::<RequirementId>().unwrap().as_str(),
            "feature-1.2"
        );
        assert_eq!(
            "Feature"
                .parse::<RequirementId>()
                .unwrap_err()
                .reason_code(),
            "invalid_requirement_id"
        );
        assert_eq!(
            "CON.txt"
                .parse::<RequirementId>()
                .unwrap_err()
                .reason_code(),
            "invalid_requirement_id"
        );
        assert_eq!(
            "con".parse::<RequirementId>().unwrap_err().reason_code(),
            "windows_device_name"
        );
        assert!("plan-task".parse::<WorkflowId>().is_ok());
        assert!("plan--task".parse::<WorkflowId>().is_err());
        assert!("20260925T120000Z-abcdef12".parse::<TransactionId>().is_ok());
        assert!(
            "20260925T120000Z-ABCDEF12"
                .parse::<TransactionId>()
                .is_err()
        );
        assert_eq!(
            "specification-001"
                .parse::<TransactionId>()
                .unwrap_err()
                .reason_code(),
            "invalid_transaction_id"
        );
        assert_eq!(
            path_segment_issue("LPT9.log"),
            Some(IdentifierIssue::WindowsDeviceName)
        );
    }
}
