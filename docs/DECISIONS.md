# Decisions

Spike results and approved deviations from docs/SPEC.md. Claude Code drafts; the owner approves.

<!-- Template:
## DEC-n: title
- Date:
- Task or spike:
- Spec section:
- Decision:
- Rationale:
- Status: draft | approved (owner, date)
-->

## DEC-1: DXF version and dimensions for AutoCAD 2018
- Date: 2026-11-26
- Task or spike: T-118, to be confirmed by S-7 at gate G-3
- Spec section: 11.1
- Decision: write DXF R2000 (AC1015), the oldest version with $INSUNITS, with header $INSUNITS 1, $LUNITS 4, $LUPREC 4; coordinates in decimal inches with 8 decimals; driving Distance constraints drawn with the 11.1 fallback (dimension line, extension lines and text on DIMENSIONS) rather than DIMENSION entities (Q-51 to Q-53).
- Rationale: AutoCAD 2018 reads R2000; the fallback cannot trigger a repair prompt; 8 decimals of an inch read back to the exact nanometer.
- G-3 attempt 1 (2026-11-26): AutoCAD 2018 refused the file: "Error in LAYER Table. Did not receive PlotStyleName on line 266. Invalid or incomplete DXF input -- drawing discarded." Fix: every layer now names the Normal plot style placeholder (group 390) held in an ACAD_PLOTSTYLENAME dictionary with default; a regression test checks the chain.
- G-3 attempt 2 (2026-11-26, owner, AutoCAD 2018): opens without repair prompt: pass; units: pass (lengths display in feet-inches, the Architectural format of $LUNITS 4, on the inch base unit of $INSUNITS 1); a known wall dimension reads exactly: pass; layers present: pass (0, DIMENSIONS, EQUIPMENT, EQUIPMENT-LABELS, OPENINGS, SLABS, WALLS, WALLS-DEMOLISHED, WALLS-EXISTING); overlay on the owner's drawing registers: pass.
- S-7 result: pass for DXF R2000 (AC1015) with $INSUNITS 1, $LUNITS 4, $LUPREC 4 and the dimension fallback. True DIMENSION entities are not needed for now (Q-52).
- G-3 repeat (2026-11-27, owner, AutoCAD 2018; Q-74 gate option (i)): the slice shop.dxf after T-195 (PROPOSED layer, 93 text lines) and T-196 (no zero-length dimensions), SHA-256 a56860098c7e54cc6b09fe4baefbc27436d25975fc7fa30960c7efa989a7376b, opens clean with no repair prompt: pass.
- Status: approved (owner, G-3, 2026-11-26)

## DEC-2: glTF validation with the Khronos validator
- Date: 2026-11-26
- Task or spike: T-119
- Spec section: 5.4, 11.3, 13.2
- Decision: the Khronos glTF validator is not installed on the build machine or in CI. For T-119 it was installed with npm (gltf-validator 2.0.0-dev.3.10, Apache-2.0, published by The Khronos Group) into a scratch folder outside the repository, not as a crate dependency (Q-56), and run on the golden GLB crates/export/src/gltf/snapshots/archviz_export__gltf__tests__golden_l_room_glb.snap.glb (SHA-256 0ebc5c07a267ea7a4a633317ac9aeb26716a33b1f1ed0f5f5b3db05e9f3718a4). CI checks structure on every push with the `gltf` crate as a test-only reader, plus closed-solid volume tests.
- Result: 0 errors, 0 warnings, 0 infos, 0 hints; 12 draw calls, 492 vertices, 252 triangles.
- Rationale: answers T-119's Done when with the reference validator without adding Node to the Rust build.
- Re-run (2026-11-27, REVIEW-1 M-3, Q-74): the same validator install (2.0.0-dev.3.10), on the format after T-122b (welded vertices, no normals, cell-grid wall solids) and T-195 (root `extras.proposed_items`), with `cargo xtask slice` run at `274bbbe`. Every file: 0 errors, 0 warnings, 0 infos, 0 hints.
  - Golden GLB, SHA-256 f2883ce95d226638491289e35e7ce0922bad0430603d5589487859223d9d82db: 12 draw calls, 196 vertices, 348 triangles.
  - Slice shop.glb, SHA-256 efa7f6ec0c03fad5358143f755e575d70aef8358f6c284a4c6a9079dbb832c95: 36 draw calls, 464 vertices, 796 triangles.
  - Slice l-room.glb, SHA-256 c6002b7a72fff83bc052ce0d9a7fb05ce92d72e47cb85d1ff024a910ba0951c0: 12 draw calls, 196 vertices, 348 triangles.
  - All 48 per-element GLBs of the shop and L-room Scene Packages.
