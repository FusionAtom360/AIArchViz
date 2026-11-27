//! DXF export for AutoCAD 2018 (spec 11.1; Q-51 to Q-53, DEC-1).
//!
//! The file is DXF R2000 (AC1015) with $INSUNITS 1 (inches), $LUNITS 4
//! (architectural) and $LUPREC 4 (1/16 in), written with CRLF line ends. It
//! carries the full R2000 structure (tables, block records, model and paper
//! space blocks and layouts, an objects dictionary) with fixed handles, so
//! the same document always gives the same bytes.
//!
//! Content, each outline a closed LWPOLYLINE:
//! - wall footprints on WALLS, WALLS-EXISTING or WALLS-DEMOLISHED by function;
//! - each opening's void through its wall on OPENINGS;
//! - slab outlines on SLABS;
//! - occurrence footprints on EQUIPMENT, labels as text on EQUIPMENT-LABELS;
//! - each driving Distance constraint as a dimension line, two extension
//!   lines and its value in feet-inches on DIMENSIONS (the 11.1 fallback);
//! - each Point datum, the fiducials (spec 5.4, S-3), as a POINT on DATUMS
//!   with its name as left-aligned text inserted at the same point.
//!
//! Every layer names the Normal plot style placeholder (group 390), which
//! AutoCAD 2018 requires in R2000 files (found at gate G-3).
//!
//! Coordinates are decimal inches with 8 decimals, computed in integer
//! arithmetic; 1e-8 in is 0.254 nm, so every stored nanometer reads back.

use std::fmt;

use archviz_core::{
    Axis, ConstraintKind, ConstraintStatus, DatumGeometry, Direction, Document, Function,
    GeometryError, Length, Point, SolveError, SpaceError, boundary_outline, footprint,
    format_length, reference_coordinate, wall_geometry, wall_profiles,
};

/// Why a document cannot be exported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DxfError {
    Geometry(GeometryError),
    Space(SpaceError),
    Solve(SolveError),
}

