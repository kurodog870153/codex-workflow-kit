from __future__ import annotations

import base64
import binascii
from typing import Annotated, Any, ClassVar, Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator, model_validator

from ..foundation.fingerprint import canonical_json_sha256, raw_sha256
from .base import WorkContract


SHA256_PATTERN = r"^[0-9a-f]{64}$"
TRANSACTION_ID_PATTERN = r"^[A-Z][A-Z0-9-]{2,63}$"


class SpecTransactionNestedModel(BaseModel):
    model_config = ConfigDict(
        strict=True,
        extra="forbid",
        frozen=True,
        validate_default=True,
    )


class SpecTransactionSnapshotModel(SpecTransactionNestedModel):
    raw_sha256: str = Field(pattern=SHA256_PATTERN)
    base64: str

    @field_validator("base64")
    @classmethod
    def validate_base64(cls, value: str) -> str:
        try:
            raw = base64.b64decode(value, validate=True)
        except (ValueError, binascii.Error) as error:
            raise ValueError("Transaction bytes must use canonical base64.") from error
        if base64.b64encode(raw).decode("ascii") != value:
            raise ValueError("Transaction bytes must use canonical base64.")
        return value

    @model_validator(mode="after")
    def validate_fingerprint(self) -> "SpecTransactionSnapshotModel":
        raw = base64.b64decode(self.base64, validate=True)
        if raw_sha256(raw) != self.raw_sha256:
            raise ValueError("Transaction bytes do not match their fingerprint.")
        return self


class SpecTransactionAddFileModel(SpecTransactionNestedModel):
    phase: int = Field(ge=0)
    path: str
    operation: Literal["add"]
    after: SpecTransactionSnapshotModel


class SpecTransactionReplaceFileModel(SpecTransactionNestedModel):
    phase: int = Field(ge=0)
    path: str
    operation: Literal["replace"]
    before: SpecTransactionSnapshotModel
    after: SpecTransactionSnapshotModel


class SpecTransactionRemoveFileModel(SpecTransactionNestedModel):
    phase: int = Field(ge=0)
    path: str
    operation: Literal["remove"]
    before: SpecTransactionSnapshotModel


SpecTransactionFileModel = Annotated[
    SpecTransactionAddFileModel
    | SpecTransactionReplaceFileModel
    | SpecTransactionRemoveFileModel,
    Field(discriminator="operation"),
]


class SpecTransactionMetadataModel(SpecTransactionNestedModel):
    request: dict[str, Any]
    artifacts: dict[str, Any]
    affected_task_ids: list[str]
    history_sha256: dict[str, str]
    source_sha256: dict[str, str]
    candidate_sha256: dict[str, str]


class SpecTransactionContract(WorkContract):
    contract_id: ClassVar[str] = "work-spec-transaction/v1"
    contract_kind: ClassVar[Literal["artifact"]] = "artifact"
    canonical_order: ClassVar[tuple[str, ...]] = (
        "schema", "transaction_id", "approval_sha256", "state",
        "published_count", "metadata", "files",
    )

    schema_: Literal["work-spec-transaction/v1"] = Field(alias="schema")
    transaction_id: str = Field(pattern=TRANSACTION_ID_PATTERN)
    approval_sha256: str = Field(pattern=SHA256_PATTERN)
    state: Literal["prepared", "publishing", "published"]
    published_count: int = Field(ge=0)
    metadata: SpecTransactionMetadataModel
    files: list[SpecTransactionFileModel] = Field(min_length=1)

    @field_validator("files")
    @classmethod
    def validate_files(
        cls, value: list[SpecTransactionFileModel]
    ) -> list[SpecTransactionFileModel]:
        for item in value:
            path = item.path
            if (
                not path
                or "\\" in path
                or path.startswith("/")
                or any(part in {"", ".", ".."} for part in path.split("/"))
            ):
                raise ValueError("Transaction paths must be safe project-relative POSIX paths.")
        order = [(item.phase, item.path) for item in value]
        paths = [item.path for item in value]
        if order != sorted(order) or len(paths) != len(set(paths)):
            raise ValueError("Transaction files must have unique paths in phase and lexical order.")
        return value

    @model_validator(mode="after")
    def validate_approval_and_progress(self) -> "SpecTransactionContract":
        files = [item.model_dump(mode="json", exclude_none=True) for item in self.files]
        metadata = self.metadata.model_dump(mode="json")
        expected_approval = canonical_json_sha256({"files": files, "metadata": metadata})
        if self.approval_sha256 != expected_approval:
            raise ValueError("The approval fingerprint does not match the file set.")
        if self.published_count > len(self.files):
            raise ValueError("The published file count is invalid.")
        expected_state = (
            "prepared"
            if self.published_count == 0
            else "published"
            if self.published_count == len(self.files)
            else "publishing"
        )
        if self.state != expected_state:
            raise ValueError("The transaction state does not match its progress.")
        return self


SpecTransactionContract.contract_example = {
    "schema": "work-spec-transaction/v1",
    "transaction_id": "SPEC-UPDATE-001",
    "approval_sha256": "655c3708cea35b6203cbeb668548c7f85921daf87dccb47d7abe3420c204a798",
    "state": "prepared",
    "published_count": 0,
    "metadata": {
        "request": {"schema": "example/v1"},
        "artifacts": {},
        "affected_task_ids": ["TASK-001"],
        "history_sha256": {},
        "source_sha256": {},
        "candidate_sha256": {},
    },
    "files": [
        {
            "phase": 10,
            "path": "example.json",
            "operation": "add",
            "after": {
                "raw_sha256": "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
                "base64": "e30=",
            },
        }
    ],
}
