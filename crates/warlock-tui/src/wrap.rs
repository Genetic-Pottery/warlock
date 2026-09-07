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
        Line::Wrapped { text, heading } => Shape {
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
    rows.extend(pieces.map(|piece| Line::Wrapped {
        text: format!("{blanks}{piece}"),
        heading: shape.heading,
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
        },
    }
}

// Never empty: a line with nothing on it is one blank row, because a blank line
// in a file is a paragraph break and swallowing it would re-flow the document
// into one long block. A `width` of zero is a panel nobody has measured yet and
// is not a width to break at — the frame that measures the panel is the one that
// wraps it.
pub(crate) fn wrapped(text: &str, width: usize) -> Vec<String> {
    if width == 0 || display_width(text) <= width {
        return vec![text.to_owned()];
    }

    let mut rows = Vec::new();
    let mut rest = text;
    loop {
        if display_width(rest) <= width {
            rows.push(rest.to_owned());
            return rows;
        }

        let end = break_at(rest, width);
        rows.push(rest[..end].trim_end().to_owned());
        // The space the break was made at goes with the row above it. A break
        // made mid-word has no space to eat, so this takes nothing off the text.
        rest = rest[end..].trim_start();
        if rest.is_empty() {
            // A line that came out even. Falling through would put a blank row
            // under it, which is a paragraph break the file does not have.
            return rows;
        }
    }
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
    loop {
        if display_width(rest) <= width {
            rows.push(rest.to_owned());
            return rows;
        }

        let end = filled(rest, width);
        rows.push(rest[..end].to_owned());
        rest = &rest[end..];
        if rest.is_empty() {
            // A text that came out even. Falling through would put a blank row
            // under it, which is a row the text does not have.
            return rows;
        }
    }
}

// Never zero for a text with anything in it, which is where `folded`'s loop gets
// its promise of making progress: it has no `trim_start` to fall back on, so the
// cut alone has to advance it. A character wider than the whole field overhangs
// its row rather than stopping the fold.
fn filled(text: &str, width: usize) -> usize {
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

    fits.max(first_character(text))
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
mod tests {
    use std::path::PathBuf;

    use super::{Line, first_character, folded, rows, shape, wrapped};
    use crate::ui::display_width;

    // Narrow enough that the clock column is a visible share of it.
    const NARROW: usize = 18;

    // Everything a byte-keeping wrapper has to survive: several words, a path
    // with nowhere to break in it, whitespace at each end, two spaces in a row,
    // and characters two columns wide.
    const DRAFTS: [&str; 6] = [
        "It walks the tree and writes what it finds.",
        "crates/warlock-engine/src/pact.rs",
        "  leading and trailing  ",
        "two  spaces  between  words",
        "日本語 and more",
        "日本語のテキストはここにあります",
    ];

    #[test]
    fn a_line_that_fits_is_the_line_it_was() {
        // Not a copy worded some other way and not a wrapped row: the value the
        // card holds, so a test asserting on a panel asserts on the card.
        for line in [
            Line::Directory {
                path: PathBuf::from("crates/engine"),
            },
            Line::Clocked {
                clock: "0:09".to_owned(),
                text: "thinking".to_owned(),
            },
            Line::Summary {
                text: "pact finished".to_owned(),
            },
            Line::Said {
                text: "why?".to_owned(),
            },
            Line::Note {
                text: "no such command".to_owned(),
            },
            Line::Text {
                text: "It walks the tree.".to_owned(),
            },
        ] {
            assert_eq!(rows(&line, NARROW), vec![line.clone()], "{line:?}");
        }
    }

    #[test]
    fn a_panel_nobody_has_measured_breaks_no_line_of_any_card() {
        let long = Line::Clocked {
            clock: "0:09".to_owned(),
            text: "the turn failed — exit status 1: session already in use".to_owned(),
        };

        assert_eq!(rows(&long, 0), vec![long.clone()]);
    }

    #[test]
    fn a_clocked_line_is_broken_into_the_clocks_own_column() {
        let line = Line::Clocked {
            clock: "0:02".to_owned(),
            text: "Read crates/warlock-engine/src/pact.rs".to_owned(),
        };

        // The first row is still the clocked line it was — it keeps the clock,
        // which is what the panel draws in front of it — and every row after it
        // is blank where that clock was, so the text stays in one column and a
        // row with no clock reads as the row above still going.
        assert_eq!(
            rows(&line, NARROW),
            vec![
                Line::Clocked {
                    clock: "0:02".to_owned(),
                    text: "Read".to_owned(),
                },
                Line::Wrapped {
                    text: "       crates/warl".to_owned(),
                    heading: false,
                },
                Line::Wrapped {
                    text: "       ock-engine/".to_owned(),
                    heading: false,
                },
                Line::Wrapped {
                    text: "       src/pact.rs".to_owned(),
                    heading: false,
                },
            ],
        );
    }

    #[test]
    fn a_question_is_broken_under_its_marker_and_stays_a_heading() {
        let rows = rows(
            &Line::Said {
                text: "what does the engine do?".to_owned(),
            },
            NARROW,
        );

        assert_eq!(
            rows,
            vec![
                Line::Said {
                    text: "what does the".to_owned(),
                },
                Line::Wrapped {
                    text: "  engine do?".to_owned(),
                    heading: true,
                },
            ],
        );
    }

    #[test]
    fn a_note_is_broken_under_its_own_marker_and_stays_plain() {
        // Warlock's own line wraps like every other row of the card: the first
        // row keeps the variant, so it keeps the marker the panel draws in
        // front of it, and the rest sit in the column that marker left. Plain
        // throughout, where a question of the same length is bold throughout.
        let rows = rows(
            &Line::Note {
                text: "the commands are /brief, /write and /chat".to_owned(),
            },
            NARROW,
        );

        assert_eq!(
            rows,
            vec![
                Line::Note {
                    text: "the commands are".to_owned(),
                },
                Line::Wrapped {
                    text: "  /brief, /write".to_owned(),
                    heading: false,
                },
                Line::Wrapped {
                    text: "  and /chat".to_owned(),
                    heading: false,
                },
            ],
        );
    }

    #[test]
    fn a_heading_too_long_for_the_panel_is_rows_rather_than_a_path_in_pieces() {
        // A piece of a path is not a path, so a broken heading gives up the
        // variant rather than holding a fragment that reads like a directory.
        let rows = rows(
            &Line::Directory {
                path: PathBuf::from("crates/warlock-engine/src"),
            },
            NARROW,
        );

        assert_eq!(
            rows,
            vec![
                Line::Wrapped {
                    text: "crates/warlock-eng".to_owned(),
                    heading: true,
                },
                Line::Wrapped {
                    text: "ine/src".to_owned(),
                    heading: true,
                },
            ],
        );
    }

    #[test]
    fn no_row_of_any_line_is_wider_than_the_panel() {
        let lines = [
            Line::Directory {
                path: PathBuf::from("crates/warlock-engine/src/pact.rs"),
            },
            Line::Clocked {
                clock: "10:09".to_owned(),
                text: "the turn failed — exit status 1: session already in use".to_owned(),
            },
            Line::Summary {
                text: "pact finished — 12 directories, 21:30, $2.10".to_owned(),
            },
            Line::Said {
                text: "what is the name of this repository?".to_owned(),
            },
            Line::Note {
                text: "the commands are /brief, /write and /chat".to_owned(),
            },
            Line::Text {
                text: "It walks the tree and writes what it finds.".to_owned(),
            },
        ];

        // Every width a panel can be. Below the width of the prefix itself
        // there is no column left to put text in, and the line is handed on
        // whole for the renderer to cut — a panel six columns wide has nothing
        // readable in it however the text is broken — so that is the one case
        // this does not ask about.
        for width in 1..40 {
            for line in &lines {
                let room = width > display_width(&shape(line).prefix);
                for row in rows(line, width) {
                    let shape = shape(&row);
                    let drawn = format!("{}{}", shape.prefix, shape.text);
                    assert!(
                        display_width(&drawn) <= width || !room,
                        "row {drawn:?} is wider than {width}",
                    );
                }
            }
        }
    }

    #[test]
    fn a_wrapped_row_is_already_composed_and_is_never_broken_again() {
        // What keeps re-flowing idempotent: a row that has been through here
        // carries its own indent, so putting it through again at the same width
        // gives back the same row rather than indenting it twice.
        let line = Line::Clocked {
            clock: "0:02".to_owned(),
            text: "Read crates/warlock-engine/src/pact.rs".to_owned(),
        };
        let once = rows(&line, NARROW);
        let twice: Vec<Line> = once.iter().flat_map(|row| rows(row, NARROW)).collect();

        assert_eq!(twice, once);
    }

    #[test]
    fn a_line_inside_the_width_is_one_row() {
        assert_eq!(wrapped("# The engine", 20), ["# The engine"]);
        assert_eq!(wrapped("exactly twelve", 14), ["exactly twelve"]);
    }

    #[test]
    fn an_empty_line_is_one_empty_row() {
        assert_eq!(wrapped("", 20), [""]);
        assert_eq!(wrapped("", 0), [""]);
    }

    #[test]
    fn a_long_line_breaks_at_spaces_and_the_rows_are_the_words_in_order() {
        let rows = wrapped("It walks the tree and writes what it finds.", 18);

        assert_eq!(rows, ["It walks the tree", "and writes what it", "finds."]);
        // The words are all still there, in order, with the spaces broken at the
        // only thing missing.
        assert_eq!(
            rows.join(" "),
            "It walks the tree and writes what it finds."
        );
    }

    #[test]
    fn no_row_is_wider_than_the_width() {
        let text =
            "crates/warlock-engine/src/pact.rs is where the manifest is written and read again";
        for width in 1..40 {
            for row in wrapped(text, width) {
                assert!(
                    display_width(&row) <= width,
                    "row {row:?} is wider than {width}"
                );
            }
        }
    }

    #[test]
    fn a_word_longer_than_the_width_is_broken_mid_word() {
        // A path with nowhere to break in it, which is what a document full of
        // them does to a narrow panel.
        let rows = wrapped("crates/warlock-engine/src/pact.rs", 12);

        assert_eq!(rows, ["crates/warlo", "ck-engine/sr", "c/pact.rs"]);
        // Nothing was dropped: a mid-word break has no space to eat.
        assert_eq!(rows.concat(), "crates/warlock-engine/src/pact.rs");
    }

    #[test]
    fn a_panel_nobody_has_measured_wraps_nothing() {
        let long = "It walks the tree and writes what it finds.";

        assert_eq!(wrapped(long, 0), [long]);
    }

    #[test]
    fn a_width_of_one_still_gets_to_the_end_of_the_line() {
        // The degenerate width, which the loop has to terminate at rather than
        // breaking off empty rows for ever.
        let rows = wrapped("ab cd", 1);

        assert_eq!(rows, ["a", "b", "c", "d"]);
    }

    #[test]
    fn a_character_wider_than_the_panel_overhangs_rather_than_stopping_the_wrap() {
        // Two columns apiece in a panel one column wide: there is no cut that
        // fits, so each character takes its own row and the line still ends.
        let rows = wrapped("日本語", 1);

        assert_eq!(rows, ["日", "本", "語"]);
    }

    #[test]
    fn columns_are_counted_rather_than_bytes_or_characters() {
        // Six columns of CJK in a panel eight wide, then a break: counted in
        // characters this would fit, and counted in bytes it would break far too
        // early.
        let rows = wrapped("日本語 and more", 8);

        assert_eq!(rows, ["日本語", "and more"]);
    }

    #[test]
    fn leading_whitespace_on_the_line_is_kept_and_never_breaks_off_an_empty_row() {
        let rows = wrapped("    indented text here", 10);

        assert_eq!(rows[0], "    indent");
        assert_eq!(rows.concat().replace(' ', ""), "indentedtexthere");
    }

    #[test]
    fn folded_rows_join_back_up_to_the_text_byte_for_byte() {
        for draft in DRAFTS {
            for width in 0..40 {
                assert_eq!(
                    folded(draft, width).concat(),
                    draft,
                    "{draft:?} folded at {width}",
                );
            }
        }
    }

    #[test]
    fn the_field_fills_every_row_where_the_panel_breaks_at_words() {
        for draft in [
            "It walks the tree and writes what it finds.",
            "crates/warlock-engine/src/pact.rs",
            "日本語 and more",
        ] {
            for width in 1..40 {
                let rows = folded(draft, width);
                for (row, next) in rows.iter().zip(rows.iter().skip(1)) {
                    // The first character of the row below did not fit on this
                    // one: that is what a filled row means, and a row of a word
                    // break would leave room for it.
                    let character = display_width(&next[..first_character(next)]);
                    assert!(
                        display_width(row) + character > width,
                        "{draft:?} at {width}: {row:?} had room for {next:?}",
                    );
                }
            }
        }
    }

    #[test]
    fn a_run_of_one_word_fills_the_row_rather_than_being_pushed_down() {
        let inside = format!("test {}", "h".repeat(10));
        let past = format!("test {}", "h".repeat(30));

        // The panel: a row of its own while the run fits one, mid-word after.
        assert_eq!(wrapped(&inside, 12), ["test", "hhhhhhhhhh"]);
        assert_eq!(
            wrapped(&past, 12),
            ["test", "hhhhhhhhhhhh", "hhhhhhhhhhhh", "hhhhhh"],
        );

        // The field: the same row, filled, either side of the length that made
        // the panel change its mind.
        assert_eq!(folded(&inside, 12), ["test hhhhhhh", "hhh"]);
        assert_eq!(
            folded(&past, 12),
            ["test hhhhhhh", "hhhhhhhhhhhh", "hhhhhhhhhhh"],
        );
        // Nothing was dropped to make room: the rows are the draft again.
        assert_eq!(folded(&past, 12).concat(), past);
    }

    #[test]
    fn folded_counts_the_columns_a_character_draws_in() {
        assert_eq!(folded("日本語 and more", 8), ["日本語 a", "nd more"]);
    }

    #[test]
    fn folded_at_a_width_of_zero_is_the_row_it_went_in_as() {
        let long = "It walks the tree and writes what it finds.";

        assert_eq!(folded(long, 0), [long]);
        assert_eq!(folded("", 0), [""]);
        assert_eq!(folded("", 20), [""]);
    }

    #[test]
    fn folded_splits_only_on_character_boundaries() {
        for draft in DRAFTS {
            for width in 1..30 {
                let mut offset = 0;
                for row in folded(draft, width) {
                    assert!(
                        draft.is_char_boundary(offset),
                        "{draft:?} at {width} split inside a character at {offset}",
                    );
                    assert_eq!(draft[offset..offset + row.len()], *row);
                    offset += row.len();
                }
                assert_eq!(offset, draft.len(), "{draft:?} at {width}");
            }
        }
    }

    #[test]
    fn no_row_of_a_folded_draft_is_wider_than_the_field() {
        assert_eq!(
            folded("and writes what it finds.", 18),
            ["and writes what it", " finds."],
        );

        for draft in DRAFTS {
            for width in 1..30 {
                for row in folded(draft, width) {
                    assert!(
                        display_width(&row) <= width || row.chars().count() == 1,
                        "row {row:?} is wider than {width}",
                    );
                }
            }
        }
    }

    #[test]
    fn folded_keeps_a_run_of_spaces_that_the_panels_wrapper_swallows() {
        assert_eq!(folded("a  b", 2), ["a ", " b"]);
        assert_eq!(wrapped("a  b", 2), ["a", "b"]);
    }

    #[test]
    fn folded_at_a_width_of_one_still_gets_to_the_end_of_the_draft() {
        assert_eq!(folded("ab cd", 1), ["a", "b", " ", "c", "d"]);
        assert_eq!(folded("日本語", 1), ["日", "本", "語"]);
    }
}
