use super::*;
use crate::gltf::tests::{IN, chunks, l_room, len, uid, user};
use crate::gltf::write_glb;
use archviz_core::*;
use serde_json::{Value, json};

const SCHEMA_FILE: &str = include_str!("../../../../schemas/scene.schema.json");

fn proposed<T>(value: T) -> Sourced<T> {
    Sourced {
        value,
        source: Source::Proposed,
    }
}

fn hash(byte: char) -> ContentHash {
    ContentHash::parse(&format!("sha256:{}", byte.to_string().repeat(64)))
        .unwrap_or_else(|| unreachable!())
}

/// The T-119 L-room plus finishes, a modeled element type, a spot light
/// with an IES profile and a Proposed color temperature, a lighting variant
/// slot and a camera bookmark with the Proposed eye height.
fn scene_doc() -> Document {
    let mut doc = l_room(true);
    for (n, name) in [(0u8, "Gypsum"), (1, "Oak"), (2, "Paint")] {
        doc.materials.insert(
            uid(6, n),
            Material {
                name: name.into(),
                asset: None,
                tile_size: None,
                properties: Default::default(),
            },
        );
    }
    if let Some(space) = doc.spaces.get_mut(&uid(11, 0)) {
        space.finishes.floor = Some(uid(6, 1));
        space.finishes.ceiling = Some(uid(6, 2));
    }
    if let Some(roof) = doc.roofs.get_mut(&uid(16, 0)) {
        roof.material_slots.insert("roofing".into(), uid(6, 2));
    }
    for (n, h) in [(0u8, 'a'), (1, 'b')] {
        doc.assets.insert(
            uid(17, n),
            Asset {
                content_hash: hash(h),
                source_url: None,
                license: "CC0".into(),
                fidelity: Fidelity::Library,
                converter_log: vec![],
            },
        );
    }
    if let Some(t) = doc.element_types.get_mut(&uid(13, 0)) {
        t.model = Some(uid(17, 0));
        t.fidelity = Fidelity::Library;
    }
    doc.element_types.insert(
        uid(13, 1),
        proxy_box(
            "Can light",
            len(6 * IN),
            len(6 * IN),
            len(4 * IN),
            Authority::UserStatedExact,
        ),
    );
    doc.occurrences.insert(
        uid(14, 1),
        Occurrence {
            label: Label::try_from("EQ-002".to_string()).unwrap_or_else(|e| panic!("{e}")),
            storey: uid(8, 0),
            element_type: uid(13, 1),
            placement: Placement::CeilingMounted {
                x: user(48 * IN),
                y: user(48 * IN),
            },
            clearance: Clearance {
                front: user(0),
                back: user(0),
                left: user(0),
                right: user(0),
            },
            fixed: false,
            overlap_allowed: true,
            light: Some(Light {
                kind: LightKind::Spot,
                lumens: Sourced {
                    value: 800,
                    source: Source::UserStated,
                },
                kelvin: proposed(2700),
                ies: Some(uid(17, 1)),
            }),
            center: Point {
                x: len(48 * IN),
                y: len(48 * IN),
            },
        },
    );
    doc.variant_slots.insert(
        uid(18, 0),
        VariantSlot {
            owner: uid(14, 1),
            kind: VariantKind::Lighting,
            options: vec![uid(20, 0), uid(20, 1)],
            selected: uid(20, 1),
        },
    );
    doc.camera_bookmarks.insert(
        uid(19, 0),
        CameraBookmark {
            name: "Entry".into(),
            storey: uid(8, 0),
            position: Point {
                x: len(60 * IN),
                y: len(30 * IN),
            },
            eye_height: proposed(len(66 * IN)),
            heading: Millidegrees::new(90_000).unwrap_or_else(|| unreachable!()),
        },
    );
    doc.default_walk_start = Some(uid(19, 0));
    doc
}

