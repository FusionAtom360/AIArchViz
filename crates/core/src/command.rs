//! Commands (spec 8.1, 8.2) and the pure `apply`.
//!
//! A command is `{id, kind, args, idempotency_key}`. It holds no clock value
//! or randomness: every new entity id is `Uuid::derive(idempotency_key,
//! "command", ordinal)`, ordinals counting from 0 within the command.
//!
//! T-105 implements the commands that are pure data edits (Q-9): CreateNode,
//! SetStorey, SetVariantSelection and SetProjectUnits; T-108 adds
//! CreateOpening (Q-23). Every other v1
//! command parses with empty `args` and returns
//! [`CommandError::NotYetImplemented`]; its arguments are defined by the task
//! that implements it.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::document::{
    Axis, ConstraintKind, Document, JoinType, Node, Opening, OpeningEdge, OpeningKind, Placement,
    Reference, Source, Sourced, Storey,
};
use crate::identity::Uuid;
use crate::length::Length;
use crate::solver::{LayerA, SolveError, Solver};
use crate::units::ProjectUnit;

/// One command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub id: Uuid,
    pub idempotency_key: Uuid,
    pub action: Action,
}

/// Arguments of a command whose arguments are not defined yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

/// Creates an isolated node. Its position is provisional until solved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateNode {
    pub x: Length,
    pub y: Length,
}

/// Creates a storey (`storey` is null) or replaces an existing one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetStorey {
    pub storey: Option<Uuid>,
    pub name: String,
    pub elevation: Sourced<Length>,
    pub floor_to_floor: Sourced<Length>,
    pub default_ceiling_height: Sourced<Length>,
}

/// Selects one of a variant slot's options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetVariantSelection {
    pub slot: Uuid,
    pub selected: Uuid,
}

/// Creates an opening, door or window in a host wall (Q-23). Null sizes take
/// the defaults catalog value with source Proposed (6.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateOpening {
    pub kind: OpeningKind,
    pub host: Uuid,
    pub reference: Reference,
    pub anchor: OpeningEdge,
    pub offset: Sourced<Length>,
    pub width: Option<Sourced<Length>>,
    pub height: Option<Sourced<Length>>,
    pub sill: Option<Sourced<Length>>,
}

/// How far a move goes: a signed delta along the axis, or the target
/// coordinate of the moved reference (Q-39).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Motion {
    Delta { delta: Length },
    Target { target: Length },
}

/// Moves a wall across its axis by editing the dimensions that fix it (8.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveWall {
    pub wall: Uuid,
    /// The wall's own face or center whose coordinate `motion` names.
    pub reference: Reference,
    pub motion: Motion,
    /// The constraint the user picked for a ReferenceSemantics ambiguity.
    pub edit: Option<Uuid>,
    /// PolicyChoices taken: dimensions that absorb the move (8.7 (b)).
    pub absorb: Vec<Uuid>,
}

/// Moves an opening along its host; `motion` names its start edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveOpening {
    pub opening: Uuid,
    pub motion: Motion,
    pub edit: Option<Uuid>,
    pub absorb: Vec<Uuid>,
}

/// Moves an occurrence's center along one axis (a wall-mounted item only
/// along its host).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveOccurrence {
    pub occurrence: Uuid,
    pub axis: Axis,
    pub motion: Motion,
    pub edit: Option<Uuid>,
    pub absorb: Vec<Uuid>,
}

/// A dimension a move can change (Q-39).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DimensionTarget {
    /// The value of a Distance or Fixed constraint.
    Constraint { constraint: Uuid },
    /// An opening's offset from its reference.
    OpeningOffset { opening: Uuid },
    /// A wall-mounted occurrence's offset from its reference.
    OccurrenceOffset { occurrence: Uuid },
    /// A node's stored coordinate (free classes, Q-38).
    NodeCoordinate { node: Uuid, axis: Axis },
    /// A floor or ceiling occurrence's stored center coordinate (Q-38).
    OccurrenceCoordinate { occurrence: Uuid, axis: Axis },
}

