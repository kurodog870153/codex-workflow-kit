#!/usr/bin/env python3
"""Measure Rust CI phases and publish comparable performance reports."""

import argparse
import hashlib
import io
import json
import math
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
import zipfile


SCHEMA_VERSION = 1
CONFIG_ID = "nextest-0.9.148-workspace-locked-ci-doc-v1"
PHASES = ("compile", "test", "doc")
DOC_RESULT = re.compile(r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored")


def write_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def measure(args):
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        raise ValueError("measure requires a command")
    log_path = Path(args.directory) / (args.phase + ".log")
    log_path.parent.mkdir(parents=True, exist_ok=True)
    started = time.time()
    tick = time.monotonic()
    code = 127
    with log_path.open("w", encoding="utf-8", errors="replace") as log:
        try:
            process = subprocess.Popen(
                command,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                encoding="utf-8",
                errors="replace",
                bufsize=1,
            )
            assert process.stdout is not None
            with process.stdout:
                for line in process.stdout:
                    log.write(line)
                    log.flush()
                    console_encoding = sys.stdout.encoding or "utf-8"
                    sys.stdout.write(line.encode(console_encoding, errors="backslashreplace").decode(console_encoding))
                    sys.stdout.flush()
            code = process.wait()
        except OSError as error:
            message = f"Could not start {command[0]}: {error}\n"
            log.write(message)
            print(message, end="", file=sys.stderr)
    duration = round(time.monotonic() - tick, 3)
    write_json(
        Path(args.directory) / (args.phase + ".json"),
        {
            "phase": args.phase,
            "command": command,
            "started_unix_seconds": round(started, 3),
            "duration_seconds": duration,
            "exit_code": code,
        },
    )
    return code if code >= 0 else 128 - code


def load_phase(directory, name):
    path = Path(directory) / (name + ".json")
    if not path.exists():
        return {"phase": name, "status": "missing", "duration_seconds": None, "exit_code": None}
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        if data["phase"] != name or data["duration_seconds"] < 0:
            raise ValueError("invalid phase content")
        return {**data, "status": "passed" if data["exit_code"] == 0 else "failed"}
    except (ValueError, KeyError, TypeError) as error:
        return {"phase": name, "status": "invalid", "duration_seconds": None, "exit_code": None, "error": str(error)}


def parse_junit(path):
    path = Path(path)
    if not path.is_file():
        return [], "JUnit report missing"
    try:
        root = ET.parse(path).getroot()
        tests = []
        for suite in root.iter("testsuite"):
            suite_name = suite.get("name", "")
            for case in suite.findall("testcase"):
                name = case.get("name", "")
                identifier = f"{suite_name}::{case.get('classname', '')}::{name}"
                seconds = float(case.get("time", "0"))
                if not math.isfinite(seconds) or seconds < 0 or not name:
                    raise ValueError("invalid JUnit testcase")
                outcome = "failed" if case.find("failure") is not None or case.find("error") is not None else "skipped" if case.find("skipped") is not None else "passed"
                tests.append({"id": identifier, "duration_seconds": seconds, "outcome": outcome})
        if not tests:
            raise ValueError("JUnit report contains no tests")
        return tests, None
    except (ET.ParseError, ValueError, OSError) as error:
        return [], f"JUnit report invalid: {error}"


def parse_doc_results(path):
    path = Path(path)
    if not path.is_file():
        return None
    matches = DOC_RESULT.findall(path.read_text(encoding="utf-8", errors="replace"))
    if not matches:
        return None
    return {
        "passed": sum(int(match[1]) for match in matches),
        "failed": sum(int(match[2]) for match in matches),
        "ignored": sum(int(match[3]) for match in matches),
    }


def build_report(directory, junit, metadata):
    phases = {name: load_phase(directory, name) for name in PHASES}
    tests, junit_error = parse_junit(junit)
    doc_results = parse_doc_results(Path(directory) / "doc.log")
    diagnostics = []
    if junit_error:
        diagnostics.append(junit_error)
    if doc_results is None:
        diagnostics.append("Doc test results missing")
    if any(phase["status"] in ("missing", "invalid") for phase in phases.values()):
        diagnostics.append("One or more phase records are missing or invalid")
    for name, phase in phases.items():
        if phase["status"] == "failed":
            diagnostics.append(f"{name} exited with {phase['exit_code']}")
    if tests and any(test["outcome"] == "failed" for test in tests):
        diagnostics.append("One or more nextest tests failed")
    if doc_results is not None and doc_results["failed"]:
        diagnostics.append("One or more doc tests failed")
    passed = not diagnostics and all(phase["status"] == "passed" for phase in phases.values())
    test_seconds = [phases[name]["duration_seconds"] for name in ("test", "doc")]
    scope_hash = hashlib.sha256("\n".join(sorted(test["id"] for test in tests)).encode("utf-8")).hexdigest() if tests else None
    return {
        "schema_version": SCHEMA_VERSION,
        "config_id": CONFIG_ID,
        **metadata,
        "phases": phases,
        "tests": tests,
        "test_count": len(tests),
        "test_ids_sha256": scope_hash,
        "doc_tests": doc_results,
        "test_execution_seconds": round(sum(test_seconds), 3) if all(value is not None for value in test_seconds) else None,
        "status": "passed" if passed else "failed" if any(phase["status"] == "failed" for phase in phases.values()) else "incomplete",
        "diagnostics": diagnostics,
    }


def summary(report):
    lines = [
        f"## Rust performance: {report['platform']}",
        "",
        f"Result: **{report['status']}** · commit `{report['commit']}` · toolchain `{report['toolchain']}`",
        "",
        "| Phase | Seconds | Result |",
        "| --- | ---: | --- |",
    ]
    for name in PHASES:
        phase = report["phases"][name]
        seconds = phase["duration_seconds"]
        lines.append(f"| {name} | {seconds if seconds is not None else 'n/a'} | {phase['status']} |")
    lines.extend(["", f"Test execution (nextest + doc): {report['test_execution_seconds']} s", "", "### Slowest tests", "", "| Test | Seconds | Result |", "| --- | ---: | --- |"])
    for test in sorted(report["tests"], key=lambda item: item["duration_seconds"], reverse=True)[:10]:
        lines.append(f"| `{test['id']}` | {test['duration_seconds']:.3f} | {test['outcome']} |")
    if not report["tests"]:
        lines.append("| Unavailable | n/a | incomplete |")
    if report["diagnostics"]:
        lines.extend(["", "Diagnostics: " + "; ".join(report["diagnostics"])])
    return "\n".join(lines) + "\n"


def append_summary(text):
    target = os.environ.get("GITHUB_STEP_SUMMARY")
    if target:
        with open(target, "a", encoding="utf-8") as output:
            output.write(text)
    else:
        print(text)


def report_command(args):
    metadata = {
        "run_id": os.environ.get("GITHUB_RUN_ID", "local"),
        "commit": os.environ.get("GITHUB_SHA", "local"),
        "workflow": os.environ.get("GITHUB_WORKFLOW", "local"),
        "platform": args.platform,
        "target": args.target,
        "runner_os": os.environ.get("RUNNER_OS", sys.platform),
        "runner_arch": os.environ.get("RUNNER_ARCH", "unknown"),
        "runner_image": os.environ.get("ImageOS", "unknown"),
        "toolchain": os.environ.get("RUSTUP_TOOLCHAIN", "unknown"),
    }
    result = build_report(args.directory, args.junit, metadata)
    write_json(args.output, result)
    append_summary(summary(result))
    return 0


def compatibility_key(report):
    return tuple(report.get(key) for key in ("schema_version", "config_id", "platform", "target", "runner_os", "runner_arch", "runner_image", "toolchain", "test_ids_sha256"))


def valid_time(value):
    try:
        return math.isfinite(float(value)) and float(value) > 0
    except (TypeError, ValueError):
        return False


def compare_reports(current, previous):
    if len(previous) < 5:
        return {"status": "insufficient", "message": "歷史資料不足：少於 5 筆成功 main 報告。"}
    if current.get("status") != "passed" or not valid_time(current.get("test_execution_seconds")):
        return {"status": "unavailable", "message": "本次測試未完成，無法比較效能。"}
    key = compatibility_key(current)
    if any(item.get("status") != "passed" or not valid_time(item.get("test_execution_seconds")) for item in previous[:5]):
        return {"status": "insufficient", "message": "歷史資料不足：少於 5 筆有效 main 報告。"}
    if any(compatibility_key(item) != key for item in previous[:5]):
        return {"status": "incomparable", "message": "歷史基準與本次測試範圍、工具鏈或 CI 設定不相容。"}
    times = sorted(float(item["test_execution_seconds"]) for item in previous[:5])
    median = times[2]
    if median <= 0:
        return {"status": "unavailable", "message": "歷史中位數無效。"}
    change = (float(current["test_execution_seconds"]) / median - 1) * 100
    is_regression = round(float(current["test_execution_seconds"]) * 1000) * 100 > round(median * 1000) * 130
    return {"status": "regression" if is_regression else "normal", "median_seconds": median, "change_percent": round(change, 2), "message": f"main 最近 5 次中位數 {median:.3f}s；本次差異 {change:+.2f}%。"}


def api_json(path, token):
    url = "https://api.github.com/" + path.lstrip("/")
    request = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json", "Authorization": "Bearer " + token, "X-GitHub-Api-Version": "2022-11-28"})
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response)


