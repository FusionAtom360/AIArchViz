# Review: Phase 1 (Core and vertical slice)

Adversarial review, 2026-11-27, of T-101 to T-122b. No code was changed.

- **Diff base:** there is no `phase-0` tag (`git tag` is empty), so the base is `447c2b0` (the last commit before T-101) and the head is `af60f73` (T-122b). That is 91 files and about 59,000 lines.
- **Baseline:** `cargo xtask check` passes. One test is ignored: `solver_500_elements_under_50_ms` (see M-2).
- **File name:** PLAN section 11 says the review goes in `docs/REVIEW-N.md`. This file uses the name the review command gave. Rename it to `docs/REVIEW-1.md` if that matters.

## Summary

| Rank | Count | IDs |
| --- | --- | --- |
| Critical | 2 | C-1, C-2 |
| High | 5 | H-1 to H-5 |
| Medium | 10 | M-1 to M-10 |

Determinism held in every experiment below. The worst problems are (1) the 5.3 grid rule is only enforced in the text parser, not in commands, and (2) the phase exit criteria are not met yet.

---

## Critical

### C-1: User-stated lengths off the 1/16 in grid are accepted through commands (spec 5.3 MUST)
- **Status: closed** by T-190 (`73002b6`, 2026-11-27). `propose` rejects off-grid user-stated values with `CommandError::OffGrid` and the nearest grid value; covered by `off_grid_user_stated_values_are_rejected_with_the_nearest_grid_value`, `an_off_grid_user_stated_value_is_rejected` and `an_off_grid_user_stated_constraint_is_rejected`.
- **Where:** `crates/core/src/create.rs:227` (`fill_constraint`), and every command argument of type `Sourced<Length>` with `source: user_stated` (CreateWall layers, left_extent, base_offset; CreateOpening; CreateRoof; AddConstraint; SetConstraintValue). The grid check exists only in `parse_length` (`crates/core/src/units.rs`), which commands never call.
- **Spec:** 5.3 says "User-stated inputs MUST lie on the 1/16 in grid: finer input is rejected and the nearest grid value is echoed". CLAUDE.md lists precision as the first priority.
- **Repro:** I replayed `fixtures/l-room/commands.txt` with the first non-zero `add_constraint` value changed from 6096000000 to 6096000001 (240 in + 1 nm, source `user_stated`). Every line committed with exit 0, and the document holds the off-grid UserStated value.
- **Impact:** the CLI (C2) and the future MCP tools (C3) take raw nanometers, so nothing stops an agent or script from committing an off-grid "user-stated" value. That value then displays as `~` but carries the highest authority (D-11).
- **Missing test:** a command-level test that rejects off-grid UserStated values with the nearest grid value.

### C-2: Phase 1 exit criteria are not met
- **Status: closed** (2026-11-27), per Q-72 (a): T-191 done (`adf53d5`), gate G-4 passed, S-1 (DEC-4) and S-3 (DEC-7) approved by the owner, S-7 in DEC-1. `phase-1` tagged.
- **Where:** `docs/DECISIONS.md` (DEC-4 is `Status: draft`), `docs/PROGRESS.md` (T-122 and T-122b both end "Next: gate G-4"), `docs/SLICE.md` (G-4 is a checklist with no result).
- **Plan:** "Exit: T-101 to T-122 done, gates G-2, G-3 and G-4 passed, and the results of S-1, S-3 and S-7 recorded in docs/DECISIONS.md."
- **Status:**
  - G-4 has not passed.
  - S-3 is not recorded anywhere in DECISIONS.md.
  - S-1 (DEC-4) is still a draft.
  - S-3 also cannot pass for DXF as built (H-2).
- **Impact:** do not tag `phase-1` yet.

---

## High

### H-1: Unchecked i64 subtraction in the wall-length check: panics in debug, wraps in release
- **Where:** `crates/core/src/validity.rs:108-124`. The pattern `b.x.nanometers() - a.x.nanometers()` and its siblings are raw i64 arithmetic.
- **Rule:** CLAUDE.md says core has "no panics on bad input". T-101 says overflow "never wraps".
- **Repro:** I replayed the first 18 lines of the shop script with node 1 at x = -9223372036854775807 and node 2 at x = 9223372036854775807, then created the `pos_x` wall W-001 between them.
  - Debug build: `panicked at crates\core\src\validity.rs:108:17`.
  - Release build: no panic, but the result is wrong. The proposal reports `non_positive_length {"length": -2}` for a wall that is about 1.8e19 nm long.
- **Note:** the same function's other arithmetic goes through i128. Only these lines do not.
- **Missing test:** a validity test with node coordinates at the i64 limits.

