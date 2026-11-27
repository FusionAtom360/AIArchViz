//! Scene Package writer (spec 11.3, 12.1; Q-57, Q-58, Q-59).
//!
//! A package is a folder holding:
//! - scene.json: canonical JSON (sorted keys, no whitespace) of [`Scene`],
//!   every length an integer count of nanometers in world coordinates
//!   (right-handed, X east, Y north, Z up); the runtime owns the mapping to
//!   its own frame (5.4);
//! - assets/<element id>.glb: one GLB per element with a mesh, in
//!   element-local coordinates; scene.json's `translation` places it;
//! - manifest.sha256: `<sha256 hex>  <path>` per file, sorted by path, in the
//!   format `sha256sum -c` checks; it does not list itself.
//!
//! [`write_scene_package`] is pure: the same document gives the same bytes.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use archviz_core::{
    Boundary, CanonicalError, ContentHash, Document, Fidelity, Length, LightKind,
    RequiredParameter, Source, Uuid, VariantKind, footprint, height_range, proposed_items,
    to_canonical_json, to_pretty_json,
};
use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::gltf::{self, Element, GltfError, P2, P3, Prism};

/// The scene.json format version.
pub const SCENE_SCHEMA_VERSION: u32 = 1;
pub const SCENE_FILE: &str = "scene.json";
pub const MANIFEST_FILE: &str = "manifest.sha256";

/// Why a Scene Package cannot be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneError {
    Gltf(GltfError),
    Canonical(CanonicalError),
    /// A model or IES reference names an asset the document does not hold.
    UnknownAsset(Uuid),
    UnknownStorey(Uuid),
    /// The target folder exists and is not empty.
    NotEmpty(String),
    Io(String),
    Overflow,
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneError::Gltf(e) => write!(f, "{e}"),
            SceneError::Canonical(e) => write!(f, "{e}"),
            SceneError::UnknownAsset(id) => write!(f, "unknown asset {id}"),
            SceneError::UnknownStorey(id) => write!(f, "unknown storey {id}"),
            SceneError::NotEmpty(p) => write!(f, "{p} exists and is not empty"),
            SceneError::Io(e) => f.write_str(e),
            SceneError::Overflow => f.write_str("a coordinate is out of range"),
        }
    }
}

impl std::error::Error for SceneError {}

impl From<GltfError> for SceneError {
    fn from(e: GltfError) -> Self {
        SceneError::Gltf(e)
    }
}

impl From<CanonicalError> for SceneError {
    fn from(e: CanonicalError) -> Self {
        SceneError::Canonical(e)
    }
}

/// The unit of every length in scene.json.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Nanometer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Handedness {
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Compass {
    East,
    North,
    Up,
}

/// The world frame (5.4): right-handed, X east, Y north, Z up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CoordinateSystem {
    pub handedness: Handedness,
    pub x: Compass,
    pub y: Compass,
    pub z: Compass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneProject {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneStorey {
    pub id: Uuid,
    pub name: String,
    pub elevation: Length,
    pub floor_to_floor: Length,
    pub default_ceiling_height: Length,
}

/// A Point datum: a labeled fiducial for conformance checks (S-3, Q-54).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fiducial {
    pub id: Uuid,
    pub name: String,
    pub x: Length,
    pub y: Length,
    pub z: Length,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Wall,
    Door,
    Window,
    Slab,
    Ceiling,
    Roof,
    Occurrence,
}

/// A collision solid: a counterclockwise plan outline between two heights.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CollisionPrism {
    pub outline: Vec<[Length; 2]>,
    pub z_min: Length,
    pub z_max: Length,
}

/// An asset by id and content hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetRef {
    pub asset: Uuid,
    pub content_hash: ContentHash,
}