class StripCrossHostAuthorization(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        redirected = super().redirect_request(request, fp, code, message, headers, new_url)
        if redirected and urllib.parse.urlparse(new_url).hostname != urllib.parse.urlparse(request.full_url).hostname:
            redirected.remove_header("Authorization")
        return redirected


def artifact_report(repository, run_id, artifact_name, token):
    data = api_json(f"repos/{repository}/actions/runs/{run_id}/artifacts?per_page=100", token)
    artifact = next((item for item in data.get("artifacts", []) if item.get("name") == artifact_name and not item.get("expired")), None)
    if artifact is None:
        return None
    request = urllib.request.Request(artifact["archive_download_url"], headers={"Authorization": "Bearer " + token, "Accept": "application/vnd.github+json"})
    opener = urllib.request.build_opener(StripCrossHostAuthorization)
    with opener.open(request, timeout=30) as response:
        raw = response.read(4_000_001)
    if len(raw) > 4_000_000:
        raise ValueError("Historical artifact exceeds report size limit")
    with zipfile.ZipFile(io.BytesIO(raw)) as archive:
        names = [name for name in archive.namelist() if name.endswith("performance.json")]
        if len(names) != 1:
            return None
        with archive.open(names[0]) as source:
            return json.load(source)


def compare_command(args):
    try:
        current = json.loads(Path(args.report).read_text(encoding="utf-8"))
        repository = os.environ["GITHUB_REPOSITORY"]
        token = os.environ["GITHUB_TOKEN"]
        workflow = urllib.parse.quote(args.workflow, safe="")
        previous = []
        incompatible = False
        for page in range(1, 4):
            data = api_json(f"repos/{repository}/actions/workflows/{workflow}/runs?branch=main&status=success&per_page=30&page={page}", token)
            runs = data.get("workflow_runs", [])
            for run in runs:
                item = artifact_report(repository, run["id"], args.artifact, token)
                if item is None or item.get("status") != "passed" or not valid_time(item.get("test_execution_seconds")):
                    continue
                if compatibility_key(item) != compatibility_key(current):
                    incompatible = True
                    continue
                previous.append(item)
                if len(previous) == 5:
                    break
            if len(previous) == 5 or len(runs) < 30:
                break
        result = compare_reports(current, previous)
        if result["status"] == "insufficient" and incompatible:
            result = {"status": "incomparable", "message": "可相容的成功 main 報告少於 5 筆；較近的基準與本次 CI 設定不相容。"}
    except (OSError, ValueError, TypeError, KeyError, urllib.error.URLError, zipfile.BadZipFile, json.JSONDecodeError) as error:
        result = {"status": "unavailable", "message": f"歷史資料取得或統計失敗：{error}"}
    append_summary("\n### main 歷史效能比較\n\n" + result["message"] + (" **效能退化警告**" if result["status"] == "regression" else "") + "\n")
    return 0


def enforce_command(args):
    result = build_report(args.directory, args.junit, {})
    if result["status"] != "passed":
        print("Rust tests did not pass: " + "; ".join(result.get("diagnostics", [])), file=sys.stderr)
        return 1
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    measure_parser = commands.add_parser("measure")
    measure_parser.add_argument("--directory", required=True)
    measure_parser.add_argument("--phase", choices=PHASES, required=True)
    measure_parser.add_argument("command", nargs=argparse.REMAINDER)
    report_parser = commands.add_parser("report")
    report_parser.add_argument("--directory", required=True)
    report_parser.add_argument("--junit", required=True)
    report_parser.add_argument("--output", required=True)
    report_parser.add_argument("--platform", required=True)
    report_parser.add_argument("--target", required=True)
    compare_parser = commands.add_parser("compare")
    compare_parser.add_argument("--report", required=True)
    compare_parser.add_argument("--workflow", required=True)
    compare_parser.add_argument("--artifact", required=True)
    enforce_parser = commands.add_parser("enforce")
    enforce_parser.add_argument("--directory", required=True)
    enforce_parser.add_argument("--junit", required=True)
    args = parser.parse_args()
    return {"measure": measure, "report": report_command, "compare": compare_command, "enforce": enforce_command}[args.action](args)


if __name__ == "__main__":
    sys.exit(main())
