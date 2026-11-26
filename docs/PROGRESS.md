# Progress

One line per finished task: ID, date, commit subject, notes. The line is committed in the task's own commit (Q-B3).

| ID | Date | Commit | Notes |
| --- | --- | --- | --- |
| T-001 | 2026-11-26 | T-001: Workspace skeleton | Toolchain pinned to 1.99.0; git repo initialized locally (Q-B4). |
| T-002 | 2026-11-26 | T-002: Guardrail files | cargo-deny 0.22.1 installed; questions Q-B1 to Q-B4 raised and answered. |
| T-003 | 2026-11-26 | T-003: The check command | Float lint is comment-aware and counts suffix literals and string contents; check fails fast in order float lint, fmt, clippy, test, wasm32, deny; golden is a no-op placeholder. |
| T-004 | 2026-11-26 | T-004: Continuous integration | GitHub Actions on windows-latest for every push; toolchain from rust-toolchain.toml; cargo-deny 0.22.1 installed with cargo install and cached. |
| T-101 | 2026-11-26 | T-101: Length type | Length(i64) nm with checked add/sub/mul and i128 round-half-to-even div; whole-unit constructors (fractions via div); proptest 1.12.0 added as dev-dependency. |
| T-102 | 2026-11-26 | T-102: Imperial parsing and formatting | parse_length (user and asset contexts, exact rationals), format_length (always ft-in, ~ before sign), echo_length (exact mm off grid); Q-1 to Q-3 answered by owner. |
