//! The conformance fixtures (T-121) as scripts of CLI commands.
//!
//! Each fixture is written to `fixtures/<name>/commands.txt` by
//! `cargo xtask golden`: one line per `archviz` invocation, `<subcommand>
//! <request JSON>`, starting with `new`. Every later line is an `apply` of
//! one command. Ids are fixed: each command's idempotency key is derived
//! from the project id and the command's number, and an entity's id from
//! that key (8.1), so later commands can name earlier entities.
//!
//! Every length below is a G-2 value (or its plain sum, such as 216 + 64),
//! a catalog default left null, or an equipment size or position that the
//! owner left to Claude (Q-64), which is marked Proposed. The solver places
//! every wall; node positions given to `create_node` are starting guesses only.

use std::collections::BTreeMap;

use archviz_core::{
    Action, AddConstraint, Authority, Axis, Boundary, Command, ConstraintKind, CreateCeiling,
    CreateDatum, CreateElementType, CreateMaterial, CreateNode, CreateOpening, CreateRoof,
    CreateSlab, CreateSpace, CreateWall, DatumGeometry, DeclaredDimensions, Direction, Function,
    Layer, Length, OpeningEdge, OpeningKind, Overhang, PROJECT_NAMESPACE, Pitch, PlaceOccurrence,
    Placement, Point, Properties, QuarterTurns, Reference, SetStorey, Source, Sourced, Uuid,
    WallSide, WallTop, to_canonical_json,
};

const IN: i64 = 25_400_000;

fn len(inches: i64) -> Length {
    Length::from_nanometers(inches * IN)
}

fn stated(inches: i64) -> Sourced<Length> {
    Sourced {
        value: len(inches),
        source: Source::UserStated,
    }
}

fn proposed(inches: i64) -> Sourced<Length> {
    Sourced {
        value: len(inches),
        source: Source::Proposed,
    }
}

fn pt(x: i64, y: i64) -> Point {
    Point {
        x: len(x),
        y: len(y),
    }
}

/// Collects the script while handing out the ids its commands will create.
struct Script {
    project: Uuid,
    lines: Vec<String>,
    count: u32,
}

impl Script {
    /// The project id is derived from the fixture's name.
    fn new(name: &str) -> Result<Self, String> {
        let id = Uuid::derive(PROJECT_NAMESPACE, "fixture", name);
        let line = format!(
            r#"new {{"building":"Main","building_id":"{}","name":"{name}","project_id":"{id}"}}"#,
            Uuid::derive(id, "fixture", "building")
        );
        Ok(Script {
            project: id,
            lines: vec![line],
            count: 0,
        })
    }

    /// Applies one command; returns the id of its first new entity.
    fn apply(&mut self, action: Action) -> Result<Uuid, String> {
        self.count += 1;
        let n = self.count.to_string();
        let command = Command {
            id: Uuid::derive(self.project, "fixture-command", &n),
            idempotency_key: Uuid::derive(self.project, "fixture-key", &n),
            action,
        };
        let text = to_canonical_json(&command).map_err(|e| e.to_string())?;
        self.lines.push(format!(
            r#"apply {{"action":{{"commands":[{text}],"kind":"commands"}},"actor":"fixture","timestamp_ms":{}}}"#,
            self.count
        ));
        Ok(Uuid::derive(command.idempotency_key, "command", "0"))
    }

    fn text(&self) -> String {
        let mut out = self.lines.join("\n");
        out.push('\n');
        out
    }

    fn storey(&mut self, ceiling: Sourced<Length>) -> Result<Uuid, String> {
        self.apply(Action::SetStorey(SetStorey {
            storey: None,
            name: "Ground".into(),
            elevation: stated(0),
            floor_to_floor: stated(120),
            default_ceiling_height: ceiling,
        }))
    }

    fn node(&mut self, x: i64, y: i64) -> Result<Uuid, String> {
        self.apply(Action::CreateNode(CreateNode {
            x: len(x),
            y: len(y),
        }))
    }

    fn datum(&mut self, name: &str, x: i64, y: i64) -> Result<Uuid, String> {
        self.apply(Action::CreateDatum(CreateDatum {
            name: name.into(),
            geometry: DatumGeometry::Point {
                x: len(x),
                y: len(y),
            },
        }))
    }

    fn material(&mut self, name: &str) -> Result<Uuid, String> {
        self.apply(Action::CreateMaterial(CreateMaterial {
            name: name.into(),
            tile_size: None,
            properties: Properties::new(),
        }))
    }

