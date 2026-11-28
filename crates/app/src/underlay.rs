//! Underlays (T-205, spec 6.6, Q-83, DEC-14): import a PNG or JPEG of the
//! paper plan into the project's `assets` folder, place it by two-point
//! calibration against a known distance, and report the residual of a second
//! known distance.
//!
//! An underlay is display-only. Clicks on the image become integer pixel
//! positions and the click that places it is snapped like any plan point;
//! the calibration itself is a CreateUnderlay or SetUnderlayCalibration
//! command, reviewed and confirmed like every proposal. Floats appear only
//! in `Placement`, which draws the image and turns a click into a pixel.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use archviz_core::{
    Action, ContentHash, CreateUnderlay, Direction, KnownDistance, Length, PIXEL_MAX, PixelPoint,
    Point, ProjectUnit, SetUnderlayCalibration, Source, Sourced, Underlay, Uuid, direction_of,
    format_length, residual, scaled_length,
};
use sha2::{Digest, Sha256};

use crate::dimension::parse_value;
use crate::viewport::World;

/// The largest image accepted, per side and in total (Q-83 (2)(a)).
pub const MAX_SIDE: u32 = 16_384;
pub const MAX_PIXELS: u64 = 64_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Jpeg,
}

impl Format {
    pub const ALL: [Format; 2] = [Format::Png, Format::Jpeg];

    fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
        }
    }

    fn image(self) -> image::ImageFormat {
        match self {
            Format::Png => image::ImageFormat::Png,
            Format::Jpeg => image::ImageFormat::Jpeg,
        }
    }
}

/// The format named by a file's first bytes.
pub fn format_of(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else {
        None
    }
}

/// A decoded image, 8-bit RGBA, rows top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

fn check_size(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("the image is empty".to_string());
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!(
            "the image is {width} x {height} px; at most 16,384 px on a side is accepted"
        ));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!(
            "the image is {width} x {height} px; at most 64 megapixels is accepted"
        ));
    }
    Ok(())
}

/// Decodes a PNG or JPEG. Its size is read from the header and checked
/// before any pixel is decoded, so an oversized image or a decompression
/// bomb is refused without decoding it.
pub fn decode(bytes: &[u8]) -> Result<Image, String> {
    let format = format_of(bytes).ok_or("not a PNG or JPEG image")?;
    let reader = || image::ImageReader::with_format(Cursor::new(bytes), format.image());
    let (width, height) = reader()
        .into_dimensions()
        .map_err(|e| format!("cannot read the image: {e}"))?;
    check_size(width, height)?;
    // The decoder's own limits back up the check above.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    let mut decoder = reader();
    decoder.limits(limits);
    let rgba = decoder
        .decode()
        .map_err(|e| format!("cannot read the image: {e}"))?
        .to_rgba8()
        .into_raw();
    Ok(Image {
        width,
        height,
        rgba,
    })
}

/// `sha256:` and the SHA-256 of `bytes` in lowercase hex.
pub fn content_hash(bytes: &[u8]) -> ContentHash {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    ContentHash::parse(&format!("sha256:{hex}")).unwrap_or_else(|| unreachable!())
}

/// Where an image with `hash` is kept in the project folder `dir`.
pub fn asset_file(dir: &Path, hash: &ContentHash, format: Format) -> PathBuf {
    let hex = hash.as_str().trim_start_matches("sha256:");
    dir.join(archviz_cli::store::ASSETS)
        .join(format!("{hex}.{}", format.extension()))
}

/// An image copied into the project.
pub struct Imported {
    pub hash: ContentHash,
    pub image: Image,
    /// The file name without its extension, a default underlay name.
    pub stem: String,
}

