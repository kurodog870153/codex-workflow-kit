"""Invocation CLI use cases."""

from ...services.invocation import parse_invocation


def parse_invocation_request(raw: bytes, *, source: str) -> dict[str, object]:
    return parse_invocation(raw, source=source).to_canonical_dict()


__all__ = ["parse_invocation_request"]