/// What only occurrences carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OccurrenceInfo {
    pub element_type: Uuid,
    pub fidelity: Fidelity,
    pub fixed: bool,
    /// The type's model; the mesh in the package is still its proxy box
    /// until the asset library resolves models (Q-57).
    pub model: Option<AssetRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneElement {
    pub id: Uuid,
    pub kind: ElementKind,
    /// The human label (W-007, EQ-012) of walls and occurrences.
    pub label: Option<String>,
    pub name: String,
    pub storey: Uuid,
    /// The element's GLB in the package, e.g. "assets/<id>.glb".
    pub mesh: Option<String>,
    /// Where the mesh's local origin sits in the world: its lowest corner.
    pub translation: [Length; 3],
    /// Solids the runtime builds collision from (12.1). Empty for doors and
    /// windows, so their openings stay passable.
    pub collision: Vec<CollisionPrism>,
    /// Material per named slot.
    pub material_slots: BTreeMap<String, Uuid>,
    pub occurrence: Option<OccurrenceInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneVariantSlot {
    pub id: Uuid,
    pub owner: Uuid,
    pub kind: VariantKind,
    pub options: Vec<Uuid>,
    pub selected: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LightDirection {
    Down,
}

/// A physical light (12.4) at its fixture's footprint center and bottom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneLight {
    /// The fixture occurrence.
    pub id: Uuid,
    pub kind: LightKind,
    pub lumens: u32,
    pub kelvin: u32,
    pub ies: Option<AssetRef>,
    pub position: [Length; 3],
    /// Down for spot and rect lights; none for point lights.
    pub direction: Option<LightDirection>,
}

/// A named viewpoint (12.6); `z` is the absolute eye height.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SceneBookmark {
    pub id: Uuid,
    pub name: String,
    pub storey: Uuid,
    pub x: Length,
    pub y: Length,
    pub z: Length,
    /// Plan heading in thousandths of a degree, 0 = east, counterclockwise.
    pub heading_millidegrees: u32,
    /// Whether the eye height is still the Proposed default.
    pub eye_height_proposed: bool,
}

/// The content of scene.json (11.3, Q-59).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub schema_version: u32,
    pub unit: Unit,
    pub coordinate_system: CoordinateSystem,
    pub project: SceneProject,
    pub storeys: Vec<SceneStorey>,
    pub fiducials: Vec<Fiducial>,
    pub elements: Vec<SceneElement>,
    pub variant_slots: Vec<SceneVariantSlot>,
    pub lights: Vec<SceneLight>,
    /// Every value still at its Proposed default (7.4, 12.4).
    pub proposed_items: Vec<RequiredParameter>,
    pub camera_bookmarks: Vec<SceneBookmark>,
    pub default_walk_start: Option<Uuid>,
}

/// The JSON Schema of scene.json, pretty-printed with sorted keys and a
/// trailing newline: the content of schemas/scene.schema.json.
pub fn scene_schema_json() -> Result<String, CanonicalError> {
    to_pretty_json(&schemars::schema_for!(Scene))
}

fn length(v: i128) -> Result<Length, SceneError> {
    i64::try_from(v)
        .map(Length::from_nanometers)
        .map_err(|_| SceneError::Overflow)
}

fn point3(p: P3) -> Result<[Length; 3], SceneError> {
    Ok([length(p[0])?, length(p[1])?, length(p[2])?])
}

fn prism(p: &Prism) -> Result<CollisionPrism, SceneError> {
    Ok(CollisionPrism {
        outline: p
            .outline
            .iter()
            .map(|&(x, y): &P2| Ok([length(x)?, length(y)?]))
            .collect::<Result<_, SceneError>>()?,
        z_min: length(p.z0)?,
        z_max: length(p.z1)?,
    })
}

fn asset_ref(doc: &Document, id: Uuid) -> Result<AssetRef, SceneError> {
    let a = doc.assets.get(&id).ok_or(SceneError::UnknownAsset(id))?;
    Ok(AssetRef {
        asset: id,
        content_hash: a.content_hash.clone(),
    })
}

fn mesh_path(id: Uuid) -> String {
    format!("assets/{id}.glb")
}

/// The space a boundary follows, if any.
fn boundary_space(b: &Boundary) -> Option<Uuid> {
    match b {
        Boundary::FromSpace { space } => Some(*space),
        _ => None,
    }
}

