<!-- work-compatibility-revision: 1 -->
# Requirement artifact path validation


Use this section as the single path-safety source for Plan, TASK, and Execute artifacts.

1. Validate a requirement ID against the union of Windows, macOS, and Linux filename restrictions. In addition to the caller's syntax requirement, reject ASCII control characters, `<`, `>`, `:`, `"`, `/`, `\`, `|`, `?`, `*`, a trailing space or dot, `.` or `..`, and Windows device names `CON`, `PRN`, `AUX`, `NUL`, `COM1` through `COM9`, and `LPT1` through `LPT9`, case-insensitively and with or without an extension.
2. When a valid requirement ID is explicit and no non-default path was requested, resolve the defaults as `outputs/work/plans/<requirement-id>.json`, `outputs/work/tasks/<requirement-id>/index.json`, and `outputs/work/executions/<requirement-id>/`. Non-default routing requires one confirmed handoff containing all three project-relative paths and the same requirement ID. The formal TASK entry filename must be `index.json` directly inside its requirement-ID directory, including non-default routing; reject `task.json`, flat `<requirement-id>.json` paths, and all legacy `.md` artifact paths.
3. Before every read or write at each mode, normalize all three paths under the target platform's actual path behavior. Remove at most one leading `./` or `.\`; reject empty or absolute paths, `.` or `..` segments, a changed requirement-ID component, aliases among the three paths, and platform aliases between different literal names.
4. Default paths must remain under their corresponding project-root `outputs/work/plans/`, `outputs/work/tasks/`, and `outputs/work/executions/` boundaries. Every non-default path must remain under the project root.
5. Resolve every existing path and its nearest existing ancestor through symbolic links, junctions, and equivalent links, and require the result to remain inside the applicable boundary. If any result cannot be determined, stop before creating a lock, record, directory, or artifact.
