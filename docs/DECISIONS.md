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
