//! Indexed colours rather than named ones, all four of them. A named colour
//! sits in the reader's theme and an indexed one overrides it; the override is
//! taken because the whole message on this screen is carried by colour — stale
//! has to be tellable from fresh from unpacted at a glance — and because a mix
//! of pinned and theme-dependent values would leave the pairwise-distinctness
//! tests below asserting over values the user could change.

use ratatui::style::Color;
use warlock_engine::NodeState;

/// ```
/// use ratatui::style::Color;
/// use warlock_engine::NodeState;
/// use warlock_tui::colour_for;
///
/// assert_eq!(colour_for(NodeState::PactedFresh), Color::Indexed(71));
/// ```
#[must_use]
pub const fn colour_for(state: NodeState) -> Color {
    // No catch-all arm: a fourth state added to the engine has to break this
    // file at compile time rather than be drawn in whatever colour a fallback
    // picked. Stale is 214, which leans orange, because true yellow is the one
    // of the three that dies on a light background and stale is the state the
    // product asks people to act on.
    match state {
        NodeState::Unpacted => Color::Indexed(245),
        NodeState::PactedStale => Color::Indexed(214),
        NodeState::PactedFresh => Color::Indexed(71),
    }
}

// Cyan because it is none of the three above: colour on this screen means node
// state, and which pane the keys are driving is not something a node can be. A
// theme's cyan could drift towards any of the three, so this is pinned for the
// same reason they are.
pub(crate) const FOCUS_COLOUR: Color = Color::Indexed(45);

// `DIM` is the obvious way to ask for a quieter line and is not taken: it is
// honoured inconsistently, so a guide would vanish on one terminal and shout on
// another. 240 sits a few steps below unpacted 245 — far enough down to recede
// under every state colour, not so far as to disappear on a light background.
pub(crate) const GUIDE_COLOUR: Color = Color::Indexed(240);

#[cfg(test)]
mod tests {
    use ratatui::style::Color;
    use warlock_engine::NodeState;

    use super::{FOCUS_COLOUR, GUIDE_COLOUR, colour_for};

    #[test]
    fn unpacted_is_gray() {
        assert_eq!(colour_for(NodeState::Unpacted), Color::Indexed(245));
    }

    #[test]
    fn pacted_stale_is_yellow() {
        assert_eq!(colour_for(NodeState::PactedStale), Color::Indexed(214));
    }

    #[test]
    fn pacted_fresh_is_green() {
        assert_eq!(colour_for(NodeState::PactedFresh), Color::Indexed(71));
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
    fn the_guide_colour_is_a_flat_indexed_gray() {
        assert_eq!(GUIDE_COLOUR, Color::Indexed(240));
    }

    #[test]
    fn the_guide_colour_is_no_states_colour() {
        for state in NodeState::ALL {
            assert_ne!(
                GUIDE_COLOUR,
                colour_for(state),
                "the indent guides share {state:?}'s colour"
            );
        }
    }

    #[test]
    fn the_guide_colour_is_not_the_focus_colour() {
        assert_ne!(
            GUIDE_COLOUR, FOCUS_COLOUR,
            "the indent guides share the focused border's colour"
        );
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
