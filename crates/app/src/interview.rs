//! The interview panel (T-203, spec 7.4, 6.1, 6.4; Q-78 (x)).
//!
//! The panel lists every required parameter with its proposed default (the
//! command API's `required-parameters`). Walls are grouped by their layers:
//! walls whose layers are the same (material, thickness, core flag, in
//! order) form one group, so four catalog walls are one row. Typing a total
//! thickness for a group sets the core layer to the total less the non-core
//! layers; typing one layer sets that layer alone. Either way every wall in
//! the group gets the new layers through SetWallLayers, the typed thickness
//! with source UserStated, and its left extent set so the reference line
//! stays on the core center (source Derived: it follows from the layers).

use archviz_core::{
    Action, Document, Layer, Length, SetWallLayers, Source, Sourced, Uuid, format_length,
};

/// Walls with the same layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WallGroup {
    /// (wall, label), in label order.
    pub walls: Vec<(Uuid, String)>,
    /// Left to right.
    pub layers: Vec<Layer>,
}

/// Every wall, grouped by layers; groups in the order of their first label.
pub fn wall_groups(doc: &Document) -> Vec<WallGroup> {
    let mut walls: Vec<(String, Uuid, Vec<Layer>)> = doc
        .walls
        .iter()
        .filter_map(|(id, w)| {
            let set = doc.layer_sets.get(&w.layer_set)?;
            Some((w.label.to_string(), *id, set.layers.clone()))
        })
        .collect();
    walls.sort_by(|a, b| (&a.0, a.1).cmp(&(&b.0, b.1)));
    let mut groups: Vec<WallGroup> = Vec::new();
    for (label, id, layers) in walls {
        match groups.iter_mut().find(|g| g.layers == layers) {
            Some(g) => g.walls.push((id, label)),
            None => groups.push(WallGroup {
                walls: vec![(id, label)],
                layers,
            }),
        }
    }
    groups
}

fn sum(layers: &[Layer]) -> Result<Length, String> {
    layers
        .iter()
        .try_fold(Length::from_nanometers(0), |acc, l| {
            acc.checked_add(l.thickness.value)
                .map_err(|e| format!("{e:?}"))
        })
}

/// The total thickness of `layers`.
pub fn total(layers: &[Layer]) -> Result<Length, String> {
    sum(layers)
}

/// Whether any layer still holds a Proposed value.
pub fn proposed(layers: &[Layer]) -> bool {
    layers
        .iter()
        .any(|l| l.thickness.source == Source::Proposed)
}

fn core_index(layers: &[Layer]) -> Result<usize, String> {
    let mut cores = layers.iter().enumerate().filter(|(_, l)| l.core);
    match (cores.next(), cores.next()) {
        (Some((i, _)), None) => Ok(i),
        _ => Err("a total thickness needs exactly one core layer".to_string()),
    }
}

/// `layers` with the core set to `total` less the other layers, UserStated.
pub fn with_total(layers: &[Layer], total: Length) -> Result<Vec<Layer>, String> {
    let i = core_index(layers)?;
    let others: Vec<Layer> = layers
        .iter()
        .enumerate()
        .filter(|(j, _)| *j != i)
        .map(|(_, l)| l.clone())
        .collect();
    let core = total
        .checked_sub(sum(&others)?)
        .map_err(|e| format!("{e:?}"))?;
    if core.nanometers() <= 0 {
        return Err(format!(
            "the total must exceed the other layers, {}",
            format_length(sum(&others)?)
        ));
    }
    with_layer(layers, i, core)
}

/// `layers` with layer `index` set to `thickness`, UserStated.
pub fn with_layer(layers: &[Layer], index: usize, thickness: Length) -> Result<Vec<Layer>, String> {
    if thickness.nanometers() <= 0 {
        return Err("a layer must be thicker than zero".to_string());
    }
    let mut out = layers.to_vec();
    let layer = out
        .get_mut(index)
        .ok_or_else(|| "no such layer".to_string())?;
    layer.thickness = Sourced {
        value: thickness,
        source: Source::UserStated,
    };
    Ok(out)
}

