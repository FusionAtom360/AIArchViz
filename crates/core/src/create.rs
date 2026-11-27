//! Commands that create entities (T-120b, Q-66, DEC-3).
//!
//! CreateWall, AddConstraint and PlaceOccurrence are in the 8.2 set; the
//! other seven fill the gap DEC-3 records. Every optional value a command
//! leaves null takes the catalog default with source Proposed (6.4).
//! [`fill_defaults`] writes those values into the command, so a proposal's
//! resolved commands are explicit (8.3) and `apply` of the filled command
//! gives the same document as `apply` of the original.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::command::CommandError;
use crate::defaults::{
    CEILING_THICKNESS, CLEARANCE, EAVE_OVERHANG, FASCIA_DEPTH, MEASURED_TOLERANCE, ROOF_THICKNESS,
    SLAB_THICKNESS, WALL_BASE_OFFSET, interior_wall, proposed,
};
use crate::document::{
    Boundary, Ceiling, Clearance, Constraint, ConstraintKind, ConstraintStatus, Datum,
    DatumGeometry, DeclaredDimensions, Direction, Document, ElementType, Fidelity, Finishes,
    Function, Layer, LayerSet, Light, Material, Occurrence, Overhang, Pitch, Placement, Point,
    Properties, Provenance, Reference, Roof, Slab, Source, Sourced, Space, Wall, WallTop,
};
use crate::identity::Uuid;
use crate::length::Length;
use crate::solver::{LayerA, Solver};

/// Creates a wall between two existing nodes (6.1, Q-66).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWall {
    pub storey: Uuid,
    pub node_a: Uuid,
    pub node_b: Uuid,
    /// From node_a to node_b (Q-12).
    pub direction: Direction,
    pub function: Function,
    /// Left to right; null takes the catalog interior stud wall.
    #[serde(default)]
    pub layers: Option<Vec<Layer>>,
    /// Required with `layers`; null with the catalog wall takes its value.
    #[serde(default)]
    pub left_extent: Option<Sourced<Length>>,
    #[serde(default)]
    pub base_offset: Option<Sourced<Length>>,
    /// Null is the storey's floor-to-floor height.
    #[serde(default)]
    pub top: Option<WallTop>,
    /// Null is 0; ties go to the lowest wall id.
    #[serde(default)]
    pub join_priority: Option<u32>,
}

/// Adds a constraint (7.1, Q-66).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddConstraint {
    pub kind: ConstraintKind,
    /// Null is driving.
    #[serde(default)]
    pub status: Option<ConstraintStatus>,
    pub source: Source,
    #[serde(default)]
    pub provenance: Option<Provenance>,
}

/// Places an occurrence of an element type (6.3, Q-66).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceOccurrence {
    pub storey: Uuid,
    pub element_type: Uuid,
    pub placement: Placement,
    /// Null takes the catalog clearance on every side.
    #[serde(default)]
    pub clearance: Option<Clearance>,
    #[serde(default)]
    pub fixed: bool,
    #[serde(default)]
    pub overlap_allowed: bool,
    #[serde(default)]
    pub light: Option<Light>,
}

/// Creates a datum (DEC-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDatum {
    pub name: String,
    pub geometry: DatumGeometry,
}

/// Creates a material (DEC-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateMaterial {
    pub name: String,
    #[serde(default)]
    pub tile_size: Option<Sourced<Length>>,
    #[serde(default)]
    pub properties: Properties,
}

/// Creates a space; its boundary is derived at commit (Q-24, DEC-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSpace {
    pub storey: Uuid,
    pub name: String,
    pub seed: Point,
    #[serde(default)]
    pub finishes: Option<Finishes>,
}

/// Creates a floor slab (DEC-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSlab {
    pub storey: Uuid,
    pub boundary: Boundary,
    #[serde(default)]
    pub thickness: Option<Sourced<Length>>,
    /// Null is 0: the slab top at the storey floor.
    #[serde(default)]
    pub elevation: Option<Sourced<Length>>,
}

/// Creates a ceiling (DEC-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCeiling {
    pub storey: Uuid,
    pub boundary: Boundary,
    #[serde(default)]
    pub thickness: Option<Sourced<Length>>,
    /// Null is the storey's default ceiling height.
    #[serde(default)]
    pub elevation: Option<Sourced<Length>>,
}

/// Creates a shed roof (6.5, DEC-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateRoof {
    pub storey: Uuid,
    pub footprint: Boundary,
    /// Required without a ledger; derived with one, so then null.
    #[serde(default)]
    pub slope_direction: Option<Direction>,
    pub high_edge_height: Sourced<Length>,
    #[serde(default)]
    pub low_edge_height: Option<Sourced<Length>>,
    #[serde(default)]
    pub pitch: Option<Sourced<Pitch>>,
    #[serde(default)]
    pub ledger: Option<Reference>,
    #[serde(default)]
    pub overhang: Option<Overhang>,
    #[serde(default)]
    pub thickness: Option<Sourced<Length>>,
    #[serde(default)]
    pub fascia_depth: Option<Sourced<Length>>,
    #[serde(default)]
    pub material_slots: BTreeMap<String, Uuid>,
}

/// Creates an element type without a model (DEC-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateElementType {
    pub name: String,
    pub declared_dimensions: DeclaredDimensions,
    /// Null is proxy.
    #[serde(default)]
    pub fidelity: Option<Fidelity>,
    #[serde(default)]
    pub properties: Properties,
    pub source: String,
    pub license: String,
}

