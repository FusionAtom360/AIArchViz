//! The status line (T-204b, Q-81 (a)): the one place the app shows its
//! messages, in the bottom bar. The current mode's instruction comes first,
//! then the last message (a commit, a discard, a refusal or an error), then
//! the pointer coordinates.

/// Separates the three parts of the status line.
pub const SEPARATOR: &str = "   |   ";

/// The status line: the parts that are not empty, in the order instruction,
/// message, coordinates.
pub fn line(instruction: &str, message: &str, coordinates: &str) -> String {
    [instruction, message, coordinates]
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(SEPARATOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruction_then_message_then_coordinates() {
        assert_eq!(
            line(
                "pick the first reference",
                "Walls: discarded.",
                "x 0'-1\"  y 0'-2\""
            ),
            "pick the first reference   |   Walls: discarded.   |   x 0'-1\"  y 0'-2\""
        );
    }

    #[test]
    fn empty_parts_are_left_out() {
        assert_eq!(line("Select", "", "x 0'-0\""), "Select   |   x 0'-0\"");
        assert_eq!(line("Select", " ", ""), "Select");
        assert_eq!(line("", "", ""), "");
    }
}
