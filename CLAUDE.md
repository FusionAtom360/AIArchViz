# CLAUDE.md

Project: parametric floor plan to Unreal Engine 5 visualization system.
Spec: docs/SPEC.md. Plan: docs/PLAN.md.
Priorities, in order: precision, predictability, ease of use, realism.

## Workflow
- Work on exactly one task ID per session. Before writing code, read the task in docs/PLAN.md and every spec section it cites.
- Write failing tests first, then implement, then run `cargo xtask check` until it passes.
- Commit as `T-xxx: title`, append one line to docs/PROGRESS.md, and stop. Never start the next task.
- If the spec is ambiguous, contradictory or impossible, stop and write docs/QUESTIONS.md with options and a recommendation. Never guess.
- Never edit docs/SPEC.md. A deviation needs an entry in docs/DECISIONS.md approved by the owner.
- Do not add features, options or abstractions the task does not ask for.

## Hard rules
- crates/core: no I/O, no clock, no randomness, no f32 or f64, no unsafe, no panics on bad input (return Result).
- Lengths are i64 nanometers inside a Length type. Never pass raw integers for lengths.
- Same input gives byte-identical output. Use BTreeMap, sort before serializing, never depend on hash order.
- The agent never computes coordinates. Every displacement comes from deterministic code (spec P2).
- Every user-visible length is shown in feet-inches-fractions at 1/16 in; off-grid values get a leading ~.
- A new dependency goes in docs/DEPENDENCIES.md with license and reason, and `cargo deny check` must pass.
- crates and ue/ArchVizBridge share no code. The Scene Package is the only contract between them.
- A task without tests is not done. Unit tests sit beside code, proptest covers invariants, insta covers golden files.
- Never write secrets (API keys, tokens) to files, logs, commits or chat.

## Commands
- `cargo xtask check`: fmt, clippy with warnings denied, tests, wasm32 build of core, float lint, cargo deny.
- `cargo xtask golden`: regenerate golden files. Review every diff by hand before committing.
