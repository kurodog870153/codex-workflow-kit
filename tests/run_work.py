"""Run Work Python tests with one discovery pass."""

from __future__ import annotations

import argparse
import sys
import unittest
from collections import Counter
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]
TEST_ROOT = PROJECT_ROOT / "tests" / "work"


def test_ids(suite: unittest.TestSuite) -> list[str]:
    ids: list[str] = []
    for test in suite:
        if isinstance(test, unittest.TestSuite):
            ids.extend(test_ids(test))
        else:
            ids.append(test.id())
    return ids


def discover(path: str = ".", pattern: str = "test*.py") -> unittest.TestSuite:
    start = (TEST_ROOT / path).resolve()
    if not start.is_relative_to(TEST_ROOT) or not start.is_dir():
        raise ValueError(f"test path must be a directory below {TEST_ROOT}: {path}")
    if str(PROJECT_ROOT) not in sys.path:
        sys.path.insert(0, str(PROJECT_ROOT))
    suite = unittest.TestLoader().discover(
        start_dir=str(start), pattern=pattern, top_level_dir=str(TEST_ROOT)
    )
    counts = Counter(test_ids(suite))
    duplicates = sorted(test_id for test_id, count in counts.items() if count > 1)
    if duplicates:
        raise ValueError("duplicate test IDs: " + ", ".join(duplicates))
    return suite


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--path", default=".", help="directory below tests/work")
    parser.add_argument("--pattern", default="test*.py")
    args = parser.parse_args(argv)
    try:
        suite = discover(args.path, args.pattern)
    except ValueError as exc:
        parser.error(str(exc))
    return 0 if unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
