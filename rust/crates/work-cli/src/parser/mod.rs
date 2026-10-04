//! Public command parser derived from the frozen Python command inventory.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use clap::builder::PossibleValuesParser;
use clap::{Arg, ArgAction, ArgGroup, Command, error::ErrorKind};
use serde::Deserialize;
use serde_json::{Value, json};
use work_flow::error::{ExitCode, WorkError};

#[derive(Debug, Deserialize)]
struct Manifest {
    schema: String,
    root: CommandSpec,
}

#[derive(Debug, Deserialize)]
struct CommandSpec {
    name: String,
    help: String,
    arguments: Vec<ArgumentSpec>,
    exclusive_groups: Vec<ExclusiveGroup>,
    children: Vec<CommandSpec>,
}

#[derive(Debug, Deserialize)]
struct ArgumentSpec {
    name: String,
    flags: Vec<String>,
    required: bool,
    nargs: Option<Value>,
    append: bool,
    boolean: bool,
    choices: Option<Vec<String>>,
    help: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExclusiveGroup {
    required: bool,
    members: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct ParsedCommand {
    pub path: Vec<String>,
    pub arguments: BTreeMap<String, Value>,
}

#[derive(Debug, PartialEq)]
pub enum ParseOutcome {
    Command(ParsedCommand),
    Help(String),
}

fn manifest() -> &'static Manifest {
    static MANIFEST: OnceLock<Manifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        let value: Manifest =
            serde_json::from_str(include_str!("commands.json")).expect("valid command inventory");
        assert_eq!(value.schema, "work-command-tree/v1");
        value
    })
}

fn command(node: &CommandSpec) -> Command {
    let mut result = Command::new(node.name.clone())
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("help")
                .long("help")
                .short('h')
                .action(ArgAction::Help),
        )
        .subcommand_required(!node.children.is_empty());
    for item in &node.arguments {
        let mut argument = Arg::new(item.name.clone()).required(item.required);
        for flag in &item.flags {
            if let Some(long) = flag.strip_prefix("--") {
                argument = argument.long(long.to_owned());
            } else if let Some(short) = flag.strip_prefix('-') {
                argument = argument.short(short.chars().next().expect("short flag"));
            }
        }
        if item.boolean {
            argument = argument.action(ArgAction::SetTrue);
        } else if item.append {
            argument = argument.action(ArgAction::Append);
        } else {
            argument = argument.action(ArgAction::Set);
        }
        if item.nargs.as_ref().is_some_and(|value| value == "*") {
            argument = argument.num_args(0..);
        }
        if let Some(choices) = &item.choices {
            argument = argument.value_parser(PossibleValuesParser::new(choices.clone()));
        }
        if let Some(help) = &item.help {
            argument = argument.help(help.clone());
        }
        result = result.arg(argument);
    }
    for (index, group) in node.exclusive_groups.iter().enumerate() {
        result = result.group(
            ArgGroup::new(format!("exclusive_{index}"))
                .args(group.members.clone())
                .required(group.required)
                .multiple(false),
        );
    }
    for child in &node.children {
        result = result.subcommand(command(child));
    }
    result
}

fn selected_help<'a>(root: &'a CommandSpec, tokens: &[String]) -> &'a str {
    let mut selected = root;
    for token in tokens {
        if token == "-h" || token == "--help" {
            break;
        }
        if let Some(child) = selected.children.iter().find(|child| child.name == *token) {
            selected = child;
        }
    }
    &selected.help
}

fn selected_path<'a>(root: &'a CommandSpec, tokens: &[String]) -> Vec<&'a CommandSpec> {
    let mut path = vec![root];
    for token in tokens {
        let selected = *path.last().expect("root exists");
        if let Some(child) = selected.children.iter().find(|child| child.name == *token) {
            path.push(child);
        }
    }
    path
}

