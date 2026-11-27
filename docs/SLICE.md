# Vertical slice

The client's shop end to end (spec 14, T-122): constraints entered from the paper plan
(gate G-2, Q-61 to Q-65), solved, and exported as a dimensioned DXF, a GLB and a Scene Package.

## Regenerate everything

From the repository root:

```powershell
cargo xtask slice
```

That one command:

1. deletes `target/slice/` if it exists;
2. replays `fixtures/shop/commands.txt` and `fixtures/l-room/commands.txt`, line by line,
   through the archviz CLI library into fresh project folders (the same calls the
   `archviz` binary makes; see fixtures/README.md);
3. solves and validates each committed document and prints one summary line;
4. writes the exports and prints the SHA-256 of each top-level file.

| Path under `target/slice/` | What it is |
| --- | --- |
| `shop/shop.dxf` | Plan for AutoCAD 2018: R2000, inches, architectural units at 1/16 in (DEC-1) |
| `shop/shop.glb` | The whole model in one GLB (glTF = (x, z, -y) in meters, spec 5.4) |
| `shop/scene/` | Scene Package: `scene.json`, `manifest.sha256`, `assets/<element id>.glb` |
| `shop/project/` | The replayed project (`genesis.json`, `log.jsonl`), usable with `archviz query`, `solve` and so on |
| `l-room/...` | The same for the L-room conformance fixture, whose fiducials gate G-4 checks (S-3) |

Expected summary lines:

```text
shop: 0 conflicts, 24 underconstrained classes, 0 violations, 92 Proposed values
l-room: 0 conflicts, 2 underconstrained classes, 0 violations, 5 Proposed values
```

The 24 classes are the twelve equipment occurrences on X and on Y (fixtures/README.md).
Running the command twice gives byte-identical files; `xtask/src/slice.rs` tests this on
every `cargo xtask check`. To check a Scene Package by hand:

```sh
cd target/slice/shop/scene && sha256sum -c manifest.sha256
```

The DXF, GLB and Scene Package come from the committed document only. To change the shop,
change its builder in `xtask/src/fixtures.rs`, run `cargo xtask golden`, review the diff of
`fixtures/shop/commands.txt`, then run `cargo xtask slice`.

## Gate G-3: DXF in AutoCAD 2018 (S-7)

Passed on 2026-11-26 (DEC-1). To repeat it: open `target/slice/shop/shop.dxf` in AutoCAD 2018.
Check that it opens without a repair prompt, that units are inches shown in feet-inches, that
a known dimension reads exactly (for example the north wall, 18'-0" exterior face to exterior
face), and that it overlays the client's drawing.

## Gate G-4: GLB in UE 5.8 (S-3)

Passed on 2026-11-27 (DEC-7). To repeat it, by hand, in a UE 5.8 editor project:

1. Import `target/slice/l-room/l-room.glb` with the default glTF import settings and place it
   at the origin with no rotation or scale.
2. Read the location of each fiducial actor or component, in centimeters. By spec 5.4,
   UE = (x, -y, z) with world X east and Y north. Expected values:

   | Fiducial | World (in) | Expected UE (cm) |
   | --- | --- | --- |
   | F-A | (0, 0) | (0, 0, 0) |
   | F-B | (240, 0) | (609.6, 0, 0) |
   | F-C | (240, 96) | (609.6, -243.84, 0) |
   | F-D | (0, 180) | (0, -457.2, 0) |

   The L's long arm runs from F-A to F-B (20 ft), and the short wing runs toward F-D. A
   mirror image (F-D at +457.2 on Y) or an axis swap fails S-3.
3. Import `target/slice/shop/shop.glb` the same way. Datum SW, the south-west exterior corner,
   should be at (0, 0, 0). Measure a known dimension. The north wall's exterior face runs
   18'-0" (548.64 cm) east-west. The west wall's exterior face runs 36'-0" (1097.28 cm)
   north-south. The garage door opening is 12'-0" (365.76 cm) wide and 9'-1" (276.86 cm)
   tall.
4. Check the geometry fixed in T-122b. Every element is one welded solid with no normals, so
   UE must compute flat normals (Q-70): faces must look flat-shaded, not smoothed across
   edges. If UE smooths them, record it, and the fallback is split vertices with normals
   (Q-70 (b)). The floor is closed under every door, and each slab reaches the walls'
   exterior faces (DEC-5). The shop roof starts at the east wall's exterior face, and no roof
   shows through a wall or ceiling. The extension walls and every L-room wall meet the roof's
   underside with no gap (DEC-6).
5. Walk the shop in Play In Editor. Note anything wrong, and take screenshots of the fiducial
   readings, the measurement and the walk.

Record the outcomes in docs/DECISIONS.md: fiducials (S-3), the measured dimension, the
shading (Q-70), and the screenshots.