### H-2: The DXF has no fiducials, so S-3 and the 5.4 conformance fixture are untested for DXF
- **Status: closed** by T-191 (`adf53d5`, 2026-11-27). The DXF writes Point datums on a DATUMS layer; covered by `l_room_fiducials_read_back_from_the_dxf` and `l_room_dxf_fiducials_read_back_exactly`.
- **Where:** `crates/export/src/dxf.rs:400` (`write_dxf`). No datums or fiducial points are written (no reference to datums anywhere in the file), and no test uses the L-room.
- **Spec:**
  - 5.4: the L-room fixture "MUST pass in every exporter".
  - S-3: "L-room fiducials match in DXF, glTF and UE".
- **Coverage elsewhere:** glTF has `l_room_fiducials_land_at_the_expected_gltf_coordinates`. The Scene Package has `header_storeys_and_fiducials`. DXF has nothing.
- **G-3:** the owner's overlay check was on the shop, not the L-room fiducials.

### H-3: A torn log line makes the project unopenable
- **Where:**
  - `crates/cli/src/store.rs:102`: `append` writes the line in place, with no temp file and rename and no length or checksum framing.
  - `crates/cli/src/store.rs:80`: `open` fails on any bad line.
- **Spec:** 8.4 says "Commit is atomic". Core's `commits_are_atomic` covers memory only, not disk.
- **Repro:** I copied a replayed L-room project and appended the partial line `{"kind":"commit","seq":` to `log.jsonl`. Every subcommand then returns `CORRUPT_PROJECT`, and there is no way to recover the 26 good entries.
- **Impact:** a crash or power loss during `apply`, `commit` or `undo` bricks the project.
- **Missing test:** crash-consistency of the log on disk.

### H-4: `new` can wipe an existing log (check-then-lock race)
- **Where:** `crates/cli/src/store.rs:51-62`.
  - Line 51 checks for `genesis.json` before line 58 takes the lock.
  - After taking the lock it does not check again.
  - Line 62 then truncates `log.jsonl` unconditionally.
- **Repro:** I deleted `genesis.json` from a replayed project that had a 26-entry log and ran `archviz new`. It returned ok, and `log.jsonl` was 0 bytes.
- **The race:** two concurrent `new` calls on one folder can both pass the line-51 check. The second then overwrites the first project's genesis and log after the first releases the lock.
- **Missing test:** `new` against a folder that already holds a log, and `new` racing `new`.

### H-5: DXF and GLB exports do not list Proposed items (spec 7.4 MUST, T-114 Build)
- **Where:**
  - `crates/export/src/dxf.rs:400` and `crates/export/src/gltf.rs:704`: neither file mentions Proposed values.
  - `crates/export/src/scene.rs` does list them (`proposed_items`).
- **Spec:**
  - 7.4: "Every export of a provisional model MUST list its Proposed items."
  - T-114 Build: "the list of Proposed items attached to every export".
- **Impact:** the shop has 92 Proposed values. Its `shop.dxf` and `shop.glb` give no sign that the model is provisional.
- **Missing test:** each exporter carries the `proposed_items` list for a provisional document.

---

## Medium

### M-1: No property test for inverse deltas (spec 13.2)
13.2 says: "for move and set-value commands the inverse delta does too [restore a byte-identical document]". The repository has no such test: `grep -i inverse crates/core/src` finds nothing. The undo tests (`undo_restores_the_exact_prior_state`, `replay_and_undo_are_exact`) cover stored-state undo, not applying the inverse delta of MoveWall, MoveOpening, MoveOccurrence or SetConstraintValue.

### M-2: The T-106 timing requirement is never enforced
- **Where:** `xtask/src/main.rs:358`.
- **Problem:** `solver_500_elements_under_50_ms` is `#[ignore]`, and `cargo xtask check` runs only debug tests. The "under 50 ms in release" Done-when line rests on one manual measurement in PROGRESS.md.
- **Also:** the workload is "500 walls / 1,500 constraints". It is not 500 mixed elements with openings and occurrences, which the solver now also handles (T-108, T-111).

### M-3: The Khronos validator result is stale
- DEC-2 validated the golden GLB with SHA-256 `0ebc5c07…`.
- The current golden, `crates/export/src/gltf/snapshots/archviz_export__gltf__tests__golden_l_room_glb.snap.glb`, has SHA-256 `f2883ce9…`.
- T-122b changed the mesh format: welded vertices, no normals, and walls as cell-grid solids. Nobody has re-run the validator on the format that G-4 will import.

### M-4: The float lint misses unsuffixed literals and identifiers that contain f64
- **Where:** `xtask/src/float_lint.rs:65-91`. A token preceded by an identifier character is flagged only when the word starts with a digit.
- **What gets through:**
  - `let x = 0.5;` has no `f64` token at all.
  - `v.as_f64()`, `f64_from(...)` and `n.to_f64()` all have a letter before `f64`.
