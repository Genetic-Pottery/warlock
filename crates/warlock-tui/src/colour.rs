//! State to colour.
//!
//! Section 5 of the design doc fixes three colours and no fourth: gray for
//! unpacted, yellow for pacted-and-stale, green for pacted-and-fresh. The
//! mapping lives on its own here, as one total `match`, so adding a state to
//! the engine breaks this file at compile time rather than quietly drawing the
//! new state in whatever colour a fallback arm happened to pick.
//!
//! The colours are given as 256-colour indices rather than as named ANSI
//! colours, and that is a trade. A named colour respects the user's palette:
//! `Color::Green` is whatever green they chose, so it sits in their theme and
//! warlock has no control over what actually lands on the screen. An indexed
//! colour is the other way round — warlock picks the value and the theme is
//! ignored, so it can look foreign next to the rest of the terminal. Ceding
//! control is the right default and it is not the default taken for these four,
//! because on this screen the whole message is carried by colour: the reader has
//! to tell stale from fresh from unpacted at a glance, and that mutual
//! distinguishability is the product. So these four, and only these four, are
//! pinned.
//!
//! Stale is 214, which leans orange, on purpose. Stale is the state the product
//! asks people to act on and true yellow is the one of the three that dies on a
//! light background; an orange-leaning yellow survives on both. And all four
//! values are indexed together, not just the awkward one, because a mix of
//! pinned and theme-dependent colours would leave the pairwise-distinctness
//! tests below asserting over values half of which the user could change. With
//! all four pinned the tests mean what they say.

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
/// assert_eq!(colour_for(NodeState::PactedFresh), Color::Indexed(71));
/// ```
#[must_use]
pub const fn colour_for(state: NodeState) -> Color {
    match state {
        NodeState::Unpacted => Color::Indexed(245),
        NodeState::PactedStale => Color::Indexed(214),
        NodeState::PactedFresh => Color::Indexed(71),
    }
}

/// The colour the focused pane's border is drawn in.
///
/// A bright cyan (45), and cyan because it is none of the three above. Colour
/// on this screen means node state; a border that lit up in green would say a
/// fourth thing in the vocabulary the other three are spoken in, and a reader
/// would have to learn that green means fresh except where it means focused.
/// Which pane the keys are driving is not something a node can be, so it is said
/// in a colour nothing else uses — and said with a modifier as well as a colour,
/// so it also reads on a terminal that shows none of them. Indexed for the same
/// reason the three states are: being unmistakably not one of them is the point,
/// and a theme's cyan could drift towards any of them.
///
/// Kept here, beside [`colour_for`], rather than next to the drawing code, so
/// that the test that no two things on this screen share a colour has both in
/// front of it.
pub(crate) const FOCUS_COLOUR: Color = Color::Indexed(45);

/// The colour the tree's vertical indent guides are drawn in.
///
/// A flat 256-colour index (240) and no modifier. `DIM` is the obvious way to
/// ask for a quieter line and it is not taken, because it is honoured
/// inconsistently: some terminals render it as a real half-brightness, some
/// ignore it outright, and some map it onto a palette entry of their own. A
/// guide that vanishes on one terminal and shouts on another is worse than one
/// dim colour everywhere, so the dimness is pinned into the value itself and
/// each guide is drawn in exactly what this says, on every terminal.
///
/// 240 against unpacted gray 245 is deliberate, and the small gap is the whole
/// point: the guides are structure, not state, and they sit a few steps below
/// the quietest thing that *is* state so they read as scaffolding behind an
/// unpacted row rather than as another row beside it. Far enough down to recede
/// under all three state colours; not so far as to disappear on a light
/// background. It is none of the four colours above, so the tests below can hold
/// every colour on the screen apart from every other.
// Nothing outside the tests draws a guide yet: the value and the distinctness
// tests land first, the drawing code that reads it lands next, and this `allow`
// goes with it.
#[allow(dead_code)]
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
