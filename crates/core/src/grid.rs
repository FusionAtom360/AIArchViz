//! The 1/16 in grid rule for user-stated command values (spec 5.3, T-190).
//!
//! The text parser already rejects user input finer than the grid; this is
//! the same rule for commands, which carry nanometers. `propose` runs it on
//! every submitted command, before defaults are filled or moves resolved, so
//! commands the System generates are not checked. User-stated means:
//!
//! - every `Sourced<Length>` with source UserStated;
//! - the length values of an AddConstraint with source UserStated;
//! - the value of a SetConstraintValue (it states the dimension);
//! - the delta or target of a move;
//! - declared dimensions with authority UserStatedExact.
//!
//! Derived, Measured, Imported and Proposed values, and manufacturer or CAD
//! dimensions, are exempt.

use crate::command::{Action, CommandError, Motion};
use crate::create::{CreateRoof, CreateWall};
use crate::document::{
    Authority, Clearance, ConstraintKind, Layer, Overhang, Placement, Source, Sourced, WallTop,
};
use crate::length::Length;
use crate::units::{GRID_NM, round_half_even};

/// User-stated lengths of one command, as (JSON pointer in the command, value).
struct Stated(Vec<(String, Length)>);

impl Stated {
    fn always(&mut self, at: &str, v: Length) {
        self.0.push((at.to_string(), v));
    }

    fn sourced(&mut self, at: &str, s: &Sourced<Length>) {
        if s.source == Source::UserStated {
            self.always(at, s.value);
        }
    }

    fn option(&mut self, at: &str, s: &Option<Sourced<Length>>) {
        if let Some(s) = s {
            self.sourced(at, s);
        }
    }

    fn layers(&mut self, at: &str, layers: &[Layer]) {
        for (i, l) in layers.iter().enumerate() {
            self.sourced(&format!("{at}/{i}/thickness"), &l.thickness);
        }
    }

    fn motion(&mut self, m: &Motion) {
        match m {
            Motion::Delta { delta } => self.always("/args/motion/delta", *delta),
            Motion::Target { target } => self.always("/args/motion/target", *target),
        }
    }

    fn placement(&mut self, p: &Placement) {
        match p {
            Placement::Floor {
                x, y, elevation, ..
            } => {
                self.sourced("/args/placement/x", x);
                self.sourced("/args/placement/y", y);
                self.sourced("/args/placement/elevation", elevation);
            }
            Placement::WallMounted {
                offset, elevation, ..
            } => {
                self.sourced("/args/placement/offset", offset);
                self.sourced("/args/placement/elevation", elevation);
            }
            Placement::CeilingMounted { x, y } => {
                self.sourced("/args/placement/x", x);
                self.sourced("/args/placement/y", y);
            }
        }
    }

    fn clearance(&mut self, c: &Clearance) {
        self.sourced("/args/clearance/front", &c.front);
        self.sourced("/args/clearance/back", &c.back);
        self.sourced("/args/clearance/left", &c.left);
        self.sourced("/args/clearance/right", &c.right);
    }

    fn overhang(&mut self, o: &Overhang) {
        self.sourced("/args/overhang/pos_x", &o.pos_x);
        self.sourced("/args/overhang/neg_x", &o.neg_x);
        self.sourced("/args/overhang/pos_y", &o.pos_y);
        self.sourced("/args/overhang/neg_y", &o.neg_y);
    }

    fn wall(&mut self, w: &CreateWall) {
        if let Some(layers) = &w.layers {
            self.layers("/args/layers", layers);
        }
        self.option("/args/left_extent", &w.left_extent);
        self.option("/args/base_offset", &w.base_offset);
        if let Some(WallTop::Height { height }) = &w.top {
            self.sourced("/args/top/height", height);
        }
    }

    fn roof(&mut self, r: &CreateRoof) {
        self.option("/args/high_edge_height", &r.high_edge_height);
        self.option("/args/low_edge_height", &r.low_edge_height);
        if let Some(o) = &r.overhang {
            self.overhang(o);
        }
        self.option("/args/thickness", &r.thickness);
        self.option("/args/fascia_depth", &r.fascia_depth);
    }
}