impl DimensionTarget {
    /// The entity the dimension belongs to; also how a move's `edit` and
    /// `absorb` name it.
    pub fn id(self) -> Uuid {
        match self {
            DimensionTarget::Constraint { constraint } => constraint,
            DimensionTarget::OpeningOffset { opening } => opening,
            DimensionTarget::OccurrenceOffset { occurrence }
            | DimensionTarget::OccurrenceCoordinate { occurrence, .. } => occurrence,
            DimensionTarget::NodeCoordinate { node, .. } => node,
        }
    }
}

/// Sets one dimension (8.2). Moves resolve to these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetConstraintValue {
    pub target: DimensionTarget,
    pub value: Length,
}

/// Sets the unit of bare numbers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetProjectUnits {
    pub default_unit: ProjectUnit,
}

/// The v1 command set (8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "args",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Action {
    CreateNode(CreateNode),
    CreateWall(NoArgs),
    SplitWall(NoArgs),
    MergeWalls(NoArgs),
    SetWallLayers(NoArgs),
    MoveWall(MoveWall),
    CreateOpening(CreateOpening),
    MoveOpening(MoveOpening),
    PlaceOccurrence(NoArgs),
    MoveOccurrence(MoveOccurrence),
    RotateOccurrence(NoArgs),
    DeleteEntity(NoArgs),
    AddConstraint(NoArgs),
    RemoveConstraint(NoArgs),
    SetConstraintValue(SetConstraintValue),
    SetStorey(SetStorey),
    ApplyOperator(NoArgs),
    RemoveOperator(NoArgs),
    SetVariantSelection(SetVariantSelection),
    SetProjectUnits(SetProjectUnits),
}

impl Action {
    /// The `kind` string of this action.
    pub fn kind(&self) -> &'static str {
        match self {
            Action::CreateNode(_) => "create_node",
            Action::CreateWall(_) => "create_wall",
            Action::SplitWall(_) => "split_wall",
            Action::MergeWalls(_) => "merge_walls",
            Action::SetWallLayers(_) => "set_wall_layers",
            Action::MoveWall(_) => "move_wall",
            Action::CreateOpening(_) => "create_opening",
            Action::MoveOpening(_) => "move_opening",
            Action::PlaceOccurrence(_) => "place_occurrence",
            Action::MoveOccurrence(_) => "move_occurrence",
            Action::RotateOccurrence(_) => "rotate_occurrence",
            Action::DeleteEntity(_) => "delete_entity",
            Action::AddConstraint(_) => "add_constraint",
            Action::RemoveConstraint(_) => "remove_constraint",
            Action::SetConstraintValue(_) => "set_constraint_value",
            Action::SetStorey(_) => "set_storey",
            Action::ApplyOperator(_) => "apply_operator",
            Action::RemoveOperator(_) => "remove_operator",
            Action::SetVariantSelection(_) => "set_variant_selection",
            Action::SetProjectUnits(_) => "set_project_units",
        }
    }
}

/// The flat wire form of a command (8.1).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    id: Uuid,
    idempotency_key: Uuid,
    kind: String,
    args: Value,
}