/// The catalog materials of the interior stud wall: ids derived from the
/// project id, so every wall that uses the catalog shares them (Q-66).
pub fn catalog_materials(project: Uuid) -> [(Uuid, &'static str); 2] {
    [
        (
            Uuid::derive(project, "catalog", "gypsum board"),
            "Gypsum board",
        ),
        (Uuid::derive(project, "catalog", "wood stud"), "Wood stud"),
    ]
}

fn invalid(text: &str) -> CommandError {
    CommandError::InvalidArguments(text.to_string())
}

fn fill<T: Clone>(slot: &mut Option<T>, value: T) {
    if slot.is_none() {
        *slot = Some(value);
    }
}

/// Writes every catalog default into `wall` (Q-66).
pub fn fill_wall(doc: &Document, wall: &mut CreateWall) -> Result<(), CommandError> {
    match (&wall.layers, &wall.left_extent) {
        (None, _) => {
            let [(gypsum, _), (stud, _)] = catalog_materials(doc.project.id);
            let (layers, left) = interior_wall(gypsum, stud);
            wall.layers = Some(layers);
            fill(&mut wall.left_extent, left);
        }
        (Some(_), None) => return Err(invalid("left_extent is required with layers")),
        (Some(_), Some(_)) => {}
    }
    fill(&mut wall.base_offset, proposed(WALL_BASE_OFFSET));
    fill(&mut wall.top, WallTop::Storey);
    fill(&mut wall.join_priority, 0);
    Ok(())
}

/// Writes the default status and, for a Measured Distance, the 7.5 tolerance.
pub fn fill_constraint(c: &mut AddConstraint) -> Result<(), CommandError> {
    fill(&mut c.status, ConstraintStatus::Driving);
    if let ConstraintKind::Distance { tolerance, .. } = &mut c.kind {
        match (c.source, tolerance.is_some()) {
            (Source::Measured, _) => fill(tolerance, MEASURED_TOLERANCE),
            (_, true) => return Err(invalid("a tolerance needs source measured")),
            (_, false) => {}
        }
    }
    Ok(())
}

pub fn fill_occurrence(o: &mut PlaceOccurrence) {
    fill(
        &mut o.clearance,
        Clearance {
            front: proposed(CLEARANCE),
            back: proposed(CLEARANCE),
            left: proposed(CLEARANCE),
            right: proposed(CLEARANCE),
        },
    );
}

pub fn fill_slab(s: &mut CreateSlab) {
    fill(&mut s.thickness, proposed(SLAB_THICKNESS));
    fill(&mut s.elevation, proposed(Length::from_nanometers(0)));
}

pub fn fill_ceiling(doc: &Document, c: &mut CreateCeiling) -> Result<(), CommandError> {
    fill(&mut c.thickness, proposed(CEILING_THICKNESS));
    if c.elevation.is_none() {
        let storey = doc
            .storeys
            .get(&c.storey)
            .ok_or(CommandError::UnknownEntity(c.storey))?;
        c.elevation = Some(proposed(storey.default_ceiling_height.value));
    }
    Ok(())
}

pub fn fill_roof(r: &mut CreateRoof) -> Result<(), CommandError> {
    match (&r.slope_direction, &r.ledger) {
        (None, None) => return Err(invalid("a roof needs a slope direction or a ledger")),
        (Some(_), Some(_)) => {
            return Err(invalid(
                "a ledger derives the slope direction; give one, not both",
            ));
        }
        _ => {}
    }
    fill(
        &mut r.overhang,
        Overhang {
            pos_x: proposed(EAVE_OVERHANG),
            neg_x: proposed(EAVE_OVERHANG),
            pos_y: proposed(EAVE_OVERHANG),
            neg_y: proposed(EAVE_OVERHANG),
        },
    );
    fill(&mut r.thickness, proposed(ROOF_THICKNESS));
    fill(&mut r.fascia_depth, proposed(FASCIA_DEPTH));
    Ok(())
}

pub fn fill_element_type(t: &mut CreateElementType) {
    fill(&mut t.fidelity, Fidelity::Proxy);
}

fn need<T>(map: &BTreeMap<Uuid, T>, id: Uuid) -> Result<(), CommandError> {
    if map.contains_key(&id) {
        Ok(())
    } else {
        Err(CommandError::UnknownEntity(id))
    }
}

fn fresh<T>(map: &BTreeMap<Uuid, T>, id: Uuid) -> Result<Uuid, CommandError> {
    if map.contains_key(&id) {
        Err(CommandError::IdCollision(id))
    } else {
        Ok(id)
    }
}

fn reference_exists(doc: &Document, r: &Reference) -> Result<(), CommandError> {
    match *r {
        Reference::Node { node } => need(&doc.nodes, node),
        Reference::WallFace { wall, .. } | Reference::WallCenter { wall } => need(&doc.walls, wall),
        Reference::OpeningEdge { opening, .. } => need(&doc.openings, opening),
        Reference::ItemEdge { item, .. } | Reference::ItemCenter { item } => {
            need(&doc.occurrences, item)
        }
        Reference::Datum { datum } => need(&doc.datums, datum),
    }
}

fn boundary_exists(doc: &Document, b: &Boundary) -> Result<(), CommandError> {
    match b {
        Boundary::Explicit { .. } => Ok(()),
        Boundary::FromSpace { space } => need(&doc.spaces, *space),
    }
}

fn label(doc: &mut Document, prefix: &str) -> Result<crate::identity::Label, CommandError> {
    doc.labels
        .allocate(prefix)
        .map_err(|e| CommandError::InvalidArguments(e.to_string()))
}

/// Applies a filled CreateWall: ordinal 0 the wall, 1 its layer set.
pub fn create_wall(
    doc: &mut Document,
    args: &CreateWall,
    new_id: impl Fn(u32) -> Uuid,
) -> Result<(), CommandError> {
    let mut a = args.clone();
    fill_wall(doc, &mut a)?;
    need(&doc.storeys, a.storey)?;
    need(&doc.nodes, a.node_a)?;
    need(&doc.nodes, a.node_b)?;
    if a.node_a == a.node_b {
        return Err(invalid("node_a and node_b must differ"));
    }
    let (Some(layers), Some(left_extent), Some(base_offset), Some(top), Some(join_priority)) = (
        a.layers,
        a.left_extent,
        a.base_offset,
        a.top,
        a.join_priority,
    ) else {
        return Err(invalid("wall defaults were not filled"));
    };
    if layers.is_empty() {
        return Err(invalid("a wall needs at least one layer"));
    }
    let catalog = catalog_materials(doc.project.id);
    for layer in &layers {
        if doc.materials.contains_key(&layer.material) {
            continue;
        }
        let (id, name) = catalog
            .iter()
            .find(|(id, _)| *id == layer.material)
            .ok_or(CommandError::UnknownEntity(layer.material))?;
        doc.materials.insert(
            *id,
            Material {
                name: (*name).to_string(),
                asset: None,
                tile_size: None,
                properties: Properties::new(),
            },
        );
    }
    let id = fresh(&doc.walls, new_id(0))?;
    let set = fresh(&doc.layer_sets, new_id(1))?;
    let label = label(doc, "W")?;
    doc.layer_sets.insert(
        set,
        LayerSet {
            name: format!("{label} layers"),
            layers,
        },
    );
    doc.walls.insert(
        id,
        Wall {
            label,
            storey: a.storey,
            node_a: a.node_a,
            node_b: a.node_b,
            direction: a.direction,
            layer_set: set,
            left_extent,
            base_offset,
            top,
            join_priority,
            function: a.function,
        },
    );
    Ok(())
}

/// Applies a filled AddConstraint.
pub fn add_constraint(
    doc: &mut Document,
    args: &AddConstraint,
    id: Uuid,
) -> Result<(), CommandError> {
    let mut a = args.clone();
    fill_constraint(&mut a)?;
    match &a.kind {
        ConstraintKind::Distance { a: ra, b: rb, .. }
        | ConstraintKind::Align { a: ra, b: rb, .. } => {
            reference_exists(doc, ra)?;
            reference_exists(doc, rb)?;
        }
        ConstraintKind::Fixed { reference, .. } => reference_exists(doc, reference)?,
        ConstraintKind::Equal { first, second } => {
            for c in [first, second] {
                match doc.constraints.get(c).map(|c| &c.kind) {
                    Some(ConstraintKind::Distance { .. }) => {}
                    Some(_) => return Err(invalid("Equal must name two Distance constraints")),
                    None => return Err(CommandError::UnknownEntity(*c)),
                }
            }
        }
        ConstraintKind::MinClearance { object, other, .. } => {
            need(&doc.occurrences, *object)?;
            if !doc.occurrences.contains_key(other) && !doc.walls.contains_key(other) {
                return Err(CommandError::UnknownEntity(*other));
            }
        }
    }
    let id = fresh(&doc.constraints, id)?;
    doc.constraints.insert(
        id,
        Constraint {
            kind: a.kind,
            status: a.status.unwrap_or(ConstraintStatus::Driving),
            source: a.source,
            provenance: a.provenance,
        },
    );
    Ok(())
}

/// Applies a filled PlaceOccurrence and stores its solved center (Q-31).
pub fn place_occurrence(
    doc: &mut Document,
    args: &PlaceOccurrence,
    id: Uuid,
) -> Result<(), CommandError> {
    let mut a = args.clone();
    fill_occurrence(&mut a);
    need(&doc.storeys, a.storey)?;
    need(&doc.element_types, a.element_type)?;
    let initial = match &a.placement {
        Placement::Floor { x, y, .. } | Placement::CeilingMounted { x, y } => Point {
            x: x.value,
            y: y.value,
        },
        Placement::WallMounted {
            wall, reference, ..
        } => {
            need(&doc.walls, *wall)?;
            reference_exists(doc, reference)?;
            Point {
                x: Length::from_nanometers(0),
                y: Length::from_nanometers(0),
            }
        }
    };
    let id = fresh(&doc.occurrences, id)?;
    let label = label(doc, "EQ")?;
    let Some(clearance) = a.clearance else {
        return Err(invalid("clearance default was not filled"));
    };
    doc.occurrences.insert(
        id,
        Occurrence {
            label,
            storey: a.storey,
            element_type: a.element_type,
            placement: a.placement,
            clearance,
            fixed: a.fixed,
            overlap_allowed: a.overlap_allowed,
            light: a.light,
            center: initial,
        },
    );
    let solution = LayerA::default().solve(doc).map_err(CommandError::Solve)?;
    if let (Some(center), Some(o)) = (solution.occurrences.get(&id), doc.occurrences.get_mut(&id)) {
        o.center = *center;
    }
    Ok(())
}

pub fn create_datum(doc: &mut Document, args: &CreateDatum, id: Uuid) -> Result<(), CommandError> {
    let id = fresh(&doc.datums, id)?;
    doc.datums.insert(
        id,
        Datum {
            name: args.name.clone(),
            geometry: args.geometry,
        },
    );
    Ok(())
}

pub fn create_material(
    doc: &mut Document,
    args: &CreateMaterial,
    id: Uuid,
) -> Result<(), CommandError> {
    let id = fresh(&doc.materials, id)?;
    doc.materials.insert(
        id,
        Material {
            name: args.name.clone(),
            asset: None,
            tile_size: args.tile_size.clone(),
            properties: args.properties.clone(),
        },
    );
    Ok(())
}

pub fn create_space(doc: &mut Document, args: &CreateSpace, id: Uuid) -> Result<(), CommandError> {
    need(&doc.storeys, args.storey)?;
    let finishes = args.finishes.clone().unwrap_or(Finishes {
        floor: None,
        walls: None,
        ceiling: None,
    });
    for m in [finishes.floor, finishes.walls, finishes.ceiling]
        .into_iter()
        .flatten()
    {
        need(&doc.materials, m)?;
    }
    let id = fresh(&doc.spaces, id)?;
    doc.spaces.insert(
        id,
        Space {
            storey: args.storey,
            name: args.name.clone(),
            seed: args.seed,
            boundary: Vec::new(),
            finishes,
        },
    );
    Ok(())
}

pub fn create_slab(doc: &mut Document, args: &CreateSlab, id: Uuid) -> Result<(), CommandError> {
    let mut a = args.clone();
    fill_slab(&mut a);
    need(&doc.storeys, a.storey)?;
    boundary_exists(doc, &a.boundary)?;
    let (Some(thickness), Some(elevation)) = (a.thickness, a.elevation) else {
        return Err(invalid("slab defaults were not filled"));
    };
    let id = fresh(&doc.slabs, id)?;
    doc.slabs.insert(
        id,
        Slab {
            storey: a.storey,
            boundary: a.boundary,
            thickness,
            elevation,
        },
    );
    Ok(())
}

pub fn create_ceiling(
    doc: &mut Document,
    args: &CreateCeiling,
    id: Uuid,
) -> Result<(), CommandError> {
    let mut a = args.clone();
    fill_ceiling(doc, &mut a)?;
    boundary_exists(doc, &a.boundary)?;
    let (Some(thickness), Some(elevation)) = (a.thickness, a.elevation) else {
        return Err(invalid("ceiling defaults were not filled"));
    };
    let id = fresh(&doc.ceilings, id)?;
    doc.ceilings.insert(
        id,
        Ceiling {
            storey: a.storey,
            boundary: a.boundary,
            thickness,
            elevation,
        },
    );
    Ok(())
}

pub fn create_roof(doc: &mut Document, args: &CreateRoof, id: Uuid) -> Result<(), CommandError> {
    let mut a = args.clone();
    fill_roof(&mut a)?;
    need(&doc.storeys, a.storey)?;
    boundary_exists(doc, &a.footprint)?;
    for m in a.material_slots.values() {
        need(&doc.materials, *m)?;
    }
    let slope_direction = match (a.slope_direction, &a.ledger) {
        (Some(d), _) => Sourced {
            value: d,
            source: Source::UserStated,
        },
        (None, Some(ledger)) => {
            reference_exists(doc, ledger)?;
            // Replaced from the ledger face at commit (derive_roofs, Q-45).
            Sourced {
                value: Direction::PosX,
                source: Source::Derived,
            }
        }
        (None, None) => return Err(invalid("a roof needs a slope direction or a ledger")),
    };
    let (Some(overhang), Some(thickness), Some(fascia_depth)) =
        (a.overhang, a.thickness, a.fascia_depth)
    else {
        return Err(invalid("roof defaults were not filled"));
    };
    let id = fresh(&doc.roofs, id)?;
    doc.roofs.insert(
        id,
        Roof {
            storey: a.storey,
            footprint: a.footprint,
            slope_direction,
            high_edge_height: a.high_edge_height,
            low_edge_height: a.low_edge_height,
            pitch: a.pitch,
            ledger: a.ledger,
            overhang,
            thickness,
            fascia_depth,
            material_slots: a.material_slots,
        },
    );
    Ok(())
}

pub fn create_element_type(
    doc: &mut Document,
    args: &CreateElementType,
    id: Uuid,
) -> Result<(), CommandError> {
    let d = &args.declared_dimensions;
    if [d.width, d.depth, d.height]
        .iter()
        .any(|l| l.nanometers() <= 0)
    {
        return Err(invalid("declared dimensions must be positive"));
    }
    let id = fresh(&doc.element_types, id)?;
    doc.element_types.insert(
        id,
        ElementType {
            name: args.name.clone(),
            declared_dimensions: args.declared_dimensions.clone(),
            fidelity: args.fidelity.unwrap_or(Fidelity::Proxy),
            model: None,
            properties: args.properties.clone(),
            source: args.source.clone(),
            license: args.license.clone(),
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::tests::{cmd, genesis, set_storey};
    use crate::command::{Action, Command, CreateNode, CreateOpening, apply};
    use crate::document::{Authority, Axis, OpeningEdge, OpeningKind, QuarterTurns, WallSide};
    use crate::log::History;
    use crate::proposal::{ProposedAction, derive, propose};

    const IN: i64 = 25_400_000;

    fn len(inches: i64) -> Length {
        Length::from_nanometers(inches * IN)
    }

    fn user(inches: i64) -> Sourced<Length> {
        Sourced {
            value: len(inches),
            source: Source::UserStated,
        }
    }

    fn command(index: usize, action: Action) -> Command {
        cmd(u8::try_from(index + 1).unwrap_or(u8::MAX), action)
    }

    /// The id of the first entity the command at `index` creates.
    fn id(index: usize) -> Uuid {
        let key = command(
            index,
            Action::CreateNode(CreateNode {
                x: len(0),
                y: len(0),
            }),
        )
        .idempotency_key;
        Uuid::derive(key, "command", "0")
    }

    fn build(actions: Vec<Action>) -> Result<Document, String> {
        let mut doc = genesis();
        for (i, action) in actions.into_iter().enumerate() {
            doc = apply(&doc, &command(i, action)).map_err(|e| format!("command {i}: {e}"))?;
        }
        Ok(doc)
    }

    fn node(x: i64, y: i64) -> Action {
        Action::CreateNode(CreateNode {
            x: len(x),
            y: len(y),
        })
    }

    fn wall(a: usize, b: usize, direction: Direction) -> Action {
        Action::CreateWall(CreateWall {
            storey: id(STOREY),
            node_a: id(a),
            node_b: id(b),
            direction,
            function: Function::Existing,
            layers: None,
            left_extent: None,
            base_offset: None,
            top: None,
            join_priority: None,
        })
    }

    fn at(i: usize) -> Reference {
        Reference::Node { node: id(i) }
    }

    fn distance(a: Reference, b: Reference, axis: Axis, value: Length, source: Source) -> Action {
        Action::AddConstraint(AddConstraint {
            kind: ConstraintKind::Distance {
                a,
                b,
                axis,
                value,
                tolerance: None,
            },
            status: None,
            source,
            provenance: None,
        })
    }

    // Command indexes in `room`.
    const STOREY: usize = 0;
    const DATUM: usize = 1;
    const SW: usize = 2;
    const SE: usize = 3;
    const NE: usize = 4;
    const NW: usize = 5;
    const SOUTH: usize = 6;
    const FIRST_DIMENSION: usize = 10;
    const MEASURED: usize = 14;
    const SPACE: usize = 16;
    const TYPE: usize = 19;
    const ITEM: usize = 20;

    /// A 10 ft x 8 ft room (wall centerlines) built only from commands:
    /// storey, datum, four catalog walls, dimensions (one redundant and
    /// Measured), a door, a space, a slab from it, a shed roof, a box type
    /// and an occurrence of it. Node positions start at a deliberately
    /// wrong 100 x 100 in, so the dimensions must move them.
    fn room() -> Vec<Action> {
        let datum = Reference::Datum { datum: id(DATUM) };
        let storey = id(STOREY);
        vec![
            set_storey(None, "Ground"),
            Action::CreateDatum(CreateDatum {
                name: "SW".into(),
                geometry: DatumGeometry::Point {
                    x: len(0),
                    y: len(0),
                },
            }),
            node(0, 0),
            node(100, 0),
            node(100, 100),
            node(0, 100),
            wall(SW, SE, Direction::PosX),
            wall(SE, NE, Direction::PosY),
            wall(NE, NW, Direction::NegX),
            wall(NW, SW, Direction::NegY),
            distance(datum, at(SW), Axis::X, len(0), Source::UserStated),
            distance(datum, at(SW), Axis::Y, len(0), Source::UserStated),
            distance(at(SW), at(SE), Axis::X, len(120), Source::UserStated),
            distance(at(SW), at(NW), Axis::Y, len(96), Source::UserStated),
            // Redundant and 1/8 in off: within the 1/4 in default.
            distance(
                at(SW),
                at(SE),
                Axis::X,
                Length::from_nanometers(120 * IN + IN / 8),
                Source::Measured,
            ),
            Action::CreateOpening(CreateOpening {
                kind: OpeningKind::Door,
                host: id(SOUTH),
                reference: at(SW),
                anchor: OpeningEdge::Start,
                offset: user(24),
                width: None,
                height: None,
                sill: None,
            }),
            Action::CreateSpace(CreateSpace {
                storey,
                name: "Room".into(),
                seed: Point {
                    x: len(60),
                    y: len(48),
                },
                finishes: None,
            }),
            Action::CreateSlab(CreateSlab {
                storey,
                boundary: Boundary::FromSpace { space: id(SPACE) },
                thickness: None,
                elevation: None,
            }),
            Action::CreateRoof(CreateRoof {
                storey,
                footprint: Boundary::FromSpace { space: id(SPACE) },
                slope_direction: Some(Direction::NegY),
                high_edge_height: user(120),
                low_edge_height: Some(user(108)),
                pitch: None,
                ledger: None,
                overhang: None,
                thickness: None,
                fascia_depth: None,
                material_slots: BTreeMap::new(),
            }),
            Action::CreateElementType(CreateElementType {
                name: "Bench".into(),
                declared_dimensions: DeclaredDimensions {
                    width: len(24),
                    depth: len(24),
                    height: len(30),
                    authority: Authority::UserStatedExact,
                },
                fidelity: None,
                properties: Properties::new(),
                source: "parametric box".into(),
                license: "n/a".into(),
            }),
            Action::PlaceOccurrence(PlaceOccurrence {
                storey,
                element_type: id(TYPE),
                placement: Placement::Floor {
                    x: user(60),
                    y: user(30),
                    rotation: QuarterTurns::new(0).unwrap_or_else(|| unreachable!()),
                    elevation: user(0),
                },
                clearance: None,
                fixed: false,
                overlap_allowed: false,
                light: None,
            }),
        ]
    }

    fn room_commands() -> Vec<Command> {
        room()
            .into_iter()
            .enumerate()
            .map(|(i, a)| command(i, a))
            .collect()
    }

    #[test]
    fn a_room_built_from_commands_solves_cleanly() -> Result<(), String> {
        let history = History::new(genesis());
        let p = propose(
            &history,
            &ProposedAction::Commands {
                commands: room_commands(),
            },
        )
        .map_err(|e| e.to_string())?;
        assert!(p.conflicts.is_empty(), "{:?}", p.conflicts);
        assert!(p.violations.is_empty(), "{:?}", p.violations);
        assert!(!p.blocked());
        let derived = derive(build(room())?).map_err(|e| format!("{e:?}"))?;
        let d = &derived.document;
        assert_eq!(d.walls.len(), 4);
        assert_eq!(
            d.nodes.get(&id(NE)).map(|n| (n.x, n.y)),
            Some((len(120), len(96)))
        );
        // The redundant Measured dimension is now a Reference (7.5).
        assert_eq!(
            derived
                .solution
                .constraints
                .get(&id(MEASURED))
                .map(|r| (r.outcome, r.residual)),
            Some((
                crate::solver::Outcome::Demoted,
                Some(Length::from_nanometers(-IN / 8))
            ))
        );
        assert_eq!(d.spaces.get(&id(SPACE)).map(|s| s.boundary.len()), Some(4));
        assert_eq!(
            d.occurrences.get(&id(ITEM)).map(|o| o.center),
            Some(Point {
                x: len(60),
                y: len(30)
            })
        );
        assert!(derived.solution.conflicts.is_empty());
        Ok(())
    }

    #[test]
    fn resolved_commands_carry_every_default() -> Result<(), String> {
        let p = propose(
            &History::new(genesis()),
            &ProposedAction::Commands {
                commands: room_commands(),
            },
        )
        .map_err(|e| e.to_string())?;
        let ProposedAction::Commands { commands } = &p.resolved else {
            return Err("not commands".into());
        };
        for c in commands {
            let explicit = match &c.action {
                Action::CreateWall(w) => {
                    w.layers.is_some()
                        && w.left_extent.is_some()
                        && w.base_offset.is_some()
                        && w.top.is_some()
                        && w.join_priority.is_some()
                }
                Action::AddConstraint(a) => {
                    a.status.is_some()
                        && match (&a.kind, a.source) {
                            (ConstraintKind::Distance { tolerance, .. }, Source::Measured) => {
                                *tolerance == Some(MEASURED_TOLERANCE)
                            }
                            _ => true,
                        }
                }
                Action::PlaceOccurrence(o) => o.clearance.is_some(),
                Action::CreateSlab(s) => s.thickness.is_some() && s.elevation.is_some(),
                Action::CreateRoof(r) => {
                    r.overhang.is_some() && r.thickness.is_some() && r.fascia_depth.is_some()
                }
                Action::CreateElementType(t) => t.fidelity.is_some(),
                _ => true,
            };
            assert!(explicit, "{c:?}");
        }
        // The resolved commands build the same document as the originals.
        let mut resolved = genesis();
        for c in commands {
            resolved = apply(&resolved, c).map_err(|e| e.to_string())?;
        }
        assert_eq!(resolved, build(room())?);
        Ok(())
    }

    #[test]
    fn catalog_defaults_are_proposed_and_shared() -> Result<(), String> {
        let doc = build(room())?;
        assert_eq!(
            doc.materials.len(),
            2,
            "four catalog walls share two materials"
        );
        let w = doc.walls.get(&id(SOUTH)).ok_or("no wall")?;
        assert_eq!(w.label.to_string(), "W-001");
        assert_eq!(w.left_extent, proposed(Length::from_nanometers(9 * IN / 4)));
        assert_eq!(w.base_offset, proposed(WALL_BASE_OFFSET));
        assert_eq!((w.top.clone(), w.join_priority), (WallTop::Storey, 0));
        let set = doc.layer_sets.get(&w.layer_set).ok_or("no layer set")?;
        assert_eq!(set.layers.len(), 3);
        assert!(
            set.layers
                .iter()
                .all(|l| doc.materials.contains_key(&l.material))
        );
        let o = doc.occurrences.get(&id(ITEM)).ok_or("no item")?;
        assert_eq!(o.label.to_string(), "EQ-001");
        assert_eq!(o.clearance.front, proposed(CLEARANCE));
        let s = doc.slabs.values().next().ok_or("no slab")?;
        assert_eq!(
            (s.thickness.clone(), s.elevation.clone()),
            (proposed(SLAB_THICKNESS), proposed(len(0)))
        );
        let r = doc.roofs.values().next().ok_or("no roof")?;
        assert_eq!(
            (
                r.thickness.clone(),
                r.fascia_depth.clone(),
                r.overhang.pos_x.clone()
            ),
            (
                proposed(ROOF_THICKNESS),
                proposed(FASCIA_DEPTH),
                proposed(EAVE_OVERHANG)
            )
        );
        let t = doc.element_types.get(&id(TYPE)).ok_or("no type")?;
        assert_eq!((t.fidelity, t.model), (Fidelity::Proxy, None));
        let m = doc.constraints.get(&id(MEASURED)).ok_or("no constraint")?;
        assert!(matches!(
            m.kind,
            ConstraintKind::Distance {
                tolerance: Some(MEASURED_TOLERANCE),
                ..
            }
        ));
        assert_eq!(MEASURED_TOLERANCE, Length::from_nanometers(IN / 4));
        Ok(())
    }

    #[test]
    fn ceiling_elevation_defaults_to_the_storey_ceiling() -> Result<(), String> {
        let doc = build(vec![set_storey(None, "Ground")])?;
        let storey_height = doc
            .storeys
            .get(&id(STOREY))
            .map(|s| s.default_ceiling_height.value);
        let c = command(
            1,
            Action::CreateCeiling(CreateCeiling {
                storey: id(STOREY),
                boundary: Boundary::Explicit { points: vec![] },
                thickness: None,
                elevation: None,
            }),
        );
        let after = apply(&doc, &c).map_err(|e| e.to_string())?;
        let ceiling = after.ceilings.get(&id(1)).ok_or("no ceiling")?;
        assert_eq!(Some(ceiling.elevation.value), storey_height);
        assert_eq!(ceiling.elevation.source, Source::Proposed);
        assert_eq!(ceiling.thickness, proposed(CEILING_THICKNESS));
        let ghost = Uuid::from_bytes([0xEE; 16]);
        let unknown = command(
            2,
            Action::CreateCeiling(CreateCeiling {
                storey: ghost,
                boundary: Boundary::Explicit { points: vec![] },
                thickness: None,
                elevation: None,
            }),
        );
        assert_eq!(
            apply(&doc, &unknown),
            Err(CommandError::UnknownEntity(ghost))
        );
        Ok(())
    }

    fn lean_to(ledger: Option<Reference>, slope: Option<Direction>) -> Action {
        let p = |x, y| Point {
            x: len(x),
            y: len(y),
        };
        Action::CreateRoof(CreateRoof {
            storey: id(STOREY),
            footprint: Boundary::Explicit {
                points: vec![p(0, -60), p(120, -60), p(120, -2), p(0, -2)],
            },
            slope_direction: slope,
            high_edge_height: user(108),
            low_edge_height: Some(user(96)),
            pitch: None,
            ledger,
            overhang: None,
            thickness: None,
            fascia_depth: None,
            material_slots: BTreeMap::new(),
        })
    }

    #[test]
    fn a_ledger_roof_takes_its_slope_from_the_wall() -> Result<(), String> {
        let mut actions = room();
        actions.truncate(TYPE);
        let doc = build(actions)?;
        let ledger = Some(Reference::WallFace {
            wall: id(SOUTH),
            side: WallSide::Right,
            boundary: 0,
        });
        let c = command(40, lean_to(ledger, None));
        let after = apply(&doc, &c).map_err(|e| e.to_string())?;
        let d = derive(after).map_err(|e| format!("{e:?}"))?.document;
        let r = d.roofs.get(&id(40)).ok_or("no roof")?;
        assert_eq!(
            (r.slope_direction.value, r.slope_direction.source),
            (Direction::NegY, Source::Derived)
        );
        assert!(matches!(
            apply(&doc, &command(41, lean_to(ledger, Some(Direction::NegY)))),
            Err(CommandError::InvalidArguments(_))
        ));
        assert!(matches!(
            apply(&doc, &command(42, lean_to(None, None))),
            Err(CommandError::InvalidArguments(_))
        ));
        Ok(())
    }

    #[test]
    fn bad_arguments_are_errors_not_panics() -> Result<(), String> {
        let doc = build(room())?;
        let ghost = Uuid::from_bytes([0xEE; 16]);
        let attempt = |i: usize, a: Action| apply(&doc, &command(100 + i, a));
        let mut w = CreateWall {
            storey: id(STOREY),
            node_a: id(SW),
            node_b: ghost,
            direction: Direction::PosX,
            function: Function::New,
            layers: None,
            left_extent: None,
            base_offset: None,
            top: None,
            join_priority: None,
        };
        assert_eq!(
            attempt(1, Action::CreateWall(w.clone())),
            Err(CommandError::UnknownEntity(ghost))
        );
        w.node_b = id(SW);
        assert!(matches!(
            attempt(2, Action::CreateWall(w.clone())),
            Err(CommandError::InvalidArguments(_))
        ));
        w.node_b = id(NE);
        w.layers = Some(vec![Layer {
            material: ghost,
            thickness: user(4),
            core: true,
        }]);
        assert!(matches!(
            attempt(3, Action::CreateWall(w.clone())),
            Err(CommandError::InvalidArguments(_))
        ));
        w.left_extent = Some(user(2));
        assert_eq!(
            attempt(4, Action::CreateWall(w.clone())),
            Err(CommandError::UnknownEntity(ghost))
        );
        w.layers = Some(Vec::new());
        assert!(matches!(
            attempt(5, Action::CreateWall(w)),
            Err(CommandError::InvalidArguments(_))
        ));
        let mut stated = AddConstraint {
            kind: ConstraintKind::Distance {
                a: at(SW),
                b: at(SE),
                axis: Axis::X,
                value: len(120),
                tolerance: Some(len(1)),
            },
            status: None,
            source: Source::UserStated,
            provenance: None,
        };
        assert!(matches!(
            attempt(6, Action::AddConstraint(stated.clone())),
            Err(CommandError::InvalidArguments(_))
        ));
        stated.kind = ConstraintKind::Equal {
            first: id(MEASURED),
            second: ghost,
        };
        assert_eq!(
            attempt(7, Action::AddConstraint(stated.clone())),
            Err(CommandError::UnknownEntity(ghost))
        );
        let align = command(
            8,
            Action::AddConstraint(AddConstraint {
                kind: ConstraintKind::Align {
                    a: at(SW),
                    b: at(NW),
                    axis: Axis::X,
                },
                status: None,
                source: Source::UserStated,
                provenance: None,
            }),
        );
        let with_align = apply(&doc, &align).map_err(|e| e.to_string())?;
        stated.kind = ConstraintKind::Equal {
            first: id(MEASURED),
            second: Uuid::derive(align.idempotency_key, "command", "0"),
        };
        assert!(matches!(
            apply(&with_align, &command(9, Action::AddConstraint(stated))),
            Err(CommandError::InvalidArguments(_))
        ));
        assert_eq!(
            attempt(
                10,
                distance(
                    Reference::Datum { datum: ghost },
                    at(SW),
                    Axis::X,
                    len(1),
                    Source::UserStated
                )
            ),
            Err(CommandError::UnknownEntity(ghost))
        );
        let item = PlaceOccurrence {
            storey: id(STOREY),
            element_type: ghost,
            placement: Placement::CeilingMounted {
                x: user(1),
                y: user(1),
            },
            clearance: None,
            fixed: false,
            overlap_allowed: false,
            light: None,
        };
        assert_eq!(
            attempt(11, Action::PlaceOccurrence(item)),
            Err(CommandError::UnknownEntity(ghost))
        );
        let flat = CreateElementType {
            name: "Flat".into(),
            declared_dimensions: DeclaredDimensions {
                width: len(1),
                depth: len(0),
                height: len(1),
                authority: Authority::UserStatedExact,
            },
            fidelity: None,
            properties: Properties::new(),
            source: String::new(),
            license: String::new(),
        };
        assert!(matches!(
            attempt(12, Action::CreateElementType(flat)),
            Err(CommandError::InvalidArguments(_))
        ));
        assert_eq!(
            attempt(
                13,
                Action::CreateSlab(CreateSlab {
                    storey: id(STOREY),
                    boundary: Boundary::FromSpace { space: ghost },
                    thickness: None,
                    elevation: None,
                })
            ),
            Err(CommandError::UnknownEntity(ghost))
        );
        assert_eq!(
            attempt(
                14,
                Action::CreateSpace(CreateSpace {
                    storey: ghost,
                    name: "Nowhere".into(),
                    seed: Point {
                        x: len(1),
                        y: len(1)
                    },
                    finishes: None,
                })
            ),
            Err(CommandError::UnknownEntity(ghost))
        );
        Ok(())
    }

    #[test]
    fn every_creation_collides_with_its_own_earlier_result() -> Result<(), String> {
        let doc = build(room())?;
        // Index 0 is SetStorey, which updates rather than creates.
        for (i, c) in room_commands().into_iter().enumerate().skip(1) {
            assert!(
                matches!(apply(&doc, &c), Err(CommandError::IdCollision(_))),
                "command {i} applied twice"
            );
        }
        Ok(())
    }

    #[test]
    fn new_commands_round_trip_on_the_wire() -> Result<(), String> {
        for c in room_commands() {
            let text = crate::canonical::to_canonical_json(&c).map_err(|e| e.to_string())?;
            let back: Command = serde_json::from_str(&text).map_err(|e| format!("{text}: {e}"))?;
            assert_eq!(back, c);
        }
        // Optional arguments may be left out entirely.
        let text = format!(
            r#"{{"id":"{}","idempotency_key":"{}","kind":"create_slab","args":{{"storey":"{}","boundary":{{"kind":"from_space","space":"{}"}}}}}}"#,
            id(30),
            id(31),
            id(STOREY),
            id(SPACE)
        );
        let parsed: Command = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        assert!(matches!(
            parsed.action,
            Action::CreateSlab(CreateSlab {
                thickness: None,
                elevation: None,
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn constraint_order_does_not_change_the_solution() -> Result<(), String> {
        let solved = |actions: Vec<Action>| -> Result<Vec<(Length, Length)>, String> {
            let d = derive(build(actions)?).map_err(|e| format!("{e:?}"))?;
            Ok(d.document.nodes.values().map(|n| (n.x, n.y)).collect())
        };
        let mut reversed = room();
        reversed[FIRST_DIMENSION..=MEASURED].reverse();
        assert_eq!(solved(room())?, solved(reversed)?);
        Ok(())
    }
}