impl fmt::Display for DxfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DxfError::Geometry(e) => write!(f, "{e}"),
            DxfError::Space(e) => write!(f, "{e}"),
            DxfError::Solve(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DxfError {}

/// `n / d` rounded half to even; `d` must be positive.
fn round_half_even(n: i128, d: i128) -> i128 {
    let q = n / d;
    let r = n % d;
    let twice = 2 * r.abs();
    if twice > d || (twice == d && q % 2 != 0) {
        q + r.signum()
    } else {
        q
    }
}

/// `nm` nanometers as decimal inches with 8 decimals.
pub fn inches(nm: i64) -> String {
    // 1e-8 in = 0.254 nm, so the value in 1e-8 in is nm * 1000 / 254.
    let e8 = round_half_even(i128::from(nm) * 1000, 254);
    let sign = if e8 < 0 { "-" } else { "" };
    let m = e8.unsigned_abs();
    format!("{sign}{}.{:08}", m / 100_000_000, m % 100_000_000)
}

/// Decimal inches back to nanometers (the inverse of [`inches`]).
pub fn parse_inches(text: &str) -> Option<i64> {
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, frac) = body.split_once('.').unwrap_or((body, ""));
    if frac.len() > 8
        || whole.is_empty()
        || !whole
            .bytes()
            .chain(frac.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole: i128 = whole.parse().ok()?;
    let frac: i128 = format!("{frac:0<8}").parse().ok()?;
    let e8 = whole.checked_mul(100_000_000)?.checked_add(frac)?;
    let e8 = if negative { -e8 } else { e8 };
    i64::try_from(round_half_even(e8 * 254, 1000)).ok()
}

const MODEL_SPACE: &str = "22";
/// The ACAD_PLOTSTYLENAME dictionary and its Normal placeholder.
const PLOT_STYLE_NAMES: &str = "35";
const PLOT_STYLE_NORMAL: &str = "36";
const LAYERS: &[(&str, u8)] = &[
    ("0", 7),
    ("WALLS", 7),
    ("WALLS-EXISTING", 8),
    ("WALLS-DEMOLISHED", 1),
    ("OPENINGS", 4),
    ("SLABS", 9),
    ("EQUIPMENT", 3),
    ("EQUIPMENT-LABELS", 3),
    ("DIMENSIONS", 2),
    ("DATUMS", 6),
];
const TEXT_HEIGHT: i64 = 6 * 25_400_000;
const IN: i64 = 25_400_000;

/// Group-code pairs, written with CRLF line ends.
struct Out {
    text: String,
    next_handle: u32,
}

impl Out {
    fn pair(&mut self, code: i32, value: &str) {
        self.text.push_str(&format!("{code:>3}\r\n{value}\r\n"));
    }

    fn pairs(&mut self, items: &[(i32, &str)]) {
        for (code, value) in items {
            self.pair(*code, value);
        }
    }

    fn handle(&mut self) -> String {
        let h = format!("{:X}", self.next_handle);
        self.next_handle += 1;
        h
    }

    fn entity(&mut self, kind: &str, layer: &str, subclass: &str) {
        let h = self.handle();
        self.pairs(&[
            (0, kind),
            (5, &h),
            (330, MODEL_SPACE),
            (100, "AcDbEntity"),
            (8, layer),
            (100, subclass),
        ]);
    }

    fn xy(&mut self, code: i32, p: (i64, i64)) {
        self.pair(code, &inches(p.0));
        self.pair(code + 10, &inches(p.1));
    }

    fn polyline(&mut self, layer: &str, points: &[(i64, i64)]) {
        self.entity("LWPOLYLINE", layer, "AcDbPolyline");
        self.pairs(&[(90, &points.len().to_string()), (70, "1"), (43, "0.0")]);
        for p in points {
            self.xy(10, *p);
        }
    }

    fn line(&mut self, layer: &str, a: (i64, i64), b: (i64, i64)) {
        self.entity("LINE", layer, "AcDbLine");
        self.xy(10, a);
        self.pair(30, "0.0");
        self.xy(11, b);
        self.pair(31, "0.0");
    }

    fn point(&mut self, layer: &str, at: (i64, i64)) {
        self.entity("POINT", layer, "AcDbPoint");
        self.xy(10, at);
        self.pair(30, "0.0");
    }

    /// Text with its baseline starting at `at`.
    fn label(&mut self, layer: &str, at: (i64, i64), value: &str) {
        self.entity("TEXT", layer, "AcDbText");
        self.xy(10, at);
        self.pair(30, "0.0");
        self.pair(40, &inches(TEXT_HEIGHT));
        self.pair(1, value);
        self.pairs(&[(50, "0.0"), (100, "AcDbText")]);
    }

    /// Text centered on `at`.
    fn text(&mut self, layer: &str, at: (i64, i64), value: &str, rotation: &str) {
        self.entity("TEXT", layer, "AcDbText");
        self.xy(10, at);
        self.pair(30, "0.0");
        self.pair(40, &inches(TEXT_HEIGHT));
        self.pair(1, value);
        self.pairs(&[(50, rotation), (72, "1")]);
        self.xy(11, at);
        self.pairs(&[(31, "0.0"), (100, "AcDbText"), (73, "2")]);
    }
}

/// What the ENTITIES section draws, collected first so the header can hold
/// the drawing's extents.
enum Item {
    Outline(&'static str, Vec<(i64, i64)>),
    Line(&'static str, (i64, i64), (i64, i64)),
    Text(&'static str, (i64, i64), String, &'static str),
    /// A labeled point: a POINT and its name inserted at it.
    Datum((i64, i64), String),
}

fn xy(p: &Point) -> (i64, i64) {
    (p.x.nanometers(), p.y.nanometers())
}

fn bounds(points: impl Iterator<Item = (i64, i64)>) -> Option<((i64, i64), (i64, i64))> {
    points.fold(None, |acc, (x, y)| {
        Some(match acc {
            None => ((x, y), (x, y)),
            Some(((x0, y0), (x1, y1))) => ((x0.min(x), y0.min(y)), (x1.max(x), y1.max(y))),
        })
    })
}

fn items(doc: &Document) -> Result<Vec<Item>, DxfError> {
    let mut out = Vec::new();
    let geometry = wall_geometry(doc).map_err(DxfError::Geometry)?;
    for (id, g) in &geometry {
        let layer = match doc.walls.get(id).map(|w| w.function) {
            Some(Function::Existing) => "WALLS-EXISTING",
            Some(Function::Demolished) => "WALLS-DEMOLISHED",
            _ => "WALLS",
        };
        out.push(Item::Outline(layer, g.footprint.iter().map(xy).collect()));
    }
    // Each opening's void: its span along the host, through the host's thickness.
    let profiles = wall_profiles(doc).map_err(DxfError::Geometry)?;
    let mut voids = Vec::new();
    for (id, p) in &profiles {
        let (Some(w), Some(g)) = (doc.walls.get(id), geometry.get(id)) else {
            continue;
        };
        let along_x = matches!(w.direction, Direction::PosX | Direction::NegX);
        let across: Vec<i64> = g
            .footprint
            .iter()
            .map(|p| {
                if along_x {
                    p.y.nanometers()
                } else {
                    p.x.nanometers()
                }
            })
            .collect();
        let (Some(&lo), Some(&hi)) = (across.iter().min(), across.iter().max()) else {
            continue;
        };
        for v in &p.voids {
            let (a, b) = (
                v.start.nanometers().min(v.end.nanometers()),
                v.start.nanometers().max(v.end.nanometers()),
            );
            let rect = if along_x {
                vec![(a, lo), (b, lo), (b, hi), (a, hi)]
            } else {
                vec![(lo, a), (hi, a), (hi, b), (lo, b)]
            };
            voids.push((v.opening, rect));
        }
    }
    voids.sort_by_key(|v| v.0);
    out.extend(voids.into_iter().map(|(_, r)| Item::Outline("OPENINGS", r)));
    for slab in doc.slabs.values() {
        let outline = boundary_outline(doc, &slab.boundary).map_err(DxfError::Space)?;
        out.push(Item::Outline("SLABS", outline.iter().map(xy).collect()));
    }
    for (id, o) in &doc.occurrences {
        let r = footprint(doc, *id).map_err(DxfError::Geometry)?;
        let (a, b) = (xy(&r.min), xy(&r.max));
        out.push(Item::Outline(
            "EQUIPMENT",
            vec![a, (b.0, a.1), b, (a.0, b.1)],
        ));
        out.push(Item::Text(
            "EQUIPMENT-LABELS",
            xy(&o.center),
            o.label.to_string(),
            "0.0",
        ));
    }
    for d in doc.datums.values() {
        if let DatumGeometry::Point { x, y } = d.geometry {
            out.push(Item::Datum(
                (x.nanometers(), y.nanometers()),
                d.name.clone(),
            ));
        }
    }
    // Dimensions (the 11.1 fallback), outside the drawing's extents.
    let extents = bounds(out.iter().flat_map(|i| match i {
        Item::Outline(_, pts) => pts.clone(),
        _ => vec![],
    }))
    .unwrap_or(((0, 0), (0, 0)));
    let (mut nx, mut ny) = (0i64, 0i64);
    for c in doc.constraints.values() {
        let ConstraintKind::Distance {
            a, b, axis, value, ..
        } = &c.kind
        else {
            continue;
        };
        if c.status != ConstraintStatus::Driving {
            continue;
        }
        let pa = reference_coordinate(doc, a, *axis)
            .map_err(DxfError::Solve)?
            .nanometers();
        let pb = reference_coordinate(doc, b, *axis)
            .map_err(DxfError::Solve)?
            .nanometers();
        let label = format_length(Length::from_nanometers(value.nanometers().saturating_abs()));
        let mid = pa / 2 + pb / 2;
        match axis {
            Axis::X => {
                let top = extents.1.1;
                let at = top + 24 * IN + 12 * IN * nx;
                nx += 1;
                out.push(Item::Line("DIMENSIONS", (pa, top), (pa, at + 3 * IN)));
                out.push(Item::Line("DIMENSIONS", (pb, top), (pb, at + 3 * IN)));
                out.push(Item::Line("DIMENSIONS", (pa, at), (pb, at)));
                out.push(Item::Text("DIMENSIONS", (mid, at + 4 * IN), label, "0.0"));
            }
            Axis::Y => {
                let right = extents.1.0;
                let at = right + 24 * IN + 12 * IN * ny;
                ny += 1;
                out.push(Item::Line("DIMENSIONS", (right, pa), (at + 3 * IN, pa)));
                out.push(Item::Line("DIMENSIONS", (right, pb), (at + 3 * IN, pb)));
                out.push(Item::Line("DIMENSIONS", (at, pa), (at, pb)));
                out.push(Item::Text("DIMENSIONS", (at - 4 * IN, mid), label, "90.0"));
            }
        }
    }
    Ok(out)
}

fn table(out: &mut Out, name: &str, handle: &str, count: usize) {
    out.pairs(&[
        (0, "TABLE"),
        (2, name),
        (5, handle),
        (330, "0"),
        (100, "AcDbSymbolTable"),
        (70, &count.to_string()),
    ]);
}

fn record(out: &mut Out, kind: &str, handle: &str, owner: &str, subclass: &str, name: &str) {
    out.pairs(&[
        (0, kind),
        (5, handle),
        (330, owner),
        (100, "AcDbSymbolTableRecord"),
        (100, subclass),
        (2, name),
        (70, "0"),
    ]);
}

fn layout(out: &mut Out, handle: &str, name: &str, flags: &str, tab: &str, block_record: &str) {
    out.pairs(&[
        (0, "LAYOUT"),
        (5, handle),
        (330, "32"),
        (100, "AcDbPlotSettings"),
        (1, ""),
        (2, "none_device"),
        (4, ""),
        (6, ""),
    ]);
    for code in [40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 140, 141] {
        out.pair(code, "0.0");
    }
    out.pairs(&[
        (142, "1.0"),
        (143, "1.0"),
        (70, "688"),
        (72, "0"),
        (73, "0"),
        (74, "5"),
        (7, ""),
        (75, "16"),
        (147, "1.0"),
        (148, "0.0"),
        (149, "0.0"),
        (100, "AcDbLayout"),
        (1, name),
        (70, flags),
        (71, tab),
        (10, "0.0"),
        (20, "0.0"),
        (11, "12.0"),
        (21, "9.0"),
        (12, "0.0"),
        (22, "0.0"),
        (32, "0.0"),
        (14, "0.0"),
        (24, "0.0"),
        (34, "0.0"),
        (15, "0.0"),
        (25, "0.0"),
        (35, "0.0"),
        (146, "0.0"),
        (13, "0.0"),
        (23, "0.0"),
        (33, "0.0"),
        (16, "1.0"),
        (26, "0.0"),
        (36, "0.0"),
        (17, "0.0"),
        (27, "1.0"),
        (37, "0.0"),
        (76, "0"),
        (330, block_record),
    ]);
}

/// The DXF text of `document`, which should be derived (solved).
pub fn write_dxf(document: &Document) -> Result<String, DxfError> {
    let items = items(document)?;
    let points = items.iter().flat_map(|i| match i {
        Item::Outline(_, pts) => pts.clone(),
        Item::Line(_, a, b) => vec![*a, *b],
        Item::Text(_, p, _, _) | Item::Datum(p, _) => vec![*p],
    });
    let (min, max) = bounds(points).unwrap_or(((0, 0), (0, 0)));
    // Entities take handles from 0x100 up; fixed objects use lower handles.
    let mut body = Out {
        text: String::new(),
        next_handle: 0x100,
    };
    for item in &items {
        match item {
            Item::Outline(layer, pts) => body.polyline(layer, pts),
            Item::Line(layer, a, b) => body.line(layer, *a, *b),
            Item::Text(layer, at, value, rotation) => body.text(layer, *at, value, rotation),
            Item::Datum(at, name) => {
                body.point("DATUMS", *at);
                body.label("DATUMS", *at, name);
            }
        }
    }
    let seed = format!("{:X}", body.next_handle);

    let mut out = Out {
        text: String::new(),
        next_handle: 0,
    };
    // HEADER
    out.pairs(&[(0, "SECTION"), (2, "HEADER")]);
    out.pairs(&[
        (9, "$ACADVER"),
        (1, "AC1015"),
        (9, "$DWGCODEPAGE"),
        (3, "ANSI_1252"),
        (9, "$HANDSEED"),
        (5, &seed),
    ]);
    out.pairs(&[
        (9, "$INSUNITS"),
        (70, "1"),
        (9, "$LUNITS"),
        (70, "4"),
        (9, "$LUPREC"),
        (70, "4"),
        (9, "$MEASUREMENT"),
        (70, "0"),
    ]);
    out.pair(9, "$EXTMIN");
    out.xy(10, min);
    out.pair(30, "0.0");
    out.pair(9, "$EXTMAX");
    out.xy(10, max);
    out.pair(30, "0.0");
    out.pair(0, "ENDSEC");
    out.pairs(&[(0, "SECTION"), (2, "CLASSES"), (0, "ENDSEC")]);
    // TABLES
    out.pairs(&[(0, "SECTION"), (2, "TABLES")]);
    table(&mut out, "VPORT", "2", 1);
    record(
        &mut out,
        "VPORT",
        "3",
        "2",
        "AcDbViewportTableRecord",
        "*Active",
    );
    let width = (max.0 - min.0).max(IN);
    let height = (max.1 - min.1).max(IN);
    let centre = (min.0 / 2 + max.0 / 2, min.1 / 2 + max.1 / 2);
    out.pairs(&[(10, "0.0"), (20, "0.0"), (11, "1.0"), (21, "1.0")]);
    out.xy(12, centre);
    out.pairs(&[
        (13, "0.0"),
        (23, "0.0"),
        (14, "0.5"),
        (24, "0.5"),
        (15, "0.5"),
        (25, "0.5"),
        (16, "0.0"),
        (26, "0.0"),
        (36, "1.0"),
        (17, "0.0"),
        (27, "0.0"),
        (37, "0.0"),
    ]);
    out.pair(40, &inches(height + height / 5));
    out.pair(41, &format!("{}", width / height.max(1) + 1));
    out.pairs(&[
        (42, "50.0"),
        (43, "0.0"),
        (44, "0.0"),
        (50, "0.0"),
        (51, "0.0"),
        (71, "0"),
        (72, "100"),
        (73, "1"),
        (74, "3"),
        (75, "0"),
        (76, "0"),
        (77, "0"),
        (78, "0"),
    ]);
    out.pair(0, "ENDTAB");
    table(&mut out, "LTYPE", "4", 3);
    for (h, name, description) in [
        ("5", "ByBlock", ""),
        ("6", "ByLayer", ""),
        ("7", "Continuous", "Solid line"),
    ] {
        record(&mut out, "LTYPE", h, "4", "AcDbLinetypeTableRecord", name);
        out.pairs(&[(3, description), (72, "65"), (73, "0"), (40, "0.0")]);
    }
    out.pair(0, "ENDTAB");
    table(&mut out, "LAYER", "8", LAYERS.len());
    for (i, (name, colour)) in LAYERS.iter().enumerate() {
        let h = format!("{:X}", 0x10 + i);
        record(&mut out, "LAYER", &h, "8", "AcDbLayerTableRecord", name);
        // R2000 layers must name a plot style (G-3): the Normal placeholder.
        out.pairs(&[
            (62, &colour.to_string()),
            (6, "Continuous"),
            (390, PLOT_STYLE_NORMAL),
        ]);
    }
    out.pair(0, "ENDTAB");
    table(&mut out, "STYLE", "9", 1);
    record(
        &mut out,
        "STYLE",
        "A",
        "9",
        "AcDbTextStyleTableRecord",
        "Standard",
    );
    out.pairs(&[
        (40, "0.0"),
        (41, "1.0"),
        (50, "0.0"),
        (71, "0"),
        (42, "2.5"),
        (3, "txt"),
        (4, ""),
    ]);
    out.pair(0, "ENDTAB");
    table(&mut out, "VIEW", "B", 0);
    out.pair(0, "ENDTAB");
    table(&mut out, "UCS", "C", 0);
    out.pair(0, "ENDTAB");
    table(&mut out, "APPID", "D", 1);
    record(&mut out, "APPID", "E", "D", "AcDbRegAppTableRecord", "ACAD");
    out.pair(0, "ENDTAB");
    table(&mut out, "DIMSTYLE", "F", 1);
    out.pairs(&[(100, "AcDbDimStyleTable"), (71, "1")]);
    out.pairs(&[
        (0, "DIMSTYLE"),
        (105, "20"),
        (330, "F"),
        (100, "AcDbSymbolTableRecord"),
        (100, "AcDbDimStyleTableRecord"),
        (2, "Standard"),
        (70, "0"),
    ]);
    out.pair(0, "ENDTAB");
    table(&mut out, "BLOCK_RECORD", "21", 2);
    record(
        &mut out,
        "BLOCK_RECORD",
        "22",
        "21",
        "AcDbBlockTableRecord",
        "*Model_Space",
    );
    out.pair(340, "33");
    record(
        &mut out,
        "BLOCK_RECORD",
        "23",
        "21",
        "AcDbBlockTableRecord",
        "*Paper_Space",
    );
    out.pair(340, "34");
    out.pairs(&[(0, "ENDTAB"), (0, "ENDSEC")]);
    // BLOCKS
    out.pairs(&[(0, "SECTION"), (2, "BLOCKS")]);
    for (begin, end, owner, name, paper) in [
        ("24", "25", "22", "*Model_Space", false),
        ("26", "27", "23", "*Paper_Space", true),
    ] {
        out.pairs(&[(0, "BLOCK"), (5, begin), (330, owner), (100, "AcDbEntity")]);
        if paper {
            out.pair(67, "1");
        }
        out.pairs(&[
            (8, "0"),
            (100, "AcDbBlockBegin"),
            (2, name),
            (70, "0"),
            (10, "0.0"),
            (20, "0.0"),
            (30, "0.0"),
            (3, name),
            (1, ""),
        ]);
        out.pairs(&[(0, "ENDBLK"), (5, end), (330, owner), (100, "AcDbEntity")]);
        if paper {
            out.pair(67, "1");
        }
        out.pairs(&[(8, "0"), (100, "AcDbBlockEnd")]);
    }
    out.pair(0, "ENDSEC");
    // ENTITIES
    out.pairs(&[(0, "SECTION"), (2, "ENTITIES")]);
    out.text.push_str(&body.text);
    out.pair(0, "ENDSEC");
    // OBJECTS
    out.pairs(&[(0, "SECTION"), (2, "OBJECTS")]);
    out.pairs(&[
        (0, "DICTIONARY"),
        (5, "30"),
        (330, "0"),
        (100, "AcDbDictionary"),
        (281, "1"),
        (3, "ACAD_GROUP"),
        (350, "31"),
        (3, "ACAD_LAYOUT"),
        (350, "32"),
        (3, "ACAD_PLOTSTYLENAME"),
        (350, PLOT_STYLE_NAMES),
    ]);
    out.pairs(&[
        (0, "DICTIONARY"),
        (5, "31"),
        (330, "30"),
        (100, "AcDbDictionary"),
        (281, "1"),
    ]);
    out.pairs(&[
        (0, "DICTIONARY"),
        (5, "32"),
        (330, "30"),
        (100, "AcDbDictionary"),
        (281, "1"),
        (3, "Layout1"),
        (350, "34"),
        (3, "Model"),
        (350, "33"),
    ]);
    layout(&mut out, "33", "Model", "1", "0", "22");
    layout(&mut out, "34", "Layout1", "0", "1", "23");
    // The plot style names dictionary, whose default is the Normal placeholder.
    out.pairs(&[
        (0, "ACDBDICTIONARYWDFLT"),
        (5, PLOT_STYLE_NAMES),
        (102, "{ACAD_REACTORS"),
        (330, "30"),
        (102, "}"),
        (330, "30"),
        (100, "AcDbDictionary"),
        (281, "1"),
        (3, "Normal"),
        (350, PLOT_STYLE_NORMAL),
        (100, "AcDbDictionaryWithDefault"),
        (340, PLOT_STYLE_NORMAL),
    ]);
    out.pairs(&[
        (0, "ACDBPLACEHOLDER"),
        (5, PLOT_STYLE_NORMAL),
        (102, "{ACAD_REACTORS"),
        (330, PLOT_STYLE_NAMES),
        (102, "}"),
        (330, PLOT_STYLE_NAMES),
    ]);
    out.pairs(&[(0, "ENDSEC"), (0, "EOF")]);
    Ok(out.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archviz_core::*;
    use proptest::prelude::*;
    use std::collections::{BTreeMap, BTreeSet};

    const IN: i64 = 25_400_000;

    fn len(nm: i64) -> Length {
        Length::from_nanometers(nm)
    }

    fn uid(kind: u8, n: u8) -> Uuid {
        let mut b = [0u8; 16];
        b[0] = kind;
        b[15] = n;
        Uuid::from_bytes(b)
    }

    fn user(nm: i64) -> Sourced<Length> {
        Sourced {
            value: len(nm),
            source: Source::UserStated,
        }
    }

    /// A 20 ft by 12 ft room (wall centerlines) of 4 in walls: the south wall
    /// existing, the west wall demolished, with a door, a slab, a bench, a
    /// light and two driving dimensions.
    fn plan() -> Document {
        let mut doc = Document::new(
            Project {
                id: uid(9, 0),
                name: "Shop".into(),
                default_unit: ProjectUnit::Inches,
            },
            Building {
                id: uid(9, 1),
                name: "Main".into(),
            },
        );
        let storey = uid(8, 0);
        doc.storeys.insert(
            storey,
            Storey {
                name: "Ground".into(),
                elevation: user(0),
                floor_to_floor: user(120 * IN),
                default_ceiling_height: user(108 * IN),
            },
        );
        doc.layer_sets.insert(
            uid(5, 0),
            LayerSet {
                name: "4 in".into(),
                layers: vec![Layer {
                    material: uid(6, 0),
                    thickness: user(4 * IN),
                    core: true,
                }],
            },
        );
        let corners = [(0, 0), (240, 0), (240, 144), (0, 144)];
        for (i, (x, y)) in corners.iter().enumerate() {
            let n = u8::try_from(i).unwrap_or(0);
            doc.nodes.insert(
                uid(1, n),
                Node {
                    x: len(x * IN),
                    y: len(y * IN),
                    join: JoinType::Isolated,
                },
            );
        }
        let mut labels = LabelAllocator::new();
        let walls = [
            (0, 1, Direction::PosX, Function::Existing),
            (1, 2, Direction::PosY, Function::New),
            (2, 3, Direction::NegX, Function::New),
            (3, 0, Direction::NegY, Function::Demolished),
        ];
        for (i, (a, b, direction, function)) in walls.into_iter().enumerate() {
            let n = u8::try_from(i).unwrap_or(0);
            doc.walls.insert(
                uid(2, n),
                Wall {
                    label: labels.allocate("W").unwrap_or_else(|_| unreachable!()),
                    storey,
                    node_a: uid(1, a),
                    node_b: uid(1, b),
                    direction,
                    layer_set: uid(5, 0),
                    left_extent: user(2 * IN),
                    base_offset: user(0),
                    top: WallTop::Storey,
                    join_priority: 0,
                    function,
                },
            );
        }
        let fixed = |reference, axis, value| Constraint {
            kind: ConstraintKind::Fixed {
                reference,
                axis,
                value: len(value),
            },
            status: ConstraintStatus::Driving,
            source: Source::UserStated,
            provenance: None,
        };
        let dist = |a, b, axis, value| Constraint {
            kind: ConstraintKind::Distance {
                a,
                b,
                axis,
                value: len(value),
                tolerance: None,
            },
            status: ConstraintStatus::Driving,
            source: Source::UserStated,
            provenance: None,
        };
        let node = |n| Reference::Node { node: uid(1, n) };
        doc.constraints
            .insert(uid(3, 1), fixed(node(0), Axis::X, 0));
        doc.constraints
            .insert(uid(3, 2), fixed(node(0), Axis::Y, 0));
        doc.constraints
            .insert(uid(3, 3), dist(node(0), node(1), Axis::X, 240 * IN));
        doc.constraints
            .insert(uid(3, 4), dist(node(1), node(2), Axis::Y, 144 * IN));
        doc.openings.insert(
            uid(10, 0),
            Opening {
                kind: OpeningKind::Door,
                host: uid(2, 0),
                reference: node(0),
                anchor: OpeningEdge::Start,
                offset: user(48 * IN),
                width: user(36 * IN),
                height: user(80 * IN),
                sill: user(0),
                start: len(0),
            },
        );
        let space = uid(11, 0);
        doc.spaces.insert(
            space,
            Space {
                storey,
                name: "Shop".into(),
                seed: Point {
                    x: len(120 * IN),
                    y: len(72 * IN),
                },
                boundary: vec![],
                finishes: Finishes {
                    floor: None,
                    walls: None,
                    ceiling: None,
                },
            },
        );
        doc.slabs.insert(
            uid(12, 0),
            Slab {
                storey,
                boundary: Boundary::FromSpace { space },
                thickness: user(4 * IN),
                elevation: user(0),
            },
        );
        let bench_type = uid(13, 0);
        doc.element_types.insert(
            bench_type,
            proxy_box(
                "Bench",
                len(48 * IN),
                len(24 * IN),
                len(30 * IN),
                Authority::UserStatedExact,
            ),
        );
        let mut eq = LabelAllocator::new();
        for (n, prefix, placement, (x, y)) in [
            (
                0u8,
                "EQ",
                Placement::Floor {
                    x: user(120 * IN),
                    y: user(60 * IN),
                    rotation: QuarterTurns::new(0).unwrap_or_else(|| unreachable!()),
                    elevation: user(0),
                },
                (120, 60),
            ),
            (
                1,
                "L",
                Placement::CeilingMounted {
                    x: user(60 * IN),
                    y: user(100 * IN),
                },
                (60, 100),
            ),
        ] {
            doc.occurrences.insert(
                uid(14, n),
                Occurrence {
                    label: eq.allocate(prefix).unwrap_or_else(|_| unreachable!()),
                    storey,
                    element_type: bench_type,
                    placement,
                    clearance: Clearance {
                        front: user(0),
                        back: user(0),
                        left: user(0),
                        right: user(0),
                    },
                    fixed: false,
                    overlap_allowed: false,
                    light: None,
                    center: Point {
                        x: len(x * IN),
                        y: len(y * IN),
                    },
                },
            );
        }
        derive(doc)
            .map(|d| d.document)
            .unwrap_or_else(|e| panic!("derive: {e}"))
    }

    fn written() -> String {
        write_dxf(&plan()).unwrap_or_else(|e| panic!("{e}"))
    }

    fn read(text: &str) -> ::dxf::Drawing {
        ::dxf::Drawing::load(&mut text.as_bytes())
            .unwrap_or_else(|e| panic!("independent reader: {e}"))
    }

    #[test]
    fn golden_plan_dxf() {
        insta::assert_binary_snapshot!("plan.dxf", written().into_bytes());
    }

    /// Done when: an independent DXF reader parses the output.
    #[test]
    fn an_independent_reader_parses_it() {
        let drawing = read(&written());
        assert_eq!(drawing.header.version, ::dxf::enums::AcadVersion::R2000);
        assert_eq!(
            drawing.header.default_drawing_units,
            ::dxf::enums::Units::Inches
        );
        // $LUNITS 4 is architectural (11.1); this reader names the value 4
        // ArchitecturalStacked because it shares the enum with $DIMLUNIT.
        assert_eq!(drawing.header.unit_format as i32, 4);
        assert_eq!(drawing.header.unit_precision, 4);
        let layers: BTreeSet<String> = drawing.layers().map(|l| l.name.clone()).collect();
        for name in [
            "0",
            "WALLS",
            "WALLS-EXISTING",
            "WALLS-DEMOLISHED",
            "OPENINGS",
            "SLABS",
            "EQUIPMENT",
            "EQUIPMENT-LABELS",
            "DIMENSIONS",
            "DATUMS",
        ] {
            assert!(layers.contains(name), "missing layer {name}: {layers:?}");
        }
        let on = |layer: &str| {
            drawing
                .entities()
                .filter(|e| e.common.layer == layer)
                .count()
        };
        assert_eq!(
            (on("WALLS"), on("WALLS-EXISTING"), on("WALLS-DEMOLISHED")),
            (2, 1, 1)
        );
        assert_eq!(
            (
                on("OPENINGS"),
                on("SLABS"),
                on("EQUIPMENT"),
                on("EQUIPMENT-LABELS")
            ),
            (1, 1, 2, 2)
        );
        let texts: BTreeSet<String> = drawing
            .entities()
            .filter_map(|e| match &e.specific {
                ::dxf::entities::EntityType::Text(t) => Some(t.value.clone()),
                _ => None,
            })
            .collect();
        for t in ["EQ-001", "L-001", "20'-0\"", "12'-0\""] {
            assert!(texts.contains(t), "missing text {t}: {texts:?}");
        }
        // Two dimensions, each a dimension line and two extension lines.
        assert_eq!(on("DIMENSIONS"), 2 * 4);
    }

    /// Done when: coordinates round-trip to the nanometer (within six decimals, Q-53).
    #[test]
    fn wall_vertices_round_trip_to_the_nanometer() {
        let doc = plan();
        let drawing = read(&write_dxf(&doc).unwrap_or_else(|e| panic!("{e}")));
        let read_back: BTreeSet<(i64, i64)> = drawing
            .entities()
            .filter_map(|e| match &e.specific {
                ::dxf::entities::EntityType::LwPolyline(p)
                    if e.common.layer.starts_with("WALLS") =>
                {
                    Some(p.vertices.clone())
                }
                _ => None,
            })
            .flatten()
            .map(|v| {
                (
                    (v.x * 25_400_000.0).round() as i64,
                    (v.y * 25_400_000.0).round() as i64,
                )
            })
            .collect();
        let expected: BTreeSet<(i64, i64)> = wall_geometry(&doc)
            .unwrap_or_else(|e| panic!("{e}"))
            .values()
            .flat_map(|g| {
                g.footprint
                    .iter()
                    .map(|p| (p.x.nanometers(), p.y.nanometers()))
            })
            .collect();
        assert_eq!(read_back, expected);
    }

    /// The objects of a DXF text: (type, group pairs), in file order.
    fn objects(text: &str) -> Vec<(String, Vec<(i32, String)>)> {
        let lines: Vec<&str> = text.split("\r\n").collect();
        let mut out: Vec<(String, Vec<(i32, String)>)> = Vec::new();
        for pair in lines.chunks(2) {
            let [code, value] = pair else { continue };
            let code: i32 = code.trim().parse().unwrap_or(-1);
            if code == 0 {
                out.push((value.to_string(), Vec::new()));
            } else if let Some(last) = out.last_mut() {
                last.1.push((code, value.to_string()));
            }
        }
        out
    }

    fn value(pairs: &[(i32, String)], code: i32) -> Option<String> {
        pairs
            .iter()
            .find(|(c, _)| *c == code)
            .map(|(_, v)| v.clone())
    }

    /// G-3, first attempt: AutoCAD 2018 refused the file with "Error in
    /// LAYER Table. Did not receive PlotStyleName". Every R2000 layer must
    /// name the Normal plot style placeholder through group 390.
    #[test]
    fn every_layer_names_the_normal_plot_style() {
        let objects = objects(&written());
        let handle_of = |kind: &str| -> Vec<String> {
            objects
                .iter()
                .filter(|(t, _)| t == kind)
                .filter_map(|(_, p)| value(p, 5))
                .collect()
        };
        let placeholders = handle_of("ACDBPLACEHOLDER");
        assert_eq!(placeholders.len(), 1);
        let layers: Vec<&Vec<(i32, String)>> = objects
            .iter()
            .filter(|(t, _)| t == "LAYER")
            .map(|(_, p)| p)
            .collect();
        assert_eq!(layers.len(), 10);
        assert!(
            layers
                .iter()
                .any(|l| value(l, 2).as_deref() == Some("DATUMS")),
            "{layers:?}"
        );
        for layer in layers {
            assert_eq!(
                value(layer, 390),
                Some(placeholders[0].clone()),
                "{layer:?}"
            );
        }
        // The placeholder is the default, named Normal, of ACAD_PLOTSTYLENAME.
        let with_default = objects
            .iter()
            .find(|(t, _)| t == "ACDBDICTIONARYWDFLT")
            .map(|(_, p)| p.clone())
            .unwrap_or_default();
        assert_eq!(value(&with_default, 3), Some("Normal".to_string()));
        assert_eq!(value(&with_default, 350), Some(placeholders[0].clone()));
        assert_eq!(value(&with_default, 340), Some(placeholders[0].clone()));
        let root = objects
            .iter()
            .find(|(t, p)| t == "DICTIONARY" && value(p, 330).as_deref() == Some("0"))
            .map(|(_, p)| p.clone())
            .unwrap_or_default();
        let named: Vec<(String, String)> = root
            .iter()
            .zip(root.iter().skip(1))
            .filter(|(a, b)| a.0 == 3 && b.0 == 350)
            .map(|(a, b)| (a.1.clone(), b.1.clone()))
            .collect();
        assert!(
            named.contains(&(
                "ACAD_PLOTSTYLENAME".to_string(),
                value(&with_default, 5).unwrap_or_default()
            )),
            "{named:?}"
        );
    }

    /// The POINTs on DATUMS by the TEXT label written at each, in
    /// nanometers read back from the decimal inches; panics on a POINT with
    /// no label or a label on no POINT.
    fn datums(text: &str) -> BTreeMap<String, (i64, i64)> {
        let at = |p: &[(i32, String)]| {
            let c = |code| value(p, code).and_then(|v| parse_inches(&v));
            (c(10).unwrap_or(i64::MIN), c(20).unwrap_or(i64::MIN))
        };
        let on_datums = |kind: &str| -> Vec<Vec<(i32, String)>> {
            objects(text)
                .into_iter()
                .filter(|(t, p)| t == kind && value(p, 8).as_deref() == Some("DATUMS"))
                .map(|(_, p)| p)
                .collect()
        };
        let points: BTreeSet<(i64, i64)> = on_datums("POINT").iter().map(|p| at(p)).collect();
        let labels: BTreeMap<String, (i64, i64)> = on_datums("TEXT")
            .iter()
            .map(|p| (value(p, 1).unwrap_or_default(), at(p)))
            .collect();
        let labelled: BTreeSet<(i64, i64)> = labels.values().copied().collect();
        assert_eq!(points, labelled, "every POINT and only POINTs are labeled");
        labels
    }

    /// T-191 Done when: the L-room DXF read back has F-A to F-D at (0, 0),
    /// (240, 0), (240, 96) and (0, 180) in inches, exact to the nanometer,
    /// each labeled with its name (S-3, spec 5.4).
    #[test]
    fn l_room_fiducials_read_back_from_the_dxf() {
        let doc = derive(crate::gltf::tests::l_room(true))
            .map(|d| d.document)
            .unwrap_or_else(|e| panic!("derive: {e}"));
        let text = write_dxf(&doc).unwrap_or_else(|e| panic!("{e}"));
        let expected: BTreeMap<String, (i64, i64)> = [
            ("F-A", (0, 0)),
            ("F-B", (240 * IN, 0)),
            ("F-C", (240 * IN, 96 * IN)),
            ("F-D", (0, 180 * IN)),
        ]
        .into_iter()
        .map(|(n, p)| (n.to_string(), p))
        .collect();
        assert_eq!(datums(&text), expected);
        // An independent reader sees four POINTs on DATUMS.
        let drawing = read(&text);
        let points = drawing
            .entities()
            .filter(|e| e.common.layer == "DATUMS")
            .filter(|e| matches!(e.specific, ::dxf::entities::EntityType::ModelPoint(_)))
            .count();
        assert_eq!(points, 4);
    }

    #[test]
    fn numbers_are_written_with_eight_decimals() {
        assert_eq!(inches(0), "0.00000000");
        assert_eq!(inches(25_400_000), "1.00000000");
        assert_eq!(inches(-12_700_000), "-0.50000000");
        assert_eq!(inches(1), "0.00000004");
        assert_eq!(inches(1_587_500), "0.06250000");
        assert_eq!(parse_inches("-0.50000000"), Some(-12_700_000));
    }

    proptest! {
        /// Every stored nanometer reads back exactly.
        #[test]
        fn inches_round_trip(nm in -(1i64 << 50)..(1i64 << 50)) {
            prop_assert_eq!(parse_inches(&inches(nm)), Some(nm));
        }
    }
}
