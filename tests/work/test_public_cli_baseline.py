from __future__ import annotations

import argparse
import io
import json
import sys
import unittest
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
SCRIPT_ROOT = PROJECT_ROOT / "skills" / "work" / "scripts"
BASELINE = Path(__file__).with_name("baselines") / "cli.json"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.cli import build_parser, main


def _leaf_commands() -> list[str]:
    result: list[str] = []

    def walk(parser: argparse.ArgumentParser, prefix: list[str]) -> None:
        subparsers = [
            action
            for action in parser._actions
            if isinstance(action, argparse._SubParsersAction)
        ]
        if not subparsers:
            result.append(" ".join(prefix))
            return
        for action in subparsers:
            for name, child in sorted(action.choices.items()):
                walk(child, [*prefix, name])

    walk(build_parser(), [])
    return result


class PublicCliBaselineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.baseline = json.loads(BASELINE.read_text(encoding="utf-8"))

    def test_public_command_tree_matches_baseline(self) -> None:
        self.assertEqual(_leaf_commands(), self.baseline["commands"])

    def test_cli_envelopes_match_canonical_byte_baselines(self) -> None:
        escaped_root = json.dumps(str(PROJECT_ROOT))[1:-1]
        for case in self.baseline["cases"]:
            with self.subTest(case=case["name"]):
                argv = [
                    str(PROJECT_ROOT) if token == "<PROJECT_ROOT>" else token
                    for token in case["argv"]
                ]
                stdout = io.StringIO()
                stderr = io.StringIO()
                exit_code = main(argv, stdout=stdout, stderr=stderr)
                actual_stdout = stdout.getvalue().replace(
                    escaped_root, "<PROJECT_ROOT>"
                )
                self.assertEqual(exit_code, case["exit_code"])
                self.assertEqual(actual_stdout, case["stdout"])
                self.assertEqual(stderr.getvalue(), case["stderr"])


if __name__ == "__main__":
    unittest.main()
