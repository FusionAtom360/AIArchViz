# Parametric Floor Plan to Architectural Visualization System: High-Level Design and Specification

Draft 0.4 | November 27, 2026 | Status: owner answers incorporated and adversarial review applied (section 18); material sourcing and generation added on the asset model (Q-16). Items marked OPEN need an owner decision.

## 1. Purpose and scope

**1.1 Purpose.** The System lets a person with no CAD, Blender, or Unreal Engine (UE) skills describe a building using measurements they know. It produces a dimensionally exact, human-verifiable floor plan and 3D model, and exports an interactive visualization to UE5. Revisions are requested in natural language; an AI agent converts them into exact, deterministic commands.

**1.2 First use case.** A shop containing many pieces of equipment (desks, toolboxes, machines) gains two interior walls and an exterior extension. The client knows CAD but wants a precise visualization of the future layout without operating CAD, Blender, or UE. The System MUST be general beyond this case within the limits of 1.4.

**1.3 Priority order.** (1) precision, (2) predictability, (3) ease of use for non-experts, (4) realism. Earlier items win conflicts.

**1.4 v1 scope.** Axis-aligned (rectilinear) walls; one storey plus a same-level extension; imperial units at 1/16 in resolution; doors, windows and openings; floors and ceilings; a parametric shed roof for the extension; equipment and furniture as exact footprints with optional models; lights; materials; variants; DXF (AutoCAD 2018 compatible) and glTF export; an interactive UE 5.8 scene as a packaged Windows build.

**1.5 Non-goals for v1.** Structural or code analysis, MEP, non-orthogonal or curved walls, stairs and multi-storey, terrain, cost estimating, multi-user collaboration, hosting or billing, AI-generated images as deliverables. Gable, hip and other roof types are v1.x or later.

## 2. Principles

- **P1 Single source of truth.** The Project Document is authoritative. The 2D drawing, 3D model, IFC and UE scene are regenerable projections. Nothing is ever edited downstream.
- **P2 The agent never computes coordinates.** The agent supplies intent, user-stated values verbatim, and a choice of deterministic operator and policy. Deterministic code computes every coordinate and displacement. Approximate quantities from the user become flagged Proposed values (9.2).
- **P3 Exactness.** Lengths are integers. Every edit is exactly invertible.
- **P4 Ambiguity is surfaced, never silently resolved.** Unresolved ambiguity of reference or direction blocks a commit. Unconstrained design choices may be proposed but are flagged.
- **P5 Everything is a command.** Commands are deterministic, logged and undoable.
- **P6 Provenance.** Every value and asset records its origin and fidelity.
- **P7 Works without the agent.** Any capability reachable by the agent is reachable from the UI and CLI.
- **P8 Free and open dependencies by default.** Paid services are optional adapters.

Keywords MUST, SHOULD and MAY follow RFC 2119. IDs: D-n decisions, S-n spikes, Q-n open questions.

## 3. Architecture

- **C1 core.** Rust library with no I/O, no wall clock, no randomness. Contains the document model, solver, commands, validation and pure export logic. MUST compile for native Windows (x86\_64) and wasm32; CI builds both.
- **C2 cli.** Rust binary. JSON in, JSON out. Used by tests, scripts and external agent harnesses.
- **C3 mcp.** MCP server over stdio exposing the tool set in section 9.
- **C4 agent.** Built-in agent loop with provider adapters, calling the same tools in-process.
- **C5 app.** 2D editor, measurement-capable 3D viewer, proposal review. A client of the same command API as C2 to C4, with no business logic of its own. UI technology is egui (D-14).
- **C6 converters.** Optional external headless processes (CAD and mesh conversion, texture conversion, mesh and material generation, Blender cleanup).
- **C7 ue-bridge.** UE5 C++ plugin and content that consume the Scene Package and provide runtime interaction.

```
User -> C5 app ----------\
Agent -> C3 mcp / C4 agent -> command API (C2) -> C1 core -> Project Document + command log
                                                        |-> DXF, glTF, IFC, Scene Package -> C7 -> UE5 scene
C6 converters <-> asset pipeline <-> C1 core
```

**3.1 Process model.** A project is opened by exactly one host process at a time: the app (C5) when it is running, otherwise the CLI or MCP server. A second client attaches to the host over a local named pipe with a per-session token. If no host is running, a client opens the project directly under an exclusive file lock. Clients never write project files concurrently. Pointer and drag input in C5 is converted to world coordinates in 64-bit arithmetic and snapped to the 1/16 in grid before it becomes a command argument (5.3); no coordinate derived from a floating-point screen position is committed unsnapped.

## 4. Foundational decisions

| ID | Decision | Rationale |
| --- | --- | --- |
| D-1 | Core language is Rust; C1 is pure. | One implementation for CLI, MCP, UI and browser; determinism. |
| D-2 | Canonical store is the System's own versioned schema with IFC-aligned semantics (4.1). IFC is an export adapter (v1.x) and import adapter (v2), not the store. | IFC records results, not intent; see 4.1. |
| D-3 | Length is a signed 64-bit integer count of nanometers. | Exact for mm and for inch fractions down to 1/64 in. |
| D-4 | Geometry is 2.5D: extruded 2D profiles and prisms. Booleans are done in 2D. No 3D B-rep kernel. | Removes the largest robustness risk. |
| D-5 | v1 is orthogonal only: every wall reference line is parallel to the X or Y axis; item rotation is a multiple of 90 degrees. | Makes exact solving possible without trigonometry. |
| D-6 | The solver sits behind one trait. Layer A, an exact axis-offset solver (section 7), ships in v1. Layer B, a general numeric solver, is v2 and optional. | Isolates the hardest generalization. |
| D-7 | All edits are commands. Agent edits go through propose then commit. | Predictability, undo, audit. |
| D-8 | The tool API is the agent contract. Two front ends: MCP server for external harnesses, and a built-in loop with a user-supplied API key. | Harness-neutral. |
| D-9 | UE target is Windows only, pinned to UE 5.8 as a packaged build (D-15). The Scene Package (section 11) is the contract. Ingest path is decided by spike S-2. | Runtime import is the least proven part. |
| D-10 | Blender and FreeCAD/OpenCascade CLIs are optional converters, never in the critical path. | Core works if they are absent. |
| D-11 | Dimension authority, highest first: user-stated exact, manufacturer spec, CAD-derived, measured with tolerance, proposed default, mesh-derived. | Resolves conflicts predictably. |
| D-12 | Every entity has a UUID. Derived entities use UUIDv5 of (parent id, operator, slot). IFC GlobalId is the standard 22-character compression of the UUID. | Stable identity across regeneration and export. |

