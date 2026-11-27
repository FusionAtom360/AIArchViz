//! The echo text of a proposal (spec 9.4; Q-76).
//!
//! Every command renders as one canonical plain-language sentence, built
//! only from labels, names, directions and lengths in feet-inches-fractions,
//! never from ids. Naming (Q-76): a label where one exists (W-002, EQ-012);
//! otherwise a name in quotes with its kind (datum "West"); otherwise a
//! description from labeled or named things (`the door in W-002 at 3'-6"
//! from W-001 left face`, `node at (10'-0", 4'-0")`). Directions are axis
//! terms (`toward +X`); view-relative wording is the app's (9.3). Values
//! with source Proposed are marked `(Proposed)`.
//!
//! A move reads as the move, then the dimensions it sets and what moves
//! with it, so the user sees both the intent and what is committed.

use std::collections::{BTreeMap, BTreeSet};

use crate::command::{Action, Command, DimensionTarget, Motion, SetConstraintValue};
use crate::create::{AddConstraint, CreateRoof, CreateWall, PlaceOccurrence};
use crate::document::{
    Axis, Boundary, ConstraintKind, ConstraintStatus, DatumGeometry, Direction, Document, Function,
    ItemSide, Layer, LightKind, OpeningEdge, OpeningKind, Placement, Point, Provenance, Reference,
    Source, Sourced, VariantKind, WallSide, WallTop,
};
use crate::edit::{MoveOutcome, displacement, request_for};
use crate::identity::Uuid;
use crate::length::Length;
use crate::units::{ProjectUnit, format_length};

/// Beyond this nesting a referenced entity is named only by its kind, so a
/// cycle of references (an Equal naming itself) cannot recurse forever.
const MAX_DEPTH: u8 = 3;

/// Looks names up in the document before a command, then in the one after
/// it (for entities the command creates).
#[derive(Clone, Copy)]
struct Names<'a> {
    before: &'a Document,
    after: &'a Document,
}

/// The echo of `command`, which turns `before` into `after` (Q-76).
pub(crate) fn render(before: &Document, after: &Document, command: &Command) -> String {
    Names { before, after }.command(command)
}

/// The echo of a move resolved on `current`: the move, the dimensions it
/// sets, and what moves with it (Q-76 (b)).
pub(crate) fn move_entry(current: &Document, command: &Command, outcome: &MoveOutcome) -> String {
    let n = Names {
        before: current,
        after: current,
    };
    let head = n.command(command);
    match outcome {
        MoveOutcome::Edits { edits, moved, .. } => {
            let mut parts: Vec<String> = edits
                .iter()
                .map(|e| format!("set {}", n.dimension_change(e)))
                .collect();
            let names: BTreeSet<String> = moved.iter().map(|id| n.entity(*id, 0)).collect();
            if !names.is_empty() {
                let verb = if names.len() == 1 { "moves" } else { "move" };
                parts.push(format!("{} {verb} with it", join(names)));
            }
            if parts.is_empty() {
                head
            } else {
                format!("{head}: {}", parts.join("; "))
            }
        }
        MoveOutcome::Ambiguous { .. } => {
            format!("{head}: which dimension changes is not chosen")
        }
        MoveOutcome::Obstructed { fixed, .. } => {
            let names: BTreeSet<String> = fixed.iter().map(|id| n.entity(*id, 0)).collect();
            format!("{head}: blocked by Fixed {}", join(names))
        }
    }
}

/// The echo of an undo or redo of commit `sequence`.
pub(crate) fn undo_redo(verb: &str, sequence: Option<u64>) -> String {
    match sequence {
        Some(s) => format!("{verb} commit {s}"),
        None => format!("{verb} the last commit"),
    }
}

fn join(items: impl IntoIterator<Item = String>) -> String {
    items.into_iter().collect::<Vec<_>>().join(", ")
}

fn len(l: Length) -> String {
    format_length(l)
}

/// A length with `(Proposed)` after it when its source is Proposed.
fn sourced(s: &Sourced<Length>) -> String {
    format!("{}{}", len(s.value), proposed(s.source))
}

fn proposed(source: Source) -> &'static str {
    if source == Source::Proposed {
        " (Proposed)"
    } else {
        ""
    }
}

fn point(p: &Point) -> String {
    format!("({}, {})", len(p.x), len(p.y))
}

fn axis(a: Axis) -> &'static str {
    match a {
        Axis::X => "X",
        Axis::Y => "Y",
    }
}

fn direction(d: Direction) -> &'static str {
    match d {
        Direction::PosX => "+X",
        Direction::NegX => "-X",
        Direction::PosY => "+Y",
        Direction::NegY => "-Y",
    }
}

/// `4" toward +X` for a signed displacement along `a`.
fn toward(a: Axis, d: Length) -> String {
    let n = d.nanometers();
    let sign = if n < 0 { "-" } else { "+" };
    let magnitude = Length::from_nanometers(n.checked_abs().unwrap_or(i64::MAX));
    format!("{} toward {sign}{}", len(magnitude), axis(a))
}

fn side(s: WallSide) -> &'static str {
    match s {
        WallSide::Left => "left",
        WallSide::Right => "right",
    }
}

fn opening_kind(k: OpeningKind) -> &'static str {
    match k {
        OpeningKind::Opening => "opening",
        OpeningKind::Door => "door",
        OpeningKind::Window => "window",
    }
}

fn anchor(e: OpeningEdge) -> &'static str {
    match e {
        OpeningEdge::Start => "start edge",
        OpeningEdge::End => "end edge",
        OpeningEdge::Center => "center",
    }
}

fn function(f: Function) -> &'static str {
    match f {
        Function::Existing => "existing",
        Function::New => "new",
        Function::Demolished => "demolished",
    }
}

fn turns(n: u8) -> String {
    if n == 1 {
        "rotated 1 quarter turn".into()
    } else {
        format!("rotated {n} quarter turns")
    }
}

