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

## Q-1: What "rounded" means for inch input finer than 1/64 in (T-102, status: answered)
- Context: spec 5.1 says "Inch fractions finer than 1/64 in are rounded on input with the error reported". This only matters in the asset-specification context, since the user context rejects anything off the 1/16 in grid (5.3). It does not say what they are rounded to. Some finer fractions are exact in nanometers (1/100 in = 254,000 nm, 1/128 in is not: 198,437.5 nm).
- Options: (a) round to the nearest nanometer, half to even, and report the residual; exact inputs such as 1/100 in stay exact; (b) round to the nearest 1/64 in and report the difference.
- Recommendation: (a). It keeps manufacturer values as exact as storage allows (the residual is at most 0.5 nm) and matches T-101's division rule. Option (b) can move a value by up to 1/128 in (about 0.2 mm).
- Answer: (a) nearest nanometer, half to even, residual reported (owner, 2026-11-26).

## Q-2: Canonical feet-inches text for edge cases (T-102, status: answered)
- Context: spec 5.2 and 5.3 give one example, 10'-4 1/2". The format for values under a foot, whole feet, zero and negatives is not stated, and the echo and display (and later golden echo strings, T-303) depend on it.
- Options: (a) always feet and inches, architectural style as in AutoCAD: 0'-4 1/2", 10'-0", 0'-0", -10'-4 1/2", ~10'-4 1/2"; fractions reduced (1/2, 3/8, 15/16); (b) drop zero feet: 4 1/2", 10'-0", 0"; (c) something else.
- Recommendation: (a). One shape for every value is the most predictable, and it matches the AutoCAD architectural units the client uses (11.1). The ~ goes before the sign: ~-0'-0 1/16".
- Answer: (a) always feet and inches, architectural style; ~ before the sign (owner, 2026-11-26).

## Q-3: What the echo function shows (T-102, status: answered)
- Context: spec 5.2 requires echoing every parsed value in canonical feet-inches-fraction form. 5.3 says an off-grid stored value's exact value is shown "on inspection and in reports". Asset-context values (metric, fine fractions) are usually off the 1/16 in grid, so the canonical text alone (~0'-0 1/2") hides what was stored. Decimal inches cannot show it exactly (1 nm = 1/25,400,000 in, and 25,400,000 has the factor 127), but millimeters can (1 nm = 0.000001 mm).
- Options: (a) echo is exactly format_length; (b) echo is format_length, and for an off-grid value it adds the exact stored value in millimeters, plus the input rounding from Q-1 when there was any; (c) something else.
- Recommendation: (b). On grid: `10'-4 1/2"`. Off grid: `~0'-0 1/2" (exact 12 mm)`. Rounded input: `~0'-0" (exact 0.198438 mm, rounded 0.5 nm from input)`. Millimeters print with trailing zeros removed.
- Answer: (b) canonical text, plus exact millimeters for off-grid values and the input rounding when any (owner, 2026-11-26).

## Q-4: Field-level provenance (T-104, status: answered)
- Context: P6 says "every value and asset records its origin and fidelity"; 6.4 says every default is "applied with status Proposed"; 7.4 and 11.3 require listing every Proposed item. The section 6 table lists provenance only on Constraint (status, provenance) and Asset. So it is unclear whether entity parameters such as wall layer thickness, opening sill or storey height carry their own source.
- Options: (a) every settable dimensional or design parameter is a `Sourced<T> { value, source }` with source from 7.2 (UserStated, Measured, Derived, Proposed, Imported); ids, references and enums stay plain; (b) provenance only on Constraint and Asset, with Proposed values tracked in a separate document-level list keyed by (entity id, field name); (c) no provenance outside constraints in T-104, deferred to a later task.
- Recommendation: (a). The Proposed list for 7.4 and 11.3 becomes a pure scan of the document, values cannot lose their source when edited, and the cost is a slightly larger JSON.
- Answer: (a) Sourced<T> per settable parameter (owner, 2026-11-26).

## Q-5: Angle representation without floating point (T-104, status: answered)
- Context: Occurrence rotation (D-5: multiples of 90 degrees), CameraBookmark heading (12.6) and the default walk start need angles; core forbids f32/f64.
- Options: (a) rotation as an enum of quarter turns counterclockwise (0, 1, 2, 3); heading as integer millidegrees in [0, 360000), 0 = +X (east), counterclockwise; (b) both as integer millidegrees, with rotation validated to multiples of 90,000; (c) heading as a quarter-turn too.
- Recommendation: (a). Rotation cannot hold an illegal value, and a camera heading needs finer steps than 90 degrees; 0.001 degree is far below anything visible.
- Answer: (a) quarter-turn rotation, integer millidegree heading (owner, 2026-11-26).

