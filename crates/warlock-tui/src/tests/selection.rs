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
            Window {
                height: 0,
                ..window
            },
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

    for (column, outside) in [
        (span.from - 1, "the cell in front"),
        (span.to, "the cell after"),
    ] {
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
        for start in [
            Position::new(0, 0),
            Position::new(0, 17),
            Position::new(1, 9),
        ] {
            for end in [
                Position::new(1, 1),
                Position::new(1, 30),
                Position::new(2, 22),
            ] {
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
                let Some(position) = position_at(&thread, cell(column, row, width), frame()) else {
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