fn package(doc: &Document) -> BTreeMap<String, Vec<u8>> {
    write_scene_package(doc).unwrap_or_else(|e| panic!("{e}"))
}

fn scene_json(files: &BTreeMap<String, Vec<u8>>) -> Value {
    serde_json::from_slice(&files[SCENE_FILE]).unwrap_or(Value::Null)
}

fn element(scene: &Value, id: Uuid) -> &Value {
    scene["elements"]
        .as_array()
        .and_then(|a| a.iter().find(|e| e["id"] == id.to_string()))
        .unwrap_or_else(|| panic!("no element {id}"))
}

#[test]
fn the_same_document_gives_identical_bytes() {
    let doc = scene_doc();
    let first = package(&doc);
    assert_eq!(first, package(&doc));
    // Also after a round trip through the canonical document text.
    let text = to_canonical_json(&doc).unwrap_or_default();
    let again = from_canonical_json(&text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(first, package(&again));
}

#[test]
fn the_package_has_scene_assets_and_manifest() {
    let files = package(&scene_doc());
    let names: Vec<&str> = files.keys().map(String::as_str).collect();
    assert!(names.contains(&SCENE_FILE));
    assert!(names.contains(&MANIFEST_FILE));
    // 6 walls, door, window, slab, ceiling, roof, marker, light fixture.
    let glbs = names.iter().filter(|n| n.starts_with("assets/")).count();
    assert_eq!(glbs, 13);
    assert_eq!(glbs + 2, names.len());
}

#[test]
fn schema_file_is_current() -> Result<(), CanonicalError> {
    // Regenerate with `cargo xtask golden` when the scene types change.
    assert_eq!(scene_schema_json()?, SCHEMA_FILE);
    Ok(())
}

#[test]
fn the_schema_validates_the_output() -> Result<(), serde_json::Error> {
    let schema: Value = serde_json::from_str(SCHEMA_FILE)?;
    let validator = jsonschema::validator_for(&schema).unwrap_or_else(|e| panic!("schema: {e}"));
    for doc in [scene_doc(), l_room(false)] {
        let scene = scene_json(&package(&doc));
        let errors: Vec<String> = validator
            .iter_errors(&scene)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:#?}");
        // It also rejects what a reader must not accept.
        let mut bad = scene.clone();
        bad["surprise"] = json!(1);
        assert!(!validator.is_valid(&bad));
        let mut bad = scene.clone();
        bad["elements"][0]["translation"][0] = json!(1.5);
        assert!(!validator.is_valid(&bad));
    }
    Ok(())
}

#[test]
fn manifest_hashes_verify() {
    let files = package(&scene_doc());
    let manifest = String::from_utf8(files[MANIFEST_FILE].clone()).unwrap_or_default();
    let mut listed = Vec::new();
    for line in manifest.lines() {
        let (hex, path) = line.split_once("  ").unwrap_or_else(|| panic!("{line}"));
        assert_eq!(hex.len(), 64);
        assert!(hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        let bytes = files.get(path).unwrap_or_else(|| panic!("{path} missing"));
        assert_eq!(hex, sha256_hex(bytes), "{path}");
        listed.push(path.to_string());
    }
    assert!(manifest.ends_with('\n'));
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(listed, sorted);
    let expected: Vec<String> = files
        .keys()
        .filter(|k| *k != MANIFEST_FILE)
        .cloned()
        .collect();
    assert_eq!(listed, expected);
}

#[test]
fn sha256_matches_the_standard_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn header_storeys_and_fiducials() {
    let scene = scene_json(&package(&scene_doc()));
    assert_eq!(scene["schema_version"], 1);
    assert_eq!(scene["unit"], "nanometer");
    assert_eq!(
        scene["coordinate_system"],
        json!({"handedness": "right", "x": "east", "y": "north", "z": "up"})
    );
    assert_eq!(scene["project"]["name"], "L-room");
    assert_eq!(scene["storeys"][0]["floor_to_floor"], 120 * IN);
    let f = &scene["fiducials"];
    assert_eq!(f.as_array().map(Vec::len), Some(4));
    assert_eq!(
        f[1],
        json!({"id": uid(4, 1).to_string(), "name": "F-B", "x": 240 * IN, "y": 0, "z": 0})
    );
    assert_eq!(f[3]["y"], 180 * IN);
}

#[test]
fn elements_are_ordered_by_kind_then_id_and_match_the_glb_placement() {
    let doc = scene_doc();
    let files = package(&doc);
    let scene = scene_json(&files);
    let kinds: Vec<&str> = scene["elements"]
        .as_array()
        .map(|a| a.iter().filter_map(|e| e["kind"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(
        kinds,
        [
            "wall",
            "wall",
            "wall",
            "wall",
            "wall",
            "wall",
            "door",
            "window",
            "slab",
            "ceiling",
            "roof",
            "occurrence",
            "occurrence"
        ]
    );
    // The whole-model GLB places each node where scene.json says.
    let (json, _) = chunks(&write_glb(&doc).unwrap_or_default());
    for n in json["nodes"].as_array().into_iter().flatten() {
        if n["extras"]["kind"] == "datum" {
            continue;
        }
        let e = scene["elements"]
            .as_array()
            .and_then(|a| a.iter().find(|e| e["id"] == n["extras"]["id"]))
            .unwrap_or_else(|| panic!("{n}"));
        let t = &e["translation"];
        let m = |v: &Value| v.as_i64().unwrap_or(i64::MAX) as f64 / 1e9;
        let want = [m(&t[0]), m(&t[2]), -m(&t[1])];
        for k in 0..3 {
            assert_eq!(n["translation"][k].as_f64(), Some(want[k]), "{}", e["name"]);
        }
        assert_eq!(
            e["mesh"],
            format!("assets/{}.glb", e["id"].as_str().unwrap_or(""))
        );
    }
}

#[test]
fn element_glbs_are_local_and_readable() {
    let doc = scene_doc();
    let files = package(&doc);
    let (whole, _) = chunks(&write_glb(&doc).unwrap_or_default());
    for (path, bytes) in files.iter().filter(|(p, _)| p.starts_with("assets/")) {
        let g = ::gltf::Gltf::from_slice(bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(g.nodes().count(), 1, "{path}");
        assert_eq!(g.meshes().count(), 1, "{path}");
        let (json, _) = chunks(bytes);
        let node = &json["nodes"][0];
        assert!(node.get("translation").is_none(), "{path}");
        // Same local mesh as the element's node in the whole-model GLB.
        let id = &node["extras"]["id"];
        let other = whole["nodes"]
            .as_array()
            .and_then(|a| a.iter().find(|n| n["extras"]["id"] == *id))
            .unwrap_or_else(|| panic!("{id}"));
        let bounds = |j: &Value, n: &Value| {
            let a = n["mesh"].as_u64().unwrap_or(0) as usize;
            let acc = j["meshes"][a]["primitives"][0]["attributes"]["POSITION"]
                .as_u64()
                .unwrap_or(0) as usize;
            (
                j["accessors"][acc]["min"].clone(),
                j["accessors"][acc]["max"].clone(),
            )
        };
        assert_eq!(bounds(&json, node), bounds(&whole, other), "{path}");
        // Local origin at the lowest world corner: glTF x and y start at 0
        // and glTF z (= -world y) ends at 0.
        let (min, max) = bounds(&json, node);
        assert_eq!(
            (min[0].as_f64(), min[1].as_f64()),
            (Some(0.0), Some(0.0)),
            "{path}"
        );
        assert_eq!(max[2].as_f64(), Some(0.0), "{path}");
    }
}

/// Inclusive plan and height extents of a collision prism, in nanometers.
fn extents(p: &Value) -> ([i64; 2], [i64; 2], [i64; 2]) {
    let pts: Vec<(i64, i64)> = p["outline"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|q| (q[0].as_i64().unwrap_or(0), q[1].as_i64().unwrap_or(0)))
        .collect();
    let xs = pts.iter().map(|q| q.0);
    let ys = pts.iter().map(|q| q.1);
    (
        [xs.clone().min().unwrap_or(0), xs.max().unwrap_or(0)],
        [ys.clone().min().unwrap_or(0), ys.max().unwrap_or(0)],
        [
            p["z_min"].as_i64().unwrap_or(0),
            p["z_max"].as_i64().unwrap_or(0),
        ],
    )
}

#[test]
fn collision_keeps_the_door_open() {
    let scene = scene_json(&package(&scene_doc()));
    // The door is 36 x 80 in, 48 in along W-001 (east along y = 0).
    let (door_x, door_z) = ([48 * IN, 84 * IN], [0, 80 * IN]);
    let wall = element(&scene, uid(2, 0));
    let prisms = wall["collision"].as_array().cloned().unwrap_or_default();
    assert_eq!(prisms.len(), 3, "left of, above and right of the door");
    let mut above = false;
    for p in &prisms {
        let (x, _, z) = extents(p);
        let overlap = |a: [i64; 2], b: [i64; 2]| a[0] < b[1] && b[0] < a[1];
        assert!(!(overlap(x, door_x) && overlap(z, door_z)), "{p}");
        above |= x == door_x && z[0] == 80 * IN;
    }
    assert!(above);
    for id in [uid(10, 0), uid(10, 1)] {
        assert_eq!(element(&scene, id)["collision"], json!([]));
    }
    // The window wall keeps solid below the sill and above the head.
    let w2 = element(&scene, uid(2, 1))["collision"]
        .as_array()
        .map(Vec::len);
    assert_eq!(w2, Some(4));
}

#[test]
fn collision_of_slabs_roofs_and_equipment() {
    let scene = scene_json(&package(&scene_doc()));
    let slab = &element(&scene, uid(12, 0))["collision"][0];
    assert_eq!(slab["z_min"], -4 * IN);
    assert_eq!(slab["z_max"], 0);
    let ceiling = &element(&scene, uid(15, 0))["collision"][0];
    assert_eq!(
        (ceiling["z_min"].as_i64(), ceiling["z_max"].as_i64()),
        (Some(108 * IN), Some(108 * IN + IN / 2))
    );
    // The marker: 12 x 6 x 10 in, centered at (200, 40), raised 36 in.
    let marker = &element(&scene, uid(14, 0))["collision"][0];
    assert_eq!(
        extents(marker),
        ([194 * IN, 206 * IN], [37 * IN, 43 * IN], [36 * IN, 46 * IN])
    );
    // The roof: the footprint grown 12 in on each side, from the low bottom
    // to the high top. The room's inner faces run 3 to 237 in by 3 to 177 in.
    let (x, y, z) = extents(&element(&scene, uid(16, 0))["collision"][0]);
    assert_eq!(x, [-9 * IN, 249 * IN]);
    assert_eq!(y, [-9 * IN, 189 * IN]);
    assert!(z[0] < z[1] && z[1] > 132 * IN);
}

#[test]
fn material_slots_and_occurrence_data() {
    let scene = scene_json(&package(&scene_doc()));
    let gypsum = uid(6, 0).to_string();
    assert_eq!(
        element(&scene, uid(2, 0))["material_slots"],
        json!({"left": gypsum, "right": gypsum})
    );
    assert_eq!(element(&scene, uid(2, 0))["label"], "W-001");
    assert_eq!(
        element(&scene, uid(12, 0))["material_slots"],
        json!({"floor": uid(6, 1).to_string()})
    );
    assert_eq!(
        element(&scene, uid(15, 0))["material_slots"],
        json!({"ceiling": uid(6, 2).to_string()})
    );
    assert_eq!(
        element(&scene, uid(16, 0))["material_slots"],
        json!({"roofing": uid(6, 2).to_string()})
    );
    assert_eq!(element(&scene, uid(10, 0))["label"], Value::Null);
    let marker = element(&scene, uid(14, 0));
    assert_eq!(marker["label"], "EQ-001");
    assert_eq!(
        marker["occurrence"],
        json!({
            "element_type": uid(13, 0).to_string(),
            "fidelity": "library",
            "fixed": false,
            "model": {"asset": uid(17, 0).to_string(), "content_hash": format!("sha256:{}", "a".repeat(64))},
        })
    );
    assert_eq!(
        element(&scene, uid(14, 1))["occurrence"]["model"],
        Value::Null
    );
}

#[test]
fn lights_slots_bookmarks_and_proposed_items() {
    let doc = scene_doc();
    let scene = scene_json(&package(&doc));
    // A 6 x 6 x 4 in can at (48, 48) under the 108 in ceiling.
    assert_eq!(
        scene["lights"],
        json!([{
            "id": uid(14, 1).to_string(),
            "kind": "spot",
            "lumens": 800,
            "kelvin": 2700,
            "ies": {"asset": uid(17, 1).to_string(), "content_hash": format!("sha256:{}", "b".repeat(64))},
            "position": [48 * IN, 48 * IN, 104 * IN],
            "direction": "down",
        }])
    );
    assert_eq!(
        scene["variant_slots"],
        json!([{
            "id": uid(18, 0).to_string(),
            "owner": uid(14, 1).to_string(),
            "kind": "lighting",
            "options": [uid(20, 0).to_string(), uid(20, 1).to_string()],
            "selected": uid(20, 1).to_string(),
        }])
    );
    assert_eq!(
        scene["camera_bookmarks"],
        json!([{
            "id": uid(19, 0).to_string(),
            "name": "Entry",
            "storey": uid(8, 0).to_string(),
            "x": 60 * IN,
            "y": 30 * IN,
            "z": 66 * IN,
            "heading_millidegrees": 90_000,
            "eye_height_proposed": true,
        }])
    );
    assert_eq!(scene["default_walk_start"], uid(19, 0).to_string());
    let expected =
        serde_json::to_value(proposed_items(&doc).unwrap_or_default()).unwrap_or_default();
    assert_eq!(scene["proposed_items"], expected);
    let fields: Vec<&str> = scene["proposed_items"]
        .as_array()
        .map(|a| a.iter().filter_map(|p| p["field"].as_str()).collect())
        .unwrap_or_default();
    assert!(fields.contains(&"/light/kelvin"), "{fields:?}");
    assert!(fields.contains(&"/eye_height"), "{fields:?}");
}

#[test]
fn an_unknown_asset_is_an_error() {
    let mut doc = scene_doc();
    doc.assets.remove(&uid(17, 1));
    assert_eq!(
        write_scene_package(&doc),
        Err(SceneError::UnknownAsset(uid(17, 1)))
    );
}

#[test]
fn the_folder_writer_writes_every_file_and_refuses_a_used_folder() {
    let doc = scene_doc();
    let dir = std::env::temp_dir().join(format!("archviz-scene-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write_scene_package_dir(&doc, &dir).unwrap_or_else(|e| panic!("{e}"));
    for (path, bytes) in package(&doc) {
        let on_disk = std::fs::read(dir.join(&path)).unwrap_or_default();
        assert_eq!(on_disk, bytes, "{path}");
    }
    assert!(matches!(
        write_scene_package_dir(&doc, &dir),
        Err(SceneError::NotEmpty(_))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn golden_scene_package() {
    let files = package(&scene_doc());
    let manifest = String::from_utf8(files[MANIFEST_FILE].clone()).unwrap_or_default();
    insta::assert_snapshot!("manifest", manifest);
    let scene = scene_json(&files);
    insta::assert_snapshot!("scene", to_pretty_json(&scene).unwrap_or_default());
}
