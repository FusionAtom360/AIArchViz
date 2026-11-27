//! The project document: every entity of spec section 6 plus Roof (6.5),
//! Underlay (6.6), CameraBookmark (12.6) and the additions of Q-6.
//!
//! Conventions (Q-4 to Q-8):
//! - lengths are integer nanometers ([`Length`]);
//! - each entity kind is a map from [`Uuid`] to entity, sorted by id;
//! - every settable dimensional or design parameter is a [`Sourced`] value;
//! - derived values (solved node positions, join types, space boundaries,
//!   feature children, pitch-derived heights) are stored, not recomputed;
//! - elements name the storey that contains them (spatial hierarchy, 4.1),
//!   and their elevations and heights are measured from that storey's
//!   elevation;
//! - angles are [`QuarterTurns`] or [`Millidegrees`]; nothing is floating point;
//! - unknown fields are rejected when parsing.
//!
//! The canonical JSON form is written by `canonical::to_canonical_json`.

use std::borrow::Cow;
use std::collections::BTreeMap;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::identity::{Label, LabelAllocator, Uuid};
use crate::length::Length;
use crate::units::ProjectUnit;

/// The schema version this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// Where a value came from (spec 7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    UserStated,
    Measured,
    Derived,
    Proposed,
    Imported,
}

/// A value together with its source (Q-4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sourced<T> {
    pub value: T,
    pub source: Source,
}

/// A rotation in plan: a whole number of quarter turns counterclockwise
/// (D-5, Q-5). Serialized as the integer 0, 1, 2 or 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct QuarterTurns(u8);

impl QuarterTurns {
    /// `turns` quarter turns counterclockwise, if `turns` is 0 to 3.
    pub const fn new(turns: u8) -> Option<Self> {
        if turns < 4 {
            Some(QuarterTurns(turns))
        } else {
            None
        }
    }

    pub const fn turns(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for QuarterTurns {
    type Error = String;

    fn try_from(turns: u8) -> Result<Self, String> {
        QuarterTurns::new(turns).ok_or_else(|| format!("{turns} quarter turns is not 0 to 3"))
    }
}

impl From<QuarterTurns> for u8 {
    fn from(q: QuarterTurns) -> u8 {
        q.0
    }
}

impl JsonSchema for QuarterTurns {
    fn schema_name() -> Cow<'static, str> {
        "QuarterTurns".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "integer", "minimum": 0, "maximum": 3 })
    }
}

/// A heading in plan in thousandths of a degree, 0 = +X (east),
/// counterclockwise, in [0, 360000) (Q-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct Millidegrees(u32);

impl Millidegrees {
    /// One full turn.
    pub const TURN: u32 = 360_000;

    /// `value` millidegrees, if it is below one full turn.
    pub const fn new(value: u32) -> Option<Self> {
        if value < Self::TURN {
            Some(Millidegrees(value))
        } else {
            None
        }
    }

    pub const fn value(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for Millidegrees {
    type Error = String;

    fn try_from(value: u32) -> Result<Self, String> {
        Millidegrees::new(value).ok_or_else(|| format!("{value} millidegrees is not below 360000"))
    }
}

impl From<Millidegrees> for u32 {
    fn from(m: Millidegrees) -> u32 {
        m.0
    }
}

impl JsonSchema for Millidegrees {
    fn schema_name() -> Cow<'static, str> {
        "Millidegrees".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "integer", "minimum": 0, "maximum": 359_999 })
    }
}

/// A SHA-256 content hash written `sha256:` plus 64 lowercase hex digits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentHash(String);

impl ContentHash {
    /// The hash in its written form, if `text` is in that form.
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix("sha256:")?;
        let ok = hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        ok.then(|| ContentHash(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentHash {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        ContentHash::parse(&text).ok_or_else(|| format!("{text:?} is not sha256:<64 hex digits>"))
    }
}

impl From<ContentHash> for String {
    fn from(h: ContentHash) -> String {
        h.0
    }
}

impl JsonSchema for ContentHash {
    fn schema_name() -> Cow<'static, str> {
        "ContentHash".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "pattern": "^sha256:[0-9a-f]{64}$" })
    }
}

/// A point in plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: Length,
    pub y: Length,
}

/// A pixel position in an underlay image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PixelPoint {
    pub x: i64,
    pub y: i64,
}

/// A plan axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    X,
    Y,
}

/// A compass direction along a plan axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    PosX,
    NegX,
    PosY,
    NegY,
}

/// A side of a wall, looking from node_a to node_b (6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WallSide {
    Left,
    Right,
}

/// An edge or the center of an opening along its host wall.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OpeningEdge {
    Start,
    End,
    Center,
}

