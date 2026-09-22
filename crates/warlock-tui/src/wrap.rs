//! Wrapping rather than a horizontal scroll, which would be a mode, a key pair
//! and an offset so a reader could shunt a sentence left and right to read it.
//! What that costs is worth saying out loud: one thing that happened is no
//! longer always one row, so the count of rows on screen is no longer the count
//! of things the pass was seen doing. The clock in front of each of them is what
//! now says where one thing ends and the next begins — a row with no clock on it
//! is the row above it, still going.
//!
//! Two wrappers, and the difference is not an oversight. `wrapped` breaks at
//! words and is what the panel is drawn from; `folded` fills each row to the
//! width and keeps every byte, because a row of a field is cells a caret sits
//! in. Reading wants the word whole; typing wants the row full.

use crate::account::Line;
use crate::ui::{NOTE_MARKER, PANEL_INDENT, SAID_MARKER, display_width};

// The one description of a row's shape, read by the two halves that have to
// agree about it: `rows` wraps at the width the prefix leaves, and `panel_row`
// draws the prefix that width was measured against. Two answers would be a line
// broken at one width and drawn at another, which is a row over the border.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shape {
    pub(crate) prefix: String,
    pub(crate) text: String,
    pub(crate) heading: bool,
}

#[must_use]
pub(crate) fn shape(line: &Line) -> Shape {
    match line {
        Line::Directory { path } => Shape {
            prefix: String::new(),
            text: path.display().to_string(),
            heading: true,
        },
        Line::Clocked { clock, text } => Shape {
            prefix: format!("{PANEL_INDENT}{clock} "),
            text: text.clone(),
            heading: false,
        },
        Line::Summary { text } => Shape {
            prefix: String::new(),
            text: text.clone(),
            heading: true,
        },
        Line::Text { text } => Shape {
            prefix: String::new(),
            text: text.clone(),
            heading: false,
        },
        Line::Said { text } => Shape {
            prefix: SAID_MARKER.to_owned(),
            text: text.clone(),
            heading: true,
        },
        // Warlock's own voice: its own marker, and plain, so it is neither the
        // question above it nor a work line under one.
        Line::Note { text } => Shape {
            prefix: NOTE_MARKER.to_owned(),
            text: text.clone(),
            heading: false,
        },
        Line::Wrapped { text, heading, .. } => Shape {
            prefix: String::new(),
            text: text.clone(),
            heading: *heading,
        },
    }
}

#[must_use]
pub(crate) fn rows(line: &Line, width: usize) -> Vec<Line> {
    let shape = shape(line);
    let indent = display_width(&shape.prefix);
    let pieces = wrapped(&shape.text, width.saturating_sub(indent));
    if pieces.len() < 2 {
        return vec![line.clone()];
    }

    let blanks = " ".repeat(indent);
    let mut rows = Vec::with_capacity(pieces.len());
    let mut pieces = pieces.into_iter();
    let first = pieces.next().unwrap_or_default();
    rows.push(continued(line, &first, &shape));
    let voice = line.voice();
    rows.extend(pieces.map(|piece| Line::Wrapped {
        text: format!("{blanks}{piece}"),
        heading: shape.heading,
        voice,
    }));
    rows
}

fn continued(line: &Line, first: &str, shape: &Shape) -> Line {
    match line {
        Line::Clocked { clock, .. } => Line::Clocked {
            clock: clock.clone(),
            text: first.to_owned(),
        },
        Line::Said { .. } => Line::Said {
            text: first.to_owned(),
        },
        Line::Note { .. } => Line::Note {
            text: first.to_owned(),
        },
        Line::Summary { .. } => Line::Summary {
            text: first.to_owned(),
        },
        Line::Text { .. } => Line::Text {
            text: first.to_owned(),
        },
        // A path fragment is not a path, and a row already composed has no
        // variant of its own to go back to.
        Line::Directory { .. } | Line::Wrapped { .. } => Line::Wrapped {
            text: first.to_owned(),
            heading: shape.heading,
            voice: line.voice(),
        },
    }
}

// Never empty: a line with nothing on it is one blank row, because a blank line
// in a file is a paragraph break and swallowing it would re-flow the document
// into one long block. A `width` of zero is a panel nobody has measured yet and
// is not a width to break at — the frame that measures the panel is the one that
// wraps it.
pub(crate) fn wrapped(text: &str, width: usize) -> Vec<String> {
    wrapped_at(text, width)
        .into_iter()
        .map(|(_, row)| row.to_owned())
        .collect()
}

