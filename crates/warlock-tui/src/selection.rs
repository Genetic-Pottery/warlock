//! A selection is positions in the thread's own text, never screen cells: a
//! piece index and a byte offset into that piece. That is what lets the same
//! selection survive a terminal made narrower, a card scrolled, and text
//! arriving under it, none of which change where a character sits in the string
//! it belongs to. A cell is turned into one of those positions in exactly one
//! place, [`position_at`], and nothing downstream of it knows about rows,
//! columns, markers or the indent — what is copied is what the thread stores,
//! not what the panel drew.

use std::time::Instant;

use crate::thread::{Sourced, Thread};
use crate::ui::display_width;
use crate::wrap::{rows, shape, wrapped_at};

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
/// counted from the first cell of the panel's rows area, where `row` is what
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

    let shape = shape(&sourced.line);
    let indent = display_width(&shape.prefix);
    let broken = wrapped_at(&shape.text, cell.width.saturating_sub(indent));
    let Some(&(start, text)) = broken.get(row) else {
        // `rows` and `wrapped_at` break a line into the same rows, so a row
        // `wrapped_at` has not got is a row nothing drew. The end of the line is
        // the answer that stays inside the text either way.
        return Position::new(sourced.piece, sourced.offset + shape.text.len());
    };

    // A column on the marker or in the clock's own column is the first character
    // of the row: the prefix is not the reader's text and holds no offset.
    let into = offset_at(text, cell.column.saturating_sub(indent));
    Position::new(sourced.piece, sourced.offset + start + into)
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

    use super::{Cell, Position, Selection, copied_text, position_at};
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