## Q-6: Entities needed elsewhere but missing from the section 6 table (T-104, status: answered)
- Context: the task asks for "every entity in spec section 6, including Roof, Underlay and CameraBookmark". Other sections also need: Material (1.4 "materials", LayerSet layers name a material, 12.4), light data (11.3: type, lumens, color temperature, optional IES), the default walk start (12.6), the label counters that make labels never reused across save and load (5.5, T-103), and the baseline snapshot of 6.7.
- Options: (a) add Material {name, asset, tile size, properties}; light data as an optional `light` block on Occurrence {kind, lumens, kelvin, ies asset}; `default_walk_start` as a bookmark id; label counters stored in the document; the baseline snapshot kept outside the document, in the log and snapshot store of T-105; (b) the same but the baseline snapshot stored inside the document; (c) leave all of these to the tasks that use them.
- Recommendation: (a). Every item is required by a cited section; the baseline is a whole earlier document, so storing it inside the document would nest documents and double the file.
- Answer: (a) add them; baseline snapshot outside the document (owner, 2026-11-26).

## Q-7: Store derived values in the document (T-104, status: answered)
- Context: section 6 lists derived values as entity fields: Node x, y "(solved)" and "derived join type", Space "derived boundary", Feature "derived children". The canonical document must be byte-identical for a given command log either way.
- Options: (a) store them: the document holds solved node positions, join types, space boundaries and feature children, rewritten by each command; (b) store only inputs and recompute derived values on load.
- Recommendation: (a). Exports, diffs (8.3) and queries read them directly; they are deterministic, so byte-identity holds, and a stored value lets a diff show before and after without re-solving the old state.
- Answer: (a) store derived values (owner, 2026-11-26).

## Q-8: Representation choices made in T-104 unless you object (T-104, status: answered)
- Context: these follow from the spec but are not spelled out. Listed for review rather than decided silently.
- Choices: lengths are integer nanometers; ids are lowercase hyphenated UUID strings; each entity kind is a map keyed by id (sorted); enums are snake_case strings; references (6.2) and placements (6.3) are tagged objects `{"kind": ...}`; ElementType properties and Feature parameters are maps of string to one of string, integer, boolean, length (no decimals); lumens and kelvin are integers; asset content hashes are `sha256:<hex>`; pitch is integer rise and run; roof eave overhang is one value per compass side (+X, -X, +Y, -Y) applied to every footprint edge facing that way; clearance zones are per-side extents (front, back, left, right) around the footprint; schema_version starts at 1; unknown JSON fields are rejected on parse. Golden files: the small document is an insta binary snapshot (pure JSON file), and schemas/document.schema.json is regenerated by `cargo xtask golden` and checked by a test.
- Recommendation: accept as listed.
- Answer: accepted as listed (owner, 2026-11-26).

## Q-9: Which 8.2 commands T-105 implements (T-105, status: answered)
- Context: T-105 asks for the whole 8.2 enum, with "variants not yet implemented" returning NotYetImplemented. Most commands need the solver (T-106), wall geometry (T-107), openings (T-108) or operators, which later tasks build. SetProjectUnits has nowhere to write: the document has no project unit (the bare-number unit of 5.2 and T-102).
- Options: (a) implement the commands that are pure data edits needing no solver or geometry: CreateNode, SetStorey (create or update a storey), SetVariantSelection, SetProjectUnits; add `default_unit` (inches or feet) to Project for the last; every other variant parses and returns NotYetImplemented until its task; (b) implement only CreateNode; (c) also implement AddConstraint, RemoveConstraint and SetConstraintValue now, without re-solving.
- Recommendation: (a). It exercises creation, update and new ids, and leaves every command that would need a re-solve to the task that brings the solver, so no command ever leaves the document unsolved.
- Answer: (a) pure data edits, with default_unit added to Project (owner, 2026-11-26).

## Q-10: What a snapshot is (T-105, status: answered)
- Context: T-105 asks for "snapshots"; 6.7 uses "a snapshot the user creates" as the baseline for ExistingConditionEdit; 8.4 says undo and redo restore exact prior states and are recorded as events.
- Options: (a) a snapshot is {version, document}: a full copy of the document at a log position, used to load quickly and as the 6.7 baseline; undo and redo keep exact prior documents in memory, rebuilt by replaying the log; replaying from a snapshot can undo back to that snapshot but not past it (an error says to replay from the start); (b) a snapshot also carries the undo and redo stacks, so replay from it can undo anything, at the cost of storing every prior document in each snapshot; (c) undo by inverse commands instead of stored states.
- Recommendation: (a). Stored prior states make undo exact by construction; the log stays the source of truth; snapshots stay one document in size.
- Answer: (a) {version, document}; undo state rebuilt by replay (owner, 2026-11-26).

## Q-11: Log envelope and idempotency (T-105, status: answered)
- Context: 5.5 keeps timestamps, user names and session metadata in the log envelope; 8.1 gives every command an idempotency_key and derives new ids as UUIDv5 of (idempotency_key, ordinal). Core has no clock, so the caller supplies the time.
- Options: (a) envelope {sequence (1, 2, 3, ...), timestamp_ms (integer milliseconds since the Unix epoch, supplied by the caller), actor (free text, e.g. "user" or an agent session id), event}; events are Commit {commands} (atomic, 8.4), Undo and Redo; a new commit clears the redo stack; idempotency_key and command id are UUIDs chosen by the caller; new ids are `Uuid::derive(idempotency_key, "command", ordinal)`; a commit containing an idempotency key already in the log is rejected with DuplicateCommand naming the earlier sequence, and nothing changes; (b) the same, but a repeated key is accepted silently as a no-op; (c) timestamps as RFC 3339 strings.
- Recommendation: (a). Integer time keeps the log free of parsing rules; rejecting a repeat tells the caller exactly what happened instead of hiding it.
- Answer: (a) integer-ms envelope; repeated idempotency key rejected with DuplicateCommand (owner, 2026-11-26).

