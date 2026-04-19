# NeuroGraph Architecture

NeuroGraph is split into two hard ownership layers:

- Rust engine: graph schema, validation, Python project scanning, rename detection, snapshots, unresolved-call tracking, diff analysis, query execution, and JSON serialization for PyO3 calls.
- Python brain: CLI, routing policy, review orchestration, agent state machine, and the HTML report layer that turns engine output into something humans can inspect quickly.

## Deliberate completions

Two small additions were necessary to make the written schema enforceable in code:

- `Node.language` is stored on every node so the engine can prove `NodeId == SHA256("{language}::{fqn}")` during construction.
- `SubgraphSummary` includes callee buckets in addition to caller buckets because the spec tracks `total_callees` and imposes a combined 25-node cap.

Both additions are documented in code and tests so the behavior stays explainable instead of becoming accidental drift.

## Current pipeline

The implemented end-to-end flow is:

1. The Rust engine scans a Python project root and extracts module, class, function, and method nodes.
2. The same sync pass resolves internal imports and direct call/instantiation edges, while recording unresolved calls instead of fabricating certainty.
3. Incremental sync deprecates disappeared nodes, auto-accepts high-confidence renames, reroutes inbound edges, and garbage-collects expired deprecated nodes.
4. A Rust diff analyzer maps unified diff hunks back onto active nodes and file summaries.
5. The Python review runner combines sync output, diff analysis, routing warnings, and subgraph queries into findings.
6. The reporting layer renders the review into a standalone interactive HTML artifact with filters, search, summaries, and evidence panels.

## Practical limitations

The current parser is intentionally honest about static uncertainty:

- direct same-module calls, imports, `self.method()` calls, and class instantiations are resolved
- dynamic variable dispatch such as `parser.parse(value)` is surfaced as unresolved unless the engine has enough structural context to prove the target
- the frontend is static HTML with embedded data, which keeps the product dependency-light while still delivering a fast, interactive review experience
