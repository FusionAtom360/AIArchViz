//! Proposal review (T-204, spec 8.3, 8.4, 9.4).
//!
//! Every proposal the app makes is shown in the review panel before it can
//! be committed: the echo text the user confirms, the diff (before and after
//! in display units), violations, ambiguities, Proposed flags, consequences
//! and alternatives, then Confirm or Discard.
//!
//! The panel is the only way to commit: `Session::commit` takes a
//! [`Confirmed`], which only this module can make, from the panel's Confirm
//! ([`Review::confirm`]) or, when the user has turned on
//! `auto_commit_unambiguous` (off by default), from [`Review::auto`] for a
//! proposal with no ambiguities, no Proposed values and no consequence moves.

use std::collections::BTreeMap;

use archviz_core::{Constraint, Document, Length, Uuid, describe_constraint, format_length};
use serde_json::Value;

use crate::dimension::{self, ConflictReport};
use crate::session::Pending;

/// Review settings (9.4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Settings {
    /// Commit, without review, proposals with no ambiguities, no Proposed
    /// values and no consequence moves. Off by default.
    pub auto_commit_unambiguous: bool,
}

/// A proposal the user has confirmed: what `Session::commit` takes. Only
/// this module makes one.
#[derive(Debug)]
pub struct Confirmed(Pending);

impl Confirmed {
    /// The proposal to commit.
    pub fn pending(&self) -> &Pending {
        &self.0
    }
}

/// A proposal as the review panel shows it, every value named and every
/// length in feet-inches.
#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    pending: Pending,
    /// The resolved commands in canonical plain language (9.4).
    pub echo: Vec<String>,
    /// One line per changed entity, then one per changed value.
    pub diff: Vec<String>,
    pub violations: Vec<String>,
    /// Unresolved Referent, Direction, Magnitude and ReferenceSemantics
    /// choices, each with its candidates.
    pub ambiguities: Vec<String>,
    /// PolicyChoice candidates: other dimensions that could absorb a move
    /// (8.7 (b)).
    pub alternatives: Vec<String>,
    /// Values the proposal adds or changes with source Proposed.
    pub proposed: Vec<String>,
    /// Existing elements the proposal changes (6.7).
    pub existing: Vec<String>,
    /// Entities that move as a consequence (8.7).
    pub consequences: Vec<String>,
    /// Solver conflicts, each naming its constraints (7.5).
    pub conflicts: ConflictReport,
    /// Whether the proposal rests on Proposed values already in the model.
    pub provisional: bool,
    /// Whether something stops the commit, and what.
    pub blocked: Vec<String>,
}

fn array(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

fn uuid(v: &Value) -> Option<Uuid> {
    serde_json::from_value(v.clone()).ok()
}

/// A JSON value in words: strings as they are, anything else compact.
fn text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "none".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// "layer_sets" reads as "layer set".
fn singular(collection: &str) -> String {
    collection
        .strip_suffix('s')
        .unwrap_or(collection)
        .replace('_', " ")
}

/// Names entities: a label ("wall W-001"), an opening by its host ("door in
/// W-005"), a node by its position, a constraint in the echo's words, and
/// anything else by its collection and the start of its id.
struct Names<'a> {
    doc: &'a Document,
    names: BTreeMap<Uuid, String>,
}

/// The name of `entity` of `collection`, from its JSON.
fn own_name(collection: &str, entity: &Value) -> Option<String> {
    if let Some(label) = entity["label"].as_str().or(entity["name"].as_str()) {
        return Some(format!("{} {label}", singular(collection)));
    }
    if collection == "nodes" {
        let at = |axis: &str| serde_json::from_value::<Length>(entity[axis].clone()).ok();
        return Some(format!(
            "node at ({}, {})",
            format_length(at("x")?),
            format_length(at("y")?)
        ));
    }
    None
}