- Status: approved (owner, Q-56, 2026-11-26)

## DEC-3: Creation commands beyond the 8.2 set
- Date: 2026-11-26
- Task or spike: T-120b (Q-60, Q-66)
- Spec section: 8.2
- Decision: add CreateDatum, CreateMaterial, CreateSpace, CreateSlab, CreateCeiling, CreateRoof and CreateElementType to the v1 command set, with the arguments of Q-66. 8.2 lists no command that creates these entities, so without them a model cannot be built through the command API, as T-121 requires.
- Rationale: every change goes through commands (8.1, P2); these fill a gap in 8.2 without changing any existing command.
- Status: approved (owner, Q-66, 2026-11-26)

## DEC-4: S-1 result, Layer A on the client's shop
- Date: 2026-11-26
- Task or spike: T-122 (S-1)
- Spec section: 7.1 to 7.5, 14
- Decision: S-1 passes; Layer A stays the solver for v1 and pivot trigger (1) does not fire.
- Evidence: the shop of gate G-2 (fixtures/shop, 15 walls, 9 constraints, 7 openings, 4 spaces and slabs, a ledger shed roof, 12 equipment boxes) was entered through the CLI and solved by `cargo xtask slice` with 0 conflicts, 0 violations and 102 Proposed values listed. The degrees-of-freedom report lists exactly 24 underconstrained classes, the 12 occurrences on X and on Y, each naming its member, because no dimension ties equipment to a wall (crates/cli/tests/fixtures.rs). A contradicting 1 in datum-to-wall distance produces one Conflict naming the new constraint and the existing one, residual exactly -1 in, and the commit is refused as BLOCKED (s1_shop_conflict_names_both_constraints). Entering the walls and constraints reversed, or rotated, gives a byte-identical solution (s1_shop_solution_is_order_independent), adding to T-106's shuffled-order property test.
- Limits: the shop is axis-aligned with only Distance constraints and shared nodes; Equal, Align and Measured tolerances are covered by T-106 and T-120b tests, not by this plan.
- Re-run after T-122b (2026-11-27): the shop now has one slab from its four spaces (DEC-5) and its extension walls follow the roof (DEC-6), created in one apply with it; `cargo xtask slice` reports 0 conflicts, 24 underconstrained classes, 0 violations and 92 Proposed values. Constraint ids moved with the reordered script, so the contradicting dimension's one Conflict now reports the existing constraint, residual +1 in (computed minus stated for the constraint reported), still naming both and refused as BLOCKED. Order independence still holds for the 12 walls created one per apply.
- Status: approved (owner, Q-72, 2026-11-27)

## DEC-5: Slab outline from Spaces out to their walls' far faces
- Date: 2026-11-27
- Task or spike: found preparing G-4 (Q-67)
- Spec section: 6 (Slab boundary), 6.1
- Decision: add a boundary `FromSpaces { spaces }` alongside Explicit and FromSpace. It is the union of the listed Spaces and the walls bounding them, out to each bounding wall's far face, derived from the solved walls at commit like FromSpace. The fixtures use one such slab per contiguous floor. Ceilings keep FromSpace. 7.7's check that a Slab boundary is a simple rectilinear polygon applies to the result.
- Rationale: a slab outline at the inner faces leaves the floor open under every door void in the 3D model; the derived outline follows the walls (P2), whereas an explicit one would not.
- Status: approved (owner, Q-67, 2026-11-27)