    fn distance(
        &mut self,
        a: Reference,
        b: Reference,
        axis: Axis,
        inches: i64,
    ) -> Result<Uuid, String> {
        self.apply(Action::AddConstraint(AddConstraint {
            kind: ConstraintKind::Distance {
                a,
                b,
                axis,
                value: len(inches),
                tolerance: None,
            },
            status: None,
            source: Source::UserStated,
            provenance: None,
        }))
    }

    fn space(&mut self, storey: Uuid, name: &str, seed: Point) -> Result<Uuid, String> {
        self.apply(Action::CreateSpace(CreateSpace {
            storey,
            name: name.into(),
            seed,
            finishes: None,
        }))
    }

    fn slab(
        &mut self,
        storey: Uuid,
        space: Uuid,
        thickness: Option<Sourced<Length>>,
    ) -> Result<Uuid, String> {
        self.apply(Action::CreateSlab(CreateSlab {
            storey,
            boundary: Boundary::FromSpace { space },
            thickness,
            elevation: None,
        }))
    }

    fn box_type(
        &mut self,
        name: &str,
        size: (i64, i64, i64),
        authority: Authority,
    ) -> Result<Uuid, String> {
        let source = match authority {
            Authority::UserStatedExact => "G-2",
            _ => "proposed size (Q-64)",
        };
        self.apply(Action::CreateElementType(CreateElementType {
            name: name.into(),
            declared_dimensions: DeclaredDimensions {
                width: len(size.0),
                depth: len(size.1),
                height: len(size.2),
                authority,
            },
            fidelity: None,
            properties: Properties::new(),
            source: source.into(),
            license: "n/a".into(),
        }))
    }

    fn place(
        &mut self,
        storey: Uuid,
        element_type: Uuid,
        center: (i64, i64),
        turns: u8,
        source: Source,
        elevation: i64,
    ) -> Result<Uuid, String> {
        let at = |inches| Sourced {
            value: len(inches),
            source,
        };
        self.apply(Action::PlaceOccurrence(PlaceOccurrence {
            storey,
            element_type,
            placement: Placement::Floor {
                x: at(center.0),
                y: at(center.1),
                rotation: QuarterTurns::new(turns).ok_or("quarter turns")?,
                elevation: at(elevation),
            },
            clearance: None,
            fixed: false,
            overlap_allowed: false,
            light: None,
        }))
    }
}

/// A wall of one core layer.
struct WallSpec {
    storey: Uuid,
    layers: Vec<Layer>,
    left_extent: Sourced<Length>,
    top: Option<WallTop>,
}

impl WallSpec {
    fn create(
        &self,
        s: &mut Script,
        a: Uuid,
        b: Uuid,
        direction: Direction,
        function: Function,
        join_priority: u32,
    ) -> Result<Uuid, String> {
        s.apply(Action::CreateWall(CreateWall {
            storey: self.storey,
            node_a: a,
            node_b: b,
            direction,
            function,
            layers: Some(self.layers.clone()),
            left_extent: Some(self.left_extent.clone()),
            base_offset: Some(stated(0)),
            top: self.top.clone(),
            join_priority: Some(join_priority),
        }))
    }
}

fn node_ref(node: Uuid) -> Reference {
    Reference::Node { node }
}

fn datum_ref(datum: Uuid) -> Reference {
    Reference::Datum { datum }
}

/// A wall's exterior (right) finish face; walls run counterclockwise.
fn outside(wall: Uuid) -> Reference {
    Reference::WallFace {
        wall,
        side: WallSide::Right,
        boundary: 0,
    }
}

fn inside(wall: Uuid) -> Reference {
    Reference::WallFace {
        wall,
        side: WallSide::Left,
        boundary: 0,
    }
}

struct OpeningSpec {
    kind: OpeningKind,
    host: Uuid,
    reference: Reference,
    anchor: OpeningEdge,
    offset: i64,
    width: i64,
    height: i64,
    sill: i64,
}

fn opening(s: &mut Script, o: OpeningSpec) -> Result<Uuid, String> {
    s.apply(Action::CreateOpening(CreateOpening {
        kind: o.kind,
        host: o.host,
        reference: o.reference,
        anchor: o.anchor,
        offset: stated(o.offset),
        width: Some(stated(o.width)),
        height: Some(stated(o.height)),
        sill: Some(stated(o.sill)),
    }))
}

