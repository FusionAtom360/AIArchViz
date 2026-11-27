//! The defaults catalog (spec 6.4; Q-21). Every value is applied with
//! source Proposed, so the user confirms it.

use crate::document::OpeningKind;
use crate::length::Length;

/// The catalog version; bump it whenever a default changes.
pub const CATALOG_VERSION: u32 = 1;

const IN: i64 = 25_400_000;

/// Minimum distance from an opening to the ends of its host's
/// full-thickness stretch (7.7): 4 in.
pub const MIN_JAMB_DISTANCE: Length = Length::from_nanometers(4 * IN);

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
        assert_eq!(CATALOG_VERSION, 1);
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
}