/// A side of an occurrence footprint, in the occurrence's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemSide {
    Front,
    Back,
    Left,
    Right,
}

/// A geometric reference that dimensions and constraints name (6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reference {
    Node {
        node: Uuid,
    },
    /// `boundary` counts layer boundaries from the `side` finish face
    /// inward: 0 is that face, the number of layers is the opposite face (Q-16).
    WallFace {
        wall: Uuid,
        side: WallSide,
        boundary: u32,
    },
    WallCenter {
        wall: Uuid,
    },
    OpeningEdge {
        opening: Uuid,
        edge: OpeningEdge,
    },
    ItemEdge {
        item: Uuid,
        side: ItemSide,
    },
    ItemCenter {
        item: Uuid,
    },
    Datum {
        datum: Uuid,
    },
}

/// Free-form property values: no decimals (Q-8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PropertyValue {
    String(String),
    Integer(i64),
    Boolean(bool),
    Length(Length),
}

/// Named properties, sorted by name.
pub type Properties = BTreeMap<String, PropertyValue>;

/// The project (IfcProject).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    /// The unit of a bare number typed by the user (5.2, Q-9).
    pub default_unit: ProjectUnit,
}

/// The building (IfcBuilding).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Building {
    pub id: Uuid,
    pub name: String,
}

/// A storey (IfcBuildingStorey).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Storey {
    pub name: String,
    pub elevation: Sourced<Length>,
    pub floor_to_floor: Sourced<Length>,
    pub default_ceiling_height: Sourced<Length>,
}

/// The shape of a datum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DatumGeometry {
    /// A point; anchors both axes.
    Point { x: Length, y: Length },
    /// The line x = constant; anchors the X axis.
    XLine { x: Length },
    /// The line y = constant; anchors the Y axis.
    YLine { y: Length },
}

/// A named point or line anchoring an axis (IfcGrid).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Datum {
    pub name: String,
    pub geometry: DatumGeometry,
}

/// How walls meet at a node (6.1). Derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JoinType {
    /// No wall ends here.
    Isolated,
    /// One wall ends here, square.
    FreeEnd,
    /// Two collinear walls meet end to end, square (Q-19).
    Straight,
    /// Two walls, mitered.
    L,
    /// Three walls; the lowest join_priority runs through.
    T,
    /// Four walls; the lowest join_priority runs through.
    X,
}

/// A polyline vertex. Position and join type are solved and stored (Q-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub x: Length,
    pub y: Length,
    pub join: JoinType,
}

/// Whether an element exists, is new work, or is to be removed (6.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Function {
    Existing,
    New,
    Demolished,
}

/// Where a wall's top is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WallTop {
    /// The storey's floor-to-floor height.
    Storey,
    /// An explicit height above the wall's base.
    Height { height: Sourced<Length> },
}

/// A wall (IfcWall).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Wall {
    pub label: Label,
    pub storey: Uuid,
    pub node_a: Uuid,
    pub node_b: Uuid,
    /// From node_a to node_b; set at creation, never changed by solving (Q-12).
    pub direction: Direction,
    pub layer_set: Uuid,
    /// From the reference line to the left face (6.1).
    pub left_extent: Sourced<Length>,
    pub base_offset: Sourced<Length>,
    pub top: WallTop,
    pub join_priority: u32,
    pub function: Function,
}

/// One material layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub material: Uuid,
    pub thickness: Sourced<Length>,
    pub core: bool,
}

/// Layers ordered left to right (IfcMaterialLayerSet).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerSet {
    pub name: String,
    pub layers: Vec<Layer>,
}

/// What fills an opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OpeningKind {
    Opening,
    Door,
    Window,
}

/// An opening, door or window in a host wall (IfcOpeningElement, IfcDoor, IfcWindow).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Opening {
    pub kind: OpeningKind,
    pub host: Uuid,
    /// The along-wall reference the opening is positioned from.
    pub reference: Reference,
    /// Which edge of the opening sits `offset` from `reference`.
    pub anchor: OpeningEdge,
    pub offset: Sourced<Length>,
    pub width: Sourced<Length>,
    pub height: Sourced<Length>,
    pub sill: Sourced<Length>,
}

/// A rectilinear boundary given explicitly or taken from a space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Boundary {
    Explicit { points: Vec<Point> },
    FromSpace { space: Uuid },
}

/// A floor slab (IfcSlab).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Slab {
    pub storey: Uuid,
    pub boundary: Boundary,
    pub thickness: Sourced<Length>,
    pub elevation: Sourced<Length>,
}

/// A ceiling (IfcCovering).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ceiling {
    pub storey: Uuid,
    pub boundary: Boundary,
    pub thickness: Sourced<Length>,
    pub elevation: Sourced<Length>,
}

