//! The plan view's world-to-screen transform (T-202, spec 3.1, 5.4).
//!
//! The view center is an exact world point in nanometers (i64) and the scale
//! is pixels per nanometer (f64). A world point reaches the screen as the
//! exact i128 difference from the center, converted to f64 and scaled, so
//! its screen position carries no error from the size of the world
//! coordinates: a building a mile from the origin draws as steadily at 1/16
//! in as one at the origin. The reverse rounds to the nanometer; a pointer
//! position becomes a command argument only after the grid snap (snap.rs).
//!
//! Plan view is X right and Y up (5.4); screen Y grows down.

/// Pixels across one 1/16 in grid step at the closest zoom.
pub const MAX_PX_PER_GRID: f64 = 400.0;
/// Nanometers across one pixel at the farthest zoom (100 ft).
pub const MAX_NM_PER_PX: f64 = 30_480_000_000.0;

const GRID: f64 = 1_587_500.0;

/// Pixels per nanometer at the closest zoom.
pub const MAX_SCALE: f64 = MAX_PX_PER_GRID / GRID;
/// Pixels per nanometer at the farthest zoom.
pub const MIN_SCALE: f64 = 1.0 / MAX_NM_PER_PX;

/// A screen position in pixels, f64 until it is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    pub x: f64,
    pub y: f64,
}

/// A world position in nanometers.
pub type World = (i64, i64);

#[derive(Debug, Clone, PartialEq)]
pub struct Viewport {
    /// The world point drawn at `origin` is `center` plus `sub`, a
    /// sub-nanometer residual within half a nanometer, so pans and zooms
    /// accumulate no rounding.
    center: World,
    sub: (f64, f64),
    /// Pixels per nanometer.
    scale: f64,
    /// The screen point the center is drawn at (the canvas center).
    origin: Screen,
}

fn clamp_i64(v: i128) -> i64 {
    i64::try_from(v).unwrap_or(if v < 0 { i64::MIN } else { i64::MAX })
}

/// `v` rounded to the nearest integer and clamped to i64.
fn round_i64(v: f64) -> i64 {
    let r = v.round();
    if r.is_nan() {
        0
    } else if r >= 9.223_372_036_854_776e18 {
        i64::MAX
    } else if r <= -9.223_372_036_854_776e18 {
        i64::MIN
    } else {
        r as i64
    }
}

impl Viewport {
    pub fn new(center: World, scale: f64, origin: Screen) -> Viewport {
        Viewport {
            center,
            sub: (0.0, 0.0),
            scale: scale.clamp(MIN_SCALE, MAX_SCALE),
            origin,
        }
    }

    /// The view that shows `min`..`max` inside a canvas of `size` pixels
    /// with `margin` pixels on each side, centered at `origin`.
    pub fn fit(min: World, max: World, size: (f64, f64), margin: f64, origin: Screen) -> Viewport {
        let center = (
            clamp_i64((i128::from(min.0) + i128::from(max.0)) / 2),
            clamp_i64((i128::from(min.1) + i128::from(max.1)) / 2),
        );
        let w = (i128::from(max.0) - i128::from(min.0)).max(1) as f64;
        let h = (i128::from(max.1) - i128::from(min.1)).max(1) as f64;
        let avail = (
            (size.0 - 2.0 * margin).max(1.0),
            (size.1 - 2.0 * margin).max(1.0),
        );
        let scale = (avail.0 / w).min(avail.1 / h);
        Viewport::new(center, scale, origin)
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Moves the canvas center (the window was resized) without moving the
    /// world point under it.
    pub fn set_origin(&mut self, origin: Screen) {
        self.origin = origin;
    }

    pub fn to_screen(&self, p: World) -> Screen {
        let dx = (i128::from(p.0) - i128::from(self.center.0)) as f64 - self.sub.0;
        let dy = (i128::from(p.1) - i128::from(self.center.1)) as f64 - self.sub.1;
        Screen {
            x: self.origin.x + dx * self.scale,
            y: self.origin.y - dy * self.scale,
        }
    }

    /// The offset from the exact center, in nanometers, of a screen point.
    fn offset(&self, s: Screen) -> (f64, f64) {
        (
            (s.x - self.origin.x) / self.scale,
            (self.origin.y - s.y) / self.scale,
        )
    }

    /// The world point under a screen point, to the nearest nanometer.
    pub fn to_world(&self, s: Screen) -> World {
        let (dx, dy) = self.offset(s);
        let (dx, dy) = (dx + self.sub.0, dy + self.sub.1);
        (
            clamp_i64(i128::from(self.center.0) + i128::from(round_i64(dx))),
            clamp_i64(i128::from(self.center.1) + i128::from(round_i64(dy))),
        )
    }

    /// Nanometers across `px` pixels.
    pub fn nm(&self, px: f64) -> i64 {
        round_i64(px / self.scale)
    }

    /// Pans by a screen delta: the world moves with the pointer.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.shift(-dx / self.scale, dy / self.scale);
    }