impl Serialize for Command {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        let tagged = serde_json::to_value(&self.action).map_err(S::Error::custom)?;
        let args = tagged.get("args").cloned().unwrap_or(Value::Null);
        Wire {
            id: self.id,
            idempotency_key: self.idempotency_key,
            kind: self.action.kind().to_string(),
            args,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Command {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let wire = Wire::deserialize(deserializer)?;
        let tagged = serde_json::json!({ "kind": wire.kind, "args": wire.args });
        let action = serde_json::from_value(tagged).map_err(D::Error::custom)?;
        Ok(Command {
            id: wire.id,
            idempotency_key: wire.idempotency_key,
            action,
        })
    }
}

/// Why a command cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// The command is in the v1 set but a later task implements it.
    NotYetImplemented(&'static str),
    /// No entity of the expected kind has this id.
    UnknownEntity(Uuid),
    /// A derived new id is already taken (the same idempotency key reused).
    IdCollision(Uuid),
    /// The selected option is not one of the slot's options.
    InvalidOption { slot: Uuid, option: Uuid },
    /// The document could not be solved after the command.
    Solve(SolveError),
    /// A move must be resolved by `propose` into SetConstraintValue commands (Q-39).
    Unresolved(&'static str),
    /// The target is not a dimension that can be set (an Align, an Equal, a
    /// floor item's offset, and so on).
    NotADimension(Uuid),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::NotYetImplemented(kind) => write!(f, "{kind} is not implemented yet"),
            CommandError::UnknownEntity(id) => write!(f, "no entity {id}"),
            CommandError::IdCollision(id) => write!(f, "new id {id} already exists"),
            CommandError::Solve(e) => write!(f, "cannot solve: {e}"),
            CommandError::Unresolved(kind) => {
                write!(f, "{kind} must be resolved through propose")
            }
            CommandError::NotADimension(id) => write!(f, "{id} has no settable dimension"),
            CommandError::InvalidOption { slot, option } => {
                write!(f, "{option} is not an option of variant slot {slot}")
            }
        }
    }
}

impl std::error::Error for CommandError {}

fn set_dimension(doc: &mut Document, args: &SetConstraintValue) -> Result<(), CommandError> {
    let value = args.value;
    let stated = |s: &mut Sourced<Length>| {
        *s = Sourced {
            value,
            source: Source::UserStated,
        };
    };
    match args.target {
        DimensionTarget::Constraint { constraint } => {
            let c = doc
                .constraints
                .get_mut(&constraint)
                .ok_or(CommandError::UnknownEntity(constraint))?;
            match &mut c.kind {
                ConstraintKind::Distance { value: v, .. }
                | ConstraintKind::Fixed { value: v, .. } => *v = value,
                _ => return Err(CommandError::NotADimension(constraint)),
            }
        }
        DimensionTarget::OpeningOffset { opening } => {
            let o = doc
                .openings
                .get_mut(&opening)
                .ok_or(CommandError::UnknownEntity(opening))?;
            stated(&mut o.offset);
        }
        DimensionTarget::OccurrenceOffset { occurrence } => {
            let o = doc
                .occurrences
                .get_mut(&occurrence)
                .ok_or(CommandError::UnknownEntity(occurrence))?;
            match &mut o.placement {
                Placement::WallMounted { offset, .. } => stated(offset),
                _ => return Err(CommandError::NotADimension(occurrence)),
            }
        }
        DimensionTarget::NodeCoordinate { node, axis } => {
            let n = doc
                .nodes
                .get_mut(&node)
                .ok_or(CommandError::UnknownEntity(node))?;
            match axis {
                Axis::X => n.x = value,
                Axis::Y => n.y = value,
            }
        }
        DimensionTarget::OccurrenceCoordinate { occurrence, axis } => {
            let o = doc
                .occurrences
                .get_mut(&occurrence)
                .ok_or(CommandError::UnknownEntity(occurrence))?;
            match axis {
                Axis::X => o.center.x = value,
                Axis::Y => o.center.y = value,
            }
            match &mut o.placement {
                Placement::Floor { x, y, .. } | Placement::CeilingMounted { x, y } => match axis {
                    Axis::X => x.value = value,
                    Axis::Y => y.value = value,
                },
                Placement::WallMounted { .. } => {
                    return Err(CommandError::NotADimension(occurrence));
                }
            }
        }
    }
    Ok(())
}