fn python_missing_reason(root: &CommandSpec, tokens: &[String]) -> Option<String> {
    let path = selected_path(root, tokens);
    for node in path.iter().rev() {
        let mut missing = Vec::new();
        for argument in &node.arguments {
            if !argument.required {
                continue;
            }
            let supplied = argument.flags.iter().any(|flag| {
                tokens
                    .iter()
                    .any(|token| token == flag || token.starts_with(&format!("{flag}=")))
            });
            let positional =
                argument.flags.is_empty() && tokens.iter().any(|token| token == &argument.name);
            if !supplied && !positional {
                missing.push(
                    argument
                        .flags
                        .first()
                        .cloned()
                        .unwrap_or_else(|| argument.name.clone()),
                );
            }
        }
        if !node.children.is_empty() && node.name == path.last().expect("root exists").name {
            missing.push(if path.len() == 1 {
                "command".to_owned()
            } else {
                format!("{}_command", node.name)
            });
        }
        if !missing.is_empty() {
            return Some(format!(
                "the following arguments are required: {}",
                missing.join(", ")
            ));
        }
        for group in &node.exclusive_groups {
            if !group.required {
                continue;
            }
            let members = group
                .members
                .iter()
                .filter_map(|member| {
                    node.arguments
                        .iter()
                        .find(|argument| argument.name == *member)
                })
                .collect::<Vec<_>>();
            if !members.iter().any(|argument| {
                argument.flags.iter().any(|flag| {
                    tokens
                        .iter()
                        .any(|token| token == flag || token.starts_with(&format!("{flag}=")))
                })
            }) {
                let labels = members
                    .iter()
                    .filter_map(|argument| argument.flags.first())
                    .cloned()
                    .collect::<Vec<_>>();
                return Some(format!(
                    "one of the arguments {} is required",
                    labels.join(" ")
                ));
            }
        }
    }
    None
}

fn selected_arguments<'a>(root: &'a CommandSpec, tokens: &[String]) -> Vec<&'a ArgumentSpec> {
    selected_path(root, tokens)
        .into_iter()
        .flat_map(|node| node.arguments.iter())
        .collect()
}

