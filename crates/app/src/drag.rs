//! Turning a pointer drag into a move command (T-202, spec 3.1, 8.7).
//!
//! A drag grabs one reference of the entity under the pointer: the wall face
//! (or center) nearest the pointer, an opening's start edge, an item's
//! center. While the pointer moves, the reference's new coordinate is its
//! start plus the pointer's world displacement, snapped (snap.rs); the
//! command is a move to that snapped target. The drag never computes the
//! consequences: core resolves the move when the command is proposed.

use archviz_core::{
    Action, Axis, Motion, MoveOccurrence, MoveOpening, MoveWall, Reference, WallSide,
};

use crate::scene::{Entity, Scene};
use crate::snap::{Snap, snap};
use crate::viewport::World;

/// The reference a drag holds and where it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drag {
    pub entity: Entity,
    grip: Grip,
    pub axis: Option<Axis>,
    /// The pointer's world position when the drag started.
    start: World,
    /// The reference's coordinate on each axis when the drag started.
    from: World,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grip {
    Wall(Reference),
    OpeningStart,
    Center,
}

/// A drag in progress: where its reference would go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub axis: Axis,
    pub snap: Snap,
    /// The snapped displacement of the reference.
    pub delta: i64,
    /// The move, or None while the reference has not moved.
    pub action: Option<Action>,
}

fn on(axis: Axis, p: World) -> i64 {
    match axis {
        Axis::X => p.0,
        Axis::Y => p.1,
    }
}

fn distance(a: i64, b: i64) -> i128 {
    (i128::from(a) - i128::from(b)).abs()
}

impl Drag {
    /// Starts dragging `entity` from the pointer at `at`. Slabs do not drag.
    pub fn begin(scene: &Scene, entity: Entity, at: World) -> Option<Drag> {
        match entity {
            Entity::Wall(id) => {
                let w = scene.walls.iter().find(|w| w.id == id)?;
                let p = on(w.across, at);
                let sum = i128::from(w.left) + i128::from(w.right);
                let mut grips = vec![
                    (w.left, WallSide::Left.into_reference(id)),
                    (w.right, WallSide::Right.into_reference(id)),
                ];
                if sum % 2 == 0
                    && let Ok(c) = i64::try_from(sum / 2)
                {
                    grips.push((c, Reference::WallCenter { wall: id }));
                }
                let (value, reference) = grips.into_iter().min_by_key(|(v, _)| distance(*v, p))?;
                let from = match w.across {
                    Axis::X => (value, 0),
                    Axis::Y => (0, value),
                };
                Some(Drag {
                    entity,
                    grip: Grip::Wall(reference),
                    axis: Some(w.across),
                    start: at,
                    from,
                })
            }
            Entity::Opening(id) => {
                let o = scene.openings.iter().find(|o| o.id == id)?;
                let from = match o.along {
                    Axis::X => (o.start, 0),
                    Axis::Y => (0, o.start),
                };
                Some(Drag {
                    entity,
                    grip: Grip::OpeningStart,
                    axis: Some(o.along),
                    start: at,
                    from,
                })
            }
            Entity::Occurrence(id) => {
                let o = scene.occurrences.iter().find(|o| o.id == id)?;
                Some(Drag {
                    entity,
                    grip: Grip::Center,
                    axis: o.along,
                    start: at,
                    from: o.center,
                })
            }
            Entity::Slab(_) => None,
        }
    }

