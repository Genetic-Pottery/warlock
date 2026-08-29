//! Breaking one line of a file into the rows a panel of some width can draw it
//! in.
//!
//! The panel's other card is an account, and an account is not wrapped: one
//! thing that happened is one row, so a line too long for the width is cut with
//! an ellipsis and the count of rows on screen stays the count of things the
//! pass was seen doing (see [`draw_panel`](crate::ui) and the module docs there).
//! A document is the opposite kind of thing. Nothing about a file's line numbers
//! is on screen, nothing is counting them, and the far end of a sentence is the
//! half that says what the sentence was about — so a document's line is drawn in
//! as many rows as it needs, and the reader gets the whole of it without a
//! second axis to move along.
//!
//! That is the whole of the choice: a `WARLOCK.md` is prose in long lines, and
//! the alternative to wrapping it is a horizontal scroll — a mode, a key pair
//! and an offset, all so a reader can shunt a paragraph left and right to read
//! it. Warlock shows a file so the reader can judge a document against the
//! directory it describes; a document they have to steer is a document they will
//! not read.
//!
//! ## What a break costs, and where it is put
//!
//! [`wrapped`] breaks at a space where there is one inside the width and mid-word
//! where there is not, so a long path or an unbroken run of `x`s still gets
//! across rather than stopping the wrap dead. Columns are counted the way
//! [`truncated`](crate::ui) counts them and the way the backend will lay the row
//! out, so a row that fits here fits there.
//!
//! Continuation rows start flush left, with nothing put in front of them: no
//! hanging indent, no marker, no ellipsis. A row of a wrapped line is the file's
//! own text and nothing else, exactly as an unwrapped one is, and a reader
//! looking at the panel is looking at bytes that are in the file.

use crate::ui::display_width;

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
    use super::wrapped;
    use crate::ui::display_width;

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
}