## Q-12: Where a wall's orientation comes from (T-106, status: answered)
- Context: Layer A needs each wall's axis (X or Y) and direction (node_a to node_b) to know which coordinate its reference line and faces lie on and which side is left (6.1). The Wall entity has no orientation field; D-5 says every wall is parallel to X or Y.
- Options: (a) add `direction` (pos_x, neg_x, pos_y, neg_y) to Wall, set at creation and never changed by solving; the validity check (7.7) can then catch a move that flips a wall; (b) derive it from the stored node positions on each solve.
- Recommendation: (a). Orientation becomes intent rather than a by-product of the last solve, so a move that drags node_b past node_a is reported instead of silently swapping left and right.
- Answer: (a) stored direction field on Wall (owner, 2026-11-26).

## Q-13: Sign of Distance and of Fixed's axis (T-106, status: answered)
- Context: 7.1 gives Distance(refA, refB, axis, value) without a sign rule; an unsigned distance |b - a| = v has two solutions and cannot be solved by union-find. Fixed(ref, value) has no axis, so Fixed on a Node or a point Datum is ambiguous.
- Options: (a) Distance is signed: coordinate(refB) - coordinate(refA) = value along `axis`; Align is Distance 0; Fixed gains an `axis` field (schema change); a reference whose plane is not perpendicular to the given axis (a face of an X-direction wall used on the X axis) is an error; (b) unsigned distances with a direction inferred from current positions.
- Recommendation: (a). It is exact and order-free; the UI and agent can present the absolute value and direction.
- Answer: (a) signed distance; Fixed gains an axis (owner, 2026-11-26).

## Q-14: What Equal(distance1, distance2) means (T-106, status: answered)
- Context: in the model every Distance carries a value, so "equal" either copies a value or only checks one.
- Options: (a) Equal(first, second) makes `second` take the resolved value of `first`; second's own stored value becomes Derived; a cycle of Equals is a Conflict; (b) Equal only checks that two driving distances have the same value, raising a Conflict otherwise; (c) Equal ties two distances whose values are both free, which needs more than union-find (Layer B).
- Recommendation: (a). It is the useful case ("same spacing as that one") and stays within Layer A.
- Answer: (a) second copies first; cycles are Conflicts (owner, 2026-11-26).

## Q-15: Which constraint is the redundant one (T-106, status: answered)
- Context: 7.5 turns a redundant Measured constraint into Reference (within tolerance) or raises a Conflict naming both (beyond). Which of two clashing constraints is "redundant" depends on the order they are processed in, and results must not depend on insertion order (7.3).
- Options: (a) process driving constraints in a fixed order: by source authority (UserStated, Imported, Measured, Derived, Proposed, following D-11), then by constraint id; the later one in that order is the redundant one; (b) by constraint id only.
- Recommendation: (a). A measured or proposed value then never overrides a user-stated one, and the order is total, so shuffled input gives identical results.
- Answer: (a) authority order, then id (owner, 2026-11-26).

## Q-16: Solver scope and provisional positions in T-106 (T-106, status: answered)
- Context: 6.2 lists seven reference kinds; openings (T-108) and occurrences (T-111) come later. 7.4 requires a provisional model even when underconstrained.
- Options: (a) T-106 solves Node, WallFace, WallCenter and Datum references; OpeningEdge, ItemEdge and ItemCenter return UnsupportedReference until T-108 and T-111 extend the solver; an underconstrained class keeps its translation from the stored position of its lowest-id member, so a provisional model always exists; WallFace boundary indexes count from the named side's finish face inward (0 = that side's face); Reference-status constraints report their computed value and residual when both ends are in one class; (b) all seven kinds now, with item and opening geometry defined here.
- Recommendation: (a). Each later task adds its own reference kinds with its own geometry and tests.
- Answer: (a) as listed (owner, 2026-11-26).

## Q-17: Mitered L joins in exact integer arithmetic (T-107, status: answered)
- Context: 6.1 says an L join is mitered; T-107 asks for exact integer rectilinear polygon arithmetic. A miter cuts each wall end along the line from the inner-face corner to the outer-face corner. Those two corners are always integer points, so wall footprints stay exact (with one diagonal edge each), but a layer boundary inside the wall crosses that line at a non-integer point whenever the two walls have different thicknesses.
- Options: (a) miter as specified: footprints exact; a layer boundary point on the miter line is rounded half to even to the nanometer when it is not exact (at most 0.5 nm, only between unequal walls); (b) deviate (DECISIONS entry): treat L like T, the lower join_priority wall runs through and the other butts, so every polygon is rectilinear and exact; (c) rational coordinates for layer polygons.
- Recommendation: (a). It follows the spec, the footprints and every face are exact, and the only rounding is inside the wall at a scale far below anything drawn or built.
- Answer: (a) miter; interior layer points on the miter rounded half to even (owner, 2026-11-26).

