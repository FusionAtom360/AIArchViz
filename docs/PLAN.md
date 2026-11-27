# Implementation Plan for Claude Code

Nov 26, 2026 · @Joseph

Claude Code builds the System in six phases and 58 tasks, one task per session, with 11 human gates where only you can verify or decide.

## 1. How to use this plan

Each task fits one Claude Code session, ends with a green check and a commit, and is started with /task T-xxx in a fresh session. Work tasks in ID order unless a task says otherwise, and never start a task before every task it depends on appears in docs/PROGRESS.md.

1. Open a fresh Claude Code session in the repository root.
2. Run /task T-xxx (defined in section 3). Claude Code reads CLAUDE.md, the task entry and the specification sections the task cites.
3. Claude Code writes failing tests, implements, and runs cargo xtask check until it passes.
4. It commits as T-xxx: title, appends one line to docs/PROGRESS.md, pushes to origin, reports the evidence for each Done when line, and stops. It does not start the next task.
5. You review the diff and the evidence. At a human gate (section 4) you do the listed action and report back before the next task starts.
6. After the last task of a phase, run /review-phase N in a fresh session (section 11), triage the findings, and tag the repository phase-N.

Files Claude Code reads and writes:

| File | Purpose | Who edits it |
| --- | --- | --- |
| docs/SPEC.md | The design specification, exported from the first tab of this doc as Markdown. Source of truth. | You, when the spec changes. Claude Code never edits it. |
| docs/PLAN.md | This tab, exported as Markdown. | You |
| docs/PROGRESS.md | One line per finished task: ID, date, commit, notes. | Claude Code |
| docs/QUESTIONS.md | Blocking ambiguities with options and a recommendation. | Claude Code writes, you answer |
| docs/DECISIONS.md | Spike results and any approved deviation from the spec. | Claude Code drafts, you approve |
| docs/DEPENDENCIES.md | Every third-party dependency with license and reason. | Claude Code |

If the specification is ambiguous, contradictory or impossible, Claude Code stops, writes the question to docs/QUESTIONS.md, and waits. It never resolves a specification question by guessing.

## 2. Repository and toolchain

The repository is one Cargo workspace plus an Unreal project, licensed GPLv3 at the root and MIT inside ue/ArchVizBridge. Install these before task T-001 (gate G-1):

- Windows 11 x64 with Git.
- Rust stable, pinned in rust-toolchain.toml, with the targets x86\_64-pc-windows-msvc and wasm32-unknown-unknown.
- Visual Studio 2022 Build Tools with the C++ workload (needed by Rust and by Unreal).
- cargo-deny.
- Claude Code.
- Later, as the phases need them: Unreal Engine 5.8 from the Epic launcher (Phase 4), AutoCAD 2018 (gates G-3), Blender and FreeCAD (Phase 5, optional).

| Path | Spec component | Rules |
| --- | --- | --- |
| crates/core (archviz-core) | C1 | No I/O, no clock, no randomness, no f32 or f64, no unsafe. Integer and rational arithmetic only. Must build for wasm32. |
| crates/export | Export adapters | DXF, glTF and Scene Package writers. Floating point is allowed here and nowhere else except the UI. Depends on core. |
| crates/cli | C2 | Binary named archviz. JSON in, JSON out. |
| crates/mcp | C3 | MCP server over stdio. |
| crates/agent | C4 | Agent loop and provider adapters. |
| crates/app | C5 | egui app (eframe) and wgpu viewer. |
| crates/converters | C6 | Sandboxed runners for CAD, mesh and generator tools. |
| crates/eval | Evaluation harness | Case files and the runner for spec 9.7. |
| xtask | Tooling | cargo xtask check, golden, ue-build, package. |
| schemas | Contracts | JSON Schemas for the document, scene.json and every tool input. |
| fixtures | Test data | L-room, shop, golden files. |
| ue/ArchVizBridge | C7 | UE plugin, MIT, own LICENSE file. Shares no code with the crates. |
| ue/ArchVizHost | Host | UE 5.8 C++ project that hosts the plugin. |
| docs | Documents | The files in section 1. |

Dependency rule: every dependency must be GPLv3-compatible. The allowed license list in deny.toml is MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, CC0-1.0 and Unicode-3.0, enforced by cargo-deny. Crates likely to be used, each to be license-checked before adoption: serde, serde\_json, uuid, thiserror, clap, proptest, insta, eframe and egui, wgpu, image, keyring, windows. Claude Code records every dependency in docs/DEPENDENCIES.md.

## 3. Working rules

Task T-002 commits the three files below verbatim. They are the only instructions Claude Code needs besides the task entry itself.

**3.1 CLAUDE.md**

