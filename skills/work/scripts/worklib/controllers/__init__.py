"""Public CLI controllers."""
from __future__ import annotations

import argparse
from typing import Protocol


class SubparserRegistry(Protocol):
    def add_parser(
        self,
        name: str,
        **kwargs: object,
    ) -> argparse.ArgumentParser: ...
