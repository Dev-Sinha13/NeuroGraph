from __future__ import annotations

import json
import uuid
import unittest
from pathlib import Path

from neurograph import (
    EngineBridge,
    RoutingConfig,
    ReviewRunner,
    render_review_report_html,
    validate_routing_config,
)
from neurograph.state_machine import (
    AgentSession,
    BudgetConfig,
    CompleteState,
    ForcedTerminationState,
    ToolCallCompleteState,
    ToolCallFailedState,
)


def make_node(language: str, fqn: str, *, body_hash: str = "a" * 64) -> dict:
    import hashlib

    node_id = hashlib.sha256(f"{language}::{fqn}".encode("utf-8")).hexdigest()
    return {
        "id": node_id,
        "language": language,
        "name": fqn.rsplit(".", 1)[-1],
        "fqn": fqn,
        "kind": "Function",
        "file_path": f"src/{fqn.replace('.', '/')}.py",
        "location": {"start_line": 1, "end_line": 8},
        "status": "Active",
        "signature": {
            "PartiallyTyped": {
                "params": [
                    {
                        "name": "value",
                        "type_annotation": None,
                        "has_default": False,
                    }
                ],
                "return_type": None,
                "is_async": False,
            }
        },
        "body_hash": body_hash,
        "introduced_at_version": 1,
    }