/// The document after `command`, or why it cannot be applied. Pure: the
/// result depends only on the two arguments.
pub fn apply(document: &Document, command: &Command) -> Result<Document, CommandError> {
    let mut doc = document.clone();
    let new_id =
        |ordinal: u32| Uuid::derive(command.idempotency_key, "command", &ordinal.to_string());
    match &command.action {
        Action::CreateNode(args) => {
            let id = new_id(0);
            if doc.nodes.contains_key(&id) {
                return Err(CommandError::IdCollision(id));
            }
            doc.nodes.insert(
                id,
                Node {
                    x: args.x,
                    y: args.y,
                    join: JoinType::Isolated,
                },
            );
        }
        Action::SetStorey(args) => {
            let id = match args.storey {
                Some(id) if doc.storeys.contains_key(&id) => id,
                Some(id) => return Err(CommandError::UnknownEntity(id)),
                None => {
                    let id = new_id(0);
                    if doc.storeys.contains_key(&id) {
                        return Err(CommandError::IdCollision(id));
                    }
                    id
                }
            };
            doc.storeys.insert(
                id,
                Storey {
                    name: args.name.clone(),
                    elevation: args.elevation.clone(),
                    floor_to_floor: args.floor_to_floor.clone(),
                    default_ceiling_height: args.default_ceiling_height.clone(),
                },
            );
        }
        Action::SetVariantSelection(args) => {
            let slot = doc
                .variant_slots
                .get_mut(&args.slot)
                .ok_or(CommandError::UnknownEntity(args.slot))?;
            if !slot.options.contains(&args.selected) {
                return Err(CommandError::InvalidOption {
                    slot: args.slot,
                    option: args.selected,
                });
            }
            slot.selected = args.selected;
        }
        Action::SetProjectUnits(args) => doc.project.default_unit = args.default_unit,
        Action::SetConstraintValue(args) => set_dimension(&mut doc, args)?,
        Action::MoveWall(_) | Action::MoveOpening(_) | Action::MoveOccurrence(_) => {
            return Err(CommandError::Unresolved(command.action.kind()));
        }
        Action::CreateOpening(args) => {
            if !doc.walls.contains_key(&args.host) {
                return Err(CommandError::UnknownEntity(args.host));
            }
            let id = new_id(0);
            if doc.openings.contains_key(&id) {
                return Err(CommandError::IdCollision(id));
            }
            let defaults = crate::defaults::opening_size(args.kind);
            let or_default = |given: &Option<Sourced<Length>>, default: Length| {
                given.clone().unwrap_or(Sourced {
                    value: default,
                    source: Source::Proposed,
                })
            };
            doc.openings.insert(
                id,
                Opening {
                    kind: args.kind,
                    host: args.host,
                    reference: args.reference,
                    anchor: args.anchor,
                    offset: args.offset.clone(),
                    width: or_default(&args.width, defaults.width),
                    height: or_default(&args.height, defaults.height),
                    sill: or_default(&args.sill, defaults.sill),
                    start: Length::from_nanometers(0),
                },
            );
            // Place it: solve and store the new opening's start (Q-20).
            let solution = LayerA::default().solve(&doc).map_err(CommandError::Solve)?;
            if let (Some(start), Some(o)) = (solution.openings.get(&id), doc.openings.get_mut(&id))
            {
                o.start = *start;
            }
        }
        other => return Err(CommandError::NotYetImplemented(other.kind())),
    }
    Ok(doc)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::canonical::to_canonical_json;
    use crate::document::{Building, Project, Source, VariantKind, VariantSlot};

    pub(crate) fn uid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    pub(crate) fn genesis() -> Document {
        let mut doc = Document::new(
            Project {
                id: uid(1),
                name: "Shop".into(),
                default_unit: ProjectUnit::Inches,
            },
            Building {
                id: uid(2),
                name: "Main".into(),
            },
        );
        doc.variant_slots.insert(
            uid(10),
            VariantSlot {
                owner: uid(1),
                kind: VariantKind::Material,
                options: vec![uid(11), uid(12)],
                selected: uid(11),
            },
        );
        doc
    }

    pub(crate) fn cmd(n: u8, action: Action) -> Command {
        Command {
            id: Uuid::derive(uid(0xC0), "id", &n.to_string()),
            idempotency_key: Uuid::derive(uid(0xC0), "key", &n.to_string()),
            action,
        }
    }

    fn sourced(nm: i64) -> Sourced<Length> {
        Sourced {
            value: Length::from_nanometers(nm),
            source: Source::UserStated,
        }
    }

    pub(crate) fn set_storey(storey: Option<Uuid>, name: &str) -> Action {
        Action::SetStorey(SetStorey {
            storey,
            name: name.into(),
            elevation: sourced(0),
            floor_to_floor: sourced(3_048_000_000),
            default_ceiling_height: sourced(2_743_200_000),
        })
    }

    pub(crate) fn create_node(x: i64, y: i64) -> Action {
        Action::CreateNode(CreateNode {
            x: Length::from_nanometers(x),
            y: Length::from_nanometers(y),
        })
    }

    #[test]
    fn create_node_uses_the_derived_id() {
        let c = cmd(1, create_node(5, -7));
        let doc = apply(&genesis(), &c);
        let expected_id = Uuid::derive(c.idempotency_key, "command", "0");
        let node = doc
            .as_ref()
            .ok()
            .and_then(|d| d.nodes.get(&expected_id))
            .cloned();
        assert_eq!(
            node,
            Some(Node {
                x: Length::from_nanometers(5),
                y: Length::from_nanometers(-7),
                join: JoinType::Isolated,
            })
        );
        // The same command again collides with its own earlier result.
        let again = doc.and_then(|d| apply(&d, &c));
        assert_eq!(again, Err(CommandError::IdCollision(expected_id)));
    }

    #[test]
    fn set_storey_creates_then_updates() {
        let c = cmd(1, set_storey(None, "Ground"));
        let id = Uuid::derive(c.idempotency_key, "command", "0");
        let created = apply(&genesis(), &c);
        assert_eq!(
            created
                .as_ref()
                .map(|d| d.storeys.get(&id).map(|s| s.name.clone())),
            Ok(Some("Ground".to_string()))
        );
        let renamed = created.and_then(|d| apply(&d, &cmd(2, set_storey(Some(id), "Level 1"))));
        assert_eq!(
            renamed
                .as_ref()
                .map(|d| (d.storeys.len(), d.storeys.get(&id).map(|s| s.name.clone()))),
            Ok((1, Some("Level 1".to_string())))
        );
        assert_eq!(
            apply(&genesis(), &cmd(3, set_storey(Some(uid(99)), "Nope"))),
            Err(CommandError::UnknownEntity(uid(99)))
        );
    }

    #[test]
    fn set_variant_selection_checks_the_options() {
        let select = |slot, selected| {
            cmd(
                1,
                Action::SetVariantSelection(SetVariantSelection { slot, selected }),
            )
        };
        let doc = apply(&genesis(), &select(uid(10), uid(12)));
        assert_eq!(
            doc.map(|d| d.variant_slots.get(&uid(10)).map(|s| s.selected)),
            Ok(Some(uid(12)))
        );
        assert_eq!(
            apply(&genesis(), &select(uid(10), uid(13))),
            Err(CommandError::InvalidOption {
                slot: uid(10),
                option: uid(13)
            })
        );
        assert_eq!(
            apply(&genesis(), &select(uid(9), uid(12))),
            Err(CommandError::UnknownEntity(uid(9)))
        );
    }

    #[test]
    fn set_project_units() {
        let c = cmd(
            1,
            Action::SetProjectUnits(SetProjectUnits {
                default_unit: ProjectUnit::Feet,
            }),
        );
        assert_eq!(
            apply(&genesis(), &c).map(|d| d.project.default_unit),
            Ok(ProjectUnit::Feet)
        );
    }

    /// A document with one storey and a solved +X wall from (0,0) to (20 ft, 0).
    pub(crate) fn with_wall() -> (Document, Uuid, Uuid) {
        use crate::document::{
            Axis, Constraint, ConstraintKind, ConstraintStatus, Direction, Function, Layer,
            LayerSet, Wall, WallTop,
        };
        let mut doc = genesis();
        let ft = |n: i64| Length::from_nanometers(n * 304_800_000);
        let (a, b, wall, storey, layers) = (uid(20), uid(21), uid(22), uid(23), uid(24));
        doc.storeys.insert(
            storey,
            Storey {
                name: "Ground".into(),
                elevation: sourced(0),
                floor_to_floor: sourced(3_048_000_000),
                default_ceiling_height: sourced(2_743_200_000),
            },
        );
        doc.nodes.insert(
            a,
            Node {
                x: ft(0),
                y: ft(0),
                join: JoinType::FreeEnd,
            },
        );
        doc.nodes.insert(
            b,
            Node {
                x: ft(20),
                y: ft(0),
                join: JoinType::FreeEnd,
            },
        );
        doc.layer_sets.insert(
            layers,
            LayerSet {
                name: "Stud".into(),
                layers: vec![Layer {
                    material: uid(25),
                    thickness: sourced(114_300_000),
                    core: true,
                }],
            },
        );
        let mut labels = crate::identity::LabelAllocator::new();
        doc.walls.insert(
            wall,
            Wall {
                label: labels.allocate("W").unwrap_or_else(|_| unreachable!()),
                storey,
                node_a: a,
                node_b: b,
                direction: Direction::PosX,
                layer_set: layers,
                left_extent: sourced(57_150_000),
                base_offset: sourced(0),
                top: WallTop::Storey,
                join_priority: 0,
                function: Function::New,
            },
        );
        for (n, node, axis, value) in [
            (26, a, Axis::X, ft(0)),
            (27, a, Axis::Y, ft(0)),
            (28, b, Axis::X, ft(20)),
        ] {
            doc.constraints.insert(
                uid(n),
                Constraint {
                    kind: ConstraintKind::Fixed {
                        reference: Reference::Node { node },
                        axis,
                        value,
                    },
                    status: ConstraintStatus::Driving,
                    source: Source::UserStated,
                    provenance: None,
                },
            );
        }
        (doc, wall, a)
    }

    fn create_opening(kind: OpeningKind, host: Uuid, from: Uuid, sizes: Option<i64>) -> Action {
        Action::CreateOpening(CreateOpening {
            kind,
            host,
            reference: Reference::Node { node: from },
            anchor: OpeningEdge::Start,
            offset: sourced(1_219_200_000),
            width: sizes.map(sourced),
            height: sizes.map(sourced),
            sill: sizes.map(|_| sourced(0)),
        })
    }

    #[test]
    fn create_opening_applies_proposed_defaults() -> Result<(), CommandError> {
        let (doc, wall, a) = with_wall();
        let c = cmd(1, create_opening(OpeningKind::Window, wall, a, None));
        let id = Uuid::derive(c.idempotency_key, "command", "0");
        let doc = apply(&doc, &c)?;
        let o = doc.openings.get(&id).cloned();
        let defaults = crate::defaults::opening_size(OpeningKind::Window);
        let proposed = |value| Sourced {
            value,
            source: Source::Proposed,
        };
        assert_eq!(
            o.as_ref().map(|o| o.width.clone()),
            Some(proposed(defaults.width))
        );
        assert_eq!(
            o.as_ref().map(|o| o.height.clone()),
            Some(proposed(defaults.height))
        );
        assert_eq!(
            o.as_ref().map(|o| o.sill.clone()),
            Some(proposed(defaults.sill))
        );
        // Placed and solved: start is 4 ft from the wall's start node.
        assert_eq!(
            o.map(|o| o.start),
            Some(Length::from_nanometers(1_219_200_000))
        );
        Ok(())
    }

    #[test]
    fn create_opening_keeps_stated_sizes_and_checks_the_host() -> Result<(), CommandError> {
        let (doc, wall, a) = with_wall();
        let c = cmd(
            1,
            create_opening(OpeningKind::Door, wall, a, Some(762_000_000)),
        );
        let id = Uuid::derive(c.idempotency_key, "command", "0");
        let after = apply(&doc, &c)?;
        let o = after.openings.get(&id).cloned();
        assert_eq!(
            o.as_ref().map(|o| o.width.clone()),
            Some(sourced(762_000_000))
        );
        assert_eq!(o.map(|o| (o.kind, o.host)), Some((OpeningKind::Door, wall)));
        assert_eq!(
            apply(
                &doc,
                &cmd(2, create_opening(OpeningKind::Door, uid(99), a, None))
            ),
            Err(CommandError::UnknownEntity(uid(99)))
        );
        Ok(())
    }

    fn unimplemented_actions() -> Vec<Action> {
        let n = || NoArgs {};
        vec![
            Action::CreateWall(n()),
            Action::SplitWall(n()),
            Action::MergeWalls(n()),
            Action::SetWallLayers(n()),
            Action::PlaceOccurrence(n()),
            Action::RotateOccurrence(n()),
            Action::DeleteEntity(n()),
            Action::AddConstraint(n()),
            Action::RemoveConstraint(n()),
            Action::ApplyOperator(n()),
            Action::RemoveOperator(n()),
        ]
    }

    #[test]
    fn the_rest_of_the_v1_set_is_not_yet_implemented() {
        let actions = unimplemented_actions();
        assert_eq!(actions.len() + 9, 20, "8.2 lists 20 commands");
        for action in actions {
            let kind = action.kind();
            assert_eq!(
                apply(&genesis(), &cmd(1, action)),
                Err(CommandError::NotYetImplemented(kind))
            );
        }
    }

    #[test]
    fn wire_form_is_flat_and_round_trips() -> Result<(), String> {
        let c = cmd(1, create_node(1, 2));
        let text = to_canonical_json(&c).map_err(|e| e.to_string())?;
        assert_eq!(
            text,
            format!(
                r#"{{"args":{{"x":1,"y":2}},"id":"{}","idempotency_key":"{}","kind":"create_node"}}"#,
                c.id, c.idempotency_key
            )
        );
        let back: Command = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        assert_eq!(back, c);
        let empty = cmd(2, Action::DeleteEntity(NoArgs {}));
        let text = to_canonical_json(&empty).map_err(|e| e.to_string())?;
        assert!(text.contains(r#""args":{},"#), "{text}");
        let back: Command = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        assert_eq!(back, empty);
        Ok(())
    }

    #[test]
    fn wire_form_rejects_bad_commands() {
        let base = serde_json::json!({
            "id": uid(1), "idempotency_key": uid(2), "kind": "create_node", "args": {"x": 1, "y": 2}
        });
        let parse = |v: Value| serde_json::from_value::<Command>(v).is_ok();
        assert!(parse(base.clone()));
        let mut extra = base.clone();
        extra["extra"] = 1.into();
        assert!(!parse(extra));
        let mut kind = base.clone();
        kind["kind"] = "teleport".into();
        assert!(!parse(kind));
        let mut args = base.clone();
        args["args"]["z"] = 3.into();
        assert!(!parse(args));
        let mut missing = base;
        if let Some(o) = missing.as_object_mut() {
            o.remove("idempotency_key");
        }
        assert!(!parse(missing));
    }

    /// The canonical document after one fixed command. A golden file makes
    /// the result identical across runs, processes and machines.
    #[test]
    fn apply_result_golden() -> Result<(), String> {
        let doc = apply(&genesis(), &cmd(1, create_node(25_400_000, -12_700_000)))
            .map_err(|e| e.to_string())?;
        let text = to_canonical_json(&doc).map_err(|e| e.to_string())?;
        insta::assert_binary_snapshot!("create_node_result.json", text.into_bytes());
        Ok(())
    }

    #[test]
    fn same_command_twice_in_one_process_agrees() -> Result<(), String> {
        let c = cmd(1, set_storey(None, "Ground"));
        let a = apply(&genesis(), &c).map_err(|e| e.to_string())?;
        let b = apply(&genesis(), &c).map_err(|e| e.to_string())?;
        assert_eq!(
            to_canonical_json(&a).map_err(|e| e.to_string())?,
            to_canonical_json(&b).map_err(|e| e.to_string())?
        );
        Ok(())
    }
}
