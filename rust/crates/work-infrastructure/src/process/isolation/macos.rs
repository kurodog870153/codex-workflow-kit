//! Seatbelt policy is inherited by exec and child processes; there is no host fallback.
use super::*;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use work_feature::ports::CommandStatus;

fn terminate(child: &mut std::process::Child) {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    // SAFETY: this child was assigned a private process group before exec.
    unsafe {
        kill(-(child.id() as i32), 9);
    }
    let _ = child.kill();
}

struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
    eof: bool,
}
impl Capture {
    fn drain(&mut self, stream: &mut impl Read) {
        let mut buffer = [0; 8192];
        // Bound each poll too: a continuously writing stream cannot starve timeout.
        for _ in 0..16 {
            match stream.read(&mut buffer) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(length) => {
                    self.bytes.extend_from_slice(&buffer[..length]);
                    if self.bytes.len() > crate::process::TAIL_BYTES {
                        self.truncated = true;
                        self.bytes
                            .drain(..self.bytes.len() - crate::process::TAIL_BYTES);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.eof = true;
                    break;
                }
            }
        }
    }
}
fn nonblocking(fd: i32) -> Result<(), WorkError> {
    unsafe extern "C" {
        fn fcntl(fd: i32, command: i32, ...) -> i32;
    }
    // SAFETY: live borrowed pipe descriptor; Darwin F_GETFL=3, F_SETFL=4,
    // O_NONBLOCK=4. Neither call transfers ownership of the descriptor.
    let flags = unsafe { fcntl(fd, 3) };
    if flags == -1 || unsafe { fcntl(fd, 4, flags | 4) } == -1 {
        return Err(unavailable());
    }
    Ok(())
}

fn profile(policy: &CommandIsolation) -> Result<String, WorkError> {
    let path = &policy.writable_directory;
    if path.chars().any(|c| c.is_control()) {
        return Err(unavailable());
    }
    let literal = path.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(format!(
        "(version 1) (deny default) (allow process-exec process-fork sysctl-read file-read*) \
        (allow file-write* (subpath \"{literal}\"))"
    ))
}