**4.1 BIM stance.** BIM supplies a vocabulary and an interchange standard (IFC). It does not supply parametric behavior. IFC stores geometry and placements, not the constraints or feature history that produced them, so the solver and feature tree must live in the System's own model regardless. The semantics to ADOPT now: spatial hierarchy (Project, Building, Storey); type versus occurrence split; host and hosted elements (an opening voids a wall, a door fills an opening); material layer sets with a reference line and offset (this is exactly the centerline versus finish-face problem); spaces bounded by walls; property sets; GUIDs; explicit units. NOT adopted: IFC as the native store, because of schema size, STEP serialization overhead, immature Rust tooling and the lack of an intent model. Consequence: an IFC exporter is a bounded adapter. Retrofitting these concepts onto a flat model of line segments and boxes later would be a rewrite of the model, command set and agent tools.

**4.2 Owner decisions.** Answers to Q-1 through Q-10 are resolved as follows.

- **D-13 Units.** Imperial only (feet, inches, fractions) at 1/16 in resolution (see 5.3). Metric appears only when reading manufacturer or CAD specifications, and is converted on import.
- **D-14 UI.** egui (eframe), native Windows first; the same code targets wasm32 later. The 3D viewer is a purpose-built wgpu view inside the app (orbit, pick, section, measure). It is a verification tool, not a visualization product.
- **D-15 Unreal target.** UE 5.8 as a packaged build. Epic describes 5.8 as the last planned major UE5 release, which makes it a stable pin. All runtime behavior is C++ or Blueprint (12.2).
- **D-16 CAD interchange.** The client uses AutoCAD 2018. DXF output MUST open there without repair prompts and register at correct scale (11.1).
- **D-17 Agent provider.** OpenAI first. The AgentBackend interface stays provider-neutral; other adapters follow after v1.
- **D-18 Mesh and material generation.** Local open-weights models by default, with a hosted service as an optional adapter (10.6). Materials are sourced and generated on the same model as 3D assets (section 10, Q-16).
- **D-19 License.** The System (C1 to C6) is GPL-licensed (GPLv3; the owner delivers a working product to the client and publishes the source). C7, the UE plugin, MUST NOT link against C1 and communicates only through the Scene Package, because Unreal Engine's license is not expected to be GPL-compatible (confirm against the Epic EULA). All dependencies MUST be GPL-compatible. C7 is licensed MIT (owner decision; the rest of the System is GPLv3) so that its linkage with Unreal Engine is unambiguous; it shares no code with C1, only the Scene Package specification.
- **D-20 Existing conditions.** Editable by the client with the same commands and constraints as new work (6.7).
- **D-21 Roof.** The parametric shed roof is in v1, because the first use case needs it (6.5).
- **D-22 Paper plan.** The client's paper floor plan enters as a calibrated underlay plus transcribed dimensions (6.6).

**D-23 Build versus adopt.** A custom core is chosen over building the agent layer on FreeCAD (BIM workbench, Python API, constraint solver) or on Blender with Bonsai, because the System needs exact integer geometry, a closed command set with an ambiguity protocol, and no CAD interface exposed to the client. The cost is a larger build. Revisit if a pivot trigger in section 15 fires, S-1 in particular.

## 5. Units, coordinates, identity

**5.1 Length.** Stored as i64 nanometers (1 in = 25,400,000). Add, subtract and integer multiply are exact. Division and rational scaling use i128 intermediates and round half to even to the nearest nanometer. Inch fractions finer than 1/64 in are rounded on input with the error reported.

**5.2 Input parsing.** The parser MUST accept forms such as 10'-4 1/2", 10 ft 4.5 in, 124.5in. A bare number uses the project default unit. The UI and agent MUST echo every parsed value in canonical feet-inches-fraction form before commit. Metric forms are accepted only in asset specification fields (declared dimensions and tile sizes, 10.2).

**5.3 Display.** All lengths display in feet, inches and fractions to the nearest 1/16 in. A stored value that is not exactly on the 1/16 in grid (for example a derived midpoint or a converted metric dimension) displays with a leading \~, and its exact value is shown on inspection and in reports. Display rounding NEVER alters stored values. User-stated inputs MUST lie on the 1/16 in grid: finer input is rejected and the nearest grid value is echoed for confirmation. Derived values and converted manufacturer or CAD dimensions are exempt.

**5.4 Coordinate systems.** World: right-handed, X east, Y north, Z up, plan view with X right and Y up. UE: left-handed, Z up, centimeters; UE = (x, -y, z) with 1 cm = 10,000,000 nm. glTF: right-handed, Y up, meters; glTF = (x, z, -y). A conformance fixture (an asymmetric L-shaped room with labeled fiducials) MUST pass in every exporter and in UE (S-3).

**5.5 Identity and versioning.** Human labels (W-007, EQ-012) are unique per project and never reused. The document has a schema\_version and migrations. Canonical serialization is JSON with sorted keys and no insignificant whitespace; the same command log MUST yield a byte-identical document. Core geometry and the solver MUST use integer or rational arithmetic only; floating point is permitted only in export adapters and the UI. Timestamps, user names and other session metadata live in the log envelope, outside the canonical document.

## 6. Domain model (IFC-aligned)

