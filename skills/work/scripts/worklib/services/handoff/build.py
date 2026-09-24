from __future__ import annotations

import copy

from ...models.handoff import DIRECTION_STAGES, HANDOFF_MARKER, DiscussionHandoffContract, DiscussionHandoffRequestContract
from ...models.common.identifiers import IdentifierPolicy


def build_handoff(direction, requirement_id, artifacts, source, payload):
    source_stage, target_stage = DIRECTION_STAGES[direction]
    return {
        "schema": "work-handoff/v1",
        "marker": HANDOFF_MARKER,
        "direction": direction,
        "requirement_id": requirement_id,
        "artifacts": copy.deepcopy(artifacts),
        "source": {"stage": source_stage, **copy.deepcopy(source)},
        "target": {"stage": target_stage},
        **copy.deepcopy(payload),
    }


def build_discussion_handoff(raw: bytes, *, source: str) -> dict[str, object]:
    request = DiscussionHandoffRequestContract.parse_json_bytes(raw, source=source).to_canonical_dict()
    IdentifierPolicy.requirement_id(request["requirement_id"])
    source_stage, target_stage = DIRECTION_STAGES[request["direction"]]
    return DiscussionHandoffContract.model_validate({
        "schema": "work-discussion-handoff/v1", "marker": "WORK-DISCUSSION-HANDOFF",
        "direction": request["direction"], "requirement_id": request["requirement_id"],
        "source_stage": source_stage, "target_stage": target_stage,
        "source_status": "unsaved_discussion", "source_validation": "not_checked",
        "grants_authorization": False,
        **{key: copy.deepcopy(value) for key, value in request.items() if key not in {"schema", "direction", "requirement_id"}},
    }).to_canonical_dict()