pub(super) fn run(
    request: &CommandRequest,
    policy: &CommandIsolation,
) -> Result<crate::process::ProcessCapture, WorkError> {
    let profile = profile(policy)?;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .args(["-p", &profile])
        .args(&request.argv)
        .current_dir(&request.cwd)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", &policy.writable_directory)
        .env("TMPDIR", &policy.writable_directory)
        .env("WORK_STAGING_DIR", &policy.writable_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(|_| unavailable())?;
    let mut stdout = child.stdout.take().ok_or_else(unavailable)?;
    let mut stderr = child.stderr.take().ok_or_else(unavailable)?;
    if nonblocking(stdout.as_raw_fd())
        .and_then(|_| nonblocking(stderr.as_raw_fd()))
        .is_err()
    {
        terminate(&mut child);
        let _ = child.wait();
        return Err(unavailable());
    }
    let mut out = Capture {
        bytes: Vec::new(),
        truncated: false,
        eof: false,
    };
    let mut err = Capture {
        bytes: Vec::new(),
        truncated: false,
        eof: false,
    };
    let deadline = std::time::Instant::now() + request.timeout;
    let mut exit = None;
    let (status, code) = loop {
        out.drain(&mut stdout);
        err.drain(&mut stderr);
        if exit.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    exit = Some(status);
                    terminate(&mut child);
                }
                Ok(None) => {}
                Err(_) => {
                    terminate(&mut child);
                    let _ = child.wait();
                    break (CommandStatus::LaunchFailed, None);
                }
            }
        }
        if out.eof && err.eof {
            if let Some(exit) = exit {
                break (CommandStatus::Exited, exit.code());
            }
        }
        if std::time::Instant::now() >= deadline {
            terminate(&mut child);
            let _ = child.wait();
            out.drain(&mut stdout);
            err.drain(&mut stderr);
            // Detached descendants still inherit Seatbelt. Do not hang on their
            // inherited pipes or claim success while they retain output handles.
            break (CommandStatus::TimedOut, None);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    Ok(crate::process::ProcessCapture {
        status,
        exit_code: code,
        stdout: out.bytes,
        stderr: err.bytes,
        stdout_truncated: out.truncated,
        stderr_truncated: err.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use work_feature::ports::CommandStatus;
    fn run_script(
        context: &RequirementWriterContext,
        script: &str,
        timeout: u64,
        number: usize,
    ) -> CommandOutcome {
        let receipt = format!("outputs/work/e/TASK-001/ATTEMPT-001/receipts/CMD-{number:03}");
        let runner = IsolatedCommandRunner::new(context.clone());
        let policy = policy(context, &receipt).unwrap();
        let preview = json!({"receipt_dir":receipt,"execution":{"work_isolation":policy},
            "working_directory":context.canonical_project_root,"request":{"timeout_seconds":timeout},
            "invocation":{"kind":"direct","executable":"/bin/sh","argv":["/bin/sh","-c",script]}});
        runner.bind(&preview).unwrap();
        runner.run(
            &work_feature::execution::command_publication::build_command_request(&preview).unwrap(),
        )
    }
    #[test]
    fn native_policy_allows_staging_but_denies_project_external_symlink_child_and_network_writes() {
        let context = super::super::tests::context();
        let root = &context.canonical_project_root;
        std::fs::write(root.join("original"), b"before").unwrap();
        let good = run_script(
            &context,
            "cat original; printf candidate > \"$WORK_STAGING_DIR/after\"",
            3,
            1,
        );
        assert_eq!(good.exit_code, Some(0), "{good:?}");
        assert_eq!(good.stdout_tail, "before");
        let p = policy(
            &context,
            "outputs/work/e/TASK-001/ATTEMPT-001/receipts/CMD-001",
        )
        .unwrap();
        assert_eq!(
            std::fs::read(Path::new(&p.writable_directory).join("after")).unwrap(),
            b"candidate"
        );
        for (n, script) in ["printf changed > original", "mkdir outside", "sh -c 'printf changed > original'",
            "ln -s \"$PWD/original\" \"$WORK_STAGING_DIR/link\"; printf changed > \"$WORK_STAGING_DIR/link\"",
            "printf changed > \"$WORK_STAGING_DIR/../escape\""].iter().enumerate() {
            let result = run_script(&context, script, 3, n + 2);
            assert_eq!(result.status, CommandStatus::Exited, "{result:?}");
            assert_ne!(result.exit_code, Some(0), "{result:?}");
            assert_eq!(std::fs::read(root.join("original")).unwrap(), b"before");
            assert!(!root.join("outside").exists());
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let result = run_script(
            &context,
            &format!(
                "/usr/bin/curl --noproxy '*' --max-time 1 http://{}/",
                listener.local_addr().unwrap()
            ),
            3,
            20,
        );
        assert_ne!(result.exit_code, Some(0), "{result:?}");
        assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
    }
    #[test]
    fn native_timeout_terminates_children_and_returns_bounded_capture() {
        let context = super::super::tests::context();
        let start = std::time::Instant::now();
        let result = run_script(
            &context,
            "printf started; (sleep 3; printf late > \"$WORK_STAGING_DIR/late\") & wait",
            1,
            1,
        );
        assert_eq!(result.status, CommandStatus::TimedOut, "{result:?}");
        assert_eq!(result.stdout_tail, "started");
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        let p = policy(
            &context,
            "outputs/work/e/TASK-001/ATTEMPT-001/receipts/CMD-001",
        )
        .unwrap();
        assert!(!Path::new(&p.writable_directory).join("late").exists());
    }
    #[test]
    fn native_capture_does_not_hang_on_detached_inherited_pipes() {
        let context = super::super::tests::context();
        let receipt = "outputs/work/e/TASK-001/ATTEMPT-001/receipts/CMD-001";
        let p = policy(&context, receipt).unwrap();
        let executable = std::env::current_exe().unwrap();
        let preview = json!({"receipt_dir":receipt,"execution":{"work_isolation":p},
            "working_directory":context.canonical_project_root,"request":{"timeout_seconds":1},
            "invocation":{"kind":"direct","executable":executable,"argv":[executable,"--exact",
                "process::isolation::macos::tests::detached_pipe_worker","--nocapture","--test-threads=1"]}});
        let runner = IsolatedCommandRunner::new(context);
        runner.bind(&preview).unwrap();
        let start = std::time::Instant::now();
        let result = runner.run(
            &work_feature::execution::command_publication::build_command_request(&preview).unwrap(),
        );
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "{result:?}"
        );
        if result.stdout_tail.contains("detached") {
            assert_eq!(result.status, CommandStatus::TimedOut, "{result:?}");
        } else {
            assert_eq!(result.exit_code, Some(0), "{result:?}");
        }
    }
    #[test]
    fn detached_pipe_worker() {
        if std::env::var_os("WORK_STAGING_DIR").is_none() {
            return;
        }
        unsafe extern "C" {
            fn fork() -> i32;
            fn setsid() -> i32;
            fn usleep(microseconds: u32) -> i32;
            fn write(fd: i32, bytes: *const u8, length: usize) -> isize;
            fn _exit(code: i32) -> !;
        }
        // After fork from the test harness, the child uses only async-signal-safe
        // libc calls and retains sandboxed stdout for a finite three seconds.
        let child = unsafe { fork() };
        assert!(child >= 0);
        if child == 0 {
            unsafe {
                if setsid() >= 0 {
                    write(1, b"detached\n".as_ptr(), 9);
                }
                usleep(3_000_000);
                _exit(0);
            }
        }
        // Give the child time to change sessions before the leader's normal exit.
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