/// Reads `source`, checks it decodes, and copies it into `dir/assets` under
/// its content hash (written whole, then renamed into place). Importing the
/// same file again writes nothing.
pub fn import(dir: &Path, source: &Path) -> Result<Imported, String> {
    let bytes = std::fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let image = decode(&bytes)?;
    let format = format_of(&bytes).ok_or("not a PNG or JPEG image")?;
    let hash = content_hash(&bytes);
    let file = asset_file(dir, &hash, format);
    if std::fs::read(&file).ok().as_deref() != Some(bytes.as_slice()) {
        let folder = dir.join(archviz_cli::store::ASSETS);
        std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        let temporary = file.with_extension("partial");
        std::fs::write(&temporary, &bytes).map_err(|e| format!("{}: {e}", temporary.display()))?;
        std::fs::rename(&temporary, &file).map_err(|e| format!("{}: {e}", file.display()))?;
    }
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Imported { hash, image, stem })
}

/// Loads the image with `hash` from `dir/assets`, refusing a file whose
/// content no longer matches its hash.
pub fn load(dir: &Path, hash: &ContentHash) -> Result<Image, String> {
    for format in Format::ALL {
        let file = asset_file(dir, hash, format);
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        if content_hash(&bytes) != *hash {
            return Err(format!(
                "{} has changed since it was imported",
                file.display()
            ));
        }
        return decode(&bytes);
    }
    Err(format!(
        "the image {} is not in the project's assets folder",
        hash.as_str()
    ))
}

/// The axis direction nearest the direction from `a` to `b` in the image,
/// with image Y down turned to plan Y up; the default heading.
pub fn nearest_direction(a: PixelPoint, b: PixelPoint) -> Direction {
    let (dx, dy) = (b.x - a.x, a.y - b.y);
    if dx.abs() >= dy.abs() {
        if dx >= 0 {
            Direction::PosX
        } else {
            Direction::NegX
        }
    } else if dy > 0 {
        Direction::PosY
    } else {
        Direction::NegY
    }
}

fn unit(d: Direction) -> (f64, f64) {
    match d {
        Direction::PosX => (1.0, 0.0),
        Direction::NegX => (-1.0, 0.0),
        Direction::PosY => (0.0, 1.0),
        Direction::NegY => (0.0, -1.0),
    }
}

fn round_i64(v: f64) -> i64 {
    // Saturating float to integer conversion; display only.
    v.round() as i64
}

/// Where an image is drawn: `world = origin + M (p - anchor)` for a
/// position `p` in pixels, pixel (i, j) centered at (i, j). Display only.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    anchor: (f64, f64),
    origin: World,
    /// Row-major 2 x 2, nanometers per pixel.
    m: [f64; 4],
}

impl Placement {
    /// The placement a calibration gives: `pixel_a` at `origin`, `pixel_b`
    /// at the known distance from it toward `heading`. None if the two
    /// points coincide or the distance is not positive.
    pub fn calibrated(c: &KnownDistance, origin: Point, heading: Direction) -> Option<Placement> {
        let v = (
            (c.pixel_b.x - c.pixel_a.x) as f64,
            (c.pixel_a.y - c.pixel_b.y) as f64,
        );
        let len = v.0.hypot(v.1);
        let d = c.distance.value.nanometers() as f64;
        if len == 0.0 || d <= 0.0 {
            return None;
        }
        let s = d / len;
        let u = unit(heading);
        let cos = (v.0 * u.0 + v.1 * u.1) / len;
        let sin = (v.0 * u.1 - v.1 * u.0) / len;
        Some(Placement {
            anchor: (c.pixel_a.x as f64, c.pixel_a.y as f64),
            origin: (origin.x.nanometers(), origin.y.nanometers()),
            m: [s * cos, s * sin, s * sin, -s * cos],
        })
    }

    /// The placement of a stored underlay.
    pub fn of(u: &Underlay) -> Option<Placement> {
        Placement::calibrated(&u.calibration, u.origin, direction_of(u.heading)?)
    }

    /// An uncalibrated image of `size` pixels fitted inside `min`..`max`,
    /// upright, to pick calibration points on.
    pub fn fit(size: (u32, u32), min: World, max: World) -> Placement {
        let w = (i128::from(max.0) - i128::from(min.0)).max(1) as f64;
        let h = (i128::from(max.1) - i128::from(min.1)).max(1) as f64;
        let s = (w / f64::from(size.0.max(1))).min(h / f64::from(size.1.max(1)));
        Placement {
            anchor: (-0.5, -0.5),
            origin: (min.0, max.1),
            m: [s, 0.0, 0.0, -s],
        }
    }