```markdown
# CLAUDE.md

Project: parametric floor plan to Unreal Engine 5 visualization system.
Spec: docs/SPEC.md. Plan: docs/PLAN.md.
Priorities, in order: precision, predictability, ease of use, realism.

## Workflow
- Work on exactly one task ID per session. Before writing code, read the task in docs/PLAN.md and every spec section it cites.
- Write failing tests first, then implement, then run `cargo xtask check` until it passes.
- Commit as `T-xxx: title`, append one line to docs/PROGRESS.md, push to origin, and stop. Never start the next task.
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
```

**3.2 .claude/commands/task.md**

```markdown
Implement task $ARGUMENTS from docs/PLAN.md.

1. Read CLAUDE.md, the task entry, and each spec section it cites in docs/SPEC.md. Confirm every dependency task appears in docs/PROGRESS.md.
2. State the files you will create or change and the tests you will write.
3. Write failing tests, implement, and run `cargo xtask check` until it is green.
4. For each line of the task's Done when, quote the command output that proves it.
5. Commit, append to docs/PROGRESS.md, push to origin, and stop. Report what changed, the evidence, and any open questions.

Stop and ask if: the spec is ambiguous, a dependency falls outside the license list, a test fails and you cannot explain why, or the task reaches a human gate.
```

**3.3 .claude/commands/review-phase.md**

```markdown
Review phase $ARGUMENTS as an adversarial reviewer with no stake in the code. Fix nothing.

1. Read the spec sections cited by the phase's tasks and the diff since the previous phase tag.
2. For every Done when line and every MUST in those sections, find the test that proves it. List each one that lacks a test.
3. Try to break determinism: run every export twice, and once with the command log in a different order where order should not matter. Compare bytes.
4. Search for floating point in crates/core, unwrap and panic paths, unchecked arithmetic, unsorted map output, and unbounded tool output.
5. Write findings to docs/REVIEW-$ARGUMENTS.md, ranked critical, high, medium, each with file and line.
```

## 4. Human gates

Eleven points need something only you can do: a licensed program, hardware, a secret, or a judgment. Claude Code stops at each one and waits for your report.

| Gate | Before or after | You do | Report back |
| --- | --- | --- | --- |
| G-1 | Before T-001 | Create the repository and install the prerequisites in section 2. Export the first tab of this doc to docs/SPEC.md and this tab to docs/PLAN.md. | Repository path and confirmation |
| G-2 | Before T-121 | Provide the shop's dimensions from the paper plan: exterior outline, interior walls, openings, equipment list with footprints, the two new walls and the extension (roof high edge, slope, pitch or low-edge height). | A list or file, with every measurement you are unsure of marked and a tolerance estimate |
| G-3 | After T-118 | Open the exported DXF in AutoCAD 2018. | Checklist: opens without repair prompt; units read inches; a known wall dimension reads exactly; layers present; overlays on your own drawing with correct registration (S-7) |
| G-4 | After T-122 | Import the GLB into a UE 5.8 editor project by hand and walk the scene. | Fiducials correct (S-3); a known dimension measured in UE; screenshots |
| G-5 | After T-207 | Have a non-expert model a small plan in the app without your help. | Time taken and every point of confusion |
| G-6 | After T-305 | Review the evaluation cases. | Approved or corrected set |
| G-7 | Before T-306 | Store an OpenAI API key in Windows Credential Manager through the app and set a spending limit. | Confirmation only. Never paste the key into chat, files or commits. |
| G-8 | During T-402 | Run the ingest spike on the client-class machine (16 to 32 GB RAM, NVIDIA mid-range GPU). | Load times, frame rate, crashes |
| G-9 | Before T-503 | Provide one real STEP file of a machine and its manufacturer dimensions. | Files and source URL |
| G-10 | During T-505 | Run the mesh generator benchmark on an NVIDIA machine. | Timings, VRAM, quality notes |
| G-11 | During T-408 | Install on the client machine with the installer and have the client reproduce the first use case. | Acceptance notes and issues |

## 5. Phase 0: Bootstrap

Phase 0 produces an empty workspace whose single command, cargo xtask check, enforces every hard rule in CLAUDE.md. Exit: T-001 to T-004 done and CI green.

**T-001 Workspace skeleton.** Spec 3, D-1. Depends on G-1.

- Build: Cargo workspace with every crate in section 2 as an empty library or binary (crate names prefixed archviz-); rust-toolchain.toml; rustfmt.toml; clippy configured to deny warnings; .gitignore; GPLv3 LICENSE at the root.
- Done when: `cargo build --workspace` succeeds and `cargo build -p archviz-core --target wasm32-unknown-unknown` succeeds.

**T-002 Guardrail files.** Spec 2, D-19. Depends on T-001.