/// The L-room (5.4, Q-54): the room of T-119's tests, rebuilt through the
/// CLI and fully dimensioned from fiducial F-A. Wall centerlines (0,0)
/// (240,0) (240,96) (96,96) (96,180) (0,180) in inches, so the arms
/// differ; 6 in walls; a door, a window and a plain opening; slab, ceiling
/// and a 2:12 shed roof; fiducials F-A to F-D; a marker box raised 36 in.
pub fn l_room() -> Result<String, String> {
    let mut s = Script::new("L-room")?;
    let storey = s.storey(stated(108))?;
    let fa = s.datum("F-A", 0, 0)?;
    s.datum("F-B", 240, 0)?;
    s.datum("F-C", 240, 96)?;
    s.datum("F-D", 0, 180)?;
    let core = s.material("6 in wall")?;
    let spec = WallSpec {
        storey,
        layers: vec![Layer {
            material: core,
            thickness: stated(6),
            core: true,
        }],
        left_extent: stated(3),
        top: None,
    };
    let corners = [(0, 0), (240, 0), (240, 96), (96, 96), (96, 180), (0, 180)];
    let mut n = Vec::new();
    for (x, y) in corners {
        n.push(s.node(x, y)?);
    }
    let directions = [
        Direction::PosX,
        Direction::PosY,
        Direction::NegX,
        Direction::PosY,
        Direction::NegX,
        Direction::NegY,
    ];
    let mut w = Vec::new();
    for (i, d) in directions.into_iter().enumerate() {
        w.push(spec.create(&mut s, n[i], n[(i + 1) % 6], d, Function::New, 0)?);
    }
    s.distance(datum_ref(fa), node_ref(n[0]), Axis::X, 0)?;
    s.distance(datum_ref(fa), node_ref(n[0]), Axis::Y, 0)?;
    s.distance(node_ref(n[0]), node_ref(n[1]), Axis::X, 240)?;
    s.distance(node_ref(n[0]), node_ref(n[3]), Axis::X, 96)?;
    s.distance(node_ref(n[0]), node_ref(n[2]), Axis::Y, 96)?;
    s.distance(node_ref(n[0]), node_ref(n[5]), Axis::Y, 180)?;
    for (kind, host, offset, width, height, sill) in [
        (OpeningKind::Door, 0usize, 48, 36, 80, 0),
        (OpeningKind::Window, 1, 24, 48, 36, 36),
        (OpeningKind::Opening, 5, 60, 30, 84, 0),
    ] {
        opening(
            &mut s,
            OpeningSpec {
                kind,
                host: w[host],
                reference: node_ref(n[host]),
                anchor: OpeningEdge::Start,
                offset,
                width,
                height,
                sill,
            },
        )?;
    }
    let space = s.space(storey, "Room", pt(48, 48))?;
    s.slab(storey, space, Some(stated(4)))?;
    s.apply(Action::CreateCeiling(CreateCeiling {
        storey,
        boundary: Boundary::FromSpace { space },
        thickness: Some(Sourced {
            value: Length::from_nanometers(IN / 2),
            source: Source::UserStated,
        }),
        elevation: Some(stated(108)),
    }))?;
    s.apply(Action::CreateRoof(CreateRoof {
        storey,
        footprint: Boundary::FromSpace { space },
        slope_direction: Some(Direction::NegY),
        high_edge_height: stated(132),
        low_edge_height: None,
        pitch: Some(Sourced {
            value: Pitch { rise: 2, run: 12 },
            source: Source::UserStated,
        }),
        ledger: None,
        overhang: Some(Overhang {
            pos_x: stated(12),
            neg_x: stated(12),
            pos_y: stated(12),
            neg_y: stated(12),
        }),
        thickness: Some(stated(6)),
        fascia_depth: Some(stated(6)),
        material_slots: BTreeMap::new(),
    }))?;
    let marker = s.box_type("Marker", (12, 6, 10), Authority::UserStatedExact)?;
    s.place(storey, marker, (200, 40), 0, Source::UserStated, 36)?;
    Ok(s.text())
}