## DEC-6: Wall tops that follow a roof's underside
- Date: 2026-11-27
- Task or spike: found preparing G-4 (Q-69)
- Spec section: D-4, 6.5
- Decision: add `WallTop::Roof { roof }`. The wall's elevation profile (a 2D polygon, as for opening voids) takes the roof's underside as its top edge. The arithmetic follows the rest of core: integer nanometers only. The underside height at a point s along the slope axis is high - (s - high_edge) x rise / run, computed in i128 and rounded half to even to the nanometer (5.1), the same rule that derives a low edge from a pitch. Profile vertices are taken only at the wall's ends, at its faces and at opening void edges, so the same document gives the same profile byte for byte, whatever order the walls were created in.
  - A wall along the slope gets a sloped top edge.
  - A wall across the slope is level at the underside height on its higher face, so it closes against the roof, reaching at most its thickness x rise / run into the roof's thickness.
  - A wall whose top would be at or below an opening head, or that lies outside the roof's footprint plus overhangs, is a violation (7.7). It is not clipped silently.
  - Such a wall's height becomes Derived, not Proposed.
- Rationale: walls stay extruded 2D profiles (D-4), but this is a second non-flat exception next to the roof in 6.5. It closes the gaps between flat wall tops and a sloped roof, with one element per wall in the model, the DXF and UE.
- Status: approved (owner, Q-69, 2026-11-27)

## DEC-7: S-3 result and gate G-4, coordinates and handedness end to end
- Date: 2026-11-27
- Task or spike: S-3, gate G-4 (T-122, T-122b, T-191; Q-72)
- Spec section: 5.4, 14
- Decision: S-3 passes for DXF, glTF and UE. The L-room fiducials land where 5.4 puts them in every exporter and in the UE 5.8 editor, with no mirror or axis swap. Phase 1 needs no coordinate or unit change.
- Evidence:
  - DXF: `l_room_fiducials_read_back_from_the_dxf` (crates/export/src/dxf.rs) and `l_room_dxf_fiducials_read_back_exactly` (xtask/src/slice.rs, on the replayed fixtures/l-room output) read F-A to F-D back at (0, 0), (240, 0), (240, 96) and (0, 180) in, exact to the nanometer (T-191).
  - glTF: `l_room_fiducials_land_at_the_expected_gltf_coordinates` (crates/export/src/gltf/tests.rs), glTF = (x, z, -y) in meters.
  - UE: gate G-4, below.
- G-4 (2026-11-27, owner, UE 5.8 editor): the `cargo xtask slice` GLBs were imported by hand with the default glTF import settings, at the origin with no rotation or scale. The GLBs are byte-identical before and after T-191: l-room.glb SHA-256 149c6f3ca9ac30b886a57131f48848d0535a67cb1ec014223ed39d732a0d686e, shop.glb 01320a60fc905688d4c86f0ff9a0f628466ad7e770bc60fcd49f50f6900843c4.
  - L-room fiducials: F-A (0, 0, 0), F-B (609.6, 0, 0), F-C (609.6, -243.84, 0), F-D (0, -457.2, 0) cm, read exactly as expected: pass.
  - Shop dimension: the north wall's exterior face measured 18'-0" (548.64 cm), exactly as expected: pass.
  - Shading (Q-70): faces look flat-shaded with welded vertices and no normals, so the Q-70 (b) fallback is not needed: pass.
  - T-122b geometry: the floor is closed under every door; each slab reaches the walls' exterior faces; the shop roof starts at the east wall's exterior face; no roof shows through a wall or ceiling; no gap between the roof-following walls and the roof's underside: pass.
  - Walk: the shop walked in Play In Editor with nothing wrong noted: pass.
  - Screenshots: four (L-room exterior with its shed roof; L-room interior with door, window, ceiling and the marker box raised 36 in; shop from above with the extension; shop interior among the equipment boxes). They were reviewed on 2026-11-27 and the owner keeps them outside the repository. They are consistent with the fixtures: the shop's main block has no roof or ceiling, since only the extension is roofed (G-2).
- Status: approved (owner, G-4, 2026-11-27)

## DEC-8: Migrations come with the first schema change
- Date: 2026-11-27
- Task or spike: REVIEW-1 M-7 (Q-74)
- Spec section: 5.5
- Decision: schema_version stays 1 and the parser keeps rejecting every other version. There is no migration hook yet. The first change to the document schema must add the hook, a migration from version 1 and a test that a version-1 document migrates and replays byte-identically, before the version is bumped.
- Rationale: with one version there is nothing to migrate, and a hook with no migration to exercise would be untested code.
- Status: approved (owner, Q-74, 2026-11-27)