| Entity | IFC analog | v1 fields |
| --- | --- | --- |
| Project, Building, Storey | IfcProject, IfcBuilding, IfcBuildingStorey | Storey: elevation, floor-to-floor height, default ceiling height |
| Datum | IfcGrid, placement origin | Named point or line anchoring an axis. At least one per axis is REQUIRED. |
| Node | polyline vertex | x, y (solved), derived join type |
| Wall | IfcWall | node\_a, node\_b, layer\_set, left\_extent, base\_offset, top (storey or explicit), join\_priority, function (existing, new, demolished) |
| LayerSet | IfcMaterialLayerSet | Ordered layers left to right: material, thickness, core flag |
| Opening, Door, Window | IfcOpeningElement, IfcDoor, IfcWindow | host wall, along-wall reference and offset, width, height, sill |
| Slab, Ceiling | IfcSlab, IfcCovering | rectilinear boundary (explicit or from Space), thickness, elevation |
| Space | IfcSpace | derived boundary, name, finishes |
| ElementType | IfcTypeObject | declared\_dimensions (authority per D-11), fidelity, model\_ref, variant slots, properties, source, license |
| Occurrence | IfcFurniture, IfcBuildingElementProxy, IfcLightFixture | type, placement, rotation, elevation, clearance zones, label |
| Feature | none | operator id and version, parameters, targets, derived children |
| Constraint | IfcConstraint (weak) | kind, references, value, tolerance, status, provenance |
| VariantSlot | none | owner, kind (material, model, lighting), options, selected |
| Material | IfcMaterial | name, fidelity, tile\_size (authority per 10.2), texture\_set\_ref (Asset of kind texture\_set) or Proxy parameters (base color, roughness, metallic), properties, source, license |
| Asset | IfcDocumentReference | kind (model, texture\_set), content hash, source URL, license, packaging permission, fidelity, converter log |

**6.1 Wall geometry.** Direction is node\_a to node\_b. Left is 90 degrees counterclockwise in plan. Layers are ordered left to right. left\_extent is the distance from the reference line to the left face; right\_extent is total thickness minus left\_extent. Joins: at a node with two walls, an L join is mitered; at three or more, the wall with the lowest join\_priority runs through and others butt; free ends are square. Rules are deterministic and independent of creation order.

**6.2 References.** A reference is one of: Node, WallFace(wall, side, layer boundary index), WallCenter, OpeningEdge(opening, start, end, center), ItemEdge(item, side), ItemCenter, Datum. Every dimension MUST name its reference kinds. Dimensions given as 'room is 20 by 30' trigger a prompt for which faces they measure.

**6.3 Placement modes.** Floor (x, y, rotation, elevation); WallMounted (wall, side, along-wall reference and offset, elevation); CeilingMounted (x, y). Wall-mounted occurrences move with their host wall. Floor occurrences do not move with walls but are re-checked for clearance. Columns, beams, electrical panels and similar fixed obstructions are Occurrences of parametric box types with an elevation and a Fixed flag in v1. A Fixed occurrence never moves as a consequence of another edit; an edit that would require it to move is a Conflict.

**6.4 Defaults catalog.** Every optional geometric parameter has a versioned default, always applied with status Proposed. Example defaults (illustrative, to be confirmed): interior stud wall with gypsum both sides, standard door sizes.

**6.5 Shed roof.** A single inclined plane over a rectilinear footprint; it is the one exception to flat-topped prisms in D-4. Parameters: footprint (explicit polygon or the Space of the extension); slope direction (one of +X, -X, +Y, -Y); high-edge and low-edge heights, or pitch as rise:run (for example 3:12); eave overhang per edge; thickness; fascia depth; material slots. Heights derived from a pitch use the rounding rule in 5.1. A roof MAY name a ledger reference (a face of an existing wall) where its high edge ends, with the high-edge height measured there. The typical case is a lean-to: the high edge sits on a ledger reference on an existing wall face and the roof slopes away from that wall, so slope direction is derived and the System asks for pitch or low-edge height as a missing parameter (7.4). The existing building's roof is not modeled in v1.

**6.6 Underlay.** A raster image (scan or flat photo of the paper plan) placed and calibrated by two points and a known distance; a second known distance reports the calibration residual. An underlay is display-only and never drives geometry. Dimensions read from it are entered as Measured constraints with provenance Transcribed and MUST be confirmed by the user. Perspective correction is out of scope for v1. The agent MAY propose transcriptions and wall traces from an underlay as Proposed values flagged for review.

**6.7 Existing conditions.** Existing elements (function = existing) are fully editable through the same commands and constraints as new elements. A proposal that changes an existing element is flagged ExistingConditionEdit and shows the original value from the baseline snapshot (the first committed state, or a snapshot the user creates). The diff view shows existing, new and demolished elements together.

## 7. Constraints and solver

**7.1 Constraint kinds.** Distance(refA, refB, axis, value, tolerance); Align(refA, refB, axis); Equal(distance1, distance2); Fixed(ref, value); MinClearance(object, object or wall, value), which is an inequality checked after solving rather than solved in v1.

**7.2 Status and source.** Each constraint is Driving or Reference (shows the computed value and residual, constrains nothing). Source is one of UserStated, Measured, Derived, Proposed, Imported.

**7.3 Layer A algorithm.** The X and Y axes are solved independently. Every reference resolves to a coordinate variable plus a known constant offset (wall thickness and face offsets are constants once specified). Each equality constraint merges two variable classes in a weighted union-find that stores the integer offset between them. Merging two members of the same class with a different offset is a Conflict. A class containing a Datum or Fixed value is determined; any other class has exactly one degree of freedom (a translation) and is reported as Underconstrained with its members. Complexity is near-linear. Results MUST NOT depend on constraint insertion order. Solver variables are reference-plane coordinates (wall faces, centerlines, opening edges, item edges); node positions are derived from them. Every parameter that affects an offset (wall thickness, item width) has a value when solving, either user-stated or Proposed, so offsets are constants. Changing such a parameter re-solves and moves any reference that is not anchored on the changed side.

