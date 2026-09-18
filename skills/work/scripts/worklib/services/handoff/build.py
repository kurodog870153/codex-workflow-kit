from __future__ import annotations

import copy

from ...models.handoff import DIRECTION_STAGES, HANDOFF_MARKER


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