## DEC-9: CLI output bounds are built with the Phase 3 tool surface
- Date: 2026-11-27
- Task or spike: REVIEW-1 M-8 (Q-74)
- Spec section: 9.1, 9.8
- Decision: the 8 KB bound, pagination and compact form of 9.8 are built with the tool surface (T-301 to T-308), and the CLI then shares them. Until then the CLI is a development interface and its output is unbounded.
- Rationale: 9.8 defines the bounds for tool results; building them once with the tools avoids a second, CLI-only format.
- Status: approved (owner, Q-74, 2026-11-27)

## DEC-10: Commit confirmation is the app's; the CLI apply stays one call
- Date: 2026-11-27
- Task or spike: REVIEW-1 M-10 (Q-74)
- Spec section: 9.4
- Decision: proposals carry the canonical echo text (T-199e). Explicit confirmation of that text is built in the app in Phase 2. The CLI `apply` keeps proposing and committing in one call, and no interactive CLI prompt is built; scripts, fixtures and harnesses that want a confirmation step use `propose` then `commit`.
- Rationale: the CLI is driven by scripts and fixtures, where a prompt would block; the propose and commit pair already lets a caller confirm the echo before committing.
- Status: approved (owner, Q-74, 2026-11-27)

## DEC-11: The inverse delta restores values, not provenance or acknowledgments
- Date: 2026-11-27
- Task or spike: T-197 (Q-75)
- Spec section: 13.2, 8.4
- Decision: the inverse of a committed MoveWall, MoveOpening or MoveOccurrence is the same move (same reference, axis, edit and absorb) by minus the displacement measured in the document before the commit, and the inverse of a SetConstraintValue sets the prior value. Committing a command and then its inverse gives a byte-identical document when the offsets the command moves are UserStated and the document holds no acknowledgments. Otherwise two differences remain: an offset that was Measured, Proposed or Imported comes back with its old value but as UserStated, because a move states the dimension (Q-39); and an acknowledgment pruned because the command cleared its violation does not come back, so the inverse brings the violation back unacknowledged and its proposal is blocked (Q-35). An inverse that brings back an unacknowledged violation the command fixed, such as the overlap a clearance_resolve push fixed, is blocked in the same way, though its resolved commands restore the document byte for byte. Undo (8.4) is the exact restore in every case. The inverse of a value that was off the 1/16 in grid is itself off the grid and is refused by the grid rule (5.3) like any user-stated value.
- Rationale: it changes no command or log format and keeps Q-39; the differences are provenance and acknowledgment bookkeeping, not geometry, and stored-state undo already restores them exactly.
- Status: approved (owner, Q-75, 2026-11-27)

## DEC-12: BSL-1.0 and 0BSD join the allowed licenses; the app ships no font
- Date: 2026-11-27
- Task or spike: T-201 (Q-77)
- Spec section: D-14, D-19
- Decision: deny.toml allows BSL-1.0 (eframe's clipboard: clipboard-win, error-code) and 0BSD (the `interprocess` pipe crate: recvmsg, doctest-file), both on the FSF's list of GPL-compatible licenses. eframe is used without `default_fonts`, so epaint_default_fonts (OFL-1.1, Ubuntu-font-1.0) is not in the build; the app loads a font installed with Windows at run time. Bundling a font for the wasm32 build is left to the task that builds it.
- Rationale: both licenses are short, permissive and GPL-compatible; they keep unsafe FFI out of our code and keep copy and paste in text fields. No font is redistributed.
- Status: approved (owner, Q-77, 2026-11-27)

## DEC-13: The app writes its pipe name and session token to host.json
- Date: 2026-11-27
- Task or spike: T-201 (Q-77)
- Spec section: 3.1
- Decision: while the app has a project open, it writes `host.json` (pipe name and a random 256-bit session token) next to project.lock, and deletes it when the project closes. A client reads it to attach; a request with any other token is refused. Under the CLAUDE.md rule "Never write secrets (API keys, tokens) to files", this token is a local capability that lasts as long as the app's session, not a credential for an outside service. API keys and other credentials stay under the rule. "Save" in the app is "Save a copy as" (genesis.json and log.jsonl copied to a new folder); autosave is the existing rule that a commit is flushed to log.jsonl before it is answered; an uncommitted proposal is not saved.
- Rationale: a client started on its own has no other way to learn the token without a manual step on every call; any process that can read host.json can already read and replace the project's files.
- Status: approved (owner, Q-77, 2026-11-27)
