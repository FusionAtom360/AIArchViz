//! Canonical JSON for the document (spec 5.5).
//!
//! The canonical form is UTF-8 JSON with object keys sorted by Unicode code
//! point, no insignificant whitespace, integers only (any non-integer number
//! is an error), `null` for absent optional values, and strings escaped as
//! `serde_json` escapes them (`"`, `\`, control characters; everything else
//! literal). The writer sorts keys itself, so the output never depends on
//! how a map was built or on serde_json's feature flags.

use std::fmt;

use serde::Serialize;
use serde_json::Value;

use crate::document::{Document, SCHEMA_VERSION};

/// Why a document could not be written or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalError {
    /// A value could not be converted to or from JSON.
    Json(String),
    /// A number that is not an integer.
    NonInteger(String),
    /// The document has no schema_version, or one this build does not read.
    UnsupportedVersion(Option<u64>),
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CanonicalError::Json(e) => write!(f, "invalid document JSON: {e}"),
            CanonicalError::NonInteger(n) => write!(f, "number {n} is not an integer"),
            CanonicalError::UnsupportedVersion(Some(v)) => {
                write!(
                    f,
                    "schema_version {v} is not supported (expected {SCHEMA_VERSION})"
                )
            }
            CanonicalError::UnsupportedVersion(None) => f.write_str("schema_version is missing"),
        }
    }
}

impl std::error::Error for CanonicalError {}

/// The canonical JSON text of `value`: a document, a log entry or a snapshot.
pub fn to_canonical_json<T: Serialize>(value: &T) -> Result<String, CanonicalError> {
    let value = serde_json::to_value(value).map_err(|e| CanonicalError::Json(e.to_string()))?;
    let mut out = String::new();
    write_value(&value, None, 0, &mut out)?;
    Ok(out)
}

/// Parses a document, rejecting unknown fields and other schema versions.
pub fn from_canonical_json(text: &str) -> Result<Document, CanonicalError> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| CanonicalError::Json(e.to_string()))?;
    let version = value.get("schema_version").and_then(Value::as_u64);
    if version != Some(u64::from(SCHEMA_VERSION)) {
        return Err(CanonicalError::UnsupportedVersion(version));
    }
    // Deserialize from the text, not the Value, so serde reports positions.
    serde_json::from_str(text).map_err(|e| CanonicalError::Json(e.to_string()))
}

/// `value` pretty-printed with sorted keys, two-space indents and a
/// trailing newline: for files people read, such as schemas.
pub fn to_pretty_json<T: Serialize>(value: &T) -> Result<String, CanonicalError> {
    let value = serde_json::to_value(value).map_err(|e| CanonicalError::Json(e.to_string()))?;
    let mut out = String::new();
    write_value(&value, Some(2), 0, &mut out)?;
    out.push('\n');
    Ok(out)
}

/// The JSON Schema of the document, pretty-printed with sorted keys and a
/// trailing newline: the content of schemas/document.schema.json.
pub fn document_schema_json() -> Result<String, CanonicalError> {
    let schema = schemars::schema_for!(Document);
    let value = serde_json::to_value(&schema).map_err(|e| CanonicalError::Json(e.to_string()))?;
    let mut out = String::new();
    write_value(&value, Some(2), 0, &mut out)?;
    out.push('\n');
    Ok(out)
}