- Build: CLAUDE.md, .claude/commands/task.md and .claude/commands/review-phase.md copied verbatim from section 3; empty templates for docs/PROGRESS.md, QUESTIONS.md, DECISIONS.md and DEPENDENCIES.md; ue/ArchVizBridge/LICENSE containing the MIT text; deny.toml with the allowed licenses from section 2.
- Done when: `cargo deny check licenses` passes on the empty workspace and every listed file exists.

**T-003 The check command.** Spec 5.5, 13.2. Depends on T-001.

- Build: `cargo xtask check` runs fmt check, clippy, tests, the wasm32 build of core, cargo deny, and a float lint that fails when f32 or f64 appears in crates/core sources (comments excluded). Add a `cargo xtask golden` placeholder.
- Done when: adding `let x: f64 = 0.0;` to core makes check fail, and removing it makes check pass.

**T-004 Continuous integration.** Depends on T-003.

- Build: a workflow that runs `cargo xtask check` on Windows for every push. If the repository is not on GitHub, write the question to docs/QUESTIONS.md instead of guessing.
- Done when: the workflow file is committed and the first run passes.

## 6. Phase 1: Core and vertical slice

Phase 1 builds the exact core and ends with a vertical slice of the client's shop: a dimensioned DXF and a walkable UE scene generated from constraints. Exit: T-101 to T-122 done, gates G-2, G-3 and G-4 passed, and the results of S-1, S-3 and S-7 recorded in docs/DECISIONS.md. T-107 to T-111 may run in parallel git worktrees after T-106; merge in task order.

**T-101 Length type.** Spec 5.1, D-3. Depends on T-003.

- Build: `Length(i64)` in nanometers in crates/core. Constructors from inches, feet and millimeters. Checked add, subtract and multiply by an integer; divide by an integer through i128 with round half to even. Overflow returns an error, never wraps.
- Done when: proptest shows (a + b) - b equals a across the valid range; 1 in equals 25,400,000 nm; 1/64 in is exact; a rounding table test passes.

**T-102 Imperial parsing and formatting.** Spec 5.2, 5.3. Depends on T-101.

- Build: `parse_length` accepting forms such as 10'-4 1/2", 10 ft 4.5 in, 124.5in and a bare number in the project unit (inches by default). Metric is accepted only in an asset-specification context. In the user context, input finer than 1/16 in is rejected with an error carrying the nearest grid value. `format_length` prints feet-inches-fractions at 1/16 in and prefixes \~ when the stored value is off the grid. An echo function renders any parsed value for confirmation.
- Done when: table tests cover every example form; on-grid values round-trip by property test; off-grid rejection and the \~ prefix are tested.

**T-103 Identity.** Spec 5.5, D-12. Depends on T-001.

- Build: a Uuid wrapper; UUIDv5 derivation under a fixed project namespace constant; a label allocator (W-007, EQ-012 style) that never reuses a label; compression of a UUID to the 22-character IFC GlobalId.
- Done when: derivation is identical across runs; deleted labels are never reissued; the GlobalId function matches published test vectors. Claude Code must find the vectors in buildingSMART or IfcOpenShell documentation and cite them in the test; if it cannot, it writes docs/QUESTIONS.md.

**T-104 Document model and canonical JSON.** Spec 5.5, 6, 6.5 to 6.7. Depends on T-101, T-103.

- Build: Rust types for every entity in spec section 6, including Roof, Underlay and CameraBookmark; serde serialization; schema\_version; a canonical writer (sorted keys, no insignificant whitespace, integers only, BTreeMap throughout); a JSON Schema generated to schemas/document.schema.json.
- Done when: a golden file of a small document exists; parse then write is byte-identical; the schema validates the golden file.

**T-105 Commands, log and undo.** Spec 8.1, 8.2, 5.5. Depends on T-104.

- Build: a Command enum with the v1 set from 8.2 (variants not yet implemented return NotYetImplemented); a pure `apply(document, command)`; a log envelope that keeps timestamps outside the document; snapshots; undo and redo recorded as events.
- Done when: replaying a log yields a byte-identical document; undo restores a byte-identical state; running the same command twice in one process and across two processes agrees.

**T-106 Solver Layer A.** Spec 6.2, 7.1 to 7.5, D-6. Depends on T-104.

- Build: a Solver trait; Layer A as a weighted union-find per axis over reference-plane coordinates; Distance, Align, Equal and Fixed constraints; Driving and Reference status; a degrees-of-freedom report listing each underconstrained class with its members; conflict detection with the tolerance policy of 7.5; node positions derived from solved references.
- Done when: a property test that shuffles constraint order gets identical results; conflicting constraints raise a Conflict naming both; each underconstrained class lists exactly its members; solving 500 elements takes under 50 ms in release mode (record the measured time).

