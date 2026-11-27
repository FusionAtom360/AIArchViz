//! The missing-parameter protocol (spec 7.4, 6.4; Q-41).
//!
//! Under Q-4 every parameter holds a value with a source, so a value the
//! user never gave is a catalog default with source Proposed.
//! `required_parameters` lists every such value, ordered by dependency: a
//! fixed order of entity kinds in which each kind depends only on earlier
//! ones, then by label (or id), then by field. The model is provisional while
//! the list is not empty, and the same list is the "Proposed items" every
//! export carries (7.4, 11.3, 12.4).

use serde_json::Value;

use crate::canonical::CanonicalError;
use crate::document::Document;
use crate::identity::{Label, Uuid};
use crate::length::Length;
use crate::units::format_length;

/// Why a parameter is listed.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// It holds a catalog default with source Proposed.
    ProposedDefault,
    /// It has no value and no default (a roof's pitch or low edge, T-115).
    Missing,
}

/// The unit a parameter is given in.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Length,
    Lumens,
    Kelvin,
    /// Rise over run.
    Pitch,
    None,
}

/// One parameter the user should supply or confirm (7.4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct RequiredParameter {
    /// The document field holding the entity, e.g. "walls".
    pub collection: String,
    pub entity: Uuid,
    /// The entity's label, when it has one (W-007).
    pub label: Option<String>,
    /// JSON pointer to the value within the entity, e.g. "/layers/0/thickness".
    pub field: String,
    pub reason: Reason,
    /// The proposed default, as stored (a length in nanometers).
    pub proposed: Option<Value>,
    /// The proposed default as shown to the user (feet-inches for lengths).
    pub proposed_display: Option<String>,
    pub unit: Unit,
    /// Other fields that satisfy this one when given instead (e.g. a roof's
    /// low edge instead of its pitch).
    pub alternatives: Vec<String>,
}

/// Entity kinds in dependency order (Q-41).
pub const DEPENDENCY_ORDER: &[&str] = &[
    "storeys",
    "layer_sets",
    "walls",
    "openings",
    "spaces",
    "slabs",
    "ceilings",
    "roofs",
    "element_types",
    "occurrences",
    "materials",
    "underlays",
    "camera_bookmarks",
];

/// JSON pointers, in document order, of every `{source: "proposed", value}`.
fn proposed_paths(v: &Value, at: &str, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            let sourced = map.len() == 2 && map.contains_key("value");
            if sourced && map.get("source").and_then(Value::as_str) == Some("proposed") {
                out.push(at.to_string());
                return;
            }
            for (k, x) in map {
                proposed_paths(x, &format!("{at}/{k}"), out);
            }
        }
        Value::Array(items) => {
            for (i, x) in items.iter().enumerate() {
                proposed_paths(x, &format!("{at}/{i}"), out);
            }
        }
        _ => {}
    }
}

/// Every parameter to supply or confirm, ordered by dependency.
pub fn required_parameters(document: &Document) -> Result<Vec<RequiredParameter>, CanonicalError> {
    let v = serde_json::to_value(document).map_err(|e| CanonicalError::Json(e.to_string()))?;
    let mut out = Vec::new();
    for collection in DEPENDENCY_ORDER {
        let Some(map) = v.get(*collection).and_then(Value::as_object) else {
            continue;
        };
        let mut entities: Vec<(Option<Label>, Uuid, &Value)> = map
            .iter()
            .filter_map(|(key, entity)| {
                let id: Uuid = serde_json::from_value(Value::String(key.clone())).ok()?;
                let label = entity
                    .get("label")
                    .and_then(Value::as_str)
                    .and_then(|l| Label::try_from(l.to_string()).ok());
                Some((label, id, entity))
            })
            .collect();
        entities.sort_by(|a, b| (a.0.is_none(), &a.0, a.1).cmp(&(b.0.is_none(), &b.0, b.1)));
        for (label, id, entity) in entities {
            let mut paths = Vec::new();
            proposed_paths(entity, "", &mut paths);
            for field in paths {
                let value = entity.pointer(&field).and_then(|s| s.get("value")).cloned();
                let unit = value.as_ref().map_or(Unit::None, |v| unit_of(&field, v));
                let proposed_display = value.as_ref().map(|v| match (unit, v.as_i64()) {
                    (Unit::Length, Some(n)) => format_length(Length::from_nanometers(n)),
                    (Unit::Pitch, _) => format!(
                        "{}:{}",
                        v.get("rise").unwrap_or(&Value::Null),
                        v.get("run").unwrap_or(&Value::Null)
                    ),
                    _ => v.as_str().map_or_else(|| v.to_string(), str::to_string),
                });
                out.push(RequiredParameter {
                    collection: (*collection).to_string(),
                    entity: id,
                    label: label.as_ref().map(ToString::to_string),
                    field,
                    reason: Reason::ProposedDefault,
                    proposed: value,
                    proposed_display,
                    unit,
                    alternatives: Vec::new(),
                });
            }
            // A roof needs a pitch or a low edge (6.5, Q-44).
            let absent = |k: &str| entity.get(k).is_none_or(Value::is_null);
            if *collection == "roofs" && absent("pitch") && absent("low_edge_height") {
                out.push(RequiredParameter {
                    collection: (*collection).to_string(),
                    entity: id,
                    label: None,
                    field: "/pitch".to_string(),
                    reason: Reason::Missing,
                    proposed: None,
                    proposed_display: None,
                    unit: Unit::Pitch,
                    alternatives: vec!["/low_edge_height".to_string()],
                });
            }
        }
    }
    Ok(out)
}

