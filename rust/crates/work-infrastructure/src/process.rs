//! Shared argv-based child process execution.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use work_feature::ports::{CommandOutcome, CommandRequest, CommandRunner, CommandStatus};

const TAIL_BYTES: usize = 4096;

pub struct ProcessCapture {
    pub status: CommandStatus,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

pub fn run_argv(argv: &[String], cwd: &Path, timeout: Duration) -> ProcessCapture {
    run_argv_internal(argv, cwd, timeout, false)
}

fn run_argv_tail(argv: &[String], cwd: &Path, timeout: Duration) -> ProcessCapture {
    run_argv_internal(argv, cwd, timeout, true)
}

fn run_argv_internal(
    argv: &[String],
    cwd: &Path,
    timeout: Duration,
    tail_only: bool,
) -> ProcessCapture {
    let Some(program) = argv.first() else {
        return launch_failed();
    };
    let mut child = match Command::new(program)
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return launch_failed(),
    };
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let out_reader = thread::spawn(move || read_stream(stdout, tail_only));
    let err_reader = thread::spawn(move || read_stream(stderr, tail_only));
    let deadline = Instant::now() + timeout;
    let (status, exit_code) = loop {
        match child.try_wait() {
            Ok(Some(exit)) => break (CommandStatus::Exited, exit.code()),
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break (CommandStatus::TimedOut, None);
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break (CommandStatus::LaunchFailed, None);
            }
        }
    };
    let (stdout, stdout_truncated) = out_reader.join().unwrap_or_default();
    let (stderr, stderr_truncated) = err_reader.join().unwrap_or_default();
    ProcessCapture {
        status,
        exit_code,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    }
}

fn read_stream(mut stream: impl Read, tail_only: bool) -> (Vec<u8>, bool) {
    let mut bytes = Vec::new();
    let mut total = 0_usize;
    let mut chunk = [0_u8; 65536];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                total += count;
                bytes.extend_from_slice(&chunk[..count]);
                if tail_only && bytes.len() > TAIL_BYTES {
                    bytes.drain(..bytes.len() - TAIL_BYTES);
                }
            }
        }
    }
    (bytes, tail_only && total > TAIL_BYTES)
}

fn launch_failed() -> ProcessCapture {
    ProcessCapture {
        status: CommandStatus::LaunchFailed,
        exit_code: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
        stdout_truncated: false,
        stderr_truncated: false,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalCommandRunner;

impl CommandRunner for LocalCommandRunner {
    fn run(&self, request: &CommandRequest) -> CommandOutcome {
        let capture = run_argv_tail(&request.argv, &request.cwd, request.timeout);
        CommandOutcome {
            status: capture.status,
            exit_code: capture.exit_code,
            stdout_tail: String::from_utf8_lossy(&capture.stdout).into_owned(),
            stdout_truncated: capture.stdout_truncated,
            stderr_tail: String::from_utf8_lossy(&capture.stderr).into_owned(),
            stderr_truncated: capture.stderr_truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "work rust process {} {}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    #[cfg(unix)]
    #[test]
    fn argv_preserves_spaces_and_tail_capture_is_bounded() {
        let cwd = root();
        let run = |argv: Vec<String>| {
            LocalCommandRunner.run(&CommandRequest {
                argv,
                cwd: cwd.clone(),
                timeout: Duration::from_secs(3),
            })
        };
        let spaced = run(vec![
            "/usr/bin/printf".into(),
            "%s".into(),
            "a b 中文".into(),
        ]);
        assert_eq!(spaced.status, CommandStatus::Exited);
        assert_eq!(spaced.stdout_tail, "a b 中文");
        assert_eq!(spaced.exit_code, Some(0));
        let large = run(vec![
            "awk".into(),
            "BEGIN { for (i=0; i<5000; i++) printf \"x\"; for (i=0; i<5000; i++) printf \"y\" > \"/dev/stderr\" }".into(),
        ]);
        assert_eq!(large.status, CommandStatus::Exited);
        assert_eq!(large.stdout_tail.len(), TAIL_BYTES);
        assert!(large.stdout_truncated);
        assert_eq!(large.stderr_tail.len(), TAIL_BYTES);
        assert!(large.stderr_truncated);
        assert!(large.stdout_tail.bytes().all(|b| b == b'x'));
        let concurrent = run(vec![
            "awk".into(),
            "BEGIN { for (i=0; i<200000; i++) printf \"o\"; for (i=0; i<200000; i++) printf \"e\" > \"/dev/stderr\"; exit 7 }".into(),
        ]);
        assert_eq!(concurrent.status, CommandStatus::Exited);
        assert_eq!(concurrent.exit_code, Some(7));
        assert_eq!(concurrent.stdout_tail, "o".repeat(TAIL_BYTES));
        assert_eq!(concurrent.stderr_tail, "e".repeat(TAIL_BYTES));
        assert!(concurrent.stdout_truncated && concurrent.stderr_truncated);
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_child_and_launch_failure_is_reported() {
        let cwd = root();
        let started = Instant::now();
        let timed = LocalCommandRunner.run(&CommandRequest {
            argv: vec!["/bin/sleep".into(), "2".into()],
            cwd: cwd.clone(),
            timeout: Duration::from_millis(50),
        });
        assert_eq!(timed.status, CommandStatus::TimedOut);
        assert_eq!(timed.exit_code, None);
        assert!(started.elapsed() < Duration::from_secs(1));
        let with_output = LocalCommandRunner.run(&CommandRequest {
            argv: vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf started; exec sleep 3".into(),
            ],
            cwd: cwd.clone(),
            timeout: Duration::from_millis(200),
        });
        assert_eq!(with_output.status, CommandStatus::TimedOut);
        assert_eq!(with_output.exit_code, None);
        assert!(with_output.stdout_tail.contains("started"));
        let missing = LocalCommandRunner.run(&CommandRequest {
            argv: vec!["work-command-that-does-not-exist".into()],
            cwd,
            timeout: Duration::from_secs(1),
        });
        assert_eq!(missing.status, CommandStatus::LaunchFailed);
        assert_eq!(missing.exit_code, None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_argv_timeout_and_launch_failure() {
        let cwd = root();
        let spaced = LocalCommandRunner.run(&CommandRequest {
            argv: vec!["cmd.exe".into(), "/C".into(), "echo a b".into()],
            cwd: cwd.clone(),
            timeout: Duration::from_secs(3),
        });
        assert_eq!(spaced.status, CommandStatus::Exited);
        assert_eq!(spaced.stdout_tail.trim(), "a b");
        let timed = LocalCommandRunner.run(&CommandRequest {
            argv: vec![
                "powershell.exe".into(),
                "-NoProfile".into(),
                "-Command".into(),
                "Start-Sleep -Seconds 2".into(),
            ],
            cwd: cwd.clone(),
            timeout: Duration::from_millis(50),
        });
        assert_eq!(timed.status, CommandStatus::TimedOut);
        let missing = LocalCommandRunner.run(&CommandRequest {
            argv: vec!["work-command-that-does-not-exist".into()],
            cwd,
            timeout: Duration::from_secs(1),
        });
        assert_eq!(missing.status, CommandStatus::LaunchFailed);
    }
}
