# NeuroGraph

NeuroGraph is a two-layer code review runtime:

- a Rust engine, compiled into a Python extension with PyO3, that owns graph schema, validation, rename detection, snapshots, and graph queries
- a Python brain that owns routing policy, CLI ergonomics, and the agentic state machine

This repository is intentionally strict about invariants. The schema is not treated as documentation alone; constructors and validators enforce it directly and tests cover the edge cases that matter for review correctness.

## What ships here

- `src/`: the Rust engine and PyO3 bridge
- `python/neurograph/`: the Python orchestration layer
- `tests/`: Python integration and state-machine tests
- `docs/architecture.md`: architecture notes and rationale for the two deliberate schema completions

## Quick start

1. Install `maturin`, then install the package in editable mode:

```bash
python -m pip install maturin
python -m pip install -e .
```

2. If PyO3 cannot discover a Python interpreter on your `PATH`, point it at the interpreter you want to bind against:

```bash
set PYO3_PYTHON=C:\path\to\python.exe
```

3. Run the Rust test suite:

```bash
cargo test
```

4. Run the Python suite:

```bash
set PYTHONPATH=%CD%\python;%CD%
python -m unittest discover -s tests -v
```

5. Validate routing against a confidence config:

```bash
neurograph validate-config confidence.json
```

## Design highlights

- `NodeId` is a dedicated type, hashed from `"{language}::{fqn}"`, not a string alias.
- `Signature` is never optional. Parsers must emit the least-specific valid variant.
- `Edge.confidence` is private and only exposed through `confidence()`.
- `SchemaValidator` is the gatekeeper for every edge write. Type-inferred and heuristic confidence checks live there so runtime calibration data never gets hardcoded into constructors.
- `GraphSnapshot.is_stale()` and rename auto-accept thresholds are implemented and tested.
- Tool-call failures produce structured JSON payloads that the Python state machine can append directly into LLM context.

## CLI

The current CLI focuses on the pieces defined by the schema:

- `neurograph validate-config <confidence-config.json>`
- `neurograph review <graph-fixture.json> <node-id> --diff-file <diff.txt>`

The review command loads a graph fixture, initializes the agent session, and prints the initial orchestration state. That makes the whole Rust/Python stack inspectable without requiring a live model provider during local development.

## Tested locally

The implementation in this repository was verified with:

- `cargo test`
- `python -m unittest discover -s tests -v`

Both suites exercise the real Rust schema/validator code. The Python suite runs through the compiled PyO3 module rather than a mock engine.