/// Writes `value` canonically (no indent) or pretty (`indent` spaces per level).
fn write_value(
    value: &Value,
    indent: Option<usize>,
    depth: usize,
    out: &mut String,
) -> Result<(), CanonicalError> {
    let newline = |out: &mut String, depth: usize| {
        if let Some(step) = indent {
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', step * depth));
        }
    };
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push_str(&i.to_string());
            } else if let Some(u) = n.as_u64() {
                out.push_str(&u.to_string());
            } else {
                return Err(CanonicalError::NonInteger(n.to_string()));
            }
        }
        Value::String(s) => write_string(s, out)?,
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_value(item, indent, depth + 1, out)?;
            }
            if !items.is_empty() {
                newline(out, depth);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            out.push('{');
            for (i, (key, item)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_string(key, out)?;
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_value(item, indent, depth + 1, out)?;
            }
            if !entries.is_empty() {
                newline(out, depth);
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_string(s: &str, out: &mut String) -> Result<(), CanonicalError> {
    let quoted = serde_json::to_string(s).map_err(|e| CanonicalError::Json(e.to_string()))?;
    out.push_str(&quoted);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::*;
    use crate::identity::{LabelAllocator, Uuid};
    use crate::length::Length;
    use proptest::prelude::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    const IN: i64 = 25_400_000;
    const FT: i64 = 12 * IN;

    fn len(nm: i64) -> Length {
        Length::from_nanometers(nm)
    }

    fn src<T>(value: T, source: Source) -> Sourced<T> {
        Sourced { value, source }
    }

    fn user(nm: i64) -> Sourced<Length> {
        src(len(nm), Source::UserStated)
    }

    fn proposed(nm: i64) -> Sourced<Length> {
        src(len(nm), Source::Proposed)
    }

    /// Deterministic ids for the fixture: UUIDv5 of a fixed root.
    fn id(kind: &str, n: u32) -> Uuid {
        let root = Uuid::from_bytes([0x11; 16]);
        Uuid::derive(root, kind, &n.to_string())
    }

    fn quarter(turns: u8) -> QuarterTurns {
        QuarterTurns::new(turns).unwrap_or_else(|| unreachable!())
    }

    fn heading(md: u32) -> Millidegrees {
        Millidegrees::new(md).unwrap_or_else(|| unreachable!())
    }

    fn hash(byte: char) -> ContentHash {
        ContentHash::parse(&format!("sha256:{}", byte.to_string().repeat(64)))
            .unwrap_or_else(|| unreachable!())
    }

    /// A small document touching every entity kind: one 20 ft wall with a
    /// door, a lean-to roof, a light, an underlay and a bookmark.
    fn small_document() -> Document {
        let mut doc = Document::new(
            Project {
                id: id("project", 0),
                name: "Shop".into(),
                default_unit: crate::units::ProjectUnit::Inches,
            },
            Building {
                id: id("building", 0),
                name: "Main \"shop\" building".into(),
            },
        );
        let mut labels = LabelAllocator::new();
        let mut label = |p: &str| labels.allocate(p).unwrap_or_else(|_| unreachable!());

        let storey = id("storey", 1);
        doc.storeys.insert(
            storey,
            Storey {
                name: "Ground".into(),
                elevation: user(0),
                floor_to_floor: user(10 * FT),
                default_ceiling_height: proposed(9 * FT),
            },
        );
        doc.datums.insert(
            id("datum", 1),
            Datum {
                name: "Origin".into(),
                geometry: DatumGeometry::Point {
                    x: len(0),
                    y: len(0),
                },
            },
        );
        doc.datums.insert(
            id("datum", 2),
            Datum {
                name: "Grid A".into(),
                geometry: DatumGeometry::XLine { x: len(-3 * IN) },
            },
        );
        let (na, nb) = (id("node", 1), id("node", 2));
        doc.nodes.insert(
            na,
            Node {
                x: len(0),
                y: len(0),
                join: JoinType::FreeEnd,
            },
        );
        doc.nodes.insert(
            nb,
            Node {
                x: len(20 * FT),
                y: len(0),
                join: JoinType::FreeEnd,
            },
        );

        let (gyp, stud, paint) = (id("material", 1), id("material", 2), id("material", 3));
        for (m, name) in [
            (gyp, "Gypsum board"),
            (stud, "Wood stud"),
            (paint, "Eggshell white"),
        ] {
            doc.materials.insert(
                m,
                Material {
                    name: name.into(),
                    asset: None,
                    tile_size: None,
                    properties: BTreeMap::new(),
                },
            );
        }
        doc.materials.entry(paint).and_modify(|m| {
            m.tile_size = Some(proposed(2 * FT));
            m.properties
                .insert("finish".into(), PropertyValue::String("eggshell".into()));
            m.properties
                .insert("coats".into(), PropertyValue::Integer(2));
            m.properties
                .insert("low_voc".into(), PropertyValue::Boolean(true));
        });

        let layers = id("layer_set", 1);
        let half_inch = IN / 2;
        let stud_depth = 3 * IN + IN / 2;
        doc.layer_sets.insert(
            layers,
            LayerSet {
                name: "Interior stud, gypsum both sides".into(),
                layers: vec![
                    Layer {
                        material: gyp,
                        thickness: proposed(half_inch),
                        core: false,
                    },
                    Layer {
                        material: stud,
                        thickness: proposed(stud_depth),
                        core: true,
                    },
                    Layer {
                        material: gyp,
                        thickness: proposed(half_inch),
                        core: false,
                    },
                ],
            },
        );
        let wall = id("wall", 1);
        doc.walls.insert(
            wall,
            Wall {
                label: label("W"),
                storey,
                node_a: na,
                node_b: nb,
                direction: Direction::PosX,
                layer_set: layers,
                left_extent: src(len(half_inch + stud_depth / 2), Source::Derived),
                base_offset: user(0),
                top: WallTop::Storey,
                join_priority: 0,
                function: Function::Existing,
            },
        );
        let door = id("opening", 1);
        doc.openings.insert(
            door,
            Opening {
                kind: OpeningKind::Door,
                host: wall,
                reference: Reference::Node { node: na },
                anchor: OpeningEdge::Start,
                offset: user(4 * FT),
                start: len(4 * FT),
                width: proposed(3 * FT),
                height: proposed(6 * FT + 8 * IN),
                sill: user(0),
            },
        );
        let room = id("space", 1);
        let rect = vec![
            Point {
                x: len(0),
                y: len(0),
            },
            Point {
                x: len(20 * FT),
                y: len(0),
            },
            Point {
                x: len(20 * FT),
                y: len(12 * FT),
            },
            Point {
                x: len(0),
                y: len(12 * FT),
            },
        ];
        doc.spaces.insert(
            room,
            Space {
                storey,
                name: "Shop floor".into(),
                seed: Point {
                    x: len(10 * FT),
                    y: len(6 * FT),
                },
                boundary: rect.clone(),
                finishes: Finishes {
                    floor: None,
                    walls: Some(paint),
                    ceiling: None,
                },
            },
        );
        doc.slabs.insert(
            id("slab", 1),
            Slab {
                storey,
                boundary: Boundary::FromSpace { space: room },
                thickness: proposed(4 * IN),
                elevation: user(0),
            },
        );
        doc.ceilings.insert(
            id("ceiling", 1),
            Ceiling {
                storey,
                boundary: Boundary::Explicit { points: rect },
                thickness: proposed(half_inch),
                elevation: proposed(9 * FT),
            },
        );

        let photo = id("asset", 1);
        let ies = id("asset", 2);
        doc.assets.insert(
            photo,
            Asset {
                content_hash: hash('a'),
                source_url: None,
                license: "client-provided".into(),
                fidelity: Fidelity::Proxy,
                converter_log: vec![],
            },
        );
        doc.assets.insert(
            ies,
            Asset {
                content_hash: hash('b'),
                source_url: Some("https://example.com/fixtures/shop-light.ies".into()),
                license: "CC0-1.0".into(),
                fidelity: Fidelity::Exact,
                converter_log: vec!["validated: 1 photometric web".into()],
            },
        );
        let bench_type = id("element_type", 1);
        let light_type = id("element_type", 2);
        doc.element_types.insert(
            bench_type,
            ElementType {
                name: "Workbench 72 x 30".into(),
                declared_dimensions: DeclaredDimensions {
                    width: len(72 * IN),
                    depth: len(30 * IN),
                    height: len(34 * IN),
                    authority: Authority::UserStatedExact,
                },
                fidelity: Fidelity::Proxy,
                model: None,
                properties: BTreeMap::from([(
                    "top_thickness".into(),
                    PropertyValue::Length(len(IN + IN / 2)),
                )]),
                source: "client tape measure".into(),
                license: "n/a".into(),
            },
        );
        doc.element_types.insert(
            light_type,
            ElementType {
                name: "LED shop light 4 ft".into(),
                declared_dimensions: DeclaredDimensions {
                    width: len(48 * IN),
                    depth: len(3 * IN),
                    height: len(2 * IN),
                    authority: Authority::ManufacturerSpec,
                },
                fidelity: Fidelity::Proxy,
                model: None,
                properties: BTreeMap::new(),
                source: "https://example.com/spec-sheet".into(),
                license: "n/a".into(),
            },
        );
        let bench = id("occurrence", 1);
        let clearance = |front| Clearance {
            front: proposed(front),
            back: proposed(0),
            left: proposed(0),
            right: proposed(0),
        };
        doc.occurrences.insert(
            bench,
            Occurrence {
                label: label("EQ"),
                storey,
                element_type: bench_type,
                placement: Placement::Floor {
                    x: src(len(10 * FT), Source::Derived),
                    y: src(len(2 * FT), Source::Derived),
                    rotation: quarter(2),
                    elevation: user(0),
                },
                clearance: clearance(36 * IN),
                fixed: false,
                overlap_allowed: false,
                center: Point {
                    x: len(10 * FT),
                    y: len(2 * FT),
                },
                light: None,
            },
        );
        doc.occurrences.insert(
            id("occurrence", 2),
            Occurrence {
                label: label("L"),
                storey,
                element_type: light_type,
                placement: Placement::CeilingMounted {
                    x: user(10 * FT),
                    y: user(6 * FT),
                },
                clearance: clearance(0),
                fixed: false,
                overlap_allowed: false,
                center: Point {
                    x: len(10 * FT),
                    y: len(6 * FT),
                },
                light: Some(Light {
                    kind: LightKind::Rect,
                    lumens: src(5000, Source::Imported),
                    kelvin: src(4000, Source::Imported),
                    ies: Some(ies),
                }),
            },
        );
        doc.occurrences.insert(
            id("occurrence", 3),
            Occurrence {
                label: label("EQ"),
                storey,
                element_type: bench_type,
                placement: Placement::WallMounted {
                    wall,
                    side: WallSide::Left,
                    reference: Reference::OpeningEdge {
                        opening: door,
                        edge: OpeningEdge::End,
                    },
                    offset: user(2 * FT),
                    elevation: user(4 * FT),
                },
                clearance: clearance(0),
                fixed: true,
                overlap_allowed: false,
                center: Point {
                    // 2 ft past the door's end edge (4 ft + 3 ft); back on the
                    // left face (2 1/4 in), half the 30 in depth out from it.
                    x: len(9 * FT),
                    y: len(IN / 2 + (3 * IN + IN / 2) / 2 + 15 * IN),
                },
                light: None,
            },
        );
        let d1 = id("constraint", 1);
        let d2 = id("constraint", 2);
        doc.constraints.insert(
            d1,
            Constraint {
                kind: ConstraintKind::Distance {
                    a: Reference::Datum {
                        datum: id("datum", 1),
                    },
                    b: Reference::WallFace {
                        wall,
                        side: WallSide::Left,
                        boundary: 0,
                    },
                    axis: Axis::Y,
                    value: len(0),
                    tolerance: None,
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
        doc.constraints.insert(
            d2,
            Constraint {
                kind: ConstraintKind::Distance {
                    a: Reference::Node { node: na },
                    b: Reference::Node { node: nb },
                    axis: Axis::X,
                    value: len(20 * FT),
                    tolerance: Some(len(IN / 4)),
                },
                status: ConstraintStatus::Driving,
                source: Source::Measured,
                provenance: Some(Provenance::Transcribed),
            },
        );
        doc.constraints.insert(
            id("constraint", 3),
            Constraint {
                kind: ConstraintKind::Equal {
                    first: d1,
                    second: d2,
                },
                status: ConstraintStatus::Reference,
                source: Source::Proposed,
                provenance: None,
            },
        );
        doc.constraints.insert(
            id("constraint", 4),
            Constraint {
                kind: ConstraintKind::Align {
                    a: Reference::ItemCenter { item: bench },
                    b: Reference::WallCenter { wall },
                    axis: Axis::X,
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
        doc.constraints.insert(
            id("constraint", 5),
            Constraint {
                kind: ConstraintKind::Fixed {
                    reference: Reference::ItemEdge {
                        item: bench,
                        side: ItemSide::Back,
                    },
                    axis: Axis::Y,
                    value: len(IN),
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
        doc.constraints.insert(
            id("constraint", 6),
            Constraint {
                kind: ConstraintKind::MinClearance {
                    object: bench,
                    other: wall,
                    value: len(36 * IN),
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
        doc.features.insert(
            id("feature", 1),
            Feature {
                operator: "baseboard".into(),
                version: 1,
                parameters: BTreeMap::from([("height".into(), PropertyValue::Length(len(4 * IN)))]),
                targets: vec![wall],
                children: vec![Uuid::derive(wall, "baseboard", "left")],
            },
        );
        doc.variant_slots.insert(
            id("variant_slot", 1),
            VariantSlot {
                owner: wall,
                kind: VariantKind::Material,
                options: vec![paint, gyp],
                selected: paint,
            },
        );
        doc.roofs.insert(
            id("roof", 1),
            Roof {
                storey,
                footprint: Boundary::FromSpace { space: room },
                slope_direction: src(Direction::PosY, Source::Derived),
                high_edge_height: user(10 * FT),
                low_edge_height: Some(src(len(7 * FT), Source::Derived)),
                pitch: Some(src(Pitch { rise: 3, run: 12 }, Source::UserStated)),
                ledger: Some(Reference::WallFace {
                    wall,
                    side: WallSide::Left,
                    boundary: 0,
                }),
                overhang: Overhang {
                    pos_x: proposed(12 * IN),
                    neg_x: proposed(12 * IN),
                    pos_y: proposed(18 * IN),
                    neg_y: proposed(0),
                },
                thickness: proposed(8 * IN),
                fascia_depth: proposed(6 * IN),
                material_slots: BTreeMap::from([("roofing".into(), paint)]),
            },
        );
        doc.underlays.insert(
            id("underlay", 1),
            Underlay {
                name: "Paper plan scan".into(),
                storey,
                image: photo,
                calibration: KnownDistance {
                    pixel_a: PixelPoint { x: 120, y: 88 },
                    pixel_b: PixelPoint { x: 2520, y: 88 },
                    distance: src(len(20 * FT), Source::Measured),
                },
                origin: Point {
                    x: len(0),
                    y: len(0),
                },
                heading: heading(0),
                check: Some(CalibrationCheck {
                    known: KnownDistance {
                        pixel_a: PixelPoint { x: 120, y: 88 },
                        pixel_b: PixelPoint { x: 120, y: 1528 },
                        distance: src(len(12 * FT), Source::Measured),
                    },
                    residual: len(-IN / 8),
                }),
            },
        );
        let start = id("camera_bookmark", 1);
        doc.camera_bookmarks.insert(
            start,
            CameraBookmark {
                name: "Front door".into(),
                storey,
                position: Point {
                    x: len(5 * FT),
                    y: len(-2 * FT),
                },
                eye_height: proposed(5 * FT + 4 * IN),
                heading: heading(90_000),
            },
        );
        doc.default_walk_start = Some(start);
        doc.acknowledged.insert(ViolationKey {
            kind: ViolationKind::OpeningOutsideHost,
            entities: vec![door],
        });
        doc.labels = labels;
        doc
    }

    fn canonical(v: &Value) -> Result<String, CanonicalError> {
        let mut out = String::new();
        write_value(v, None, 0, &mut out)?;
        Ok(out)
    }

    #[test]
    fn small_document_golden() -> Result<(), CanonicalError> {
        let text = to_canonical_json(&small_document())?;
        insta::assert_binary_snapshot!("small_document.json", text.into_bytes());
        Ok(())
    }

    #[test]
    fn writer_sorts_keys_and_drops_whitespace() {
        let v = json!({"b": [1, {"z": null, "a": true}], "a": "x", "A": -2, "é": 0});
        assert_eq!(
            canonical(&v),
            Ok(r#"{"A":-2,"a":"x","b":[1,{"a":true,"z":null}],"é":0}"#.to_string())
        );
    }

    #[test]
    fn writer_escapes_strings_like_serde_json() {
        let v = json!({"s": "quote \" backslash \\ tab \t nul \u{0} é ☃"});
        assert_eq!(
            canonical(&v),
            Ok(r#"{"s":"quote \" backslash \\ tab \t nul \u0000 é ☃"}"#.to_string())
        );
    }

    #[test]
    fn writer_rejects_non_integers() {
        for v in [json!(1.5), json!({"a": [0.25]}), json!(1e3)] {
            assert!(
                matches!(canonical(&v), Err(CanonicalError::NonInteger(_))),
                "{v}"
            );
        }
        assert_eq!(canonical(&json!(i64::MIN)), Ok(i64::MIN.to_string()));
        assert_eq!(canonical(&json!(u64::MAX)), Ok(u64::MAX.to_string()));
    }

    #[test]
    fn pretty_writer_indents_and_sorts() {
        let v = json!({"b": [1, 2], "a": {}, "c": []});
        let mut out = String::new();
        assert_eq!(write_value(&v, Some(2), 0, &mut out), Ok(()));
        assert_eq!(
            out,
            "{\n  \"a\": {},\n  \"b\": [\n    1,\n    2\n  ],\n  \"c\": []\n}"
        );
    }

    const GOLDEN: &str =
        include_str!("snapshots/archviz_core__canonical__tests__small_document.snap.json");

    #[test]
    fn parse_then_write_is_byte_identical() -> Result<(), CanonicalError> {
        let doc = from_canonical_json(GOLDEN)?;
        assert_eq!(doc, small_document());
        assert_eq!(to_canonical_json(&doc)?, GOLDEN);
        Ok(())
    }

    const SCHEMA_FILE: &str = include_str!("../../../schemas/document.schema.json");

    #[test]
    fn schema_file_is_current() -> Result<(), CanonicalError> {
        // Regenerate with `cargo xtask golden` when the model changes.
        assert_eq!(document_schema_json()?, SCHEMA_FILE);
        Ok(())
    }

    #[test]
    fn schema_validates_golden_file() -> Result<(), serde_json::Error> {
        let schema: Value = serde_json::from_str(SCHEMA_FILE)?;
        let golden: Value = serde_json::from_str(GOLDEN)?;
        let validator = jsonschema::validator_for(&schema);
        assert!(
            validator.is_ok(),
            "schema does not compile: {:?}",
            validator.err()
        );
        if let Ok(validator) = validator {
            let errors: Vec<String> = validator
                .iter_errors(&golden)
                .map(|e| e.to_string())
                .collect();
            assert!(errors.is_empty(), "{errors:#?}");
            // The schema also rejects what the parser rejects.
            let mut bad = golden.clone();
            bad["surprise"] = json!(1);
            assert!(!validator.is_valid(&bad));
        }
        Ok(())
    }

    /// Replaces `from` with `to` once in the golden text and parses it.
    fn parse_edited(from: &str, to: &str) -> Result<Document, CanonicalError> {
        assert!(GOLDEN.contains(from), "{from}");
        from_canonical_json(&GOLDEN.replacen(from, to, 1))
    }

    #[test]
    fn parser_rejects_invalid_documents() {
        let json_err =
            |r: Result<Document, CanonicalError>| matches!(r, Err(CanonicalError::Json(_)));
        assert!(json_err(parse_edited(
            "\"schema_version\":1",
            "\"schema_version\":1,\"extra\":0"
        )));
        assert!(json_err(parse_edited(
            "\"label\":\"W-001\"",
            "\"label\":\"W-1\""
        )));
        assert!(json_err(parse_edited("\"rotation\":2", "\"rotation\":4")));
        assert!(json_err(parse_edited(
            "\"heading\":90000",
            "\"heading\":360000"
        )));
        assert!(json_err(parse_edited("sha256:aaaa", "sha256:AAAA")));
        assert!(json_err(parse_edited(
            "\"join_priority\":0",
            "\"join_priority\":0.5"
        )));
        assert!(json_err(parse_edited(
            "\"kind\":\"door\"",
            "\"kind\":\"hatch\""
        )));
        assert!(json_err(from_canonical_json("not json")));
        assert_eq!(
            parse_edited("\"schema_version\":1", "\"schema_version\":2").err(),
            Some(CanonicalError::UnsupportedVersion(Some(2)))
        );
        assert_eq!(
            from_canonical_json("{}").err(),
            Some(CanonicalError::UnsupportedVersion(None))
        );
    }

    #[test]
    fn labels_survive_save_and_load() -> Result<(), CanonicalError> {
        let mut doc = from_canonical_json(GOLDEN)?;
        // The fixture issued EQ-001 and EQ-002; deleting both must not free them.
        doc.occurrences.retain(|_, o| o.label.prefix() != "EQ");
        let mut reloaded = from_canonical_json(&to_canonical_json(&doc)?)?;
        let next = reloaded.labels.allocate("EQ").map(|l| l.to_string());
        assert_eq!(next, Ok("EQ-003".to_string()));
        Ok(())
    }

    fn json_tree() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i64>().prop_map(|n| json!(n)),
            ".{0,6}".prop_map(Value::String),
        ];
        leaf.prop_recursive(4, 32, 6, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..5).prop_map(Value::Array),
                proptest::collection::vec((".{0,4}", inner), 0..5)
                    .prop_map(|kv| { Value::Object(kv.into_iter().collect()) }),
            ]
        })
    }

    /// Rebuilds every object with its keys inserted in reverse order.
    fn reversed(v: &Value) -> Value {
        match v {
            Value::Array(a) => Value::Array(a.iter().map(reversed).collect()),
            Value::Object(m) => {
                let mut out = serde_json::Map::new();
                for (k, v) in m.iter().rev() {
                    out.insert(k.clone(), reversed(v));
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }

    proptest! {
        /// Output depends only on content, not on key insertion order, and
        /// reads back to the same value.
        #[test]
        fn canonical_output_is_order_independent(v in json_tree()) {
            let a = canonical(&v);
            prop_assert_eq!(&a, &canonical(&reversed(&v)));
            let text = a.map_err(|e| TestCaseError::fail(e.to_string()))?;
            let back: Value = serde_json::from_str(&text).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(back, v);
        }
    }
}
