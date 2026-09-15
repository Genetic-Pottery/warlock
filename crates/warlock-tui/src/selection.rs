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
    let mut copied = String::new();

    for (index, text) in pieces
        .iter()
        .enumerate()
        .take(end.piece.saturating_add(1))
        .skip(start.piece)
    {
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
        let slice = text.get(from..to).unwrap_or_default();
        if slice.is_empty() {
            continue;
        }
        if !copied.is_empty() {
            copied.push_str("\n\n");
        }
        copied.push_str(slice);
    }

    copied
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
mod tests {
    use std::time::{Duration, Instant};

    use super::{Cell, Position, Selection, Span, Window, copied_text, position_at, spans_at};
    use crate::claude::Activity;
    use crate::thread::{Ending, Thread};

    // Narrow enough that a question, an answer and a note all break, and wide
    // enough that the clock's column is not the whole of it.
    const NARROW: usize = 20;

    // Nothing in any fixture here is still running, so every clock is frozen and
    // no row's prefix depends on which instant this is.
    fn frame() -> Instant {
        Instant::now()
    }

    const fn cell(column: usize, row: usize, width: usize) -> Cell {
        Cell {
            column,
            row,
            scroll: 0,
            width,
        }
    }

    // Taller than any fixture here, so the window is the whole card unless the
    // test says otherwise.
    const fn whole(width: usize) -> Window {
        Window {
            scroll: 0,
            width,
            height: 40,
        }
    }

    const fn span(row: usize, from: usize, to: usize) -> Span {
        Span { row, from, to }
    }

    fn conversation() -> Thread {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("what is in crates/warlock-engine?", base);
        thread.record(&Activity::Thinking, base + Duration::from_secs(1));
        thread.answer(
            "The engine crate holds the pact model.",
            base + Duration::from_secs(2),
        );
        thread.note("a note about\n  something", base + Duration::from_secs(3));

        thread
    }

    #[test]
    fn a_selection_inside_one_answer_copies_that_slice() {
        let thread = conversation();
        let pieces = thread.pieces();

        let selection = Selection::new(&pieces, Position::new(1, 4), Position::new(1, 16));

        assert_eq!(copied_text(&pieces, selection), "engine crate");
    }

    #[test]
    fn a_selection_spanning_a_message_an_answer_and_a_note_copies_all_three() {
        let thread = conversation();
        let pieces = thread.pieces();

        let selection = Selection::new(&pieces, Position::new(0, 8), Position::new(2, 6));

        assert_eq!(
            copied_text(&pieces, selection),
            "in crates/warlock-engine?\n\nThe engine crate holds the pact model.\n\na note",
        );
    }

    #[test]
    fn a_backwards_selection_is_the_forwards_one() {
        let thread = conversation();
        let pieces = thread.pieces();
        let start = Position::new(0, 8);
        let end = Position::new(2, 6);

        let forwards = Selection::new(&pieces, start, end);
        let backwards = Selection::new(&pieces, end, start);

        assert_eq!(backwards, forwards);
        assert_eq!(
            copied_text(&pieces, backwards),
            copied_text(&pieces, forwards),
        );
    }

    #[test]
    fn a_piece_with_nothing_selected_in_it_contributes_nothing() {
        let pieces = ["one", "two", "three"];

        let selection = Selection::new(&pieces, Position::new(0, 3), Position::new(2, 0));

        assert_eq!(copied_text(&pieces, selection), "two");
    }

    #[test]
    fn an_offset_inside_a_character_snaps_to_a_boundary() {
        let pieces = ["héllo"];

        let selection = Selection::new(&pieces, Position::new(0, 0), Position::new(0, 2));

        assert_eq!(selection.end().offset(), 1);
        assert_eq!(copied_text(&pieces, selection), "h");
    }

    #[test]
    fn an_offset_past_the_end_of_a_piece_is_clamped() {
        let pieces = ["one", "two"];

        let selection = Selection::new(&pieces, Position::new(0, 0), Position::new(1, 99));

        assert_eq!(selection.end().offset(), 3);
        assert_eq!(copied_text(&pieces, selection), "one\n\ntwo");
    }

