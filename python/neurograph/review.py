from __future__ import annotations

import dataclasses
import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from .engine import EngineBridge
from .routing import RoutingConfig, validate_routing_config


@dataclass(slots=True)
class ReviewFinding:
    id: str
    severity: str
    title: str
    summary: str
    recommendation: str
    kind: str
    file_path: str | None = None
    node_fqn: str | None = None
    confidence: float | None = None
    evidence: list[str] = field(default_factory=list)


@dataclass(slots=True)
class ReviewReport:
    pr_identifier: str
    project_root: str
    baseline_cache_path: str
    graph_version: int
    snapshot: dict[str, Any]
    snapshot_stale: bool
    overlay: dict[str, Any]
    sync_report: dict[str, Any]
    diff_analysis: dict[str, Any]
    findings: list[ReviewFinding]
    warnings: list[str]
    summary: dict[str, int]

    def to_dict(self) -> dict[str, Any]:
        return {
            "pr_identifier": self.pr_identifier,
            "project_root": self.project_root,
            "baseline_cache_path": self.baseline_cache_path,
            "graph_version": self.graph_version,
            "snapshot": self.snapshot,
            "snapshot_stale": self.snapshot_stale,
            "overlay": self.overlay,
            "sync_report": self.sync_report,
            "diff_analysis": self.diff_analysis,
            "findings": [dataclasses.asdict(finding) for finding in self.findings],
            "warnings": self.warnings,
            "summary": self.summary,
        }

    def to_json(self) -> str:
        return json.dumps(self.to_dict(), indent=2)


