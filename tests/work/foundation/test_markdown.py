from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.foundation.errors import WorkError
from worklib.foundation.markdown import (
    parse_json_contract,
    render_json_contract,
    require_canonical_json_contract,
)


class JsonContractTests(unittest.TestCase):
    def test_render_and_parse_round_trip(self) -> None:
        contract = {"schema": "example/v1", "名稱": "工作"}
        rendered = render_json_contract(contract)

        self.assertEqual(json.loads(rendered), contract)
        self.assertEqual(parse_json_contract(rendered, source="example.json"), contract)
        self.assertEqual(
            rendered,
            '{\n  "schema": "example/v1",\n  "名稱": "工作"\n}\n'.encode("utf-8"),
        )

    def test_canonical_check_rejects_equivalent_noncanonical_bytes(self) -> None:
        contract = {"schema": "example/v1"}
        rendered = render_json_contract(contract)
        for raw in (rendered + b"\n", rendered.replace(b"\n", b"\r\n"),
                    b"\xef\xbb\xbf" + rendered, b'{"schema":"example/v1"}\n'):
            with self.subTest(raw=raw):
                with self.assertRaises(WorkError) as context:
                    require_canonical_json_contract(
                        raw, contract=contract, source="example.json",
                    )
                self.assertEqual(context.exception.code, "noncanonical_json_contract")

    def test_json_parser_rejects_duplicate_keys(self) -> None:
        with self.assertRaises(WorkError) as context:
            parse_json_contract(b'{"schema":"one","schema":"two"}', source="stdin")
        self.assertEqual(context.exception.code, "duplicate_json_key")
        self.assertEqual(context.exception.details["key"], "schema")

    def test_json_parser_rejects_markdown_and_extra_content(self) -> None:
        fence = bytes([96]) * 3
        for raw in (
            b'# Example\n\n' + fence + b'json\n{"schema":"example/v1"}\n' + fence + b'\n',
            fence + b'json\n{"schema":"example/v1"}\n' + fence + b'\n',
            b'{"schema":"example/v1"}\nExtra',
            b'{"schema":"example/v1"}\n{}',
        ):
            with self.subTest(raw=raw):
                with self.assertRaises(WorkError) as context:
                    parse_json_contract(raw, source="example.json")
                self.assertEqual(context.exception.code, "invalid_json_contract")

    def test_json_parser_rejects_nonstandard_constants_and_nonobject_roots(self) -> None:
        for raw, code in ((b'{"value": NaN}', "invalid_json_constant"),
                          (b'{"value": Infinity}', "invalid_json_constant"),
                          (b'[]', "json_contract_not_object")):
            with self.subTest(raw=raw):
                with self.assertRaises(WorkError) as context:
                    parse_json_contract(raw, source="stdin")
                self.assertEqual(context.exception.code, code)

    def test_renderer_normalizes_unicode_and_preserves_escaped_newlines(self) -> None:
        rendered = render_json_contract({"value": "cafe\u0301\nline"})
        self.assertEqual(json.loads(rendered), {"value": "caf\u00e9\nline"})
        self.assertTrue(rendered.endswith(b"}\n"))


if __name__ == "__main__":
    unittest.main()