fn python_choice_reason(root: &CommandSpec, tokens: &[String]) -> Option<String> {
    for argument in selected_arguments(root, tokens) {
        let Some(choices) = &argument.choices else {
            continue;
        };
        for (index, token) in tokens.iter().enumerate() {
            for flag in &argument.flags {
                let value = if token == flag {
                    tokens.get(index + 1).map(String::as_str)
                } else {
                    token.strip_prefix(&format!("{flag}="))
                };
                if let Some(value) =
                    value.filter(|value| !choices.iter().any(|choice| choice == *value))
                {
                    let quoted = choices
                        .iter()
                        .map(|choice| format!("'{choice}'"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Some(format!(
                        "argument {flag}: invalid choice: '{value}' (choose from {quoted})"
                    ));
                }
            }
        }
    }
    None
}

fn python_conflict_reason(root: &CommandSpec, tokens: &[String]) -> Option<String> {
    for node in selected_path(root, tokens) {
        for group in &node.exclusive_groups {
            let mut supplied = group
                .members
                .iter()
                .filter_map(|member| {
                    node.arguments
                        .iter()
                        .find(|argument| argument.name == *member)
                })
                .flat_map(|argument| argument.flags.iter())
                .filter_map(|flag| {
                    tokens
                        .iter()
                        .position(|token| token == flag || token.starts_with(&format!("{flag}=")))
                        .map(|position| (position, flag))
                })
                .collect::<Vec<_>>();
            supplied.sort_by_key(|(position, _)| *position);
            if supplied.len() > 1 {
                return Some(format!(
                    "argument {}: not allowed with argument {}",
                    supplied[1].1, supplied[0].1
                ));
            }
        }
    }
    None
}

fn python_unknown_reason(error: &clap::Error) -> Option<String> {
    let message = error.to_string();
    let token = message.split('\'').nth(1)?;
    Some(format!("unrecognized arguments: {token}"))
}

fn python_usage_reason(root: &CommandSpec, tokens: &[String], error: &clap::Error) -> String {
    let translated = match error.kind() {
        ErrorKind::MissingRequiredArgument | ErrorKind::MissingSubcommand => {
            python_missing_reason(root, tokens)
        }
        ErrorKind::UnknownArgument => {
            python_missing_reason(root, tokens).or_else(|| python_unknown_reason(error))
        }
        ErrorKind::InvalidValue => python_choice_reason(root, tokens),
        ErrorKind::ArgumentConflict => python_conflict_reason(root, tokens),
        _ => None,
    };
    translated.unwrap_or_else(|| error.to_string().trim().to_owned())
}

fn collect(node: &CommandSpec, matches: &clap::ArgMatches, output: &mut ParsedCommand) {
    for item in &node.arguments {
        let value = if item.boolean {
            Some(json!(matches.get_flag(&item.name)))
        } else if item.append || item.nargs.as_ref().is_some_and(|value| value == "*") {
            matches
                .get_many::<String>(&item.name)
                .map(|values| json!(values.cloned().collect::<Vec<_>>()))
        } else {
            matches
                .get_one::<String>(&item.name)
                .map(|value| json!(value))
        };
        if let Some(value) = value {
            output.arguments.insert(item.name.clone(), value);
        }
    }
    if let Some((name, child_matches)) = matches.subcommand() {
        let child = node
            .children
            .iter()
            .find(|child| child.name == name)
            .expect("known subcommand");
        output.path.push(name.to_owned());
        collect(child, child_matches, output);
    }
}

pub fn parse_tokens(tokens: &[String]) -> Result<ParseOutcome, WorkError> {
    if tokens
        .iter()
        .any(|token| token == "--stdin" || token.starts_with("--stdin="))
    {
        return Err(WorkError::new(
            ExitCode::CliUsage,
            "stdin_removed",
            "Write the JSON request to a UTF-8 file and use --input-file <path>.",
            json!({"replacement":"--input-file"}),
        ));
    }
    let root = &manifest().root;
    let result = command(root)
        .try_get_matches_from(std::iter::once("work".to_owned()).chain(tokens.iter().cloned()));
    match result {
        Ok(matches) => {
            let mut parsed = ParsedCommand {
                path: Vec::new(),
                arguments: BTreeMap::new(),
            };
            collect(root, &matches, &mut parsed);
            Ok(ParseOutcome::Command(parsed))
        }
        Err(error) if error.kind() == ErrorKind::DisplayHelp => {
            Ok(ParseOutcome::Help(selected_help(root, tokens).to_owned()))
        }
        Err(error) => Err(WorkError::new(
            ExitCode::CliUsage,
            "cli_usage_error",
            "The CLI arguments are invalid.",
            json!({"reason":python_usage_reason(root, tokens, &error)}),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(node: &CommandSpec) -> usize {
        if node.children.is_empty() {
            1
        } else {
            node.children.iter().map(leaves).sum()
        }
    }

    #[test]
    fn command_manifest_keeps_all_python_public_leaves() {
        assert_eq!(leaves(&manifest().root), 88);
        command(&manifest().root).debug_assert();
    }

    #[test]
    fn specification_lifecycle_is_separate_from_task() {
        for (name, approval) in [
            ("prepare", false),
            ("preview", false),
            ("apply", true),
            ("verify", false),
            ("recover", true),
        ] {
            let mut args = vec![
                "--project-root",
                "/project",
                "specification",
                name,
                "--input-file",
                "request.json",
                "--user-config-root",
                "/config",
            ];
            if approval {
                args.extend([
                    "--approved-sha256",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ]);
            }
            let ParseOutcome::Command(parsed) =
                parse_tokens(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).unwrap()
            else {
                panic!("expected specification command");
            };
            assert_eq!(parsed.path, ["specification", name]);
        }
        for name in [
            "spec-prepare",
            "spec-validate",
            "spec-update",
            "spec-verify",
            "spec-recover",
        ] {
            let args = ["--project-root", "/project", "task", name, "--help"].map(str::to_owned);
            assert_eq!(
                parse_tokens(&args).unwrap_err().exit_code,
                ExitCode::CliUsage
            );
        }
    }

    #[test]
    fn handoff_parser_requires_sources_targets_and_explicit_return_context() {
        let parse = |suffix: &[&str]| {
            let tokens = [&["--project-root", "/project", "handoff"][..], suffix]
                .concat()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            parse_tokens(&tokens)
        };
        for command in ["validate", "render"] {
            let ParseOutcome::Command(parsed) =
                parse(&[command, "--input-file", "request.json"]).unwrap()
            else {
                panic!("expected handoff command");
            };
            assert_eq!(parsed.arguments["input_file"], "request.json");
            assert_eq!(
                parse(&[command]).unwrap_err().reason_code,
                "cli_usage_error"
            );
        }
        for arguments in [
            vec!["build-plan-to-task"],
            vec!["build-plan-to-task", "--input-file", "request.json"],
            vec![
                "build-plan-to-task",
                "--input-file",
                "request.json",
                "--plan-path",
                "plan.json",
            ],
            vec![
                "build-plan-to-task",
                "--plan-path",
                "plan.json",
                "--user-config-root",
                "/config",
            ],
            vec!["verify-plan-to-task"],
            vec![
                "verify-plan-to-task",
                "--input-file",
                "request.json",
                "--plan-path",
                "plan.json",
            ],
            vec![
                "build-task-to-execute",
                "--input-file",
                "request.json",
                "--user-config-root",
                "/config",
            ],
            vec![
                "build-task-to-execute",
                "--input-file",
                "request.json",
                "--user-config-root",
                "/config",
                "--task-path",
                "task.json",
            ],
            vec![
                "build-task-to-plan",
                "--input-file",
                "request.json",
                "--user-config-root",
                "/config",
            ],
        ] {
            assert_eq!(
                parse(&arguments).unwrap_err().reason_code,
                "cli_usage_error",
                "{arguments:?}"
            );
        }
        let common = [
            "--input-file",
            "request.json",
            "--user-config-root",
            "/config",
            "--task-path",
            "task.json",
            "--task-id",
            "TASK-001",
        ];
        for command in ["build-execute-to-task", "verify-execute-to-task"] {
            let mut arguments = vec![command];
            arguments.extend(common);
            assert_eq!(
                parse(&arguments).unwrap_err().reason_code,
                "cli_usage_error",
                "{command}"
            );
            for context in [["--preflight", ""], ["--attempt-id", "ATTEMPT-001"]] {
                let mut selected = arguments.clone();
                selected.push(context[0]);
                if !context[1].is_empty() {
                    selected.push(context[1]);
                }
                let ParseOutcome::Command(parsed) = parse(&selected).unwrap() else {
                    panic!("expected {command}");
                };
                assert_eq!(parsed.arguments["task_id"], "TASK-001");
                assert_eq!(parsed.arguments["task_path"], "task.json");
                assert!(!parsed.arguments.contains_key("plan_path"));
                if context[0] == "--preflight" {
                    assert_eq!(parsed.arguments["preflight"], true);
                } else {
                    assert_eq!(parsed.arguments["attempt_id"], "ATTEMPT-001");
                }
            }
            let mut both = arguments;
            both.extend(["--preflight", "--attempt-id", "ATTEMPT-001"]);
            assert_eq!(parse(&both).unwrap_err().reason_code, "cli_usage_error");
        }
    }

    #[test]
    fn parses_global_arguments_and_exclusive_input_sources() {
        let tokens = [
            "--project-root",
            "/project",
            "--verbose",
            "attempt",
            "validate",
            "--path",
            "outputs/work/attempt.json",
        ]
        .map(str::to_owned);
        let ParseOutcome::Command(parsed) = parse_tokens(&tokens).unwrap() else {
            panic!("expected command");
        };
        assert_eq!(parsed.path, ["attempt", "validate"]);
        assert_eq!(parsed.arguments["project_root"], "/project");
        assert_eq!(parsed.arguments["verbose"], true);
        assert_eq!(parsed.arguments["path"], "outputs/work/attempt.json");
        assert!(!parsed.arguments.contains_key("input_file"));
        let mut conflicting = tokens.to_vec();
        conflicting.extend(["--input-file".into(), "input.json".into()]);
        assert_eq!(
            parse_tokens(&conflicting).unwrap_err().reason_code,
            "cli_usage_error"
        );
        let missing = ["--project-root", "/project", "attempt", "render"].map(str::to_owned);
        assert_eq!(
            parse_tokens(&missing).unwrap_err().reason_code,
            "cli_usage_error"
        );
        let missing_value = [
            "--project-root",
            "/project",
            "attempt",
            "render",
            "--input-file",
        ]
        .map(str::to_owned);
        assert_eq!(
            parse_tokens(&missing_value).unwrap_err().reason_code,
            "cli_usage_error"
        );
    }

    #[test]
    fn correction_validate_and_render_require_exclusive_input_file() {
        let tokens = [
            "--project-root",
            "/project",
            "correction",
            "validate",
            "--input-file",
            "request.json",
        ]
        .map(str::to_owned);
        let ParseOutcome::Command(parsed) = parse_tokens(&tokens).unwrap() else {
            panic!("expected correction validate command");
        };
        assert_eq!(parsed.path, ["correction", "validate"]);
        assert_eq!(parsed.arguments["input_file"], "request.json");
        assert!(!parsed.arguments.contains_key("path"));
        let mut conflicting = tokens.to_vec();
        conflicting.extend(["--path".into(), "correction.json".into()]);
        assert_eq!(
            parse_tokens(&conflicting).unwrap_err().reason_code,
            "cli_usage_error"
        );
        let missing = ["--project-root", "/project", "correction", "render"].map(str::to_owned);
        assert_eq!(
            parse_tokens(&missing).unwrap_err().reason_code,
            "cli_usage_error"
        );
    }

    #[test]
    fn task_parser_preserves_specification_arguments() {
        let parse_spec = |suffix: &[&str]| {
            let tokens = [&["--project-root", "/project", "specification"][..], suffix]
                .concat()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let ParseOutcome::Command(parsed) = parse_tokens(&tokens).unwrap() else {
                panic!("expected specification command");
            };
            parsed
        };
        for command in ["verify", "preview"] {
            let parsed = parse_spec(&[
                command,
                "--input-file",
                "request.json",
                "--user-config-root",
                "/config",
            ]);
            assert_eq!(parsed.path, ["specification", command]);
            assert_eq!(parsed.arguments["input_file"], "request.json");
        }
        let parsed = parse_spec(&[
            "apply",
            "--input-file",
            "request.json",
            "--user-config-root",
            "/config",
            "--approved-sha256",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ]);
        assert_eq!(parsed.path, ["specification", "apply"]);
        assert_eq!(
            parsed.arguments["approved_sha256"].as_str().unwrap().len(),
            64
        );
        for command in [
            "reconciliation-preview",
            "reconciliation-prepare",
            "reconciliation-apply",
            "reconciliation-recover",
        ] {
            let mut args = vec![
                command,
                "--input-file",
                "request.json",
                "--user-config-root",
                "/config",
            ];
            if matches!(command, "reconciliation-apply" | "reconciliation-recover") {
                args.extend([
                    "--approved-sha256",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ]);
            }
            let parsed = parse_spec(&args);
            assert_eq!(parsed.path, ["specification", command]);
            let old = [&["--project-root", "/project", "task"][..], &args]
                .concat()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            assert_eq!(
                parse_tokens(&old).unwrap_err().reason_code,
                "cli_usage_error"
            );
        }
        for command in [
            "semantic-prepare",
            "semantic-preview",
            "semantic-apply",
            "semantic-recover",
        ] {
            let tokens = ["--project-root", "/project", "migration", command].map(str::to_owned);
            assert_eq!(
                parse_tokens(&tokens).unwrap_err().reason_code,
                "cli_usage_error",
                "{command}"
            );
        }
        for command in [
            "migration-prepare",
            "migration-preview",
            "migration-apply",
            "migration-recover",
        ] {
            let tokens = ["--project-root", "/project", "task", command].map(str::to_owned);
            assert_eq!(
                parse_tokens(&tokens).unwrap_err().reason_code,
                "cli_usage_error"
            );
        }
        for command in [
            "layout-preflight",
            "layout-prepare",
            "layout-validate",
            "layout-apply",
            "layout-recover",
            "layout-verify",
            "migrate-preflight",
            "migrate-prepare",
            "migrate-validate",
            "migrate",
            "migrate-recover",
            "migrate-verify",
            "draft-init-request",
            "draft-list-prepare",
        ] {
            let tokens = ["--project-root", "/project", "task", command].map(str::to_owned);
            assert_eq!(
                parse_tokens(&tokens).unwrap_err().exit_code,
                ExitCode::CliUsage
            );
        }
    }

    #[test]
    fn task_parser_limits_summary_and_single_draft_request_inputs() {
        let parse = |suffix: &[&str]| {
            let tokens = [&["--project-root", "/project", "task"][..], suffix]
                .concat()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            parse_tokens(&tokens)
        };
        let parse_spec = |suffix: &[&str]| {
            let tokens = [&["--project-root", "/project", "specification"][..], suffix]
                .concat()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            parse_tokens(&tokens)
        };
        for command in ["prepare", "preview", "apply"] {
            let mut tokens = vec![
                command,
                "--input-file",
                "request.json",
                "--user-config-root",
                "/config",
                "--summary",
            ];
            if command == "apply" {
                tokens.extend([
                    "--approved-sha256",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ]);
            }
            let ParseOutcome::Command(parsed) = parse_spec(&tokens).unwrap() else {
                panic!("expected specification command");
            };
            assert_eq!(parsed.arguments["summary"], true);
        }
        let tokens = [
            "recover",
            "--input-file",
            "request.json",
            "--user-config-root",
            "/config",
            "--summary",
        ];
        assert_eq!(
            parse_spec(&tokens).unwrap_err().exit_code,
            ExitCode::CliUsage
        );
        assert_eq!(
            parse(&[
                "repair-prepare",
                "--input-file",
                "request.json",
                "--user-config-root",
                "/config",
                "--summary"
            ])
            .unwrap_err()
            .exit_code,
            ExitCode::CliUsage
        );
        for command in ["draft-save-request", "draft-recover-request"] {
            let base = [
                command,
                "--requirement-id",
                "example",
                "--task-id",
                "TASK-001",
                "--expected-revision",
                "1",
                "--plan-path",
                "outputs/work/plans/example.json",
                "--user-config-root",
                "/config",
            ];
            for extra in [
                &[][..],
                &["--general-only"][..],
                &[
                    "--input-file",
                    "request.json",
                    "--general-only",
                    "--instruction-path",
                    "web",
                ][..],
            ] {
                assert_eq!(
                    parse(&[base.as_slice(), extra].concat())
                        .unwrap_err()
                        .reason_code,
                    "cli_usage_error"
                );
            }
        }
    }

    #[test]
    fn task_public_commands_are_exact_and_legacy_commands_are_rejected() {
        let task = manifest()
            .root
            .children
            .iter()
            .find(|child| child.name == "task")
            .unwrap();
        let mut names = task
            .children
            .iter()
            .map(|child| child.name.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            [
                "apply", "prepare", "preview", "recover", "save", "status", "validate"
            ]
        );
        for command in [
            "draft-init",
            "semantic-prepare",
            "draft-save",
            "draft-recover",
            "draft-read",
            "draft-status",
            "draft-check",
            "draft-save-request",
            "draft-recover-request",
            "draft-list-update",
            "draft-list-recover",
            "draft-source-update",
            "draft-source-recover",
            "draft-assemble",
            "draft-create",
            "create",
            "recover-create",
        ] {
            let tokens = ["--project-root", "/project", "task", command].map(str::to_owned);
            assert_eq!(
                parse_tokens(&tokens).unwrap_err().reason_code,
                "cli_usage_error",
                "{command}"
            );
        }
        let tokens = [
            "--project-root",
            "/project",
            "task",
            "save",
            "--requirement-id",
            "example",
            "--user-config-root",
            "/config",
            "--input-file",
            "request.json",
        ]
        .map(str::to_owned);
        let ParseOutcome::Command(parsed) = parse_tokens(&tokens).unwrap() else {
            panic!("expected task save");
        };
        assert_eq!(parsed.path, ["task", "save"]);
        assert!(!parsed.arguments.contains_key("task_id"));
    }
    #[test]
    fn help_and_removed_stdin_match_public_contract() {
        let help = ["task", "--help"].map(str::to_owned);
        let ParseOutcome::Help(content) = parse_tokens(&help).unwrap() else {
            panic!("expected help");
        };
        assert!(content.starts_with("usage: work task"));
        assert!(!content.contains("draft-"));
        assert!(!content.contains("work.py"));
        for option in ["--stdin", "--stdin=true"] {
            let error = parse_tokens(&[option.into()]).unwrap_err();
            assert_eq!(error.reason_code, "stdin_removed");
            assert_eq!(error.details["replacement"], "--input-file");
        }
    }

    #[test]
    fn usage_errors_follow_argparse_precedence_and_wording() {
        let cases = [
            (
                &["--bogus"][..],
                "the following arguments are required: --project-root, command",
            ),
            (
                &["task", "--bogus"],
                "the following arguments are required: task_command",
            ),
            (
                &["attempt", "validate", "--path", "x", "--input-file", "y"],
                "argument --input-file: not allowed with argument --path",
            ),
            (
                &["delegation", "validate", "--role", "bad"],
                "argument --role: invalid choice: 'bad' (choose from 'task-coordinator', 'execute', 'task-skill', 'artifact-editor', 'progress-saver')",
            ),
        ];
        for (arguments, expected) in cases {
            let tokens = arguments
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>();
            assert_eq!(
                parse_tokens(&tokens).unwrap_err().details["reason"],
                expected
            );
        }
    }

    #[test]
    fn execute_parser_keeps_scope_repeated_inputs_and_command_requirements() {
        let scope = [
            "--user-config-root",
            "/config",
            "--task-path",
            "outputs/work/tasks/example/task.json",
            "--execution-dir",
            "outputs/work/executions/example",
            "--task-id",
            "TASK-001",
            "--skill-root",
            "repo:.agents/skills=/repo-skills",
            "--skill-root",
            "user:skills=/user-skills",
        ];
        let parse = |command: &str, suffix: &[&str]| {
            let tokens = [
                &["--project-root", "/project", "execute", command][..],
                &scope,
                suffix,
            ]
            .concat()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
            parse_tokens(&tokens)
        };
        let ParseOutcome::Command(preflight) = parse(
            "preflight",
            &[
                "--confirmed-input",
                "INPUT-001",
                "--confirmed-input",
                "INPUT-002",
            ],
        )
        .unwrap() else {
            panic!("expected preflight command");
        };
        assert_eq!(preflight.path, ["execute", "preflight"]);
        assert_eq!(preflight.arguments["project_root"], "/project");
        assert_eq!(preflight.arguments["user_config_root"], "/config");
        assert_eq!(
            preflight.arguments["skill_root"],
            json!([
                "repo:.agents/skills=/repo-skills",
                "user:skills=/user-skills"
            ])
        );
        assert_eq!(
            preflight.arguments["confirmed_input"],
            json!(["INPUT-001", "INPUT-002"])
        );
        for command in [
            "attempt-start",
            "attempt-start-prepare",
            "deviation-prepare-semantic",
        ] {
            let ParseOutcome::Command(parsed) =
                parse(command, &["--input-file", "request.json"]).unwrap()
            else {
                panic!("expected {command} command");
            };
            assert_eq!(parsed.path, ["execute", command]);
            assert_eq!(parsed.arguments["input_file"], "request.json");
            assert!(!parsed.arguments.contains_key("confirmed_input"));
            assert_eq!(
                parse(command, &[]).unwrap_err().reason_code,
                "cli_usage_error"
            );
        }
        let removed =
            ["--project-root", "/project", "execute", "deviation-prepare"].map(str::to_owned);
        assert_eq!(
            parse_tokens(&removed).unwrap_err().reason_code,
            "cli_usage_error"
        );
        let ParseOutcome::Command(record) = parse(
            "deviation-record",
            &[
                "--input-file",
                "request.json",
                "--approved-sha256",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "--authorization-evidence",
                "User approved this exact deviation preview.",
            ],
        )
        .unwrap() else {
            panic!("expected deviation-record command");
        };
        assert_eq!(record.arguments["approved_sha256"], "a".repeat(64));
        assert_eq!(
            record.arguments["authorization_evidence"],
            "User approved this exact deviation preview."
        );
        let ParseOutcome::Command(begin) =
            parse("record-begin", &["--record-id", "RECORD-001"]).unwrap()
        else {
            panic!("expected record-begin command");
        };
        assert_eq!(begin.arguments["record_id"], "RECORD-001");
    }
    #[test]
    fn invocation_confirm_requires_input_and_has_no_origin_override() {
        assert!(
            parse_tokens(&[
                "--project-root".into(),
                ".".into(),
                "invocation".into(),
                "confirm".into()
            ])
            .is_err()
        );
        assert!(
            parse_tokens(&[
                "--project-root".into(),
                ".".into(),
                "invocation".into(),
                "confirm".into(),
                "--input-file".into(),
                "request.json".into(),
                "--origin".into(),
                "explicit".into()
            ])
            .is_err()
        );
    }
}