**7.4 Missing-parameter protocol.** required\_parameters() returns an ordered list of {entity, field, reason, proposed default if any, unit}, ordered by dependency. The System MUST provide a provisional model using Proposed values so exploration is never blocked. Every export of a provisional model MUST list its Proposed items.

**7.5 Conflicts and reconciliation.** If a redundant Measured constraint disagrees within its tolerance (default 1/4 in, configurable), it becomes a Reference constraint and the residual is shown. If it disagrees beyond tolerance, a Conflict is raised naming both constraints; the user decides which is driving. No silent averaging.

**7.6 Layer B (v2).** General 2D constraint solving for non-orthogonal geometry. Candidate libraries (planegcs, SolveSpace slvs) have licenses requiring review before distribution (OPEN Q-9). The trait boundary in D-6 MUST NOT leak solver internals.

**7.7 Validity invariants.** After every solve, and before any proposal is returned, the System MUST check the following and report failures as violations that block commit unless the user marks them Acknowledged: every wall has positive length; no two walls overlap or cross unless they form a declared join; every opening lies fully inside its host wall with at least a minimum jamb distance from the wall ends (default Proposed); every Space and Slab boundary is a simple rectilinear polygon; no Occurrence footprint overlaps a wall or another Occurrence unless marked as intentionally overlapping. The equality solver does not enforce ordering, so these checks are the only guard against a move that carries a wall through another.

**7.8 Out-of-square buildings.** Existing buildings are rarely exactly orthogonal, and field checks by diagonals will disagree with an orthogonal model. Diagonal measurements are stored as Reference constraints whose residual is reported. If any residual exceeds its tolerance, the System warns that the as-built geometry is out of square by that amount, states that v1 idealizes it as orthogonal, and asks the user which wall faces are treated as true. New work that ties into an out-of-square wall is flagged with the gap or overlap the idealization introduces.

## 8. Commands, proposals, transactions

**8.1 Command.** {id, kind, args, idempotency\_key}. Commands contain no clock values or randomness. New entity ids are UUIDv5 of (idempotency\_key, ordinal).

**8.2 v1 command set.** CreateNode, CreateWall, SplitWall, MergeWalls, SetWallLayers, MoveWall(wall, delta or target, reference), CreateOpening, MoveOpening, PlaceOccurrence, MoveOccurrence, RotateOccurrence, DeleteEntity, AddConstraint, RemoveConstraint, SetConstraintValue, SetStorey, ApplyOperator, RemoveOperator, SetVariantSelection, SetProjectUnits.

**8.3 Proposal.** propose(commands\[\]) returns {proposal\_id, base\_version, resolved\_commands (explicit values), diff (per entity, before and after, in nanometers and display units), constraint\_status, violations\[\], ambiguities\[\], flags\[\]}. Nothing is persisted by propose.

**8.4 Commit.** commit(proposal\_id) succeeds only if base\_version equals head; otherwise it fails and the proposal must be regenerated. Commit is atomic. Undo and redo restore exact prior states; the command log is append-only with undo recorded as events.

**8.5 Operators (features).** A Feature stores an operator id, version, parameters and targets, and owns derived children with UUIDv5 ids. When a target changes, the System regenerates children and reports the diff. v1 operators: baseboard, ceiling\_light\_grid, wall\_finish, floor\_finish, clearance\_resolve (8.8). Each declares a parameter schema with units, defaults and regeneration triggers. Operators are pure functions of the document.

**8.6 Unconstrained choices.** When a command leaves a design decision open (for example how neighboring equipment adjusts to a moved wall), the System MAY generate a proposal with each decided value flagged Proposed and a list of the unconstrained choices. The user accepts, or pins a value, which converts it to a UserStated constraint.

**8.7 Editing under constraints.** Position commands (MoveWall, MoveOpening, MoveOccurrence) are shorthand for editing constraints. The System resolves each to the driving constraint or constraints that fix the moved reference and edits their values. (a) Exactly one driving constraint fixes the reference: its value changes by the delta. (b) The reference is fixed only through a chain to a Datum: everything on the far side of the edited link moves, and the proposal MUST list those entities as consequences and MUST offer each alternative absorber (holding the far side fixed and changing the free dimension on the other side of the link) as a PolicyChoice with the resulting dimensions shown. (c) Two or more independent driving constraints fix the reference: raise a ReferenceSemantics ambiguity naming each candidate constraint; the agent MUST ask which dimension changes. (d) The reference, or anything that would move, is a Fixed obstruction: Conflict.

**8.8 clearance\_resolve.** The deterministic operator behind requests such as adjust the furniture to fit. Inputs: the set of Occurrences allowed to move, an axis policy (the normal of the moved wall by default), and the violated clearance or validity constraints. For each violation in label order, compute the minimal translation along the policy axis that satisfies it. If the translated item then violates another constraint, push that item the minimal amount in the same direction, repeating until no violation remains. If a Fixed obstruction or a wall blocks the chain, the operator fails and reports the blocking chain. Shifts are integer nanometers rounded away from the violated clearance to the 1/16 in grid, and ties are broken by label order. The result is a proposal, never a commit. The agent chooses the inputs and the policy but never the displacements.

## 9. Agent integration

**9.1 Tool set.** Tiers: T0 read, T1 propose, T2 commit and file output, T3 network fetch, T4 spend.

| Tool | Tier | Purpose |
| --- | --- | --- |
| get\_scene\_summary, get\_entity, find\_entities | T0 | Query by label, type, region |
| get\_view\_context | T0 | Camera pose, viewport frame, selection, hovered entity |
| resolve\_reference, resolve\_direction | T0 | Candidates with confidence for a phrase |
| measure, list\_violations, required\_parameters | T0 | Distances, constraint and clearance status, missing inputs |
| propose\_edit, discard\_proposal | T1 | Section 8.3 |
| commit\_proposal, undo, redo | T2 | Section 8.4 |
| apply\_operator | T1 | Wraps ApplyOperator in a proposal |
| export\_dxf, export\_scene\_package, push\_to\_ue | T2 | File output |
| search\_assets, fetch\_asset | T3 | Section 10, models |
| search\_materials, fetch\_material | T3 | Section 10, texture sets |
| request\_purchase | T4 | Section 10.4, models and materials |