    pub fn to_world(&self, p: (f64, f64)) -> World {
        let (dx, dy) = (p.0 - self.anchor.0, p.1 - self.anchor.1);
        let [a, b, c, d] = self.m;
        (
            round_i64(self.origin.0 as f64 + a * dx + b * dy),
            round_i64(self.origin.1 as f64 + c * dx + d * dy),
        )
    }

    pub fn to_pixel(&self, w: World) -> (f64, f64) {
        let [a, b, c, d] = self.m;
        let det = a * d - b * c;
        let rx = (i128::from(w.0) - i128::from(self.origin.0)) as f64;
        let ry = (i128::from(w.1) - i128::from(self.origin.1)) as f64;
        (
            self.anchor.0 + (d * rx - b * ry) / det,
            self.anchor.1 + (a * ry - c * rx) / det,
        )
    }

    /// The image's corners, top left first, clockwise in the image.
    pub fn corners(&self, size: (u32, u32)) -> [World; 4] {
        let (w, h) = (f64::from(size.0) - 0.5, f64::from(size.1) - 0.5);
        [(-0.5, -0.5), (w, -0.5), (w, h), (-0.5, h)].map(|p| self.to_world(p))
    }

    /// The pixel under `w`, if it lies on an image of `size`.
    pub fn pixel_at(&self, size: (u32, u32), w: World) -> Option<PixelPoint> {
        let (x, y) = self.to_pixel(w);
        let (x, y) = (round_i64(x), round_i64(y));
        let inside = (0..i64::from(size.0)).contains(&x)
            && (0..i64::from(size.1)).contains(&y)
            && x <= PIXEL_MAX
            && y <= PIXEL_MAX;
        inside.then_some(PixelPoint { x, y })
    }
}

/// What a calibration makes: a new underlay or a new placement of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    New { name: String, image: ContentHash },
    Existing(Uuid),
}

/// The next click a calibration takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    First,
    Second,
    Origin,
    CheckFirst,
    CheckSecond,
    Ready,
}

/// The calibration being made (Q-83 (3)(i)): two image points and their
/// known distance, the axis direction they run, where the first lies on the
/// plan, and optionally a check distance.
#[derive(Debug, Clone, PartialEq)]
pub struct Calibration {
    pub target: Target,
    pub size: (u32, u32),
    /// Where the image is drawn until the calibration is complete.
    pub start: Placement,
    pub a: Option<PixelPoint>,
    pub b: Option<PixelPoint>,
    pub distance: String,
    /// None: the direction nearest the image's.
    pub direction: Option<Direction>,
    pub origin: Option<World>,
    pub check_a: Option<PixelPoint>,
    pub check_b: Option<PixelPoint>,
    pub check_distance: String,
}

fn measured(value: Length) -> Sourced<Length> {
    Sourced {
        value,
        source: Source::Measured,
    }
}

impl Calibration {
    pub fn new(target: Target, size: (u32, u32), start: Placement) -> Calibration {
        Calibration {
            target,
            size,
            start,
            a: None,
            b: None,
            distance: String::new(),
            direction: None,
            origin: None,
            check_a: None,
            check_b: None,
            check_distance: String::new(),
        }
    }

    pub fn step(&self) -> Step {
        match (self.a, self.b, self.origin, self.check_a, self.check_b) {
            (None, ..) => Step::First,
            (_, None, ..) => Step::Second,
            (_, _, None, ..) => Step::Origin,
            (.., None, _) => Step::CheckFirst,
            (.., None) => Step::CheckSecond,
            _ => Step::Ready,
        }
    }

    pub fn instruction(&self) -> String {
        match self.step() {
            Step::First => "Underlay: click the first end of a known distance on the image",
            Step::Second => "Underlay: click the second end of the known distance on the image",
            Step::Origin => "Underlay: click where the first point lies on the plan",
            Step::CheckFirst => {
                "Underlay: click the first end of a check distance on the image, \
                 or Propose without one"
            }
            Step::CheckSecond => "Underlay: click the second end of the check distance",
            Step::Ready => "Underlay: type the distances in the panel at the right, then Propose",
        }
        .to_string()
    }

