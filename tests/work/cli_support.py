"""Real request files for CLI tests, outside each artifact workspace."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path


class FileInputTestCase(unittest.TestCase):
    def input_file(self, raw: str | bytes) -> str:
        directory = tempfile.TemporaryDirectory(prefix="work-cli-")
        self.addCleanup(directory.cleanup)
        path = Path(directory.name) / "請求 input.json"
        path.write_bytes(raw.encode("utf-8") if isinstance(raw, str) else raw)
        return str(path)

    def input_arguments(self, arguments, raw: str | bytes) -> list[str]:
        result = list(arguments)
        if "--input-file" in result:
            position = result.index("--input-file") + 1
            result[position] = self.input_file(raw)
        return result
