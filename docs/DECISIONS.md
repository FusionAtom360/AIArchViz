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
- Status: draft (pending G-3 attempt 2: opens without repair prompt, units read inches, a known wall dimension reads exactly, layers present, overlay registers)
