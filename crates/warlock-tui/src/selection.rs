//! A selection is positions in the thread's own text, never screen cells: a
//! piece index and a byte offset into that piece. That is what lets the same
//! selection survive a terminal made narrower, a card scrolled, and text
//! arriving under it, none of which change where a character sits in the string
//! it belongs to.
//!
//! Cells and positions meet in two functions and nowhere else — [`position_at`]
//! for the pointer, [`spans_at`] for the highlight — and each row they work on
//! is laid out by the same [`Laid`], so a cell drawn highlighted is a cell that
//! maps back into the selection. What is copied is what the thread stores, not
//! what the panel drew: no marker, no indent, and no break where a line wrapped.

use std::time::Instant;

use crate::account::Line;
use crate::thread::{Sourced, Thread};
use crate::ui::display_width;
use crate::wrap::{Shape, rows, shape, wrapped_at};

/// A byte offset, so the arithmetic is the string's own; [`Selection::new`] is
/// what keeps one on a char boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    piece: usize,
    offset: usize,
}

impl Position {
    #[must_use]
    pub const fn new(piece: usize, offset: usize) -> Self {
        Self { piece, offset }
    }

    #[must_use]
    pub const fn piece(self) -> usize {
        self.piece
    }

    #[must_use]
    pub const fn offset(self) -> usize {
        self.offset
    }
}

/// Start and end in reading order, whichever way the two positions arrived, so
/// a drag upwards and the same drag downwards are one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    start: Position,
    end: Position,
}

impl Selection {
    /// `pieces` is what [`Thread::pieces`](crate::Thread::pieces) gives back.
    /// Both positions are clamped to their piece and floored to a char boundary
    /// here and nowhere else, which is why every other function in this file may
    /// slice with `[..]`.
    #[must_use]
    pub fn new(pieces: &[&str], start: Position, end: Position) -> Self {
        let start = snapped(pieces, start);
        let end = snapped(pieces, end);
        let (start, end) = if end < start {
            (end, start)
        } else {
            (start, end)
        };
        Self { start, end }
    }

    #[must_use]
    pub const fn start(self) -> Position {
        self.start
    }

    #[must_use]
    pub const fn end(self) -> Position {
        self.end
    }
}

/// The thread's exact stored text, one blank line between pieces and nothing
/// else: no marker, no indent, and no break where the panel wrapped. Work rows
/// cannot appear because they are not pieces.
#[must_use]
pub fn copied_text(pieces: &[&str], selection: Selection) -> String {
    let Selection { start, end } = selection;

    pieces
        .iter()
        .enumerate()
        .take(end.piece.saturating_add(1))
        .skip(start.piece)
        .filter_map(|(index, text)| {
            let from = if index == start.piece {
                start.offset
            } else {
                0
            };
            let to = if index == end.piece {
                end.offset
            } else {
                text.len()
            };
            text.get(from..to).filter(|slice| !slice.is_empty())
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// A cell of the thread card, in the panel's own terms: `column` and `row`
/// counted from the first cell of the panel's rows area, which is the pair
/// [`Hit::PanelLine`](crate::Hit::PanelLine) reports, and `scroll` and `width`
/// the panel's own — so `scroll + row` is the row of the card under the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub column: usize,
    pub row: usize,
    pub scroll: usize,
    pub width: usize,
}

/// The character under `cell`, as a position in the thread's own text.
///
/// It goes back through `wrap::rows`, which is what the panel is drawn with,
/// rather than counting rows for itself: the character under the pointer has to
/// be the character the reader sees, and a second idea of where a line breaks
/// would put the two a word apart on a narrow panel. `now` is the frame's,
/// because a clock is a row's prefix and a clock that has ticked into another
/// column is a row that breaks somewhere else.
///
/// `None` is a cell with no text under it — past the last row of the card — which
/// is a place a reader can put the pointer and not a position in the
/// conversation.
#[must_use]
pub fn position_at(thread: &Thread, cell: Cell, now: Instant) -> Option<Position> {
    let mut wanted = cell.scroll.checked_add(cell.row)?;
    let pieces = thread.pieces();

    for sourced in thread.sourced(now) {
        let height = rows(&sourced.line, cell.width).len();
        if wanted >= height {
            wanted -= height;
            continue;
        }
        return Some(snapped(&pieces, at_row(&sourced, wanted, cell)));
    }

    None
}

fn at_row(sourced: &Sourced, row: usize, cell: Cell) -> Position {
    if sourced.work {
        return Position::new(sourced.piece, sourced.offset);
    }

    let laid = Laid::new(&sourced.line, cell.width);
    let broken = laid.rows();
    let Some(&(start, text)) = broken.get(row) else {
        // `rows` and `wrapped_at` break a line into the same rows, so a row
        // `wrapped_at` has not got is a row nothing drew. The end of the line is
        // the answer that stays inside the text either way.
        return Position::new(sourced.piece, sourced.offset + laid.shape.text.len());
    };

    // A column on the marker or in the clock's own column is the first character
    // of the row: the prefix is not the reader's text and holds no offset.
    let into = offset_at(text, cell.column.saturating_sub(laid.indent));
    Position::new(sourced.piece, sourced.offset + start + into)
}

/// The panel's own window on the card: `scroll` is the card row its first row
/// shows, and `width` and `height` are the rows area's. `scroll` and `width` are
/// what a [`Cell`] carries; the height is here because a span is only wanted for
/// a row that is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub scroll: usize,
    pub width: usize,
    pub height: usize,
}

/// The cells of one row on screen that a selection covers. `row` is counted from
/// the window's first row and `from` from the first cell of the panel's rows
/// area, which is the pair a [`Cell`] carries, and `to` is one past the last
/// cell covered.
///
/// Never empty and never a prefix column: a row holding none of the selected
/// text has no span at all rather than one of no width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub row: usize,
    pub from: usize,
    pub to: usize,
}