**T-107 Wall geometry.** Spec 6.1. Depends on T-106.

- Build: layer sets, left\_extent, wall footprint polygons, and joins (L mitered; T and X by join\_priority; free ends square) in exact integer rectilinear polygon arithmetic with no floating point.
- Done when: golden polygons exist for L, T and X joins and for a three-layer wall; results do not depend on wall creation order.

**T-108 Openings, doors, windows.** Spec 6, 7.7. Depends on T-107.

- Build: the host and hosted relationship; position along the wall by reference and offset; the void cut in the host profile; sill and head heights; default sizes from the defaults catalog, always flagged Proposed.
- Done when: tests cover position from each opening-edge reference, and an opening outside its host fails validation.

**T-109 Spaces, slabs, ceilings.** Spec 6. Depends on T-107.

- Build: Space boundary derived from the bounding wall faces; Slab and Ceiling from a Space or an explicit rectilinear polygon; a simple-polygon check.
- Done when: rectangular and L-shaped rooms give the correct boundary and area in exact integer arithmetic, and self-intersecting input is rejected.

**T-110 Validity invariants.** Spec 7.7. Depends on T-107, T-108, T-109.

- Build: every check listed in 7.7 returning typed violations, plus the Acknowledged flag.
- Done when: there is one test per invariant, and a move that carries a wall through another wall is caught.

**T-111 Element types and occurrences.** Spec 6, 6.3, 10.1, 10.2. Depends on T-104, T-106.

- Build: ElementType with declared\_dimensions, fidelity and license fields; Occurrence with floor, wall-mounted and ceiling-mounted placement; the Fixed flag and elevation for obstructions; clearance zones; MinClearance evaluated after solving; a parametric box type at the Proxy tier.
- Done when: wall-mounted items follow a moved host wall, floor items stay and are rechecked, and a Fixed item refuses to move.

**T-112 Proposals and commit.** Spec 8.3, 8.4, 9.4. Depends on T-105, T-110, T-111.

- Build: `propose` returning proposal\_id, base\_version, resolved commands, a diff in nanometers and display units, constraint status, violations, ambiguities and flags; `commit` that checks base\_version and is atomic; undo and redo through proposals.
- Done when: a stale base\_version is rejected; propose persists nothing (tested); the diff lists every changed entity.

**T-113 Editing under constraints.** Spec 8.7. Depends on T-112.

- Build: MoveWall, MoveOpening and MoveOccurrence resolved by cases (a) to (d) of 8.7, the consequence list, alternative absorbers as PolicyChoice ambiguities, and a ReferenceSemantics ambiguity for case (c).
- Done when: one test per case passes, and a chain edit lists every downstream entity that moves and offers each alternative with its resulting dimensions.

**T-114 Missing parameters and defaults.** Spec 6.4, 7.4. Depends on T-112.

- Build: a versioned defaults catalog applied with status Proposed; `required_parameters()` ordered by dependency; a provisional-model flag; the list of Proposed items attached to every export.
- Done when: a plan with no wall thickness yields a provisional model and a prompt list, and supplying the value clears its Proposed flag.

**T-115 Shed roof.** Spec 6.5. Depends on T-109.

- Build: a Roof entity with footprint, slope direction, high and low edge heights or pitch as rise:run, overhang, thickness, fascia depth and ledger reference. Heights derived from a pitch use the rounding rule in 5.1. With a ledger reference the slope direction is derived (the lean-to case).
- Done when: a 3:12 pitch over a 12 ft run gives exactly a 3 ft rise; the ledger case derives its direction; a missing pitch or low edge appears in `required_parameters()`.

**T-116 clearance\_resolve.** Spec 8.8. Depends on T-113.

- Build: the operator exactly as written in 8.8, with label-order determinism, shifts rounded to the 1/16 in grid, and a failure report naming the blocking chain. Its output is a proposal.
- Done when: tests cover a single push, a chained push, a chain blocked by a Fixed obstruction (fails and names the chain), and a label-order tie.

**T-117 Command-line interface.** Spec 3, 3.1, C2. Depends on T-112.

- Build: the `archviz` binary with JSON in and out and subcommands new, apply, propose, commit, undo, query, solve, validate and required-parameters; an exclusive project lock; stable error codes; a JSON Schema in schemas/ for every subcommand.
- Done when: golden transcripts pass, and a second process receives the lock error.

**T-118 DXF export.** Spec 11.1. Depends on T-117.