    /// Moves the exact center by (ax, ay) nanometers.
    fn shift(&mut self, ax: f64, ay: f64) {
        let (tx, ty) = (self.sub.0 + ax, self.sub.1 + ay);
        let (wx, wy) = (round_i64(tx), round_i64(ty));
        self.center = (
            clamp_i64(i128::from(self.center.0) + i128::from(wx)),
            clamp_i64(i128::from(self.center.1) + i128::from(wy)),
        );
        let residual = |t: f64, w: i64| {
            let r = t - w as f64;
            if r.abs() <= 0.5 { r } else { 0.0 }
        };
        self.sub = (residual(tx, wx), residual(ty, wy));
    }

    /// Zooms by `factor` keeping the world point under `at` where it is.
    pub fn zoom_about(&mut self, at: Screen, factor: f64) {
        let scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        let (ox, oy) = self.offset(at);
        let nx = (at.x - self.origin.x) / scale;
        let ny = (self.origin.y - at.y) / scale;
        self.shift(ox - nx, oy - ny);
        self.scale = scale;
    }

    #[cfg(test)]
    pub fn at_max_zoom(&self) -> bool {
        self.scale >= MAX_SCALE
    }
}

/// Clips a closed polygon in screen space to the box `lo`..`hi`
/// (Sutherland-Hodgman), so nothing far off screen reaches f32.
pub fn clip_polygon(points: &[Screen], lo: Screen, hi: Screen) -> Vec<Screen> {
    let mut out = points.to_vec();
    for edge in 0..4_u8 {
        let inside = |p: &Screen, lo: &Screen, hi: &Screen| match edge {
            0 => p.x >= lo.x,
            1 => p.x <= hi.x,
            2 => p.y >= lo.y,
            _ => p.y <= hi.y,
        };
        let input = std::mem::take(&mut out);
        let Some(&last) = input.last() else {
            return out;
        };
        let mut prev = last;
        for &cur in &input {
            let (ci, pi) = (inside(&cur, &lo, &hi), inside(&prev, &lo, &hi));
            if ci != pi {
                out.push(cross(prev, cur, edge, lo, hi));
            }
            if ci {
                out.push(cur);
            }
            prev = cur;
        }
    }
    out
}