    #[test]
    fn work_rows_are_not_pieces() {
        let thread = conversation();
        let pieces = thread.pieces();

        let selection = Selection::new(
            &pieces,
            Position::new(0, 0),
            Position::new(pieces.len() - 1, usize::MAX),
        );

        assert_eq!(
            copied_text(&pieces, selection),
            "what is in crates/warlock-engine?\n\n\
             The engine crate holds the pact model.\n\n\
             a note about something",
        );
    }

    #[test]
    fn an_answer_arriving_moves_no_position_taken_before_it() {
        let base = Instant::now();
        let mut thread = Thread::new();
        thread.ask("first question", base);
        thread.answer("first answer", base + Duration::from_secs(1));
        thread.note("a note", base + Duration::from_secs(2));
        thread.ask("second question", base + Duration::from_secs(3));

        let selection = Selection::new(&thread.pieces(), Position::new(0, 6), Position::new(3, 6));
        let before = copied_text(&thread.pieces(), selection);

        thread.answer("second answer", base + Duration::from_secs(4));

        assert_eq!(before, "question\n\nfirst answer\n\na note\n\nsecond");
        assert_eq!(copied_text(&thread.pieces(), selection), before);
    }

    #[test]
    fn a_cell_on_a_wrapped_row_is_the_character_the_reader_sees() {
        let thread = conversation();
        let pieces = thread.pieces();

        // Three rows under a marker two columns wide — `what is in`,
        // `crates/warlock-eng`, `ine?` — so the second row starts a word into
        // the question and two columns in from the panel's edge.
        let position =
            position_at(&thread, cell(4, 1, NARROW), frame()).expect("the question has three rows");

        assert_eq!(position, Position::new(0, 13));
        assert_eq!(&pieces[0][position.offset()..], "ates/warlock-engine?");
    }

    #[test]
    fn a_row_of_a_message_broken_at_its_own_newline_is_that_lines_own_bytes() {
        let base = Instant::now();
        let mut thread = Thread::new();
        thread.ask("first line\r\nsecond line", base);
        let pieces = thread.pieces();

        // One piece, two rows, and the break between them — `\r\n`, which is in
        // neither row — is what the second row's offset has to have stepped over.
        let position = position_at(&thread, cell(2, 1, 40), frame()).expect("a second row");

        assert_eq!(position, Position::new(0, 12));
        assert_eq!(&pieces[0][position.offset()..], "second line");
    }

    #[test]
    fn a_cell_on_a_marker_is_the_first_character_of_the_row_it_marks() {
        let thread = conversation();

        // `› ` and `· ` are warlock's own columns and hold none of the text, so
        // a cell on one is the row's first character rather than a position
        // inside the marker.
        for column in 0..=2 {
            assert_eq!(
                position_at(&thread, cell(column, 0, NARROW), frame()),
                Some(Position::new(0, 0)),
                "column {column} of the question",
            );
            assert_eq!(
                position_at(&thread, cell(column, 7, NARROW), frame()),
                Some(Position::new(2, 0)),
                "column {column} of the note",
            );
        }

        assert_eq!(
            position_at(&thread, cell(3, 0, NARROW), frame()),
            Some(Position::new(0, 1)),
        );
        assert_eq!(
            position_at(&thread, cell(4, 7, NARROW), frame()),
            Some(Position::new(2, 2)),
        );
    }

    #[test]
    fn a_cell_on_a_work_row_is_the_end_of_the_question_above_it() {
        let thread = conversation();
        let pieces = thread.pieces();
        let end = Position::new(0, pieces[0].len());

        // Anywhere on the row: the clock's own column, its text, and past the
        // end of both.
        for column in [0, 3, 8, 19, 99] {
            assert_eq!(
                position_at(&thread, cell(column, 3, NARROW), frame()),
                Some(end),
                "column {column} of the work row",
            );
        }

        let selection = Selection::new(&pieces, Position::new(0, 11), end);
        assert_eq!(copied_text(&pieces, selection), "crates/warlock-engine?");
    }