- Build: a writer in crates/export with layers per entity class; wall footprints, openings, slab outline and labeled equipment footprints; driving constraints as dimensions, falling back to dimension lines and text on a DIMENSIONS layer; header variables INSUNITS 1, LUNITS 4 and LUPREC 4; a DXF version AutoCAD 2018 reads (R2013 or older, choice recorded in docs/DECISIONS.md); decimal inches with at least six decimals.
- Done when: an independent DXF reader parses the output; coordinates round-trip to the nanometer within six decimals; a golden file exists. Then gate G-3.

**T-119 glTF export.** Spec 5.4, 11.3. Depends on T-117.

- Build: a GLB writer with one mesh per element in element-local coordinates plus a node transform; the mapping glTF = (x, z, -y) in meters; meshes from wall, slab, roof and opening profiles; equipment as boxes or referenced models.
- Done when: the L-room fiducials land at the expected glTF coordinates; the Khronos validator reports no errors (if it is not installed, say so in docs/DECISIONS.md); output is deterministic.

**T-120 Scene Package.** Spec 11.3. Depends on T-119.

- Build: a writer producing scene.json (schema\_version, unit in nanometers, coordinate system, storeys, elements with ids, labels, transforms and footprints, material slots, variant slots, lights, Proposed items, camera bookmarks), assets/\*.glb and manifest.sha256, plus schemas/scene.schema.json.
- Done when: the same document gives identical bytes on repeat; the schema validates the output; manifest hashes verify.

**T-120b Entity-creating commands.** Spec 8.2, 6, 7.1. Depends on T-117. Added by Q-60.

- Build: arguments, apply, CLI schemas and tests for CreateWall, AddConstraint and PlaceOccurrence; the creation commands 8.2 lacks (CreateDatum, CreateSpace, CreateSlab, CreateCeiling, CreateRoof, CreateElementType), recorded in docs/DECISIONS.md for owner approval.
- Done when: a script of CLI commands alone builds a document with datums, walls, openings, constraints, a space, a slab, a shed roof and box occurrences, and it solves.

**T-121 Fixtures.** Spec 5.4, 13.2. Depends on T-117, T-120b and gate G-2.

- Build: fixtures/l-room (asymmetric, with labeled fiducials) and fixtures/shop, built from your dimensions by a script of CLI commands rather than a hand-edited document. The shop includes both new walls, the extension with its shed roof, and equipment as parametric boxes.
- Done when: the shop solves with no Conflicts, its underconstrained items are listed, and every measurement you marked unsure is a Measured constraint with a tolerance.

**T-122 Vertical slice.** Spec 14 (S-1, S-3, S-7). Depends on T-118 to T-121.

- Build: docs/SLICE.md with the exact steps that regenerate the DXF, GLB and Scene Package for the shop from one command; the S-1 result recorded in docs/DECISIONS.md.
- Done when: outputs regenerate from one command, and you have completed gates G-3 and G-4 and recorded the outcomes.

**T-122b Slice geometry fixes.** Spec 6, 6.1, 6.5, 7.7, D-4; Q-67 to Q-70, DEC-5, DEC-6. Depends on T-122. Runs before gate G-4.

- Build: slab boundary `FromSpaces { spaces }` out to the bounding walls' far faces (DEC-5); roof heights at the underside with the thickness above, and no overhang on a ledger edge (Q-68); `WallTop::Roof { roof }` with integer, round-half-to-even top edges and the new violations (DEC-6); GLB meshes with shared indexed vertices and no normals (Q-70); the shop and L-room fixtures updated to use them (one slab per contiguous floor, L-room roof bearing on its walls at 120 in with a 2:12 pitch and walls following it, extension walls following the roof); fixtures, goldens and schemas regenerated with `cargo xtask golden`.
- Done when: in `cargo xtask slice` output, every slab reaches the walls' exterior faces and nothing is open under a door void; no roof passes through a wall or ceiling and the shop roof starts at the east wall's exterior face; no gap between wall tops and roof underside, shown by a test that checks every roof-following wall's top against the underside to the nanometer; each GLB element has one vertex per distinct position and is a closed solid; reversed wall creation order gives identical wall profiles; then gate G-4.

## 7. Phase 2: App

Phase 2 puts the command API behind an egui app so a non-expert can enter measurements, review proposals and verify dimensions without the command line. Exit: T-201 to T-208 done and gate G-5 passed.

**T-201 App shell and host process.** Spec 3.1, D-14. Depends on T-117.

- Build: an eframe app with open, save and autosave. The app is the host process: it serves the command API over a named pipe with a per-session token, and the CLI attaches to it when it is running.
- Done when: CLI commands issued while the app is open appear in the app, and a concurrency test shows two clients cannot corrupt the project.

**T-202 Plan view.** Spec 3.1, 5.3. Depends on T-201.

