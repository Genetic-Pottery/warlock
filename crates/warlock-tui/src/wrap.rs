//! Breaking one line of a card into the rows a panel of some width can draw it
//! in.
//!
//! Every line of every card, and that is the rule: nothing the panel draws runs
//! off the right-hand edge. A document's line is the obvious case — nothing
//! about a file's line numbers is on screen, nothing is counting them, and the
//! far end of a sentence is the half that says what the sentence was about — but
//! an account's is the same case wearing different words. `the turn failed —
//! exit status 1: Error: Session ID … is already in` is a row that tells the
//! reader something went wrong and then takes away what it was, and a line the
//! panel cannot finish is a line the panel did not say.
//!
//! That is the whole of the choice: the alternative to wrapping is a horizontal
//! scroll — a mode, a key pair and an offset, all so a reader can shunt a
//! sentence left and right to read it. Warlock shows a document so the reader
//! can judge it against the directory it describes and an account so they can
//! see what a pass did; neither survives having to be steered.
//!
//! What is lost is worth saying out loud: one thing that happened is no longer
//! always one row, so the count of rows on screen is no longer the count of
//! things the pass was seen doing. The clock in front of each of them is what
//! now says where one thing ends and the next begins, and it is enough — a row
//! with no clock on it is the row above it, still going.
//!
//! ## What a break costs, and where it is put
//!
//! [`wrapped`] breaks at a space where there is one inside the width and mid-word
//! where there is not, so a long path or an unbroken run of `x`s still gets
//! across rather than stopping the wrap dead. Columns are counted the way
//! [`truncated`](crate::ui) counts them and the way the backend will lay the row
//! out, so a row that fits here fits there.
//!
//! [`folded`] is the rule for a caller that cannot afford to lose a byte: it
//! fills each row to the width and cuts there, so its rows join back up to the
//! text byte for byte. Nothing in this module's account of the panel changes —
//! the panel draws [`wrapped`]'s rows.
//!
//! What [`folded`] does not do is break at words, and that is the difference
//! rather than an oversight. A row of the panel is read; a row of a field is
//! cells a caret sits in, and a word break leaves the end of one standing for no
//! offset at all — `test` alone on a row with the whole width beside it
//! belonging to nothing, and the run of `h`s after it re-flowing the draft from
//! the top the moment it outgrows the row it had to itself. Filling has no such
//! step in it: what is typed goes in the next cell along, every time. Reading
//! wants the word whole; typing wants the row full.
//!
//! Continuation rows carry no marker and no ellipsis — a row of a wrapped
//! document line is the file's own text and nothing else, exactly as an
//! unwrapped one is. What they do carry is the blank width of whatever the
//! renderer puts in front of the line's first row ([`shape`]): the clock column
//! of an account line, the marker of a question. That is not decoration, it is
//! the column the text is already in, and a continuation that jumped back to
//! the left margin would read as a new thing that happened rather than as the
//! rest of the last one.
//!
//! ## Who wraps, and when
//!
//! [`rows`] is the whole of the answer, and it is called on the way to the
//! screen rather than when a line is made. A card holds what happened; how many
//! rows that takes is a question about a terminal, and the terminal changes
//! size. So a panel dragged narrower re-flows what is on it, and nothing a card
//! holds is ever cut down to a width it happened to have once.

use crate::account::Line;
use crate::ui::{NOTE_MARKER, PANEL_INDENT, SAID_MARKER, display_width};

/// How the renderer draws one line: what it puts in front of it, the line's own
/// text, and whether the row is bold.
///
/// The one description of a row's shape, read by the two halves that have to
/// agree about it — [`rows`] wraps at the width the prefix leaves, and
/// `panel_row` draws the prefix that width was measured against. Two answers
/// would be a line broken at one width and drawn at another, which is a row over
/// the border.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shape {
    /// What is drawn in front of the line's own text: the indent and clock of an
    /// account line, the marker of a question, nothing at all for a heading, a
    /// summary, a line of prose or a row that is already composed.
    pub(crate) prefix: String,
    /// The line's own text, as the card holds it.
    pub(crate) text: String,
    /// Whether the row is drawn bold: a heading, a summary or a question.
    pub(crate) heading: bool,
}

/// How `line` is drawn, before any width is taken into account.
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

