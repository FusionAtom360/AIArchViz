//! `cargo xtask slice` (T-122): regenerates the vertical-slice outputs.
//!
//! Each fixture script in fixtures/<name>/commands.txt is replayed in
//! process through the archviz CLI library into a fresh project folder, and
//! the committed document is exported. The output folder is replaced on
//! every run:
//!
//! ```text
//! <out>/<name>/project/          genesis.json, log.jsonl, project.lock
//! <out>/<name>/<name>.dxf        write_dxf (T-118)
//! <out>/<name>/<name>.glb        write_glb (T-119)
//! <out>/<name>/scene/            write_scene_package (T-120): scene.json,
//!                                manifest.sha256, assets/<element id>.glb
//! ```
//!
//! The shop is the vertical slice; the L-room is exported beside it because
//! gate G-4 checks its fiducials in UE (S-3).

use std::collections::BTreeMap;
use std::path::Path;

use archviz_cli::{Host, run, store};
use archviz_core::{LayerA, MIN_JAMB_DISTANCE, Solver, Uuid, required_parameters, validate};
use archviz_export::{sha256_hex, write_dxf, write_glb, write_scene_package};

/// The fixtures the slice exports, in output order.
pub const FIXTURES: [&str; 2] = ["shop", "l-room"];

/// The fixture scripts give every id and timestamp, so the host is never
/// asked for one; if it were, it would still answer deterministically.
struct Replay;

impl Host for Replay {
    fn now_ms(&mut self) -> i64 {
        0
    }

    fn new_uuid(&mut self) -> Uuid {
        Uuid::from_bytes([0; 16])
    }
}

/// Runs one archviz call in process and returns its response.
pub fn archviz(sub: &str, project: &Path, request: &str) -> Result<String, String> {
    let args = [sub.to_string(), project.display().to_string()];
    let (out, status) = run(&args, request, &mut Replay);
    if status == 0 {
        Ok(out)
    } else {
        Err(format!("archviz {sub}: exit status {status}: {out}"))
    }
}

/// Replays a script of `<subcommand> <request JSON>` lines into `project`.
pub fn replay(script: &str, project: &Path) -> Result<(), String> {
    for (i, line) in script.lines().enumerate() {
        let (sub, request) = line.split_once(' ').unwrap_or((line, ""));
        archviz(sub, project, request).map_err(|e| format!("line {}: {e}", i + 1))?;
    }
    Ok(())
}

