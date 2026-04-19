# NeuroGraph

NeuroGraph is a two-layer code review runtime:

- a Rust engine, compiled into a Python extension with PyO3, that owns graph schema, project scanning, Python AST-style extraction, incremental sync, rename detection, diff mapping, and graph queries
- a Python brain that owns routing policy, review orchestration, CLI ergonomics, the agentic state machine, and the interactive standalone report experience

This repository is intentionally strict about invariants. The schema is not treated as documentation alone; constructors and validators enforce it directly and tests cover the edge cases that matter for review correctness.

## What ships here

- `src/`: the Rust engine and PyO3 bridge
- `python/neurograph/`: the Python orchestration layer
- `python/neurograph/review.py`: the real review runner that syncs a project, analyzes a diff, and emits findings
- `python/neurograph/reporting.py`: the interactive HTML report generator
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

5. Run a real project review:

```bash
neurograph review-project path\to\project --diff-file change.diff --pr-id PR-123
```

6. Generate the standalone interactive HTML report:

```bash
neurograph render-report path\to\project --diff-file change.diff --pr-id PR-123 --output review.html
```

7. Validate routing against a confidence config:

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
- The Rust engine can now scan Python projects directly, build a graph, detect unresolved calls, diff-map changed nodes, and produce review-ready sync metadata.
- The Python review runner emits findings from real engine output and can render a polished interactive report without a web build step.

## CLI

The current CLI focuses on the pieces defined by the schema:

- `neurograph validate-config <confidence-config.json>`
- `neurograph review <graph-fixture.json> <node-id> --diff-file <diff.txt>`
- `neurograph review-project <project-root> --diff-file <diff.txt> --pr-id <id>`
- `neurograph render-report <project-root> --diff-file <diff.txt> --pr-id <id> --output <report.html>`

The fixture-based `review` command is still useful for state-machine testing. The product path is `review-project`, which runs the real Rust-backed sync and diff pipeline, then emits a structured JSON report. `render-report` turns that same report into a responsive, filterable HTML artifact you can open locally.

## Current analysis scope

The implemented parser currently targets Python code. It handles modules, classes, functions, methods, imports, direct internal calls, constructor-style instantiations, unresolved call tracking, incremental renames, and diff-to-node mapping. Dynamic variable dispatch is intentionally surfaced as unresolved instead of being guessed away.

## Tested locally

The implementation in this repository was verified with:

- `cargo test`
- `python -m unittest discover -s tests -v`

Both suites exercise the real Rust schema/validator code. The Python suite runs through the compiled PyO3 module rather than a mock engine.
