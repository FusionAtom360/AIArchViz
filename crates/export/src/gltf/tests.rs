use super::*;
use archviz_core::*;
use serde_json::Value;

pub(crate) const IN: i64 = 25_400_000;
/// Meters per inch.
const M: f64 = 0.0254;

pub(crate) fn len(nm: i64) -> Length {
    Length::from_nanometers(nm)
}

pub(crate) fn uid(kind: u8, n: u8) -> Uuid {
    let mut b = [0u8; 16];
    b[0] = kind;
    b[15] = n;
    Uuid::from_bytes(b)
}

pub(crate) fn user(nm: i64) -> Sourced<Length> {
    Sourced {
        value: len(nm),
        source: Source::UserStated,
    }
}

/// The L-room (Q-54): wall centerlines (0,0) (240,0) (240,96) (96,96)
/// (96,180) (0,180) in inches, so the arms differ in length; 6 in walls; a
/// door, a window and a plain opening; slab, ceiling and a 2:12 shed roof
/// over the room; four fiducials; a marker box raised 36 in.
pub(crate) fn l_room(with_openings: bool) -> Document {
    let mut doc = Document::new(
        Project {
            id: uid(9, 0),
            name: "L-room".into(),
            default_unit: ProjectUnit::Inches,
        },
        Building {
            id: uid(9, 1),
            name: "Main".into(),
        },
    );
    let storey = uid(8, 0);
    doc.storeys.insert(
        storey,
        Storey {
            name: "Ground".into(),
            elevation: user(0),
            floor_to_floor: user(120 * IN),
            default_ceiling_height: user(108 * IN),
        },
    );
    doc.layer_sets.insert(
        uid(5, 0),
        LayerSet {
            name: "6 in".into(),
            layers: vec![Layer {
                material: uid(6, 0),
                thickness: user(6 * IN),
                core: true,
            }],
        },
    );
    let corners = [(0, 0), (240, 0), (240, 96), (96, 96), (96, 180), (0, 180)];
    for (i, (x, y)) in corners.iter().enumerate() {
        let n = u8::try_from(i).unwrap_or(0);
        doc.nodes.insert(
            uid(1, n),
            Node {
                x: len(x * IN),
                y: len(y * IN),
                join: JoinType::Isolated,
            },
        );
    }
    let mut labels = LabelAllocator::new();
    let dirs = [
        Direction::PosX,
        Direction::PosY,
        Direction::NegX,
        Direction::PosY,
        Direction::NegX,
        Direction::NegY,
    ];
    for (i, direction) in dirs.into_iter().enumerate() {
        let n = u8::try_from(i).unwrap_or(0);
        doc.walls.insert(
            uid(2, n),
            Wall {
                label: labels.allocate("W").unwrap_or_else(|_| unreachable!()),
                storey,
                node_a: uid(1, n),
                node_b: uid(1, (n + 1) % 6),
                direction,
                layer_set: uid(5, 0),
                left_extent: user(3 * IN),
                base_offset: user(0),
                top: WallTop::Storey,
                join_priority: 0,
                function: Function::New,
            },
        );
    }
    let node = |n| Reference::Node { node: uid(1, n) };
    for (i, axis) in [Axis::X, Axis::Y].into_iter().enumerate() {
        doc.constraints.insert(
            uid(3, u8::try_from(i).unwrap_or(0)),
            Constraint {
                kind: ConstraintKind::Fixed {
                    reference: node(0),
                    axis,
                    value: len(0),
                },
                status: ConstraintStatus::Driving,
                source: Source::UserStated,
                provenance: None,
            },
        );
    }
    for (n, name, (x, y)) in [
        (0u8, "F-A", (0, 0)),
        (1, "F-B", (240, 0)),
        (2, "F-C", (240, 96)),
        (3, "F-D", (0, 180)),
    ] {
        doc.datums.insert(
            uid(4, n),
            Datum {
                name: name.into(),
                geometry: DatumGeometry::Point {
                    x: len(x * IN),
                    y: len(y * IN),
                },
            },
        );
    }
    if with_openings {
        for (n, kind, host, offset, width, height, sill) in [
            (0u8, OpeningKind::Door, 0u8, 48, 36, 80, 0),
            (1, OpeningKind::Window, 1, 24, 48, 36, 36),
            (2, OpeningKind::Opening, 5, 60, 30, 84, 0),
        ] {
            doc.openings.insert(
                uid(10, n),
                Opening {
                    kind,
                    host: uid(2, host),
                    reference: node(host),
                    anchor: OpeningEdge::Start,
                    offset: user(offset * IN),
                    width: user(width * IN),
                    height: user(height * IN),
                    sill: user(sill * IN),
                    start: len(0),
                },
            );
        }
    }
    let space = uid(11, 0);
    doc.spaces.insert(
        space,
        Space {
            storey,
            name: "Room".into(),
            seed: Point {
                x: len(48 * IN),
                y: len(48 * IN),
            },
            boundary: vec![],
            finishes: Finishes {
                floor: None,
                walls: None,
                ceiling: None,
            },
        },
    );
    doc.slabs.insert(
        uid(12, 0),
        Slab {
            storey,
            boundary: Boundary::FromSpace { space },
            thickness: user(4 * IN),
            elevation: user(0),
        },
    );
    doc.ceilings.insert(
        uid(15, 0),
        Ceiling {
            storey,
            boundary: Boundary::FromSpace { space },
            thickness: user(IN / 2),
            elevation: user(108 * IN),
        },
    );
    doc.roofs.insert(
        uid(16, 0),
        Roof {
            storey,
            footprint: Boundary::FromSpace { space },
            slope_direction: Sourced {
                value: Direction::NegY,
                source: Source::UserStated,
            },
            high_edge_height: user(132 * IN),
            low_edge_height: None,
            pitch: Some(Sourced {
                value: Pitch { rise: 2, run: 12 },
                source: Source::UserStated,
            }),
            ledger: None,
            overhang: Overhang {
                pos_x: user(12 * IN),
                neg_x: user(12 * IN),
                pos_y: user(12 * IN),
                neg_y: user(12 * IN),
            },
            thickness: user(6 * IN),
            fascia_depth: user(6 * IN),
            material_slots: Default::default(),
        },
    );
    let marker = uid(13, 0);
    doc.element_types.insert(
        marker,
        proxy_box(
            "Marker",
            len(12 * IN),
            len(6 * IN),
            len(10 * IN),
            Authority::UserStatedExact,
        ),
    );
    let mut eq = LabelAllocator::new();
    doc.occurrences.insert(
        uid(14, 0),
        Occurrence {
            label: eq.allocate("EQ").unwrap_or_else(|_| unreachable!()),
            storey,
            element_type: marker,
            placement: Placement::Floor {
                x: user(200 * IN),
                y: user(40 * IN),
                rotation: QuarterTurns::new(0).unwrap_or_else(|| unreachable!()),
                elevation: user(36 * IN),
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
                x: len(200 * IN),
                y: len(40 * IN),
            },
        },
    );
    derive(doc)
        .map(|d| d.document)
        .unwrap_or_else(|e| panic!("derive: {e}"))
}