/// Clips a segment to the box; None when it lies wholly outside.
pub fn clip_segment(a: Screen, b: Screen, lo: Screen, hi: Screen) -> Option<(Screen, Screen)> {
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    for (p, q) in [
        (-dx, a.x - lo.x),
        (dx, hi.x - a.x),
        (-dy, a.y - lo.y),
        (dy, hi.y - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    (t0 <= t1).then_some((
        Screen {
            x: a.x + t0 * dx,
            y: a.y + t0 * dy,
        },
        Screen {
            x: a.x + t1 * dx,
            y: a.y + t1 * dy,
        },
    ))
}

fn cross(a: Screen, b: Screen, edge: u8, lo: Screen, hi: Screen) -> Screen {
    let along_x = |x: f64| {
        let t = (x - a.x) / (b.x - a.x);
        Screen {
            x,
            y: a.y + t * (b.y - a.y),
        }
    };
    let along_y = |y: f64| {
        let t = (y - a.y) / (b.y - a.y);
        Screen {
            x: a.x + t * (b.x - a.x),
            y,
        }
    };
    match edge {
        0 => along_x(lo.x),
        1 => along_x(hi.x),
        2 => along_y(lo.y),
        _ => along_y(hi.y),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IN: i64 = 25_400_000;
    const FT: i64 = 12 * IN;
    const G: i64 = 1_587_500;
    /// A mile from the origin, where an f32 world coordinate is coarser
    /// than a pixel at close zoom.
    const FAR: i64 = 5280 * FT;

    fn origin() -> Screen {
        Screen { x: 640.0, y: 400.0 }
    }

    /// A 60 ft x 40 ft building whose corner sits a mile northeast of the
    /// origin.
    fn building() -> (World, World) {
        ((FAR, FAR), (FAR + 60 * FT, FAR + 40 * FT))
    }

    fn whole_building() -> Viewport {
        let (min, max) = building();
        Viewport::fit(min, max, (1280.0, 800.0), 40.0, origin())
    }

    /// What reaches the GPU: f32.
    fn drawn(v: &Viewport, p: World) -> (f32, f32) {
        let s = v.to_screen(p);
        (s.x as f32, s.y as f32)
    }

    #[test]
    fn the_whole_building_fits() {
        let v = whole_building();
        let (min, max) = building();
        for p in [min, max, (min.0, max.1), (max.0, min.1)] {
            let s = v.to_screen(p);
            assert!(
                (39.0..=1241.0).contains(&s.x) && (39.0..=761.0).contains(&s.y),
                "{s:?}"
            );
        }
    }

    /// Done when (part 1): no jitter at the closest zoom. Successive 1/16 in
    /// grid points a mile from the origin land exactly MAX_PX_PER_GRID
    /// pixels apart after the f32 conversion, frame after frame.
    #[test]
    fn no_jitter_at_one_sixteenth_inch() {
        let corner = (FAR + 17 * FT + 3 * G, FAR + 9 * FT + 11 * G);
        let mut v = Viewport::new(corner, MAX_SCALE, origin());
        assert!(v.at_max_zoom());
        let (x0, y0) = drawn(&v, corner);
        assert_eq!((x0, y0), (640.0, 400.0));
        for k in -2..=2_i64 {
            let (x, y) = drawn(&v, (corner.0 + k * G, corner.1 + k * G));
            let want = 400.0 * k as f32;
            assert!((x - 640.0 - want).abs() < 1e-3, "x at {k}: {x}");
            assert!((400.0 - y - want).abs() < 1e-3, "y at {k}: {y}");
        }
        // One nanometer is visible as a steady sub-pixel step.
        let (x1, _) = drawn(&v, (corner.0 + 1, corner.1));
        assert!((x1 - 640.0 - MAX_SCALE as f32).abs() < 1e-5, "{x1}");
        // Whole-pixel pans move every point by exactly that much.
        for _ in 0..1000 {
            v.pan(3.0, -2.0);
        }
        let (x, y) = drawn(&v, corner);
        assert!(
            (x - 3640.0).abs() < 1e-3 && (y - -1600.0).abs() < 1e-3,
            "{x} {y}"
        );
    }

    /// The control: the naive transform, world coordinates in f32 as egui
    /// and the GPU hold them, cannot place a mile-out point within a pixel
    /// at this zoom. This is the jitter the integer center removes.
    #[test]
    fn naive_f32_world_coordinates_would_jitter() {
        let corner = FAR + 17 * FT + 3 * G;
        let errors: Vec<f32> = (0..16_i64)
            .map(|k| {
                let p = corner + k * G;
                let naive = (p as f32 - corner as f32) * MAX_SCALE as f32;
                (naive - 400.0 * k as f32).abs()
            })
            .collect();
        assert!(
            errors.iter().any(|e| *e > 10.0),
            "f32 world coordinates err by {errors:?} px"
        );
    }

    /// Done when (part 1): zooming from the whole building to 1/16 in and
    /// back about a fixed screen point keeps the world point under it still
    /// at every step, at both extremes.
    #[test]
    fn zooming_from_the_whole_building_to_one_sixteenth_inch_shows_no_jitter() {
        let mut v = whole_building();
        // A wall corner, off the grid by nothing, and the pointer on it.
        let corner = (
            FAR + 22 * FT + 7 * IN + 5 * G,
            FAR + 31 * FT + 2 * IN + 9 * G,
        );
        let at = v.to_screen(corner);
        let mut steps = 0;
        while !v.at_max_zoom() {
            v.zoom_about(at, 1.1);
            let s = v.to_screen(corner);
            assert!(
                (s.x - at.x).abs() < 0.01 && (s.y - at.y).abs() < 0.01,
                "drift at step {steps}: {s:?} vs {at:?}"
            );
            steps += 1;
            assert!(steps < 1000);
        }
        // At 1/16 in the neighbouring grid point is MAX_PX_PER_GRID away.
        let here = v.to_screen(corner);
        let next = v.to_screen((corner.0 + G, corner.1));
        assert!((next.x - here.x - MAX_PX_PER_GRID).abs() < 1e-6);
        // And back out past the whole building to the far limit.
        while v.scale() > MIN_SCALE {
            v.zoom_about(at, 1.0 / 1.1);
            let s = v.to_screen(corner);
            assert!(
                (s.x - at.x).abs() < 0.01 && (s.y - at.y).abs() < 0.01,
                "drift zooming out: {s:?} vs {at:?}"
            );
        }
        // The pointer still reads the same world point to the nanometer.
        assert_eq!(v.to_world(at), corner);
    }

    #[test]
    fn screen_to_world_round_trips_to_the_nanometer_at_close_zoom() {
        let v = Viewport::new((FAR, -FAR), MAX_SCALE, origin());
        for p in [
            (FAR, -FAR),
            (FAR + 12_345, -FAR - 678),
            (FAR - 3 * G, -FAR + G),
        ] {
            assert_eq!(v.to_world(v.to_screen(p)), p);
        }
    }

    #[test]
    fn extreme_coordinates_do_not_overflow() {
        let mut v = Viewport::new((i64::MAX, i64::MIN), MIN_SCALE, origin());
        v.pan(-1e9, 1e9);
        v.zoom_about(Screen { x: 0.0, y: 0.0 }, 1e-9);
        let w = v.to_world(Screen { x: 1e12, y: -1e12 });
        assert!(w.0 >= i64::MAX - 2 && w.1 >= i64::MAX - 2, "{w:?}");
        assert!(v.to_screen((i64::MIN, i64::MAX)).x.is_finite());
    }

    #[test]
    fn far_geometry_is_clipped_before_f32() {
        // A 100 ft wall at 1/16 in zoom spans millions of pixels.
        let v = Viewport::new((FAR, FAR), MAX_SCALE, origin());
        let wall: Vec<Screen> = [
            (FAR - 50 * FT, FAR),
            (FAR + 50 * FT, FAR),
            (FAR + 50 * FT, FAR + 6 * IN),
            (FAR - 50 * FT, FAR + 6 * IN),
        ]
        .into_iter()
        .map(|p| v.to_screen(p))
        .collect();
        let (lo, hi) = (
            Screen { x: -64.0, y: -64.0 },
            Screen {
                x: 1344.0,
                y: 864.0,
            },
        );
        let clipped = clip_polygon(&wall, lo, hi);
        assert!(!clipped.is_empty());
        for p in &clipped {
            assert!(p.x >= lo.x - 1e-9 && p.x <= hi.x + 1e-9, "{p:?}");
            assert!(p.y >= lo.y - 1e-9 && p.y <= hi.y + 1e-9, "{p:?}");
        }
        // The face at y = FAR is still exactly on the center row.
        assert!(clipped.iter().any(|p| (p.y - 400.0).abs() < 1e-9));
        let seg = clip_segment(wall[0], wall[1], lo, hi);
        let (a, b) = seg.unwrap_or_else(|| panic!("the wall crosses the canvas"));
        assert!(
            (a.x + 64.0).abs() < 1e-6 && (b.x - 1344.0).abs() < 1e-6,
            "{a:?} {b:?}"
        );
        assert!((a.y - 400.0).abs() < 1e-9 && (b.y - 400.0).abs() < 1e-9);
        let outside = clip_segment(
            Screen { x: -500.0, y: 0.0 },
            Screen { x: -100.0, y: 10.0 },
            lo,
            hi,
        );
        assert_eq!(outside, None);
    }
}