- Build: 2D rendering of walls, openings, slabs, occurrences and labels; pan and zoom with 64-bit transforms; snapping to the 1/16 in grid and to references; selection.
- Done when: zooming from the whole building to 1/16 in shows no jitter at either extreme, and a drag yields a command whose value is a snapped integer.

**T-203 Dimensions and missing parameters.** Spec 6.2, 7.4. Depends on T-202.

- Build: add Distance constraints by selecting two references and typing a value with parser echo; highlight underconstrained and conflicting geometry; an interview panel listing required parameters with their proposed defaults.
- Done when: a user can build a rectangular room from four typed dimensions and a wall thickness, and a conflict names both constraints.

**T-204 Proposal review.** Spec 8.3, 8.4, 9.4. Depends on T-202.

- Build: a panel with the diff, violations, ambiguities, Proposed flags, consequence list and alternatives, the echo text, and confirm or discard.
- Done when: no commit path bypasses this panel except the explicit auto\_commit\_unambiguous setting, which is off by default.

**T-205 Underlay.** Spec 6.6. Depends on T-202.

- Build: import a PNG or JPEG; two-point calibration against a known distance; a second known distance reports the residual; transcribed dimensions are entered as Measured constraints with provenance Transcribed.
- Done when: the residual is displayed, and an underlay never changes geometry (tested).

**T-206 3D viewer.** Spec 13.1, D-14. Depends on T-202.

- Build: a wgpu view with orbit and walk navigation, picking, a section plane and point-to-point measurement reported in feet-inches. A view-context provider returns camera pose, heading, selection and hovered entity in the shape the get\_view\_context tool will use.
- Done when: measuring a known wall length reads exactly, and the view context matches the camera (tested).

**T-207 Existing, new, demolished.** Spec 6.7. Depends on T-204.

- Build: a baseline snapshot; function tags; the ExistingConditionEdit flag on proposals; a diff view showing all three states.
- Done when: editing an existing wall flags the proposal and shows the original value. Then gate G-5.

**T-208 Usability pass.** Depends on T-203 to T-207 and gate G-5.

- Build: fix the top issues found at G-5 and add a first-run walkthrough.
- Done when: a non-expert completes the shop slice without help, confirmed by you.

## 8. Phase 3: Agent

Phase 3 builds the tool surface first, proves it through an existing harness over MCP, and only then adds the built-in agent. Exit: T-301 to T-308 done and the S-5 thresholds met, or pivot trigger 3 applied.

**T-301 Query tools.** Spec 9.1, 9.8. Depends on T-117.

- Build: get\_scene\_summary (counts and a region index), get\_entity, find\_entities (region, type, label, nearest to selection), measure, list\_violations and required\_parameters. Every result is bounded to 8 KB with pagination, and strings that came from outside the project are marked untrusted.
- Done when: a 500-element fixture stays within the bound, pagination is tested, and untrusted strings carry the marker.

**T-302 Reference and direction resolution.** Spec 9.2, 9.3. Depends on T-206, T-301.

- Build: resolve\_reference (candidates with confidence) and resolve\_direction as written in 9.3: the camera right vector snapped to the nearest world axis, ambiguous when the minor to major component ratio exceeds 0.7, compass terms only when the user uses them, along-the-wall without a target ambiguous, toward X relative to the named entity.
- Done when: tests cover mirrored views, diagonal views and each ambiguity kind.

**T-303 Edit tools and echo.** Spec 8.3, 9.4. Depends on T-112, T-302.

- Build: propose\_edit, discard\_proposal, commit\_proposal (returns pending\_confirmation until the user confirms in the app), undo, redo and apply\_operator, plus the canonical echo-text renderer.
- Done when: echo text for every v1 command matches golden strings, and commit without confirmation is impossible under default settings.

**T-304 MCP server.** Spec 9.5, 9.6, C3. Depends on T-301, T-303.

- Build: a stdio MCP server exposing every tool, with JSON Schema validation, tier gating T0 to T4, no shell execution, and path confinement to the project directory.
- Done when: an external harness (Claude Code) lists and calls every tool, and a fuzz test shows invalid inputs are rejected before reaching core.

**T-305 Evaluation harness.** Spec 9.7. Depends on T-304.

- Build: crates/eval with a case format (scene fixture, utterance, view context, expected resolved commands or expected clarification kind), a runner with a pluggable backend, five runs per case, the metrics from 9.7, a development and held-out split, and 40 starting cases covering ambiguous referents, approximate quantities, mirrored views and instructions hidden in asset names.
- Done when: the runner executes against a mock backend and reports every metric. Then gate G-6.

**T-306 Built-in agent.** Spec 9.5, D-17. Depends on T-305 and gate G-7.

