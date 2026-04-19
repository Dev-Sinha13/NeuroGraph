from __future__ import annotations

import argparse
import dataclasses
import json
from pathlib import Path

from .engine import EngineBridge
from .routing import RoutingConfig, validate_routing_config
from .state_machine import AgentSession


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="neurograph",
        description="Inspect NeuroGraph engine configuration and run local review sessions.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    config_parser = subparsers.add_parser("validate-config", help="Validate routing against confidence ranges.")
    config_parser.add_argument("confidence_config", type=Path)
    config_parser.add_argument("--routing-threshold", type=float, default=0.70)
    config_parser.add_argument("--node-threshold", type=int, default=30)
    config_parser.add_argument("--allow-remote-source", action="store_true")

    review_parser = subparsers.add_parser("review", help="Run a local agent session against a graph fixture.")
    review_parser.add_argument("graph_fixture", type=Path, help="JSON file with `nodes` and `edges` lists.")
    review_parser.add_argument("node_id", help="The queried NodeId to seed the review.")
    review_parser.add_argument("--diff-file", type=Path, required=True)
    review_parser.add_argument("--threshold", type=float, default=0.70)

    args = parser.parse_args()

    if args.command == "validate-config":
        confidence_config = json.loads(args.confidence_config.read_text(encoding="utf-8"))
        routing = RoutingConfig(
            escalation_confidence_threshold=args.routing_threshold,
            escalation_node_count_threshold=args.node_threshold,
            privacy_lock=not args.allow_remote_source,
        )
        warnings = validate_routing_config(routing, confidence_config)
        print(
            json.dumps(
                {"warnings": warnings, "routing_config": dataclasses.asdict(routing)},
                indent=2,
            )
        )
        return

    fixture = json.loads(args.graph_fixture.read_text(encoding="utf-8"))
    diff_text = args.diff_file.read_text(encoding="utf-8")
    engine = EngineBridge.create()
    for node in fixture.get("nodes", []):
        engine.upsert_node(node)
    for edge in fixture.get("edges", []):
        engine.write_edge(edge)

    session = AgentSession(
        engine=engine,
        queried_node_id=args.node_id,
        diff_text=diff_text,
        escalation_confidence_threshold=args.threshold,
    )
    initial_state = session.initialize()
    report = {
        "state": initial_state.__class__.__name__,
        "context_messages": len(initial_state.context),
        "queried_node_id": args.node_id,
    }
    print(json.dumps(report, indent=2))