/// Finish materials of a space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Finishes {
    pub floor: Option<Uuid>,
    pub walls: Option<Uuid>,
    pub ceiling: Option<Uuid>,
}

/// A space (IfcSpace). The boundary is derived and stored (Q-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Space {
    pub storey: Uuid,
    pub name: String,
    pub boundary: Vec<Point>,
    pub finishes: Finishes,
}

/// A material (Q-6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub name: String,
    pub asset: Option<Uuid>,
    /// Real-world size of one texture tile (12.4).
    pub tile_size: Option<Sourced<Length>>,
    pub properties: Properties,
}

/// Asset fidelity tiers (10.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    Exact,
    Library,
    Generated,
    Proxy,
}

/// Dimension authority, highest first (D-11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    UserStatedExact,
    ManufacturerSpec,
    CadDerived,
    MeasuredWithTolerance,
    ProposedDefault,
    MeshDerived,
}

/// Declared footprint and height of an element type (10.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclaredDimensions {
    pub width: Length,
    pub depth: Length,
    pub height: Length,
    pub authority: Authority,
}

/// A type of placed object (IfcTypeObject).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ElementType {
    pub name: String,
    pub declared_dimensions: DeclaredDimensions,
    pub fidelity: Fidelity,
    pub model: Option<Uuid>,
    pub properties: Properties,
    /// Where the dimensions came from, e.g. a manufacturer URL.
    pub source: String,
    pub license: String,
}

/// How an occurrence is placed (6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Placement {
    Floor {
        x: Sourced<Length>,
        y: Sourced<Length>,
        rotation: QuarterTurns,
        elevation: Sourced<Length>,
    },
    WallMounted {
        wall: Uuid,
        side: WallSide,
        reference: Reference,
        offset: Sourced<Length>,
        elevation: Sourced<Length>,
    },
    CeilingMounted {
        x: Sourced<Length>,
        y: Sourced<Length>,
    },
}

/// Clearance zone extents around a footprint (Q-8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Clearance {
    pub front: Sourced<Length>,
    pub back: Sourced<Length>,
    pub left: Sourced<Length>,
    pub right: Sourced<Length>,
}

/// Light emitter kinds (11.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    Point,
    Spot,
    Rect,
}

/// Light data of a light-fixture occurrence (Q-6, 11.3, 12.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Light {
    pub kind: LightKind,
    pub lumens: Sourced<u32>,
    pub kelvin: Sourced<u32>,
    pub ies: Option<Uuid>,
}

/// A placed object (IfcFurniture, IfcBuildingElementProxy, IfcLightFixture).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    pub label: Label,
    pub storey: Uuid,
    pub element_type: Uuid,
    pub placement: Placement,
    pub clearance: Clearance,
    /// Never moves as a consequence of another edit (6.3).
    pub fixed: bool,
    pub light: Option<Light>,
}

/// An operator instance and the children it derived (8.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub operator: String,
    pub version: u32,
    pub parameters: Properties,
    pub targets: Vec<Uuid>,
    pub children: Vec<Uuid>,
}

/// Driving or Reference (7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintStatus {
    Driving,
    Reference,
}

/// How a value entered beyond its source (6.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Read from the underlay of the paper plan.
    Transcribed,
}

/// Constraint kinds (7.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConstraintKind {
    /// coordinate(b) - coordinate(a) = value along `axis` (signed, Q-13).
    Distance {
        a: Reference,
        b: Reference,
        axis: Axis,
        value: Length,
        tolerance: Option<Length>,
    },
    Align {
        a: Reference,
        b: Reference,
        axis: Axis,
    },
    /// Two Distance constraints, by id, have equal values.
    Equal { first: Uuid, second: Uuid },
    /// The reference's coordinate on `axis` equals `value` (Q-13).
    Fixed {
        reference: Reference,
        axis: Axis,
        value: Length,
    },
    /// Checked after solving, not solved (7.1).
    MinClearance {
        object: Uuid,
        other: Uuid,
        value: Length,
    },
}

/// A constraint (IfcConstraint, weak).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    pub kind: ConstraintKind,
    pub status: ConstraintStatus,
    pub source: Source,
    pub provenance: Option<Provenance>,
}

/// What a variant slot swaps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VariantKind {
    Material,
    Model,
    Lighting,
}

/// A set of interchangeable options on an owner entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantSlot {
    pub owner: Uuid,
    pub kind: VariantKind,
    pub options: Vec<Uuid>,
    pub selected: Uuid,
}

/// An external file (IfcDocumentReference).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub content_hash: ContentHash,
    pub source_url: Option<String>,
    pub license: String,
    pub fidelity: Fidelity,
    pub converter_log: Vec<String>,
}