impl<'a> Names<'a> {
    fn get<T>(&self, pick: fn(&Document) -> &BTreeMap<Uuid, T>, id: Uuid) -> Option<&'a T> {
        pick(self.before)
            .get(&id)
            .or_else(|| pick(self.after).get(&id))
    }

    /// The first entity of a collection that `after` holds and `before` does not.
    fn created<T>(&self, pick: fn(&Document) -> &BTreeMap<Uuid, T>) -> Option<&'a T> {
        let before = pick(self.before);
        pick(self.after)
            .iter()
            .find(|(id, _)| !before.contains_key(id))
            .map(|(_, t)| t)
    }

    /// Any entity, by label, name or description; never by id.
    fn entity(&self, id: Uuid, depth: u8) -> String {
        if depth > MAX_DEPTH {
            return "another entity".into();
        }
        let d = depth.saturating_add(1);
        if let Some(w) = self.get(|x| &x.walls, id) {
            return w.label.to_string();
        }
        if let Some(o) = self.get(|x| &x.occurrences, id) {
            return o.label.to_string();
        }
        if let Some(x) = self.get(|x| &x.datums, id) {
            return format!("datum {:?}", x.name);
        }
        if let Some(x) = self.get(|x| &x.storeys, id) {
            return format!("storey {:?}", x.name);
        }
        if let Some(x) = self.get(|x| &x.spaces, id) {
            return format!("space {:?}", x.name);
        }
        if let Some(x) = self.get(|x| &x.materials, id) {
            return format!("material {:?}", x.name);
        }
        if let Some(x) = self.get(|x| &x.element_types, id) {
            return format!("type {:?}", x.name);
        }
        if let Some(x) = self.get(|x| &x.layer_sets, id) {
            return format!("layer set {:?}", x.name);
        }
        if let Some(x) = self.get(|x| &x.camera_bookmarks, id) {
            return format!("camera bookmark {:?}", x.name);
        }
        if self.get(|x| &x.openings, id).is_some() {
            return self.opening(id, depth);
        }
        if self.get(|x| &x.nodes, id).is_some() {
            return self.node(id);
        }
        if let Some(c) = self.get(|x| &x.constraints, id) {
            return self.constraint(&c.kind, d);
        }
        if let Some(x) = self.get(|x| &x.slabs, id) {
            let count =
                |doc: &Document| doc.slabs.values().filter(|s| s.storey == x.storey).count();
            return self.surface("slab", x.storey, &x.boundary, count, d);
        }
        if let Some(x) = self.get(|x| &x.ceilings, id) {
            let count = |doc: &Document| {
                doc.ceilings
                    .values()
                    .filter(|s| s.storey == x.storey)
                    .count()
            };
            return self.surface("ceiling", x.storey, &x.boundary, count, d);
        }
        if let Some(x) = self.get(|x| &x.roofs, id) {
            let count =
                |doc: &Document| doc.roofs.values().filter(|s| s.storey == x.storey).count();
            return self.surface("roof", x.storey, &x.footprint, count, d);
        }
        if let Some(x) = self.get(|x| &x.variant_slots, id) {
            return format!(
                "the {} slot of {}",
                variant_kind(x.kind),
                self.entity(x.owner, d)
            );
        }
        if self.get(|x| &x.features, id).is_some() {
            return "an operator".into();
        }
        if self.get(|x| &x.assets, id).is_some() {
            return "an asset".into();
        }
        if self.get(|x| &x.underlays, id).is_some() {
            return "an underlay".into();
        }
        if id == self.before.project.id {
            return format!("project {:?}", self.before.project.name);
        }
        if id == self.before.building.id {
            return format!("building {:?}", self.before.building.name);
        }
        "an unknown entity".into()
    }

    /// `the slab on storey "Ground"`, with where it lies when the storey has more than one.
    fn surface(
        &self,
        kind: &str,
        storey: Uuid,
        boundary: &Boundary,
        count: impl Fn(&Document) -> usize,
        depth: u8,
    ) -> String {
        let base = format!("the {kind} on {}", self.entity(storey, depth));
        if count(self.before).max(count(self.after)) <= 1 {
            return base;
        }
        match boundary {
            Boundary::Explicit { points } => match points.first() {
                Some(p) => format!("{base} from {}", point(p)),
                None => base,
            },
            _ => format!("{base} over {}", self.boundary(boundary, depth)),
        }
    }

    fn node(&self, id: Uuid) -> String {
        match self.get(|x| &x.nodes, id) {
            Some(n) => format!("node at ({}, {})", len(n.x), len(n.y)),
            None => "an unknown node".into(),
        }
    }

    /// `the door in W-002 at 3'-6" from W-001 left face`; deeper, `the door in W-002`.
    fn opening(&self, id: Uuid, depth: u8) -> String {
        let Some(o) = self.get(|x| &x.openings, id) else {
            return "an unknown opening".into();
        };
        let d = depth.saturating_add(1);
        let base = format!("the {} in {}", opening_kind(o.kind), self.entity(o.host, d));
        if depth > 0 {
            return base;
        }
        let at = match o.anchor {
            OpeningEdge::Start => " at".to_string(),
            edge => format!(", {} at", anchor(edge)),
        };
        format!(
            "{base}{at} {} from {}",
            len(o.offset.value),
            self.reference(&o.reference, d)
        )
    }

    fn reference(&self, r: &Reference, depth: u8) -> String {
        let d = depth.saturating_add(1);
        match *r {
            Reference::Node { node } => self.node(node),
            Reference::WallFace {
                wall,
                side: s,
                boundary,
            } => {
                if boundary == 0 {
                    format!("{} {} face", self.entity(wall, d), side(s))
                } else {
                    format!(
                        "{} layer boundary {boundary} from the {} face",
                        self.entity(wall, d),
                        side(s)
                    )
                }
            }
            Reference::WallCenter { wall } => format!("{} center", self.entity(wall, d)),
            Reference::OpeningEdge { opening, edge } => {
                format!("the {} of {}", anchor(edge), self.opening(opening, d))
            }
            Reference::ItemEdge { item, side: s } => {
                let s = match s {
                    ItemSide::Front => "front",
                    ItemSide::Back => "back",
                    ItemSide::Left => "left side",
                    ItemSide::Right => "right side",
                };
                format!("the {s} of {}", self.entity(item, d))
            }
            Reference::ItemCenter { item } => format!("{} center", self.entity(item, d)),
            Reference::Datum { datum } => self.entity(datum, d),
        }
    }

    fn constraint(&self, kind: &ConstraintKind, depth: u8) -> String {
        let d = depth.saturating_add(1);
        match kind {
            ConstraintKind::Distance { a, b, axis: x, .. } => format!(
                "the distance from {} to {} along {}",
                self.reference(a, d),
                self.reference(b, d),
                axis(*x)
            ),
            ConstraintKind::Align { a, b, axis: x } => format!(
                "the alignment of {} and {} on {}",
                self.reference(a, d),
                self.reference(b, d),
                axis(*x)
            ),
            ConstraintKind::Equal { first, second } => format!(
                "the equality of {} and {}",
                self.entity(*first, d),
                self.entity(*second, d)
            ),
            ConstraintKind::Fixed {
                reference, axis: x, ..
            } => format!(
                "the position of {} on {}",
                self.reference(reference, d),
                axis(*x)
            ),
            ConstraintKind::MinClearance { object, other, .. } => format!(
                "the minimum clearance between {} and {}",
                self.entity(*object, d),
                self.entity(*other, d)
            ),
        }
    }

    fn boundary(&self, b: &Boundary, depth: u8) -> String {
        match b {
            Boundary::Explicit { points } => {
                format!("the outline {}", join(points.iter().map(point)))
            }
            Boundary::FromSpace { space } => self.entity(*space, depth),
            Boundary::FromSpaces { spaces } => join(spaces.iter().map(|s| self.entity(*s, depth))),
        }
    }

    fn layers(&self, layers: &[Layer]) -> String {
        join(layers.iter().map(|l| {
            let core = if l.core { " (core)" } else { "" };
            format!(
                "{} {}{core}",
                sourced(&l.thickness),
                self.entity(l.material, 1)
            )
        }))
    }

    /// `the distance from ... from 10'-0" to 11'-0"`: one dimension and its new value.
    fn dimension_change(&self, e: &SetConstraintValue) -> String {
        let (what, old) = match e.target {
            DimensionTarget::Constraint { constraint } => {
                match self.get(|x| &x.constraints, constraint) {
                    Some(c) => {
                        let old = match c.kind {
                            ConstraintKind::Distance { value, .. }
                            | ConstraintKind::Fixed { value, .. }
                            | ConstraintKind::MinClearance { value, .. } => Some(value),
                            _ => None,
                        };
                        (self.constraint(&c.kind, 0), old)
                    }
                    None => ("an unknown dimension".to_string(), None),
                }
            }
            DimensionTarget::OpeningOffset { opening } => {
                match self.get(|x| &x.openings, opening) {
                    Some(o) => (
                        format!(
                            "the offset of {} from {}",
                            self.opening(opening, 1),
                            self.reference(&o.reference, 1)
                        ),
                        Some(o.offset.value),
                    ),
                    None => ("the offset of an unknown opening".to_string(), None),
                }
            }
            DimensionTarget::OccurrenceOffset { occurrence } => {
                let name = self.entity(occurrence, 0);
                match self
                    .get(|x| &x.occurrences, occurrence)
                    .map(|o| &o.placement)
                {
                    Some(Placement::WallMounted {
                        reference, offset, ..
                    }) => (
                        format!("the offset of {name} from {}", self.reference(reference, 1)),
                        Some(offset.value),
                    ),
                    _ => (format!("the offset of {name}"), None),
                }
            }
            DimensionTarget::NodeCoordinate { node, axis: a } => {
                let old = self.get(|x| &x.nodes, node).map(|n| match a {
                    Axis::X => n.x,
                    Axis::Y => n.y,
                });
                (
                    format!("the {} coordinate of {}", axis(a), self.node(node)),
                    old,
                )
            }
            DimensionTarget::OccurrenceCoordinate {
                occurrence,
                axis: a,
            } => {
                let old = match self
                    .get(|x| &x.occurrences, occurrence)
                    .map(|o| &o.placement)
                {
                    Some(Placement::Floor { x, y, .. } | Placement::CeilingMounted { x, y }) => {
                        Some(match a {
                            Axis::X => x.value,
                            Axis::Y => y.value,
                        })
                    }
                    _ => None,
                };
                (
                    format!(
                        "the {} coordinate of {} center",
                        axis(a),
                        self.entity(occurrence, 0)
                    ),
                    old,
                )
            }
        };
        match old {
            Some(old) => format!("{what} from {} to {}", len(old), len(e.value)),
            None => format!("{what} to {}", len(e.value)),
        }
    }

    /// `Move W-002 center 1'-0" toward +X`, with a target, its edit pick and absorbers.
    fn movement(
        &self,
        action: &Action,
        subject: Moved<'_>,
        motion: Motion,
        edit: Option<Uuid>,
        absorb: &[Uuid],
    ) -> String {
        let Moved {
            name: subject,
            axis: fallback_axis,
            target_of,
        } = subject;
        let measured = request_for(self.before, action)
            .and_then(Result::ok)
            .and_then(|req| displacement(self.before, &req).ok().map(|d| (req.axis, d)));
        let ax = measured.map(|(a, _)| a).or(fallback_axis);
        let mut text = format!("Move {subject}");
        match (motion, measured) {
            (Motion::Delta { delta }, _) => match ax {
                Some(a) => text.push_str(&format!(" {}", toward(a, delta))),
                None => text.push_str(&format!(" by {}", len(delta))),
            },
            (Motion::Target { target }, Some((a, d))) => text.push_str(&format!(
                " {}, {target_of}to {} = {}",
                toward(a, d),
                axis(a),
                len(target)
            )),
            (Motion::Target { target }, None) => match ax {
                Some(a) => text.push_str(&format!(" {target_of}to {} = {}", axis(a), len(target))),
                None => text.push_str(&format!(" {target_of}to {}", len(target))),
            },
        }
        if let Some(e) = edit {
            text.push_str(&format!(", changing {}", self.entity(e, 0)));
        }
        if !absorb.is_empty() {
            text.push_str(&format!(
                ", absorbed by {}",
                join(absorb.iter().map(|a| self.entity(*a, 0)))
            ));
        }
        text
    }

    fn placement(&self, p: &Placement) -> String {
        match p {
            Placement::Floor {
                x,
                y,
                rotation,
                elevation,
            } => {
                let r = if rotation.turns() == 0 {
                    String::new()
                } else {
                    format!(", {}", turns(rotation.turns()))
                };
                format!(
                    "on the floor at ({}, {}){r}, elevation {}",
                    sourced(x),
                    sourced(y),
                    sourced(elevation)
                )
            }
            Placement::WallMounted {
                wall,
                side: s,
                reference,
                offset,
                elevation,
            } => format!(
                "on the {} side of {}, {} from {}, elevation {}",
                side(*s),
                self.entity(*wall, 1),
                sourced(offset),
                self.reference(reference, 1),
                sourced(elevation)
            ),
            Placement::CeilingMounted { x, y } => {
                format!("on the ceiling at ({}, {})", sourced(x), sourced(y))
            }
        }
    }

    fn command(&self, command: &Command) -> String {
        match &command.action {
            Action::CreateNode(a) => format!("Create a node at ({}, {})", len(a.x), len(a.y)),
            Action::CreateWall(a) => self.create_wall(a),
            Action::SplitWall(_) => "Split a wall".into(),
            Action::MergeWalls(_) => "Merge walls".into(),
            Action::SetWallLayers(a) => format!(
                "Set the layers of {} to {}; left extent {}",
                self.entity(a.wall, 0),
                self.layers(&a.layers),
                sourced(&a.left_extent)
            ),
            Action::MoveWall(m) => self.movement(
                &command.action,
                Moved {
                    name: self.reference(&m.reference, 0),
                    axis: self.get(|x| &x.walls, m.wall).map(|w| match w.direction {
                        Direction::PosX | Direction::NegX => Axis::Y,
                        Direction::PosY | Direction::NegY => Axis::X,
                    }),
                    target_of: "",
                },
                m.motion,
                m.edit,
                &m.absorb,
            ),
            Action::CreateOpening(a) => {
                let mut text = format!(
                    "Create a {} in {}, {} {} from {}",
                    opening_kind(a.kind),
                    self.entity(a.host, 0),
                    anchor(a.anchor),
                    sourced(&a.offset),
                    self.reference(&a.reference, 0)
                );
                if let Some(w) = &a.width {
                    text.push_str(&format!(", width {}", sourced(w)));
                }
                if let Some(h) = &a.height {
                    text.push_str(&format!(", height {}", sourced(h)));
                }
                if let Some(s) = &a.sill {
                    text.push_str(&format!(", sill {}", sourced(s)));
                }
                text
            }
            Action::MoveOpening(m) => self.movement(
                &command.action,
                Moved {
                    name: self.opening(m.opening, 0),
                    axis: None,
                    target_of: "start edge ",
                },
                m.motion,
                m.edit,
                &m.absorb,
            ),
            Action::PlaceOccurrence(a) => self.place_occurrence(a),
            Action::MoveOccurrence(m) => self.movement(
                &command.action,
                Moved {
                    name: format!("{} center", self.entity(m.occurrence, 0)),
                    axis: Some(m.axis),
                    target_of: "",
                },
                m.motion,
                m.edit,
                &m.absorb,
            ),
            Action::RotateOccurrence(_) => "Rotate an occurrence".into(),
            Action::DeleteEntity(_) => "Delete an entity".into(),
            Action::AddConstraint(a) => self.add_constraint(a),
            Action::RemoveConstraint(_) => "Remove a constraint".into(),
            Action::SetConstraintValue(e) => format!("Set {}", self.dimension_change(e)),
            Action::SetStorey(a) => {
                let values = format!(
                    "elevation {}, floor to floor {}, ceiling height {}",
                    sourced(&a.elevation),
                    sourced(&a.floor_to_floor),
                    sourced(&a.default_ceiling_height)
                );
                match a.storey.and_then(|id| self.before.storeys.get(&id)) {
                    None => format!("Create storey {:?}: {values}", a.name),
                    Some(old) if old.name == a.name => {
                        format!("Set storey {:?}: {values}", a.name)
                    }
                    Some(old) => format!("Set storey {:?}: name {:?}, {values}", old.name, a.name),
                }
            }
            Action::ApplyOperator(_) => "Apply an operator".into(),
            Action::RemoveOperator(_) => "Remove an operator".into(),
            Action::SetVariantSelection(a) => format!(
                "Select {} for {}",
                self.entity(a.selected, 0),
                self.entity(a.slot, 0)
            ),
            Action::SetProjectUnits(a) => format!(
                "Set the unit of bare numbers to {}",
                match a.default_unit {
                    ProjectUnit::Inches => "inches",
                    ProjectUnit::Feet => "feet",
                }
            ),
            Action::CreateDatum(a) => match a.geometry {
                DatumGeometry::Point { x, y } => {
                    format!("Create datum {:?} at ({}, {})", a.name, len(x), len(y))
                }
                DatumGeometry::XLine { x } => {
                    format!("Create datum {:?} on the line X = {}", a.name, len(x))
                }
                DatumGeometry::YLine { y } => {
                    format!("Create datum {:?} on the line Y = {}", a.name, len(y))
                }
            },
            Action::CreateMaterial(a) => match &a.tile_size {
                Some(t) => format!("Create material {:?}, tile size {}", a.name, sourced(t)),
                None => format!("Create material {:?}", a.name),
            },
            Action::CreateSpace(a) => {
                let mut text = format!(
                    "Create space {:?} on {} around {}",
                    a.name,
                    self.entity(a.storey, 0),
                    point(&a.seed)
                );
                if let Some(f) = &a.finishes {
                    for (what, m) in [
                        ("floor", f.floor),
                        ("wall", f.walls),
                        ("ceiling", f.ceiling),
                    ] {
                        if let Some(m) = m {
                            text.push_str(&format!(", {what} finish {}", self.entity(m, 0)));
                        }
                    }
                }
                text
            }
            Action::CreateSlab(a) => {
                self.create_surface("slab", a.storey, &a.boundary, &a.thickness, &a.elevation)
            }
            Action::CreateCeiling(a) => {
                self.create_surface("ceiling", a.storey, &a.boundary, &a.thickness, &a.elevation)
            }
            Action::CreateRoof(a) => self.create_roof(a),
            Action::CreateElementType(a) => format!(
                "Create type {:?}, {} wide, {} deep, {} high",
                a.name,
                len(a.declared_dimensions.width),
                len(a.declared_dimensions.depth),
                len(a.declared_dimensions.height)
            ),
        }
    }

    fn create_wall(&self, a: &CreateWall) -> String {
        let label = self
            .created(|x| &x.walls)
            .map_or_else(|| "a wall".to_string(), |w| format!("wall {}", w.label));
        let mut text = format!(
            "Create {label} on {} from {} to {}, toward {}, {}",
            self.entity(a.storey, 0),
            self.node(a.node_a),
            self.node(a.node_b),
            direction(a.direction),
            function(a.function)
        );
        match &a.layers {
            Some(l) => text.push_str(&format!("; layers {}", self.layers(l))),
            None => text.push_str("; catalog layers"),
        }
        if let Some(e) = &a.left_extent {
            text.push_str(&format!("; left extent {}", sourced(e)));
        }
        if let Some(b) = &a.base_offset {
            text.push_str(&format!("; base offset {}", sourced(b)));
        }
        match &a.top {
            None | Some(WallTop::Storey) => text.push_str("; top at the storey height"),
            Some(WallTop::Height { height }) => {
                text.push_str(&format!("; height {}", sourced(height)))
            }
            Some(WallTop::Roof { roof }) => {
                text.push_str(&format!("; top under {}", self.entity(*roof, 0)))
            }
        }
        if let Some(p) = a.join_priority.filter(|p| *p != 0) {
            text.push_str(&format!("; join priority {p}"));
        }
        text
    }

    fn place_occurrence(&self, a: &PlaceOccurrence) -> String {
        let label = self
            .created(|x| &x.occurrences)
            .map_or_else(|| "an item".to_string(), |o| o.label.to_string());
        let mut text = format!(
            "Place {label} ({}) on {}, {}",
            self.entity(a.element_type, 0),
            self.entity(a.storey, 0),
            self.placement(&a.placement)
        );
        if let Some(c) = &a.clearance {
            text.push_str(&format!(
                "; clearance front {}, back {}, left {}, right {}",
                sourced(&c.front),
                sourced(&c.back),
                sourced(&c.left),
                sourced(&c.right)
            ));
        }
        if a.fixed {
            text.push_str("; Fixed");
        }
        if a.overlap_allowed {
            text.push_str("; may overlap");
        }
        if let Some(l) = &a.light {
            let kind = match l.kind {
                LightKind::Point => "point",
                LightKind::Spot => "spot",
                LightKind::Rect => "rect",
            };
            text.push_str(&format!(
                "; {kind} light, {} lm{}, {} K{}",
                l.lumens.value,
                proposed(l.lumens.source),
                l.kelvin.value,
                proposed(l.kelvin.source)
            ));
        }
        text
    }

    fn add_constraint(&self, a: &AddConstraint) -> String {
        let mut text = match &a.kind {
            ConstraintKind::Distance {
                a: from,
                b: to,
                axis: x,
                value,
                tolerance,
            } => {
                let mut t = format!(
                    "Add the distance from {} to {} along {}: {}",
                    self.reference(from, 0),
                    self.reference(to, 0),
                    axis(*x),
                    len(*value)
                );
                if let Some(tol) = tolerance {
                    t.push_str(&format!(", tolerance {}", len(*tol)));
                }
                t
            }
            ConstraintKind::Align {
                a: p,
                b: q,
                axis: x,
            } => format!(
                "Align {} and {} on {}",
                self.reference(p, 0),
                self.reference(q, 0),
                axis(*x)
            ),
            ConstraintKind::Equal { first, second } => format!(
                "Make {} equal to {}",
                self.entity(*first, 0),
                self.entity(*second, 0)
            ),
            ConstraintKind::Fixed {
                reference,
                axis: x,
                value,
            } => format!(
                "Fix {} on {} at {}",
                self.reference(reference, 0),
                axis(*x),
                len(*value)
            ),
            ConstraintKind::MinClearance {
                object,
                other,
                value,
            } => format!(
                "Keep at least {} between {} and {}",
                len(*value),
                self.entity(*object, 0),
                self.entity(*other, 0)
            ),
        };
        text.push_str(proposed(a.source));
        if a.status == Some(ConstraintStatus::Reference) {
            text.push_str(", as a reference dimension");
        }
        if a.provenance == Some(Provenance::Transcribed) {
            text.push_str(", transcribed from the underlay");
        }
        text
    }

    fn create_surface(
        &self,
        kind: &str,
        storey: Uuid,
        boundary: &Boundary,
        thickness: &Option<Sourced<Length>>,
        elevation: &Option<Sourced<Length>>,
    ) -> String {
        let mut text = format!(
            "Create a {kind} on {} over {}",
            self.entity(storey, 0),
            self.boundary(boundary, 0)
        );
        if let Some(t) = thickness {
            text.push_str(&format!(", thickness {}", sourced(t)));
        }
        if let Some(e) = elevation {
            text.push_str(&format!(", elevation {}", sourced(e)));
        }
        text
    }

    fn create_roof(&self, a: &CreateRoof) -> String {
        let mut text = format!(
            "Create a shed roof on {} over {}",
            self.entity(a.storey, 0),
            self.boundary(&a.footprint, 0)
        );
        if let Some(d) = a.slope_direction {
            text.push_str(&format!(", sloping down toward {}", direction(d)));
        }
        if let Some(h) = &a.high_edge_height {
            text.push_str(&format!(", high edge {}", sourced(h)));
        }
        if let Some(h) = &a.low_edge_height {
            text.push_str(&format!(", low edge {}", sourced(h)));
        }
        if let Some(p) = &a.pitch {
            text.push_str(&format!(
                ", pitch {}:{}{}",
                p.value.rise,
                p.value.run,
                proposed(p.source)
            ));
        }
        if let Some(r) = &a.ledger {
            text.push_str(&format!(", ledger {}", self.reference(r, 0)));
        }
        if let Some(o) = &a.overhang {
            text.push_str(&format!(
                ", overhang +X {}, -X {}, +Y {}, -Y {}",
                sourced(&o.pos_x),
                sourced(&o.neg_x),
                sourced(&o.pos_y),
                sourced(&o.neg_y)
            ));
        }
        if let Some(t) = &a.thickness {
            text.push_str(&format!(", thickness {}", sourced(t)));
        }
        if let Some(f) = &a.fascia_depth {
            text.push_str(&format!(", fascia depth {}", sourced(f)));
        }
        for (slot, m) in &a.material_slots {
            text.push_str(&format!(", {slot} {}", self.entity(*m, 0)));
        }
        text
    }
}

