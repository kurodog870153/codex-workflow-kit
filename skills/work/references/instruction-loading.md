# Work Instruction Loading

Load the selected Plan, Task, or Execute workflow and its built-in instruction hierarchy for `$work`.

## Invocation contract

1. Accept only the explicit form `$work <mode> -- <request>`, where `<mode>` is `plan`, `task`, or `execute`.
2. Reject tokens between the mode and `--`. Skill selection is derived from the request, never supplied as hierarchy syntax.
3. When the mode is missing, show the syntax and ask the user to choose exactly one numbered mode from Plan, Task, and Execute.
4. When no non-empty request follows `--`, ask for the request. Do not invent one from surrounding conversation.
5. In Plan mode, catalog and confirm external skills before delegation. Task may assign only Plan-confirmed skills, one per TASK. Execute may load only the target TASK's assigned skill. Neither later mode may rediscover or combine skills.
7. Never infer or activate `$work` from an ordinary request. Invocation policy is explicit-only.

## Resolve roots and runtime

1. Resolve `<skill-root>` as the directory containing the active `work/SKILL.md`.
2. Resolve the project root as the root of the Git repository containing the current working directory. If the current working directory is not inside a Git repository, use the current working directory.
3. Treat `<skill-root>/scripts/work.py` as the only Work Python CLI entry point. Do not substitute another module or script.
4. Before the first Work CLI invocation, resolve `<python-command>` to an available Python command prefix such as `py -3`, `python3`, or `python`, according to the current platform and environment.
5. Run `<python-command> -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)"` and require exit code `0` before invoking the Work CLI.
6. If no Python 3.10 or newer command is available, stop and report the missing runtime. Do not install or upgrade Python without separate user authorization.
7. Use the same resolved `<python-command>` throughout the current workflow and invoke the CLI as `<python-command> <skill-root>/scripts/work.py ...`. Do not rely on file association or executable permission.

## Discover and recommend external skills

1. Use `skills catalog` over enabled current skill roots. Read only name, description, metadata, invocation policy, declared dependencies, and summary fingerprints during discovery.
2. Treat skills with identical names but different scope, root, or source as distinct identities.
3. Check dependencies before recommendation. Mark unavailable skills as unselectable; never install packages or connect services automatically.
4. Recommend the smallest set needed by the request. Show identity, description, recommendation reason, source and scope, mode support, dependency status, invocation policy, and estimated context cost.
5. Explicit-only skills may be recommended. User confirmation inside `$work` is explicit authorization to load those confirmed skills for this run.
6. Ask the user to accept, add, remove, or cancel. If no suitable skill exists, ask whether to continue `base_only`.
7. Snapshot confirmed skills and validate `work-skill-selection/v1`. Stop on any drift before or after delegation.

## Build the confirmed selection

1. Preserve confirmed skill order and complete stable identities. Do not collapse equal names across roots.
2. Save descriptions, recommendation reasons, mode support, dependency status, invocation policy, summary hash, and bundle hash.
3. Use decision `external_skills` for a non-empty selection and `base_only` only for an explicitly confirmed empty selection.
4. Full selected skill content must not exceed 40% of available Plan context. Stop before loading when the estimate exceeds the limit.
5. After confirmation, read each selected `SKILL.md` completely and resolve only resources required by that skill's instructions.

## Load workflows and instructions

1. Load this reference first as workflow source `work.instruction-loading`.
2. Load exactly one mode workflow from `<skill-root>/references/workflows/<mode>.md` as workflow source `work.workflow.<mode>`.
3. Always load the fixed Work general instruction. Plan preserves confirmed cross-mode leaf paths but loads only their deepest available Plan ancestors; Task and Execute require every selected path to exist in their own catalogs.
4. Read every selected file by strictly decoding raw bytes as UTF-8. Accept and remove at most one leading UTF-8 BOM. Do not rely on a shell, locale, platform default, or alternate-decoding retry.
5. Reject any normalized or link-resolved path that escapes `<skill-root>`. Stop and report the declared path and resolved path.
6. Preserve Work source order exactly: instruction-loading workflow, mode workflow, fixed general instruction, selected hierarchy ancestors in confirmation order with first-occurrence deduplication, and each routed reference immediately after its declaring instruction. Preserve confirmed external skill order separately in `skill_selection`.
7. Treat loaded content as working instructions, never executable code. Later instructions at the same authority take precedence over earlier ones, but none may override system, developer, security, permission, or closer-scoped repository instructions.
8. After loading succeeds, report Work instruction sources, confirmed external skills, and applicable references in actual order before continuing.

