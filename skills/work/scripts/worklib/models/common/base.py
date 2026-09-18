from __future__ import annotations

import json
import unicodedata
from typing import Any, ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, ValidationError

from .errors import ExitCode, WorkError


ContractKind = Literal["request", "response", "artifact", "envelope"]


class WorkContract(BaseModel):
    model_config = ConfigDict(
        strict=True,
        extra="forbid",
        frozen=True,
        validate_default=True,
    )

    contract_id: ClassVar[str]
    contract_kind: ClassVar[ContractKind]
    canonical_order: ClassVar[tuple[str, ...]]
    omit_none: ClassVar[bool] = True
    contract_example: ClassVar[dict[str, Any] | None] = None
    field_constraints: ClassVar[dict[str, dict[str, Any]]] = {}
    field_references: ClassVar[dict[str, str]] = {}

    @classmethod
    def __pydantic_init_subclass__(cls, **kwargs: Any) -> None:
        super().__pydantic_init_subclass__(**kwargs)
        for attribute in ("contract_id", "contract_kind", "canonical_order"):
            if attribute not in cls.__dict__:
                raise TypeError(f"{cls.__name__} must declare {attribute}.")
        aliases = tuple(
            field.alias or name for name, field in cls.model_fields.items()
        )
        if cls.canonical_order != aliases:
            raise TypeError(
                f"{cls.__name__}.canonical_order must exactly match its model fields."
            )

    @classmethod
    def _location(cls, parts: tuple[object, ...]) -> str:
        result = ""
        for part in parts:
            if isinstance(part, int):
                result += f"[{part}]"
            elif result:
                result += f".{part}"
            else:
                result = str(part)
        return result or "contract"

    @classmethod
    def _canonical_text(cls, text: str) -> str:
        normalized = unicodedata.normalize("NFC", text)
        normalized = normalized.replace("\r\n", "\n").replace("\r", "\n")
        return normalized.rstrip("\n") + "\n"

    @classmethod
    def _decode_utf8(cls, raw: bytes, *, source: str) -> str:
        if raw.startswith(b"\xef\xbb\xbf"):
            raw = raw[3:]
        try:
            return raw.decode("utf-8", errors="strict")
        except UnicodeDecodeError as error:
            raise WorkError(
                ExitCode.INPUT_FORMAT,
                "invalid_utf8",
                "The input is not valid UTF-8.",
                {"source": source, "byte_offset": error.start},
            ) from error

    @classmethod
    def _strict_object(cls, pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise WorkError(
                    ExitCode.INPUT_FORMAT,
                    "duplicate_json_key",
                    "The JSON contract contains a duplicate key.",
                    {"key": key},
                )
            result[key] = value
        return result

    @classmethod
    def _reject_json_constant(cls, value: str) -> None:
        raise WorkError(
            ExitCode.INPUT_FORMAT,
            "invalid_json_constant",
            "The JSON contract contains a non-standard numeric constant.",
            {"value": value},
        )

    @classmethod
    def _parse_json_contract(cls, raw: bytes, *, source: str) -> dict[str, Any]:
        text = cls._canonical_text(cls._decode_utf8(raw, source=source))
        try:
            contract = json.loads(
                text,
                object_pairs_hook=cls._strict_object,
                parse_constant=cls._reject_json_constant,
            )
        except WorkError:
            raise
        except json.JSONDecodeError as error:
            raise WorkError(
                ExitCode.INPUT_FORMAT,
                "invalid_json_contract",
                "The JSON contract is invalid.",
                {"line": error.lineno, "column": error.colno},
            ) from error
        if not isinstance(contract, dict):
            raise WorkError(
                ExitCode.CONTRACT,
                "json_contract_not_object",
                "The JSON contract root must be an object.",
            )
        return contract

    @classmethod
    def parse_json_bytes(cls, raw: bytes, *, source: str) -> Self:
        value = cls._parse_json_contract(raw, source=source)
        try:
            return cls.model_validate(value)
        except ValidationError as error:
            raise cls._work_error(error) from error

    @classmethod
    def _work_error(cls, error: ValidationError) -> WorkError:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
        aggregated = sorted(
            (
                {
                    "location": cls._location(tuple(issue["loc"])),
                    "validation_type": issue["type"],
                }
                for issue in issues
            ),
            key=lambda issue: (issue["location"], issue["validation_type"]),
        )
        field_issues = [
            issue for issue in issues if issue["type"] in {"missing", "extra_forbidden"}
        ]
        if field_issues:
            first_location = tuple(field_issues[0]["loc"])
            parent = first_location[:-1]
            related = [
                issue
                for issue in field_issues
                if tuple(issue["loc"])[:-1] == parent
            ]
            return WorkError(
                ExitCode.CONTRACT,
                "invalid_object_fields",
                "The JSON object has missing or unknown fields.",
                {
                    "location": cls._location(parent),
                    "missing": sorted(
                        str(issue["loc"][-1])
                        for issue in related
                        if issue["type"] == "missing"
                    ),
                    "unknown": sorted(
                        str(issue["loc"][-1])
                        for issue in related
                        if issue["type"] == "extra_forbidden"
                    ),
                    "issues": aggregated,
                },
            )
        first = issues[0]
        return WorkError(
            ExitCode.CONTRACT,
            "invalid_contract_value",
            "The JSON contract contains an invalid value.",
            {
                "location": cls._location(tuple(first["loc"])),
                "validation_type": first["type"],
                "issues": aggregated,
            },
        )

    def to_canonical_dict(self) -> dict[str, Any]:
        dumped = self.model_dump(
            mode="json",
            by_alias=True,
            exclude_none=False,
            exclude_unset=self.omit_none,
        )
        return {
            field: dumped[field]
            for field in self.canonical_order
            if field in dumped
        }

    def render_canonical_json(self) -> bytes:
        payload = json.dumps(
            self.to_canonical_dict(),
            ensure_ascii=False,
            indent=2,
            allow_nan=False,
        )
        return self._canonical_text(payload).encode("utf-8")


__all__ = ["ContractKind", "WorkContract"]