// The same rows [`wrapped`] gives, each with where it starts in `text`. The
// trimmed space at a break belongs to no row, so the rows do not concatenate
// back byte for byte the way `folded`'s do and an offset cannot be counted up
// from their lengths — it has to be carried through the trim, which is what this
// is for. Keep the two as one function: a second wrapper beside this one is two
// answers about where a line breaks, and the offsets would be counted against
// rows nobody drew.
pub(crate) fn wrapped_at(text: &str, width: usize) -> Vec<(usize, &str)> {
    if width == 0 || display_width(text) <= width {
        return vec![(0, text)];
    }

    let mut rows = Vec::new();
    let mut start = 0;
    let mut rest = text;
    // Counted down by what each row consumes rather than re-measured from the
    // remainder each turn. `display_width` walks every character it is given,
    // so asking it about the whole rest per row made wrapping one line
    // quadratic in its length; measuring only the bytes just consumed sums to
    // one walk of the line. It stays exact because the width of a string is the
    // sum of the widths of its pieces.
    let mut remaining = display_width(text);
    while remaining > width {
        let end = break_at(rest, width);
        rows.push((start, rest[..end].trim_end()));
        // The space the break was made at goes with the row above it. A break
        // made mid-word has no space to eat, so this takes nothing off the text.
        let after = &rest[end..];
        let trimmed = after.trim_start();
        let consumed = end + (after.len() - trimmed.len());
        remaining -= display_width(&rest[..consumed]);
        rest = trimmed;
        start += consumed;
    }

    // Only when something is left: a line that came out even has nothing for a
    // last row, and pushing one anyway would be a blank row — a paragraph break
    // the file does not have.
    if !rest.is_empty() {
        rows.push((start, rest));
    }
    rows
}

// Keeps every byte: the cut is the width itself, wherever in a word that lands,
// and nothing is trimmed off either side of it, so the rows concatenate back to
// `text` exactly. That is what a caller counting an offset through them needs —
// a byte that vanished at a word break would be a cell the cursor cannot aim at.
//
// Breaking at words instead would leave the end of a row standing for no offset
// at all: `test` alone on a row with the whole width beside it belonging to
// nothing, and one more character re-flowing the draft from the top and taking
// the cursor with it.
pub(crate) fn folded(text: &str, width: usize) -> Vec<String> {
    if width == 0 || display_width(text) <= width {
        return vec![text.to_owned()];
    }

    let mut rows = Vec::new();
    let mut rest = text;
    // Counted down rather than re-measured, for the reason `wrapped_at` gives:
    // the composer folds the whole draft on every frame and on every cursor
    // key, and re-measuring the remainder per row made that quadratic in the
    // draft's length.
    let mut remaining = display_width(text);
    while remaining > width {
        let end = filled(rest, width);
        remaining -= display_width(&rest[..end]);
        rows.push(rest[..end].to_owned());
        rest = &rest[end..];
    }

    // Only when something is left: a text that came out even has nothing for a
    // last row, and pushing one anyway would be a row the text does not have.
    if !rest.is_empty() {
        rows.push(rest.to_owned());
    }
    rows
}

// Never zero for a text with anything in it, which is where `folded`'s loop gets
// its promise of making progress: it has no `trim_start` to fall back on, so the
// cut alone has to advance it. A character wider than the whole field overhangs
// its row rather than stopping the fold.
fn filled(text: &str, width: usize) -> usize {
    fits_in(text, width).max(first_character(text))
}

// Where a display column lands in the bytes, which the panel's truncation, the
// composer's fold and the footer's clipping all have to answer the same way: a
// second idea of how many characters fit in a field is a highlight a column off
// the text it claims to cover. Stops *before* a character that would overhang,
// so it never reports a width larger than it was given.
pub(crate) fn fits_in(text: &str, width: usize) -> usize {
    let mut taken = 0;
    let mut fits = 0;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        let columns = display_width(&text[index..next]);
        if taken + columns > width {
            break;
        }
        taken += columns;
        fits = next;
    }

    fits
}

// A space is only a candidate once a word has been seen, so a line's own
// indentation is never what it breaks at: an indented line breaking at its
// indent would put an empty row above itself and say nothing.
//
// Always past the first character, whatever the width. A cut at zero would be a
// row with nothing on it and a loop that never reached the end of the line, so a
// character wider than the whole panel overhangs rather than stopping the wrap.
fn break_at(text: &str, width: usize) -> usize {
    let mut taken = 0;
    let mut fits = 0;
    let mut word = false;
    let mut space = None;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        let columns = display_width(&text[index..next]);
        if taken + columns > width {
            break;
        }
        taken += columns;
        fits = next;
        if character.is_whitespace() {
            // Only a space with a word in front of it: see above.
            if word {
                space = Some(index);
            }
        } else {
            word = true;
        }
    }

    // The prefix that fits is whole words when what follows it is a space, and
    // backing up to the space before that would give a row up a word for
    // nothing.
    if fits > 0 && text[fits..].starts_with(char::is_whitespace) {
        return fits;
    }

    space.unwrap_or_else(|| fits.max(first_character(text)))
}

fn first_character(text: &str) -> usize {
    text.chars().next().map_or(0, char::len_utf8)
}

#[cfg(test)]
#[path = "tests/wrap.rs"]
mod tests;
