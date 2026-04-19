from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

import neurograph_engine

from .models import SubgraphSummary


@dataclass(slots=True)
class EngineBridge:
    """Thin Python wrapper around the PyO3 module."""

    _engine: Any

    @classmethod
    def create(cls, confidence_config: dict[str, Any] | None = None) -> "EngineBridge":
        payload = json.dumps(confidence_config) if confidence_config is not None else None
        return cls(neurograph_engine.GraphEngine(payload))

    def startup_warning(self) -> str | None:
        return self._engine.startup_warning()

    def current_version(self) -> int:
        return self._engine.current_version()

    def increment_version(self) -> int:
        return self._engine.increment_version()

    def confidence_config(self) -> dict[str, Any]:
        return json.loads(self._engine.confidence_config_json())

    def set_confidence_config(self, config: dict[str, Any]) -> None:
        self._engine.set_confidence_config(json.dumps(config))

    def upsert_node(self, node: dict[str, Any]) -> dict[str, Any]:
        return json.loads(self._engine.upsert_node(json.dumps(node)))

    def write_edge(self, edge: dict[str, Any]) -> dict[str, Any]:
        return json.loads(self._engine.write_edge(json.dumps(edge)))

    def sync_python_project(self, root: str) -> dict[str, Any]:
        return json.loads(self._engine.sync_python_project(root))

    def analyze_diff(self, diff_text: str) -> dict[str, Any]:
        return json.loads(self._engine.analyze_diff(diff_text))

    def deprecate_node(self, node_id: str, deprecated_status: dict[str, Any]) -> dict[str, Any]:
        return json.loads(self._engine.deprecate_node(node_id, json.dumps(deprecated_status)))

    def mark_node_deleted_in_overlay(self, node_id: str) -> None:
        self._engine.mark_node_deleted_in_overlay(node_id)

    def get_subgraph(
        self,
        node_id: str,
        escalation_confidence_threshold: float,
        max_nodes: int = 25,
    ) -> SubgraphSummary:
        payload = self._engine.get_subgraph(node_id, escalation_confidence_threshold, max_nodes)
        return SubgraphSummary.from_dict(json.loads(payload))

    def get_node_detail(self, node_id: str) -> dict[str, Any]:
        return json.loads(self._engine.get_node_detail(node_id))

    def get_unresolved_calls(self, node_id: str) -> list[str]:
        return json.loads(self._engine.get_unresolved_calls(node_id))

    def create_snapshot(self, pr_identifier: str) -> dict[str, Any]:
        return json.loads(self._engine.create_snapshot(pr_identifier))

    def snapshot_with_live_version(
        self, snapshot: dict[str, Any], current_baseline_version: int
    ) -> dict[str, Any]:
        payload = self._engine.snapshot_with_live_version(
            json.dumps(snapshot), current_baseline_version
        )
        return json.loads(payload)

    def snapshot_is_stale(self, snapshot: dict[str, Any], current_baseline_version: int) -> bool:
        return self._engine.snapshot_is_stale(json.dumps(snapshot), current_baseline_version)

    def detect_rename(self, deprecated_node_id: str, candidate_node_id: str) -> dict[str, Any]:
        return json.loads(self._engine.detect_rename(deprecated_node_id, candidate_node_id))

    def apply_rename(self, deprecated_node_id: str, candidate_node_id: str) -> dict[str, Any]:
        return json.loads(self._engine.apply_rename(deprecated_node_id, candidate_node_id))