## Q-18: Which wall "runs through" at T and X joins (T-107, status: answered)
- Context: 6.1: "at three or more, the wall with the lowest join_priority runs through and others butt". Walls meeting at a node from opposite sides form a run. At a T the stem is a run of one; it can have the lowest priority.
- Options: (a) the run containing the lowest-priority wall (ties by lowest wall id) runs through. A run of two collinear walls meets square at the node and is continuous; a run of one (a T stem) extends through the junction to the far face of the crossing walls. Every other wall butts, ending at the nearest face of the through run (with collinear walls of different thickness, the face that reaches farthest toward it); (b) at a T the collinear pair always runs through, and join_priority only decides between the two pairs of an X.
- Recommendation: (a). It is the rule as written and is fully deterministic.
- Answer: (a) the run with the lowest-priority wall runs through (owner, 2026-11-26).

## Q-19: Other node cases and where footprints live (T-107, status: answered)
- Context: 6.1 does not cover two collinear walls meeting end to end, or two walls leaving a node in the same direction. Q-7 stores derived node join types, but footprints were not listed.
- Options: (a) two collinear walls at a node meet with square ends at the node (a new join type `straight`, schema change); two walls leaving a node in the same direction, or more than four walls, are an error naming the node; footprints and layer polygons are computed on demand by a pure function, never stored; join types are stored on nodes; polygons are listed counterclockwise from the vertex with the lowest x, then lowest y; layers are listed left to right; (b) the same, but footprints stored in the document.
- Recommendation: (a). Footprints are cheap to recompute from nodes and layer sets, and storing them would put large derived data in every diff.
- Answer: (a) as listed: straight join type, errors, polygons computed on demand (owner, 2026-11-26).

## Q-20: How an opening is placed and solved (T-108, status: answered)
- Context: 6 gives an opening "host wall, along-wall reference and offset, width, height, sill"; T-104 added `anchor` (which opening edge sits at the offset). The sign of the offset, which edge is "start", which references are allowed, and where the solved position lives are not stated. Q-16 left OpeningEdge references for T-108.
- Options: (a) along coordinates run in the host's direction (node_a to node_b); the Start edge is the one nearer node_a; the anchor edge lies at coordinate(reference) + offset, measured in the host's direction; any reference with a coordinate on the host's along axis is allowed (nodes, datums, faces or centers of perpendicular walls, other openings' edges); the solver gives each opening one variable on that axis, so OpeningEdge references solve like any other; the solved start-edge coordinate is stored on the Opening as a derived field `start` (Q-7), written by apply_to; a clash involving a placement names the opening's id in the Conflict; (b) the same but the position is computed on demand and not stored.
- Recommendation: (a). It follows Q-7 and lets validation and export read positions without re-solving.
- Answer: (a) as listed, with the derived `start` stored on Opening (owner, 2026-11-26).

## Q-21: Default opening sizes and minimum jamb distance (T-108, status: answered)
- Context: 6.4 requires versioned defaults, always Proposed, and calls the examples "illustrative, to be confirmed". 7.7 requires a minimum jamb distance from the wall ends, "default Proposed". These numbers are the owner's.
- Options: (a) defaults catalog v1: door 3'-0" wide, 6'-8" high, sill 0"; window 3'-0" wide, 4'-0" high, sill 3'-0"; plain opening 3'-0" wide, 6'-8" high, sill 0"; minimum jamb distance 4"; (b) other values you give.
- Recommendation: (a): common US residential and light-commercial sizes, with 4" leaving room for king and jack studs at a corner. All are Proposed, so the user confirms each one.
- Answer: (a) door 3'-0" x 6'-8" sill 0; window 3'-0" x 4'-0" sill 3'-0"; opening 3'-0" x 6'-8" sill 0; minimum jamb distance 4" (owner, 2026-11-26).

## Q-22: The host profile, the void and "inside the host" (T-108, status: answered)
- Context: T-108 asks for "the void cut in the host profile" and sill and head heights; 7.7 requires every opening to lie fully inside its host with the minimum jamb distance from the wall ends.
- Options: (a) the host profile is the wall's elevation: along the wall from its start cut to its end cut, and up from its base (storey elevation + base_offset) to its top (storey elevation + floor_to_floor, or base + explicit height); each opening voids a rectangle from its start to end edge and from sill to head (head = sill + height), with sill measured from the storey elevation; "inside" means the opening's along span lies within the stretch where the wall has its full thickness on both faces, shrunk by the minimum jamb distance at each end, and sill and head lie within base and top; violations are typed and returned by a validation function that T-110 will gather with the other 7.7 checks; (b) sill measured from the wall base instead.
- Recommendation: (a). Sill above finished floor is how sills are specified; using the full-thickness stretch keeps an opening out of a crossing wall at a T and out of a miter at an L.
- Answer: (a) sill measured from the storey elevation (owner, 2026-11-26).

