from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass(slots=True)
class NodeSummary:
    id: str
    fqn: str
    kind: Any
    confidence: float
    resolution: str
    source_available: bool


@dataclass(slots=True)
class SubgraphSummary:
    queried_node_id: str
    queried_node_fqn: str
    total_callers: int
    total_callees: int
    high_confidence_callers: list[NodeSummary] = field(default_factory=list)
    low_confidence_callers: list[NodeSummary] = field(default_factory=list)
    high_confidence_callees: list[NodeSummary] = field(default_factory=list)
    low_confidence_callees: list[NodeSummary] = field(default_factory=list)
    truncated: bool = False
    omitted_count: int | None = None

    @classmethod
    def from_dict(cls, payload: dict[str, Any]) -> "SubgraphSummary":
        def load_many(key: str) -> list[NodeSummary]:
            return [NodeSummary(**item) for item in payload.get(key, [])]

        return cls(
            queried_node_id=payload["queried_node_id"],
            queried_node_fqn=payload["queried_node_fqn"],
            total_callers=payload["total_callers"],
            total_callees=payload["total_callees"],
            high_confidence_callers=load_many("high_confidence_callers"),
            low_confidence_callers=load_many("low_confidence_callers"),
            high_confidence_callees=load_many("high_confidence_callees"),
            low_confidence_callees=load_many("low_confidence_callees"),
            truncated=payload["truncated"],
            omitted_count=payload.get("omitted_count"),
        )