/// Every user-stated length in `action`, in field order. The match is
/// exhaustive so that a new command kind has to be considered here.
fn user_stated(action: &Action) -> Vec<(String, Length)> {
    let mut s = Stated(Vec::new());
    match action {
        // Node positions are provisional until solved; datums, spaces and
        // the rest carry no length with a source.
        Action::CreateNode(_)
        | Action::SplitWall(_)
        | Action::MergeWalls(_)
        | Action::RotateOccurrence(_)
        | Action::DeleteEntity(_)
        | Action::RemoveConstraint(_)
        | Action::ApplyOperator(_)
        | Action::RemoveOperator(_)
        | Action::SetVariantSelection(_)
        | Action::SetProjectUnits(_)
        | Action::CreateDatum(_)
        | Action::CreateSpace(_) => {}
        Action::CreateWall(w) => s.wall(w),
        Action::SetWallLayers(w) => {
            s.layers("/args/layers", &w.layers);
            s.sourced("/args/left_extent", &w.left_extent);
        }
        Action::MoveWall(m) => s.motion(&m.motion),
        Action::MoveOpening(m) => s.motion(&m.motion),
        Action::MoveOccurrence(m) => s.motion(&m.motion),
        Action::CreateOpening(o) => {
            s.sourced("/args/offset", &o.offset);
            s.option("/args/width", &o.width);
            s.option("/args/height", &o.height);
            s.option("/args/sill", &o.sill);
        }
        Action::PlaceOccurrence(o) => {
            s.placement(&o.placement);
            if let Some(c) = &o.clearance {
                s.clearance(c);
            }
        }
        Action::AddConstraint(c) if c.source == Source::UserStated => match &c.kind {
            ConstraintKind::Distance { value, .. }
            | ConstraintKind::Fixed { value, .. }
            | ConstraintKind::MinClearance { value, .. } => s.always("/args/kind/value", *value),
            ConstraintKind::Align { .. } | ConstraintKind::Equal { .. } => {}
        },
        Action::AddConstraint(_) => {}
        Action::SetConstraintValue(v) => s.always("/args/value", v.value),
        Action::SetStorey(st) => {
            s.sourced("/args/elevation", &st.elevation);
            s.sourced("/args/floor_to_floor", &st.floor_to_floor);
            s.sourced("/args/default_ceiling_height", &st.default_ceiling_height);
        }
        Action::CreateMaterial(m) => s.option("/args/tile_size", &m.tile_size),
        Action::CreateSlab(sl) => {
            s.option("/args/thickness", &sl.thickness);
            s.option("/args/elevation", &sl.elevation);
        }
        Action::CreateCeiling(c) => {
            s.option("/args/thickness", &c.thickness);
            s.option("/args/elevation", &c.elevation);
        }
        Action::CreateRoof(r) => s.roof(r),
        Action::CreateUnderlay(u) => {
            s.sourced("/args/calibration/distance", &u.calibration.distance);
            if let Some(k) = &u.check {
                s.sourced("/args/check/distance", &k.distance);
            }
        }
        Action::SetUnderlayCalibration(u) => {
            s.sourced("/args/calibration/distance", &u.calibration.distance);
            if let Some(k) = &u.check {
                s.sourced("/args/check/distance", &k.distance);
            }
        }
        Action::CreateElementType(t) => {
            let d = &t.declared_dimensions;
            if d.authority == Authority::UserStatedExact {
                s.always("/args/declared_dimensions/width", d.width);
                s.always("/args/declared_dimensions/depth", d.depth);
                s.always("/args/declared_dimensions/height", d.height);
            }
        }
    }
    s.0
}

/// The grid value nearest `v`, ties to even as the parser rounds (5.3). At
/// the ends of the range, where that value does not fit, the next grid value
/// toward zero.
pub(crate) fn nearest_grid(v: Length) -> Length {
    let n = i128::from(v.nanometers());
    let d = i128::from(GRID_NM);
    let fit = |q: i128| i64::try_from(q.saturating_mul(d)).ok();
    let nm = fit(round_half_even(n, d))
        .or_else(|| fit(n / d))
        .unwrap_or(0);
    Length::from_nanometers(nm)
}