/// What a move moves: its name, its axis if known without solving, and
/// what a target coordinate names when that is not the subject itself.
struct Moved<'s> {
    name: String,
    axis: Option<Axis>,
    target_of: &'s str,
}

fn variant_kind(k: VariantKind) -> &'static str {
    match k {
        VariantKind::Material => "material",
        VariantKind::Model => "model",
        VariantKind::Lighting => "lighting",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::canonical::to_canonical_json;
    use crate::command::tests::cmd;
    use crate::command::{
        CreateNode, CreateOpening, MoveOccurrence, MoveOpening, MoveWall, NoArgs, SetProjectUnits,
        SetStorey, SetVariantSelection, SetWallLayers, apply,
    };
    use crate::create::{
        CreateCeiling, CreateDatum, CreateElementType, CreateMaterial, CreateSlab, CreateSpace,
    };
    use crate::document::*;
    use crate::edit::tests::{EAST, IN, STOREY, center, chain, datum, uid};

    fn l(nm: i64) -> Length {
        Length::from_nanometers(nm)
    }

    fn user(nm: i64) -> Sourced<Length> {
        Sourced {
            value: l(nm),
            source: Source::UserStated,
        }
    }

    fn prop(nm: i64) -> Sourced<Length> {
        Sourced {
            value: l(nm),
            source: Source::Proposed,
        }
    }

    fn turns(n: u8) -> QuarterTurns {
        QuarterTurns::new(n).unwrap_or_else(|| unreachable!())
    }

    const STUD: Uuid = Uuid::from_bytes([6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    const OAK: Uuid = Uuid::from_bytes([6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    const SAW: Uuid = Uuid::from_bytes([11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    const SLOT: Uuid = Uuid::from_bytes([12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    const SHOP: Uuid = Uuid::from_bytes([13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    const SAW_TYPE: Uuid = Uuid::from_bytes([7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

    /// The 8.7 chain (west datum -120- W-001 -96- W-002 -72- W-003, a window
    /// on W-003) with distinct wall labels, the layer materials, a band saw
    /// EQ-001 with a material slot, and a space "Shop".
    pub(crate) fn labeled_chain() -> (Document, [Uuid; 3], [Uuid; 3], Uuid) {
        let (mut doc, walls, constraints, window) = chain();
        for w in walls {
            let label = doc.labels.allocate("W");
            if let (Some(wall), Ok(label)) = (doc.walls.get_mut(&w), label) {
                wall.label = label;
            }
        }
        for (id, name) in [(STUD, "Stud"), (OAK, "Oak")] {
            doc.materials.insert(
                id,
                Material {
                    name: name.into(),
                    asset: None,
                    tile_size: None,
                    properties: Properties::new(),
                },
            );
        }
        doc.element_types.insert(
            SAW_TYPE,
            ElementType {
                name: "Band saw".into(),
                declared_dimensions: DeclaredDimensions {
                    width: l(30 * IN),
                    depth: l(26 * IN),
                    height: l(68 * IN),
                    authority: Authority::ProposedDefault,
                },
                fidelity: Fidelity::Proxy,
                model: None,
                properties: Properties::new(),
                source: String::new(),
                license: String::new(),
            },
        );
        let label = doc.labels.allocate("EQ").unwrap_or_else(|_| unreachable!());
        doc.occurrences.insert(
            SAW,
            Occurrence {
                label,
                storey: STOREY,
                element_type: SAW_TYPE,
                placement: Placement::Floor {
                    x: user(60 * IN),
                    y: user(36 * IN),
                    rotation: turns(0),
                    elevation: user(0),
                },
                clearance: Clearance {
                    front: user(0),
                    back: user(0),
                    left: user(0),
                    right: user(0),
                },
                fixed: false,
                overlap_allowed: false,
                light: None,
                center: Point {
                    x: l(60 * IN),
                    y: l(36 * IN),
                },
            },
        );
        doc.variant_slots.insert(
            SLOT,
            VariantSlot {
                owner: SAW,
                kind: VariantKind::Material,
                options: vec![STUD, OAK],
                selected: STUD,
            },
        );
        doc.spaces.insert(
            SHOP,
            Space {
                storey: STOREY,
                name: "Shop".into(),
                seed: Point {
                    x: l(60 * IN),
                    y: l(36 * IN),
                },
                boundary: Vec::new(),
                finishes: Finishes {
                    floor: None,
                    walls: None,
                    ceiling: None,
                },
            },
        );
        (doc, walls, constraints, window)
    }

    /// Renders `action` against `doc` and the document it gives (or `doc`
    /// itself for a command that cannot apply yet).
    fn echo(doc: &Document, action: Action) -> (String, Command, Document) {
        let c = cmd(1, action);
        let after = apply(doc, &c).unwrap_or_else(|_| doc.clone());
        (render(doc, &after, &c), c, after)
    }

    /// Every `kind` the command schema lists.
    fn all_kinds() -> BTreeSet<String> {
        let schema = serde_json::to_value(schemars::schema_for!(Action)).unwrap_or_default();
        let mut kinds = BTreeSet::new();
        for branch in schema["oneOf"].as_array().into_iter().flatten() {
            let kind = &branch["properties"]["kind"];
            for k in kind["enum"]
                .as_array()
                .into_iter()
                .flatten()
                .chain([&kind["const"]])
            {
                if let Some(k) = k.as_str() {
                    kinds.insert(k.to_string());
                }
            }
        }
        kinds
    }

    /// Every substring of `json` shaped like a UUID (8-4-4-4-12 hex digits).
    fn uuids(json: &str) -> BTreeSet<String> {
        let b = json.as_bytes();
        let mut out = BTreeSet::new();
        for s in b.windows(36) {
            let shaped = s.iter().enumerate().all(|(i, c)| match i {
                8 | 13 | 18 | 23 => *c == b'-',
                _ => c.is_ascii_hexdigit(),
            });
            if shaped {
                out.insert(String::from_utf8_lossy(s).into_owned());
            }
        }
        out
    }

    fn ceiling_outline() -> Boundary {
        Boundary::Explicit {
            points: [(0, 0), (120, 0), (120, 96), (0, 96)]
                .iter()
                .map(|(x, y)| Point {
                    x: l(x * IN),
                    y: l(y * IN),
                })
                .collect(),
        }
    }

    /// One row per command kind, plus a target move with an absorber.
    fn table(
        walls: [Uuid; 3],
        constraints: [Uuid; 3],
        window: Uuid,
    ) -> Vec<(Action, &'static str)> {
        let [a, b, c] = walls;
        let [_, c2, c3] = constraints;
        let none = NoArgs {};
        vec![
            (
                Action::CreateNode(CreateNode {
                    x: l(144 * IN),
                    y: l(0),
                }),
                r#"Create a node at (12'-0", 0'-0")"#,
            ),
            (
                Action::CreateWall(CreateWall {
                    storey: STOREY,
                    node_a: uid(1, 2),
                    node_b: uid(1, 4),
                    direction: Direction::PosX,
                    function: Function::New,
                    layers: Some(vec![Layer {
                        material: STUD,
                        thickness: user(4 * IN),
                        core: true,
                    }]),
                    left_extent: Some(user(2 * IN)),
                    base_offset: None,
                    top: None,
                    join_priority: None,
                }),
                r#"Create wall W-004 on storey "Ground" from node at (10'-0", 0'-0") to node at (18'-0", 0'-0"), toward +X, new; layers 0'-4" material "Stud" (core); left extent 0'-2"; top at the storey height"#,
            ),
            (Action::SplitWall(none.clone()), "Split a wall"),
            (Action::MergeWalls(none.clone()), "Merge walls"),
            (
                Action::SetWallLayers(SetWallLayers {
                    wall: b,
                    layers: vec![
                        Layer {
                            material: STUD,
                            thickness: user(4 * IN),
                            core: true,
                        },
                        Layer {
                            material: OAK,
                            thickness: prop(IN / 2),
                            core: false,
                        },
                    ],
                    left_extent: user(9 * IN / 4),
                }),
                r#"Set the layers of W-002 to 0'-4" material "Stud" (core), 0'-0 1/2" (Proposed) material "Oak"; left extent 0'-2 1/4""#,
            ),
            (
                Action::MoveWall(MoveWall {
                    wall: b,
                    reference: center(b),
                    motion: Motion::Delta { delta: l(12 * IN) },
                    edit: None,
                    absorb: vec![],
                }),
                r#"Move W-002 center 1'-0" toward +X"#,
            ),
            (
                Action::MoveWall(MoveWall {
                    wall: b,
                    reference: center(b),
                    motion: Motion::Target {
                        target: l(228 * IN),
                    },
                    edit: None,
                    absorb: vec![c3],
                }),
                r#"Move W-002 center 1'-0" toward +X, to X = 19'-0", absorbed by the distance from W-002 center to W-003 center along X"#,
            ),
            (
                Action::CreateOpening(CreateOpening {
                    kind: OpeningKind::Door,
                    host: a,
                    reference: Reference::Node { node: uid(1, 2) },
                    anchor: OpeningEdge::Start,
                    offset: user(36 * IN),
                    width: None,
                    height: Some(prop(80 * IN)),
                    sill: None,
                }),
                r#"Create a door in W-001, start edge 3'-0" from node at (10'-0", 0'-0"), height 6'-8" (Proposed)"#,
            ),
            (
                Action::MoveOpening(MoveOpening {
                    opening: window,
                    motion: Motion::Delta { delta: l(-6 * IN) },
                    edit: None,
                    absorb: vec![],
                }),
                r#"Move the window in W-003 at 2'-0" from node at (24'-0", 0'-0") 0'-6" toward -Y"#,
            ),
            (
                Action::PlaceOccurrence(PlaceOccurrence {
                    storey: STOREY,
                    element_type: SAW_TYPE,
                    placement: Placement::Floor {
                        x: user(96 * IN),
                        y: prop(48 * IN),
                        rotation: turns(1),
                        elevation: user(0),
                    },
                    clearance: None,
                    fixed: true,
                    overlap_allowed: false,
                    light: None,
                }),
                r#"Place EQ-002 (type "Band saw") on storey "Ground", on the floor at (8'-0", 4'-0" (Proposed)), rotated 1 quarter turn, elevation 0'-0"; Fixed"#,
            ),
            (
                Action::MoveOccurrence(MoveOccurrence {
                    occurrence: SAW,
                    axis: Axis::Y,
                    motion: Motion::Delta { delta: l(6 * IN) },
                    edit: None,
                    absorb: vec![],
                }),
                r#"Move EQ-001 center 0'-6" toward +Y"#,
            ),
            (
                Action::RotateOccurrence(none.clone()),
                "Rotate an occurrence",
            ),
            (Action::DeleteEntity(none.clone()), "Delete an entity"),
            (
                Action::AddConstraint(AddConstraint {
                    kind: ConstraintKind::Distance {
                        a: datum(EAST),
                        b: center(c),
                        axis: Axis::X,
                        value: l(48 * IN),
                        tolerance: None,
                    },
                    status: None,
                    source: Source::UserStated,
                    provenance: None,
                }),
                r#"Add the distance from datum "East" to W-003 center along X: 4'-0""#,
            ),
            (
                Action::RemoveConstraint(none.clone()),
                "Remove a constraint",
            ),
            (
                Action::SetConstraintValue(SetConstraintValue {
                    target: DimensionTarget::Constraint { constraint: c2 },
                    value: l(108 * IN),
                }),
                r#"Set the distance from W-001 center to W-002 center along X from 8'-0" to 9'-0""#,
            ),
            (
                Action::SetStorey(SetStorey {
                    storey: Some(STOREY),
                    name: "Level 1".into(),
                    elevation: user(0),
                    floor_to_floor: user(120 * IN),
                    default_ceiling_height: user(108 * IN),
                }),
                r#"Set storey "Ground": name "Level 1", elevation 0'-0", floor to floor 10'-0", ceiling height 9'-0""#,
            ),
            (Action::ApplyOperator(none.clone()), "Apply an operator"),
            (Action::RemoveOperator(none), "Remove an operator"),
            (
                Action::SetVariantSelection(SetVariantSelection {
                    slot: SLOT,
                    selected: OAK,
                }),
                r#"Select material "Oak" for the material slot of EQ-001"#,
            ),
            (
                Action::SetProjectUnits(SetProjectUnits {
                    default_unit: ProjectUnit::Feet,
                }),
                "Set the unit of bare numbers to feet",
            ),
            (
                Action::CreateDatum(CreateDatum {
                    name: "North".into(),
                    geometry: DatumGeometry::YLine { y: l(360 * IN) },
                }),
                r#"Create datum "North" on the line Y = 30'-0""#,
            ),
            (
                Action::CreateMaterial(CreateMaterial {
                    name: "Tile".into(),
                    tile_size: Some(user(12 * IN)),
                    properties: Properties::new(),
                }),
                r#"Create material "Tile", tile size 1'-0""#,
            ),
            (
                Action::CreateSpace(CreateSpace {
                    storey: STOREY,
                    name: "Office".into(),
                    seed: Point {
                        x: l(168 * IN),
                        y: l(48 * IN),
                    },
                    finishes: None,
                }),
                r#"Create space "Office" on storey "Ground" around (14'-0", 4'-0")"#,
            ),
            (
                Action::CreateSlab(CreateSlab {
                    storey: STOREY,
                    boundary: Boundary::FromSpace { space: SHOP },
                    thickness: Some(prop(4 * IN)),
                    elevation: None,
                }),
                r#"Create a slab on storey "Ground" over space "Shop", thickness 0'-4" (Proposed)"#,
            ),
            (
                Action::CreateCeiling(CreateCeiling {
                    storey: STOREY,
                    boundary: ceiling_outline(),
                    thickness: None,
                    elevation: Some(user(108 * IN)),
                }),
                r#"Create a ceiling on storey "Ground" over the outline (0'-0", 0'-0"), (10'-0", 0'-0"), (10'-0", 8'-0"), (0'-0", 8'-0"), elevation 9'-0""#,
            ),
            (
                Action::CreateRoof(CreateRoof {
                    storey: STOREY,
                    footprint: Boundary::FromSpace { space: SHOP },
                    slope_direction: None,
                    high_edge_height: Some(user(108 * IN)),
                    low_edge_height: None,
                    pitch: Some(Sourced {
                        value: Pitch { rise: 3, run: 12 },
                        source: Source::UserStated,
                    }),
                    ledger: Some(Reference::WallFace {
                        wall: a,
                        side: WallSide::Right,
                        boundary: 0,
                    }),
                    overhang: None,
                    thickness: None,
                    fascia_depth: None,
                    material_slots: BTreeMap::new(),
                }),
                r#"Create a shed roof on storey "Ground" over space "Shop", high edge 9'-0", pitch 3:12, ledger W-001 right face"#,
            ),
            (
                Action::CreateElementType(CreateElementType {
                    name: "Drill press".into(),
                    declared_dimensions: DeclaredDimensions {
                        width: l(24 * IN),
                        depth: l(16 * IN),
                        height: l(66 * IN),
                        authority: Authority::ProposedDefault,
                    },
                    fidelity: None,
                    properties: Properties::new(),
                    source: String::new(),
                    license: String::new(),
                }),
                r#"Create type "Drill press", 2'-0" wide, 1'-4" deep, 5'-6" high"#,
            ),
        ]
    }

    /// Done when: a table test covers one rendering per command kind, and
    /// the text uses labels, never ids, where a label exists.
    #[test]
    fn one_rendering_per_command_kind() {
        let (doc, walls, constraints, window) = labeled_chain();
        let rows = table(walls, constraints, window);
        let covered: BTreeSet<String> = rows.iter().map(|(a, _)| a.kind().to_string()).collect();
        assert_eq!(covered, all_kinds());
        assert_eq!(covered.len(), 27);
        for (action, expected) in rows {
            let (text, c, after) = echo(&doc, action);
            assert_eq!(text, expected, "{}", c.action.kind());
            // No id of either document or of the command appears in the text.
            let mut ids = uuids(&to_canonical_json(&doc).unwrap_or_default());
            ids.extend(uuids(&to_canonical_json(&after).unwrap_or_default()));
            ids.extend(uuids(&to_canonical_json(&c).unwrap_or_default()));
            assert!(ids.len() > 20, "{}", ids.len());
            for id in ids {
                assert!(!text.contains(&id), "{text} contains {id}");
            }
        }
    }

    /// Unlabeled things are described; a reference cycle cannot recurse forever.
    #[test]
    fn unlabeled_entities_are_described() {
        let (mut doc, _, [c1, _, _], window) = labeled_chain();
        let n = Names {
            before: &doc,
            after: &doc,
        };
        assert_eq!(
            n.entity(c1, 0),
            r#"the distance from datum "West" to W-001 center along X"#
        );
        assert_eq!(
            n.entity(window, 0),
            r#"the window in W-003 at 2'-0" from node at (24'-0", 0'-0")"#
        );
        assert_eq!(n.entity(SLOT, 0), "the material slot of EQ-001");
        assert_eq!(n.entity(uid(99, 99), 0), "an unknown entity");
        let looped = uid(3, 50);
        doc.constraints.insert(
            looped,
            Constraint {
                kind: ConstraintKind::Equal {
                    first: looped,
                    second: c1,
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
        let n = Names {
            before: &doc,
            after: &doc,
        };
        assert!(n.entity(looped, 0).contains("another entity"));
    }

    /// Off-grid lengths keep their leading ~ (CLAUDE.md).
    #[test]
    fn off_grid_values_show_the_tilde() {
        let (doc, _, [_, c2, _], _) = labeled_chain();
        let (text, _, _) = echo(
            &doc,
            Action::SetConstraintValue(SetConstraintValue {
                target: DimensionTarget::Constraint { constraint: c2 },
                value: l(108 * IN + 1),
            }),
        );
        assert_eq!(
            text,
            r#"Set the distance from W-001 center to W-002 center along X from 8'-0" to ~9'-0""#
        );
    }
}