/// The rows `line` draws as in a panel `width` columns wide.
///
/// One row, and `line` itself, whenever it fits — which is every line of a
/// panel nobody has measured yet, since a width of zero is not a width to break
/// at (see [`wrapped`]). A line that does not fit comes back as the rows it
/// needs, in order, and what each of them *is* follows one rule: the first row
/// keeps the line's own variant wherever that variant can hold a piece of the
/// text — a question keeps its marker, a clocked line keeps its clock, so the
/// row still says what it is — and every other row is a [`Line::Wrapped`],
/// indented under the text it continues and bold or not with the line it belongs
/// to.
///
/// A [`Line::Directory`] is the exception, and only because a piece of a path is
/// not a path: a heading too long for the panel comes back as wrapped rows
/// throughout rather than as a `Directory` holding a fragment.
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

/// `line` with `first` in place of its text: the first row of a line that had to
/// be broken.
///
/// The variant is kept where it can hold a piece of the text, because what the
/// renderer puts in front of that first row is the line's own — a clock, a
/// marker — and the wrap was measured against it.
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

/// `text` in as many rows of `width` columns as it takes, in order.
///
/// Never empty: a line with nothing on it is one blank row, because a blank line
/// in a file is a paragraph break and swallowing it would re-flow the document
/// into one long block.
///
/// A `width` of zero is a panel nobody has measured yet — the height field's
/// zero means the same thing — and is not a width to break at: the text comes
/// back as the one row it went in as, and the frame that measures the panel is
/// the one that wraps it.
///
/// Breaks are put at the last space that fits and mid-character-boundary where
/// no space does, which is what keeps a line of `x`s a thousand long from
/// wrapping to nothing. The space a row was broken at is not drawn — it would be
/// a trailing blank on one row and a leading one on the next — but a break made
/// mid-word takes nothing away, so the rows of a wrapped line always join back
/// up to the line, give or take the spaces broken at.
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

/// `text` in as many rows of `width` columns as it takes, keeping every byte.
///
/// Every row is filled: the cut is the width itself ([`filled`]), wherever in a
/// word that lands, and no byte is trimmed off either side of it. So the rows
/// concatenate back to `text` byte for byte, which is what a caller that counts
/// an offset through them needs — a space the reader typed has a cell of its own
/// to put a cursor in, and a byte that vanished at a word break would be a cell
/// that is not there to aim at.
///
/// It does not break at words, and that is the difference from [`wrapped`]
/// rather than an omission. The rows of a field are cells a caret sits in, and a
/// word break leaves the end of a row standing for no offset at all: a draft of
/// `test` and then a run of `h`s puts `test` alone on a row with the whole width
/// beside it belonging to nothing, and one more `h` — the one that outgrows the
/// row the run had to itself — re-flows the draft from the top and takes the
/// cursor with it. Filling has no such step in it: what is typed goes in the
/// next cell along, every time, and a row breaks only where the field ends.
/// Prose pays for that in words cut in half, which is a field being a field. The
/// panel is read rather than typed into and still breaks at spaces.
///
/// A `width` of zero is a field nobody has measured yet and is not a width to
/// break at: the text comes back as the one row it went in as. A text with
/// nothing in it is one empty row, as it is in [`wrapped`], so a caller always
/// has a row to sit a cursor on.
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

/// Where to cut `text` so the row before the cut is as much of it as `width`
/// columns hold.
///
/// [`folded`]'s whole rule, and there is nothing in it about words: as many
/// characters as fit, cut on a `char` boundary so no character is halved and
/// none is dropped. What is on either side of the cut is whatever the reader
/// typed there.
///
/// Never zero for a text with anything in it, which is where [`folded`]'s loop
/// gets its promise of making progress: it has no `trim_start` to fall back on,
/// so the cut alone has to advance it. A character wider than the whole field
/// overhangs its row rather than stopping the fold, exactly as one wider than
/// the panel overhangs in [`break_at`].
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

/// Where to cut `text` so the row before the cut fits in `width` columns.
///
/// Three answers in order of preference: the width itself, when the character
/// that did not fit is a space and the rows either side of the cut are whole
/// words already; the last space inside the widest prefix that fits, which is
/// the ordinary word break; and failing both, as many characters as fit, which
/// is the mid-word break a long path gets.
///
/// A space is only a candidate once a word has been seen, so a line's own
/// indentation is never what it breaks at — an indented line breaking at its
/// indent would put an empty row above itself and say nothing.
///
/// Always past the first character, whatever the width: a cut at zero would be a
/// row with nothing on it and a loop that never reached the end of the line, so
/// a character wider than the whole panel overhangs rather than stopping the
/// wrap.
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