/// The client's shop from G-2, as answered in Q-61 to Q-65.
///
/// World: X east, Y north, origin at the shop's south-west exterior corner
/// (datum "SW"). Exterior 216 in E-W by 432 in N-S, 4 in walls, 120 in
/// high. Walls run counterclockwise, so each wall's right face is its
/// exterior face; reference lines are the wall centerlines (left extent 2 in).
/// The east, west and south walls are split where other walls meet them
/// (T joins); exterior walls run through (priority 0), new walls butt.
pub fn shop() -> Result<String, String> {
    let mut s = Script::new("Shop")?;
    let storey = s.storey(proposed(108))?;
    let sw = s.datum("SW", 0, 0)?;
    let existing = s.material("Existing wall")?;
    let new = s.material("New wall")?;
    let spec = |material| WallSpec {
        storey,
        layers: vec![Layer {
            material,
            thickness: stated(4),
            core: true,
        }],
        left_extent: stated(2),
        top: None,
    };
    let (old, fresh) = (spec(existing), spec(new));
    // The extension's walls: tops at the roof's low edge (Q-63).
    let extension_walls = WallSpec {
        top: Some(WallTop::Height {
            height: proposed(96),
        }),
        ..spec(new)
    };

    // Starting guesses; the dimensions below place every node.
    let a = s.node(0, 0)?; // south-west
    let b = s.node(60, 0)?; // new wall 2 meets the south wall
    let c = s.node(200, 0)?; // south-east
    let d = s.node(200, 30)?; // extension south wall meets the east wall
    let e = s.node(200, 100)?; // new wall 1 meets the east wall
    let f = s.node(200, 400)?; // extension north wall meets the east wall
    let g = s.node(200, 420)?; // north-east
    let h = s.node(0, 420)?; // north-west
    let i = s.node(0, 100)?; // new wall 1 meets the west wall
    let j = s.node(60, 100)?; // new wall 2 meets new wall 1
    let k = s.node(260, 30)?; // extension south-east
    let l = s.node(260, 400)?; // extension north-east

    let ex = Function::Existing;
    let south_1 = old.create(&mut s, a, b, Direction::PosX, ex, 0)?;
    let south_2 = old.create(&mut s, b, c, Direction::PosX, ex, 0)?;
    let east_1 = old.create(&mut s, c, d, Direction::PosY, ex, 0)?;
    old.create(&mut s, d, e, Direction::PosY, ex, 0)?;
    let east_3 = old.create(&mut s, e, f, Direction::PosY, ex, 0)?;
    old.create(&mut s, f, g, Direction::PosY, ex, 0)?;
    let north = old.create(&mut s, g, h, Direction::NegX, ex, 0)?;
    let west_1 = old.create(&mut s, h, i, Direction::NegY, ex, 0)?;
    let west_2 = old.create(&mut s, i, a, Direction::NegY, ex, 0)?;
    let nw = Function::New;
    // New wall 1, west to east; its left face is its north face.
    let new_1 = fresh.create(&mut s, i, j, Direction::PosX, nw, 1)?;
    fresh.create(&mut s, j, e, Direction::PosX, nw, 1)?;
    // New wall 2, south to north; its right face is its east face.
    let new_2 = fresh.create(&mut s, b, j, Direction::PosY, nw, 2)?;
    // The extension, counterclockwise.
    let ext = &extension_walls;
    let ext_s = ext.create(&mut s, d, k, Direction::PosX, nw, 1)?;
    let ext_e = ext.create(&mut s, k, l, Direction::PosY, nw, 1)?;
    let ext_n = ext.create(&mut s, l, f, Direction::NegX, nw, 1)?;

    // The outline: 18 ft E-W by 36 ft N-S (Q-61), from the datum.
    s.distance(datum_ref(sw), outside(west_2), Axis::X, 0)?;
    s.distance(datum_ref(sw), outside(south_1), Axis::Y, 0)?;
    s.distance(outside(west_2), outside(east_1), Axis::X, 216)?;
    s.distance(outside(south_1), outside(north), Axis::Y, 432)?;
    // New wall 1: north face 312 in south of the north exterior face.
    s.distance(outside(north), inside(new_1), Axis::Y, -312)?;
    // New wall 2: east face 144 in west of the east exterior face.
    s.distance(outside(east_1), outside(new_2), Axis::X, -144)?;
    // The extension: 64 in out, 36 in in from the north and south faces.
    s.distance(outside(east_1), outside(ext_e), Axis::X, 64)?;
    s.distance(outside(north), outside(ext_n), Axis::Y, -36)?;
    s.distance(outside(south_1), outside(ext_s), Axis::Y, 36)?;

    // Openings (Q-62): offsets from the north wall's exterior face to the
    // opening's north edge; west-wall openings run south, so that edge is
    // their start. Centered openings are centered on the 216 in exterior.
    let from_north = outside(north);
    let west = [
        (OpeningKind::Door, west_1, 42, 36, 80, 0),
        (OpeningKind::Window, west_1, 138, 60, 45, 35),
        (OpeningKind::Door, west_2, 323, 32, 80, 0),
        (OpeningKind::Window, west_2, 377, 36, 45, 35),
    ];
    for (kind, host, offset, width, height, sill) in west {
        opening(
            &mut s,
            OpeningSpec {
                kind,
                host,
                reference: from_north,
                anchor: OpeningEdge::Start,
                offset,
                width,
                height,
                sill,
            },
        )?;
    }
    // Garage door: 12 ft wide, 109 in tall, centered: its center is half
    // of 216 in west of the east exterior face (the north wall runs west).
    opening(
        &mut s,
        OpeningSpec {
            kind: OpeningKind::Door,
            host: north,
            reference: outside(east_1),
            anchor: OpeningEdge::Center,
            offset: 108,
            width: 144,
            height: 109,
            sill: 0,
        },
    )?;
    // South window, centered: half of 216 in east of the west exterior face.
    opening(
        &mut s,
        OpeningSpec {
            kind: OpeningKind::Window,
            host: south_2,
            reference: outside(west_2),
            anchor: OpeningEdge::Center,
            offset: 108,
            width: 60,
            height: 45,
            sill: 35,
        },
    )?;
    // The extension door in the east wall (Q-63): its north edge is 103 in
    // south of the north exterior face; the east wall runs north, so that
    // edge is its end.
    opening(
        &mut s,
        OpeningSpec {
            kind: OpeningKind::Door,
            host: east_3,
            reference: from_north,
            anchor: OpeningEdge::End,
            offset: -103,
            width: 48,
            height: 96,
            sill: 0,
        },
    )?;

    // Rooms and floors.
    let shop = s.space(storey, "Shop", pt(108, 300))?;
    let south_west = s.space(storey, "South room west", pt(36, 60))?;
    let south_east = s.space(storey, "South room east", pt(140, 60))?;
    let extension = s.space(storey, "Extension", pt(248, 216))?;
    for space in [shop, south_west, south_east, extension] {
        s.slab(storey, space, None)?;
    }

    // The extension's shed roof (Q-63): over its outline, ledger on the
    // east wall's exterior face, 108 in there and 96 in at the outer side.
    s.apply(Action::CreateRoof(CreateRoof {
        storey,
        footprint: Boundary::Explicit {
            points: vec![pt(216, 36), pt(280, 36), pt(280, 396), pt(216, 396)],
        },
        slope_direction: None,
        high_edge_height: stated(108),
        low_edge_height: Some(stated(96)),
        pitch: None,
        ledger: Some(outside(east_3)),
        overhang: None,
        thickness: None,
        fascia_depth: None,
        material_slots: BTreeMap::new(),
    }))?;

    // Equipment (Q-64): G-2 sizes where given, otherwise proposed sizes;
    // positions chosen in the large room, proposed, clear of walls, doors
    // and each other. Sizes are X x Y x Z at rotation 0.
    let stated_size = Authority::UserStatedExact;
    let proposed_size = Authority::ProposedDefault;
    let table = s.box_type("Large table", (48, 82, 36), stated_size)?;
    let chest = s.box_type("Tool chest", (20, 42, 56), stated_size)?;
    let rack = s.box_type("Storage rack", (24, 80, 76), stated_size)?;
    let band_saw = s.box_type("Band saw", (30, 26, 68), proposed_size)?;
    let sander = s.box_type("Belt sander", (30, 20, 48), proposed_size)?;
    let grinder = s.box_type("Bench grinder", (20, 14, 44), proposed_size)?;
    let laser = s.box_type("CO2 laser", (56, 36, 42), proposed_size)?;
    let drill = s.box_type("Drill press", (24, 16, 66), proposed_size)?;
    let cabinet = s.box_type("File cabinet", (15, 28, 52), proposed_size)?;
    let p = Source::Proposed;
    for (element_type, center, turns) in [
        (rack, (199, 386), 0),
        (rack, (199, 236), 0),
        (rack, (130, 134), 1),
        (rack, (46, 134), 1),
        (table, (100, 250), 0),
        (chest, (16, 190), 0),
        (laser, (40, 320), 0),
        (band_saw, (130, 330), 0),
        (drill, (170, 330), 0),
        (sander, (60, 180), 0),
        (grinder, (160, 180), 0),
        (cabinet, (199, 160), 0),
    ] {
        s.place(storey, element_type, center, turns, p, 0)?;
    }
    Ok(s.text())
}

/// Every fixture: (folder name, commands.txt contents).
pub fn all() -> Result<Vec<(&'static str, String)>, String> {
    Ok(vec![("l-room", l_room()?), ("shop", shop()?)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_files_are_current() -> Result<(), String> {
        let root = crate::workspace_root();
        for (name, text) in all()? {
            let path = root.join("fixtures").join(name).join("commands.txt");
            let on_disk = std::fs::read_to_string(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .replace("\r\n", "\n");
            assert_eq!(on_disk, text, "{name}: run cargo xtask golden");
        }
        Ok(())
    }

    #[test]
    fn scripts_are_deterministic() -> Result<(), String> {
        assert_eq!(all()?, all()?);
        Ok(())
    }
}