- **Today:** core contains none of these (I searched). But CLAUDE.md's "no f32 or f64" rule is weaker than it looks. `core_crate_is_clean` would still pass if one were added.

### M-5: No tooling enforces "no panics" and "never wraps"
- **Where:** `Cargo.toml:21-25`. The workspace denies `clippy::all` but not:
  - `clippy::arithmetic_side_effects`
  - `clippy::indexing_slicing`
  - `clippy::unwrap_used` / `clippy::expect_used`
  - `clippy::panic`
- **Also:** there is no `[profile.release] overflow-checks = true`, so release builds wrap silently (H-1 shows the result).
- **Non-test core today:**
  - No `unwrap`, `expect`, `panic!`, `assert!` or `unreachable!` outside test modules. All 22 `unreachable!` and every `assert!` sit after `#[cfg(test)]`.
  - Direct slice indexing on internally built vectors: `edit.rs:229-251, 326-621`, `geometry.rs:211-581`, `solver.rs:297-517`, `roof.rs:262-279`, `items.rs:50-53`.
  - I found no input that reaches an out-of-bounds index. Nothing but review guards those paths, though.

### M-6: ExistingConditionEdit does not show the baseline value (spec 6.7)
- **Where:** `crates/core/src/proposal.rs:139` and `:600`.
- **Spec:** 6.7 says the flag "shows the original value from the baseline snapshot (the first committed state, or a snapshot the user creates)".
- **What it does:** the flag carries only `{collection, id}`. The diff's `before` is the head value, not the baseline.
- **Missing test:** an edit to an existing element made after an earlier edit shows the baseline value.

### M-7: The document has no migrations (spec 5.5)
- **Where:** `crates/core/src/canonical.rs:58-60`.
- **Spec:** 5.5 says "The document has a schema_version and migrations."
- **What it does:** the parser rejects every `schema_version` other than 1. There is no migration hook or migration test. This is acceptable at version 1, but it is not yet decided whether this is deferred (no QUESTIONS entry).

### M-8: CLI output is unbounded (spec 9.8, C2 used by agent harnesses)
- **Where:**
  - `crates/cli/src/lib.rs:354`: `query` with no collection returns the whole document.
  - `crates/cli/src/lib.rs:267`: a `BLOCKED` error embeds the entire proposal in `details`. A one-wall shop commit's error was about 3 KB; a large edit scales with the diff.
  - `solve`, `validate` and `required-parameters` return full lists (92 Proposed values for the shop).
- **Spec:** 9.8 bounds tool results at 8 KB, paginated, for a 500-element project. This is formally Phase 3, but 9.1 and C2 say external harnesses use the CLI now.