## Q-23: Implement CreateOpening now (T-108, status: answered)
- Context: Q-9 left CreateOpening returning NotYetImplemented "until its task"; T-108 asks for default sizes applied as Proposed, which happens at creation. MoveOpening belongs to T-113 (8.7).
- Options: (a) implement CreateOpening with args {kind, host, reference, anchor, offset, width, height, sill}, where width, height and sill may be null and then take the catalog default with source Proposed; the new id is the command's derived id; it fails if the host is unknown; (b) leave commands to a later task and expose defaults only as a function.
- Recommendation: (a). It is the one place defaults are applied, and it lets tests create openings the way the app will.
- Answer: (a) implement CreateOpening (owner, 2026-11-26).

## Q-24: What identifies a Space's room (T-109, status: answered)
- Context: 6 lists Space as "derived boundary, name, finishes", and T-109 derives the boundary "from the bounding wall faces". Nothing in the Space says which region of the plan it is, so there is no input to derive from.
- Options: (a) add `seed: Point` to Space, a point inside the room (as a room placement point in CAD); the boundary is the region containing the seed that the storey's wall footprints enclose; a seed inside a wall, or a region that is not closed, is an error; (b) add an ordered list of bounding walls and the side facing the room; (c) the user draws the boundary and it is only checked against the walls.
- Recommendation: (a). One point is the least the user or agent must supply, it survives walls moving, and the boundary follows from the walls deterministically.
- Answer: (a) seed point (owner, 2026-11-26).

## Q-25: A wall standing free inside a room (T-109, status: answered)
- Context: a wall that touches no other wall, inside a room, makes the room's region have a hole. Space stores one boundary polygon.
- Options: (a) v1 reports this as an error (IslandInSpace naming the walls), so every Space is one simple polygon; (b) add `holes` to Space; the area is the outer polygon minus the holes; slabs and ceilings from such a Space get the holes too.
- Recommendation: (a) for v1. The first use case has no free-standing walls, and it keeps Space, Slab and Ceiling single polygons; holes can be added later without changing anything else.
- Answer: (a) error in v1 (owner, 2026-11-26).

## Q-26: Representation choices for T-109 unless you object (T-109, status: answered)
- Choices: areas are exact i128 square nanometers (an i64 holds at most about 9.2e18 nm², which is only about 9.2 m² or 99 sq ft); boundaries are listed counterclockwise from the vertex with the lowest x, then lowest y, with collinear vertices removed; an explicit polygon must have at least 4 vertices, axis-aligned edges, positive area and no self-intersection or touching edges; a slab or ceiling from a space uses the space boundary unchanged (the room side of the wall faces); only walls on the space's storey bound it; the derived boundary is written to Space.boundary by an update function (Q-7).
- Recommendation: accept as listed.
- Answer: accepted as listed (owner, 2026-11-26).

## Q-27: The occurrence-overlap invariant before occurrences have footprints (T-110, status: answered)
- Context: 7.7's last invariant is "no Occurrence footprint overlaps a wall or another Occurrence unless marked as intentionally overlapping". T-110 depends on T-107 to T-109 only; occurrence placement geometry (what a Floor placement's x, y mean, wall-mounted depth, rotation) and any "intentionally overlapping" mark come in T-111. The document has no such mark yet.
- Options: (a) T-110 implements the other four invariants and reserves the violation kind `OccurrenceOverlap`; T-111 defines occurrence footprints, adds the `overlap_allowed` flag and the check with its test; (b) T-110 defines occurrence footprints now (Floor x, y as the footprint center, rotation by quarter turns, declared width and depth) and adds `overlap_allowed`, leaving wall-mounted footprints to T-111.
- Recommendation: (a). Footprint geometry belongs to T-111, whose Done when already covers rechecking items; defining it twice risks two definitions.
- Answer: (a) defer the occurrence check to T-111; the kind is reserved (owner, 2026-11-26).

## Q-28: Where an acknowledgment lives (T-110, status: answered)
- Context: violations "block commit unless the user marks them Acknowledged". Violations are computed, so the mark must be stored and must name the violation it covers.
- Options: (a) every violation has a key: its kind plus the sorted ids of the entities involved; the document stores `acknowledged`, a sorted set of keys; a violation whose key is in the set is reported with `acknowledged: true` and does not block; the mark stays while the same entities keep the same kind of violation and is dropped from the set when that violation no longer occurs (an update function prunes it); (b) the key also includes the violation's measured amount, so any change requires a new acknowledgment; (c) acknowledgments live in the proposal, not the document.
- Recommendation: (a). The key is stable across solves, the document records the decision, and pruning keeps old marks from silently covering a later, unrelated violation of the same entities.
- Answer: (a) key set in the document, pruned when the violation is gone (owner, 2026-11-26).