    #[test]
    fn a_cell_on_the_line_that_ended_the_turn_is_the_end_of_the_question_too() {
        let base = Instant::now();
        let mut thread = Thread::new();
        thread.ask("read the tree", base);
        thread.record(&Activity::Thinking, base + Duration::from_secs(1));
        thread.end(&Ending::Cancelled, base + Duration::from_secs(2));

        // The ending is filed as an ordinary line of the same log, and wraps
        // into two rows at this width: none of the three is anybody's text.
        for row in 1..=3 {
            assert_eq!(
                position_at(&thread, cell(4, row, NARROW), frame()),
                Some(Position::new(0, "read the tree".len())),
                "row {row}",
            );
        }
        assert_eq!(thread.pieces(), vec!["read the tree"]);
    }

    #[test]
    fn the_same_character_is_the_same_position_at_either_width() {
        let thread = conversation();
        let pieces = thread.pieces();

        // The `p` of `pact`: the second row of a broken answer, and the only row
        // of the same answer on a panel wide enough to hold it.
        let narrow = position_at(&thread, cell(10, 5, NARROW), frame()).expect("a broken answer");
        let wide = position_at(&thread, cell(27, 2, 40), frame()).expect("an answer on one row");
        let to = Position::new(1, 31);

        assert_eq!(narrow, wide);
        assert_eq!(narrow, Position::new(1, 27));
        assert_eq!(
            copied_text(&pieces, Selection::new(&pieces, narrow, to)),
            "pact",
        );
        assert_eq!(
            copied_text(&pieces, Selection::new(&pieces, wide, to)),
            "pact",
        );
    }

    #[test]
    fn the_row_resolved_is_the_panels_own_scroll_offset_plus_the_hit() {
        let thread = conversation();
        let scrolled = Cell {
            column: 4,
            row: 1,
            scroll: 4,
            width: NARROW,
        };

        assert_eq!(
            position_at(&thread, scrolled, frame()),
            position_at(&thread, cell(4, 5, NARROW), frame()),
        );
    }

    #[test]
    fn a_cell_with_no_text_under_it_has_an_answer_rather_than_a_panic() {
        let thread = conversation();
        let pieces = thread.pieces();

        // Past the end of a row is the end of that row's own text: the space the
        // break was made at went with the row and belongs to neither.
        assert_eq!(
            position_at(&thread, cell(19, 0, NARROW), frame()),
            Some(Position::new(0, 10)),
        );
        assert_eq!(&pieces[0][..10], "what is in");

        // Past the last row is a place the pointer can be and the conversation
        // is not.
        assert_eq!(position_at(&thread, cell(0, 9, NARROW), frame()), None);
        assert_eq!(position_at(&thread, cell(99, 999, NARROW), frame()), None);
        assert_eq!(
            position_at(
                &thread,
                Cell {
                    column: 0,
                    row: 1,
                    scroll: usize::MAX,
                    width: NARROW,
                },
                frame(),
            ),
            None,
        );

        // A panel nobody has measured breaks no line, so each line of the card
        // is one row of it and the columns still count from the marker.
        assert_eq!(
            position_at(&thread, cell(5, 0, 0), frame()),
            Some(Position::new(0, 3)),
        );
        assert_eq!(
            position_at(&thread, cell(0, 3, 0), frame()),
            Some(Position::new(2, 0)),
        );
        assert_eq!(position_at(&thread, cell(0, 4, 0), frame()), None);
    }

    #[test]
    fn a_wide_character_is_one_character_under_either_of_its_columns() {
        let base = Instant::now();
        let mut thread = Thread::new();
        thread.ask("日本語", base);
        thread.answer("日本語 and more", base + Duration::from_secs(1));

        for column in [2, 3] {
            assert_eq!(
                position_at(&thread, cell(column, 0, 40), frame()),
                Some(Position::new(0, 0)),
                "column {column}",
            );
        }
        for column in [4, 5] {
            assert_eq!(
                position_at(&thread, cell(column, 0, 40), frame()),
                Some(Position::new(0, 3)),
                "column {column}",
            );
        }
    }