/// The scene.json entry of one exported element.
fn element(doc: &Document, e: &Element) -> Result<Option<SceneElement>, SceneError> {
    let mut slots = BTreeMap::new();
    let mut occurrence = None;
    let mut label = None;
    let (kind, storey) = match e.kind {
        "wall" => {
            let Some(w) = doc.walls.get(&e.id) else {
                return Ok(None);
            };
            let layers = doc.layer_sets.get(&w.layer_set).map(|s| &s.layers);
            if let Some((first, last)) = layers.and_then(|l| l.first().zip(l.last())) {
                slots.insert("left".to_string(), first.material);
                slots.insert("right".to_string(), last.material);
            }
            label = Some(w.label.to_string());
            (ElementKind::Wall, w.storey)
        }
        "door" | "window" => {
            let Some(o) = doc.openings.get(&e.id) else {
                return Ok(None);
            };
            let storey = doc.walls.get(&o.host).map(|w| w.storey);
            let kind = if e.kind == "door" {
                ElementKind::Door
            } else {
                ElementKind::Window
            };
            (kind, storey.ok_or(SceneError::UnknownStorey(o.host))?)
        }
        "slab" => {
            let Some(s) = doc.slabs.get(&e.id) else {
                return Ok(None);
            };
            let finish = boundary_space(&s.boundary)
                .and_then(|id| doc.spaces.get(&id))
                .and_then(|sp| sp.finishes.floor);
            if let Some(m) = finish {
                slots.insert("floor".to_string(), m);
            }
            (ElementKind::Slab, s.storey)
        }
        "ceiling" => {
            let Some(c) = doc.ceilings.get(&e.id) else {
                return Ok(None);
            };
            let finish = boundary_space(&c.boundary)
                .and_then(|id| doc.spaces.get(&id))
                .and_then(|sp| sp.finishes.ceiling);
            if let Some(m) = finish {
                slots.insert("ceiling".to_string(), m);
            }
            (ElementKind::Ceiling, c.storey)
        }
        "roof" => {
            let Some(r) = doc.roofs.get(&e.id) else {
                return Ok(None);
            };
            slots.extend(r.material_slots.iter().map(|(k, v)| (k.clone(), *v)));
            (ElementKind::Roof, r.storey)
        }
        "occurrence" => {
            let Some(o) = doc.occurrences.get(&e.id) else {
                return Ok(None);
            };
            let t = doc.element_types.get(&o.element_type);
            let model = match t.and_then(|t| t.model) {
                Some(m) => Some(asset_ref(doc, m)?),
                None => None,
            };
            occurrence = Some(OccurrenceInfo {
                element_type: o.element_type,
                fidelity: t.map_or(Fidelity::Proxy, |t| t.fidelity),
                fixed: o.fixed,
                model,
            });
            label = Some(o.label.to_string());
            (ElementKind::Occurrence, o.storey)
        }
        _ => return Ok(None),
    };
    Ok(Some(SceneElement {
        id: e.id,
        kind,
        label,
        name: e.name.clone(),
        storey,
        mesh: (!e.faces.is_empty()).then(|| mesh_path(e.id)),
        translation: point3(e.origin())?,
        collision: e.prisms.iter().map(prism).collect::<Result<_, _>>()?,
        material_slots: slots,
        occurrence,
    }))
}

fn lights(doc: &Document) -> Result<Vec<SceneLight>, SceneError> {
    let mut out = Vec::new();
    for (id, o) in &doc.occurrences {
        let Some(l) = &o.light else { continue };
        let f = footprint(doc, *id).map_err(GltfError::Geometry)?;
        let (z0, _) = height_range(doc, *id).map_err(GltfError::Geometry)?;
        let mid = |a: Length, b: Length| {
            // Footprints are centered on the solved center, so this is exact
            // except for a half nanometer, which rounds toward the minimum.
            length((i128::from(a.nanometers()) + i128::from(b.nanometers())).div_euclid(2))
        };
        out.push(SceneLight {
            id: *id,
            kind: l.kind,
            lumens: l.lumens.value,
            kelvin: l.kelvin.value,
            ies: match l.ies {
                Some(a) => Some(asset_ref(doc, a)?),
                None => None,
            },
            position: [mid(f.min.x, f.max.x)?, mid(f.min.y, f.max.y)?, z0],
            direction: match l.kind {
                LightKind::Point => None,
                LightKind::Spot | LightKind::Rect => Some(LightDirection::Down),
            },
        });
    }
    Ok(out)
}