/// How many bytes the first character of `text` takes, and zero for a text with
/// no first character.
fn first_character(text: &str) -> usize {
    text.chars().next().map_or(0, char::len_utf8)
}

/// What a line comes back as at a width: how many rows, what is on them, and
/// what is deliberately not.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{Line, first_character, folded, rows, shape, wrapped};
    use crate::ui::display_width;

    /// A width every line below is too long for, and narrow enough that the
    /// clock column is a visible share of it.
    const NARROW: usize = 18;

    /// Everything a byte-keeping wrapper has to survive: several words, a path
    /// with nowhere to break in it, whitespace at each end, two spaces in a row,
    /// and characters two columns wide.
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

    /// A line that fits is the one row it was, byte for byte.
    #[test]
    fn a_line_inside_the_width_is_one_row() {
        assert_eq!(wrapped("# The engine", 20), ["# The engine"]);
        assert_eq!(wrapped("exactly twelve", 14), ["exactly twelve"]);
    }

    /// The blank line survives: it is what a paragraph break in a document is
    /// made of.
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

    /// The whole of the reason `folded` exists: whatever the width and whatever
    /// is in the text, the rows are the text again, byte for byte. A caller
    /// counting an offset through them counts every byte the reader typed,
    /// including the spaces the rows were broken at.
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

    /// The field fills where the panel breaks at words, whatever is in the draft
    /// and whatever the width: the two answers are the same text, cut in
    /// different places. Every row of the field is as full as the width allows,
    /// which is the property the panel's rows do not have and a caret needs.
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

    /// The draft that put the rule here, a word and then a run of `h`s, at the
    /// two lengths that matter: one that fits a row of its own and one that does
    /// not. The panel pushes the run down in the first case and only there,
    /// which is the step the field must not have — the field fills both, so the
    /// row the run outgrows is the row it goes on filling.
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

    /// Columns, not characters: eight characters of `日本語 and` would fit a
    /// width of eight if they were counted one apiece, but they draw in eleven
    /// cells, so the row is the three that draw in six, the space, and the one
    /// letter the last column has room for.
    #[test]
    fn folded_counts_the_columns_a_character_draws_in() {
        assert_eq!(folded("日本語 and more", 8), ["日本語 a", "nd more"]);
    }

    /// A field nobody has measured yet wraps nothing, and a draft with nothing
    /// in it is still one row — there has to be a row for the cursor to sit on.
    #[test]
    fn folded_at_a_width_of_zero_is_the_row_it_went_in_as() {
        let long = "It walks the tree and writes what it finds.";

        assert_eq!(folded(long, 0), [long]);
        assert_eq!(folded("", 0), [""]);
        assert_eq!(folded("", 20), [""]);
    }

    /// No row holds half a character, at any width: the narrow widths are where
    /// a wrapper slicing by bytes would panic, and the rows still add back up to
    /// the text.
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

    /// A filled row is a row that fits: the cut is the width itself, so no row of
    /// a folded draft is ever wider than the field it was folded to — including
    /// the rows that end in the space between two words, which is a cell of the
    /// draft like any other rather than something to hang over the edge.
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

    /// A run of spaces is kept whole, wherever the breaks fall in it: this is
    /// the case `wrapped` throws away, and a draft that lost one of two spaces
    /// would put the cursor a cell out for the rest of the line.
    #[test]
    fn folded_keeps_a_run_of_spaces_that_the_panels_wrapper_swallows() {
        assert_eq!(folded("a  b", 2), ["a ", " b"]);
        assert_eq!(wrapped("a  b", 2), ["a", "b"]);
    }

    /// The degenerate width, which the loop has to terminate at rather than
    /// breaking off empty rows for ever — and with no `trim_start` to fall back
    /// on, the cut is the only thing moving it along.
    #[test]
    fn folded_at_a_width_of_one_still_gets_to_the_end_of_the_draft() {
        assert_eq!(folded("ab cd", 1), ["a", "b", " ", "c", "d"]);
        assert_eq!(folded("日本語", 1), ["日", "本", "語"]);
    }
}
