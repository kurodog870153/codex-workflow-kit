from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT_ROOT = Path(__file__).resolve().parents[3] / "skills" / "work" / "scripts"
sys.path.insert(0, str(SCRIPT_ROOT))

from worklib.models.common.errors import WorkError
from worklib.technical.infrastructure.file_io import fingerprint_file, read_raw
from worklib.technical.infrastructure.text_codec import decode_utf8


class FileIoTests(unittest.TestCase):
    def test_reads_and_fingerprints_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "source.txt"
            path.write_bytes(b"source\r\n")

            self.assertEqual(read_raw(path), b"source\r\n")
            result = fingerprint_file(path)
            self.assertEqual(len(result["canonical_sha256"]), 64)
            self.assertEqual(len(result["raw_sha256"]), 64)
            self.assertNotEqual(
                result["canonical_sha256"],
                result["raw_sha256"],
            )

    def test_missing_file_preserves_error_contract(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "missing.txt"
            with self.assertRaises(WorkError) as context:
                read_raw(path)

            self.assertEqual(context.exception.code, "file_not_found")

    def test_directory_is_rejected_as_non_regular_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(WorkError) as context:
                read_raw(Path(temporary))
        self.assertEqual(context.exception.code, "file_not_found")

    def test_invalid_utf8_preserves_error_contract(self) -> None:
        with self.assertRaises(WorkError) as context:
            decode_utf8(b"\xff", source="test input")

        self.assertEqual(context.exception.code, "invalid_utf8")
        self.assertEqual(context.exception.details["byte_offset"], 0)


if __name__ == "__main__":
    unittest.main()
