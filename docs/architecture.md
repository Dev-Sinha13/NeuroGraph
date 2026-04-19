# NeuroGraph Architecture

NeuroGraph is split into two hard ownership layers:

- Rust engine: graph schema, validation, rename detection, snapshots, query execution, and JSON serialization for PyO3 calls.
- Python brain: CLI, routing policy, agent state machine, and the orchestration layer that decides what to ask the engine for next.

## Deliberate completions

Two small additions were necessary to make the written schema enforceable in code:

- `Node.language` is stored on every node so the engine can prove `NodeId == SHA256("{language}::{fqn}")` during construction.
- `SubgraphSummary` includes callee buckets in addition to caller buckets because the spec tracks `total_callees` and imposes a combined 25-node cap.

Both additions are documented in code and tests so the behavior stays explainable instead of becoming accidental drift.

