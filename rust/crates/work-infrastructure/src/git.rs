//! Git adapter and porcelain v1 -z parser.

use std::path::Path;
use std::time::Duration;

use serde_json::json;
use work_feature::error::{ExitCode, WorkError};
use work_feature::ports::{CommandStatus, Git};

use crate::process::run_argv;

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalGit;

impl Git for LocalGit {
    fn read_only(&self, root: &Path, args: &[String]) -> Result<Vec<u8>, WorkError> {
        if !is_read_only(args) {
            return Err(WorkError::new(
                ExitCode::Contract,
                "execute_worktree_git_invalid_command",
                "The Git operation is not read-only.",
                json!({}),
            ));
        }
        self.run(root, args, true)
    }

    fn mutate(&self, root: &Path, args: &[String]) -> Result<Vec<u8>, WorkError> {
        self.run(root, args, false)
    }
}

impl LocalGit {
    fn run(&self, root: &Path, args: &[String], read_only: bool) -> Result<Vec<u8>, WorkError> {
        let argv: Vec<String> = std::iter::once("git".to_owned())
            .chain(args.iter().cloned())
            .collect();
        let result = run_argv(&argv, root, Duration::from_secs(30));
        let prefix = if read_only {
            "execute_worktree_git"
        } else {
            "execute_git"
        };
        match result.status {
            CommandStatus::LaunchFailed => Err(WorkError::new(
                ExitCode::IoFailure,
                format!("{prefix}_missing"),
                "Git is not available.",
                json!({}),
            )),
            CommandStatus::TimedOut => Err(WorkError::new(
                ExitCode::IoFailure,
                format!("{prefix}_timeout"),
                if read_only {
                    "The read-only Git command timed out."
                } else {
                    "The Git command timed out."
                },
                json!({}),
            )),
            CommandStatus::Exited if result.exit_code != Some(0) => Err(WorkError::new(
                ExitCode::IoFailure,
                format!("{prefix}_failed"),
                if read_only {
                    "The read-only Git command failed."
                } else {
                    "The Git command failed."
                },
                json!({"exit_code": result.exit_code}),
            )),
            CommandStatus::Exited => Ok(result.stdout),
        }
    }
}

fn is_read_only(args: &[String]) -> bool {
    let mut index = 0;
    while args.get(index).is_some_and(|argument| argument == "-c") {
        index += 2;
    }
    matches!(
        args.get(index).map(String::as_str),
        Some("rev-parse" | "status" | "diff" | "ls-files")
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatus {
    pub index_status: char,
    pub worktree_status: char,
    pub path: String,
    pub original_path: Option<String>,
}

pub fn parse_porcelain_v1_z(raw: &[u8]) -> Result<Vec<GitStatus>, WorkError> {
    let mut parts: Vec<&[u8]> = raw.split(|byte| *byte == 0).collect();
    if parts.last() == Some(&b"".as_slice()) {
        parts.pop();
    }
    let mut results = Vec::new();
    let mut index = 0;
    while index < parts.len() {
        let row = parts[index];
        if row.len() < 4 || row[2] != b' ' || !row[0].is_ascii() || !row[1].is_ascii() {
            return Err(invalid_porcelain());
        }
        let index_status = row[0] as char;
        let worktree_status = row[1] as char;
        let path = decode_path(&row[3..])?;
        index += 1;
        let original_path =
            if matches!(index_status, 'R' | 'C') || matches!(worktree_status, 'R' | 'C') {
                let original = parts.get(index).ok_or_else(invalid_porcelain)?;
                index += 1;
                Some(decode_path(original)?)
            } else {
                None
            };
        results.push(GitStatus {
            index_status,
            worktree_status,
            path,
            original_path,
        });
    }
    Ok(results)
}

fn invalid_porcelain() -> WorkError {
    WorkError::new(
        ExitCode::InputFormat,
        "execute_worktree_invalid_porcelain",
        "Git returned malformed porcelain v1 data.",
        json!({}),
    )
}

fn decode_path(raw: &[u8]) -> Result<String, WorkError> {
    let path = std::str::from_utf8(raw).map_err(|_| {
        WorkError::new(
            ExitCode::InputFormat,
            "execute_worktree_invalid_utf8_path",
            "Git returned a path that is not valid UTF-8.",
            json!({}),
        )
    })?;
    if path.is_empty() || path.starts_with('/') || path.split('/').any(|part| part == "..") {
        return Err(WorkError::new(
            ExitCode::InputFormat,
            "execute_worktree_invalid_path",
            "Git returned an invalid project-relative path.",
            json!({"path": path}),
        ));
    }
    Ok(path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work-rust-git-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn parses_unicode_spaces_and_renames() {
        let rows = parse_porcelain_v1_z(
            b"?? some \xe4\xb8\xad\xe6\x96\x87.txt\0R  new name.txt\0old name.txt\0",
        )
        .unwrap();
        assert_eq!(rows[0].path, "some 中文.txt");
        assert_eq!(rows[1].original_path.as_deref(), Some("old name.txt"));
        assert_eq!(parse_porcelain_v1_z(b"?? a//b\0").unwrap()[0].path, "a//b");
        assert_eq!(
            parse_porcelain_v1_z(b"bad\0").unwrap_err().reason_code,
            "execute_worktree_invalid_porcelain"
        );
        assert_eq!(
            parse_porcelain_v1_z(b"?? \xff\0").unwrap_err().reason_code,
            "execute_worktree_invalid_utf8_path"
        );
    }

    #[test]
    fn temporary_repo_success_rejection_and_failure() {
        let root = root();
        let git = LocalGit;
        git.mutate(&root, &["init".into(), "-q".into()]).unwrap();
        fs::write(root.join("some 中文 file.txt"), b"contents\n").unwrap();
        let raw = git
            .read_only(
                &root,
                &[
                    "-c".into(),
                    "core.quotepath=false".into(),
                    "status".into(),
                    "--porcelain=v1".into(),
                    "-z".into(),
                    "--untracked-files=all".into(),
                ],
            )
            .unwrap();
        assert_eq!(
            parse_porcelain_v1_z(&raw).unwrap()[0].path,
            "some 中文 file.txt"
        );
        assert_eq!(
            git.read_only(&root, &["add".into(), ".".into()])
                .unwrap_err()
                .reason_code,
            "execute_worktree_git_invalid_command"
        );
        assert_eq!(
            git.read_only(
                &root,
                &["status".into(), "--definitely-unknown-option".into()]
            )
            .unwrap_err()
            .reason_code,
            "execute_worktree_git_failed"
        );
    }
}
