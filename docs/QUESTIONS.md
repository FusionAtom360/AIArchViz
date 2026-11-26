# Questions

Blocking or open ambiguities, each with options and a recommendation. Claude Code writes; the owner answers.

<!-- Template:
## Q-n: title (task, status: open | answered)
- Context:
- Options:
- Recommendation:
- Answer:
-->

## Q-B1: GPLv3 "only" or "or later" (T-001, status: answered)
- Context: PLAN section 2 and spec D-19 say "GPLv3". The root LICENSE file text is the same either way, but an SPDX `license` field in Cargo.toml and source headers must say `GPL-3.0-only` or `GPL-3.0-or-later`. T-001 left the `license` field out so it would not have to guess.
- Options: (a) GPL-3.0-only; (b) GPL-3.0-or-later.
- Recommendation: (b) GPL-3.0-or-later, the FSF's default recommendation, which keeps future GPL versions open. Once answered, add `license = "..."` to `[workspace.package]`.
- Answer: (b) GPL-3.0-or-later (owner, 2026-11-26). Applied: `license = "GPL-3.0-or-later"` in `[workspace.package]`, inherited by every crate.

## Q-B2: Copyright holder in ue/ArchVizBridge/LICENSE (T-002, status: answered)
- Context: MIT text needs a copyright holder. The plan's byline is "@Joseph", so the file currently says "Copyright (c) 2026 Joseph".
- Options: (a) keep it; (b) your full legal name; (c) a company name.
- Recommendation: (b) or (c), whichever will own the delivered product.
- Answer: Joseph Yakligian (owner, 2026-11-26). Applied to ue/ArchVizBridge/LICENSE.

## Q-B3: PROGRESS.md commit hash versus the task commit (T-002, status: answered)
- Context: PLAN section 1 says each task commits, then appends a PROGRESS line with the commit. A line that holds a commit's hash cannot be part of that same commit.
- Options: (a) append the line after the task commit and commit it with the next task; (b) add a separate `docs: progress T-xxx` commit after each task; (c) record the commit subject (`T-xxx: title`) in place of the hash, so the line goes in the task commit.
- Recommendation: (c). It keeps one commit per task and a clean working tree, and the subject identifies the commit uniquely.
- Interim: in the bootstrap session the T-001 line (with hash) is committed in T-002, and the T-002 line is appended after the T-002 commit and left uncommitted.
- Answer: (c) commit subject in place of the hash (owner, 2026-11-26). Applied: PROGRESS.md rows for T-001 and T-002 rewritten; each future line is committed in its task commit.

## Q-B4: Gate G-1 was incomplete at bootstrap (T-001, status: answered)
- Context: when the bootstrap session started there was no git repository, no wasm32-unknown-unknown target and no cargo-deny. Claude Code ran `git init -b main`, rust-toolchain.toml installed the pinned 1.99.0 toolchain with the wasm32 target, and cargo-deny was installed with `cargo install cargo-deny --locked`. No remote is configured, so T-004 (CI) depends on where the repository will be hosted.
- Options: (a) host on GitHub and add the remote before T-004; (b) host somewhere else.
- Recommendation: (a). T-004 is written for GitHub-style workflows.
- Answer: (a) published on GitHub with `origin` set (owner, 2026-11-26). T-004 targets GitHub Actions.

## Q-1: What "rounded" means for inch input finer than 1/64 in (T-102, status: answered)
- Context: spec 5.1 says "Inch fractions finer than 1/64 in are rounded on input with the error reported". This only matters in the asset-specification context, since the user context rejects anything off the 1/16 in grid (5.3). It does not say what they are rounded to. Some finer fractions are exact in nanometers (1/100 in = 254,000 nm, 1/128 in is not: 198,437.5 nm).
- Options: (a) round to the nearest nanometer, half to even, and report the residual; exact inputs such as 1/100 in stay exact; (b) round to the nearest 1/64 in and report the difference.
- Recommendation: (a). It keeps manufacturer values as exact as storage allows (the residual is at most 0.5 nm) and matches T-101's division rule. Option (b) can move a value by up to 1/128 in (about 0.2 mm).
- Answer: (a) nearest nanometer, half to even, residual reported (owner, 2026-11-26).

## Q-2: Canonical feet-inches text for edge cases (T-102, status: answered)
- Context: spec 5.2 and 5.3 give one example, 10'-4 1/2". The format for values under a foot, whole feet, zero and negatives is not stated, and the echo and display (and later golden echo strings, T-303) depend on it.
- Options: (a) always feet and inches, architectural style as in AutoCAD: 0'-4 1/2", 10'-0", 0'-0", -10'-4 1/2", ~10'-4 1/2"; fractions reduced (1/2, 3/8, 15/16); (b) drop zero feet: 4 1/2", 10'-0", 0"; (c) something else.
- Recommendation: (a). One shape for every value is the most predictable, and it matches the AutoCAD architectural units the client uses (11.1). The ~ goes before the sign: ~-0'-0 1/16".
- Answer: (a) always feet and inches, architectural style; ~ before the sign (owner, 2026-11-26).

## Q-3: What the echo function shows (T-102, status: answered)
- Context: spec 5.2 requires echoing every parsed value in canonical feet-inches-fraction form. 5.3 says an off-grid stored value's exact value is shown "on inspection and in reports". Asset-context values (metric, fine fractions) are usually off the 1/16 in grid, so the canonical text alone (~0'-0 1/2") hides what was stored. Decimal inches cannot show it exactly (1 nm = 1/25,400,000 in, and 25,400,000 has the factor 127), but millimeters can (1 nm = 0.000001 mm).
- Options: (a) echo is exactly format_length; (b) echo is format_length, and for an off-grid value it adds the exact stored value in millimeters, plus the input rounding from Q-1 when there was any; (c) something else.
- Recommendation: (b). On grid: `10'-4 1/2"`. Off grid: `~0'-0 1/2" (exact 12 mm)`. Rounded input: `~0'-0" (exact 0.198438 mm, rounded 0.5 nm from input)`. Millimeters print with trailing zeros removed.
- Answer: (b) canonical text, plus exact millimeters for off-grid values and the input rounding when any (owner, 2026-11-26).