/// The left extent that puts the reference line on the core center: the
/// layers left of the core plus half the core (6.1). Derived.
pub fn core_center(layers: &[Layer]) -> Result<Sourced<Length>, String> {
    let i = core_index(layers)?;
    let left = sum(layers.get(..i).unwrap_or_default())?;
    let half = layers
        .get(i)
        .map(|l| l.thickness.value)
        .ok_or_else(|| "no core layer".to_string())?
        .checked_div(2)
        .map_err(|e| format!("{e:?}"))?;
    Ok(Sourced {
        value: left.checked_add(half).map_err(|e| format!("{e:?}"))?,
        source: Source::Derived,
    })
}

/// One SetWallLayers per wall of `group`, giving it `layers` with the
/// reference line on the core center.
pub fn set_layers(group: &WallGroup, layers: &[Layer]) -> Result<Vec<Action>, String> {
    let left_extent = core_center(layers)?;
    Ok(group
        .walls
        .iter()
        .map(|(wall, _)| {
            Action::SetWallLayers(SetWallLayers {
                wall: *wall,
                layers: layers.to_vec(),
                left_extent: left_extent.clone(),
            })
        })
        .collect())
}

/// A required parameter as the panel lists it: what, which field, and the
/// proposed default (`required-parameters` result rows, 7.4).
pub fn rows(result: &serde_json::Value) -> Vec<(String, String, String)> {
    result["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let what = p["label"].as_str().map_or_else(
                || p["collection"].as_str().unwrap_or("").to_string(),
                str::to_string,
            );
            let field = p["field"].as_str().unwrap_or("").to_string();
            let default = p["proposed_display"]
                .as_str()
                .unwrap_or("needs a value")
                .to_string();
            (what, field, default)
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::draw::tests::empty_project;
    use crate::draw::{Run, run_commands, setup_actions, storey_for};
    use crate::session::Session;
    use archviz_core::Command;

    const IN: i64 = 25_400_000;

    fn l(n: i64) -> Length {
        Length::from_nanometers(n)
    }

    fn layer(n: i64, core: bool) -> Layer {
        Layer {
            material: Uuid::from_bytes([u8::from(core); 16]),
            thickness: Sourced {
                value: l(n),
                source: Source::Proposed,
            },
            core,
        }
    }

    /// 1/2" gypsum, 3 1/2" stud (core), 1/2" gypsum, as the catalog wall.
    fn catalog() -> Vec<Layer> {
        vec![
            layer(IN / 2, false),
            layer(7 * IN / 2, true),
            layer(IN / 2, false),
        ]
    }

    /// A project with a storey, the origin datum and four catalog walls,
    /// all committed through the command API.
    pub(crate) fn four_walls() -> Document {
        let (dir, _) = empty_project("interview");
        let session = Session::open(&dir, Box::new(|| {})).unwrap_or_else(|e| panic!("{e:?}"));
        let wrap = |actions: Vec<Action>| -> Vec<Command> {
            actions
                .into_iter()
                .map(|action| {
                    let (id, idempotency_key) = session.ids();
                    Command {
                        id,
                        idempotency_key,
                        action,
                    }
                })
                .collect()
        };
        let p = session
            .propose_commands(wrap(setup_actions(&session.document())))
            .unwrap_or_else(|e| panic!("{e}"));
        session
            .commit(&crate::session::confirmed(&session, p.clone()))
            .unwrap_or_else(|e| panic!("{e}"));
        let storey = storey_for(&session.document()).unwrap_or_else(|| panic!());
        let ft = 12 * IN;
        let run = Run {
            points: vec![(0, 0), (8 * ft, 0), (8 * ft, 6 * ft), (0, 6 * ft)],
            closed: true,
        };
        let walls = run_commands(&run, storey, || session.ids()).unwrap_or_else(|| panic!());
        let p = session
            .propose_commands(walls)
            .unwrap_or_else(|e| panic!("{e}"));
        session
            .commit(&crate::session::confirmed(&session, p.clone()))
            .unwrap_or_else(|e| panic!("{e}"));
        session.document()
    }

    #[test]
    fn a_total_sets_the_core_and_keeps_the_finishes() {
        let six = with_total(&catalog(), l(6 * IN)).unwrap_or_else(|e| panic!("{e}"));
        let t: Vec<(i64, Source)> = six
            .iter()
            .map(|x| (x.thickness.value.nanometers(), x.thickness.source))
            .collect();
        assert_eq!(
            t,
            [
                (IN / 2, Source::Proposed),
                (5 * IN, Source::UserStated),
                (IN / 2, Source::Proposed)
            ]
        );
        assert_eq!(total(&six), Ok(l(6 * IN)));
        let center = core_center(&six).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(center.value, l(3 * IN));
        assert_eq!(center.source, Source::Derived);
        assert_eq!(
            core_center(&catalog()).map(|c| c.value),
            Ok(l(9 * IN / 4)),
            "the catalog's 2 1/4 in"
        );
    }

    #[test]
    fn an_odd_core_puts_the_reference_line_off_the_grid() {
        // 4 1/16" total: core 3 1/16", center 2 1/32" from the left face.
        let odd = with_total(&catalog(), l(65 * IN / 16)).unwrap_or_else(|e| panic!("{e}"));
        let c = core_center(&odd).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.value, l(IN / 2 + 49 * IN / 32));
        assert_eq!(format_length(c.value), "~0'-2\"");
    }

    #[test]
    fn refused_thicknesses() {
        assert_eq!(
            with_total(&catalog(), l(IN)),
            Err("the total must exceed the other layers, 0'-1\"".to_string())
        );
        assert!(with_total(&catalog(), l(-IN)).is_err());
        let two_cores = vec![layer(IN, true), layer(IN, true)];
        assert_eq!(
            with_total(&two_cores, l(4 * IN)),
            Err("a total thickness needs exactly one core layer".to_string())
        );
        assert!(with_total(&[layer(IN, false)], l(4 * IN)).is_err());
        assert!(with_layer(&catalog(), 0, l(0)).is_err());
        assert!(with_layer(&catalog(), 3, l(IN)).is_err());
    }

    #[test]
    fn one_layer_stays_editable_on_its_own() {
        let five_eighths =
            with_layer(&catalog(), 2, l(5 * IN / 8)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(five_eighths[1], catalog()[1], "the core is untouched");
        assert_eq!(five_eighths[2].thickness.source, Source::UserStated);
        assert_eq!(total(&five_eighths), Ok(l(4 * IN + 5 * IN / 8)));
        // The reference line stays on the core center: 1/2 + 1 3/4.
        assert_eq!(
            core_center(&five_eighths).map(|c| c.value),
            Ok(l(9 * IN / 4))
        );
    }

    #[test]
    fn walls_with_the_same_layers_are_one_group() {
        let mut doc = four_walls();
        let groups = wall_groups(&doc);
        assert_eq!(groups.len(), 1);
        let labels: Vec<&str> = groups[0].walls.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, ["W-001", "W-002", "W-003", "W-004"]);
        assert!(proposed(&groups[0].layers));
        // Give W-003 its own layer set with a thicker core: two groups.
        let (w3, _) = groups[0].walls[2].clone();
        let set = doc.walls[&w3].layer_set;
        let mut own = doc.layer_sets[&set].clone();
        own.layers[1].thickness.value = l(11 * IN / 2);
        let id = Uuid::from_bytes([77; 16]);
        doc.layer_sets.insert(id, own);
        if let Some(w) = doc.walls.get_mut(&w3) {
            w.layer_set = id;
        }
        let groups = wall_groups(&doc);
        let labels: Vec<Vec<&str>> = groups
            .iter()
            .map(|g| g.walls.iter().map(|(_, l)| l.as_str()).collect())
            .collect();
        assert_eq!(labels, [vec!["W-001", "W-002", "W-004"], vec!["W-003"]]);
        let actions = set_layers(&groups[0], &groups[0].layers).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(actions.len(), 3);
        assert!(actions.iter().all(|a| a.kind() == "set_wall_layers"));
    }

    #[test]
    fn rows_list_each_parameter_with_its_default() {
        let result = serde_json::json!({"parameters": [
            {"collection": "storeys", "label": null, "field": "/elevation", "proposed_display": "0'-0\""},
            {"collection": "walls", "label": "W-001", "field": "/left_extent", "proposed_display": "0'-2 1/4\""},
            {"collection": "roofs", "label": null, "field": "/pitch", "proposed_display": null},
        ]});
        assert_eq!(
            rows(&result),
            [
                ("storeys".into(), "/elevation".into(), "0'-0\"".into()),
                ("W-001".into(), "/left_extent".into(), "0'-2 1/4\"".into()),
                ("roofs".into(), "/pitch".into(), "needs a value".into()),
            ]
        );
    }
}
