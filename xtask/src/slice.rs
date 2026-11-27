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

    /// S-1: the shop's solution does not depend on the order in which its
    /// walls and constraints were entered.
    #[test]
    fn s1_shop_solution_is_order_independent() -> Result<(), String> {
        let root = crate::workspace_root();
        let lines: Vec<String> = script(&root, "shop")?.lines().map(str::to_string).collect();
        let of_kind = |kind: &str| -> Vec<usize> {
            let tag = format!("\"kind\":\"{kind}\"}}],\"kind\":\"commands\"");
            (0..lines.len())
                .filter(|&i| lines[i].contains(&tag))
                .collect()
        };
        let walls = of_kind("create_wall");
        let constraints = of_kind("add_constraint");
        assert_eq!((walls.len(), constraints.len()), (15, 9));

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
        // Off by exactly the 1 in it contradicts.
        assert_eq!(
            conflicts[0]["residual"],
            Value::from(-25_400_000_i64),
            "{out}"
        );

        let mut apply = action.clone();
        apply["timestamp_ms"] = Value::from(1000);
        let refused = archviz("apply", &project, &apply.to_string());
        assert!(refused.is_err_and(|e| e.contains("BLOCKED")));
        let _ = std::fs::remove_dir_all(&project);
        Ok(())
    }
}