Mesh and material generation (10.6) are started from the app and have no agent tool.

**9.2 Ambiguity protocol.** An ambiguity is {kind, candidates\[\], default if any}. Kinds: Referent (which object), Direction, Magnitude, ReferenceSemantics (which face or center), PolicyChoice. The agent MUST NOT commit a proposal containing unresolved Referent, Direction or ReferenceSemantics ambiguities; it MUST ask the user. PolicyChoice ambiguities may be committed only as Proposed values after user review. Approximate quantities (about, roughly, a bit, a couple) are Magnitude ambiguities: the agent MUST propose one specific value flagged Proposed and MUST NOT commit it silently.

**9.3 View context and directions.** Default frame for left, right, forward, back is the current 3D camera when a 3D view is active, otherwise the 2D screen axes. The camera right vector, projected horizontally, is snapped to the nearest world axis. If the ratio of its minor to major component exceeds 0.7, the direction is ambiguous. Compass terms are used only if the user uses them. 'Along the wall' without a target is ambiguous (two senses). 'Toward X' resolves relative to the named entity.

**9.4 Echo rule.** Before commit, the System renders the resolved command in canonical plain language, for example: 'Move L-003 4 in toward the east along W-007 (east is your right in the current view).' The user's confirmation applies to that text. By default every commit requires explicit user confirmation in the app (or at the CLI prompt): commit\_proposal from an MCP or built-in agent session returns pending\_confirmation until the user confirms. A setting auto\_commit\_unambiguous, off by default, MAY allow commits of proposals that have no ambiguities, no Proposed values and no consequence moves.

**9.5 Backends.** (a) The MCP server (C3), stdio by default; an HTTP listener, if added, binds to 127.0.0.1 with a bearer token. (b) The built-in agent (C4) behind an AgentBackend interface with provider adapters (first adapter: OpenAI, D-17; more adapters after v1). API keys are stored in Windows Credential Manager, never in the project file. The System ships no billing. It MUST disclose that scene summaries are sent to the chosen provider.

**9.6 Security.** All tool inputs are validated against JSON schemas. No tool executes shell commands or arbitrary code. File paths resolve inside the project directory. Content fetched from the web is untrusted data: it is never executed, and any tool call whose justification originates in fetched content and which escalates tier requires explicit user confirmation. T3 uses an allowlist plus per-domain user approval. The System never holds payment credentials. Strings stored in the project that originated outside it (asset and material names and descriptions, source page titles, imported notes) are also untrusted: tool outputs mark them as untrusted data, and they never appear in tool descriptions or system instructions.

**9.7 Evaluation.** A versioned evaluation set of at least 100 natural-language requests with expected resolved commands or expected clarifying questions. v1 acceptance: at least 95% of cases correct in a majority of runs, at least 85% correct in every run, and zero commits with unresolved Referent or Direction ambiguities in any run. Results are recorded per provider and per pinned model identifier; changing either re-runs the set. Every case runs at least five times, and the report states the fraction of cases whose runs all produced identical resolved commands. The set is split into a development half used for tuning and a held-out half used for acceptance, and it includes adversarial cases: ambiguous referents, approximate quantities, mirrored views, and instructions embedded in asset and material names.

**9.8 Context budget.** Every tool result is bounded (default 8 KB, paginated) and uses a compact canonical form: labels, types and dimensions, with no coordinates the task does not need. get\_scene\_summary returns counts and a region index, not every entity; find\_entities supports region, type, label and nearest-to-selection queries. A 500-element project MUST be workable within a 32K-token agent context.

## 10. Asset acquisition

An Asset is one of two kinds. A model is a mesh that an ElementType references (model\_ref). A texture set is the image maps a Material references (texture\_set\_ref): base color (sRGB), normal (linear, OpenGL convention as in glTF), roughness, metallic and ambient occlusion (linear), and optionally height (linear). Both kinds follow the same rules: one fidelity scale, an authoritative declared dimension, one pipeline, one license and spend model, sandboxed converters and optional generators. Each subsection below states its rule for both kinds.

**10.1 Fidelity tiers.** Exact (manufacturer CAD; for a material, a manufacturer-supplied texture set with a stated physical size), Library (licensed model or material), Generated (approximate mesh or texture set from a generator), Proxy (parametric box or template; for a material, parametric base color, roughness and metallic values with no textures). Fidelity is stored and shown in the UI and exports; Generated assets are visibly marked. Every element type can fall back to its Proxy box and every material to its Proxy values, so every occurrence and material slot always resolves.

**10.2 Dimension authority.** For a model, declared\_dimensions are the declared dimension; for a material, tile\_size is: the physical width and height of one texture repeat. Declared dimensions come from the user or a cited manufacturer spec; a tile size comes from the user, a cited manufacturer spec, or the physical size a library publishes for the material, which ranks as measured in D-11. Neither is inferred by the agent: a missing value is a Proposed value under 9.2 and is confirmed by the user before commit. A mesh is normalized so its bounding box matches declared dimensions; a texture set is normalized so one repeat spans the tile size. Uniform scale is applied freely. Non-uniform scale (for a texture set, a pixel aspect different from the tile aspect) up to 2% is permitted and logged; beyond 2% the asset is rejected or downgraded to Proxy. A material downgraded to Proxy takes the mean of its base color map as its base color.