    /// Where the reference goes with the pointer at `at`, snapping to a
    /// reference within `tolerance` nanometers, else to a multiple of the
    /// visible grid `step` (Q-79). A free item moves along the axis the
    /// pointer has moved farther on.
    pub fn update(&self, scene: &Scene, at: World, tolerance: i64, step: i64) -> Option<Preview> {
        let dx = i128::from(at.0) - i128::from(self.start.0);
        let dy = i128::from(at.1) - i128::from(self.start.1);
        let axis = self.axis.unwrap_or(if dy.abs() > dx.abs() {
            Axis::Y
        } else {
            Axis::X
        });
        let d = match axis {
            Axis::X => dx,
            Axis::Y => dy,
        };
        let from = on(axis, self.from);
        let raw = i64::try_from(i128::from(from) + d).ok()?;
        let snap = snap(
            raw,
            &scene.references(axis, Some(self.entity)),
            tolerance,
            step,
        );
        let delta = i64::try_from(i128::from(snap.value) - i128::from(from)).ok()?;
        let target = Motion::Target {
            target: archviz_core::Length::from_nanometers(snap.value),
        };
        let action = match (self.entity, self.grip) {
            _ if delta == 0 => None,
            (Entity::Wall(wall), Grip::Wall(reference)) => Some(Action::MoveWall(MoveWall {
                wall,
                reference,
                motion: target,
                edit: None,
                absorb: vec![],
            })),
            (Entity::Opening(opening), _) => Some(Action::MoveOpening(MoveOpening {
                opening,
                motion: target,
                edit: None,
                absorb: vec![],
            })),
            (Entity::Occurrence(occurrence), _) => Some(Action::MoveOccurrence(MoveOccurrence {
                occurrence,
                axis,
                motion: target,
                edit: None,
                absorb: vec![],
            })),
            // `begin` never makes these.
            (Entity::Wall(_) | Entity::Slab(_), _) => None,
        };
        Some(Preview {
            axis,
            snap,
            delta,
            action,
        })
    }
}

trait IntoReference {
    fn into_reference(self, wall: archviz_core::Uuid) -> Reference;
}

