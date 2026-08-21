//! State to colour.
//!
//! Section 5 of the design doc fixes three colours and no fourth: gray for
//! unpacted, yellow for pacted-and-stale, green for pacted-and-fresh. The
//! mapping lives on its own here, as one total `match`, so adding a state to
//! the engine breaks this file at compile time rather than quietly drawing the
//! new state in whatever colour a fallback arm happened to pick.

use ratatui::style::Color;
use warlock_engine::NodeState;

/// The colour a node in `state` is drawn in.
///
/// Pure: it takes no terminal and no app state, so it is callable and testable
/// with nothing attached to stdout.
///
/// ```
/// use ratatui::style::Color;
/// use warlock_engine::NodeState;
/// use warlock_tui::colour_for;
///
/// assert_eq!(colour_for(NodeState::PactedFresh), Color::Green);
/// ```
#[must_use]
pub const fn colour_for(state: NodeState) -> Color {
    match state {
        NodeState::Unpacted => Color::Gray,
        NodeState::PactedStale => Color::Yellow,
        NodeState::PactedFresh => Color::Green,
    }
}

/// The colour the focused pane's border is drawn in.
///
/// Cyan, and cyan because it is none of the three above. Colour on this screen
/// means node state; a border that lit up in green would say a fourth thing in
/// the vocabulary the other three are spoken in, and a reader would have to
/// learn that green means fresh except where it means focused. Which pane the
/// keys are driving is not something a node can be, so it is said in a colour
/// nothing else uses — and said with a modifier as well as a colour, so it also
/// reads on a terminal that shows none of them.
///
/// Kept here, beside [`colour_for`], rather than next to the drawing code, so
/// that the test that no two things on this screen share a colour has both in
/// front of it.
pub(crate) const FOCUS_COLOUR: Color = Color::Cyan;

#[cfg(test)]
mod tests {
    use ratatui::style::Color;
    use warlock_engine::NodeState;

    use super::{FOCUS_COLOUR, colour_for};

    #[test]
    fn unpacted_is_gray() {
        assert_eq!(colour_for(NodeState::Unpacted), Color::Gray);
    }

    #[test]
    fn pacted_stale_is_yellow() {
        assert_eq!(colour_for(NodeState::PactedStale), Color::Yellow);
    }

    #[test]
    fn pacted_fresh_is_green() {
        assert_eq!(colour_for(NodeState::PactedFresh), Color::Green);
    }

    #[test]
    fn the_focus_colour_is_no_states_colour() {
        for state in NodeState::ALL {
            assert_ne!(
                FOCUS_COLOUR,
                colour_for(state),
                "the focused border shares {state:?}'s colour"
            );
        }
    }

    #[test]
    fn no_two_states_share_a_colour() {
        for state in NodeState::ALL {
            for other in NodeState::ALL {
                assert_eq!(
                    state == other,
                    colour_for(state) == colour_for(other),
                    "{state:?} and {other:?} are drawn the same"
                );
            }
        }
    }
}