    #[test]
    fn a_selection_inside_one_row_is_that_rows_own_columns() {
        let thread = conversation();
        let pieces = thread.pieces();

        // `engine crate`, four columns into the answer's first row, which is the
        // fifth row of the card and has no marker in front of it.
        let selection = Selection::new(&pieces, Position::new(1, 4), Position::new(1, 16));

        assert_eq!(copied_text(&pieces, selection), "engine crate");
        assert_eq!(
            spans_at(&thread, selection, whole(NARROW), frame()),
            vec![span(4, 4, 16)],
        );
    }

    #[test]
    fn a_selection_across_a_break_covers_both_rows_and_not_the_space_between() {
        let thread = conversation();
        let pieces = thread.pieces();

        // `engine crate holds the pact`: the end of one row of the answer and
        // the whole of the next. The space the break was made at is in neither
        // row, so no cell stands for it and none is highlighted for it.
        let selection = Selection::new(&pieces, Position::new(1, 4), Position::new(1, 31));

        assert_eq!(
            copied_text(&pieces, selection),
            "engine crate holds the pact",
        );
        assert_eq!(
            spans_at(&thread, selection, whole(NARROW), frame()),
            vec![span(4, 4, 16), span(5, 0, 14)],
        );
    }

    #[test]
    fn a_selection_across_pieces_covers_every_row_but_the_work_row() {
        let thread = conversation();
        let pieces = thread.pieces();

        let selection = Selection::new(&pieces, Position::new(0, 8), Position::new(2, 6));

        // Rows 0 to 2 are the question under its marker, row 3 is the work row
        // — nobody's text — rows 4 to 6 are the answer, and rows 7 and 8 are the
        // note. The question starts eight bytes in and the note ends six bytes
        // in, so those two rows are part of a row and the rest are whole ones.
        assert_eq!(
            spans_at(&thread, selection, whole(NARROW), frame()),
            vec![
                span(0, 10, 12),
                span(1, 2, 20),
                span(2, 2, 6),
                span(4, 0, 16),
                span(5, 0, 14),
                span(6, 0, 6),
                span(7, 2, 8),
            ],
        );
    }

    #[test]
    fn a_selection_with_nothing_between_its_ends_covers_no_cell() {
        let thread = conversation();
        let pieces = thread.pieces();

        for position in [Position::new(0, 0), Position::new(1, 12)] {
            let selection = Selection::new(&pieces, position, position);

            assert!(copied_text(&pieces, selection).is_empty());
            assert_eq!(
                spans_at(&thread, selection, whole(NARROW), frame()),
                Vec::new(),
                "{position:?}",
            );
        }
    }

    #[test]
    fn only_the_rows_the_window_is_over_get_a_span() {
        let thread = conversation();
        let pieces = thread.pieces();
        let selection = Selection::new(&pieces, Position::new(0, 8), Position::new(2, 6));
        let window = Window {
            scroll: 5,
            width: NARROW,
            height: 2,
        };

        // The same selection through a window two rows tall: the rows above and
        // below it are as selected as they were and have nowhere to be drawn, so
        // what is left is rows 5 and 6 of the card, counted from the window.
        assert_eq!(
            spans_at(&thread, selection, window, frame()),
            vec![span(0, 0, 14), span(1, 0, 6)],
        );
        assert_eq!(
            spans_at(
                &thread,
                selection,
                Window { height: 0, ..window },
                frame(),
            ),
            Vec::new(),
        );
    }