**10.3 Pipeline.** Discover, user review, acquire, validate, normalize, record provenance, add to library. Validation for a model: format, size limit, triangle budget default 100,000, units, pivot. Validation for a texture set: format (PNG or JPEG), size limit, resolution budget default 4096 px per side (maps are resized to a power of two), one pixel size shared by all maps, color space by map kind, normal-map convention (DirectX maps are converted to OpenGL and the conversion is logged), and a seam check on opposite edges whose failure is a warning the user may accept. Sources, for both kinds: user-provided files (for models STEP, DXF blocks and glTF; for materials texture files, including a library the user already owns, imported from a local folder), licensed libraries (search lists CC0 sources before paid ones; paid sources are sites on the T3 allowlist), manufacturer downloads, and photos plus specs processed by a generator (10.6; local open-weights models by default, a hosted service as an optional adapter; D-18).

**10.4 Licensing and spend.** Each asset records a license class: PublicDomain/CC0, Attribution, Commercial (purchased), Restricted, Unknown. Unknown is blocked unless the user overrides with a recorded note. Commercial assets bought outside the System, such as an existing purchased material library, are imported with class Commercial and a recorded note of where they were bought; the System does not verify the purchase. Any purchase requires user approval showing price, seller and license, and is limited by a per-project cap; checkout is performed by the user, not the System. Each asset also records whether its license permits including its files in a delivered Scene Package. When it does not, or this is unknown, export uses the Proxy form of the element type or material and lists it in the Proposed-items list with the reason. Fixtures and golden files use only CC0 or System-generated assets, because the source is published (D-19). Generating meshes or materials from manufacturer photos has copyright implications; the System records source URLs and warns that use is the user's responsibility.

**10.5 Converters.** STEP to mesh through an OpenCascade-based CLI; mesh cleanup through headless Blender; texture conversion (decoding, resizing, normal-map convention, seam check) through a converter process with caps on pixel count and decoded size. All are discovered through configuration with pinned versions, invoked with fixed argument templates (no shell interpolation), and their outputs are re-validated by C1. Converters process untrusted files, so each run uses a restricted working directory, no network access, and time and memory limits (a Windows job object or equivalent); a crash or timeout is reported as a failed import.

**10.6 Generators.** Generation is an optional external process (C6) that may run on the developer's machine or through a hosted adapter, never a runtime dependency of the System. A hosted adapter sends its input to the provider, which is a T3 network call with the data-egress disclosure of 9.5, and any charge is T4. Generators redraw the source image, so output has fidelity Generated, takes its declared dimension from 10.2, carries a visible mark, and records the model id and version, the seed and the hash of its input as provenance.

*Mesh generators.* Input: a photo plus declared dimensions. Local candidates, with licenses as their repositories listed them in November 2026 (verify before adoption): TRELLIS (MIT), TripoSR (MIT), Stable Fast 3D (Stability AI Community License), Hunyuan3D-2 (Tencent Community License, territorially restricted). Quality and hardware needs vary widely; one repository reports TRELLIS.2 running on NVIDIA under Linux only. A mid-range client GPU (Q-7) may not run the higher-fidelity models. Spike S-6 selects the default mesh generator.

*Material generators.* Input, in order of preference: a photo of a real sample plus one known dimension of it, which sets tile\_size; or a text prompt, with tile\_size stated by the user (a default is a Proposed value). Candidates are chosen and their licenses verified before adoption; a model whose license restricts commercial use of its output is excluded, because output is delivered to a client. Spike S-8 selects the default material generator.

## 11. Exports and the Scene Package

**11.1 DXF.** Layers per entity class; DIMENSION entities for Driving constraints; labeled equipment footprints. The exported plan MUST open in AutoCAD 2018 without repair prompts and overlay on the client's own drawings with exact registration. The writer emits a DXF version that AutoCAD 2018 reads (R2013 or older; the exact version is fixed by testing in S-7), sets the INSUNITS header variable to 1 (inches), LUNITS to 4 (architectural) and LUPREC to 4 (1/16 in), and writes coordinates in decimal inches converted from stored nanometers with at least six decimal places. Variable values are verified in S-7. DIMENSION entities need rendering geometry that AutoCAD 2018 accepts; if the writer cannot produce it reliably (S-7), the System falls back to dimension lines, extension lines and text on a DIMENSIONS layer.

**11.2 IFC (v1.x).** IFC4 with IfcWall and layer set usage, openings, doors, windows, spaces, furniture proxies, property sets, GlobalIds from D-12. Acceptance: opens without errors in a free viewer.

**11.3 Scene Package.** A folder or zip containing scene.json, assets/\*.glb, assets/textures/\*, and manifest.sha256. scene.json declares schema\_version, unit (integer nanometers), coordinate system, storeys, elements with ids, labels, transforms, footprints (used for collision), material slots, materials (id, name, fidelity, tile size in nanometers, and either Proxy values or texture paths by map kind), variant slots and options, lights (type, lumens, color temperature, optional IES reference) and a Proposed-items list. Each mesh is authored in element-local coordinates and placed by transform to preserve float precision. Model and texture files are named by content hash and copied byte for byte from the project library, subject to the packaging permission of 10.4. The package is deterministic: same document yields identical bytes.

## 12. Unreal Engine runtime

**12.1 Ingest.** The C7 plugin loads the Scene Package. Epic documents Datasmith Runtime as supported on Windows and macOS, but forum reports describe packaged-build import crashes and lost collision in some 5.x versions. Therefore the ingest path (Datasmith Runtime, glTF runtime plugin, or a plugin that builds UE mesh components from raw vertex and index buffers in the package; the plugin MUST NOT re-derive geometry from profiles or parameters, so that C1 remains the only geometry implementation) is decided by spike S-2 against explicit criteria. Regardless of path, collision is built by the plugin from scene.json footprints, not from imported meshes.

**12.2 Authoring versus runtime.** Python and Remote Control scripting are editor-only. Per D-15 the target is a packaged UE 5.8 build, so all runtime behavior MUST be C++ or Blueprint; editor Python is allowed only in developer tooling such as automated test scenes. Packaging is needed when the app changes, not when a project changes: project data is loaded at runtime from the Scene Package, and S-2 verifies this. Editor play mode is the fallback if S-2 fails. MegaLights, which Epic lists as production-ready in 5.8, SHOULD be evaluated in S-2 for scenes with many light fixtures.

