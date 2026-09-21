"""Specification single-function services."""

from .document_io import parse_json_contract, raw_sha256
from .history import execution_history_fingerprints
from .storage import read_raw, storage_path, write_prepared_output
from .transaction import (
    completion_marker_matches,
    encode_snapshot,
    transaction_approval_sha256,
    canonicalize_spec_transaction,
    render_spec_transaction,
    validate_spec_transaction,
    publish_journal,
    require_no_spec_update,
    transaction_completion_state,
    write_journal,
)

__all__ = [
    "execution_history_fingerprints",
    "parse_json_contract",
    "raw_sha256",
    "read_raw",
    "storage_path",
    "write_prepared_output",
    "completion_marker_matches",
    "encode_snapshot",
    "transaction_approval_sha256",
    "canonicalize_spec_transaction",
    "render_spec_transaction",
    "validate_spec_transaction",
    "publish_journal",
    "require_no_spec_update",
    "transaction_completion_state",
    "write_journal",
]
