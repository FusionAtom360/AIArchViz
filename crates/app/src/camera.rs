//! The 3D viewer's camera (T-206, D-14): orbit and walk navigation, the
//! perspective projection, rays for picking and the view context (9.1, 9.3).
//!
//! The eye is held as an f64 offset in nanometers from an exact i64 anchor
//! (the model's center), so geometry far from the world origin projects as
//! steadily as geometry at it; points reach the screen as their exact i128
//! difference from the anchor before any float arithmetic. The camera never
//! produces a coordinate that is committed: picks resolve to exact vertices
//! (model3d.rs), and the view context rounds the pose to integers.

use archviz_core::{CameraPitch, CameraPose, Length, Millidegrees, Uuid, ViewContext};

/// An exact world point in nanometers.
pub type P3 = [i128; 3];
/// A vector in nanometers.
pub type V3 = [f64; 3];

/// Vertical field of view: 50 degrees.
pub const FOV_Y: f64 = 50.0 * std::f64::consts::PI / 180.0;
/// The nearest orbit distance: 1/16 in, so the finest grid step fills the view.
pub const MIN_DISTANCE: f64 = 1_587_500.0;
/// The farthest orbit distance: 10 miles.
pub const MAX_DISTANCE: f64 = 1.609_344e13;
/// Pitch stays a degree short of straight up and down, so "right" exists.
const PITCH_LIMIT: f64 = 89.0 * std::f64::consts::PI / 180.0;
/// Radians of turn per pixel dragged.
const TURN_PER_PX: f64 = 0.005;
/// The near clip plane in walk mode: 1 cm.
const WALK_NEAR: f64 = 1e7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The eye circles a target point; drag turns about it.
    Orbit,
    /// The eye stays put; drag looks around and keys walk.
    Walk,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    /// An exact world point the eye is measured from.
    anchor: [i64; 3],
    /// The eye, nanometers from the anchor.
    eye: V3,
    /// Heading in radians: 0 looks toward +X, counterclockwise (Q-5).
    yaw: f64,
    /// Radians above the horizon.
    pitch: f64,
    /// Orbit: from the eye to the point it circles.
    distance: f64,
    pub mode: Mode,
}

pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn add(a: V3, b: V3, s: f64) -> V3 {
    [a[0] + b[0] * s, a[1] + b[1] * s, a[2] + b[2] * s]
}

fn round_i64(v: f64) -> i64 {
    v.round().clamp(i64::MIN as f64, i64::MAX as f64) as i64
}

/// The center of `min` and `max`, if it fits an i64 on every axis.
fn center(min: P3, max: P3) -> Option<[i64; 3]> {
    let mut c = [0i64; 3];
    for k in 0..3 {
        c[k] = i64::try_from(min[k].checked_add(max[k])?.div_euclid(2)).ok()?;
    }
    Some(c)
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            anchor: [0; 3],
            eye: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            distance: 1e10,
            mode: Mode::Orbit,
        }
    }
}

impl Camera {
    /// Looks at the box from the southwest, a little above, all of it in view.
    pub fn fit(&mut self, min: P3, max: P3) {
        let Some(anchor) = center(min, max) else {
            return;
        };
        self.anchor = anchor;
        let half: Vec<f64> = (0..3).map(|k| (max[k] - min[k]) as f64 / 2.0).collect();
        let radius = half.iter().map(|h| h * h).sum::<f64>().sqrt();
        self.yaw = 45f64.to_radians();
        self.pitch = -30f64.to_radians();
        self.distance = (radius / (FOV_Y / 2.0).sin() * 1.1).clamp(MIN_DISTANCE, MAX_DISTANCE);
        self.eye = add([0.0; 3], self.forward(), -self.distance);
        self.mode = Mode::Orbit;
    }

    /// Orbits about the exact point `target` from `distance` away, keeping
    /// the heading and pitch: a double click on a vertex does this.
    pub fn look_at(&mut self, target: P3, distance: f64) {
        let Some(anchor) = center(target, target) else {
            return;
        };
        self.anchor = anchor;
        self.distance = distance.clamp(MIN_DISTANCE, MAX_DISTANCE);
        self.eye = add([0.0; 3], self.forward(), -self.distance);
        self.mode = Mode::Orbit;
    }

    /// Where the camera looks.
    pub fn forward(&self) -> V3 {
        let (c, s) = (self.pitch.cos(), self.pitch.sin());
        [c * self.yaw.cos(), c * self.yaw.sin(), s]
    }

    /// The screen's right, always horizontal: the heading less 90 degrees.
    pub fn right(&self) -> V3 {
        [self.yaw.sin(), -self.yaw.cos(), 0.0]
    }