impl IntoReference for WallSide {
    fn into_reference(self, wall: archviz_core::Uuid) -> Reference {
        Reference::WallFace {
            wall,
            side: self,
            boundary: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::tests::shop;
    use crate::snap::{SnapKind, visible_step};
    use crate::viewport::{MAX_SCALE, Screen, Viewport};
    use archviz_core::{GRID_NM, reference_coordinate};
    use proptest::prelude::*;

    fn motion_target(action: &Action) -> i64 {
        let m = match action {
            Action::MoveWall(m) => m.motion,
            Action::MoveOpening(m) => m.motion,
            Action::MoveOccurrence(m) => m.motion,
            other => panic!("not a move: {other:?}"),
        };
        match m {
            Motion::Target { target } => target.nanometers(),
            Motion::Delta { .. } => panic!("a drag sends a target"),
        }
    }

    fn scene() -> (Scene, archviz_core::Document) {
        let (_, doc) = shop("drag");
        (Scene::build(&doc).unwrap_or_else(|e| panic!("{e}")), doc)
    }

    #[test]
    fn a_wall_drag_grabs_the_nearest_face_and_targets_the_grid() {
        let (scene, doc) = scene();
        let w = &scene.walls[0];
        let at = match w.across {
            Axis::Y => (0, w.left),
            Axis::X => (w.left, 0),
        };
        let drag = Drag::begin(&scene, Entity::Wall(w.id), at).unwrap_or_else(|| panic!());
        let raw = 12 * 25_400_000 + 1_234_567; // 1 ft and a fraction of a sixteenth
        let to = match w.across {
            Axis::Y => (at.0, at.1 + raw),
            Axis::X => (at.0 + raw, at.1),
        };
        let p = drag
            .update(&scene, to, 0, GRID_NM)
            .unwrap_or_else(|| panic!());
        assert_eq!(p.snap.kind, SnapKind::Grid);
        let action = p.action.unwrap_or_else(|| panic!("moved"));
        let Action::MoveWall(m) = &action else {
            panic!("{action:?}")
        };
        assert_eq!(m.reference, WallSide::Left.into_reference(w.id));
        let target = motion_target(&action);
        assert_eq!(target % GRID_NM, 0);
        let face = reference_coordinate(&doc, &m.reference, w.across)
            .unwrap_or_else(|e| panic!("{e:?}"))
            .nanometers();
        assert_eq!(target, crate::snap::to_grid(face + raw));
    }

    #[test]
    fn no_move_no_command() {
        let (scene, _) = scene();
        let o = &scene.occurrences[0];
        let drag =
            Drag::begin(&scene, Entity::Occurrence(o.id), o.center).unwrap_or_else(|| panic!());
        let p = drag.update(&scene, (o.center.0 + 10, o.center.1), 0, GRID_NM);
        if o.center.0 % GRID_NM == 0 {
            assert_eq!(p.map(|p| p.action), Some(None));
        }
        assert_eq!(
            Drag::begin(&scene, Entity::Slab(scene.slabs[0].id), (0, 0)),
            None
        );
    }

    #[test]
    fn a_free_item_moves_on_the_dominant_axis_and_snaps_to_a_reference() {
        let (scene, _) = scene();
        let Some(o) = scene.occurrences.iter().find(|o| o.along.is_none()) else {
            panic!("the shop has floor items")
        };
        let drag =
            Drag::begin(&scene, Entity::Occurrence(o.id), o.center).unwrap_or_else(|| panic!());
        // Toward the nearest on-grid X reference other than its own.
        let refs = scene.references(Axis::X, Some(Entity::Occurrence(o.id)));
        let (goal, name) = refs
            .iter()
            .filter(|(v, _)| v % GRID_NM == 0 && *v != o.center.0)
            .min_by_key(|(v, _)| (i128::from(*v) - i128::from(o.center.0)).abs())
            .cloned()
            .unwrap_or_else(|| panic!("references"));
        let to = (goal + 1000, o.center.1 + 10);
        let p = drag
            .update(&scene, to, 5000, GRID_NM)
            .unwrap_or_else(|| panic!());
        assert_eq!(p.axis, Axis::X);
        assert_eq!(p.snap.kind, SnapKind::Reference(name));
        assert_eq!(motion_target(&p.action.unwrap_or_else(|| panic!())), goal);
    }

    proptest! {
        /// Done when (part 2): a drag at any zoom, from and to any screen
        /// position, yields a move whose value is a snapped integer.
        #[test]
        fn every_drag_command_value_is_on_the_grid(
            which in 0_usize..1000,
            zoom in 0.0_f64..1.0,
            from in (0.0_f64..1280.0, 0.0_f64..800.0),
            to in (0.0_f64..1280.0, 0.0_f64..800.0),
        ) {
            let (scene, _) = scene_once();
            let entities: Vec<Entity> = scene
                .walls.iter().map(|w| Entity::Wall(w.id))
                .chain(scene.openings.iter().map(|o| Entity::Opening(o.id)))
                .chain(scene.occurrences.iter().map(|o| Entity::Occurrence(o.id)))
                .collect();
            let entity = entities[which % entities.len()];
            let ((x0, y0), (x1, y1)) = scene.bounds().unwrap_or(((0, 0), (0, 0)));
            let mut v = Viewport::fit((x0, y0), (x1, y1), (1280.0, 800.0), 40.0, Screen { x: 640.0, y: 400.0 });
            v.zoom_about(Screen { x: from.0, y: from.1 }, (MAX_SCALE / v.scale()).powf(zoom));
            let start = v.to_world(Screen { x: from.0, y: from.1 });
            let end = v.to_world(Screen { x: to.0, y: to.1 });
            let drag = Drag::begin(&scene, entity, start);
            prop_assert!(drag.is_some());
            let step = visible_step(v.scale());
            let p = drag.and_then(|d| d.update(&scene, end, v.nm(6.0), step));
            prop_assert!(p.is_some());
            if let Some(p) = p {
                // On the visible step, or on an on-grid reference (Q-79).
                if p.snap.kind == SnapKind::Grid {
                    prop_assert_eq!(p.snap.value % step, 0);
                }
                if let Some(action) = p.action {
                    prop_assert_eq!(motion_target(&action) % GRID_NM, 0);
                    prop_assert_eq!(motion_target(&action), p.snap.value);
                }
            }
        }
    }

    fn scene_once() -> (Scene, archviz_core::Document) {
        use std::sync::OnceLock;
        static SCENE: OnceLock<(Scene, archviz_core::Document)> = OnceLock::new();
        SCENE.get_or_init(scene).clone()
    }
}