/// The script of fixture `name`, with line endings normalized.
pub fn script(root: &Path, name: &str) -> Result<String, String> {
    let path = root.join("fixtures").join(name).join("commands.txt");
    std::fs::read_to_string(&path)
        .map(|t| t.replace("\r\n", "\n"))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// What one fixture's export produced.
pub struct Exports {
    /// Files by path relative to the fixture's output folder (forward slashes).
    pub files: BTreeMap<String, Vec<u8>>,
    /// One line on the solved model: conflicts, free classes, violations,
    /// Proposed values.
    pub summary: String,
}

/// Exports the committed document of the project in `project`.
pub fn exports(name: &str, project: &Path) -> Result<Exports, String> {
    let project = store::open(project).map_err(|e| e.message)?;
    let document = project.history.document();
    let solution = LayerA::default()
        .solve(document)
        .map_err(|e| e.to_string())?;
    let violations = validate(document, MIN_JAMB_DISTANCE).map_err(|e| e.to_string())?;
    let proposed = required_parameters(document).map_err(|e| e.to_string())?;
    let summary = format!(
        "{} conflicts, {} underconstrained classes, {} violations, {} Proposed values",
        solution.conflicts.len(),
        solution.underconstrained.len(),
        violations.len(),
        proposed.len()
    );
    let mut files = BTreeMap::new();
    files.insert(
        format!("{name}.dxf"),
        write_dxf(document).map_err(|e| e.to_string())?.into_bytes(),
    );
    files.insert(
        format!("{name}.glb"),
        write_glb(document).map_err(|e| e.to_string())?,
    );
    for (path, bytes) in write_scene_package(document).map_err(|e| e.to_string())? {
        files.insert(format!("scene/{path}"), bytes);
    }
    Ok(Exports { files, summary })
}

/// Replaces `out` with the outputs of every fixture and returns a report.
pub fn slice(root: &Path, out: &Path) -> Result<String, String> {
    let io = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    if out.exists() {
        std::fs::remove_dir_all(out).map_err(|e| io(out, e))?;
    }
    let mut report = String::new();
    for name in FIXTURES {
        let dir = out.join(name);
        let project = dir.join("project");
        replay(&script(root, name)?, &project).map_err(|e| format!("{name}: {e}"))?;
        let exports = exports(name, &project)?;
        report.push_str(&format!("{name}: {}\n", exports.summary));
        for (path, bytes) in &exports.files {
            let target = path.split('/').fold(dir.clone(), |p, s| p.join(s));
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
            }
            std::fs::write(&target, bytes).map_err(|e| io(&target, e))?;
            if !path.starts_with("scene/assets/") {
                report.push_str(&format!("  {}  {}\n", sha256_hex(bytes), target.display()));
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("archviz-slice-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Every file under `dir` by relative path with forward slashes.
    fn tree(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for entry in std::fs::read_dir(&d).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path
                        .strip_prefix(dir)
                        .map_err(|e| e.to_string())?
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    out.insert(rel, std::fs::read(&path).map_err(|e| e.to_string())?);
                }
            }
        }
        Ok(out)
    }

    fn solve(project: &Path) -> Result<Value, String> {
        let out = archviz("solve", project, "")?;
        let v: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
        Ok(v["result"].clone())
    }

    #[test]
    fn slice_regenerates_identical_bytes() -> Result<(), String> {
        let root = crate::workspace_root();
        let (a, b) = (temp("a"), temp("b"));
        slice(&root, &a)?;
        // A second run replaces the folder rather than failing on it.
        slice(&root, &a)?;
        slice(&root, &b)?;
        let (ta, tb) = (tree(&a)?, tree(&b)?);
        for name in FIXTURES {
            for file in [
                format!("{name}/{name}.dxf"),
                format!("{name}/{name}.glb"),
                format!("{name}/scene/scene.json"),
                format!("{name}/scene/manifest.sha256"),
                format!("{name}/project/genesis.json"),
                format!("{name}/project/log.jsonl"),
            ] {
                assert!(ta.get(&file).is_some_and(|f| !f.is_empty()), "{file}");
            }
            assert!(
                ta.get(&format!("{name}/{name}.glb"))
                    .is_some_and(|g| g.starts_with(b"glTF"))
            );
        }
        assert_eq!(ta, tb);
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
        Ok(())
    }

    #[test]
    fn scene_manifest_covers_every_package_file() -> Result<(), String> {
        let root = crate::workspace_root();
        let project = temp("manifest");
        replay(&script(&root, "shop")?, &project)?;
        let files = exports("shop", &project)?.files;
        let manifest = String::from_utf8(
            files
                .get("scene/manifest.sha256")
                .cloned()
                .ok_or("no manifest")?,
        )
        .map_err(|e| e.to_string())?;
        let listed: BTreeMap<String, String> = manifest
            .lines()
            .filter_map(|l| l.split_once("  "))
            .map(|(h, p)| (format!("scene/{p}"), h.to_string()))
            .collect();
        let package: Vec<&String> = files
            .keys()
            .filter(|p| p.starts_with("scene/") && p.as_str() != "scene/manifest.sha256")
            .collect();
        assert_eq!(listed.len(), package.len());
        for path in package {
            assert_eq!(
                listed.get(path),
                files.get(path).map(|b| sha256_hex(b)).as_ref(),
                "{path}"
            );
        }
        let _ = std::fs::remove_dir_all(&project);
        Ok(())
    }

    /// T-191 (S-3, spec 5.4): the slice's L-room DXF, read back as group
    /// pairs, has a POINT on DATUMS at each fiducial and a TEXT naming it at
    /// the same point, exact to the nanometer.
    #[test]
    fn l_room_dxf_fiducials_read_back_exactly() -> Result<(), String> {
        let root = crate::workspace_root();
        let project = temp("l-room-dxf");
        replay(&script(&root, "l-room")?, &project)?;
        let dxf = exports("l-room", &project)?
            .files
            .get("l-room.dxf")
            .cloned()
            .ok_or("no DXF")?;
        let _ = std::fs::remove_dir_all(&project);
        let text = String::from_utf8(dxf).map_err(|e| e.to_string())?;
        let lines: Vec<&str> = text.split("\r\n").collect();
        let mut objects: Vec<(&str, Vec<(i32, &str)>)> = Vec::new();
        for pair in lines.chunks(2) {
            let [code, value] = pair else { continue };
            match code.trim().parse::<i32>() {
                Ok(0) => objects.push((value, Vec::new())),
                Ok(c) => {
                    if let Some(last) = objects.last_mut() {
                        last.1.push((c, value));
                    }
                }
                Err(_) => {}
            }
        }
        fn get<'a>(p: &[(i32, &'a str)], code: i32) -> Option<&'a str> {
            p.iter().find(|(c, _)| *c == code).map(|(_, v)| *v)
        }
        let at = |p: &[(i32, &str)]| {
            let c = |code| get(p, code).and_then(archviz_export::parse_inches);
            (c(10), c(20))
        };
        let mut points = Vec::new();
        let mut labels = BTreeMap::new();
        for (kind, p) in &objects {
            if get(p, 8) != Some("DATUMS") {
                continue;
            }
            match *kind {
                "POINT" => points.push(at(p)),
                "TEXT" => {
                    labels.insert(get(p, 1).unwrap_or_default(), at(p));
                }
                _ => {}
            }
        }
        let i = |n: i64| Some(n * 25_400_000);
        let expected = BTreeMap::from([
            ("F-A", (i(0), i(0))),
            ("F-B", (i(240), i(0))),
            ("F-C", (i(240), i(96))),
            ("F-D", (i(0), i(180))),
        ]);
        assert_eq!(labels, expected);
        points.sort();
        let mut labelled: Vec<_> = expected.values().copied().collect();
        labelled.sort();
        assert_eq!(points, labelled);
        Ok(())
    }

    /// S-1: the shop's solution does not depend on the order in which its
    /// walls and constraints were entered.
    #[test]
    fn s1_shop_solution_is_order_independent() -> Result<(), String> {
        let root = crate::workspace_root();
        let lines: Vec<String> = script(&root, "shop")?.lines().map(str::to_string).collect();
        // Single-command lines only: the extension's walls are created in
        // one apply with the roof they follow, which must exist first.
        let of_kind = |kind: &str| -> Vec<usize> {
            let tag = format!("\"kind\":\"{kind}\"}}],\"kind\":\"commands\"");
            (0..lines.len())
                .filter(|&i| {
                    lines[i].contains(&tag) && lines[i].matches("\"idempotency_key\"").count() == 1
                })
                .collect()
        };
        let walls = of_kind("create_wall");
        let constraints = of_kind("add_constraint");
        assert_eq!((walls.len(), constraints.len()), (12, 9));

        let original = temp("order-0");
        replay(&lines.join("\n"), &original)?;
        let expected = solve(&original)?;
        assert_eq!(expected["conflicts"], Value::Array(vec![]));

        // Permute each block within its own slots: reversed, then rotated.
        let permutations: [fn(&mut Vec<String>); 2] = [|v| v.reverse(), |v| v.rotate_left(4)];
        for (k, permute) in permutations.iter().enumerate() {
            let mut shuffled = lines.clone();
            for block in [&walls, &constraints] {
                let mut moved: Vec<String> = block.iter().map(|&i| lines[i].clone()).collect();
                permute(&mut moved);
                for (&slot, line) in block.iter().zip(moved) {
                    shuffled[slot] = line;
                }
            }
            assert_ne!(shuffled, lines);
            let project = temp(&format!("order-{}", k + 1));
            replay(&shuffled.join("\n"), &project)?;
            assert_eq!(solve(&project)?, expected, "permutation {}", k + 1);
            let _ = std::fs::remove_dir_all(&project);
        }
        let _ = std::fs::remove_dir_all(&original);
        Ok(())
    }

    /// S-1: a dimension that contradicts the shop is reported as a Conflict
    /// naming both constraints, and the commit is refused.
    #[test]
    fn s1_shop_conflict_names_both_constraints() -> Result<(), String> {
        let root = crate::workspace_root();
        let text = script(&root, "shop")?;
        let project = temp("conflict");
        replay(&text, &project)?;

        // The first constraint: datum SW to the west wall's exterior face, 0 in.
        let line = text
            .lines()
            .find(|l| l.contains("\"kind\":\"add_constraint\""))
            .ok_or("no constraint")?;
        let (_, json) = line.split_once(' ').ok_or("bad line")?;
        let mut request: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let command = &mut request["action"]["commands"][0];
        let kind = command["args"]["kind"].clone();
        command["id"] = Value::String(
            Uuid::derive(Uuid::from_bytes([0; 16]), "t122-test", "command").to_string(),
        );
        command["idempotency_key"] =
            Value::String(Uuid::derive(Uuid::from_bytes([0; 16]), "t122-test", "key").to_string());
        command["args"]["kind"]["value"] = Value::from(25_400_000_i64);
        let action = serde_json::json!({ "action": request["action"] });

        let out = archviz("propose", &project, &action.to_string())?;
        let proposal: Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
        // Constraint ids are derived from the command (Q-66), so read them
        // back: the stored constraint with the original kind, and the one
        // this proposal adds.
        let stored: Value = serde_json::from_str(&archviz(
            "query",
            &project,
            r#"{"collection":"constraints"}"#,
        )?)
        .map_err(|e| e.to_string())?;
        let existing = stored["result"]
            .as_object()
            .and_then(|m| m.iter().find(|(_, c)| c["kind"] == kind))
            .map(|(id, _)| id.clone())
            .ok_or("original constraint not stored")?;
        let new_id = proposal["result"]["diff"][0]["id"]
            .as_str()
            .ok_or("no added constraint")?
            .to_string();
        let conflicts = proposal["result"]["conflicts"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert_eq!(conflicts.len(), 1, "{out}");
        let ids: Vec<&str> = std::iter::once(&conflicts[0]["constraint"])
            .chain(conflicts[0]["with"].as_array().into_iter().flatten())
            .filter_map(Value::as_str)
            .collect();
        assert!(ids.contains(&existing.as_str()), "{out}");
        assert!(ids.contains(&new_id.as_str()), "{out}");
        assert_ne!(existing, new_id);
        // Off by exactly the 1 in it contradicts: computed minus stated for
        // the constraint reported, the new one (1 in stated) or the existing
        // one (0 in stated).
        let residual = if conflicts[0]["constraint"] == Value::String(new_id.clone()) {
            -25_400_000_i64
        } else {
            25_400_000_i64
        };
        assert_eq!(conflicts[0]["residual"], Value::from(residual), "{out}");

        let mut apply = action.clone();
        apply["timestamp_ms"] = Value::from(1000);
        let refused = archviz("apply", &project, &apply.to_string());
        assert!(refused.is_err_and(|e| e.contains("BLOCKED")));
        let _ = std::fs::remove_dir_all(&project);
        Ok(())
    }

    // T-122b: the slice geometry (DEC-5, DEC-6, Q-67 to Q-70).

    use archviz_core::{
        Document, Point, WallTop, boundary_outline, roof_geometry, roof_outline, wall_geometry,
        wall_profiles,
    };

    const IN: i128 = 25_400_000;

    /// Replays fixture `name` (optionally with its script rewritten) and
    /// returns the committed document.
    fn document(name: &str, tag: &str, edit: impl Fn(&str) -> String) -> Result<Document, String> {
        let root = crate::workspace_root();
        let project = temp(&format!("{name}-{tag}"));
        replay(&edit(&script(&root, name)?), &project)?;
        let doc = store::open(&project)
            .map_err(|e| e.message)?
            .history
            .document()
            .clone();
        let _ = std::fs::remove_dir_all(&project);
        Ok(doc)
    }

    fn xy(p: &Point) -> (i128, i128) {
        (i128::from(p.x.nanometers()), i128::from(p.y.nanometers()))
    }

    /// The bounding rectangle of a polygon.
    fn bounds(points: &[Point]) -> ((i128, i128), (i128, i128)) {
        let v: Vec<_> = points.iter().map(xy).collect();
        let min = |f: fn(&(i128, i128)) -> i128| v.iter().map(f).min().unwrap_or(0);
        let max = |f: fn(&(i128, i128)) -> i128| v.iter().map(f).max().unwrap_or(0);
        ((min(|p| p.0), min(|p| p.1)), (max(|p| p.0), max(|p| p.1)))
    }

    /// The area a rectilinear polygon shares with an axis-aligned rectangle,
    /// exact: the polygon clipped to the rectangle (Sutherland-Hodgman on
    /// axis-aligned planes keeps integer vertices), then the shoelace sum.
    fn shared_area(poly: &[Point], (lo, hi): ((i128, i128), (i128, i128))) -> i128 {
        let mut v: Vec<(i128, i128)> = poly.iter().map(xy).collect();
        let planes: [(bool, i128, i128); 4] = [
            (true, lo.0, 1),
            (true, hi.0, -1),
            (false, lo.1, 1),
            (false, hi.1, -1),
        ];
        for (on_x, c, sign) in planes {
            let get = |p: (i128, i128)| if on_x { p.0 } else { p.1 };
            let keep = |p: (i128, i128)| (get(p) - c) * sign >= 0;
            let input = std::mem::take(&mut v);
            for i in 0..input.len() {
                let (p, q) = (input[i], input[(i + 1) % input.len()]);
                if keep(p) {
                    v.push(p);
                }
                if keep(p) != keep(q) {
                    // Rectilinear edges cross an axis plane only when
                    // perpendicular to it, so the crossing is exact.
                    v.push(if on_x { (c, p.1) } else { (p.0, c) });
                }
            }
        }
        let n = v.len();
        ((0..n)
            .map(|i| v[i].0 * v[(i + 1) % n].1 - v[(i + 1) % n].0 * v[i].1)
            .sum::<i128>()
            / 2)
        .abs()
    }

    fn area(((x0, y0), (x1, y1)): ((i128, i128), (i128, i128))) -> i128 {
        (x1 - x0) * (y1 - y0)
    }

    /// Done when: every slab reaches the walls' exterior faces and nothing
    /// is open under a door void. Every wall's footprint, and so every void
    /// through it, lies on a slab; each fixture has one slab (DEC-5).
    #[test]
    fn slabs_reach_the_walls_exterior_faces() -> Result<(), String> {
        for name in FIXTURES {
            let doc = document(name, "slab", str::to_string)?;
            assert_eq!(doc.slabs.len(), 1, "{name}: one slab per contiguous floor");
            let slabs: Vec<Vec<Point>> = doc
                .slabs
                .values()
                .map(|s| boundary_outline(&doc, &s.boundary))
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?;
            for (id, g) in wall_geometry(&doc).map_err(|e| e.to_string())? {
                let rect = bounds(&g.footprint);
                let covered: i128 = slabs.iter().map(|s| shared_area(s, rect)).sum();
                assert_eq!(covered, area(rect), "{name}: wall {id} is not on the slab");
            }
            assert!(
                doc.openings
                    .values()
                    .any(|o| o.sill.value.nanometers() == 0)
            );
        }
        // The shop's slab: the 216 x 432 in shell and the extension to x = 280.
        let shop = document("shop", "slab-outline", str::to_string)?;
        let outline = shop
            .slabs
            .values()
            .map(|s| boundary_outline(&shop, &s.boundary))
            .next()
            .ok_or("no slab")?
            .map_err(|e| e.to_string())?;
        let inches: Vec<(i128, i128)> = outline
            .iter()
            .map(|p| (xy(p).0 / IN, xy(p).1 / IN))
            .collect();
        assert_eq!(
            inches,
            vec![
                (0, 0),
                (216, 0),
                (216, 36),
                (280, 36),
                (280, 396),
                (216, 396),
                (216, 432),
                (0, 432)
            ]
        );
        Ok(())
    }

    /// Done when: no roof passes through a wall or ceiling, and the shop
    /// roof starts at the east wall's exterior face. A wall whose footprint
    /// shares area with a roof's plan follows that roof; a ceiling stays
    /// below the underside over it.
    #[test]
    fn no_roof_passes_through_a_wall_or_ceiling() -> Result<(), String> {
        for name in FIXTURES {
            let doc = document(name, "roof", str::to_string)?;
            assert_eq!(doc.roofs.len(), 1);
            for (roof_id, roof) in &doc.roofs {
                let plan = roof_outline(&doc, *roof_id).map_err(|e| e.to_string())?;
                let g = roof_geometry(&doc, *roof_id).map_err(|e| e.to_string())?;
                let level = doc
                    .storeys
                    .get(&roof.storey)
                    .map(|s| i128::from(s.elevation.value.nanometers()))
                    .ok_or("no storey")?;
                for (id, wg) in wall_geometry(&doc).map_err(|e| e.to_string())? {
                    let w = doc.walls.get(&id).ok_or("no wall")?;
                    if shared_area(&plan, bounds(&wg.footprint)) > 0 {
                        assert_eq!(
                            w.top,
                            WallTop::Roof { roof: *roof_id },
                            "{name}: roof passes through wall {}",
                            w.label
                        );
                    }
                }
                for c in doc.ceilings.values() {
                    let top = i128::from(c.elevation.value.nanometers())
                        + i128::from(c.thickness.value.nanometers());
                    let outline = boundary_outline(&doc, &c.boundary).map_err(|e| e.to_string())?;
                    // The underside is a plane, so over the ceiling it is
                    // lowest at one of the ceiling's corners.
                    for p in &outline {
                        let s = match g.axis() {
                            archviz_core::Axis::X => p.x,
                            archviz_core::Axis::Y => p.y,
                        };
                        let under = level
                            + i128::from(g.underside(s).map_err(|e| e.to_string())?.nanometers());
                        assert!(top <= under, "{name}: the roof passes through a ceiling");
                    }
                }
            }
        }
        let shop = document("shop", "roof-start", str::to_string)?;
        let (id, _) = shop.roofs.iter().next().ok_or("no roof")?;
        let plan = roof_outline(&shop, *id).map_err(|e| e.to_string())?;
        let ((x0, y0), (x1, y1)) = bounds(&plan);
        // The east wall's exterior face is x = 216 in; 12 in overhangs elsewhere.
        assert_eq!([x0, y0, x1, y1].map(|v| v / IN), [216, 24, 292, 408]);
        assert_eq!(x0 % IN, 0);
        Ok(())
    }

    /// `n / d` rounded half to even (d > 0), written apart from core.
    fn half_even(n: i128, d: i128) -> i128 {
        let (q, r) = (n.div_euclid(d), n.rem_euclid(d));
        match (2 * r).cmp(&d) {
            std::cmp::Ordering::Less => q,
            std::cmp::Ordering::Greater => q + 1,
            std::cmp::Ordering::Equal => q + (q & 1),
        }
    }

    /// Done when: no gap between wall tops and the roof underside. Every
    /// vertex of every roof-following wall's top edge is checked to the
    /// nanometer against the underside at the same point on both faces,
    /// computed here from the stored heights: equal along the slope; across
    /// it, level with the higher face, so no face is left below it.
    #[test]
    fn roof_following_wall_tops_meet_the_underside() -> Result<(), String> {
        let mut checked = 0;
        for name in FIXTURES {
            let doc = document(name, "tops", str::to_string)?;
            let profiles = wall_profiles(&doc).map_err(|e| e.to_string())?;
            let geometry = wall_geometry(&doc).map_err(|e| e.to_string())?;
            for (id, w) in &doc.walls {
                let WallTop::Roof { roof } = &w.top else {
                    continue;
                };
                let r = doc.roofs.get(roof).ok_or("no roof")?;
                let g = roof_geometry(&doc, *roof).map_err(|e| e.to_string())?;
                let n = |l: archviz_core::Length| i128::from(l.nanometers());
                let (high, low) = (
                    n(r.high_edge_height.value),
                    n(r.low_edge_height.as_ref().ok_or("no low edge")?.value),
                );
                let level = doc
                    .storeys
                    .get(&r.storey)
                    .map(|s| n(s.elevation.value))
                    .ok_or("no storey")?;
                let sign = match g.slope_direction {
                    archviz_core::Direction::PosX | archviz_core::Direction::PosY => 1,
                    _ => -1,
                };
                let under = |s: i128| {
                    level + high - half_even((s - n(g.high_edge)) * sign * (high - low), n(g.run))
                };
                let along_x = matches!(
                    w.direction,
                    archviz_core::Direction::PosX | archviz_core::Direction::NegX
                );
                let slope_x = g.axis() == archviz_core::Axis::X;
                let ((x0, y0), (x1, y1)) =
                    bounds(&geometry.get(id).ok_or("no geometry")?.footprint);
                let faces = if along_x { [y0, y1] } else { [x0, x1] };
                let p = profiles.get(id).ok_or("no profile")?;
                assert!(p.top_edge.len() >= 2);
                for &(u, z) in &p.top_edge {
                    let z = n(z);
                    let at_faces = if along_x == slope_x {
                        [under(n(u)), under(n(u))]
                    } else {
                        faces.map(under)
                    };
                    let highest = at_faces[0].max(at_faces[1]);
                    assert_eq!(z, highest, "{name} {} at {}", w.label, n(u));
                    assert!(at_faces.iter().all(|&h| z >= h), "gap above {}", w.label);
                    checked += 1;
                }
            }
        }
        // Six L-room walls and three extension walls, several vertices each.
        assert!(checked >= 9 * 3, "{checked}");
        Ok(())
    }

    /// Reverses the order in which walls are created: the single-command
    /// wall lines among their own slots, and the walls within each batch.
    fn reverse_walls(text: &str) -> String {
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        let single = |l: &str| {
            l.starts_with("apply ")
                && l.matches("\"idempotency_key\"").count() == 1
                && l.contains("\"kind\":\"create_wall\"")
        };
        let slots: Vec<usize> = (0..lines.len()).filter(|&i| single(&lines[i])).collect();
        let moved: Vec<String> = slots.iter().rev().map(|&i| lines[i].clone()).collect();
        for (&slot, line) in slots.iter().zip(moved) {
            lines[slot] = line;
        }
        for line in &mut lines {
            if !line.starts_with("apply ") || single(line) {
                continue;
            }
            let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&line[6..]) else {
                continue;
            };
            let Some(commands) = v["action"]["commands"].as_array_mut() else {
                continue;
            };
            let walls: Vec<usize> = (0..commands.len())
                .filter(|&i| commands[i]["kind"] == "create_wall")
                .collect();
            let reversed: Vec<serde_json::Value> =
                walls.iter().rev().map(|&i| commands[i].clone()).collect();
            for (&slot, c) in walls.iter().zip(reversed) {
                commands[slot] = c;
            }
            *line = format!("apply {v}");
        }
        let mut out = lines.join("\n");
        out.push('\n');
        out
    }

    /// Done when: reversed wall creation order gives identical wall profiles.
    #[test]
    fn reversed_wall_creation_gives_identical_profiles() -> Result<(), String> {
        for name in FIXTURES {
            let root = crate::workspace_root();
            let text = script(&root, name)?;
            assert_ne!(reverse_walls(&text), text, "{name}");
            let a = document(name, "order-a", str::to_string)?;
            let b = document(name, "order-b", reverse_walls)?;
            let labels = |d: &Document| -> Vec<String> {
                d.walls.values().map(|w| w.label.to_string()).collect()
            };
            assert_ne!(labels(&a), labels(&b), "{name}: the order did change");
            let pa = wall_profiles(&a).map_err(|e| e.to_string())?;
            let pb = wall_profiles(&b).map_err(|e| e.to_string())?;
            assert_eq!(pa, pb, "{name}");
            assert!(
                pa.values()
                    .any(|p| p.top_edge.windows(2).any(|e| e[0].1 != e[1].1))
            );
        }
        Ok(())
    }

    fn u32_at(b: &[u8], at: usize) -> usize {
        let mut w = [0u8; 4];
        w.copy_from_slice(&b[at..at + 4]);
        u32::from_le_bytes(w) as usize
    }

    /// Checks one GLB: every mesh has one vertex per distinct position, no
    /// normals, and every edge in exactly two triangles, once each way.
    fn welded_and_closed(glb: &[u8], what: &str) -> usize {
        let n = u32_at(glb, 12);
        let json: serde_json::Value =
            serde_json::from_slice(&glb[20..20 + n]).unwrap_or(serde_json::Value::Null);
        let bin = if 20 + n < glb.len() {
            &glb[20 + n + 8..]
        } else {
            &glb[0..0]
        };
        let slice = |accessor: &serde_json::Value| {
            let a = &json["accessors"][accessor.as_u64().unwrap_or(0) as usize];
            let v = &json["bufferViews"][a["bufferView"].as_u64().unwrap_or(0) as usize];
            let off = v["byteOffset"].as_u64().unwrap_or(0) as usize;
            let len = v["byteLength"].as_u64().unwrap_or(0) as usize;
            bin[off..off + len].to_vec()
        };
        let mut meshes = 0;
        for m in json["meshes"].as_array().into_iter().flatten() {
            let prim = &m["primitives"][0];
            assert!(prim["attributes"].get("NORMAL").is_none(), "{what}");
            let positions: Vec<Vec<u8>> = slice(&prim["attributes"]["POSITION"])
                .chunks(12)
                .map(<[u8]>::to_vec)
                .collect();
            let distinct: BTreeMap<&Vec<u8>, ()> = positions.iter().map(|p| (p, ())).collect();
            assert_eq!(
                distinct.len(),
                positions.len(),
                "{what} {}: repeated",
                m["name"]
            );
            let mut edges: BTreeMap<(usize, usize), usize> = BTreeMap::new();
            for t in slice(&prim["indices"]).chunks(12) {
                let t = [u32_at(t, 0), u32_at(t, 4), u32_at(t, 8)];
                for k in 0..3 {
                    *edges.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
                }
            }
            for (&(a, b), &count) in &edges {
                assert_eq!(count, 1, "{what} {}: edge {a}-{b}", m["name"]);
                assert_eq!(
                    edges.get(&(b, a)),
                    Some(&1),
                    "{what} {}: open at {a}-{b}",
                    m["name"]
                );
            }
            meshes += 1;
        }
        meshes
    }

    /// Done when: each GLB element has one vertex per distinct position and
    /// is a closed solid, in the whole-model GLB and in every Scene Package
    /// element GLB of both fixtures.
    #[test]
    fn every_fixture_glb_element_is_welded_and_closed() -> Result<(), String> {
        let root = crate::workspace_root();
        for name in FIXTURES {
            let project = temp(&format!("{name}-glb"));
            replay(&script(&root, name)?, &project)?;
            let files = exports(name, &project)?.files;
            let model = files.get(&format!("{name}.glb")).ok_or("no glb")?;
            let meshes = welded_and_closed(model, name);
            let mut elements = 0;
            for (path, bytes) in &files {
                if path.starts_with("scene/assets/") && path.ends_with(".glb") {
                    elements += welded_and_closed(bytes, path);
                }
            }
            assert_eq!(meshes, elements, "{name}");
            assert!(meshes > 10, "{name}: {meshes}");
            let _ = std::fs::remove_dir_all(&project);
        }
        Ok(())
    }
}