/// The first user-stated length in `action` off the 1/16 in grid, as an
/// error naming it and the nearest grid value (5.3).
pub(crate) fn check_user_grid(action: &Action) -> Result<(), CommandError> {
    match user_stated(action)
        .into_iter()
        .find(|(_, v)| v.nanometers() % GRID_NM != 0)
    {
        Some((argument, value)) => Err(CommandError::OffGrid {
            argument,
            value,
            nearest: nearest_grid(value),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const ID: &str = "01010101-0101-0101-0101-010101010101";
    /// 240 in: on the grid.
    const ON: i64 = 6_096_000_000;

    /// Builds a command with the given value in the field under test.
    type Make = Box<dyn Fn(i64) -> Action>;

    fn user(v: i64) -> Value {
        json!({ "source": "user_stated", "value": v })
    }

    fn action(kind: &str, args: Value) -> Action {
        serde_json::from_value(json!({ "kind": kind, "args": args }))
            .unwrap_or_else(|e| panic!("{kind}: {e}"))
    }

    fn reference() -> Value {
        json!({ "kind": "node", "node": ID })
    }

    fn layers(v: Value) -> Value {
        json!([{ "material": ID, "thickness": v, "core": true }])
    }

    fn element_type(authority: &str, v: i64) -> Value {
        json!({
            "name": "Bench",
            "declared_dimensions": { "width": v, "depth": ON, "height": ON, "authority": authority },
            "source": "G-2",
            "license": "n/a",
        })
    }

    fn constraint(source: &str, v: i64) -> Value {
        json!({
            "kind": { "kind": "distance", "a": reference(), "b": reference(), "axis": "x", "value": v, "tolerance": null },
            "source": source,
        })
    }

    fn wall(field: &str, v: Value) -> Value {
        let mut args = json!({
            "storey": ID, "node_a": ID, "node_b": ID, "direction": "pos_x", "function": "new",
            "layers": layers(user(ON)), "left_extent": user(ON),
        });
        args[field] = v;
        args
    }

    fn roof(field: &str, v: Value) -> Value {
        let mut args = json!({
            "storey": ID,
            "footprint": { "kind": "from_space", "space": ID },
            "slope_direction": "neg_y",
        });
        args[field] = v;
        args
    }

    fn mv(kind: &str, motion: &str, v: i64) -> Action {
        let field = match kind {
            "move_wall" => json!({ "wall": ID, "reference": reference() }),
            "move_opening" => json!({ "opening": ID }),
            _ => json!({ "occurrence": ID, "axis": "x" }),
        };
        let mut args = field;
        args["motion"] = json!({ "kind": motion, motion: v });
        args["edit"] = Value::Null;
        args["absorb"] = json!([]);
        action(kind, args)
    }

    /// (pointer, command with value `v` there), one per user-stated field
    /// kind of every command.
    fn cases() -> Vec<(&'static str, Make)> {
        let storey = |field: &'static str| {
            move |v: i64| {
                let mut args = json!({
                    "storey": null, "name": "Ground", "elevation": user(0),
                    "floor_to_floor": user(ON), "default_ceiling_height": user(ON),
                });
                args[field] = user(v);
                action("set_storey", args)
            }
        };
        let opening = |field: &'static str| {
            move |v: i64| {
                let mut args = json!({
                    "kind": "door", "host": ID, "reference": reference(), "anchor": "start",
                    "offset": user(ON), "width": null, "height": null, "sill": null,
                });
                args[field] = user(v);
                action("create_opening", args)
            }
        };
        let occurrence = |placement: Value, clearance: Value| {
            action(
                "place_occurrence",
                json!({ "storey": ID, "element_type": ID, "placement": placement, "clearance": clearance }),
            )
        };
        let floor = |field: &'static str| {
            move |v: i64| {
                let mut p = json!({ "kind": "floor", "x": user(ON), "y": user(ON), "rotation": 0, "elevation": user(0) });
                p[field] = user(v);
                occurrence(p, Value::Null)
            }
        };
        let slab = |kind: &'static str, field: &'static str| {
            move |v: i64| {
                let mut args =
                    json!({ "storey": ID, "boundary": { "kind": "from_space", "space": ID } });
                args[field] = user(v);
                action(kind, args)
            }
        };
        let roof_field =
            |field: &'static str| move |v: i64| action("create_roof", roof(field, user(v)));
        let overhang = |v: i64| {
            let o =
                json!({ "pos_x": user(0), "neg_x": user(0), "pos_y": user(0), "neg_y": user(v) });
            action("create_roof", roof("overhang", o))
        };
        vec![
            ("/args/elevation", Box::new(storey("elevation"))),
            ("/args/floor_to_floor", Box::new(storey("floor_to_floor"))),
            (
                "/args/default_ceiling_height",
                Box::new(storey("default_ceiling_height")),
            ),
            (
                "/args/layers/0/thickness",
                Box::new(|v| action("create_wall", wall("layers", layers(user(v))))),
            ),
            (
                "/args/left_extent",
                Box::new(|v| action("create_wall", wall("left_extent", user(v)))),
            ),
            (
                "/args/base_offset",
                Box::new(|v| action("create_wall", wall("base_offset", user(v)))),
            ),
            (
                "/args/top/height",
                Box::new(|v| {
                    action(
                        "create_wall",
                        wall("top", json!({ "kind": "height", "height": user(v) })),
                    )
                }),
            ),
            (
                "/args/layers/0/thickness",
                Box::new(|v| {
                    action(
                        "set_wall_layers",
                        json!({ "wall": ID, "layers": layers(user(v)), "left_extent": user(ON) }),
                    )
                }),
            ),
            (
                "/args/left_extent",
                Box::new(|v| {
                    action(
                        "set_wall_layers",
                        json!({ "wall": ID, "layers": layers(user(ON)), "left_extent": user(v) }),
                    )
                }),
            ),
            (
                "/args/motion/delta",
                Box::new(|v| mv("move_wall", "delta", v)),
            ),
            (
                "/args/motion/target",
                Box::new(|v| mv("move_wall", "target", v)),
            ),
            (
                "/args/motion/delta",
                Box::new(|v| mv("move_opening", "delta", v)),
            ),
            (
                "/args/motion/target",
                Box::new(|v| mv("move_occurrence", "target", v)),
            ),
            ("/args/offset", Box::new(opening("offset"))),
            ("/args/width", Box::new(opening("width"))),
            ("/args/height", Box::new(opening("height"))),
            ("/args/sill", Box::new(opening("sill"))),
            ("/args/placement/x", Box::new(floor("x"))),
            ("/args/placement/y", Box::new(floor("y"))),
            ("/args/placement/elevation", Box::new(floor("elevation"))),
            (
                "/args/placement/offset",
                Box::new(move |v| {
                    occurrence(
                        json!({ "kind": "wall_mounted", "wall": ID, "side": "left", "reference": reference(), "offset": user(v), "elevation": user(ON) }),
                        Value::Null,
                    )
                }),
            ),
            (
                "/args/placement/elevation",
                Box::new(move |v| {
                    occurrence(
                        json!({ "kind": "wall_mounted", "wall": ID, "side": "left", "reference": reference(), "offset": user(ON), "elevation": user(v) }),
                        Value::Null,
                    )
                }),
            ),
            (
                "/args/placement/y",
                Box::new(move |v| {
                    occurrence(
                        json!({ "kind": "ceiling_mounted", "x": user(ON), "y": user(v) }),
                        Value::Null,
                    )
                }),
            ),
            (
                "/args/clearance/right",
                Box::new(move |v| {
                    occurrence(
                        json!({ "kind": "ceiling_mounted", "x": user(ON), "y": user(ON) }),
                        json!({ "front": user(0), "back": user(0), "left": user(0), "right": user(v) }),
                    )
                }),
            ),
            (
                "/args/kind/value",
                Box::new(|v| action("add_constraint", constraint("user_stated", v))),
            ),
            (
                "/args/kind/value",
                Box::new(|v| {
                    action(
                        "add_constraint",
                        json!({ "kind": { "kind": "fixed", "reference": reference(), "axis": "y", "value": v }, "source": "user_stated" }),
                    )
                }),
            ),
            (
                "/args/kind/value",
                Box::new(|v| {
                    action(
                        "add_constraint",
                        json!({ "kind": { "kind": "min_clearance", "object": ID, "other": ID, "value": v }, "source": "user_stated" }),
                    )
                }),
            ),
            (
                "/args/value",
                Box::new(|v| {
                    action(
                        "set_constraint_value",
                        json!({ "target": { "kind": "constraint", "constraint": ID }, "value": v }),
                    )
                }),
            ),
            (
                "/args/value",
                Box::new(|v| {
                    action(
                        "set_constraint_value",
                        json!({ "target": { "kind": "node_coordinate", "node": ID, "axis": "x" }, "value": v }),
                    )
                }),
            ),
            (
                "/args/tile_size",
                Box::new(|v| {
                    action(
                        "create_material",
                        json!({ "name": "Tile", "tile_size": user(v) }),
                    )
                }),
            ),
            (
                "/args/thickness",
                Box::new(slab("create_slab", "thickness")),
            ),
            (
                "/args/elevation",
                Box::new(slab("create_slab", "elevation")),
            ),
            (
                "/args/thickness",
                Box::new(slab("create_ceiling", "thickness")),
            ),
            (
                "/args/elevation",
                Box::new(slab("create_ceiling", "elevation")),
            ),
            (
                "/args/high_edge_height",
                Box::new(roof_field("high_edge_height")),
            ),
            (
                "/args/low_edge_height",
                Box::new(roof_field("low_edge_height")),
            ),
            ("/args/thickness", Box::new(roof_field("thickness"))),
            ("/args/fascia_depth", Box::new(roof_field("fascia_depth"))),
            ("/args/overhang/neg_y", Box::new(overhang)),
            (
                "/args/declared_dimensions/width",
                Box::new(|v| action("create_element_type", element_type("user_stated_exact", v))),
            ),
        ]
    }

    #[test]
    fn off_grid_user_stated_values_are_rejected_with_the_nearest_grid_value() {
        for (at, make) in cases() {
            let a = make(ON + 1);
            assert_eq!(
                check_user_grid(&a),
                Err(CommandError::OffGrid {
                    argument: at.to_string(),
                    value: Length::from_nanometers(ON + 1),
                    nearest: Length::from_nanometers(ON),
                }),
                "{} {at}",
                a.kind()
            );
            assert_eq!(check_user_grid(&make(ON)), Ok(()), "{} {at}", a.kind());
            // Negative values too (a move to the west, a signed distance).
            assert_eq!(check_user_grid(&make(-ON)), Ok(()), "{} {at}", a.kind());
            assert!(
                check_user_grid(&make(-ON - 1)).is_err(),
                "{} {at}",
                a.kind()
            );
        }
    }

    #[test]
    fn the_message_names_the_argument_and_echoes_the_nearest_grid_value() {
        let e = check_user_grid(&action("add_constraint", constraint("user_stated", ON + 1)));
        assert_eq!(
            e.map_err(|e| e.to_string()),
            Err("/args/kind/value is 6096000001 nm, finer than 1/16 in; nearest grid value is 20'-0\"".to_string())
        );
    }

    #[test]
    fn values_that_are_not_user_stated_are_exempt() {
        let off = ON + 1;
        let sourced = |source: &str| json!({ "source": source, "value": off });
        for source in ["measured", "derived", "imported", "proposed"] {
            assert_eq!(
                check_user_grid(&action("add_constraint", constraint(source, off))),
                Ok(()),
                "{source}"
            );
            assert_eq!(
                check_user_grid(&action("create_wall", wall("base_offset", sourced(source)))),
                Ok(()),
                "{source}"
            );
        }
        for authority in [
            "manufacturer_spec",
            "cad_derived",
            "measured_with_tolerance",
            "proposed_default",
            "mesh_derived",
        ] {
            assert_eq!(
                check_user_grid(&action("create_element_type", element_type(authority, off))),
                Ok(()),
                "{authority}"
            );
        }
        // Node positions are provisional; datums carry no source.
        assert_eq!(
            check_user_grid(&action("create_node", json!({ "x": off, "y": off }))),
            Ok(())
        );
        assert_eq!(
            check_user_grid(&action(
                "create_datum",
                json!({ "name": "A", "geometry": { "kind": "x_line", "x": off } })
            )),
            Ok(())
        );
    }

    #[test]
    fn the_first_off_grid_field_is_reported() {
        let a = action("create_wall", wall("base_offset", user(ON + 2)));
        let a = match a {
            Action::CreateWall(mut w) => {
                w.left_extent = Some(Sourced {
                    value: Length::from_nanometers(ON + 1),
                    source: Source::UserStated,
                });
                Action::CreateWall(w)
            }
            other => other,
        };
        assert!(matches!(
            check_user_grid(&a),
            Err(CommandError::OffGrid { argument, .. }) if argument == "/args/left_extent"
        ));
    }

    #[test]
    fn nearest_grid_rounds_half_to_even_and_stays_in_range() {
        let g = GRID_NM;
        for (v, nearest) in [
            (1, 0),
            (g / 2, 0),
            (g / 2 + 1, g),
            (g + g / 2, 2 * g),
            (-1, 0),
            (-(g + g / 2), -2 * g),
            (i64::MAX, (i64::MAX / g) * g),
            (i64::MIN, (i64::MIN / g) * g),
        ] {
            assert_eq!(
                nearest_grid(Length::from_nanometers(v)),
                Length::from_nanometers(nearest),
                "{v}"
            );
        }
    }
}