**12.3 Interaction.** Every entity with a variant slot is tagged with its id. The player looks at an entity and presses the interact key (default E) to cycle its options (materials, lighting presets, pre-imported model variants). A model variant in a slot MUST share the owner's declared footprint within the tolerance in 10.2. A swap to a different footprint is not a variant but a SetType command in the document followed by clearance checks, and it is not offered in the UE interaction. Model variants beyond those present in the package require regeneration. Selections are written to selections.json and imported by C1 as SetVariantSelection commands so the document stays authoritative.

**12.4 Rendering rules.** Materials use world-aligned mapping with tile size in real-world units taken from scene.json. Lights from scene.json are physical (lumens, Kelvin, optional IES). Lighting presets are variant slots. Provisional state is shown by an on-screen list of Proposed items, which also lists Generated models and materials and anything exported in its Proxy form because of packaging permission (10.4).

**12.5 Updates.** v1 uses full reload. v1.x adds an authenticated local channel (127.0.0.1) that applies diffs by stable id. Target reload time and limits are set in S-2.

**12.6 Cameras and capture.** The document stores named camera bookmarks (position, heading, eye height; the default eye height is a Proposed value) and a default walk start. The runtime provides first-person walking with collision, bookmark selection and a screenshot key. Offline cinematic video and stills through Movie Render Queue are assumed to be editor-only (verify, section 17) and are out of scope for v1; real-time capture of the packaged build is the v1 output.

## 13. Verification and quality

**13.1 User-facing.** Point-to-point measurement in 2D and 3D; dimension overlays; a numeric diff report after every commit; user-defined assertions (for example, aisle width at least 36 in) evaluated after every commit; DXF round trip for overlay in the client's CAD.

**13.2 Automated.** Property tests: undo restores a byte-identical document, and for move and set-value commands the inverse delta does too; solver results are independent of insertion order; serialization round-trips. Golden tests: DXF, glTF and Scene Package hashes. Conformance fixtures: the L-room (5.4). Agent evaluation: 9.7. CI builds native and wasm32.

## 14. Phases and spikes

| Spike | Question | Pass criteria |
| --- | --- | --- |
| S-1 | Does Layer A solve realistic shop plans? | The first use case modeled with explained DOF and conflict reports, results order-independent |
| S-2 | Which UE ingest path works on the client machine? | 500 elements load in a packaged build (UE 5.8) with correct collision, lights and variant swap; full reload within a measured, accepted time; changing only project data requires no repackaging |
| S-3 | Are coordinates and handedness correct end to end? | L-room fiducials match in DXF, glTF and UE |
| S-4 | Can existing equipment be brought in with correct scale? | STEP to glTF via converter within 0.5% of declared dimensions |
| S-5 | Is the agent reliable on resolution? | 9.7 thresholds met on the first 100 cases |
| S-6 | Which local mesh generator runs acceptably on the client machine class? | A chosen model produces a usable mesh from a manufacturer photo within 0.5% of declared dimensions after normalization, in an accepted time, on 16-32 GB RAM and a mid-range GPU |
| S-7 | Does the DXF open correctly in AutoCAD 2018? | No repair prompt; units read as inches at 1/16 in precision; a known wall dimension reads exactly; the plan overlays on a client drawing with correct registration |
| S-8 | Which local material generator runs acceptably on the client machine class? | A chosen model produces a texture set from a photo of a real surface with one known dimension that passes the seam check and has a tile size equal to that dimension after normalization, in an accepted time, on 16-32 GB RAM and a mid-range GPU |

- **Phase 1.** C1, C2, shed roof template, DXF and glTF export, command log, evaluation harness. Exit: S-1, S-3 and S-7 pass for DXF and glTF.
- **Phase 2.** C5 viewer and 2D editor, proposal review, measurement tools. Exit: first use case modeled by a non-expert.
- **Phase 3.** C3, C4 and the tool set. Exit: S-5.
- **Phase 4.** C7 and the Scene Package in UE. Exit: S-2 and S-3 in UE.
- **Phase 5.** Asset pipeline for models and materials, operators, IFC export. Phase 6 (v2): IFC import, Layer B, roofs.

**Vertical slice first.** The first deliverable, inside Phase 1, is a thin end-to-end run of the client's shop: constraints entered from the paper plan, walls, openings, slab, shed roof, equipment as parametric boxes, a dimensioned DXF opened in AutoCAD 2018, and the glTF imported by hand into a UE 5.8 editor scene and walked. This retires the client's core need and the coordinate and unit risks (S-3, S-7) before the viewer, agent and plugin exist. S-1 runs on the client's real plan, and S-2 starts in parallel as a time-boxed experiment.

**Agent sequencing.** Phase 3 builds the MCP server first and the built-in agent loop second, so the tool set is exercised through an existing harness before the in-app loop is written.

## 15. Risks

| Risk | Mitigation |
| --- | --- |
| Existing building is not square or the client's measurements conflict | Calibrated underlay and transcribed dimensions from the paper plan (6.6), a field verification list, conflict protocol 7.5 |
| UE runtime import fragility | Spike S-2; editor-mode fallback; plugin-built collision |
| Asset accuracy and licensing | Declared dimensions and tile sizes authoritative; fidelity tiers; license classes; packaging permission with Proxy fallback at export |
| Agent misreads references or directions | Ambiguity protocol, echo rule, evaluation set |
| Scope growth ('any scenario') | Section 1.4 limits; Layer B deferred |
| LLM provider outage or cost | P7: full manual path; bring-your-own key |
| UE version drift | Pinned version; plugin tested per upgrade |
| S-2 fails: runtime ingest does not work in packaged UE 5.8 | Fallback chain: plugin mesh builder; automated repackaging per project on the developer machine; editor play on the developer machine. Each step is slower and keeps the owner in the loop, so S-2 starts early. |
| Solver cannot express the client's real plan (out of square, ordering, slopes) | 7.7, 7.8; pivot trigger below |
| Agent consistency below target | Repeat-run evaluation (9.7); every commit confirmed by the user (9.4); manual path always available (P7) |
| Scope exceeds one developer | Vertical slice first; defer IFC, hosted generators and operators beyond the v1 list; MCP before the built-in agent |
| Hostile input through assets and web content | Untrusted-data rules (9.6), sandboxed converters (10.5), T3 and T4 approvals |