    /// The screen's up.
    pub fn up(&self) -> V3 {
        cross(self.right(), self.forward())
    }

    fn target(&self) -> V3 {
        add(self.eye, self.forward(), self.distance)
    }

    /// The near clip plane, nanometers in front of the eye.
    pub fn near(&self) -> f64 {
        match self.mode {
            Mode::Walk => WALK_NEAR,
            Mode::Orbit => (self.distance / 100.0).min(WALK_NEAR),
        }
    }

    fn turn(&mut self, dx: f64, dy: f64) {
        self.yaw = (self.yaw - dx * TURN_PER_PX).rem_euclid(std::f64::consts::TAU);
        self.pitch = (self.pitch - dy * TURN_PER_PX).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// A drag of (dx, dy) pixels: orbit turns about the target, walk looks
    /// around from where the eye stands.
    pub fn drag(&mut self, dx: f64, dy: f64) {
        match self.mode {
            Mode::Orbit => {
                let target = self.target();
                self.turn(dx, dy);
                self.eye = add(target, self.forward(), -self.distance);
            }
            Mode::Walk => self.turn(dx, dy),
        }
    }

    /// Nanometers per pixel at the target, in a view `height` px tall.
    fn scale(&self, height: f64) -> f64 {
        2.0 * self.distance * (FOV_Y / 2.0).tan() / height.max(1.0)
    }

    /// Slides eye and target so the scene follows a drag of (dx, dy) px.
    pub fn pan(&mut self, dx: f64, dy: f64, height: f64) {
        let s = self.scale(height);
        self.eye = add(add(self.eye, self.right(), -dx * s), self.up(), dy * s);
    }

    /// Orbit: moves toward the target by `factor` of the distance (below 1
    /// comes closer). Walk: steps forward a tenth of the distance per halving.
    pub fn zoom(&mut self, factor: f64) {
        match self.mode {
            Mode::Orbit => {
                let target = self.target();
                self.distance = (self.distance * factor).clamp(MIN_DISTANCE, MAX_DISTANCE);
                self.eye = add(target, self.forward(), -self.distance);
            }
            Mode::Walk => {
                let step = -factor.ln() / std::f64::consts::LN_2 * self.distance / 10.0;
                self.walk(step, 0.0, 0.0);
            }
        }
    }

    /// Moves the eye `ahead` along the heading, `aside` to the right and
    /// `rise` up, in nanometers; the view direction is unchanged.
    pub fn walk(&mut self, ahead: f64, aside: f64, rise: f64) {
        let level = [self.yaw.cos(), self.yaw.sin(), 0.0];
        self.eye = add(
            add(add(self.eye, level, ahead), self.right(), aside),
            [0.0, 0.0, 1.0],
            rise,
        );
    }

    /// `p` relative to the eye, nanometers, from its exact offset from the anchor.
    pub fn relative(&self, p: P3) -> V3 {
        let mut v = [0.0; 3];
        for k in 0..3 {
            v[k] = (p[k] - i128::from(self.anchor[k])) as f64 - self.eye[k];
        }
        v
    }

    /// `p` in camera space: (right, up, ahead) in nanometers.
    pub fn to_view(&self, p: P3) -> V3 {
        let d = self.relative(p);
        [
            dot(d, self.right()),
            dot(d, self.up()),
            dot(d, self.forward()),
        ]
    }

    /// The pixel `p` lands on in a view of `size` px (origin top left), or
    /// None when it is behind the near plane.
    pub fn project(&self, p: P3, size: [f64; 2]) -> Option<[f64; 2]> {
        let v = self.to_view(p);
        if v[2] < self.near() {
            return None;
        }
        let t = (FOV_Y / 2.0).tan();
        let aspect = size[0] / size[1].max(1.0);
        let nx = v[0] / (v[2] * t * aspect);
        let ny = v[1] / (v[2] * t);
        Some([(nx + 1.0) / 2.0 * size[0], (1.0 - ny) / 2.0 * size[1]])
    }

    /// The direction (unit length) from the eye through pixel `at`.
    pub fn ray(&self, at: [f64; 2], size: [f64; 2]) -> V3 {
        let t = (FOV_Y / 2.0).tan();
        let aspect = size[0] / size[1].max(1.0);
        let nx = at[0] / size[0].max(1.0) * 2.0 - 1.0;
        let ny = 1.0 - at[1] / size[1].max(1.0) * 2.0;
        let d = add(
            add(self.forward(), self.right(), nx * t * aspect),
            self.up(),
            ny * t,
        );
        let n = dot(d, d).sqrt();
        [d[0] / n, d[1] / n, d[2] / n]
    }

    /// Rows of the camera-relative projection for the GPU, in meters, with
    /// reversed depth (near maps to 1, infinitely far to 0): clip = row .
    /// (p - eye, 1).
    pub fn rows(&self, size: [f64; 2]) -> [[f32; 4]; 4] {
        let t = (FOV_Y / 2.0).tan();
        let aspect = size[0] / size[1].max(1.0);
        let (r, u, f) = (self.right(), self.up(), self.forward());
        let row = |v: V3, s: f64, w: f64| {
            [
                (v[0] * s) as f32,
                (v[1] * s) as f32,
                (v[2] * s) as f32,
                w as f32,
            ]
        };
        [
            row(r, 1.0 / (t * aspect), 0.0),
            row(u, 1.0 / t, 0.0),
            row([0.0; 3], 0.0, self.near() / 1e9),
            row(f, 1.0, 0.0),
        ]
    }

    /// The pose as the view context gives it: the eye to the nanometer, the
    /// heading and pitch to the millidegree.
    pub fn pose(&self) -> CameraPose {
        let exact = |k: usize| {
            let v = i128::from(self.anchor[k]) + i128::from(round_i64(self.eye[k]));
            Length::from_nanometers(v.clamp(i64::MIN.into(), i64::MAX.into()) as i64)
        };
        let heading = round_i64(self.yaw.to_degrees() * 1000.0).rem_euclid(360_000);
        let pitch = round_i64(self.pitch.to_degrees() * 1000.0).clamp(-90_000, 90_000);
        CameraPose {
            x: exact(0),
            y: exact(1),
            z: exact(2),
            heading: Millidegrees::new(u32::try_from(heading).unwrap_or(0))
                .unwrap_or(Millidegrees::of_direction(archviz_core::Direction::PosX)),
            pitch: i32::try_from(pitch)
                .ok()
                .and_then(CameraPitch::new)
                .unwrap_or(CameraPitch::LEVEL),
        }
    }

    /// The view context (9.1): this camera's pose, the selection sorted and
    /// the hovered entity.
    pub fn context(&self, selection: &[Uuid], hovered: Option<Uuid>) -> ViewContext {
        let mut selection = selection.to_vec();
        selection.sort();
        selection.dedup();
        ViewContext {
            camera: self.pose(),
            selection,
            hovered,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FT: i128 = 304_800_000;
    const MILE: i128 = 5280 * FT;
    const SIZE: [f64; 2] = [1280.0, 720.0];

    fn building() -> (P3, P3) {
        ([MILE, MILE, 0], [MILE + 60 * FT, MILE + 40 * FT, 12 * FT])
    }

    fn fitted() -> Camera {
        let mut c = Camera::default();
        let (a, b) = building();
        c.fit(a, b);
        c
    }

    /// The world point the pose says the eye is at, plus `metres` along
    /// the direction its heading and pitch give.
    fn along_pose(pose: &CameraPose, nm: f64) -> P3 {
        let h = f64::from(pose.heading.value()).to_radians() / 1000.0;
        let p = f64::from(pose.pitch.value()).to_radians() / 1000.0;
        let d = [p.cos() * h.cos(), p.cos() * h.sin(), p.sin()];
        let e = [pose.x, pose.y, pose.z].map(|l| i128::from(l.nanometers()));
        [0, 1, 2].map(|k| e[k] + (d[k] * nm).round() as i128)
    }

    fn assert_context_matches(c: &Camera) {
        let pose = c.pose();
        // The eye: the anchor plus the offset, to the nanometer.
        for (k, l) in [pose.x, pose.y, pose.z].iter().enumerate() {
            let want = i128::from(c.anchor[k]) + c.eye[k].round() as i128;
            assert_eq!(i128::from(l.nanometers()), want);
        }
        // A point 30 ft ahead along the pose's heading and pitch lies on
        // the screen center, within a millidegree of rounding (0.0055 mm
        // at 30 ft, well under a pixel).
        let ahead = along_pose(&pose, 30.0 * FT as f64);
        let px = c.project(ahead, SIZE).unwrap_or_else(|| panic!("behind"));
        assert!((px[0] - SIZE[0] / 2.0).abs() < 0.05, "{px:?}");
        assert!((px[1] - SIZE[1] / 2.0).abs() < 0.05, "{px:?}");
        // The right vector is the heading less a quarter turn (9.3).
        let h = f64::from(pose.heading.value()).to_radians() / 1000.0;
        let r = c.right();
        assert!((r[0] - h.sin()).abs() < 1e-4 && (r[1] + h.cos()).abs() < 1e-4);
        assert_eq!(r[2], 0.0);
    }

    #[test]
    fn view_context_matches_the_camera() {
        let mut c = fitted();
        assert_context_matches(&c);
        c.drag(123.4, -40.25);
        assert_context_matches(&c);
        c.pan(17.5, 3.25, SIZE[1]);
        c.zoom(0.37);
        assert_context_matches(&c);
        c.mode = Mode::Walk;
        c.walk(3.0 * FT as f64, -1.5 * FT as f64, 0.25 * FT as f64);
        c.drag(-700.0, 400.0);
        assert_context_matches(&c);
        let (a, b) = (Uuid::from_bytes([2; 16]), Uuid::from_bytes([1; 16]));
        let ctx = c.context(&[a, b, a], Some(b));
        assert_eq!(ctx.selection, vec![b, a]);
        assert_eq!(ctx.hovered, Some(b));
        assert_eq!(ctx.camera, c.pose());
    }

    #[test]
    fn known_headings_read_exactly() {
        let mut c = fitted();
        assert_eq!(c.pose().heading.value(), 45_000);
        assert_eq!(c.pose().pitch.value(), -30_000);
        // Turn left a quarter turn: the heading grows (counterclockwise).
        c.drag(-std::f64::consts::FRAC_PI_2 / TURN_PER_PX, 0.0);
        assert_eq!(c.pose().heading.value(), 135_000);
        c.drag(-std::f64::consts::PI / TURN_PER_PX, 0.0);
        assert_eq!(c.pose().heading.value(), 315_000);
        c.drag(0.0, 1e6);
        assert_eq!(c.pose().pitch.value(), -89_000);
    }

    #[test]
    fn fit_shows_the_whole_box_a_mile_from_the_origin() {
        let c = fitted();
        let (a, b) = building();
        for i in 0..8 {
            let p = [0, 1, 2].map(|k| if i >> k & 1 == 0 { a[k] } else { b[k] });
            let px = c.project(p, SIZE).unwrap_or_else(|| panic!("behind"));
            assert!((0.0..=SIZE[0]).contains(&px[0]) && (0.0..=SIZE[1]).contains(&px[1]));
        }
    }

    #[test]
    fn a_ray_through_a_projected_point_hits_it() {
        let mut c = fitted();
        c.drag(31.0, 12.0);
        let (a, _) = building();
        let px = c.project(a, SIZE).unwrap_or_else(|| panic!("behind"));
        let d = c.ray(px, SIZE);
        let v = c.relative(a);
        let along = dot(v, d);
        let miss = add(v, d, -along);
        // Within a micrometer at about 100 ft.
        assert!(dot(miss, miss).sqrt() < 1_000.0, "{miss:?}");
    }

    #[test]
    fn orbit_keeps_the_target_and_pan_moves_it_with_the_pointer() {
        let mut c = fitted();
        let target = c.target();
        c.drag(250.0, 80.0);
        let t2 = c.target();
        assert!((0..3).all(|k| (t2[k] - target[k]).abs() < 1.0));
        // A pan of 10 px moves a point at the target 10 px on screen.
        let tp = |c: &Camera| {
            let t = c.target();
            [0, 1, 2].map(|k| i128::from(c.anchor[k]) + t[k].round() as i128)
        };
        let p = tp(&c);
        let before = c.project(p, SIZE).unwrap_or_else(|| panic!());
        c.pan(10.0, -4.0, SIZE[1]);
        let after = c.project(p, SIZE).unwrap_or_else(|| panic!());
        assert!((after[0] - before[0] - 10.0).abs() < 1e-3);
        assert!((after[1] - before[1] + 4.0).abs() < 1e-3);
    }

    #[test]
    fn look_at_centers_the_point_a_mile_from_the_origin() {
        let mut c = fitted();
        c.drag(77.0, -21.0);
        let p = [MILE + 7, MILE - 3, 5 * FT + 1];
        c.look_at(p, 8.0 * FT as f64);
        let px = c.project(p, SIZE).unwrap_or_else(|| panic!("behind"));
        assert!((px[0] - SIZE[0] / 2.0).abs() < 1e-6 && (px[1] - SIZE[1] / 2.0).abs() < 1e-6);
        // A micrometer off the point still moves it on screen at 8 ft
        // (about 3e-4 px): nothing is lost to the mile from the origin.
        let q = [p[0] + 1_000, p[1], p[2]];
        let qx = c.project(q, SIZE).unwrap_or_else(|| panic!("behind"));
        assert!((qx[0] - px[0]).hypot(qx[1] - px[1]) > 1e-4);
    }

    #[test]
    fn zoom_stops_at_one_sixteenth_inch() {
        let mut c = fitted();
        for _ in 0..1000 {
            c.zoom(0.5);
        }
        assert_eq!(c.distance, MIN_DISTANCE);
        assert!(c.near() < MIN_DISTANCE);
    }
}