class ReviewRunner:
    def __init__(
        self,
        engine: EngineBridge | None = None,
        routing_config: RoutingConfig | None = None,
        baseline_cache_name: str = "baseline.json",
    ) -> None:
        self.engine = engine or EngineBridge.create()
        self.routing_config = routing_config or RoutingConfig()
        self.baseline_cache_name = baseline_cache_name

    def run(self, project_root: Path, diff_text: str, pr_identifier: str) -> ReviewReport:
        cache_dir = project_root / ".neurograph"
        cache_path = cache_dir / self.baseline_cache_name
        cache_dir.mkdir(exist_ok=True)

        baseline_loaded = cache_path.exists()
        if baseline_loaded:
            self.engine.load_graph_state(str(cache_path))
            sync_report: dict[str, Any] | None = None
        else:
            sync_report = self.engine.sync_python_project(str(project_root))
            self.engine.save_graph_state(str(cache_path))

        overlay = self.engine.create_overlay_review(pr_identifier, diff_text)
        snapshot = overlay["snapshot"]
        diff_analysis = overlay["diff_analysis"]

        if baseline_loaded:
            live_engine = EngineBridge.create(self.engine.confidence_config())
            live_engine.load_graph_state(str(cache_path))
            sync_report = live_engine.sync_python_project(str(project_root))
            live_engine.save_graph_state(str(cache_path))
            snapshot = live_engine.snapshot_with_live_version(snapshot, sync_report["version"])
            snapshot_stale = live_engine.snapshot_is_stale(snapshot, sync_report["version"])
        else:
            if sync_report is None:
                raise RuntimeError("sync_report must exist after initial baseline creation")
            snapshot = self.engine.snapshot_with_live_version(snapshot, sync_report["version"])
            snapshot_stale = False

        overlay["snapshot"] = snapshot

        warnings = validate_routing_config(
            self.routing_config, self.engine.confidence_config()
        )
        startup_warning = self.engine.startup_warning()
        if startup_warning:
            warnings.append(startup_warning)
        if baseline_loaded:
            warnings.append(f"Loaded cached baseline graph from {cache_path}.")
        else:
            warnings.append(
                f"No cached baseline graph existed. A fresh baseline was created at {cache_path}."
            )
        warnings.extend(sync_report.get("warnings", []))
        warnings.extend(overlay.get("warnings", []))
        warnings.extend(
            f"Deprecated node still unresolved after sync: {fqn}"
            for fqn in sync_report.get("unresolved_deprecations", [])
        )
        if snapshot_stale:
            warnings.append(
                "The cached overlay baseline is stale compared with the live baseline sync. Some deprecated nodes may already be garbage collected."
            )
        if not diff_analysis.get("changed_node_ids"):
            warnings.append(
                "The diff did not map to any active nodes. Review coverage is limited to file-level symbols."
            )

        findings: list[ReviewFinding] = []
        deleted_symbols = {
            symbol.rsplit(".", 1)[-1] for symbol in diff_analysis.get("deleted_symbols", [])
        }
        for index, node_id in enumerate(diff_analysis.get("changed_node_ids", []), start=1):
            try:
                detail = self.engine.get_overlay_node_detail(overlay, node_id)
            except ValueError as error:
                try:
                    payload = json.loads(str(error))
                except json.JSONDecodeError:
                    raise
                if payload.get("error") != "NODE_DELETED_IN_PR":
                    raise

                baseline_detail = self.engine.get_node_detail(node_id)
                findings.append(
                    ReviewFinding(
                        id=f"finding-{index}-deleted-in-pr",
                        severity="high",
                        title="Baseline node removed in the PR overlay",
                        summary=payload["detail"],
                        recommendation=payload["suggestion"],
                        kind="node-deleted-in-pr",
                        file_path=baseline_detail["file_path"],
                        node_fqn=baseline_detail["fqn"],
                        confidence=0.95,
                        evidence=[
                            f"Overlay deleted node id: {node_id}",
                            f"Deleted symbols in diff: {', '.join(sorted(deleted_symbols)) or 'none'}",
                        ],
                    )
                )
                continue

            summary = self.engine.get_overlay_subgraph(
                overlay,
                node_id,
                self.routing_config.escalation_confidence_threshold,
            )
            unresolved_calls = self.engine.get_unresolved_calls(node_id)

            if unresolved_calls:
                broken_calls = [
                    call for call in unresolved_calls if call.rsplit(".", 1)[-1] in deleted_symbols
                ]
                findings.append(
                    ReviewFinding(
                        id=f"finding-{index}-unresolved",
                        severity="high" if broken_calls else "medium",
                        title="Unresolved call targets in changed code",
                        summary=(
                            f"{detail['fqn']} contains unresolved calls: "
                            + ", ".join(sorted(set(unresolved_calls)))
                        ),
                        recommendation=(
                            "Restore or replace the missing callable, or add explicit structure the engine can resolve."
                        ),
                        kind="unresolved-calls",
                        file_path=detail["file_path"],
                        node_fqn=detail["fqn"],
                        confidence=0.8 if broken_calls else 0.55,
                        evidence=[
                            f"Deleted symbols in diff: {', '.join(sorted(deleted_symbols)) or 'none'}",
                            f"Unresolved calls: {', '.join(sorted(set(unresolved_calls)))}",
                        ],
                    )
                )

            low_confidence_neighbors = (
                summary.low_confidence_callers + summary.low_confidence_callees
            )
            if low_confidence_neighbors:
                findings.append(
                    ReviewFinding(
                        id=f"finding-{index}-low-confidence",
                        severity="medium",
                        title="Low-confidence dependencies near changed code",
                        summary=(
                            f"{detail['fqn']} is surrounded by {len(low_confidence_neighbors)} "
                            "heuristic or uncertain graph edges."
                        ),
                        recommendation=(
                            "Inspect the neighboring call paths before trusting automated impact analysis."
                        ),
                        kind="low-confidence-neighbors",
                        file_path=detail["file_path"],
                        node_fqn=detail["fqn"],
                        confidence=min(
                            neighbor.confidence for neighbor in low_confidence_neighbors
                        ),
                        evidence=[
                            f"Low-confidence callers: {len(summary.low_confidence_callers)}",
                            f"Low-confidence callees: {len(summary.low_confidence_callees)}",
                        ],
                    )
                )

            if summary.truncated:
                findings.append(
                    ReviewFinding(
                        id=f"finding-{index}-truncated",
                        severity="low",
                        title="Subgraph context was truncated",
                        summary=(
                            f"The dependency summary for {detail['fqn']} hit the node cap and omitted "
                            f"{summary.omitted_count or 0} nodes."
                        ),
                        recommendation="Drill into this node manually if the change surface is larger than expected.",
                        kind="truncated-subgraph",
                        file_path=detail["file_path"],
                        node_fqn=detail["fqn"],
                        confidence=0.35,
                        evidence=[
                            f"Total callers: {summary.total_callers}",
                            f"Total callees: {summary.total_callees}",
                        ],
                    )
                )

        for rename_index, rename in enumerate(sync_report.get("renamed_nodes", []), start=1):
            findings.append(
                ReviewFinding(
                    id=f"rename-{rename_index}",
                    severity="info",
                    title="Rename auto-detected during sync",
                    summary=(
                        f"{rename['deprecated_fqn']} was auto-mapped to {rename['candidate_fqn']} "
                        f"with confidence {rename['confidence']:.2f}."
                    ),
                    recommendation="Confirm that the rename preserved behavior and not just structure.",
                    kind="rename-detected",
                    node_fqn=rename["candidate_fqn"],
                    confidence=rename["confidence"],
                    evidence=[
                        f"Body hash match: {rename['evidence']['body_hash_match']}",
                        f"Same directory: {rename['evidence']['same_directory']}",
                    ],
                )
            )

        for file_summary in diff_analysis.get("changed_files", []):
            if file_summary["deleted_symbols"] and not file_summary["changed_nodes"]:
                findings.append(
                    ReviewFinding(
                        id=f"deleted-symbols-{file_summary['file_path']}",
                        severity="high",
                        title="Deleted symbols without surviving mapped nodes",
                        summary=(
                            f"{file_summary['file_path']} deletes "
                            + ", ".join(file_summary["deleted_symbols"])
                            + " without any active replacement node mapped from the diff."
                        ),
                        recommendation="Confirm callers are updated or a rename path exists.",
                        kind="deleted-symbols",
                        file_path=file_summary["file_path"],
                        confidence=0.82,
                        evidence=[
                            f"Deleted symbols: {', '.join(file_summary['deleted_symbols'])}",
                            f"Added symbols: {', '.join(file_summary['added_symbols']) or 'none'}",
                        ],
                    )
                )

        summary = build_summary(diff_analysis, findings)
        return ReviewReport(
            pr_identifier=pr_identifier,
            project_root=str(project_root),
            baseline_cache_path=str(cache_path),
            graph_version=sync_report["version"],
            snapshot=snapshot,
            snapshot_stale=snapshot_stale,
            overlay=overlay,
            sync_report=sync_report,
            diff_analysis=diff_analysis,
            findings=sort_findings(findings),
            warnings=list(dict.fromkeys(warnings)),
            summary=summary,
        )


def build_summary(diff_analysis: dict[str, Any], findings: list[ReviewFinding]) -> dict[str, int]:
    return {
        "changed_files": len(diff_analysis.get("changed_files", [])),
        "changed_nodes": len(diff_analysis.get("changed_node_ids", [])),
        "findings": len(findings),
        "high": sum(1 for finding in findings if finding.severity == "high"),
        "medium": sum(1 for finding in findings if finding.severity == "medium"),
        "low": sum(1 for finding in findings if finding.severity == "low"),
        "info": sum(1 for finding in findings if finding.severity == "info"),
    }


def sort_findings(findings: list[ReviewFinding]) -> list[ReviewFinding]:
    order = {"high": 0, "medium": 1, "low": 2, "info": 3}
    return sorted(findings, key=lambda finding: (order.get(finding.severity, 9), finding.title))
