from __future__ import annotations

from collections.abc import Mapping
from typing import Any, ClassVar, Literal, Self

from pydantic import BaseModel, ConfigDict, ValidationError

from ..foundation.errors import ExitCode, WorkError
from ..foundation.markdown import parse_json_contract, render_json_contract


ContractKind = Literal["request", "response", "artifact", "envelope"]


def _location(parts: tuple[object, ...]) -> str:
    result = ""
    for part in parts:
        if isinstance(part, int):
            result += f"[{part}]"
        elif result:
            result += f".{part}"
        else:
            result = str(part)
    return result or "contract"


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
    def parse_json_bytes(cls, raw: bytes, *, source: str) -> Self:
        value = parse_json_contract(raw, source=source)
        try:
            return cls.model_validate(value)
        except ValidationError as error:
            raise cls._work_error(error) from error

    @classmethod
    def _work_error(cls, error: ValidationError) -> WorkError:
        issues = error.errors(include_url=False, include_context=False, include_input=False)
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
                    "location": _location(parent),
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
                },
            )
        first = issues[0]
        return WorkError(
            ExitCode.CONTRACT,
            "invalid_contract_value",
            "The JSON contract contains an invalid value.",
            {
                "location": _location(tuple(first["loc"])),
                "validation_type": first["type"],
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
        return render_json_contract(self.to_canonical_dict())


def contract_mapping(value: WorkContract) -> Mapping[str, Any]:
    return value.to_canonical_dict()
