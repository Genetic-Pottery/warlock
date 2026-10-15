//! Indexed colours rather than named ones, every one of them. A named colour
//! sits in the reader's theme and an indexed one overrides it; the override is
//! taken because the tree's whole message is carried by colour — scoped has to
//! be tellable from unscoped at a glance — and because a mix of pinned and
//! theme-dependent values would leave the pairwise-distinctness tests below
//! asserting over values the user could change.

use ratatui::style::Color;

/// ```
/// use ratatui::style::Color;
/// use warlock_tui::colour_for;
///
/// assert_eq!(colour_for(true), Color::Indexed(71));
/// assert_eq!(colour_for(false), Color::Indexed(245));
/// ```
#[must_use]
pub const fn colour_for(covered: bool) -> Color {
    if covered {
        Color::Indexed(71)
    } else {
        Color::Indexed(245)
    }
}

// Colour on this screen means whether a scope covers a directory, and which
// pane the keys are driving is not something a node can be, so this has to stay
// apart from the two tree colours above and from `CONVERSATION_COLOUR` below —
// the tests below hold it there. Pinned and indexed for the reason the header
// gives: a theme's own value could drift onto any of them.
//
// `SYSTEM_COLOUR` below is the same value, so focus is told apart by weight and
// position — a bold border, at the edge of the pane — rather than by hue.
pub(crate) const FOCUS_COLOUR: Color = Color::Indexed(42);

// Deliberately the focus colour and not a near neighbour. Everything warlock
// says about its own work in the conversation is one instrument talking, and a
// second blue a few steps away would read as a second meaning rather than as
// one voice. What it must stay apart
// from is the two tree colours and the model's own words, which the tests below
// hold it to.
pub(crate) const SYSTEM_COLOUR: Color = FOCUS_COLOUR;

// `DIM` is the obvious way to ask for a quieter line and is not taken: it is
// honoured inconsistently, so a guide would vanish on one terminal and shout on
// another. 240 sits a few steps below unscoped 245 — far enough down to recede
// under both tree colours, not so far as to disappear on a light background.
pub(crate) const GUIDE_COLOUR: Color = Color::Indexed(240);

// The model's own words, and nothing else: what the operator typed keeps the
// terminal's default foreground, and the work lines and notes around the answer
// are warlock's voice in `SYSTEM_COLOUR`. A mauve because it is neither tree
// colour and is not the cyan those two share; muted so a long
// exchange reads as body text rather than as three screens of alarm. Pinned and
// indexed like the rest, because a named magenta is the reader's magenta and
// could land on any of the values the tests below hold it apart from.
pub(crate) const CONVERSATION_COLOUR: Color = Color::Indexed(139);

#[cfg(test)]
#[path = "tests/colour.rs"]
mod tests;