**Pivot triggers.** (1) S-1 fails on the client's real plan with Layer A: bring Layer B forward, or adopt a mature solver behind the D-6 trait (D-23). (2) S-2 fails for every ingest path: adopt the repackaging fallback and reconsider the promise that the client never touches UE. (3) S-5 stays below target after two tuning rounds: agent proposals always require UI confirmation and the agent is limited to the tools that passed. (4) S-6 (meshes) or S-8 (materials) finds no generator usable on the client GPU: use a hosted adapter, or no Generated assets of that kind in v1.

## 16. Resolved questions

| Question | Owner answer | Result |
| --- | --- | --- |
| Q-1 UE version and run mode | Target 5.8; prefers a packaged build if compile times are not prohibitive | D-15, 12.2; S-2 verifies project data loads without repackaging |
| Q-2 UI technology | egui | D-14 |
| Q-3 Units | Always imperial, 1/16 in precision | D-13, 5.2, 5.3, Q-11 |
| Q-4 DXF and existing CAD | AutoCAD 2018; no existing CAD, but has a paper floor plan | D-16, D-22, 6.6, 11.1 |
| Q-5 Roof type | Shed roof | D-21, 6.5, Q-12 |
| Q-6 Mesh generators | Prefers local open weights (unfamiliar with the options) with a hosted option | D-18, 10.6, S-6 |
| Q-7 Client hardware | 16-32 GB RAM, mid-range GPU | Limits local generators (10.6); scene budgets set in S-2 |
| Q-8 First agent provider | OpenAI | D-17, 9.5 |
| Q-9 Distribution terms | GPL | D-19, Q-13 |
| Q-10 Existing-conditions model | Editable by the client in the same well-structured way | D-20, 6.7 |
| Q-11 1/16 in grid | As recommended | User-stated inputs on the grid; derived values may be finer (5.3) |
| Q-12 Shed roof definition | Meets an existing wall and slopes away from it; unsure how it is defined | Lean-to template with ledger reference; remaining parameters asked at time of use (6.5) |
| Q-13 GPL version | GPLv3; will deliver a working product to the client's machine and publish the source | D-19 |
| Q-14 Delivery | Whichever is best practice; probably an installer | Windows installer; packaging method chosen in Phase 4 |
| Q-15 Client GPU | NVIDIA | S-6 evaluates NVIDIA models on Windows, including WSL2 for models that are Linux-only |
| Q-16 Material sourcing | Handle materials like 3D assets: find them in libraries or generate them with a model. Generate from a photo of a real sample first, a text prompt second. CC0 libraries first, then paid trusted sites; an existing purchased material library is linked later | D-18, section 10 (texture sets), S-8 |

## 17. Items to verify

No owner questions remain open. These are claims in this document that were written from memory or from secondary sources and MUST be verified before the dependent work starts.

- Compatibility of the MIT-licensed UE plugin with the current Epic EULA (D-19).
- Datasmith Runtime and glTF runtime behavior in UE 5.8 packaged builds (S-2).
- Current licenses and Windows hardware requirements of the local mesh generators (10.6, S-6).
- Candidates, licenses (including output-use terms) and Windows hardware requirements of local material generators (10.6, S-8).
- Terms, APIs and published physical sizes of the CC0 and paid material libraries considered (10.3).
- Runtime loading of PNG and JPEG textures in UE 5.8 packaged builds (11.3, 12.4).
- DXF version and header variable values accepted by AutoCAD 2018 (11.1, S-7).
- IFC4 entity mappings and free-viewer acceptance (11.2).

* Whether Movie Render Queue works in packaged builds in UE 5.8 (12.6).
* Current capabilities of FreeCAD and Bonsai as the adopt alternative (D-23).

## 18. Review log

Adversarial review of draft 0.2. Each finding is resolved in the section named.

| # | Finding | Severity | Resolved in |
| --- | --- | --- | --- |
| 1 | Position edits were undefined under constraints: which dimension changes and what moves as a consequence | Critical | 8.7 |
| 2 | The equality solver cannot see ordering, so a move could carry a wall through another | Critical | 7.7 |
| 3 | Adjusting furniture to fit had no deterministic implementation, contradicting P2 | Critical | 8.8, P2 |
| 4 | Commit authority was unclear when an external agent harness drives the project | High | 9.4 |
| 5 | No single-writer rule for UI, CLI and MCP clients | High | 3.1 |
| 6 | Variant swaps could silently change the layout | High | 12.3 |
| 7 | Plugin-built geometry would duplicate the core's geometry logic | High | 12.1 |
| 8 | Out-of-square existing buildings conflict with an orthogonal model | High | 7.8 |
| 9 | First client value arrived only after Phase 4 | High | 14 |
| 10 | Agent evaluation measured accuracy but not consistency and had no held-out set | High | 9.7 |
| 11 | Prompt injection through project-stored strings; unbounded tool output | Medium | 9.6, 9.8 |
| 12 | Fixed obstructions (columns, beams, panels) were missing from the model | Medium | 6.3 |
| 13 | Converters parse untrusted files without isolation | Medium | 10.5 |
| 14 | DXF DIMENSION entities may not render without block geometry | Medium | 11.1 |
| 15 | No camera or capture definition; offline video is not available in packaged builds | Medium | 12.6 |
| 16 | Floating point and timestamps could break byte-identical reproduction | Medium | 5.5 |
| 17 | Licensing of the UE plugin was unclear | Medium | D-19 |
| 18 | No build-versus-adopt record and no pivot triggers | Medium | D-23, 15 |