/// Roof pitch as rise over run, e.g. 3:12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pitch {
    pub rise: u32,
    pub run: u32,
}

/// Eave overhang per compass side, applied to every footprint edge facing
/// that way (Q-8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Overhang {
    pub pos_x: Sourced<Length>,
    pub neg_x: Sourced<Length>,
    pub pos_y: Sourced<Length>,
    pub neg_y: Sourced<Length>,
}

/// A shed roof (6.5). With a pitch, the derived height is stored with
/// source Derived (Q-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Roof {
    pub storey: Uuid,
    pub footprint: Boundary,
    /// The direction the roof slopes down; derived when a ledger is named.
    pub slope_direction: Sourced<Direction>,
    pub high_edge_height: Sourced<Length>,
    pub low_edge_height: Sourced<Length>,
    pub pitch: Option<Sourced<Pitch>>,
    /// A face of an existing wall where the high edge ends.
    pub ledger: Option<Reference>,
    pub overhang: Overhang,
    pub thickness: Sourced<Length>,
    pub fascia_depth: Sourced<Length>,
    /// Material per named slot, e.g. "roofing", "fascia", "soffit".
    pub material_slots: BTreeMap<String, Uuid>,
}

/// Two image points and the real distance between them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KnownDistance {
    pub pixel_a: PixelPoint,
    pub pixel_b: PixelPoint,
    pub distance: Sourced<Length>,
}

/// A second known distance and how far the calibration misses it. The
/// residual is derived and stored (Q-7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalibrationCheck {
    pub known: KnownDistance,
    pub residual: Length,
}

/// A calibrated raster underlay of the paper plan (6.6). Display only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Underlay {
    pub name: String,
    pub storey: Uuid,
    pub image: Uuid,
    /// Sets the scale.
    pub calibration: KnownDistance,
    /// Where `calibration.pixel_a` lies in the world.
    pub origin: Point,
    /// World heading of the direction from `pixel_a` to `pixel_b`.
    pub heading: Millidegrees,
    pub check: Option<CalibrationCheck>,
}

/// A named viewpoint (12.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CameraBookmark {
    pub name: String,
    pub storey: Uuid,
    pub position: Point,
    pub eye_height: Sourced<Length>,
    pub heading: Millidegrees,
}

/// The whole project document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub project: Project,
    pub building: Building,
    pub storeys: BTreeMap<Uuid, Storey>,
    pub datums: BTreeMap<Uuid, Datum>,
    pub nodes: BTreeMap<Uuid, Node>,
    pub walls: BTreeMap<Uuid, Wall>,
    pub layer_sets: BTreeMap<Uuid, LayerSet>,
    pub openings: BTreeMap<Uuid, Opening>,
    pub slabs: BTreeMap<Uuid, Slab>,
    pub ceilings: BTreeMap<Uuid, Ceiling>,
    pub spaces: BTreeMap<Uuid, Space>,
    pub materials: BTreeMap<Uuid, Material>,
    pub element_types: BTreeMap<Uuid, ElementType>,
    pub occurrences: BTreeMap<Uuid, Occurrence>,
    pub features: BTreeMap<Uuid, Feature>,
    pub constraints: BTreeMap<Uuid, Constraint>,
    pub variant_slots: BTreeMap<Uuid, VariantSlot>,
    pub assets: BTreeMap<Uuid, Asset>,
    pub roofs: BTreeMap<Uuid, Roof>,
    pub underlays: BTreeMap<Uuid, Underlay>,
    pub camera_bookmarks: BTreeMap<Uuid, CameraBookmark>,
    pub default_walk_start: Option<Uuid>,
    /// Label counters, so labels are never reused across save and load (5.5).
    pub labels: LabelAllocator,
}

impl Document {
    /// An empty document for a new project.
    pub fn new(project: Project, building: Building) -> Self {
        Document {
            schema_version: SCHEMA_VERSION,
            project,
            building,
            storeys: BTreeMap::new(),
            datums: BTreeMap::new(),
            nodes: BTreeMap::new(),
            walls: BTreeMap::new(),
            layer_sets: BTreeMap::new(),
            openings: BTreeMap::new(),
            slabs: BTreeMap::new(),
            ceilings: BTreeMap::new(),
            spaces: BTreeMap::new(),
            materials: BTreeMap::new(),
            element_types: BTreeMap::new(),
            occurrences: BTreeMap::new(),
            features: BTreeMap::new(),
            constraints: BTreeMap::new(),
            variant_slots: BTreeMap::new(),
            assets: BTreeMap::new(),
            roofs: BTreeMap::new(),
            underlays: BTreeMap::new(),
            camera_bookmarks: BTreeMap::new(),
            default_walk_start: None,
            labels: LabelAllocator::new(),
        }
    }
}