/// The unit of the value at `path`.
fn unit_of(path: &str, value: &Value) -> Unit {
    if path.ends_with("/lumens") {
        Unit::Lumens
    } else if path.ends_with("/kelvin") {
        Unit::Kelvin
    } else if value.get("rise").is_some() && value.get("run").is_some() {
        Unit::Pitch
    } else if value.is_i64() || value.is_u64() {
        Unit::Length
    } else {
        Unit::None
    }
}

/// Whether the model rests on any Proposed value (7.4).
pub fn is_provisional(document: &Document) -> bool {
    required_parameters(document).is_ok_and(|r| !r.is_empty())
}

/// The Proposed items every export of a provisional model lists (7.4).
pub fn proposed_items(document: &Document) -> Result<Vec<RequiredParameter>, CanonicalError> {
    Ok(required_parameters(document)?
        .into_iter()
        .filter(|p| p.reason == Reason::ProposedDefault)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::tests::{cmd, with_wall};
    use crate::command::{Action, CreateOpening, SetWallLayers};
    use crate::defaults::interior_wall;
    use crate::document::*;
    use crate::length::Length;
    use crate::log::History;
    use crate::proposal::{ProposedAction, commit, propose};

    const IN: i64 = 25_400_000;

    fn uid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    /// with_wall, but its layers and left extent are catalog defaults.
    fn no_wall_thickness() -> (Document, Uuid) {
        let (mut doc, wall, _) = with_wall();
        let (layers, left) = interior_wall(uid(25), uid(30));
        if let Some(set) = doc.layer_sets.get_mut(&uid(24)) {
            set.layers = layers;
        }
        if let Some(w) = doc.walls.get_mut(&wall) {
            w.left_extent = left;
        }
        (doc, wall)
    }

    fn fields(r: &[RequiredParameter]) -> Vec<(String, Uuid, String)> {
        r.iter()
            .map(|p| (p.collection.clone(), p.entity, p.field.clone()))
            .collect()
    }

    fn user(inches_times_4: i64) -> Sourced<Length> {
        Sourced {
            value: Length::from_nanometers(inches_times_4 * IN / 4),
            source: Source::UserStated,
        }
    }

    /// Done when: a plan with no wall thickness yields a provisional model
    /// and a prompt list, and supplying the value clears its Proposed flag.
    #[test]
    fn no_wall_thickness_is_provisional_until_supplied() -> Result<(), String> {
        let (doc, wall) = no_wall_thickness();
        assert!(is_provisional(&doc));
        let r = required_parameters(&doc).map_err(|e| e.to_string())?;
        let set = uid(24);
        assert_eq!(
            fields(&r),
            vec![
                ("layer_sets".into(), set, "/layers/0/thickness".into()),
                ("layer_sets".into(), set, "/layers/1/thickness".into()),
                ("layer_sets".into(), set, "/layers/2/thickness".into()),
                ("walls".into(), wall, "/left_extent".into()),
            ]
        );
        let shown: Vec<Option<String>> = r.iter().map(|p| p.proposed_display.clone()).collect();
        let s = |t: &str| Some(t.to_string());
        assert_eq!(
            shown,
            vec![
                s("0'-0 1/2\""),
                s("0'-3 1/2\""),
                s("0'-0 1/2\""),
                s("0'-2 1/4\"")
            ]
        );
        assert!(
            r.iter()
                .all(|p| p.unit == Unit::Length && p.reason == Reason::ProposedDefault)
        );
        assert_eq!(r.last().and_then(|p| p.label.clone()), s("W-001"));
        assert_eq!(
            r.first().and_then(|p| p.proposed.clone()),
            Some(Value::from(12_700_000_i64))
        );
        assert_eq!(proposed_items(&doc).map_err(|e| e.to_string())?, r);

        // The user states the wall: 1/2" + 5 1/2" + 1/2", reference 3 1/4" from the left face.
        let mut history = History::new(doc.clone());
        let layers = vec![
            Layer {
                material: uid(25),
                thickness: user(2),
                core: false,
            },
            Layer {
                material: uid(30),
                thickness: user(22),
                core: true,
            },
            Layer {
                material: uid(25),
                thickness: user(2),
                core: false,
            },
        ];
        let supply = ProposedAction::Commands {
            commands: vec![cmd(
                1,
                Action::SetWallLayers(SetWallLayers {
                    wall,
                    layers,
                    left_extent: user(13),
                }),
            )],
        };
        let before = propose(&history, &ProposedAction::Commands { commands: vec![] })
            .map_err(|e| e.to_string())?;
        assert!(before.provisional);
        let p = propose(&history, &supply).map_err(|e| e.to_string())?;
        assert!(!p.provisional);
        commit(&mut history, &p, 1_000, "user").map_err(|e| e.to_string())?;
        assert_eq!(
            required_parameters(history.document()).map_err(|e| e.to_string())?,
            vec![]
        );
        assert!(!is_provisional(history.document()));
        Ok(())
    }

    #[test]
    fn parameters_come_in_dependency_order() -> Result<(), String> {
        let (mut doc, wall) = no_wall_thickness();
        let (storey, a) = (uid(23), uid(20));
        if let Some(st) = doc.storeys.get_mut(&storey) {
            st.default_ceiling_height.source = Source::Proposed;
        }
        let door = cmd(
            5,
            Action::CreateOpening(CreateOpening {
                kind: OpeningKind::Door,
                host: wall,
                reference: Reference::Node { node: a },
                anchor: OpeningEdge::Start,
                offset: user(16 * 4 * 3),
                width: None,
                height: None,
                sill: None,
            }),
        );
        let doc = crate::command::apply(&doc, &door).map_err(|e| e.to_string())?;
        let kinds: Vec<String> = required_parameters(&doc)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|p| p.collection)
            .collect();
        assert_eq!(
            kinds,
            [
                "storeys",
                "layer_sets",
                "layer_sets",
                "layer_sets",
                "walls",
                "openings",
                "openings",
                "openings"
            ]
        );
        Ok(())
    }

    #[test]
    fn units_follow_the_value() {
        assert_eq!(unit_of("/light/lumens", &Value::from(800)), Unit::Lumens);
        assert_eq!(unit_of("/light/kelvin", &Value::from(4000)), Unit::Kelvin);
        assert_eq!(
            unit_of("/pitch", &serde_json::json!({"rise": 3, "run": 12})),
            Unit::Pitch
        );
        assert_eq!(
            unit_of("/slope_direction", &Value::from("pos_y")),
            Unit::None
        );
        assert_eq!(
            unit_of("/width", &Value::from(914_400_000_i64)),
            Unit::Length
        );
    }

    #[test]
    fn set_wall_layers_copies_a_shared_set() -> Result<(), String> {
        let (mut doc, wall) = no_wall_thickness();
        let mut other = doc.walls.get(&wall).cloned().ok_or("no wall")?;
        other.node_a = uid(20);
        let other_id = uid(41);
        doc.walls.insert(other_id, other);
        let c = cmd(
            2,
            Action::SetWallLayers(SetWallLayers {
                wall,
                layers: vec![Layer {
                    material: uid(30),
                    thickness: user(16),
                    core: true,
                }],
                left_extent: user(8),
            }),
        );
        let after = crate::command::apply(&doc, &c).map_err(|e| e.to_string())?;
        let new_set = Uuid::derive(c.idempotency_key, "command", "0");
        assert_eq!(after.walls.get(&wall).map(|w| w.layer_set), Some(new_set));
        assert_eq!(
            after.layer_sets.get(&new_set).map(|s| s.layers.len()),
            Some(1)
        );
        // The other wall keeps the shared set, unchanged.
        assert_eq!(
            after.walls.get(&other_id).map(|w| w.layer_set),
            Some(uid(24))
        );
        assert_eq!(after.layer_sets.get(&uid(24)), doc.layer_sets.get(&uid(24)));
        // A set used by one wall only is edited in place.
        let (doc, wall) = no_wall_thickness();
        let after = crate::command::apply(&doc, &c).map_err(|e| e.to_string())?;
        assert_eq!(after.walls.get(&wall).map(|w| w.layer_set), Some(uid(24)));
        assert_eq!(
            after.layer_sets.get(&uid(24)).map(|s| s.layers.len()),
            Some(1)
        );
        Ok(())
    }
}
