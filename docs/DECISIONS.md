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
- Status: approved (owner, G-3, 2026-11-26)

## DEC-2: glTF validation with the Khronos validator
- Date: 2026-11-26
- Task or spike: T-119
- Spec section: 5.4, 11.3, 13.2
- Decision: the Khronos glTF validator is not installed on the build machine or in CI. For T-119 it was installed with npm (gltf-validator 2.0.0-dev.3.10, Apache-2.0, published by The Khronos Group) into a scratch folder outside the repository, not as a crate dependency (Q-56), and run on the golden GLB crates/export/src/gltf/snapshots/archviz_export__gltf__tests__golden_l_room_glb.snap.glb (SHA-256 0ebc5c07a267ea7a4a633317ac9aeb26716a33b1f1ed0f5f5b3db05e9f3718a4). CI checks structure on every push with the `gltf` crate as a test-only reader, plus closed-solid volume tests.
- Result: 0 errors, 0 warnings, 0 infos, 0 hints; 12 draw calls, 492 vertices, 252 triangles.
- Rationale: answers T-119's Done when with the reference validator without adding Node to the Rust build.
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
- Status: draft
