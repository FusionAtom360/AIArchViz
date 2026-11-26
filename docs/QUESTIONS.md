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
