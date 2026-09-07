from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.foundation.markdown import (
    parse_json_contract,
    parse_markdown_json_contract,
    render_markdown_json_contract,
    require_canonical_markdown_json_contract,
)


class MarkdownContractTests(unittest.TestCase):
    def test_render_and_parse_round_trip(self) -> None:
        contract = {"schema": "example/v1", "名稱": "工作"}

        rendered = render_markdown_json_contract("Example", contract)
        title, parsed = parse_markdown_json_contract(rendered, source="example.md")

        self.assertEqual(title, "Example")
        self.assertEqual(parsed, contract)
        self.assertEqual(
            rendered,
            (
                '# Example\n\n```json\n{\n  "schema": "example/v1",\n'
                '  "名稱": "工作"\n}\n```\n'
            ).encode("utf-8"),
        )

    def test_canonical_check_rejects_equivalent_noncanonical_bytes(self) -> None:
        contract = {"schema": "example/v1"}
        rendered = render_markdown_json_contract("Example", contract)

        with self.assertRaises(WorkError) as context:
            require_canonical_markdown_json_contract(
                rendered + b"\n",
                title="Example",
                contract=contract,
                source="example.md",
            )

        self.assertEqual(context.exception.code, "noncanonical_markdown_contract")

    def test_json_parser_rejects_duplicate_keys(self) -> None:
        with self.assertRaises(WorkError) as context:
            parse_json_contract(b'{"schema":"one","schema":"two"}', source="stdin")

        self.assertEqual(context.exception.code, "duplicate_json_key")
        self.assertEqual(context.exception.details["key"], "schema")

    def test_markdown_parser_requires_title_before_json_fence(self) -> None:
        raw = b'```json\n{"schema":"example/v1"}\n```\n'

        with self.assertRaises(WorkError) as context:
            parse_markdown_json_contract(raw, source="example.md")

        self.assertEqual(context.exception.code, "invalid_markdown_title")


if __name__ == "__main__":
    unittest.main()