impl<'a> Names<'a> {
    fn new(doc: &'a Document, diff: &[Value]) -> Names<'a> {
        let mut names = BTreeMap::new();
        let mut openings = Vec::new();
        if let Ok(Value::Object(fields)) = serde_json::to_value(doc) {
            for (collection, entities) in fields {
                let Value::Object(entities) = entities else {
                    continue;
                };
                for (id, entity) in entities {
                    let Some(id) = uuid(&Value::String(id)) else {
                        continue;
                    };
                    if collection == "openings" {
                        openings.push((id, entity));
                    } else if let Some(name) = own_name(&collection, &entity) {
                        names.insert(id, name);
                    }
                }
            }
        }
        // Entities the proposal adds are named as they are after it.
        for entry in diff {
            let collection = entry["collection"].as_str().unwrap_or_default();
            if let Some(id) = uuid(&entry["id"])
                && !names.contains_key(&id)
            {
                if collection == "openings" {
                    openings.push((id, entry["after"].clone()));
                } else if let Some(name) = own_name(collection, &entry["after"]) {
                    names.insert(id, name);
                }
            }
        }
        for (id, opening) in openings {
            let kind = opening["kind"].as_str().unwrap_or("opening");
            let host = uuid(&opening["host"])
                .and_then(|h| names.get(&h))
                .map_or_else(String::new, |h| {
                    format!(" in {}", h.trim_start_matches("wall "))
                });
            names.insert(id, format!("{}{host}", words(kind).to_lowercase()));
        }
        Names { doc, names }
    }

    /// An entity of `collection`, with `value` its before or after JSON.
    fn entity(&self, collection: &str, id: Option<Uuid>, value: Option<&Value>) -> String {
        let kind = singular(collection);
        let Some(id) = id else {
            return kind;
        };
        if let Some(name) = self.names.get(&id) {
            return name.clone();
        }
        if collection == "constraints"
            && let Some(c) = value
                .cloned()
                .and_then(|v| serde_json::from_value::<Constraint>(v).ok())
        {
            return describe_constraint(self.doc, &c.kind);
        }
        format!("{kind} {}", short(id))
    }

    /// Any id: a named entity, a constraint of the document, or the id.
    fn id(&self, id: Uuid) -> String {
        if let Some(name) = self.names.get(&id) {
            return name.clone();
        }
        if let Some(c) = self.doc.constraints.get(&id) {
            return describe_constraint(self.doc, &c.kind);
        }
        short(id)
    }

    /// A violation's detail in words, every length in feet-inches.
    fn detail(&self, key: Option<&str>, v: &Value) -> String {
        const LENGTHS: [&str; 9] = [
            "length",
            "offset",
            "dx",
            "dy",
            "required",
            "clearance",
            "by",
            "stated",
            "derived",
        ];
        match v {
            Value::Null => String::new(),
            Value::String(s) => match uuid(v) {
                Some(id) => self.id(id),
                None => s.replace('_', " "),
            },
            Value::Number(n) if key.is_some_and(|k| LENGTHS.contains(&k)) => n
                .as_i64()
                .map(|nm| format_length(Length::from_nanometers(nm)))
                .unwrap_or_else(|| n.to_string()),
            Value::Array(items) => items
                .iter()
                .map(|i| self.detail(key, i))
                .collect::<Vec<_>>()
                .join("; "),
            Value::Object(fields) => fields
                .iter()
                .map(|(k, v)| {
                    let inner = self.detail(Some(k), v);
                    let k = k.replace('_', " ");
                    if inner.is_empty() {
                        k
                    } else {
                        format!("{k} {inner}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", "),
            other => other.to_string(),
        }
    }

    /// The dimension a move edit changes (`DimensionTarget`).
    fn target(&self, target: &Value) -> String {
        let of = |field: &str| uuid(&target[field]).map(|id| self.id(id));
        let axis = target["axis"].as_str().unwrap_or("?").to_uppercase();
        match target["kind"].as_str() {
            Some("constraint") => of("constraint").unwrap_or_default(),
            Some("opening_offset") => {
                format!("the offset of {}", of("opening").unwrap_or_default())
            }
            Some("occurrence_offset") => {
                format!("the offset of {}", of("occurrence").unwrap_or_default())
            }
            Some("node_coordinate") => format!("{} {axis}", of("node").unwrap_or_default()),
            Some("occurrence_coordinate") => {
                format!("{} {axis}", of("occurrence").unwrap_or_default())
            }
            _ => target.to_string(),
        }
    }

    /// One candidate of an ambiguity: the dimension, what it is set to and
    /// what moves.
    fn candidate(&self, c: &Value, default: Option<Uuid>) -> String {
        let choice = uuid(&c["choice"]).map(|id| (id, self.id(id)));
        let mut line = choice.as_ref().map_or_else(String::new, |(_, n)| n.clone());
        let edits: Vec<String> = array(&c["edits"])
            .iter()
            .map(|e| {
                let value = serde_json::from_value::<Length>(e["value"].clone())
                    .map(format_length)
                    .unwrap_or_else(|_| e["value"].to_string());
                format!("{} to {value}", self.target(&e["target"]))
            })
            .collect();
        if !edits.is_empty() {
            line.push_str(&format!(": set {}", edits.join(", ")));
        }
        let moved: Vec<String> = array(&c["moved"])
            .iter()
            .filter_map(uuid)
            .map(|id| self.id(id))
            .collect();
        if !moved.is_empty() {
            line.push_str(&format!("; moves {}", moved.join(", ")));
        }
        if choice.is_some_and(|(id, _)| Some(id) == default) {
            line.push_str(" (default)");
        }
        line
    }
}

fn short(id: Uuid) -> String {
    id.to_string().chars().take(8).collect()
}

/// "reference_semantics" reads as "Reference semantics".
fn words(snake: &str) -> String {
    let spaced = snake.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => spaced,
    }
}

impl Review {
    /// The review of `pending`, proposed against `doc`.
    pub fn new(doc: &Document, pending: Pending) -> Review {
        let result = &pending.result;
        let diff_json = array(&result["diff"]);
        let names = Names::new(doc, diff_json);

        let mut diff = Vec::new();
        for entry in diff_json {
            let collection = entry["collection"].as_str().unwrap_or_default();
            let after = entry.get("after").filter(|v| !v.is_null());
            let before = entry.get("before").filter(|v| !v.is_null());
            let name = names.entity(collection, uuid(&entry["id"]), after.or(before));
            let change = entry["change"].as_str().unwrap_or_default();
            diff.push(format!("{} {name}", words(change)));
            if change == "changed" {
                for path in array(&entry["changed_paths"])
                    .iter()
                    .filter_map(Value::as_str)
                {
                    let at = |field: &str| entry[field].pointer(path);
                    diff.push(format!(
                        "  {path}: {} -> {}",
                        text(at("before_display")),
                        text(at("after_display"))
                    ));
                }
            }
        }

        let violations = array(&result["violations"])
            .iter()
            .map(|v| {
                let kind = words(v["key"]["kind"].as_str().unwrap_or_default());
                let of: Vec<String> = array(&v["key"]["entities"])
                    .iter()
                    .filter_map(uuid)
                    .map(|id| names.id(id))
                    .collect();
                let ack = if v["acknowledged"] == Value::Bool(true) {
                    " (acknowledged)"
                } else {
                    ""
                };
                let detail = names.detail(None, &v["detail"]);
                let detail = if detail.is_empty() {
                    String::new()
                } else {
                    format!(" ({detail})")
                };
                format!("{kind}: {}{detail}{ack}", of.join(", "))
            })
            .collect();

        let mut ambiguities = Vec::new();
        let mut alternatives = Vec::new();
        for a in array(&result["ambiguities"]) {
            let kind = a["kind"].as_str().unwrap_or_default();
            let default = uuid(&a["default"]);
            let candidates = array(&a["candidates"])
                .iter()
                .map(|c| names.candidate(c, default));
            if kind == "policy_choice" {
                alternatives.extend(candidates);
            } else {
                ambiguities.push(format!("{}: which one?", words(kind)));
                ambiguities.extend(candidates.map(|c| format!("  {c}")));
            }
        }

        let mut proposed = Vec::new();
        let mut existing = Vec::new();
        for flag in array(&result["flags"]) {
            if let Some(f) = flag.get("proposed_value") {
                let collection = f["collection"].as_str().unwrap_or_default();
                let name = names.entity(collection, uuid(&f["id"]), None);
                proposed.push(format!("{name} {}", text(f.get("path"))));
            } else if let Some(f) = flag.get("existing_condition_edit") {
                let collection = f["collection"].as_str().unwrap_or_default();
                existing.push(names.entity(collection, uuid(&f["id"]), None));
            }
        }

        let consequences = array(&result["consequences"])
            .iter()
            .filter_map(uuid)
            .map(|id| names.id(id))
            .collect();
        let conflicts = dimension::conflicts(doc, &pending);

        let mut blocked = Vec::new();
        if !conflicts.lines.is_empty() {
            blocked.push("it conflicts with constraints in the model".to_string());
        }
        if array(&result["violations"])
            .iter()
            .any(|v| v["acknowledged"] != Value::Bool(true))
        {
            blocked.push("it leaves violations".to_string());
        }
        if !ambiguities.is_empty()
            && array(&result["ambiguities"]).iter().any(|a| {
                matches!(
                    a["kind"].as_str(),
                    Some("referent" | "direction" | "reference_semantics")
                )
            })
        {
            blocked.push("it has an ambiguity to resolve".to_string());
        }

        Review {
            echo: pending.echo(),
            diff,
            violations,
            ambiguities,
            alternatives,
            proposed,
            existing,
            consequences,
            conflicts,
            provisional: result["provisional"] == Value::Bool(true),
            blocked,
            pending,
        }
    }

    /// The user confirmed the echo in the panel.
    pub fn confirm(self) -> Confirmed {
        Confirmed(self.pending)
    }

    /// No ambiguities, no Proposed values, no consequence moves, and
    /// nothing blocking: what auto_commit_unambiguous may commit (9.4).
    pub fn unambiguous(&self) -> bool {
        let result = &self.pending.result;
        array(&result["ambiguities"]).is_empty()
            && self.proposed.is_empty()
            && array(&result["consequences"]).is_empty()
            && self.blocked.is_empty()
    }

    /// A commit without review, only when the user has turned on
    /// auto_commit_unambiguous and this proposal is unambiguous.
    pub fn auto(&self, settings: &Settings) -> Option<Confirmed> {
        (settings.auto_commit_unambiguous && self.unambiguous())
            .then(|| Confirmed(self.pending.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::{Review, Settings};
    use crate::session::Pending;
    use serde_json::json;

    fn review(result: serde_json::Value) -> Review {
        Review::new(
            &crate::draw::tests::empty_project("review").1,
            Pending {
                action: json!({"kind": "commands", "commands": []}),
                result,
            },
        )
    }

    #[test]
    fn auto_commit_unambiguous_is_off_by_default() {
        assert!(!Settings::default().auto_commit_unambiguous);
        let plain = review(json!({"echo": ["Create node"]}));
        assert!(plain.unambiguous());
        assert!(plain.auto(&Settings::default()).is_none());
        let on = Settings {
            auto_commit_unambiguous: true,
        };
        assert!(plain.auto(&on).is_some());
    }

    #[test]
    fn every_section_is_shown() {
        let a = "00000000-0000-4000-8000-00000000000a";
        let b = "00000000-0000-4000-8000-00000000000b";
        let r = review(json!({
            "echo": ["Move W-001 left face 1'-0\" toward +X"],
            "diff": [
                {"collection": "walls", "id": a, "change": "changed",
                 "before": {"label": "W-001"}, "after": {"label": "W-001"},
                 "before_display": {"left_extent": {"value": "0'-3\""}},
                 "after_display": {"left_extent": {"value": "0'-4\""}},
                 "changed_paths": ["/left_extent/value"]},
                {"collection": "layer_sets", "id": b, "change": "added",
                 "after": {"layers": []}, "changed_paths": []}
            ],
            "violations": [{"key": {"kind": "wall_overlap", "entities": [a]},
                            "detail": "overlap", "acknowledged": false}],
            "ambiguities": [
                {"kind": "reference_semantics", "default": null,
                 "candidates": [{"choice": a, "edits": [
                     {"target": {"kind": "constraint", "constraint": b}, "value": 304_800_000}],
                     "moved": []}]},
                {"kind": "policy_choice", "default": a,
                 "candidates": [{"choice": a, "edits": [], "moved": [a]}]}
            ],
            "flags": [
                {"proposed_value": {"collection": "walls", "id": a, "path": "/left_extent"}},
                {"existing_condition_edit": {"collection": "walls", "id": a, "baseline": []}}
            ],
            "consequences": [a],
            "conflicts": [],
            "provisional": true
        }));
        assert_eq!(r.echo, ["Move W-001 left face 1'-0\" toward +X"]);
        assert_eq!(
            r.diff,
            [
                "Changed wall W-001",
                "  /left_extent/value: 0'-3\" -> 0'-4\"",
                "Added layer set 00000000",
            ]
        );
        assert_eq!(r.violations, ["Wall overlap: wall W-001 (overlap)"]);
        assert_eq!(
            r.ambiguities,
            [
                "Reference semantics: which one?",
                "  wall W-001: set 00000000 to 1'-0\""
            ]
        );
        assert_eq!(r.alternatives, ["wall W-001; moves wall W-001 (default)"]);
        assert_eq!(r.proposed, ["wall W-001 /left_extent"]);
        assert_eq!(r.existing, ["wall W-001"]);
        assert_eq!(r.consequences, ["wall W-001"]);
        assert!(r.provisional);
        assert_eq!(
            r.blocked,
            ["it leaves violations", "it has an ambiguity to resolve"]
        );
        assert!(!r.unambiguous());
        let on = Settings {
            auto_commit_unambiguous: true,
        };
        assert!(r.auto(&on).is_none());
    }

    /// Each thing that rules out auto-commit does so on its own.
    #[test]
    fn auto_commit_needs_no_ambiguity_proposed_value_or_consequence() {
        let on = Settings {
            auto_commit_unambiguous: true,
        };
        let a = "00000000-0000-4000-8000-00000000000a";
        let cases = [
            json!({"ambiguities": [{"kind": "magnitude", "default": null, "candidates": []}]}),
            json!({"ambiguities": [{"kind": "policy_choice", "default": null, "candidates": []}]}),
            json!({"flags": [{"proposed_value": {"collection": "walls", "id": a, "path": "/x"}}]}),
            json!({"consequences": [a]}),
            json!({"violations": [{"key": {"kind": "wall_length", "entities": [a]},
                                   "detail": null, "acknowledged": false}]}),
        ];
        for case in cases {
            let r = review(case.clone());
            assert!(!r.unambiguous(), "{case}");
            assert!(r.auto(&on).is_none(), "{case}");
        }
        // An acknowledged violation and an existing-condition edit do not.
        let r = review(json!({
            "violations": [{"key": {"kind": "wall_length", "entities": [a]},
                            "detail": null, "acknowledged": true}],
            "flags": [{"existing_condition_edit": {"collection": "walls", "id": a, "baseline": []}}]
        }));
        assert!(r.unambiguous());
    }

    /// Done when: no commit path bypasses this panel except the explicit
    /// auto_commit_unambiguous setting, off by default. `Session::commit`
    /// takes a `Confirmed`, whose field is private to this module, so the
    /// compiler rejects any other path; this test checks the app's sources
    /// keep it that way.
    #[test]
    fn no_commit_path_bypasses_the_review_panel() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut sources = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("{e}")) {
            let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let code = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
            // Only the part outside test modules ships.
            let code = code.split("#[cfg(test)]").next().unwrap_or("").to_string();
            sources.push((name, code));
        }
        assert!(sources.len() > 5, "read the app's sources");
        for (name, code) in &sources {
            // The command API's commit is requested in one place only.
            let requests = code.matches("request(\"commit\"").count();
            let expected = usize::from(name == "session.rs");
            assert_eq!(requests, expected, "{name} requests a commit");
            // Only this module makes a Confirmed.
            if name != "review.rs" {
                assert!(!code.contains("Confirmed("), "{name} makes a Confirmed");
            }
            // Nothing turns the setting on but the user's checkbox.
            assert!(
                !code.contains("auto_commit_unambiguous: true")
                    && !code.contains("auto_commit_unambiguous = true"),
                "{name} turns auto-commit on"
            );
            // No other command API route that commits.
            for sub in ["\"apply\"", "\"undo\"", "\"redo\""] {
                assert!(!code.contains(sub), "{name} sends {sub}");
            }
        }
        let session = &sources
            .iter()
            .find(|(n, _)| n == "session.rs")
            .unwrap_or_else(|| panic!("session.rs"))
            .1;
        assert!(
            session.contains("pub fn commit(&self, confirmed: &Confirmed)"),
            "Session::commit takes a Confirmed"
        );
        let review = &sources
            .iter()
            .find(|(n, _)| n == "review.rs")
            .unwrap_or_else(|| panic!("review.rs"))
            .1;
        assert_eq!(
            review.matches("Confirmed(self.pending").count(),
            2,
            "made by confirm and by auto only"
        );
    }
}