### M-9: The CLI fallback error is built by string formatting
- **Where:** `crates/cli/src/lib.rs:212-216`. If canonical serialization fails, the response is `format!` with `"message":"{e}"`, unescaped.
- **Impact:** a message containing `"` or `\` gives invalid JSON on stdout, which breaks C2's "JSON out" contract.

### M-10: The echo rule (spec 9.4) is not implemented in proposals
- **Where:** `crates/core/src/proposal.rs:169` (`Proposal`).
- **Spec:** T-112 cites 9.4: "Before commit, the System renders the resolved command in canonical plain language".
- **What exists:** `Proposal` has resolved commands and display-unit diffs, but no rendered echo text.
- **Also:** CLI `apply` proposes and commits in one call with no confirmation step (`crates/cli/src/lib.rs`, `"apply"` arm). 9.4 asks for "explicit user confirmation … at the CLI prompt".
- **Deferral:** if this is meant for T-2xx/T-3xx, it needs a QUESTIONS or DECISIONS entry. None exists.

---

## Done-when and MUST coverage

Every Done-when line of T-101 to T-122b has a test, except those listed here. Lines whose test I checked and found adequate are not repeated.

| Task / section | Requirement | Gap |
| --- | --- | --- |
| T-106 | 500 elements under 50 ms in release | Test ignored; never run by check (M-2) |
| T-114 / 7.4 | Proposed items attached to every export | DXF and GLB: no list and no test (H-5) |
| T-118 / 5.4 / S-3 | L-room fiducials in DXF | No fiducials in DXF and no test (H-2) |
| T-119 | Khronos validator reports no errors | Manual and stale for the current GLB format (M-3) |
| T-121 | Unsure measurements are Measured with tolerance | Vacuous: no value was marked unsure (Q-65); fine |
| T-122 / T-122b | Gates G-3 and G-4 completed and recorded | G-4 not done; S-3 not recorded; DEC-4 draft (C-2) |
| 5.3 | User-stated inputs MUST lie on the grid | Enforced only in the parser, not in commands (C-1, closed by T-190) |
| 5.5 | Migrations | None (M-7) |
| 6.7 | ExistingConditionEdit shows baseline original | Not implemented (M-6) |
| 8.4 | Commit is atomic | In memory only; not on disk (H-3) |
| 9.4 (cited by T-112) | Canonical plain-language echo before commit | Not implemented (M-10) |
| 13.2 | Inverse delta restores a byte-identical document | No test (M-1) |

**Covered and verified by test name:**
- 5.1 rounding and overflow (`rounding_table`, `overflow_is_an_error_never_a_wrap`)
- 5.2 forms and echo (`user_forms_table`, `echo_*`)
- 5.5 replay byte-identical (`replaying_a_log_is_byte_identical`, `a_committed_log_replays_byte_identically`)
- 6.1 order independence (`independent_of_creation_order_and_wall_sense`, `reversed_wall_creation_gives_identical_profiles`)
- 6.3 Fixed refuses (`a_fixed_item_refuses_to_move`, `case_d_fixed_obstructions_refuse`)
- 7.3 insertion order (`shuffled_insertion_gives_identical_results`, `s1_shop_solution_is_order_independent`)
- 7.5 (`measured_within_tolerance_becomes_reference`, `conflicting_constraints_name_both`)
- 7.7, one test per invariant (`walls_must_have_positive_length`, `walls_must_not_overlap_without_a_join`, `openings_must_lie_inside_their_host` plus the jamb tests in geometry.rs, `space_outlines_must_be_simple`, `slab_outlines_must_be_simple`, `occurrences_overlap_only_in_plan_and_height`, `a_wall_moved_through_another_is_caught`)
- 8.3 and 8.4 (`propose_persists_nothing`, `a_stale_base_version_is_rejected`, `the_diff_lists_every_changed_entity`)
- 8.7 (a) to (d) (`case_a_*` to `case_d_*`)
- 8.8 (`a_single_push_clears_the_wall`, `a_chained_push_moves_the_next_item`, `a_fixed_item_blocks_the_chain`, `a_label_order_tie_moves_the_later_label`, `shifts_round_up_to_the_grid`)
- 11.1 (`wall_vertices_round_trip_to_the_nanometer`, `numbers_are_written_with_eight_decimals`, `an_independent_reader_parses_it`)
- 11.3 (`the_same_document_gives_identical_bytes`, `manifest_hashes_verify`, `the_schema_validates_the_output`, `element_glbs_are_local_and_readable`)

## Determinism experiments

I used a scratch harness outside the repository that links `archviz-cli`, `archviz-core` and `archviz-export`. For each script it replays the lines through `archviz_cli::run` into a fresh folder, then hashes the canonical document, the DXF, the GLB and every Scene Package file. Every variant ran twice, in separate processes and separate folders.

| Variant | Run 1 vs run 2 | vs unmodified script |
| --- | --- | --- |
| shop, l-room unmodified | identical | — |
| Every `timestamp_ms` and `actor` changed (envelope only) | identical | identical |
| All `add_constraint` lines moved to the end, reversed | identical | identical |
| `create_element_type` lines reversed | identical | identical |
| Release harness vs debug `cargo xtask slice` | — | identical hashes (shop DXF `905b4db2…`, GLB `01320a60…`, scene.json `f9a0b3af…`; l-room DXF `ac9213a9…`, GLB `149c6f3c…`) |

- No HashMap, HashSet, clock or randomness is reachable from core or export. The only clock and random ids are in `crates/cli/src/main.rs` (Host), outside the document.
- serde_json's `preserve_order` is not enabled anywhere in the graph, and the canonical writer sorts keys explicitly anyway.
- Reordering that changes labels (walls, occurrences) changes output by design (5.5 labels), so I did not test it as order-independent.

## Scan results (step 4)

- **Floating point in crates/core:** none (no `f32`/`f64` tokens, no decimal literals, no `as_f64`/`sqrt`/`powf`). `clearance.rs:165` uses an integer `ceil_sqrt`. Lint gaps: M-4.
- **unwrap, expect and panic paths:** none in non-test core, cli or export source. Indexing relies on internal invariants (M-5).
- **Unchecked arithmetic:** `validity.rs:108-124` overflows (H-1). The rest of the core arithmetic I sampled (geometry, space, roof, items, clearance, solver union-find) widens to i128 first and narrows with `try_from`. I found no other reachable overflow, but nothing enforces this (M-5).
- **Unsorted map output:** none found. Collections are BTreeMap or BTreeSet, scene elements are ordered by kind then id (`elements_are_ordered_by_kind_then_id_and_match_the_glb_placement`), and violations are sorted (`violations_are_sorted_and_deterministic`).
- **Unbounded tool output:** M-8.
