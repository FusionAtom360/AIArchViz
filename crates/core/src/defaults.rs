//! The defaults catalog (spec 6.4; Q-21, Q-42). Every value is applied with
//! source Proposed, so the user confirms it.
//!
//! Version 1: opening sizes and the minimum jamb distance. Version 2 adds
//! the interior stud wall, storey heights, slab and ceiling thickness, eye
//! height, clearance, and roof thickness, fascia and overhang.

use crate::document::{Layer, OpeningKind, Source, Sourced};
use crate::identity::Uuid;
use crate::length::Length;

/// The catalog version; bump it whenever a default changes.
pub const CATALOG_VERSION: u32 = 2;

const IN: i64 = 25_400_000;

/// Minimum distance from an opening to the ends of its host's
/// full-thickness stretch (7.7): 4 in.
pub const MIN_JAMB_DISTANCE: Length = Length::from_nanometers(4 * IN);

/// A wall's base above the storey elevation: 0".
pub const WALL_BASE_OFFSET: Length = Length::from_nanometers(0);
/// Storey floor-to-floor height: 10'-0".
pub const FLOOR_TO_FLOOR: Length = Length::from_nanometers(120 * IN);
/// Storey default ceiling height: 9'-0".
pub const CEILING_HEIGHT: Length = Length::from_nanometers(108 * IN);
/// Slab thickness: 4".
pub const SLAB_THICKNESS: Length = Length::from_nanometers(4 * IN);
/// Ceiling thickness: 1/2".
pub const CEILING_THICKNESS: Length = Length::from_nanometers(IN / 2);
/// Camera eye height (12.6): 5'-4".
pub const EYE_HEIGHT: Length = Length::from_nanometers(64 * IN);
/// Clearance zone extent on every side: 0".
pub const CLEARANCE: Length = Length::from_nanometers(0);
/// Shed roof thickness: 8".
pub const ROOF_THICKNESS: Length = Length::from_nanometers(8 * IN);
/// Shed roof fascia depth: 6".
pub const FASCIA_DEPTH: Length = Length::from_nanometers(6 * IN);
/// Shed roof eave overhang on every side: 12".
pub const EAVE_OVERHANG: Length = Length::from_nanometers(12 * IN);

/// Tolerance of a Measured constraint that states none (7.5): 1/4 in.
pub const MEASURED_TOLERANCE: Length = Length::from_nanometers(IN / 4);

/// `value` as a catalog default: source Proposed.
pub const fn proposed(value: Length) -> Sourced<Length> {
    Sourced {
        value,
        source: Source::Proposed,
    }
}

/// The interior stud wall, left to right: 1/2" gypsum, 3 1/2" stud (core),
/// 1/2" gypsum, with the reference line on the stud center (left extent
/// 2 1/4"). The materials are the caller's.
pub fn interior_wall(gypsum: Uuid, stud: Uuid) -> (Vec<Layer>, Sourced<Length>) {
    let layer = |material, thickness, core| Layer {
        material,
        thickness: proposed(Length::from_nanometers(thickness)),
        core,
    };
    (
        vec![
            layer(gypsum, IN / 2, false),
            layer(stud, 3 * IN + IN / 2, true),
            layer(gypsum, IN / 2, false),
        ],
        proposed(Length::from_nanometers(IN / 2 + (3 * IN + IN / 2) / 2)),
    )
}

/// Default size of an opening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpeningSize {
    pub width: Length,
    pub height: Length,
    /// Above the storey elevation (Q-22).
    pub sill: Length,
}

/// The default size for an opening of `kind`.
pub const fn opening_size(kind: OpeningKind) -> OpeningSize {
    let (width, height, sill) = match kind {
        // 3'-0" x 6'-8", on the floor.
        OpeningKind::Door | OpeningKind::Opening => (36 * IN, 80 * IN, 0),
        // 3'-0" x 4'-0", sill 3'-0".
        OpeningKind::Window => (36 * IN, 48 * IN, 36 * IN),
    };
    OpeningSize {
        width: Length::from_nanometers(width),
        height: Length::from_nanometers(height),
        sill: Length::from_nanometers(sill),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::format_length;

    fn text(s: OpeningSize) -> [String; 3] {
        [
            format_length(s.width),
            format_length(s.height),
            format_length(s.sill),
        ]
    }

    #[test]
    fn catalog_v1_values() {
        assert_eq!(CATALOG_VERSION, 2);
        assert_eq!(
            text(opening_size(OpeningKind::Door)),
            ["3'-0\"", "6'-8\"", "0'-0\""]
        );
        assert_eq!(
            text(opening_size(OpeningKind::Window)),
            ["3'-0\"", "4'-0\"", "3'-0\""]
        );
        assert_eq!(
            text(opening_size(OpeningKind::Opening)),
            ["3'-0\"", "6'-8\"", "0'-0\""]
        );
        assert_eq!(format_length(MIN_JAMB_DISTANCE), "0'-4\"");
    }

    #[test]
    fn catalog_v2_values() {
        let shown = [
            WALL_BASE_OFFSET,
            FLOOR_TO_FLOOR,
            CEILING_HEIGHT,
            SLAB_THICKNESS,
            CEILING_THICKNESS,
            EYE_HEIGHT,
            CLEARANCE,
            ROOF_THICKNESS,
            FASCIA_DEPTH,
            EAVE_OVERHANG,
        ]
        .map(format_length);
        assert_eq!(
            shown,
            [
                "0'-0\"",
                "10'-0\"",
                "9'-0\"",
                "0'-4\"",
                "0'-0 1/2\"",
                "5'-4\"",
                "0'-0\"",
                "0'-8\"",
                "0'-6\"",
                "1'-0\""
            ]
        );
        let (layers, left) = interior_wall(Uuid::from_bytes([1; 16]), Uuid::from_bytes([2; 16]));
        let t: Vec<String> = layers
            .iter()
            .map(|l| format_length(l.thickness.value))
            .collect();
        assert_eq!(t, ["0'-0 1/2\"", "0'-3 1/2\"", "0'-0 1/2\""]);
        assert_eq!(
            layers.iter().map(|l| l.core).collect::<Vec<_>>(),
            [false, true, false]
        );
        assert!(
            layers
                .iter()
                .all(|l| l.thickness.source == Source::Proposed)
        );
        assert_eq!(
            (format_length(left.value), left.source),
            ("0'-2 1/4\"".to_string(), Source::Proposed)
        );
    }
}
