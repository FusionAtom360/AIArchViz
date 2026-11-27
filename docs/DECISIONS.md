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
- Re-run after T-122b (2026-11-27): the shop now has one slab from its four spaces (DEC-5) and its extension walls follow the roof (DEC-6), created in one apply with it; `cargo xtask slice` reports 0 conflicts, 24 underconstrained classes, 0 violations and 92 Proposed values. Constraint ids moved with the reordered script, so the contradicting dimension's one Conflict now reports the existing constraint, residual +1 in (computed minus stated for the constraint reported), still naming both and refused as BLOCKED. Order independence still holds for the 12 walls created one per apply.
- Status: draft

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
- Status: draft
