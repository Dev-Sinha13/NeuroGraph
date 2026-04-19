from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Protocol


class EngineProtocol(Protocol):
    def get_subgraph(self, node_id: str, escalation_confidence_threshold: float, max_nodes: int = 25) -> Any: ...

    def get_node_detail(self, node_id: str) -> dict[str, Any]: ...


@dataclass(slots=True)
class InitializingState:
    diff_text: str
    queried_node_id: str


@dataclass(slots=True)
class InitialAnalysisState:
    iteration: int
    context: list[dict[str, Any]] = field(default_factory=list)


@dataclass(slots=True)
class ValidatingToolCallState:
    iteration: int
    tool_call: dict[str, Any]


@dataclass(slots=True)
class ToolCallPendingState:
    iteration: int
    tool_call: dict[str, Any]


@dataclass(slots=True)
class ToolCallFailedState:
    iteration: int
    error: dict[str, Any]


@dataclass(slots=True)
class ToolCallCompleteState:
    iteration: int
    result: dict[str, Any]


@dataclass(slots=True)
class ForcedTerminationReason:
    label: str


@dataclass(slots=True)
class TokenBudgetExceeded:
    tokens_used: int
    limit: int
    label: str = "TokenBudgetExceeded"


@dataclass(slots=True)
class IterationLimitReached:
    iterations: int
    limit: int
    label: str = "IterationLimitReached"


@dataclass(slots=True)
class ForcedTerminationState:
    reason: TokenBudgetExceeded | IterationLimitReached
    findings: list[str]
    untraversed_regions: list[str]


@dataclass(slots=True)
class CompleteState:
    findings: list[str]


@dataclass(slots=True)
class BudgetConfig:
    max_tokens: int = 24_000
    max_iterations: int = 12


class AgentSession:
    def __init__(
        self,
        engine: EngineProtocol,
        queried_node_id: str,
        diff_text: str,
        escalation_confidence_threshold: float,
        budget: BudgetConfig | None = None,
    ) -> None:
        self.engine = engine
        self.queried_node_id = queried_node_id
        self.diff_text = diff_text
        self.escalation_confidence_threshold = escalation_confidence_threshold
        self.budget = budget or BudgetConfig()
        self.tokens_used = 0
        self.iterations = 0
        self.findings: list[str] = []
        self.context: list[dict[str, Any]] = []
        self.state: Any = InitializingState(diff_text=diff_text, queried_node_id=queried_node_id)

    def initialize(self) -> InitialAnalysisState | ForcedTerminationState:
        summary = self.engine.get_subgraph(
            self.queried_node_id,
            self.escalation_confidence_threshold,
        )
        self.context.append(
            {
                "role": "system",
                "diff": self.diff_text,
                "subgraph": summary,
            }
        )
        self.state = InitialAnalysisState(iteration=self.iterations, context=self.context.copy())
        return self.state

    def consume_tokens(self, tokens: int) -> ForcedTerminationState | None:
        self.tokens_used += tokens
        if self.tokens_used > self.budget.max_tokens:
            self.state = ForcedTerminationState(
                reason=TokenBudgetExceeded(self.tokens_used, self.budget.max_tokens),
                findings=self.findings.copy(),
                untraversed_regions=[self.queried_node_id],
            )
            return self.state
        return None

    def handle_tool_call(self, tool_call: dict[str, Any]) -> Any:
        self.iterations += 1
        if self.iterations > self.budget.max_iterations:
            self.state = ForcedTerminationState(
                reason=IterationLimitReached(self.iterations, self.budget.max_iterations),
                findings=self.findings.copy(),
                untraversed_regions=[self.queried_node_id],
            )
            return self.state

        self.state = ValidatingToolCallState(iteration=self.iterations, tool_call=tool_call)
        if "tool" not in tool_call:
            error = {"error": "INVALID_SCHEMA", "detail": "tool call is missing the `tool` field"}
            self.context.append({"role": "tool", "payload": error})
            self.state = ToolCallFailedState(iteration=self.iterations, error=error)
            return self.state

        self.state = ToolCallPendingState(iteration=self.iterations, tool_call=tool_call)
        try:
            result = self._execute_tool(tool_call)
        except Exception as exc:  # pragma: no cover - exercised by integration tests
            error = self._try_parse_json_error(str(exc))
            self.context.append({"role": "tool", "payload": error})
            self.state = ToolCallFailedState(iteration=self.iterations, error=error)
            return self.state

        self.context.append({"role": "tool", "payload": result})
        self.state = ToolCallCompleteState(iteration=self.iterations, result=result)
        return self.state

    def complete(self, findings: list[str]) -> CompleteState:
        self.findings = findings
        self.state = CompleteState(findings=findings)
        return self.state

    def partial_report(self) -> dict[str, Any]:
        if not isinstance(self.state, ForcedTerminationState):
            raise RuntimeError("partial_report is only available after forced termination")
        return {
            "status": "[PARTIAL]",
            "reason": self.state.reason.label,
            "findings": self.state.findings,
            "untraversed_regions": self.state.untraversed_regions,
        }

    def _execute_tool(self, tool_call: dict[str, Any]) -> dict[str, Any]:
        tool = tool_call["tool"]
        if tool == "get_node_detail":
            return self.engine.get_node_detail(tool_call["node_id"])
        if tool == "get_subgraph":
            summary = self.engine.get_subgraph(
                tool_call["node_id"],
                self.escalation_confidence_threshold,
                tool_call.get("max_nodes", 25),
            )
            return {
                "queried_node_id": summary.queried_node_id,
                "total_callers": summary.total_callers,
                "total_callees": summary.total_callees,
            }
        raise ValueError('{"error":"INVALID_SCHEMA","detail":"unknown tool"}')

    @staticmethod
    def _try_parse_json_error(message: str) -> dict[str, Any]:
        import json

        try:
            return json.loads(message)
        except json.JSONDecodeError:
            return {"error": "UNKNOWN", "detail": message}
