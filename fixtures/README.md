# Fixtures

Conformance fixtures (T-121, spec 5.4 and 13.2), each a script of CLI commands:
`<name>/commands.txt` holds one `archviz` call per line, `<subcommand> <request JSON>`,
starting with `new`. Each `apply` holds one command, except where walls follow a roof
(DEC-6): a wall can only name a roof that exists, so the roof and its walls (and, for the
L-room, the room its footprint comes from) are created in one `apply`, one transaction. The scripts are generated from typed builders in
`xtask/src/fixtures.rs` by `cargo xtask golden`. Never edit them by hand.
`crates/cli/tests/fixtures.rs` replays them through the binary on every `cargo xtask check`.

Replay into a new project folder:

```sh
while read -r sub json; do printf '%s' "$json" | archviz "$sub" out/shop; done < fixtures/shop/commands.txt
```

```powershell
Get-Content fixtures/shop/commands.txt | ForEach-Object { $sub, $json = $_ -split ' ', 2; $json | archviz $sub out/shop }
```

## l-room

The L-room of Q-54, the room T-119's tests build in code, rebuilt through the CLI and fully dimensioned from fiducial F-A.
- Wall centerlines run (0,0) (240,0) (240,96) (96,96) (96,180) (0,180) in inches, with 6 in walls.
- Point datums F-A (0,0), F-B (240,0), F-C (240,96) and F-D (0,180).
- A door, a window and a plain opening.
- A slab out to the walls' exterior faces (DEC-5) and a ceiling at 108 in.
- A 2:12 shed roof bearing on the walls (Q-68): its footprint is the walls' exterior outline,
  its underside is 120 in at the south (low) side, the high edge is derived from the pitch
  (151 in at the north side), and every wall's top follows its underside (DEC-6).
- A marker box raised 36 in.

Underconstrained: the marker box (EQ-001) on both axes.

## shop

The client's shop from gate G-2, as answered in Q-61 to Q-65.
- **Frame:** X east, Y north. Datum SW is the south-west exterior corner.
- **Shell:** 216 in E-W by 432 in N-S, measured exterior face to exterior face. Walls are 4 in thick and 120 in high, with the reference line on the centerline. Every given dimension is UserStated, and none is Measured (Q-65).
- **New walls:**
  - New wall 1: its north face is 312 in south of the north exterior face. It runs from the west wall to the east wall.
  - New wall 2: its east face is 144 in west of the east exterior face. It runs from the south wall to new wall 1.
- **Extension:** 64 in out from the east wall, from 36 in off the north face to 36 in off the south face, with three new walls. A shed roof sits on a ledger on the east wall's exterior face: its underside is 108 in there and 96 in at the outer side (Q-68), it has no overhang on the ledger edge, and the three walls' tops follow its underside (DEC-6).
- **Openings** (Q-62, measured to the north edge from the north exterior face):
  - North wall: a garage door 144 in wide and 109 in tall, centered.
  - West wall: a door 36 x 80 at 42 in, a window 60 wide at 138 in, a door 32 x 80 at 323 in, and a window 36 wide at 377 in. Both windows have a 35 in sill and an 80 in head.
  - South wall: a window 60 wide, centered.
  - East wall: a door 48 x 96 at 103 in, opening into the extension (Q-63).
- **Rooms:** Shop, South room west, South room east and Extension, under one slab that reaches the exterior faces (DEC-5).
- **Equipment** (Q-64): sizes come from G-2 where given and are Proposed otherwise. Positions are Proposed and were chosen in the large room, clear of walls, doors and each other.

Underconstrained: only the equipment. Each occurrence is free on X and on Y (24 classes), because no dimension ties it to a wall. Centers are in inches.

| Label | Type | Center (x, y) | Size X x Y x Z | Size source |
| --- | --- | --- | --- | --- |
| EQ-001 | Storage rack | 199, 386 | 24 x 80 x 76 | G-2 |
| EQ-002 | Storage rack | 199, 236 | 24 x 80 x 76 | G-2 |
| EQ-003 | Storage rack, turned 90 degrees | 130, 134 | 24 x 80 x 76 | G-2 |
| EQ-004 | Storage rack, turned 90 degrees | 46, 134 | 24 x 80 x 76 | G-2 |
| EQ-005 | Large table | 100, 250 | 48 x 82 x 36 | G-2 |
| EQ-006 | Tool chest | 16, 190 | 20 x 42 x 56 | G-2 |
| EQ-007 | CO2 laser | 40, 320 | 56 x 36 x 42 | Proposed |
| EQ-008 | Band saw | 130, 330 | 30 x 26 x 68 | Proposed |
| EQ-009 | Drill press | 170, 330 | 24 x 16 x 66 | Proposed |
| EQ-010 | Belt sander | 60, 180 | 30 x 20 x 48 | Proposed |
| EQ-011 | Bench grinder | 160, 180 | 20 x 14 x 44 | Proposed |
| EQ-012 | File cabinet | 199, 160 | 15 x 28 x 52 | Proposed |

`archviz required-parameters` lists every remaining Proposed value (92 of them).
