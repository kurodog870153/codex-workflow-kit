"""Preview and execute one reserved argv CMD; never retry or finish its record."""
from __future__ import annotations

import json
import os
import platform
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

from .commands import formal_command
from ..artifacts.task_collection import load_task_execution_context
from .context import validate_execution_identity
from .instructions import validate_execute_instructions
from .records import next_record_id
from .recovery import _validate_attempt_bytes, _validate_index_bytes
from ..contracts.command_correction import canonicalize_command_correction
from ..contracts.task import validate_task_contract
from ..contracts.validation import nonempty_string, sha256, strict_keys
from ..foundation.errors import ExitCode, WorkError
from ..foundation.fingerprint import raw_sha256, read_raw
from ..foundation.markdown import parse_json_contract
from ..foundation.paths import normalize_relative_path
from ..foundation.spec_update import require_idle_writer, require_no_spec_update, state_writer, storage_path


def _json(value):
    return (json.dumps(value, ensure_ascii=True, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()


def _fail(code, message, **details):
    raise WorkError(ExitCode.WORKFLOW_STATE, code, message, details)


def _prepare(raw, *, source, project_root, user_config_root, raw_task_path, raw_execution_dir, task_id, skill_roots=None):
    request = strict_keys(parse_json_contract(raw, source=source), location="command_run", required={
        "schema", "attempt_id", "record_id", "timeout_seconds"})
    if request["schema"] != "work-command-run-request/v1":
        _fail("command_run_schema", "Use work-command-run-request/v1.")
    for field, pattern in (("attempt_id", r"ATTEMPT-[0-9]{3}"), ("record_id", r"CMD-[0-9]{3}(?:#[1-9][0-9]*)?")):
        if not isinstance(request[field], str) or not re.fullmatch(pattern, request[field]):
            _fail("command_run_identity", "Supply explicit canonical Attempt and reserved CMD IDs.")
    timeout = request["timeout_seconds"]
    if type(timeout) is not int or not 1 <= timeout <= 3600:
        _fail("command_run_timeout", "timeout_seconds must be an integer from 1 to 3600.")
    if not isinstance(task_id, str) or not re.fullmatch(r"TASK-[0-9]{3}", task_id):
        _fail("command_run_identity", "Supply a canonical TASK ID.")
    execution = normalize_relative_path(raw_execution_dir, field="execution_dir")
    task_relative = normalize_relative_path(raw_task_path, field="task_path")
    require_no_spec_update(project_root, execution)
    directory = storage_path(project_root, execution)
    if any(directory.glob(".work-*.tmp")):
        _fail("command_run_pending_transaction", "Resolve pending transactions before executing a CMD.")
    observed = {}

    def snapshot(relative):
        content = read_raw(storage_path(project_root, relative))
        observed[relative] = content
        return content

    task_context = load_task_execution_context(
        project_root,
        user_config_root,
        task_relative,
        task_id,
        skill_roots=skill_roots,
    )
    contract = task_context["contract"]
    validation = task_context["validation"]
    sources = task_context["sources"]
    assert isinstance(contract, dict) and isinstance(validation, dict)
    assert isinstance(sources, dict)
    observed.update(sources)
    artifacts = contract.get("artifacts", {})
    if not isinstance(artifacts, dict) or artifacts.get("task") != task_relative or artifacts.get("execution") != execution:
        _fail("command_run_paths", "Explicit paths must match the formal TASK.")
    plan_raw = snapshot(artifacts["plan"])
    task = next((row for row in contract["tasks"] if row["id"] == task_id), None)
    if task is None:
        _fail("command_run_identity", "Unknown TASK ID.")
    index_relative = execution + "/index.json"
    index = _validate_index_bytes(snapshot(index_relative), source=index_relative)
    attempt_id, record_id = request["attempt_id"], request["record_id"]
    attempt_directory = f"{execution}/{task_id}/{attempt_id}"
    attempt_relative = attempt_directory + "/attempt.json"
    attempt = _validate_attempt_bytes(snapshot(attempt_relative), project_root=project_root, source=attempt_relative)
    row = validate_execution_identity(task_contract=contract, task_validation=validation,
        index=index, attempt=attempt, task_id=task_id)
    lock = index.get("lock")
    expected = {"kind": "execution", "task_id": task_id, "attempt_id": attempt_id,
                "record_id": record_id, "execute_instructions_sha256": attempt["execute_instructions_sha256"]}
    if (attempt["attempt_id"] != attempt_id or attempt["status"] != "in_progress" or row["status"] != "in_progress"
        or row.get("latest_attempt") != attempt_id or not isinstance(lock, dict)
        or any(lock.get(key) != value for key, value in expected.items())):
        _fail("command_run_lock", "The active Attempt and lock must reserve exactly the requested CMD.")
    base_id = record_id.split("#", 1)[0]
    if next_record_id(base_id, attempt) != record_id:
        _fail("command_run_sequence", "The reserved CMD is not the next record instance.")
    validate_execute_instructions(task, attempt, operation="command_run")
    command = formal_command(task, base_id)
    if "command_correction" in lock:
        correction = canonicalize_command_correction(lock["command_correction"])
        if correction["original_command"] != command:
            _fail("command_run_correction", "The saved correction differs from the formal CMD.")
        command = correction["actual_command"]
    if command["mode"] != "argv":
        _fail("command_run_argv_only", "This executor supports argv CMDs only.")
    if any("\x00" in arg for arg in command["argv"]):
        _fail("command_run_argv", "argv cannot contain NUL bytes.")
    formal = next(item for item in task["commands"] if item["id"] == base_id)
    settings = formal.get("execution", contract["execution_defaults"])
    actual_os = {"Darwin": "macos", "Linux": "linux", "Windows": "windows"}.get(platform.system())
    if settings["os"] != actual_os:
        _fail("command_run_os", "The CMD execution OS differs from the current runtime.")
    cwd = project_root if settings["working_directory"] == "." else storage_path(project_root, settings["working_directory"])
    if not cwd.is_dir():
        _fail("command_run_cwd", "The specified working directory does not exist.")
    executable = command["argv"][0]
    if "/" in executable or "\\" in executable:
        selected = Path(executable)
        if not selected.is_absolute():
            selected = cwd / selected
    else:
        search_path = os.pathsep.join(str(Path(entry) if Path(entry).is_absolute() else cwd / entry)
                                     for entry in os.get_exec_path())
        located = shutil.which(executable, path=search_path)
        if located is None:
            _fail("command_run_executable", "The approved executable is not available on PATH.")
        selected = Path(located)
    # Preserve the symlink name so venv interpreters retain their environment.
    selected = selected.absolute()
    if not selected.is_file() or (os.name != "nt" and not os.access(selected, os.X_OK)):
        _fail("command_run_executable", "The selected executable is not runnable.")
    if any(path.suffix.lower() in {".bat", ".cmd"} for path in (selected, selected.resolve())):
        _fail("command_run_argv_only", "Batch files require shell handling and are outside this argv executor.")
    safe_id = record_id.replace("#", "-retry-")
    receipt = attempt_directory + f"/.work-command-{safe_id}"
    for suffix in (".started.json", ".finished.json"):
        if storage_path(project_root, receipt + suffix).exists():
            _fail("command_run_already_started", "This record already has execution evidence; never run it again.", receipt=receipt)
    if any(read_raw(storage_path(project_root, path)) != content for path, content in observed.items()):
        _fail("command_run_source_changed", "A command source changed during preparation.")
    preview = {"schema": "work-command-preview/v1", "request": request, "task_id": task_id,
        "argv": command["argv"], "working_directory": str(cwd), "execution": settings,
        "selected_executable": str(selected), "executable_sha256": raw_sha256(read_raw(selected)),
        "receipt_prefix": receipt, "sources": {path: raw_sha256(content) for path, content in observed.items()}}
    preview["approved_sha256"] = raw_sha256(_json(preview))
    return preview


def prepare_command(raw, **options):
    require_idle_writer(options["project_root"], options["raw_execution_dir"])
    return _prepare(raw, **options)


def _write_receipt(path, value):
    with path.open("xb") as stream:
        stream.write(_json(value))
        stream.flush()
        os.fsync(stream.fileno())


def _execute(argv, cwd, timeout):
    # Spool output to temporary files so child output cannot exhaust RAM. Only
    # the last 4096 bytes of each stream are returned as untrusted diagnostic data.
    with tempfile.TemporaryFile() as stdout, tempfile.TemporaryFile() as stderr:
        try:
            process = subprocess.run(argv, cwd=cwd, shell=False, stdin=subprocess.DEVNULL,
                stdout=stdout, stderr=stderr, timeout=timeout, check=False)
            result = {"status": "exited", "exit_code": process.returncode}
        except subprocess.TimeoutExpired:
            result = {"status": "timed_out", "exit_code": None}
        except OSError:
            result = {"status": "launch_failed", "exit_code": None}
        for name, stream in (("stdout", stdout), ("stderr", stderr)):
            size = stream.seek(0, os.SEEK_END)
            stream.seek(max(0, size - 4096))
            result[name + "_tail"] = stream.read().decode("utf-8", errors="replace")
            result[name + "_truncated"] = size > 4096
        return result


def run_command(raw, *, approved_sha256, authorization_evidence, **options):
    sha256(approved_sha256, location="approved_sha256")
    nonempty_string(authorization_evidence, location="authorization_evidence")
    root = options["project_root"]
    with state_writer(root, options["raw_execution_dir"]):
        preview = _prepare(raw, **options)
        if preview["approved_sha256"] != approved_sha256:
            _fail("command_run_approval_changed", "Sources or command parameters changed after review.")
        prefix = preview["receipt_prefix"]
        started = {"schema": "work-command-started/v1", "preview": preview,
                   "authorization_evidence": authorization_evidence}
        try:
            _write_receipt(storage_path(root, prefix + ".started.json"), started)
            result = _execute([preview["selected_executable"], *preview["argv"][1:]],
                              preview["working_directory"], preview["request"]["timeout_seconds"])
            receipt = {"schema": "work-command-result/v1", "approved_sha256": approved_sha256,
                       "record_id": preview["request"]["record_id"], **result}
            _write_receipt(storage_path(root, prefix + ".finished.json"), receipt)
        except (OSError, KeyboardInterrupt) as error:
            raise WorkError(ExitCode.IO_FAILURE, "command_run_interrupted",
                "Preserve command evidence and inspect effects; do not rerun this record.", {"receipt_prefix": prefix}) from error
    response = {**receipt, "receipt_prefix": prefix, "record_finish_required": True}
    if result["status"] == "exited":
        response["record_finish_request"] = {"schema": "work-record-finish-request/v1", "record": {
            "id": receipt["record_id"], "kind": "command", "exit_code": result["exit_code"],
            "result": f"Command exited with code {result['exit_code']}; inspect retained execution evidence."}}
    if result["status"] != "exited" or result["exit_code"] != 0:
        raise WorkError(ExitCode.WORKFLOW_STATE, "command_run_failed",
            "The command did not succeed. Review evidence and effects before continuing; never retry automatically.", response)
    return response