/// The scene description and the element GLBs, by package path.
fn scene_and_assets(doc: &Document) -> Result<(Scene, BTreeMap<String, Vec<u8>>), SceneError> {
    let all = gltf::elements(doc)?;
    let mut fiducials = Vec::new();
    let mut elements = Vec::new();
    let mut assets = BTreeMap::new();
    for e in &all {
        if e.kind == "datum" {
            let [x, y, z] = point3(e.origin())?;
            fiducials.push(Fiducial {
                id: e.id,
                name: e.name.clone(),
                x,
                y,
                z,
            });
            continue;
        }
        let Some(se) = element(doc, e)? else { continue };
        if let Some(path) = &se.mesh {
            assets.insert(path.clone(), gltf::element_glb(e)?);
        }
        elements.push(se);
    }
    elements.sort_by_key(|e| (e.kind, e.id));
    let storeys = doc
        .storeys
        .iter()
        .map(|(id, s)| SceneStorey {
            id: *id,
            name: s.name.clone(),
            elevation: s.elevation.value,
            floor_to_floor: s.floor_to_floor.value,
            default_ceiling_height: s.default_ceiling_height.value,
        })
        .collect();
    let camera_bookmarks = doc
        .camera_bookmarks
        .iter()
        .map(|(id, b)| {
            let level = doc
                .storeys
                .get(&b.storey)
                .ok_or(SceneError::UnknownStorey(b.storey))?;
            Ok(SceneBookmark {
                id: *id,
                name: b.name.clone(),
                storey: b.storey,
                x: b.position.x,
                y: b.position.y,
                z: length(
                    i128::from(level.elevation.value.nanometers())
                        + i128::from(b.eye_height.value.nanometers()),
                )?,
                heading_millidegrees: b.heading.value(),
                eye_height_proposed: b.eye_height.source == Source::Proposed,
            })
        })
        .collect::<Result<_, SceneError>>()?;
    let scene = Scene {
        schema_version: SCENE_SCHEMA_VERSION,
        unit: Unit::Nanometer,
        coordinate_system: CoordinateSystem {
            handedness: Handedness::Right,
            x: Compass::East,
            y: Compass::North,
            z: Compass::Up,
        },
        project: SceneProject {
            id: doc.project.id,
            name: doc.project.name.clone(),
        },
        storeys,
        fiducials,
        elements,
        variant_slots: doc
            .variant_slots
            .iter()
            .map(|(id, v)| SceneVariantSlot {
                id: *id,
                owner: v.owner,
                kind: v.kind,
                options: v.options.clone(),
                selected: v.selected,
            })
            .collect(),
        lights: lights(doc)?,
        proposed_items: proposed_items(doc)?,
        camera_bookmarks,
        default_walk_start: doc.default_walk_start,
    };
    Ok((scene, assets))
}

/// The scene.json description of `document`.
pub fn scene(document: &Document) -> Result<Scene, SceneError> {
    Ok(scene_and_assets(document)?.0)
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Every file of the Scene Package for `document`, by path relative to the
/// package folder (forward slashes). The same document gives the same bytes.
pub fn write_scene_package(document: &Document) -> Result<BTreeMap<String, Vec<u8>>, SceneError> {
    let (scene, mut files) = scene_and_assets(document)?;
    files.insert(
        SCENE_FILE.to_string(),
        to_canonical_json(&scene)?.into_bytes(),
    );
    let manifest: String = files
        .iter()
        .map(|(path, bytes)| format!("{}  {path}\n", sha256_hex(bytes)))
        .collect();
    files.insert(MANIFEST_FILE.to_string(), manifest.into_bytes());
    Ok(files)
}

/// Writes the Scene Package for `document` into `dir`, which must not exist
/// or be empty, so no file from an earlier export is left behind.
pub fn write_scene_package_dir(document: &Document, dir: &Path) -> Result<(), SceneError> {
    let files = write_scene_package(document)?;
    let io = |p: &Path, e: std::io::Error| SceneError::Io(format!("{}: {e}", p.display()));
    if dir.exists() {
        let mut entries = std::fs::read_dir(dir).map_err(|e| io(dir, e))?;
        if entries.next().is_some() {
            return Err(SceneError::NotEmpty(dir.display().to_string()));
        }
    }
    for (path, bytes) in &files {
        let target = path.split('/').fold(dir.to_path_buf(), |p, s| p.join(s));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
        }
        std::fs::write(&target, bytes).map_err(|e| io(&target, e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