## Q-29: What counts as overlapping walls and a declared join (T-110, status: answered)
- Context: 7.7: "no two walls overlap or cross unless they form a declared join".
- Options: (a) two walls form a declared join when they share a node; any other two walls on the same storey violate if their exact footprints share area (touching along an edge or at a point is allowed); a wall that fails "positive length" (nodes not ordered along its direction, or zero length) is reported and left out of the other geometric checks, along with openings it hosts, so one bad wall does not hide everything else; Space and Slab outlines that cannot be derived or are not simple rectilinear polygons are reported with the reason; (b) also flag walls sharing a node that overlap beyond their join.
- Recommendation: (a). With orthogonal walls, two walls that share a node can only meet at that node (same-direction walls at a node are already an error), so (b) adds nothing.
- Answer: (a) as listed (owner, 2026-11-26).

## Q-30: Occurrence frame and placement geometry (T-111, status: answered)
- Context: 6.3 gives Floor (x, y, rotation, elevation), WallMounted (wall, side, along-wall reference and offset, elevation) and CeilingMounted (x, y), and ItemEdge names front, back, left and right, but none of these is pinned down: which point x, y is, which side is the front, which point the wall-mounted offset reaches.
- Options: (a) an item's footprint is its declared width along its local X and depth along its local Y; its front faces local -Y, back +Y, left -X, right +X; rotation turns that frame counterclockwise in quarter turns. Floor and ceiling x, y are the footprint center (a center on an odd number of nanometers is rounded half to even, as WallCenter). Elevation is the item's bottom above the storey elevation. A ceiling-mounted item has rotation 0 and its top at the storey's default ceiling height. A wall-mounted item has its back against the `side` face of the wall and its front facing away, and `offset` runs from the reference to the item's center along the wall, positive toward node_b; (b) the same, but x, y is the footprint corner with the lowest x and y, and wall-mounted offset reaches the item's start edge.
- Recommendation: (a). Center placement does not change under rotation, and it matches ItemCenter.
- Answer: (a) center placement (owner, 2026-11-26).

## Q-31: Occurrences in the solver; Fixed (T-111, status: answered)
- Context: Q-16 left ItemEdge and ItemCenter to T-111. 6.3: floor items do not move with walls; wall-mounted items move with their host; "a Fixed occurrence never moves as a consequence of another edit; an edit that would require it to move is a Conflict".
- Options: (a) floor and ceiling items get an x and a y variable (their center), like nodes: positions are solved, an unconstrained item keeps its stored position, and nothing ties it to walls, so it stays when a wall moves unless a constraint links them; a wall-mounted item gets one variable along its host (placed by reference and offset, like an opening) and its across coordinate is the host's face, so it follows the wall; ItemEdge and ItemCenter resolve through the item's rotation, and a reference with no coordinate on the requested axis is WrongAxis; a Fixed item is anchored at its stored position on both axes, so any constraint or wall move that would shift it is a Conflict naming the occurrence; the solved center is stored on the Occurrence as a derived `center` (Q-7) and floor and ceiling x, y are written back; (b) floor x, y with source UserStated are anchored like Fixed, and only Proposed or Derived positions are free.
- Recommendation: (a). It treats item positions exactly as node positions are treated, and Fixed is the one explicit way to pin something.
- Answer: (a) items solved like nodes; Fixed pins them (owner, 2026-11-26).