fn glb(doc: &Document) -> Vec<u8> {
    write_glb(doc).unwrap_or_else(|e| panic!("{e}"))
}

fn u32_at(b: &[u8], at: usize) -> usize {
    let mut w = [0u8; 4];
    w.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(w) as usize
}

/// The JSON and BIN chunks of a GLB.
pub(crate) fn chunks(b: &[u8]) -> (Value, Vec<u8>) {
    assert_eq!(&b[0..4], b"glTF");
    assert_eq!(u32_at(b, 4), 2);
    assert_eq!(u32_at(b, 8), b.len());
    let n = u32_at(b, 12);
    assert_eq!(&b[16..20], b"JSON");
    let json = serde_json::from_slice(&b[20..20 + n]).unwrap_or(Value::Null);
    let at = 20 + n;
    let bin = if at < b.len() {
        assert_eq!(&b[at + 4..at + 8], b"BIN\0");
        b[at + 8..at + 8 + u32_at(b, at)].to_vec()
    } else {
        Vec::new()
    };
    (json, bin)
}

fn node<'a>(json: &'a Value, name: &str) -> &'a Value {
    json["nodes"]
        .as_array()
        .and_then(|n| n.iter().find(|n| n["name"] == name))
        .unwrap_or_else(|| panic!("no node {name}"))
}

fn translation(n: &Value) -> [f64; 3] {
    let t = |k: usize| n["translation"][k].as_f64().unwrap_or(f64::NAN);
    [t(0), t(1), t(2)]
}

