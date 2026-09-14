//! Indexed colours rather than named ones, every one of them. A named colour
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
//
// It is no longer worn by the focused border alone — `SYSTEM_COLOUR` below is
// the same value — so focus is told apart by weight and position rather than by
// hue: the border is the only cyan thing that is a border, and it is the only
// cyan thing drawn bold.
pub(crate) const FOCUS_COLOUR: Color = Color::Indexed(45);

// Deliberately the focus colour and not a near neighbour. Everything warlock
// says about its own work — the pacting card's directories and work lines, its
// notes in the conversation, the run header and its bar — is one instrument
// talking, and a second blue a few steps away would read as a second meaning
// rather than as one voice. What it must stay apart from is the three states
// and the model's own words, which the tests below hold it to.
pub(crate) const SYSTEM_COLOUR: Color = FOCUS_COLOUR;

// `DIM` is the obvious way to ask for a quieter line and is not taken: it is
// honoured inconsistently, so a guide would vanish on one terminal and shout on
// another. 240 sits a few steps below unpacted 245 — far enough down to recede
// under every state colour, not so far as to disappear on a light background.
pub(crate) const GUIDE_COLOUR: Color = Color::Indexed(240);

// The model's own words, and nothing else: what the operator typed keeps the
// terminal's default foreground, and the work lines and notes around the answer
// are warlock's voice in `SYSTEM_COLOUR`. A mauve because it belongs to none of
// the three states and is not the cyan those two share; muted so a long
// exchange reads as body text rather than as three screens of alarm. Pinned and
// indexed like the rest, because a named magenta is the reader's magenta and
// could land on any of the values the tests below hold it apart from.
pub(crate) const CONVERSATION_COLOUR: Color = Color::Indexed(139);

#[cfg(test)]
mod tests {
    use ratatui::style::Color;
    use warlock_engine::NodeState;

    use super::{CONVERSATION_COLOUR, FOCUS_COLOUR, GUIDE_COLOUR, SYSTEM_COLOUR, colour_for};

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
    fn the_conversation_colour_is_a_flat_indexed_mauve() {
        assert_eq!(CONVERSATION_COLOUR, Color::Indexed(139));
    }

    #[test]
    fn the_conversation_colour_is_no_states_colour() {
        for state in NodeState::ALL {
            assert_ne!(
                CONVERSATION_COLOUR,
                colour_for(state),
                "the conversation card shares {state:?}'s colour"
            );
        }
    }

    #[test]
    fn the_conversation_colour_is_not_the_focus_colour() {
        assert_ne!(
            CONVERSATION_COLOUR, FOCUS_COLOUR,
            "the conversation card shares the focused border's colour"
        );
    }

    #[test]
    fn the_conversation_colour_is_not_the_guide_colour() {
        assert_ne!(
            CONVERSATION_COLOUR, GUIDE_COLOUR,
            "the conversation card shares the indent guides' colour"
        );
    }

    #[test]
    fn the_system_colour_is_the_focus_colour() {
        assert_eq!(
            SYSTEM_COLOUR, FOCUS_COLOUR,
            "warlock's own voice and the focused border are one colour on purpose",
        );
    }

    #[test]
    fn the_system_colour_is_no_states_colour() {
        for state in NodeState::ALL {
            assert_ne!(
                SYSTEM_COLOUR,
                colour_for(state),
                "warlock's own voice shares {state:?}'s colour"
            );
        }
    }

    #[test]
    fn the_system_colour_is_not_the_model_s_own_words() {
        assert_ne!(
            SYSTEM_COLOUR, CONVERSATION_COLOUR,
            "warlock talking about its work is drawn as the model talking",
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