class NeuroGraphPythonTests(unittest.TestCase):
    @staticmethod
    def _temp_path(name: str) -> Path:
        root = Path.cwd() / ".tmp-tests"
        root.mkdir(exist_ok=True)
        path = root / f"{name}-{uuid.uuid4().hex}"
        path.mkdir()
        return path

    def setUp(self) -> None:
        self.engine = EngineBridge.create()
        self.target = make_node("python", "pkg.target")
        self.caller = make_node("python", "pkg.caller")
        self.callee = make_node("python", "pkg.callee")
        for node in (self.target, self.caller, self.callee):
            self.engine.upsert_node(node)
        self.engine.write_edge(
            {
                "source": self.caller["id"],
                "target": self.target["id"],
                "kind": "Calls",
                "confidence": 0.8,
                "resolution": "TypeInferred",
                "introduced_at_version": 1,
            }
        )
        self.engine.write_edge(
            {
                "source": self.target["id"],
                "target": self.callee["id"],
                "kind": "Calls",
                "confidence": 0.4,
                "resolution": "Heuristic",
                "introduced_at_version": 1,
            }
        )

    def test_routing_warnings_cover_uncalibrated_and_threshold_mismatch(self) -> None:
        warnings = validate_routing_config(
            RoutingConfig(escalation_confidence_threshold=0.7),
            self.engine.confidence_config(),
        )
        self.assertEqual(len(warnings), 2)

    def test_agent_session_runs_happy_path(self) -> None:
        session = AgentSession(
            engine=self.engine,
            queried_node_id=self.target["id"],
            diff_text="diff --git a.py b.py",
            escalation_confidence_threshold=0.7,
        )
        initial = session.initialize()
        self.assertEqual(initial.__class__.__name__, "InitialAnalysisState")
        tool_state = session.handle_tool_call(
            {"tool": "get_node_detail", "node_id": self.target["id"]}
        )
        self.assertIsInstance(tool_state, ToolCallCompleteState)
        complete = session.complete(["Potential regression in pkg.target"])
        self.assertIsInstance(complete, CompleteState)
        self.assertEqual(complete.findings[0], "Potential regression in pkg.target")

    def test_snapshot_staleness_round_trips_through_bridge(self) -> None:
        snapshot = self.engine.create_snapshot("PR-42")
        self.assertFalse(self.engine.snapshot_is_stale(snapshot, 1))
        self.assertTrue(self.engine.snapshot_is_stale(snapshot, 2))

    def test_agent_session_forces_termination_on_iteration_budget(self) -> None:
        session = AgentSession(
            engine=self.engine,
            queried_node_id=self.target["id"],
            diff_text="diff",
            escalation_confidence_threshold=0.7,
            budget=BudgetConfig(max_iterations=0),
        )
        session.initialize()
        state = session.handle_tool_call(
            {"tool": "get_node_detail", "node_id": self.target["id"]}
        )
        self.assertIsInstance(state, ForcedTerminationState)
        report = session.partial_report()
        self.assertEqual(report["status"], "[PARTIAL]")

    def test_agent_session_reports_schema_failures(self) -> None:
        session = AgentSession(
            engine=self.engine,
            queried_node_id=self.target["id"],
            diff_text="diff",
            escalation_confidence_threshold=0.7,
        )
        session.initialize()
        state = session.handle_tool_call({"node_id": self.target["id"]})
        self.assertIsInstance(state, ToolCallFailedState)
        self.assertEqual(state.error["error"], "INVALID_SCHEMA")

    def test_review_runner_emits_findings_from_real_project_sync(self) -> None:
        project = self._temp_path("project")
        diff_file = project / "change.diff"
        (project / "app.py").write_text(
            "def run(value):\n    return removed_helper(value)\n",
            encoding="utf-8",
        )
        diff_file.write_text(
            "\n".join(
                [
                    "diff --git a/app.py b/app.py",
                    "--- a/app.py",
                    "+++ b/app.py",
                    "@@ -1,1 +1,1 @@",
                    "-def run(value):",
                    "+def run(value):",
                    "-    return helper(value)",
                    "+    return removed_helper(value)",
                ]
            ),
            encoding="utf-8",
        )

        runner = ReviewRunner()
        report = runner.run(project, diff_file.read_text(encoding="utf-8"), "PR-99")
        self.assertGreaterEqual(report.summary["findings"], 1)
        self.assertTrue(
            any(finding.kind == "unresolved-calls" for finding in report.findings)
        )

    def test_render_review_report_html_contains_interactive_payload(self) -> None:
        project = self._temp_path("report-project")
        (project / "app.py").write_text("def run(value):\n    return value\n", encoding="utf-8")
        diff_text = "\n".join(
            [
                "diff --git a/app.py b/app.py",
                "--- a/app.py",
                "+++ b/app.py",
                "@@ -1,1 +1,1 @@",
                "-def run(value):",
                "+def run(value):",
            ]
        )
        report = ReviewRunner().run(project, diff_text, "PR-HTML")
        html = render_review_report_html(report)
        self.assertIn("NeuroGraph review for PR-HTML", html)
        self.assertIn("Interactive review output", html)
        self.assertIn('"pr_identifier": "PR-HTML"', html)

    def test_cli_validate_config_outputs_json(self) -> None:
        from neurograph.cli import main
        import contextlib
        import io
        import sys

        temp_root = Path.cwd() / ".tmp-tests"
        temp_root.mkdir(exist_ok=True)
        config_path = temp_root / "confidence.json"
        config_path.write_text(json.dumps(self.engine.confidence_config()), encoding="utf-8")
        stdout = io.StringIO()
        argv = sys.argv
        sys.argv = [
            "neurograph",
            "validate-config",
            str(config_path),
        ]
        try:
            with contextlib.redirect_stdout(stdout):
                main()
        finally:
            sys.argv = argv
            if config_path.exists():
                config_path.unlink()
        payload = json.loads(stdout.getvalue())
        self.assertIn("warnings", payload)

    def test_cli_review_project_outputs_json_report(self) -> None:
        from neurograph.cli import main
        import contextlib
        import io
        import sys

        project = self._temp_path("cli-project")
        diff_path = project / "change.diff"
        (project / "app.py").write_text(
            "def run(value):\n    return missing_call(value)\n",
            encoding="utf-8",
        )
        diff_path.write_text(
            "\n".join(
                [
                    "diff --git a/app.py b/app.py",
                    "--- a/app.py",
                    "+++ b/app.py",
                    "@@ -1,1 +1,1 @@",
                    "-def run(value):",
                    "+def run(value):",
                ]
            ),
            encoding="utf-8",
        )

        stdout = io.StringIO()
        argv = sys.argv
        sys.argv = [
            "neurograph",
            "review-project",
            str(project),
            "--diff-file",
            str(diff_path),
            "--pr-id",
            "PR-CLI",
        ]
        try:
            with contextlib.redirect_stdout(stdout):
                main()
        finally:
            sys.argv = argv
        payload = json.loads(stdout.getvalue())
        self.assertEqual(payload["pr_identifier"], "PR-CLI")
        self.assertIn("summary", payload)


if __name__ == "__main__":
    unittest.main()