/// A mesh node's triangles in glTF world meters (translation + local).
fn triangles(json: &Value, bin: &[u8], n: &Value) -> Vec<[[f64; 3]; 3]> {
    let t = translation(n);
    let mesh = n["mesh"].as_u64().unwrap_or(u64::MAX) as usize;
    let prim = &json["meshes"][mesh]["primitives"][0];
    let slice = |accessor: &Value| {
        let a = &json["accessors"][accessor.as_u64().unwrap_or(0) as usize];
        let v = &json["bufferViews"][a["bufferView"].as_u64().unwrap_or(0) as usize];
        let off = v["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = v["byteLength"].as_u64().unwrap_or(0) as usize;
        bin[off..off + len].to_vec()
    };
    let pos: Vec<[f64; 3]> = slice(&prim["attributes"]["POSITION"])
        .chunks(12)
        .map(|c| {
            let f = |k: usize| {
                let mut w = [0u8; 4];
                w.copy_from_slice(&c[4 * k..4 * k + 4]);
                f64::from(f32::from_le_bytes(w)) + t[k]
            };
            [f(0), f(1), f(2)]
        })
        .collect();
    slice(&prim["indices"])
        .chunks(12)
        .map(|c| [pos[u32_at(c, 0)], pos[u32_at(c, 4)], pos[u32_at(c, 8)]])
        .collect()
}

/// Signed volume enclosed by the triangles, in cubic inches.
fn volume(tris: &[[[f64; 3]; 3]]) -> f64 {
    let v: f64 = tris
        .iter()
        .map(|[a, b, c]| {
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0
        })
        .sum();
    v / (M * M * M)
}

fn bounds(tris: &[[[f64; 3]; 3]]) -> ([f64; 3], [f64; 3]) {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in tris.iter().flatten() {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-6)
}

/// Done when: the L-room fiducials land at the expected glTF coordinates.
#[test]
fn l_room_fiducials_land_at_the_expected_gltf_coordinates() {
    let (json, bin) = chunks(&glb(&l_room(true)));
    // glTF = (x, z, -y) in meters; written from integers, so exact.
    assert_eq!(translation(node(&json, "F-A")), [0.0, 0.0, 0.0]);
    assert_eq!(translation(node(&json, "F-B")), [6.096, 0.0, 0.0]);
    assert_eq!(translation(node(&json, "F-C")), [6.096, 0.0, -2.4384]);
    assert_eq!(translation(node(&json, "F-D")), [0.0, 0.0, -4.572]);
    assert!(node(&json, "F-A").get("mesh").is_none());
    // The marker: x 194..206 in, y 37..43 in, z 36..46 in, so glTF x
    // 194..206, Y up 36..46 and Z -43..-37.
    let marker = node(&json, "EQ-001");
    assert_eq!(translation(marker), [194.0 * M, 36.0 * M, -37.0 * M]);
    let (lo, hi) = bounds(&triangles(&json, &bin, marker));
    assert!(close(lo, [194.0 * M, 36.0 * M, -43.0 * M]), "{lo:?}");
    assert!(close(hi, [206.0 * M, 46.0 * M, -37.0 * M]), "{hi:?}");
}

#[test]
fn every_solid_is_closed_and_has_the_right_volume() {
    let (json, bin) = chunks(&glb(&l_room(true)));
    let vol = |name: &str| volume(&triangles(&json, &bin, node(&json, name)));
    let near = |a: f64, b: f64| (a - b).abs() < 1e-3 * b.abs().max(1.0);
    assert!(near(vol("EQ-001"), 12.0 * 6.0 * 10.0));
    assert!(near(
        vol("door 0a000000-0000-0000-0000-000000000000"),
        36.0 * 1.75 * 80.0
    ));
    assert!(near(
        vol("window 0a000000-0000-0000-0000-000000000001"),
        48.0 * 0.25 * 36.0
    ));
    // The roof is a vertical 6 in thickness over the room's inner L grown
    // by 12 in: (-9,-9) (249,-9) (249,105) (105,105) (105,189) (-9,189).
    let roof = node(&json, "roof 10000000-0000-0000-0000-000000000000");
    assert!(near(
        volume(&triangles(&json, &bin, roof)),
        (258.0 * 114.0 + 114.0 * 84.0) * 6.0
    ));
    // Walls lose exactly their openings' voids.
    let (plain, plain_bin) = chunks(&glb(&l_room(false)));
    let walls = |json: &Value, bin: &[u8]| -> f64 {
        (1..=6)
            .map(|i| volume(&triangles(json, bin, node(json, &format!("W-{i:03}")))))
            .sum()
    };
    let voids = 36.0 * 6.0 * 80.0 + 48.0 * 6.0 * 36.0 + 30.0 * 6.0 * 84.0;
    assert!(near(walls(&json, &bin), walls(&plain, &plain_bin) - voids));
    // Plain openings have no node.
    assert!(
        json["nodes"]
            .as_array()
            .is_some_and(|n| !n.iter().any(|n| n["extras"]["kind"] == "opening"))
    );
}

#[test]
fn nodes_carry_ids_and_kinds() {
    let (json, _) = chunks(&glb(&l_room(true)));
    let w = node(&json, "W-001");
    assert_eq!(w["extras"]["kind"], "wall");
    assert_eq!(w["extras"]["id"], uid(2, 0).to_string());
    assert_eq!(
        node(&json, "EQ-001")["extras"]["element_type"],
        uid(13, 0).to_string()
    );
}

/// Done when: output is deterministic.
#[test]
fn output_is_deterministic() {
    assert_eq!(glb(&l_room(true)), glb(&l_room(true)));
}

#[test]
fn golden_l_room_glb() {
    insta::assert_binary_snapshot!(".glb", glb(&l_room(true)));
}

#[test]
fn an_independent_reader_parses_it() {
    let bytes = glb(&l_room(true));
    let g = ::gltf::Gltf::from_slice(&bytes).unwrap_or_else(|e| panic!("{e}"));
    let blob = g.blob.clone();
    let mut meshes = 0;
    for mesh in g.meshes() {
        for p in mesh.primitives() {
            let r = p.reader(|_| blob.as_deref());
            let positions = r.read_positions().map(|i| i.count()).unwrap_or(0);
            // Q-70: no normals; readers compute flat ones.
            assert!(positions > 0);
            assert!(r.read_normals().is_none());
            let indices = r.read_indices().map(|i| i.into_u32().count()).unwrap_or(0);
            assert!(indices > 0 && indices % 3 == 0);
        }
        meshes += 1;
    }
    // 6 walls, door, window, slab, ceiling, roof, marker.
    assert_eq!(meshes, 12);
    assert_eq!(g.nodes().count(), 16);
}

#[test]
fn ear_clipping_covers_an_l_exactly() {
    let l = [(0, 0), (10, 0), (10, 4), (4, 4), (4, 10), (0, 10)];
    let tris = triangulate(&l).unwrap_or_default();
    assert_eq!(tris.len(), 4);
    let sum: i128 = tris.iter().map(|t| cross(l[t[0]], l[t[1]], l[t[2]])).sum();
    assert_eq!(sum, area2(&l));
    assert!(tris.iter().all(|t| cross(l[t[0]], l[t[1]], l[t[2]]) > 0));
}

#[test]
fn clean_drops_repeats_and_collinear_points_and_turns_counterclockwise() {
    let cw = [(0, 0), (0, 5), (0, 10), (10, 10), (10, 0), (10, 0)];
    assert_eq!(clean(&cw), Some(vec![(10, 0), (10, 10), (0, 10), (0, 0)]));
    assert_eq!(clean(&[(0, 0), (5, 0), (10, 0)]), None);
}

/// The L-room with the roof bearing on its walls (Q-68, DEC-5, DEC-6): the
/// footprint is the walls' exterior outline, the underside 120 in at the
/// low side with the high edge from the 2:12 pitch, every wall following it.
pub(crate) fn l_room_under_roof() -> Document {
    let mut doc = l_room(true);
    let roof = uid(16, 0);
    if let Some(r) = doc.roofs.get_mut(&roof) {
        r.footprint = Boundary::FromSpaces {
            spaces: doc.spaces.keys().copied().collect(),
        };
        r.high_edge_height = Sourced {
            value: len(0),
            source: Source::Derived,
        };
        r.low_edge_height = Some(user(120 * IN));
    }
    for w in doc.walls.values_mut() {
        w.top = WallTop::Roof { roof };
    }
    derive(doc)
        .map(|d| d.document)
        .unwrap_or_else(|e| panic!("derive: {e}"))
}

/// A mesh node's name, POSITION values (as raw bits) and triangles.
type Mesh = (String, Vec<[u32; 3]>, Vec<[u32; 3]>);

/// Each mesh node's POSITION values and triangle index triples.
fn meshes(json: &Value, bin: &[u8]) -> Vec<Mesh> {
    let slice = |accessor: &Value| {
        let a = &json["accessors"][accessor.as_u64().unwrap_or(0) as usize];
        let v = &json["bufferViews"][a["bufferView"].as_u64().unwrap_or(0) as usize];
        let off = v["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = v["byteLength"].as_u64().unwrap_or(0) as usize;
        bin[off..off + len].to_vec()
    };
    let mut out = Vec::new();
    for n in json["nodes"].as_array().into_iter().flatten() {
        let Some(mesh) = n["mesh"].as_u64() else {
            continue;
        };
        let prim = &json["meshes"][mesh as usize]["primitives"][0];
        let positions: Vec<[u32; 3]> = slice(&prim["attributes"]["POSITION"])
            .chunks(12)
            .map(|c| {
                [
                    u32_at(c, 0) as u32,
                    u32_at(c, 4) as u32,
                    u32_at(c, 8) as u32,
                ]
            })
            .collect();
        let tris: Vec<[u32; 3]> = slice(&prim["indices"])
            .chunks(12)
            .map(|c| {
                [
                    u32_at(c, 0) as u32,
                    u32_at(c, 4) as u32,
                    u32_at(c, 8) as u32,
                ]
            })
            .collect();
        out.push((
            n["name"].as_str().unwrap_or_default().to_string(),
            positions,
            tris,
        ));
    }
    out
}

/// Q-70: every mesh has one vertex per distinct position, no normals, and
/// is closed: each edge is shared by exactly two triangles, once in each
/// direction, so the surface is watertight and consistently wound.
pub(crate) fn assert_welded_and_closed(glb: &[u8]) {
    let (json, bin) = chunks(glb);
    let all = meshes(&json, &bin);
    assert!(!all.is_empty());
    for m in json["meshes"].as_array().into_iter().flatten() {
        let attributes = &m["primitives"][0]["attributes"];
        assert!(attributes.get("NORMAL").is_none(), "{m}");
    }
    for (name, positions, tris) in all {
        let distinct: std::collections::BTreeSet<_> = positions.iter().collect();
        assert_eq!(distinct.len(), positions.len(), "{name}: repeated vertices");
        let mut edges: std::collections::BTreeMap<(u32, u32), i32> = Default::default();
        for t in &tris {
            assert!(
                t[0] != t[1] && t[1] != t[2] && t[0] != t[2],
                "{name}: degenerate"
            );
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *edges.entry((a, b)).or_default() += 1;
            }
        }
        for (&(a, b), &count) in &edges {
            assert_eq!(count, 1, "{name}: edge {a}-{b} used {count} times one way");
            assert_eq!(edges.get(&(b, a)), Some(&1), "{name}: edge {a}-{b} is open");
        }
        let used: std::collections::BTreeSet<u32> = tris.iter().flatten().copied().collect();
        assert_eq!(used.len(), positions.len(), "{name}: unused vertices");
    }
}

/// Done when (T-122b): each GLB element has one vertex per distinct
/// position and is a closed solid.
#[test]
fn every_element_is_one_welded_closed_solid() {
    assert_welded_and_closed(&glb(&l_room(true)));
    assert_welded_and_closed(&glb(&l_room(false)));
    assert_welded_and_closed(&glb(&l_room_under_roof()));
}

/// Q-68: the roof's stated heights are its underside, with the thickness
/// above; walls that follow it reach the underside. The L-room under its
/// roof: underside 120 in at y = -3 and 151 in at y = 183 (2:12 over 186 in),
/// overhangs 12 in, so 118 in at y = -15 and 153 in at y = 195; 6 in thick.
#[test]
fn the_roof_sits_on_its_underside_and_the_walls_reach_it() {
    let doc = l_room_under_roof();
    let (json, bin) = chunks(&glb(&doc));
    let roof = node(&json, "roof 10000000-0000-0000-0000-000000000000");
    let (lo, hi) = bounds(&triangles(&json, &bin, roof));
    assert!(close(lo, [-15.0 * M, 118.0 * M, -195.0 * M]), "{lo:?}");
    assert!(close(hi, [255.0 * M, 159.0 * M, 15.0 * M]), "{hi:?}");
    // The north wall W-005 is level at the underside on its higher face,
    // y = 183: 151 in.
    let (_, hi) = bounds(&triangles(&json, &bin, node(&json, "W-005")));
    assert!((hi[1] - 151.0 * M).abs() < 1e-6, "{hi:?}");
    // The west wall W-006 slopes from 120 in at y = -3 to 151 in at y = 183.
    let w6 = triangles(&json, &bin, node(&json, "W-006"));
    let top_at = |z: f64| {
        w6.iter()
            .flatten()
            .filter(|p| (p[2] - z).abs() < 1e-6)
            .map(|p| p[1])
            .fold(f64::NEG_INFINITY, f64::max)
    };
    assert!((top_at(3.0 * M) - 120.0 * M).abs() < 1e-6);
    assert!((top_at(-183.0 * M) - 151.0 * M).abs() < 1e-6);
    // Walls lose exactly their openings' voids under the sloped tops too:
    // the solid is the profile area times the thickness.
    let vol = volume(&w6);
    // W-006 runs y = 183 down to -3, mitered at both ends and 6 in thick:
    // a trapezoid of mean length 180 in whose every line along the wall is
    // centered on y = 90, where the top is 135.5 in; less the plain
    // opening's 30 x 84 x 6.
    let expected = 135.5 * 180.0 * 6.0 - 30.0 * 84.0 * 6.0;
    assert!(
        (vol - expected).abs() < 1e-3 * expected,
        "{vol} vs {expected}"
    );
}

/// `doc` with its wall thickness and default ceiling height at their
/// Proposed defaults.
pub(crate) fn provisional(mut doc: Document) -> Document {
    for set in doc.layer_sets.values_mut() {
        for layer in &mut set.layers {
            layer.thickness.source = Source::Proposed;
        }
    }
    for storey in doc.storeys.values_mut() {
        storey.default_ceiling_height.source = Source::Proposed;
    }
    doc
}

/// T-195 Done when: the GLB of a provisional document carries every item
/// of `proposed_items`, in order, and supplying the values removes the list.
#[test]
fn a_provisional_glb_lists_its_proposed_items() {
    let doc = provisional(l_room(true));
    let items = proposed_items(&doc).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(items.len(), 2);
    let (json, _) = chunks(&glb(&doc));
    let expected = serde_json::to_value(&items).unwrap_or_default();
    assert_eq!(json["extras"]["proposed_items"], expected);
    // Supplied: the list is gone and the GLB is the non-provisional one.
    let mut supplied = doc;
    for set in supplied.layer_sets.values_mut() {
        for layer in &mut set.layers {
            layer.thickness.source = Source::UserStated;
        }
    }
    for storey in supplied.storeys.values_mut() {
        storey.default_ceiling_height.source = Source::UserStated;
    }
    let bytes = glb(&supplied);
    assert!(chunks(&bytes).0.get("extras").is_none());
    assert_eq!(bytes, glb(&l_room(true)));
}

#[test]
fn solids_are_the_glb_elements_with_exact_triangles_covering_each_face() {
    let doc = l_room(true);
    let got = solids(&doc).unwrap_or_else(|e| panic!("{e}"));
    let with_faces: Vec<(Uuid, &str)> = elements(&doc)
        .unwrap_or_else(|e| panic!("{e}"))
        .iter()
        .filter(|e| !e.faces.is_empty())
        .map(|e| (e.id, e.kind))
        .collect();
    let ids: Vec<(Uuid, &str)> = got.iter().map(|s| (s.id, s.kind)).collect();
    assert_eq!(ids, with_faces);
    assert!(got.iter().any(|s| s.kind == "wall") && got.iter().any(|s| s.kind == "door"));
    for s in &got {
        assert!(!s.faces.is_empty(), "{}", s.name);
        for f in &s.faces {
            // The triangles' Newell areas (all with the face's winding) add
            // up to the face's area, exactly.
            let whole = newell(&f.points);
            let mut sum = [0i128; 3];
            for t in &f.triangles {
                let tri: Vec<P3> = t.iter().map(|&i| f.points[i]).collect();
                let n = newell(&tri);
                // Parallel to the face's normal and the same way round.
                assert!((0..3).all(|k| n[k].signum() == whole[k].signum()));
                for k in 0..3 {
                    sum[k] += n[k];
                }
            }
            assert_eq!(sum, whole, "{}", s.name);
            assert_ne!(whole, [0, 0, 0]);
        }
    }
}
