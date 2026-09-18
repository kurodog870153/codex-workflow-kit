"""Handoff source I/O capability."""

from ....technical.foundation.fingerprint import raw_sha256
from ....technical.infrastructure.file_io import read_raw
from ....technical.infrastructure.json_contract import parse_json_contract
from ....technical.infrastructure.work_paths import (
    resolve_project_relative_path,
    resolve_task_collection_item_path,
    validate_execution_task_layout,
)

__all__ = [
    "parse_json_contract", "read_raw", "raw_sha256",
    "resolve_project_relative_path", "resolve_task_collection_item_path",
    "validate_execution_task_layout",
]