    #[test]
    fn a_wide_character_is_the_two_columns_it_is_drawn_in() {
        let base = Instant::now();
        let mut thread = Thread::new();
        thread.ask("日本語 and more", base);
        let pieces = thread.pieces();

        // `本語`: two characters, six bytes, four columns, two of them past the
        // marker's own two.
        let selection = Selection::new(&pieces, Position::new(0, 3), Position::new(0, 9));

        assert_eq!(copied_text(&pieces, selection), "本語");
        assert_eq!(
            spans_at(&thread, selection, whole(40), frame()),
            vec![span(0, 4, 8)],
        );
    }

    #[test]
    fn a_highlighted_cell_is_a_cell_inside_the_selection() {
        let thread = conversation();
        let pieces = thread.pieces();
        let selection = Selection::new(&pieces, Position::new(0, 8), Position::new(2, 6));
        let window = Window {
            scroll: 2,
            width: NARROW,
            height: 5,
        };

        let spans = spans_at(&thread, selection, window, frame());
        assert!(!spans.is_empty());
        for span in spans {
            // Both ends of the span, since a span one column too wide at either
            // end is a cell drawn highlighted that the reader is not copying.
            for column in [span.from, span.to - 1] {
                let cell = Cell {
                    column,
                    row: span.row,
                    scroll: window.scroll,
                    width: window.width,
                };
                let position = position_at(&thread, cell, frame()).expect("text under the cell");
                assert!(
                    position >= selection.start() && position < selection.end(),
                    "column {column} of row {} is at {position:?}",
                    span.row,
                );
            }
        }
    }

    #[test]
    fn the_column_in_front_of_a_span_is_outside_the_selection() {
        let thread = conversation();
        let pieces = thread.pieces();
        let selection = Selection::new(&pieces, Position::new(1, 4), Position::new(1, 16));

        let spans = spans_at(&thread, selection, whole(NARROW), frame());
        let [span] = spans[..] else {
            panic!("one row of the answer, so one span: {spans:?}");
        };

        for (column, outside) in [(span.from - 1, "the cell in front"), (span.to, "the cell after")]
        {
            let position = position_at(&thread, cell(column, span.row, NARROW), frame())
                .expect("text under the cell");
            assert!(
                position < selection.start() || position >= selection.end(),
                "{outside}, column {column}, is at {position:?}",
            );
        }
    }

    #[test]
    fn every_highlighted_cell_of_every_width_maps_back_into_the_selection() {
        let thread = conversation();
        let pieces = thread.pieces();

        for width in 1..30 {
            for start in [Position::new(0, 0), Position::new(0, 17), Position::new(1, 9)] {
                for end in [Position::new(1, 1), Position::new(1, 30), Position::new(2, 22)] {
                    let selection = Selection::new(&pieces, start, end);
                    let window = whole(width);
                    for span in spans_at(&thread, selection, window, frame()) {
                        for column in [span.from, span.to - 1] {
                            let cell = Cell {
                                column,
                                row: span.row,
                                scroll: window.scroll,
                                width,
                            };
                            let position =
                                position_at(&thread, cell, frame()).expect("text under the cell");
                            assert!(
                                position >= selection.start() && position < selection.end(),
                                "column {column} of row {} at {width} is at {position:?}, \
                                 outside {selection:?}",
                                span.row,
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn every_cell_of_every_width_lands_on_a_character_boundary() {
        let base = Instant::now();
        let mut thread = Thread::new();
        thread.ask("日本語のテキストはここにあります", base);
        thread.record(&Activity::Thinking, base + Duration::from_secs(1));
        thread.answer(
            "The engine crate holds 日本語 and more.",
            base + Duration::from_secs(2),
        );
        thread.note("a note about something", base + Duration::from_secs(3));
        let pieces = thread.pieces();

        for width in 0..30 {
            for row in 0..40 {
                for column in 0..width + 4 {
                    let Some(position) = position_at(&thread, cell(column, row, width), frame())
                    else {
                        continue;
                    };
                    let text = pieces[position.piece()];
                    assert!(
                        text.is_char_boundary(position.offset()),
                        "column {column}, row {row} at {width} landed at {} inside {text:?}",
                        position.offset(),
                    );
                }
            }
        }
    }
}
