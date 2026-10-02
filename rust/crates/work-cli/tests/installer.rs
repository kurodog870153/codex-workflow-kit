//! Installer contracts, including macOS and Windows execution.

use std::fs;
use std::path::{Path, PathBuf};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

#[test]
fn mac_installer_requires_local_rust_build_and_startup_check() {
    let script =
        fs::read_to_string(repository().join("os-scripts/mac/install-work.command")).unwrap();
    for required in [
        "cargo build --release --locked",
        "xcrun --show-sdk-path",
        "\"$built_work\" --help",
        "rustc cargo",
        "install_binary",
    ] {
        assert!(script.contains(required), "missing {required}");
    }
    assert!(!script.contains("work.py"));
    assert!(!script.contains("worklib"));
}

#[test]
fn windows_installer_requires_local_rust_build_and_startup_check() {
    let script =
        fs::read_to_string(repository().join("os-scripts/windows/install-work.bat")).unwrap();
    for required in [
        "cargo build --release --locked",
        "rustc -vV",
        "\"!built_work!\" --help",
        "-windows-msvc",
        "-windows-gnu",
        "where link.exe",
        "where gcc.exe",
        "if exist \"!CARGO_HOME!\\bin\\rustc.exe\" if exist \"!CARGO_HOME!\\bin\\cargo.exe\"",
        "if exist \"%USERPROFILE%\\.cargo\\bin\\rustc.exe\" if exist \"%USERPROFILE%\\.cargo\\bin\\cargo.exe\"",
        "set \"PATH=!CARGO_HOME!\\bin;!PATH!\"",
        "set \"PATH=%USERPROFILE%\\.cargo\\bin;!PATH!\"",
        "pushd \"%project_directory%\\rust\"",
        "if errorlevel 1 goto runtime_error",
        "if errorlevel 1 goto source_error",
        "if errorlevel 1 goto install_error",
        "copy /Y \"!built_work!\"",
        "move /Y \"!staged_work!\"",
    ] {
        assert!(script.contains(required), "missing {required}");
    }
    assert!(!script.contains("work.py"));
    assert!(!script.contains("worklib"));
    assert!(!script.contains("pip install"));
    assert!(script.contains("set \"install_home=%USERPROFILE%\""));
    assert!(
        script.contains("if \"!home_choice!\"==\"1\" set \"install_home=!install_home!\\.agents\"")
    );
    assert!(script.contains("set \"target_work=!install_home!\\skills\\work\""));
    assert!(!script.contains("!install_home!\\.agents\\skills\\work"));
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::process::{Command, Output, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn workspace(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "work-installer-rust-{label}-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn installer() -> PathBuf {
        repository().join("os-scripts/mac/install-work.command")
    }

    fn run(script: &Path, input: &str, home: &Path, path_prefix: Option<&Path>) -> Output {
        let mut command = Command::new("bash");
        command
            .arg(script)
            .current_dir(repository())
            .env("HOME", home)
            .env("RUSTUP_TOOLCHAIN", "1.85.0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in ["RUSTUP_HOME", "CARGO_HOME"] {
            if std::env::var_os(key).is_none() {
                let original = std::env::var_os("HOME").unwrap();
                command.env(
                    key,
                    PathBuf::from(original).join(if key == "RUSTUP_HOME" {
                        ".rustup"
                    } else {
                        ".cargo"
                    }),
                );
            }
        }
        if let Some(prefix) = path_prefix {
            let mut paths = vec![prefix.to_path_buf()];
            paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
            command.env("PATH", std::env::join_paths(paths).unwrap());
        }
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn success(output: &Output) {
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn work(home: &Path) -> PathBuf {
        work_at(&home.join(".agents"))
    }

    fn work_at(install_root: &Path) -> PathBuf {
        let root = install_root.join("skills/work");
        for relative in [
            "SKILL.md",
            "agents/openai.yaml",
            "references/instruction-loading.md",
            "references/instruction-loading/invocation.md",
            "scripts/work",
            "references/workflows/plan.md",
            "references/workflows/task.md",
            "references/workflows/execute.md",
            "references/subagents/plan.md",
            "references/subagents/task-coordinator.md",
            "references/subagents/task-skill.md",
            "references/subagents/execute.md",
        ] {
            assert!(root.join(relative).is_file(), "{relative}");
        }
        for relative in [
            "scripts/work.py",
            "scripts/worklib",
            "scripts/tests",
            "plan",
            "task",
            "execute",
            "shared",
        ] {
            assert!(!root.join(relative).exists(), "{relative}");
        }
        for relative in ["agents", "rules"] {
            assert!(!install_root.join(relative).exists());
        }
        root
    }

    fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, dir: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    if entry.file_name() != "__pycache__" {
                        visit(root, &path, output);
                    }
                } else if path.is_file() && entry.file_name() != ".DS_Store" {
                    output.insert(
                        path.strip_prefix(root).unwrap().to_path_buf(),
                        fs::read(path).unwrap(),
                    );
                }
            }
        }
        let mut result = BTreeMap::new();
        visit(root, root, &mut result);
        result
    }

    fn branch(path: &Path, source: &Path) -> Option<String> {
        let instructions = source.join("references/instructions");
        if !path.starts_with(&instructions) {
            return None;
        }
        let mut parent = path.parent().unwrap();
        while parent != instructions {
            if parent.join("instructions.md").is_file() {
                return Some(
                    parent
                        .strip_prefix(&instructions)
                        .unwrap()
                        .components()
                        .skip(1)
                        .collect::<PathBuf>()
                        .to_string_lossy()
                        .replace(std::path::MAIN_SEPARATOR, "/"),
                );
            }
            parent = parent.parent().unwrap();
        }
        panic!("instruction without owning branch: {}", path.display());
    }

    fn assert_contents(home: &Path, branches: Option<&BTreeSet<&str>>) -> PathBuf {
        let source = repository().join("skills/work");
        let mut expected = files(&source);
        expected.retain(|relative, _| {
            if matches!(
                relative.extension().and_then(|v| v.to_str()),
                Some("py" | "pyc")
            ) {
                return false;
            }
            branch(&source.join(relative), &source)
                .is_none_or(|name| branches.is_none_or(|selected| selected.contains(name.as_str())))
        });
        let installed = work(home);
        let mut actual = files(&installed);
        assert!(actual.remove(&PathBuf::from("scripts/work")).is_some());
        assert_eq!(
            actual.keys().collect::<Vec<_>>(),
            expected.keys().collect::<Vec<_>>()
        );
        for (relative, content) in expected {
            assert_eq!(actual[&relative], content, "{}", relative.display());
        }
        installed
    }

    fn assert_branch(work: &Path, mode: &str, branch: &str, present: bool) {
        assert_eq!(
            work.join("references/instructions")
                .join(mode)
                .join(branch)
                .join("instructions.md")
                .is_file(),
            present,
            "{mode}/{branch}"
        );
    }

    #[test]
    fn installer_is_executable() {
        assert_ne!(
            fs::metadata(installer()).unwrap().permissions().mode() & 0o111,
            0
        );
    }

    #[test]
    fn installer_finds_rust_outside_click_launch_path() {
        let home = workspace("rust-path");
        let rust_bin = home.join(".cargo/bin");
        fs::create_dir_all(&rust_bin).unwrap();
        for (tool, content) in [
            ("rustc", "#!/bin/sh\necho 'rustc 1.85.0'\n"),
            (
                "cargo",
                "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'cargo 1.85.0'; else echo 'fallback cargo invoked' >&2; exit 1; fi\n",
            ),
        ] {
            let path = rust_bin.join(tool);
            fs::write(&path, content).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        let mut child = Command::new("bash")
            .arg(installer())
            .current_dir(repository())
            .env("HOME", &home)
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"1\n1\n").unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("fallback cargo invoked"), "{stderr}");
        assert!(stderr.contains("Rust build failed"), "{stderr}");
        assert!(!home.join(".agents").exists());
    }

    #[test]
    fn default_home_installs_backend_only() {
        let home = workspace("backend");
        success(&run(&installer(), "1\n3\n", &home, None));
        let installed = work(&home);
        for mode in ["plan", "task", "execute"] {
            for branch in ["general", "web", "web/backend"] {
                assert_branch(&installed, mode, branch, true);
            }
            assert_branch(&installed, mode, "programming-language", false);
            assert_branch(&installed, mode, "programming-language/java", false);
        }
    }

    #[test]
    fn custom_home_installs_jpa_and_mybatis_at_correct_levels() {
        let home = workspace("java");
        success(&run(
            &installer(),
            &format!("2\n{}\n5 6\n", home.display()),
            &repository(),
            None,
        ));
        let installed = work_at(&home);
        assert!(!home.join(".agents").exists());
        assert_branch(&installed, "plan", "programming-language", true);
        assert_branch(&installed, "plan", "programming-language/java", true);
        assert_branch(
            &installed,
            "plan",
            "programming-language/java/persistence",
            false,
        );
        for branch in [
            "programming-language/java/persistence/jpa",
            "programming-language/java/persistence/mybatis",
        ] {
            assert_branch(&installed, "plan", branch, false);
            for mode in ["task", "execute"] {
                assert_branch(
                    &installed,
                    mode,
                    "programming-language/java/persistence",
                    true,
                );
                assert_branch(&installed, mode, branch, true);
            }
        }
    }

    #[test]
    fn custom_home_installs_astro_and_tailwind_at_correct_levels() {
        let home = workspace("frontend");
        success(&run(
            &installer(),
            &format!("2\n{}\n9 11\n", home.display()),
            &repository(),
            None,
        ));
        let installed = work_at(&home);
        assert!(!home.join(".agents").exists());
        for branch in ["web/frontend", "web/frontend/css"] {
            assert_branch(&installed, "plan", branch, true);
        }
        for branch in ["web/frontend/astro", "web/frontend/css/tailwind"] {
            assert_branch(&installed, "plan", branch, false);
            for mode in ["task", "execute"] {
                assert_branch(&installed, mode, branch, true);
            }
        }
    }

    #[test]
    fn typescript_option_installs_language_without_web_frontend() {
        let home = workspace("typescript");
        success(&run(&installer(), "1\n8\n", &home, None));
        let installed = work(&home);
        for mode in ["plan", "task", "execute"] {
            assert_branch(&installed, mode, "programming-language", true);
            assert_branch(&installed, mode, "programming-language/typescript", true);
            assert_branch(&installed, mode, "web", false);
        }
    }

    #[test]
    fn frontend_and_typescript_options_install_independent_paths() {
        let home = workspace("frontend-typescript");
        success(&run(&installer(), "1\n7 8\n", &home, None));
        let installed = work(&home);
        for mode in ["plan", "task", "execute"] {
            assert_branch(&installed, mode, "web/frontend", true);
            assert_branch(&installed, mode, "programming-language/typescript", true);
            assert_branch(&installed, mode, "web/frontend/typescript", false);
        }
    }

    #[test]
    fn installed_contents_and_cli_work_outside_repository() {
        let cases: [(&str, Option<&[&str]>); 5] = [
            ("1", Some(&["general"])),
            ("all", None),
            ("3", Some(&["general", "web", "web/backend"])),
            (
                "5 6",
                Some(&[
                    "general",
                    "programming-language",
                    "programming-language/java",
                    "programming-language/java/persistence",
                    "programming-language/java/persistence/jpa",
                    "programming-language/java/persistence/mybatis",
                ]),
            ),
            (
                "9 11",
                Some(&[
                    "general",
                    "web",
                    "web/frontend",
                    "web/frontend/astro",
                    "web/frontend/css",
                    "web/frontend/css/tailwind",
                ]),
            ),
        ];
        for (selection, branches) in cases {
            let home = workspace("contents");
            success(&run(
                &installer(),
                &format!("1\n{selection}\n"),
                &home,
                None,
            ));
            let selected = branches.map(|values| values.iter().copied().collect());
            let installed = assert_contents(&home, selected.as_ref());
            let cli = installed.join("scripts/work");
            let help = Command::new(&cli)
                .arg("--help")
                .current_dir(&home)
                .env_remove("PYTHONPATH")
                .output()
                .unwrap();
            success(&help);
            assert!(String::from_utf8_lossy(&help.stdout).contains("usage: work"));
            let status = Command::new(&cli)
                .args([
                    "--project-root",
                    home.to_str().unwrap(),
                    "workflow",
                    "status",
                    "--requirement-id",
                    "example",
                    "--user-config-root",
                    home.to_str().unwrap(),
                ])
                .current_dir(&home)
                .env_remove("PYTHONPATH")
                .output()
                .unwrap();
            success(&status);
            assert!(
                String::from_utf8_lossy(&status.stdout)
                    .contains("\"next_action\": \"prepare_plan\"")
            );
        }
    }

    #[test]
    fn reinstall_refreshes_official_files_and_preserves_user_files() {
        let home = workspace("reinstall");
        success(&run(&installer(), "1\nall\n", &home, None));
        let installed = assert_contents(&home, None);
        let old_branch = installed.join("references/instructions/task/web/backend");
        for (relative, content) in [
            ("stale.txt", "keep this file"),
            ("SKILL.md", "outdated"),
            (
                "references/instructions/task/web/backend/instructions.md",
                "outdated",
            ),
            (
                "references/instructions/task/web/backend/references/security.md",
                "outdated",
            ),
            (
                "references/instructions/task/web/backend/custom.md",
                "keep custom",
            ),
            ("scripts/work.py", "legacy entry"),
            ("scripts/worklib/legacy.py", "legacy module"),
        ] {
            let path = installed.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        let output = run(&installer(), "1\n1\n", &home, None);
        success(&output);
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("Previously installed branches and stale files will be kept")
        );
        let source = repository().join("skills/work");
        for (relative, content) in files(&source) {
            if matches!(
                relative.extension().and_then(|v| v.to_str()),
                Some("py" | "pyc")
            ) {
                continue;
            }
            assert_eq!(
                fs::read(installed.join(&relative)).unwrap(),
                content,
                "{}",
                relative.display()
            );
        }
        assert_eq!(
            fs::read_to_string(installed.join("stale.txt")).unwrap(),
            "keep this file"
        );
        assert_eq!(
            fs::read_to_string(old_branch.join("custom.md")).unwrap(),
            "keep custom"
        );
        assert_eq!(
            fs::read_to_string(installed.join("scripts/work.py")).unwrap(),
            "legacy entry"
        );
        assert_eq!(
            fs::read_to_string(installed.join("scripts/worklib/legacy.py")).unwrap(),
            "legacy module"
        );
    }

    fn copy_tree(source: &Path, target: &Path, omit: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let from = entry.path();
            if from == omit
                || entry.file_name() == "__pycache__"
                || entry.file_name() == ".DS_Store"
            {
                continue;
            }
            let to = target.join(entry.file_name());
            if from.is_dir() {
                copy_tree(&from, &to, omit);
            } else {
                fs::copy(from, to).unwrap();
            }
        }
    }

    #[test]
    fn missing_source_never_writes_installation() {
        for missing in [
            "references/instruction-loading/invocation.md",
            "references/workflows/specification.md",
            "references/workflows/task.md",
            "references/workflows/progress.md",
            "references/subagents/artifact-editor.md",
            "references/subagents/progress-saver.md",
            "references/instructions/task/web/backend/instructions.md",
        ] {
            for existing in [false, true] {
                let root = workspace("missing");
                let fixture = root.join("source");
                let script = fixture.join("os-scripts/mac/install-work.command");
                fs::create_dir_all(script.parent().unwrap()).unwrap();
                fs::copy(installer(), &script).unwrap();
                fs::create_dir_all(fixture.join("rust")).unwrap();
                for manifest in ["Cargo.toml", "Cargo.lock"] {
                    fs::copy(
                        repository().join("rust").join(manifest),
                        fixture.join("rust").join(manifest),
                    )
                    .unwrap();
                }
                let source = repository().join("skills/work");
                assert!(source.join(missing).is_file(), "{missing}");
                copy_tree(&source, &fixture.join("skills/work"), &source.join(missing));
                let home = root.join("home");
                fs::create_dir(&home).unwrap();
                let installed = home.join(".agents/skills/work");
                if existing {
                    fs::create_dir_all(&installed).unwrap();
                    fs::write(installed.join("SKILL.md"), b"existing installation").unwrap();
                }
                let output = run(&script, "1\n3\n", &home, None);
                assert_eq!(
                    output.status.code(),
                    Some(1),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                let message = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    message.contains("required Work skill source not found"),
                    "{message}"
                );
                assert!(message.contains(missing), "{message}");
                if existing {
                    assert_eq!(
                        files(&installed),
                        BTreeMap::from([(
                            PathBuf::from("SKILL.md"),
                            b"existing installation".to_vec()
                        )])
                    );
                } else {
                    assert!(!home.join(".agents").exists());
                }
            }
        }
    }

    #[test]
    fn failed_build_preserves_existing_binary_and_instructions() {
        let home = workspace("build-failure");
        let scripts = home.join(".agents/skills/work/scripts");
        fs::create_dir_all(&scripts).unwrap();
        fs::write(scripts.join("work"), b"previous work binary").unwrap();
        fs::write(
            scripts.parent().unwrap().join("SKILL.md"),
            b"previous instructions",
        )
        .unwrap();
        let fake_tools = home.join("fake-tools");
        fs::create_dir(&fake_tools).unwrap();
        let fake_cargo = fake_tools.join("cargo");
        fs::write(&fake_cargo, "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"cargo 1.85.0\"; else exit 1; fi\n").unwrap();
        fs::set_permissions(&fake_cargo, fs::Permissions::from_mode(0o755)).unwrap();
        let output = run(&installer(), "1\n1\n", &home, Some(&fake_tools));
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stderr).contains("Rust build failed"));
        assert_eq!(
            fs::read(scripts.join("work")).unwrap(),
            b"previous work binary"
        );
        assert_eq!(
            fs::read(scripts.parent().unwrap().join("SKILL.md")).unwrap(),
            b"previous instructions"
        );
        assert_eq!(fs::read_dir(&scripts).unwrap().count(), 1);
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::io::{Read, Write};
    use std::process::{Command, Output, Stdio};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn workspace(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "work-installer-windows-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn run(home: &Path, rustflags: Option<&str>, default_home: bool) -> Output {
        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("USERPROFILE").unwrap()).join(".cargo")
            });
        let mut command = Command::new("cmd.exe");
        command
            .arg("/C")
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../os-scripts/windows/install-work.bat"),
            )
            .current_dir(home)
            .env("USERPROFILE", home)
            .env("CARGO_HOME", cargo_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(flags) = rustflags {
            command.env("RUSTFLAGS", flags);
        }
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take();
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let stderr_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let mut prompts = vec![(
            "Select an installation location [1]: ",
            if default_home { "1\r\n" } else { "2\r\n" }.to_owned(),
        )];
        if !default_home {
            prompts.push((
                "Enter the installation directory: ",
                format!("{}\r\n", home.display()),
            ));
        }
        prompts.push((
            "Select hierarchy numbers, enter \"all\", or press Enter for general only: ",
            "1\r\n".to_owned(),
        ));
        let mut next = 0;
        let mut output = Vec::new();
        let mut byte = [0];
        while stdout.read(&mut byte).unwrap() != 0 {
            output.push(byte[0]);
            if let Some((prompt, reply)) = prompts.get(next) {
                if output.ends_with(prompt.as_bytes()) {
                    stdin.as_mut().unwrap().write_all(reply.as_bytes()).unwrap();
                    next += 1;
                    if next == prompts.len() {
                        stdin.take();
                    }
                }
            }
        }
        Output {
            status: child.wait().unwrap(),
            stdout: output,
            stderr: stderr_reader.join().unwrap(),
        }
    }

    #[test]
    fn custom_home_installs_native_work_outside_repository() {
        let home = workspace("custom-home");
        let output = run(&home, None, false);
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let installed = home.join("skills/work");
        let binary = installed.join("scripts/work.exe");
        assert!(
            binary.is_file(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read(installed.join("SKILL.md")).unwrap(),
            fs::read(repository().join("skills/work/SKILL.md")).unwrap()
        );
        assert!(!installed.join("scripts/work.py").exists());
        let help = Command::new(binary)
            .arg("--help")
            .current_dir(repository())
            .output()
            .unwrap();
        assert!(help.status.success());
        assert!(String::from_utf8_lossy(&help.stdout).contains("usage: work"));
    }

    #[test]
    fn default_home_installs_native_work() {
        let home = workspace("default-home");
        let output = run(&home, None, true);
        assert!(
            output.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let binary = home.join(".agents/skills/work/scripts/work.exe");
        assert!(binary.is_file());
        let help = Command::new(binary)
            .arg("--help")
            .current_dir(&home)
            .output()
            .unwrap();
        assert!(help.status.success());
        assert!(String::from_utf8_lossy(&help.stdout).contains("usage: work"));
    }

    #[test]
    fn failed_build_preserves_existing_binary_and_instructions() {
        let home = workspace("build-failure");
        let installed = home.join("skills/work");
        fs::create_dir_all(installed.join("scripts")).unwrap();
        fs::write(installed.join("scripts/work.exe"), b"previous work binary").unwrap();
        fs::write(installed.join("SKILL.md"), b"previous instructions").unwrap();
        let output = run(&home, Some("-C invalid-issue55-option"), false);
        assert_eq!(output.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&output.stdout).contains("Rust build failed"));
        assert_eq!(
            fs::read(installed.join("scripts/work.exe")).unwrap(),
            b"previous work binary"
        );
        assert_eq!(
            fs::read(installed.join("SKILL.md")).unwrap(),
            b"previous instructions"
        );
    }
}