/// Which cells of the rows on screen the selected text is drawn in, in row
/// order, one span per row that holds any of it.
///
/// [`position_at`] backwards, through the same walk of `thread.sourced(now)` and
/// the same `Laid`: a second idea of where a line breaks or how wide a marker
/// is would be a highlight a column off the text it claims to cover. `now` is
/// the frame's, for the reason it is there.
///
/// A selection with nothing between its ends covers no cell at all — the press
/// that starts a drag nobody makes has to leave the card exactly as it was.
#[must_use]
pub fn spans_at(thread: &Thread, selection: Selection, window: Window, now: Instant) -> Vec<Span> {
    if selection.start == selection.end {
        return Vec::new();
    }

    let bottom = window.scroll.saturating_add(window.height);
    let mut spans = Vec::new();
    let mut next = 0_usize;

    for sourced in thread.sourced(now) {
        let top = next;
        next = top.saturating_add(rows(&sourced.line, window.width).len());
        if next <= window.scroll {
            continue;
        }
        if top >= bottom {
            break;
        }
        // A work row stands for the end of the question above it and holds no
        // text of its own, so there is nothing on it to draw highlighted.
        if sourced.work
            || sourced.piece < selection.start.piece
            || sourced.piece > selection.end.piece
        {
            continue;
        }

        let laid = Laid::new(&sourced.line, window.width);
        for (index, (start, text)) in laid.rows().into_iter().enumerate() {
            let card = top.saturating_add(index);
            if card < window.scroll {
                continue;
            }
            if card >= bottom {
                break;
            }
            let begin = sourced.offset.saturating_add(start);
            if let Some((from, to)) = laid.covered(text, begin, sourced.piece, selection) {
                spans.push(Span {
                    row: card - window.scroll,
                    from,
                    to,
                });
            }
        }
    }

    spans
}

/// One line of the card as the panel lays it out: the prefix drawn in front of
/// it, how many columns that takes, and the rows the rest was broken into. Both
/// directions read it — a column out of a row, and columns back onto one — so
/// the indent is measured once and the two cannot come to disagree about which
/// column a byte is drawn in.
struct Laid {
    shape: Shape,
    indent: usize,
    inside: usize,
}

impl Laid {
    fn new(line: &Line, width: usize) -> Self {
        let shape = shape(line);
        let indent = display_width(&shape.prefix);
        Self {
            inside: width.saturating_sub(indent),
            shape,
            indent,
        }
    }

    fn rows(&self) -> Vec<(usize, &str)> {
        wrapped_at(&self.shape.text, self.inside)
    }

    /// `text` is one of [`Laid::rows`] and `begin` where it starts in `piece`.
    fn covered(
        &self,
        text: &str,
        begin: usize,
        piece: usize,
        selection: Selection,
    ) -> Option<(usize, usize)> {
        let end = begin.saturating_add(text.len());
        let first = if piece == selection.start.piece {
            selection.start.offset
        } else {
            begin
        };
        let last = if piece == selection.end.piece {
            selection.end.offset
        } else {
            end
        };
        // Floored because a selection holds offsets snapped against the pieces
        // it was built from rather than against this row, and `[..]` at a byte
        // inside a character is a panic. The clamp is what lets the two ends
        // meet: the space a wrap break was made at belongs to no row, so a
        // selection can end between two rows and cover neither's bytes.
        let first = text.floor_char_boundary(first.clamp(begin, end) - begin);
        let last = text.floor_char_boundary(last.clamp(begin, end) - begin);
        if first >= last {
            return None;
        }

        let from = self.indent + display_width(&text[..first]);
        let to = self.indent + display_width(&text[..last]);
        (from < to).then_some((from, to))
    }
}

/// Columns, not bytes and not characters: a wide character is two of them and a
/// pointer on either half of it is on that one character.
fn offset_at(text: &str, column: usize) -> usize {
    let mut taken = 0;
    for (index, character) in text.char_indices() {
        let next = index + character.len_utf8();
        taken += display_width(&text[index..next]);
        if taken > column {
            return index;
        }
    }

    text.len()
}

fn snapped(pieces: &[&str], position: Position) -> Position {
    let Some(text) = pieces.get(position.piece) else {
        return Position::new(position.piece, 0);
    };
    let offset = text.floor_char_boundary(position.offset.min(text.len()));
    Position::new(position.piece, offset)
}

#[cfg(test)]
#[path = "tests/selection.rs"]
mod tests;