    /// The heading: the one chosen, else the nearest to the image's.
    pub fn heading(&self) -> Option<Direction> {
        self.direction
            .or_else(|| Some(nearest_direction(self.a?, self.b?)))
    }

    pub fn known(&self, unit: ProjectUnit) -> Option<KnownDistance> {
        Some(KnownDistance {
            pixel_a: self.a?,
            pixel_b: self.b?,
            distance: measured(parse_value(&self.distance, unit).ok()?),
        })
    }

    pub fn check(&self, unit: ProjectUnit) -> Option<KnownDistance> {
        Some(KnownDistance {
            pixel_a: self.check_a?,
            pixel_b: self.check_b?,
            distance: measured(parse_value(&self.check_distance, unit).ok()?),
        })
    }

    /// Where the image is drawn now: as calibrated once the calibration is
    /// complete, else where it started.
    pub fn placement(&self, unit: ProjectUnit) -> Placement {
        let calibrated = || {
            let (x, y) = self.origin?;
            let origin = Point {
                x: Length::from_nanometers(x),
                y: Length::from_nanometers(y),
            };
            Placement::calibrated(&self.known(unit)?, origin, self.heading()?)
        };
        calibrated().unwrap_or_else(|| self.start.clone())
    }

    /// A click at `at` (raw) or `snapped` (the plan's snap, for the origin).
    pub fn click(
        &mut self,
        unit: ProjectUnit,
        at: World,
        snapped: World,
    ) -> Result<(), &'static str> {
        let step = self.step();
        if step == Step::Origin {
            self.origin = Some(snapped);
            return Ok(());
        }
        if step == Step::Ready {
            return Err("the points are picked: Propose, or Esc to start over");
        }
        let p = self
            .placement(unit)
            .pixel_at(self.size, at)
            .ok_or("click on the image")?;
        let slot = match step {
            Step::First => &mut self.a,
            Step::Second if self.a == Some(p) => return Err("pick a second, different point"),
            Step::Second => &mut self.b,
            Step::CheckFirst => &mut self.check_a,
            Step::CheckSecond if self.check_a == Some(p) => {
                return Err("pick a second, different point");
            }
            _ => &mut self.check_b,
        };
        *slot = Some(p);
        Ok(())
    }

    /// Forgets the check distance.
    pub fn clear_check(&mut self) {
        self.check_a = None;
        self.check_b = None;
        self.check_distance.clear();
    }

    /// The residual of the check as core computes it, once both distances
    /// are typed and the check's points picked.
    pub fn residual(&self, unit: ProjectUnit) -> Option<Result<Length, String>> {
        let (known, check) = (self.known(unit)?, self.check(unit)?);
        Some(residual(&known, &check).map_err(|e| e.to_string()))
    }

    /// The command this calibration proposes on `storey`.
    pub fn action(&self, storey: Uuid, unit: ProjectUnit) -> Result<Action, String> {
        let (Some(_), Some(_)) = (self.a, self.b) else {
            return Err("pick both ends of the known distance on the image".to_string());
        };
        parse_value(&self.distance, unit).map_err(|e| format!("known distance: {e}"))?;
        let calibration = self.known(unit).ok_or("type the known distance")?;
        let (x, y) = self
            .origin
            .ok_or("click where the first point lies on the plan")?;
        let origin = Point {
            x: Length::from_nanometers(x),
            y: Length::from_nanometers(y),
        };
        let heading = self.heading().ok_or("pick the known distance")?;
        let check = match (self.check_a, self.check_b) {
            (None, None) => None,
            (Some(_), Some(_)) => {
                parse_value(&self.check_distance, unit)
                    .map_err(|e| format!("check distance: {e}"))?;
                self.check(unit)
            }
            _ => return Err("pick the second end of the check distance, or clear it".to_string()),
        };
        Ok(match &self.target {
            Target::New { name, image } => Action::CreateUnderlay(CreateUnderlay {
                name: name.clone(),
                storey,
                image: image.clone(),
                calibration,
                origin,
                heading,
                check,
            }),
            Target::Existing(underlay) => Action::SetUnderlayCalibration(SetUnderlayCalibration {
                underlay: *underlay,
                calibration,
                origin,
                heading,
                check,
            }),
        })
    }
}