- Build: the AgentBackend trait; an OpenAI adapter with a tool-use loop calling the same tool set in process; the API key read only from Windows Credential Manager; a data-egress disclosure; context-budget enforcement; a chat panel in the app.
- Done when: with a test key the agent completes three scripted edits and asks a clarifying question on an ambiguous one, and no key appears in logs (tested).

**T-307 Security hardening.** Spec 9.6. Depends on T-306.

- Build: untrusted-data marking end to end; a T3 allowlist and per-domain approval stub; a T4 approval flow stub with a spending cap; injection test cases in which instructions inside asset names or fetched text must not trigger a tier-escalating call without confirmation.
- Done when: every injection case passes, and no tool can write outside the project directory (tested).

**T-308 Evaluation acceptance.** Spec 9.7, S-5. Depends on T-307.

- Build: run the held-out set five times per case against the pinned OpenAI model, tuning tool descriptions and resolution logic but never the held-out cases, and record results in docs/DECISIONS.md.
- Done when: at least 95% of cases are correct in a majority of runs, at least 85% are correct in every run, and no run commits with an unresolved Referent or Direction ambiguity. Otherwise apply pivot trigger 3 from the spec.

## 9. Phase 4: Unreal

Phase 4 proves runtime ingest first, then builds the loader, interaction and installer on whichever path passes. Exit: T-401 to T-408 done, S-2 and S-3 passed in UE, and gates G-8 and G-11 passed. Unreal C++ must build with Visual Studio 2022 and UE 5.8; Claude Code cannot judge visual results, so every visual check ends at a human gate.

**T-401 Host project and plugin scaffold.** Spec 12, D-15. Depends on T-120.

- Build: ue/ArchVizHost (a UE 5.8 C++ project) and the ue/ArchVizBridge plugin (MIT). The first loader, path C, reads scene.json and the vertex and index buffers of the GLB files and builds UE mesh components from them, without re-deriving any geometry. Add `cargo xtask ue-build`, which runs UAT BuildCookRun for Win64.
- Done when: the L-room Scene Package loads in a packaged build, the fiducials match (S-3), and the build runs from one command.

**T-402 Ingest spike.** Spec 12.1, S-2. Depends on T-401.

- Build: paths A (Datasmith Runtime) and B (a glTF runtime plugin) behind the same loader interface as path C; an `xtask` generator for a synthetic 500-element Scene Package; measurements of load time, collision correctness, lights, a variant swap, and reload when only project data changes (no repackaging).
- Done when: a result table for each path is in docs/DECISIONS.md and gate G-8 has been run on the client-class machine.

**T-403 Ingest decision.** Depends on T-402.

- Build: record the chosen path and the fallback chain from the spec risk table in docs/DECISIONS.md; remove unchosen paths or keep them behind a feature flag with a note.
- Done when: you approve the decision.

**T-404 Full loader.** Spec 5.4, 12.1, 12.4. Depends on T-403.

- Build: actors tagged with entity ids; a world-aligned parametric master material with tile size in real-world units; physical lights (lumens, Kelvin, optional IES); collision generated from scene.json footprints, never from imported meshes; the mapping UE = (x, -y, z) in centimeters; an on-screen list of Proposed items.
- Done when: an Unreal automation test shows the fiducials and a known dimension match the document within 1 mm.

**T-405 Interaction.** Spec 12.3, 12.6. Depends on T-404.

- Build: a first-person pawn with collision; a look-at trace; an interact key (default E) that cycles the focused entity's variant slot (materials, lighting presets, pre-imported variants); camera bookmarks; a screenshot key; selections.json written on every change.
- Done when: an automation test cycles each slot type and the written selections.json matches.

**T-406 Selection round trip.** Spec 12.3. Depends on T-405, T-117.

- Build: a CLI command that imports selections.json as SetVariantSelection commands through propose and commit.
- Done when: a selection made in UE appears in the document after import, and the next export reproduces it.

**T-407 Installer.** Spec D-15. Depends on T-405.

- Build: `cargo xtask package` producing a Windows installer with a free tool (record the choice in docs/DEPENDENCIES.md) that installs the app and the packaged UE build side by side with version numbers, per user where possible, and uninstalls both completely.
- Done when: install and uninstall on a clean Windows machine leave no residue.

**T-408 Client acceptance.** Depends on T-407 and gate G-11.

- Build: nothing new. Install on the client machine; the client reproduces the first use case from the paper plan; issues are triaged into docs/QUESTIONS.md or new tasks.
- Done when: you sign off v1.

## 10. Phase 5: Assets, operators and IFC

Phase 5 adds real equipment models, finishing operators and IFC export; once T-501 is done, the other tasks are independent and can run in any order. Exit: T-501 to T-508 done and gates G-9 and G-10 passed.