## Q-32: Clearance zones, MinClearance and occurrence overlap (T-111, status: answered)
- Context: T-111 asks for clearance zones and "MinClearance evaluated after solving" (7.1); Q-27 moved the 7.7 occurrence-overlap check here with an "intentionally overlapping" mark.
- Options: (a) a clearance zone is the footprint grown by its front, back, left and right extents (in the item's frame); it must not overlap any wall footprint or another occurrence's footprint on the storey; MinClearance(object, other, value) holds when the exact distance between the object's footprint and the other's (an occurrence footprint, or a wall footprint's bounding rectangle) is at least value, compared as squared distances in exact integers; two occurrences overlap when their footprints share plan area and their height ranges overlap (so a ceiling light over a bench is fine), and an occurrence overlaps a wall when their footprints share area; `overlap_allowed` on Occurrence exempts it; failures are validity violations of kinds ClearanceZone, MinClearance and OccurrenceOverlap that block commit unless acknowledged; (b) clearance zones only feed MinClearance and are not checked on their own.
- Recommendation: (a). It makes "floor items are rechecked" concrete and uses the acknowledgment mechanism already in place.
- Answer: (a) as listed (owner, 2026-11-26).

## Q-33: The parametric box type (T-111, status: answered)
- Context: T-111 asks for "a parametric box type at the Proxy tier" (10.1: Proxy is a parametric box or template). PlaceOccurrence and MoveOccurrence are in the 8.2 command set but T-111 does not mention them.
- Options: (a) a function that builds an ElementType box from a name, width, depth, height and authority, with fidelity Proxy, no model, empty properties, source "parametric box" and license "n/a"; PlaceOccurrence stays NotYetImplemented for a later task, and MoveOccurrence stays with T-113; (b) also implement PlaceOccurrence now.
- Recommendation: (a). T-111's Done when is about solving and checking placed items, which tests can build directly.
- Answer: (a) function only (owner, 2026-11-26).

## Q-34: Where a proposal lives between propose and commit (T-112, status: answered)
- Context: 8.3: "Nothing is persisted by propose"; 8.4: commit(proposal_id) succeeds only if base_version equals head. Core has no I/O and no store.
- Options: (a) propose returns a Proposal value and keeps nothing; the caller (app, CLI, MCP server) holds it and passes it to commit; proposal_id is the UUIDv5 of the base version and the resolved action, so it is deterministic, and commit recomputes it to make sure the proposal was not altered; (b) core keeps an in-memory table of pending proposals keyed by id inside a session object.
- Recommendation: (a). It is pure, it makes "propose persists nothing" literally true, and the id still identifies the proposal exactly; the hosts in T-114 and later can keep their own table of ids.
- Answer: (a) the caller holds the proposal; deterministic id (owner, 2026-11-26).

## Q-35: The commit pipeline and what blocks a commit (T-112, status: answered)
- Context: until now a Commit event only applied commands; solving, join types and space boundaries were separate functions. A committed document should be fully derived, and replay must give the same bytes. 7.7 says unacknowledged violations block commit; 9.2 says unresolved Referent, Direction and ReferenceSemantics ambiguities block it; 7.5 says a Conflict is for the user to decide.
- Options: (a) every commit, and every replay of one, applies the commands and then derives: solve and write back positions, store join types, store space boundaries (a space whose boundary cannot be derived keeps its previous one and shows up as a violation); propose runs the same pipeline; commit is refused while the proposal has an unacknowledged violation, a solver Conflict, or a blocking ambiguity; (b) the same, but solver Conflicts do not block.
- Recommendation: (a). A conflicted model has no consistent geometry, and committing it would store positions that no constraint set explains.
- Answer: (a) derive on every commit and replay; violations, Conflicts and blocking ambiguities block (owner, 2026-11-26).

## Q-36: Diff, flags and ambiguities in T-112 (T-112, status: answered)
- Context: 8.3 asks for a diff "per entity, before and after, in nanometers and display units", plus ambiguities and flags; 9.2 defines ambiguity kinds; 6.7 defines ExistingConditionEdit; 7.4 and 9.4 care about Proposed values. JSON alone cannot tell a length from another integer.
- Options: (a) the diff has one entry per changed entity: entity maps by collection and id, and project, building, labels, acknowledged and default_walk_start as single entries; each entry says added, removed or changed and carries the whole entity before and after in nanometers and in display form (every Length shown as feet-inches-fractions), plus the JSON paths that changed. Lengths are found through the JSON Schema, where Length becomes a named definition (schema change). Flags: ProposedValue for every changed or added field whose source is Proposed, and ExistingConditionEdit for a changed or removed entity with function existing, or hosted on such a wall. The Ambiguity type follows 9.2 exactly, and the list stays empty until T-113 resolves moves; (b) a diff of changed fields only, without whole entities.
- Recommendation: (a). Whole entities make the diff self-explanatory to the user and the agent, and finding lengths by schema keeps one source of truth for which values are lengths.
- Answer: (a) whole entities in nm and display units, lengths found via the schema (owner, 2026-11-26).

## Q-37: Cases (a) to (d) of 8.7 as precise rules (T-113, status: answered)
- Context: 8.7 describes the cases in words. To implement them they must be stated over the solver's graph for the move's axis: vertices are solver variables plus ground (Datums and Fixed values); edges are driving constraints and placements (editable dimensions), wall ties and Fixed-item anchors (not editable). The moved reference R is a variable plus a constant.
- Options: (a) rules: R's entity is "fixed through" the editable bridges that separate R from ground (edges whose removal disconnects R from ground), taken from the one nearest R. Case (a): the nearest such bridge touches ground directly (a Datum distance or a Fixed): change its value by the delta. Case (b): it does not (the chain passes through other entities): change it by the delta (the default), list as consequences every entity whose solved position changes, and offer one PolicyChoice per further editable edge leaving R's entity away from ground (the "far side"): change that edge by minus the delta, so what lies beyond it holds still, with the resulting value of every edited dimension. Case (c): no editable bridge separates R from ground because R reaches ground by two or more edge-disjoint routes: a ReferenceSemantics ambiguity naming each editable constraint at R's entity on those routes, and no edit until the user picks one. Case (d): R's entity, or any entity that the chosen edit would move, is a Fixed occurrence (or a wall anchored by one): a Conflict naming it. Consequences are found by solving the edited document and comparing positions, so they are exact; (b) a simpler reading: only the constraints directly attached to R's entity are ever edited, with no chain analysis.
- Recommendation: (a). It is the literal content of 8.7 expressed over the structure the solver already builds, and every case is decided deterministically.
- Answer: (a) graph rules (owner, 2026-11-26).

## Q-38: Moving something no constraint fixes (T-113, status: answered)
- Context: 8.7 covers references fixed by constraints. A reference in an underconstrained class (7.3) is held only by its provisional position (7.4).
- Options: (a) the move translates the stored positions of the whole free class by the delta (everything in it is rigidly linked), lists the other members as consequences, and edits no constraint; (b) the move adds a Proposed Fixed constraint at the new position.
- Recommendation: (a). It moves exactly what the user sees move and adds nothing they did not ask for.
- Answer: (a) translate the free class (owner, 2026-11-26).

## Q-39: Move command arguments and what is logged (T-113, status: answered)
- Context: 8.2 gives MoveWall(wall, delta or target, reference) and names MoveOpening and MoveOccurrence without arguments. 8.7 says moves are "shorthand for editing constraints". Replay must reproduce a resolved move exactly, and 9.2 needs a way to answer an ambiguity or pick a PolicyChoice.
- Options: (a) MoveWall {wall, reference (the wall's face or center whose coordinate the target names), delta or target (on the wall's perpendicular axis, positive toward +X or +Y), edit (optional: the constraint the user picked in case (c)), absorb (optional: the PolicyChoice the user picked)}; MoveOpening {opening, delta or target (the start edge, on the host's axis), edit, absorb}; MoveOccurrence {occurrence, axis, delta or target (the center), edit, absorb}, one axis per command, wall-mounted items only along their host. Resolution turns a move into explicit SetConstraintValue commands (now implemented, with a target that is a constraint's value, an opening's offset, a wall-mounted occurrence's offset, or, for Q-38, a stored position), and those are what the proposal's resolved commands and the log hold; (b) the log keeps the Move command and re-resolves it on replay.
- Recommendation: (a). The log then records exactly which dimensions changed, replay cannot drift if resolution rules change later, and SetConstraintValue is itself in 8.2.
- Answer: (a) log the resolved SetConstraintValue edits (owner, 2026-11-26).

## Q-40: Case (c): what happens to the other dimension (T-113, status: answered)
- Context: a wall held 10 ft from a west datum and 5 ft from an east datum is moved 1 ft east; the user picks which dimension changes.
- Options: (a) both update: the picked one becomes 11 ft and the other is updated to 4 ft so it stays consistent, both shown as resulting dimensions; (b) the other is demoted to Reference with a 1 ft residual; (c) the other holds its value, so whatever it connects to moves (a Conflict if that is anchored).
- Recommendation: (a).
- Answer: (a) both update (owner, 2026-11-26).

## Q-41: What required_parameters() lists, and in what order (T-114, status: answered)
- Context: 7.4 lists {entity, field, reason, proposed default if any, unit}, "ordered by dependency". Under Q-4 every parameter holds a value with a source, so nothing is ever absent: a value the user did not give is a catalog default with source Proposed. T-114's Done when speaks of "a plan with no wall thickness"; T-115 will add a roof's missing pitch or low edge.
- Options: (a) required_parameters() lists every value with source Proposed (reason: proposed default, with that value as the proposal) plus any parameter a task declares required that has no value and no default (reason: missing, no proposal; none exist until T-115); "no wall thickness" means the wall's layer thicknesses are catalog defaults; the model is provisional while the list is not empty, and the Proposal carries that flag; the Proposed items attached to exports are the same list; order by dependency is a fixed order of kinds, each depending only on earlier ones: storeys, layer sets, walls, openings, spaces, slabs, ceilings, roofs, element types, occurrences, materials, underlays, camera bookmarks; within a kind by label (or id), then by field path; units are length, lumens, kelvin, pitch or none; (b) list only truly missing values and keep Proposed values in a separate list.
- Recommendation: (a). It is the only reading in which "a plan with no wall thickness" can exist under Q-4, and one list serves 7.4, 11.3 and 12.4.
- Answer: (a) one list of Proposed values, ordered by kind (owner, 2026-11-26).

## Q-42: Defaults catalog v2 values (T-114, status: answered)
- Context: 6.4: "every optional geometric parameter has a versioned default"; the examples are "illustrative, to be confirmed". Catalog v1 (Q-21) has openings and the jamb distance. These numbers are the owner's.
- Options: (a) catalog v2 adds: interior stud wall = 1/2" gypsum, 3 1/2" stud (core), 1/2" gypsum, reference line on the stud center (left extent 2 1/4"); wall base offset 0"; storey floor-to-floor 10'-0" and default ceiling height 9'-0"; slab thickness 4"; ceiling thickness 1/2"; camera eye height 5'-4"; clearance extents 0"; roof thickness 8", fascia depth 6", eave overhang 12" on every side; (b) other values you give.
- Recommendation: (a): common US light-commercial values; each is Proposed, so the user confirms it.
- Answer: (a) the listed v2 values (owner, 2026-11-26).

## Q-43: How the user supplies a wall thickness (T-114, status: answered)
- Context: the Done when needs "supplying the value clears its Proposed flag". SetWallLayers is in 8.2 but not implemented; layer sets can be shared by several walls.
- Options: (a) implement SetWallLayers {wall, layers, left_extent}: the wall gets exactly those layers and left extent with the sources given; if its layer set is shared with other walls, the wall gets a new layer set (the command's derived id) so the others are untouched, otherwise its set is edited in place; then the document is solved like any commit; (b) add layer thickness as a SetConstraintValue target instead.
- Recommendation: (a). It is the 8.2 command made for this, and never changing other walls by surprise matches P4.
- Answer: (a) SetWallLayers, copying a shared layer set (owner, 2026-11-26).
