//! A selection is positions in the thread's own text, never screen cells: a
//! piece index and a byte offset into that piece. That is what lets the same
//! selection survive a terminal made narrower, a card scrolled, and text
//! arriving under it, none of which change where a character sits in the string
//! it belongs to. Nothing here knows about rows, columns, markers or the indent
//! — what is copied is what the thread stores, not what the panel drew.

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

    use super::{Position, Selection, copied_text};
    use crate::claude::Activity;
    use crate::thread::Thread;

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
}