fn direction_name(d: Direction) -> &'static str {
    match d {
        Direction::PosX => "+X",
        Direction::NegX => "-X",
        Direction::PosY => "+Y",
        Direction::NegY => "-Y",
    }
}

/// The residual line: the residual in feet-inches, the check distance, what
/// the calibration reads for it, and the residual as a percentage of the
/// check to one decimal (Q-83 (4)(i)).
pub fn residual_line(calibration: &KnownDistance, check: &KnownDistance, r: Length) -> String {
    let reads = scaled_length(calibration, check.pixel_a, check.pixel_b)
        .map(format_length)
        .unwrap_or_else(|e| e.to_string());
    let k = check.distance.value.nanometers();
    let percent = if k == 0 {
        0.0
    } else {
        r.nanometers() as f64 / k as f64 * 100.0
    };
    format!(
        "Residual {} ({percent:.1}%): check {} reads {reads}",
        format_length(r),
        format_length(check.distance.value)
    )
}

/// A stored underlay in words, for the panel: its calibration, then its
/// residual or that it has no check.
pub fn summary(u: &Underlay) -> Vec<String> {
    let c = &u.calibration;
    let heading = direction_of(u.heading).map_or("an angle", direction_name);
    let mut lines = vec![
        format!("Underlay {:?}", u.name),
        format!(
            "Known distance {} between pixels ({}, {}) and ({}, {}), toward {heading} from ({}, {})",
            format_length(c.distance.value),
            c.pixel_a.x,
            c.pixel_a.y,
            c.pixel_b.x,
            c.pixel_b.y,
            format_length(u.origin.x),
            format_length(u.origin.y)
        ),
    ];
    lines.push(match &u.check {
        Some(check) => residual_line(c, &check.known, check.residual),
        None => "No check distance: Recalibrate and add one to see the residual".to_string(),
    });
    lines
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const IN: i64 = 25_400_000;
    const FT: i64 = 12 * IN;
    const UNIT: ProjectUnit = ProjectUnit::Inches;

    fn px(x: i64, y: i64) -> PixelPoint {
        PixelPoint { x, y }
    }

    /// A PNG of `w` x `h` pixels, encoded by the image crate.
    pub(crate) fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png)
            .unwrap_or_else(|e| panic!("{e}"));
        out.into_inner()
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, _| image::Rgb([(x % 256) as u8, 90, 200]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Jpeg)
            .unwrap_or_else(|e| panic!("{e}"));
        out.into_inner()
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFF_u32;
        for b in bytes {
            crc ^= u32::from(*b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A PNG whose header claims `w` x `h` pixels and whose data is one
    /// small compressed stream: a decompression bomb if decoded.
    fn claimed_png(w: u32, h: u32) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let chunk = |out: &mut Vec<u8>, kind: &[u8], data: &[u8]| {
            out.extend((data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend(data);
            out.extend(&body);
            out.extend(crc32(&body).to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(
            &mut out,
            b"IDAT",
            &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
        );
        chunk(&mut out, b"IEND", &[]);
        out
    }

    pub(crate) fn folder(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("archviz-underlay-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        dir
    }

    #[test]
    fn png_and_jpeg_decode_and_anything_else_is_refused() {
        let image = decode(&png(5, 3)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(image.size(), (5, 3));
        assert_eq!(image.rgba.len(), 5 * 3 * 4);
        assert_eq!(image.rgba.get(4..8), Some(&[1, 0, 128, 255][..]));
        let image = decode(&jpeg(16, 8)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(image.size(), (16, 8));
        assert_eq!(
            decode(b"GIF89a....").err().as_deref(),
            Some("not a PNG or JPEG image")
        );
        let mut truncated = png(40, 40);
        truncated.truncate(60);
        let e = decode(&truncated).err().unwrap_or_default();
        assert!(e.starts_with("cannot read the image"), "{e}");
    }

    #[test]
    fn oversized_images_are_refused_before_decoding() {
        let e = decode(&png(MAX_SIDE + 1, 1)).err().unwrap_or_default();
        assert!(e.contains("16,384 px on a side"), "{e}");
        let e = decode(&claimed_png(10_000, 10_000))
            .err()
            .unwrap_or_default();
        assert!(e.contains("64 megapixels"), "{e}");
        let e = decode(&claimed_png(16_000, 4_001))
            .err()
            .unwrap_or_default();
        assert!(e.contains("64 megapixels"), "{e}");
        // The largest accepted header passes the size check; this one then
        // fails on its tiny data stream instead of allocating 256 MB first.
        let e = decode(&claimed_png(8_000, 8_000)).err().unwrap_or_default();
        assert!(e.starts_with("cannot read the image"), "{e}");
    }

    #[test]
    fn an_import_is_kept_under_its_hash_and_loads_back() {
        let dir = folder("import");
        let source = dir.join("Paper plan.png");
        let bytes = png(30, 20);
        std::fs::write(&source, &bytes).unwrap_or_else(|e| panic!("{e}"));
        let imported = import(&dir, &source).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(imported.stem, "Paper plan");
        assert_eq!(imported.image.size(), (30, 20));
        assert_eq!(imported.hash, content_hash(&bytes));
        let file = asset_file(&dir, &imported.hash, Format::Png);
        assert_eq!(
            file.file_name().map(|n| n.to_string_lossy().len()),
            Some(64 + 4)
        );
        assert_eq!(std::fs::read(&file).ok(), Some(bytes.clone()));
        assert_eq!(load(&dir, &imported.hash), Ok(imported.image.clone()));
        // Again: the same file, nothing else written.
        import(&dir, &source).unwrap_or_else(|e| panic!("{e}"));
        let files = std::fs::read_dir(dir.join("assets"))
            .map(Iterator::count)
            .ok();
        assert_eq!(files, Some(1));
        // A changed file is refused.
        std::fs::write(&file, png(31, 20)).unwrap_or_else(|e| panic!("{e}"));
        let e = load(&dir, &imported.hash).err().unwrap_or_default();
        assert!(e.contains("has changed since it was imported"), "{e}");
        let missing = content_hash(b"other");
        assert!(load(&dir, &missing).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SHA-256 of "abc" (FIPS 180-2).
    #[test]
    fn the_content_hash_is_sha256() {
        assert_eq!(
            content_hash(b"abc").as_str(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    fn known(a: PixelPoint, b: PixelPoint, nm: i64) -> KnownDistance {
        KnownDistance {
            pixel_a: a,
            pixel_b: b,
            distance: measured(Length::from_nanometers(nm)),
        }
    }

    fn point(x: i64, y: i64) -> Point {
        Point {
            x: Length::from_nanometers(x),
            y: Length::from_nanometers(y),
        }
    }

    fn close(a: World, b: World) -> bool {
        (a.0 - b.0).abs() <= 2 && (a.1 - b.1).abs() <= 2
    }

    #[test]
    fn a_calibrated_placement_puts_the_points_where_the_calibration_says() {
        let origin = (10 * FT, -4 * FT);
        for (a, b) in [
            (px(120, 88), px(2520, 88)),
            (px(2520, 88), px(120, 88)),
            (px(120, 1528), px(120, 88)),
            (px(10, 10), px(310, 410)),
        ] {
            for heading in [
                Direction::PosX,
                Direction::NegX,
                Direction::PosY,
                Direction::NegY,
            ] {
                let c = known(a, b, 20 * FT);
                let p = Placement::calibrated(&c, point(origin.0, origin.1), heading)
                    .unwrap_or_else(|| panic!("placement"));
                let at = |q: PixelPoint| p.to_world((q.x as f64, q.y as f64));
                assert!(close(at(a), origin), "{:?}", at(a));
                let (ux, uy) = unit(heading);
                let far = (
                    origin.0 + (ux * (20 * FT) as f64) as i64,
                    origin.1 + (uy * (20 * FT) as f64) as i64,
                );
                assert!(close(at(b), far), "{heading:?}: {:?} vs {far:?}", at(b));
                // A pixel reads back from its world position.
                for q in [a, b, px(700, 300)] {
                    let w = at(q);
                    assert_eq!(p.pixel_at((4000, 4000), w), Some(q));
                }
                // Not a mirror image: image right-of-line stays right of it.
                let [m0, m1, m2, m3] = p.m;
                assert!(m0 * m3 - m1 * m2 < 0.0, "image Y down flips once");
            }
        }
        let none = known(px(3, 3), px(3, 3), FT);
        assert_eq!(
            Placement::calibrated(&none, point(0, 0), Direction::PosX),
            None
        );
    }

    #[test]
    fn the_image_is_upright_with_a_horizontal_line_to_plus_x() {
        let c = known(px(0, 0), px(100, 0), 100 * IN);
        let p = Placement::calibrated(&c, point(0, 0), Direction::PosX)
            .unwrap_or_else(|| panic!("placement"));
        // Down the image is down the plan (-Y).
        assert!(close(p.to_world((0.0, 50.0)), (0, -50 * IN)));
        assert_eq!(
            p.corners((100, 50)),
            [
                (-IN / 2, IN / 2),
                (99 * IN + IN / 2, IN / 2),
                (99 * IN + IN / 2, -49 * IN - IN / 2),
                (-IN / 2, -49 * IN - IN / 2)
            ]
        );
        assert_eq!(nearest_direction(px(0, 0), px(100, 3)), Direction::PosX);
        assert_eq!(nearest_direction(px(0, 0), px(-100, 3)), Direction::NegX);
        assert_eq!(nearest_direction(px(0, 100), px(3, 0)), Direction::PosY);
        assert_eq!(nearest_direction(px(0, 0), px(3, 100)), Direction::NegY);
    }

    #[test]
    fn a_fitted_image_lies_inside_the_view() {
        let p = Placement::fit((400, 200), (0, 0), (40 * FT, 40 * FT));
        let [tl, tr, br, bl] = p.corners((400, 200));
        assert_eq!(tl, (0, 40 * FT));
        assert_eq!(tr, (40 * FT, 40 * FT));
        assert_eq!(br, (40 * FT, 20 * FT));
        assert_eq!(bl, (0, 20 * FT));
        assert_eq!(p.pixel_at((400, 200), (0, 0)), None, "below the image");
        assert_eq!(
            p.pixel_at((400, 200), (FT / 20, 40 * FT - FT / 20)),
            Some(px(0, 0))
        );
    }

    /// Picks the calibration through world clicks, as the plan reports them.
    pub(crate) fn calibrate(tool: &mut Calibration, clicks: &[PixelPoint], origin: World) {
        let world = |tool: &Calibration, q: PixelPoint| {
            tool.placement(UNIT).to_world((q.x as f64, q.y as f64))
        };
        let mut clicks = clicks.iter();
        for step in [Step::First, Step::Second] {
            assert_eq!(tool.step(), step);
            let q = *clicks.next().unwrap_or_else(|| panic!("click"));
            let w = world(tool, q);
            assert_eq!(tool.click(UNIT, w, (0, 0)), Ok(()));
        }
        assert_eq!(tool.step(), Step::Origin);
        assert_eq!(tool.click(UNIT, (1, 1), origin), Ok(()));
        for q in clicks {
            let w = world(tool, *q);
            assert_eq!(tool.click(UNIT, w, (0, 0)), Ok(()));
        }
    }

    #[test]
    fn a_calibration_proposes_create_underlay_with_its_residual() {
        let start = Placement::fit((3000, 2000), (0, 0), (60 * FT, 40 * FT));
        let hash = content_hash(b"scan");
        let mut tool = Calibration::new(
            Target::New {
                name: "Paper plan".into(),
                image: hash.clone(),
            },
            (3000, 2000),
            start,
        );
        assert_eq!(
            tool.click(UNIT, (-FT, -FT), (0, 0)),
            Err("click on the image")
        );
        let first = tool.start.to_world((120.0, 88.0));
        tool.click(UNIT, first, (0, 0))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            tool.click(UNIT, first, (0, 0)),
            Err("pick a second, different point")
        );
        tool.a = None;
        tool.distance = "20'".into();
        tool.check_distance = "12' 1/4\"".into();
        calibrate(
            &mut tool,
            &[px(120, 88), px(2520, 88), px(120, 88), px(120, 1528)],
            (2 * FT, 0),
        );
        assert_eq!(tool.step(), Step::Ready);
        assert_eq!(
            tool.residual(UNIT),
            Some(Ok(Length::from_nanometers(IN / 4)))
        );
        let storey = Uuid::from_bytes([1; 16]);
        assert_eq!(
            tool.action(storey, UNIT),
            Ok(Action::CreateUnderlay(CreateUnderlay {
                name: "Paper plan".into(),
                storey,
                image: hash,
                calibration: known(px(120, 88), px(2520, 88), 20 * FT),
                origin: point(2 * FT, 0),
                heading: Direction::PosX,
                check: Some(known(px(120, 88), px(120, 1528), 12 * FT + IN / 4)),
            }))
        );
        // Calibrated, the check's ends lie 12 ft apart on the plan.
        let p = tool.placement(UNIT);
        assert!(close(p.to_world((120.0, 88.0)), (2 * FT, 0)));
        assert!(close(p.to_world((120.0, 1528.0)), (2 * FT, -12 * FT)));
    }

    #[test]
    fn a_recalibration_proposes_set_underlay_calibration() {
        let id = Uuid::from_bytes([7; 16]);
        let start = Placement::fit((500, 500), (0, 0), (10 * FT, 10 * FT));
        let mut tool = Calibration::new(Target::Existing(id), (500, 500), start);
        tool.distance = "10'".into();
        calibrate(&mut tool, &[px(10, 400), px(10, 100)], (0, FT));
        tool.direction = Some(Direction::NegY);
        assert_eq!(tool.residual(UNIT), None, "no check");
        assert_eq!(
            tool.action(Uuid::from_bytes([1; 16]), UNIT),
            Ok(Action::SetUnderlayCalibration(SetUnderlayCalibration {
                underlay: id,
                calibration: known(px(10, 400), px(10, 100), 10 * FT),
                origin: point(0, FT),
                heading: Direction::NegY,
                check: None,
            }))
        );
        // Unfinished or untyped values are named, not proposed.
        tool.distance = "1/32\"".into();
        let e = tool.action(id, UNIT).err().unwrap_or_default();
        assert!(e.starts_with("known distance:"), "{e}");
        tool.distance = "10'".into();
        tool.check_a = Some(px(1, 1));
        let e = tool.action(id, UNIT).err().unwrap_or_default();
        assert!(e.contains("check distance"), "{e}");
        tool.clear_check();
        assert!(tool.action(id, UNIT).is_ok());
    }

    #[test]
    fn the_summary_shows_the_residual() {
        let u = Underlay {
            name: "Paper plan".into(),
            storey: Uuid::from_bytes([1; 16]),
            image: Uuid::from_bytes([2; 16]),
            calibration: known(px(120, 88), px(2520, 88), 20 * FT),
            origin: point(0, 0),
            heading: archviz_core::Millidegrees::of_direction(Direction::PosX),
            check: Some(archviz_core::CalibrationCheck {
                known: known(px(120, 88), px(120, 1528), 12 * FT + IN / 4),
                residual: Length::from_nanometers(IN / 4),
            }),
        };
        assert_eq!(
            summary(&u),
            [
                "Underlay \"Paper plan\"",
                "Known distance 20'-0\" between pixels (120, 88) and (2520, 88), toward +X from (0'-0\", 0'-0\")",
                "Residual 0'-0 1/4\" (0.2%): check 12'-0 1/4\" reads 12'-0\"",
            ]
        );
        let none = Underlay { check: None, ..u };
        assert_eq!(
            summary(&none).last().map(String::as_str),
            Some("No check distance: Recalibrate and add one to see the residual")
        );
    }
}