**T-501 Asset model and normalization.** Spec 10.1, 10.2, 10.4. Depends on T-111.

- Build: fidelity tiers; declared\_dimensions with source; license classes (Unknown is blocked unless the user overrides with a recorded note); provenance; normalization where uniform scale is free, non-uniform scale up to 2% is logged, and anything beyond is rejected or downgraded to Proxy; a Scene Package validator that rejects a model variant whose footprint differs from its slot owner beyond the tolerance.
- Done when: there is a test for each rule.

**T-502 Parametric templates.** Spec 10.1. Depends on T-501.

- Build: parametric types for desk, shelving unit, workbench, toolbox and a labeled machine box, each taking its dimensions as parameters and producing an exact mesh.
- Done when: every template's dimensions equal its declared values exactly, and golden meshes exist.

**T-503 Converters.** Spec 10.5, D-10. Depends on T-501 and gate G-9.

- Build: crates/converters running STEP to mesh (an OpenCascade-based command-line tool such as FreeCAD's) and headless Blender cleanup, with fixed argument templates, a restricted working directory, no network, and time and memory limits through a Windows job object. Core re-validates every output.
- Done when: your real STEP file normalizes to within 0.5% of its declared dimensions (S-4), and a crashing or hung converter is reported as a failed import without freezing the app.

**T-504 Asset acquisition tools.** Spec 10.3, 10.4, 9.1. Depends on T-503, T-307.

- Build: search\_assets and fetch\_asset with an allowlist and per-domain approval; request\_purchase showing price, seller and license, with a per-project cap and checkout performed by the user; license recording; validation of every download before it enters the library.
- Done when: injection tests pass, and a purchase can never complete without a user action.

**T-505 Mesh generator adapter.** Spec 10.6, D-18, S-6. Depends on T-503 and gate G-10.

- Build: an adapter interface; a local runner for the model chosen after benchmarking TRELLIS, TripoSR, Stable Fast 3D and, if its license fits, Hunyuan3D-2 on an NVIDIA GPU under Windows or WSL2; a hosted-adapter stub; output marked fidelity Generated.
- Done when: docs/DECISIONS.md holds a benchmark table (time, VRAM, quality, license), and a photo plus declared dimensions yields a normalized mesh within 0.5%.

**T-506 Feature operators.** Spec 8.5. Depends on T-112.

- Build: the Feature framework with regeneration and UUIDv5 child ids; the operators baseboard, ceiling\_light\_grid, wall\_finish and floor\_finish.
- Done when: moving a wall regenerates its baseboard with stable ids, and regenerating twice gives identical bytes.

**T-507 IFC export.** Spec 4.1, 11.2. Depends on T-117.

- Build: an IFC4 writer for walls with material layer set usage, openings, doors, windows, spaces, furniture proxies, property sets, and GlobalIds from D-12.
- Done when: the file opens without errors in a free viewer (name the viewer in docs/DECISIONS.md) and entity counts match the document.

**T-508 Hardening and documentation.** Depends on every task above.

- Build: a 500-element performance test covering solve, export and load; a non-technical user guide for the client with screenshots; a developer README.
- Done when: the budgets recorded in docs/DECISIONS.md are met and you have reviewed the documents.

## 11. Phase reviews and release

After the last task of each phase, a fresh /review-phase session audits the work before the next phase starts. A review never runs in a session that wrote the code under review.

1. Run /review-phase N in a new Claude Code session. It writes docs/REVIEW-N.md and changes no code.
2. You triage each finding: accept it as a new task (use IDs T-N90 and up, with the same Build and Done when format), or record in docs/DECISIONS.md why it is not needed.
3. Run the new tasks through the normal /task loop.
4. Tag the repository phase-N when the review findings are closed.

**Failure protocol.** If a task fails its Done when twice in separate sessions, stop. Write a root-cause note to docs/QUESTIONS.md and split the task into smaller tasks with new IDs; do not loosen the Done when.

**Release checklist for v1**

- [ ] Every task is in docs/PROGRESS.md and every gate G-1 to G-11 has a recorded outcome.
- [ ] `cargo xtask check` is green on a fresh clone.
- [ ] `cargo deny check` is clean and docs/DEPENDENCIES.md is complete.
- [ ] Spikes S-1 to S-7 have results in docs/DECISIONS.md.
- [ ] Every item in spec section 17 is verified and recorded in docs/DECISIONS.md.
- [ ] Two exports of the shop fixture on two different machines are byte-identical.
- [ ] The installer is tested on the client machine and the client's acceptance is recorded.
- [ ] The source is published under GPLv3 with the MIT license file inside ue/ArchVizBridge.
- [ ] Every critical and high finding from the phase reviews is closed.