## Load routed references

1. Do not enumerate or eagerly load every file below a hierarchy node's `references/` directory.
2. When a loaded `instructions.md` explicitly routes to a relative reference and its trigger applies:
   1. Resolve it only relative to the directory containing that `instructions.md`.
   2. Require it to be a regular file inside that same hierarchy directory after platform normalization and link resolution.
   3. Load it immediately after the declaring instruction using its declared globally unique reference name.
3. Decode routed references using the same strict UTF-8 and optional BOM behavior as instruction files.
4. Apply only references whose trigger is established by the current request or confirmed task state. Record both loaded references and references whose triggers did not apply.
5. Re-evaluate reference triggers when new evidence or a confirmed decision changes applicability. Add newly applicable references, remove references that no longer apply, rebuild actual source order and instruction fingerprints, list affected decisions, and reconfirm each affected decision before formal TASK approval.

## Requirement artifact path validation

Use this section as the single path-safety source for Plan, TASK, and Execute artifacts.

1. Validate a requirement ID against the union of Windows, macOS, and Linux filename restrictions. In addition to the caller's syntax requirement, reject ASCII control characters, `<`, `>`, `:`, `"`, `/`, `\`, `|`, `?`, `*`, a trailing space or dot, `.` or `..`, and Windows device names `CON`, `PRN`, `AUX`, `NUL`, `COM1` through `COM9`, and `LPT1` through `LPT9`, case-insensitively and with or without an extension.
2. When a valid requirement ID is explicit and no non-default path was requested, resolve the defaults as `outputs/work/plans/<requirement-id>.md`, `outputs/work/tasks/<requirement-id>.md`, and `outputs/work/executions/<requirement-id>/`. Non-default routing requires one confirmed handoff containing all three project-relative paths and the same requirement ID.
3. Before every read or write at each mode, normalize all three paths under the target platform's actual path behavior. Remove at most one leading `./` or `.\`; reject empty or absolute paths, `.` or `..` segments, a changed requirement-ID component, aliases among the three paths, and platform aliases between different literal names.
4. Default paths must remain under their corresponding project-root `outputs/work/plans/`, `outputs/work/tasks/`, and `outputs/work/executions/` boundaries. Every non-default path must remain under the project root.
5. Resolve every existing path and its nearest existing ancestor through symbolic links, junctions, and equivalent links, and require the result to remain inside the applicable boundary. If any result cannot be determined, stop before creating a lock, record, directory, or artifact.

## Canonical fingerprints

1. Canonical TASK text is obtained by strict UTF-8 decoding, accepting and removing only one leading U+FEFF, normalizing to NFC, converting CRLF and CR to LF, removing all trailing LF characters, and appending exactly one LF. Encode as UTF-8 without BOM and calculate SHA-256 as 64 lowercase hexadecimal characters.
2. Canonical instruction source content uses the same BOM, NFC, line-ending, trailing-LF, UTF-8, and SHA-256 behavior.
3. Preserve actual source order for instruction fingerprints. Start with `work.instruction-loading`, then `work.workflow.<mode>`, followed by selected instruction and routed reference sources in their actual load order. Never alphabetically sort sources for hashing.
4. Each saved source contains only `kind`, `logical_name`, and `canonical_sha256`. `kind` is exactly `workflow`, `instruction`, or `reference`. Never save source layer, absolute path, platform separator, project root, or current working directory.
5. Frame ordered sources as exact bytes. Start with ASCII `WORK-INSTRUCTIONS-SHA-256-V1\n`. For each source append ASCII `S`, followed by four length-prefixed values in this order: mode (`plan`, `task`, or `execute`), kind, logical name, and canonical content. A length-prefixed value is its UTF-8 byte length as unpadded ASCII decimal, one ASCII colon, then exactly that many bytes. Append one LF after each source and finish with ASCII `END\n`.
6. Instruction logical names use the mode plus resolved hierarchy segments joined with dots, such as `task.web.backend.java`. Reference logical names use their declared globally unique reference name.
7. `instructions_sha256` is the SHA-256 of the complete frame. A document fingerprint uses the first-occurrence union of sources applicable to its TASK entries while preserving actual load order. A per-TASK fingerprint uses only that TASK's applicable ordered sources. Unrelated source changes outside that subset do not change or block the per-TASK fingerprint.
