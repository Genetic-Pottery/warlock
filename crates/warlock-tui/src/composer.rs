//! The multi-line draft at the foot of the panel's column, and the two pure
//! functions saying what a keystroke and what a pasted block do to it.
//!
//! Keys and pastes are two functions with two return types on purpose.
//! [`Pasted`] has one variant, so no block of text the terminal hands over —
//! however many newlines are in it — can come back saying "submit" or "leave";
//! folded into [`compose_for`] that would be a rule every call site has to
//! remember rather than one the type keeps.
//!
//! Every warlock command is a single letter, so while this field holds the
//! keyboard the loop consults it *instead of* `input::action_for` rather than as
//! well: `p` is the letter p. Nothing here reads a terminal or a clock — the
//! width and the muting are facts about the session, told in from outside once a
//! round, which is what keeps both functions pure functions of a key and a
//! composer.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::ui::display_width;
use crate::wrap::folded;

/// A cap rather than a field, because the panel above loses exactly the rows the
/// composer takes: a draft allowed to grow without limit would eat the account
/// it is being typed next to. Past it the draft scrolls within the rows it has.
pub const COMPOSER_MAX_ROWS: u16 = 6;

// How many rows the window keeps above the cursor's row while it can. Two rather
// than none, because a window that started at the cursor's row would scroll at
// every Down and one that ended there at every Up. A fixed margin rather than
// the least scroll that would do, because `window` keeps no previous window to
// be least against — it is recomputed from scratch every frame.
const MARGIN: usize = 2;

// The modifiers that make a character a command rather than something somebody
// typed. Shift is deliberately absent: an upper-case letter arrives with it on
// some terminals and without it on others, and either way it is text. Ctrl-C is
// in here, and the loop above has already had its chance at it.
const CHORD: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::HYPER)
    .union(KeyModifiers::META);

/// The invariant every value in this module holds by construction: `cursor` is
/// a `char` boundary of `draft` and at most its length. [`Composer::at`] is the
/// only road in from outside and it panics rather than clamps.
///
/// `width` and `muted` are facts about the session rather than about the draft,
/// told in once a round by whoever is about to draw. Both take part in
/// [`PartialEq`] and [`Hash`] with everything else, so every value built here
/// must carry the incoming pair through untouched — a keystroke that dropped
/// one would read as a redraw to any whole-value comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Composer {
    draft: String,
    /// A byte offset rather than a row and a column, so it goes on meaning the
    /// same place when the terminal is resized and the draft re-flows under it.
    cursor: usize,
    /// Zero until something draws — a column nobody has measured, which folds
    /// every line to one row. The loop draws before it waits for a key, so no
    /// keystroke is answered in that state.
    width: u16,
    muted: bool,
}

impl Composer {
    #[must_use]
    pub fn new(draft: impl Into<String>) -> Self {
        let draft = draft.into();
        let cursor = draft.len();

        Self {
            draft,
            cursor,
            width: 0,
            muted: false,
        }
    }

    /// # Panics
    ///
    /// If `offset` is past the end of the draft or falls inside a character. A
    /// panic rather than a clamp because there is no production caller: the
    /// values this module builds hold the invariant by construction and do not
    /// come through here, so everything reaching it is a test with a wrong
    /// expectation, which should be loud rather than quietly moved.
    #[must_use]
    pub fn at(mut self, offset: usize) -> Self {
        assert!(
            offset <= self.draft.len(),
            "cursor offset {offset} is past the end of {:?}",
            self.draft
        );
        assert!(
            self.draft.is_char_boundary(offset),
            "cursor offset {offset} is inside a character of {:?}",
            self.draft
        );
        self.cursor = offset;

        self
    }

    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Told rather than measured, because the frame is cut somewhere else. It is
    /// on the value at all because Home, End, Up and Down move by *display* row,
    /// and which row an offset is on has no answer until something has said how
    /// wide the field is — carrying it here is what leaves [`compose_for`] a
    /// function of a key and a composer.
    ///
    /// Nothing about the draft or the cursor moves: the same bytes re-flow into
    /// different rows and the offset goes on meaning the byte it always meant.
    pub const fn set_width(&mut self, width: u16) {
        self.width = width;
    }

    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Told every round rather than at the keystrokes that change it, so a turn
    /// that ended in any of its ways, or a run that ended in any of its, leaves
    /// a live field behind without each of those ways having to say so.
    ///
    /// The draft does not move: muting is about which keys the field hears, not
    /// about what somebody has written.
    pub const fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    #[must_use]
    pub const fn is_muted(&self) -> bool {
        self.muted
    }

    #[must_use]
    pub fn draft(&self) -> &str {
        &self.draft
    }

    #[must_use]
    pub fn is_submittable(&self) -> bool {
        !self.draft.trim().is_empty()
    }

    #[must_use]
    pub fn height(&self, width: u16) -> u16 {
        // At most `COMPOSER_MAX_ROWS`, and the row count is at least one, so
        // this never truncates and never comes back zero.
        u16::try_from(self.rows(width).len())
            .unwrap_or(COMPOSER_MAX_ROWS)
            .min(COMPOSER_MAX_ROWS)
    }

    /// The rows and the caret come back together as one [`ComposerWindow`], so
    /// `Composer::place` is applied once and a set of rows that disagrees with
    /// the caret drawn on them cannot be built. `height` is the rows the border
    /// actually left, which can be fewer than [`Composer::height`] asked for.
    ///
    /// The first row on screen is a pure function of the draft, the cursor, the
    /// width and the height, with nothing remembered between frames:
    ///
    /// ```text
    /// first = clamp(cursor row - MARGIN, 0, rows - height)
    /// ```
    ///
    /// So there is no scroll offset on the value and nothing to reset when a
    /// long draft is backspaced short again. A `height` of zero is no rows and a
    /// caret row of zero, which is the one window whose caret is not a row of
    /// its own rows.
    #[must_use]
    pub fn window(&self, width: u16, height: u16) -> ComposerWindow {
        let (row, column) = self.place(self.cursor, width);
        let rows = self.rows(width);
        let height = usize::from(height);
        if height == 0 {
            return ComposerWindow {
                rows: Vec::new(),
                row: 0,
                column,
            };
        }

        let first = row
            .saturating_sub(MARGIN)
            // Never past the last window there is rows for...
            .min(rows.len().saturating_sub(height))
            // ...and never so far up that the cursor's own row falls off the
            // bottom, which the margin alone allows only in a field shorter
            // than the margin. `row` is a row of `rows`, so this floor is at
            // most the ceiling above it and the two never cross.
            .max((row + 1).saturating_sub(height));

        ComposerWindow {
            rows: rows.into_iter().skip(first).take(height).collect(),
            // `first` is at most `row`, both when the margin decided it and
            // when either bound did, so this is a row of the window.
            row: row - first,
            column,
        }
    }

    fn rows(&self, width: u16) -> Vec<String> {
        self.placed_rows(width)
            .into_iter()
            .map(|(_, row)| row)
            .collect()
    }

    // Every row the draft draws as, each with the byte offset its first
    // character is at, and the only place the arithmetic between bytes and rows
    // is done. It works because `folded` keeps every byte: the rows of one line
    // join back up to that line, so a row starts where the row before it ended.
    // `wrapped` drops the character it breaks at, and every offset past the
    // first wrap would then be wrong by one byte per wrap.
    fn placed_rows(&self, width: u16) -> Vec<(usize, String)> {
        let mut placed = Vec::new();
        let mut offset = 0;
        for line in self.draft.split('\n') {
            for row in folded(line, usize::from(width)) {
                let start = offset;
                offset += row.len();
                placed.push((start, row));
            }
            // The `\n` that `split` took out from between this line and the
            // next. Past the end of the draft after the last line, where the
            // loop has stopped and nothing reads it.
            offset += 1;
        }

        placed
    }

    // Where a byte offset is drawn: its row, and the column of that row in cells
    // rather than characters, so a wide character is two columns along.
    //
    // An offset at a soft wrap has two cells it could be drawn in — past the end
    // of the row above, and column zero of the row below — and `rfind` takes the
    // lower one. `offset_at` keeps the same choice, which is what makes the pair
    // round-trip and stops End on a wrapped row parking the cursor in a cell the
    // next character typed would not appear in. A newline is not a wrap in this
    // sense: its own byte lies between the two rows and belongs to the upper one.
    fn place(&self, offset: usize, width: u16) -> (usize, usize) {
        let placed = self.placed_rows(width);
        let (row, start, text) = placed
            .iter()
            .enumerate()
            .rfind(|(_, (start, _))| *start <= offset)
            .map(|(row, (start, text))| (row, *start, text.as_str()))
            .expect("the first row starts at zero, so a row starts at or before every offset");

        (row, display_width(&text[..offset - start]))
    }

    // The inverse of `place`, and total on purpose: a `row` past the last is
    // answered by the last row and a `column` past the end of a row by the
    // offset at its end. That is what lets End hand in `usize::MAX` and lets the
    // row-wise keys hand in a row they have not range-checked.
    fn offset_at(&self, row: usize, column: usize, width: u16) -> usize {
        let placed = self.placed_rows(width);
        let row = row.min(placed.len().saturating_sub(1));
        let (start, text) = placed
            .get(row)
            .expect("a draft always has at least one row to sit a cursor on");

        let mut offset = *start;
        let mut taken = 0;
        for (index, character) in text.char_indices() {
            let next = index + character.len_utf8();
            let cells = display_width(&text[index..next]);
            if taken + cells > column {
                // The target column is one of this character's own cells, so
                // the offset is the one in front of it.
                break;
            }
            taken += cells;
            offset = start + next;
        }

        offset
    }
}

/// The rows and the caret come back together because they are answers to the
/// same window: worked out separately, a caret could name a row that had been
/// scrolled off the rows beside it. [`Composer::window`] is the only thing that
/// builds one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComposerWindow {
    pub rows: Vec<String>,
    /// An index into *these* rows and not into the draft's, so the drawing adds
    /// nothing to it and cannot add the scroll twice.
    pub row: usize,
    /// In cells rather than characters, and straight from `Composer::place`,
    /// so it keeps that map's row-break rule. It can be one cell past the end of
    /// its row, which is where a cursor at the end of the draft sits.
    pub column: usize,
}

/// There is deliberately no variant for "the key meant nothing": a key that
/// means nothing leaves the composer exactly where it was, which is
/// [`Composed::Typing`] with the same draft in it.
///
/// [`Composed::Submit`] carries no text of its own, so there is one copy of what
/// was typed and no way for a submission to disagree with the field it came
/// from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Composed {
    Typing(Composer),
    Leave,
    Submit,
}

/// Only presses count. Crossterm reports releases and auto-repeats on some
/// platforms and not on others, and a release acted on here would type the
/// release of the very key that moved the focus into the field.
///
/// Every key not named below leaves the composer as it was, the tree's own
/// bindings included: the loop consults this *instead of* `action_for` rather
/// than as well, so while the field has the keyboard `j`, `p` and `r` are
/// letters somebody is typing.
#[must_use]
pub fn compose_for(key: KeyEvent, composer: &Composer) -> Composed {
    if key.kind != KeyEventKind::Press {
        return Composed::Typing(composer.clone());
    }

    let unchanged = || Composed::Typing(composer.clone());
    // The three builders below are written literally rather than through
    // `Composer::at`, and each holds that constructor's invariant by
    // construction: `composer.cursor` is a boundary of the draft, so the halves
    // it splits into are whole strings and the offset after a whole character of
    // the result is a boundary of the result. Every one of them carries `width`
    // and `muted` through untouched — this function is neither a redraw nor
    // where a turn starts or ends.
    let inserted = |character: char| {
        let cursor = composer.cursor;
        let mut draft = String::with_capacity(composer.draft.len() + character.len_utf8());
        draft.push_str(&composer.draft[..cursor]);
        draft.push(character);
        draft.push_str(&composer.draft[cursor..]);

        Composed::Typing(Composer {
            cursor: cursor + character.len_utf8(),
            draft,
            width: composer.width,
            muted: composer.muted,
        })
    };
    // `start` and `end` are the two ends of a whole `char`, which is what makes
    // this deletion by character rather than by byte: half a character left
    // behind would not be a `String` at all. Which character goes is the only
    // thing Backspace and Delete disagree about.
    let removed = |start: usize, end: usize| {
        let mut draft = String::with_capacity(composer.draft.len() - (end - start));
        draft.push_str(&composer.draft[..start]);
        draft.push_str(&composer.draft[end..]);

        Composed::Typing(Composer {
            draft,
            cursor: start,
            width: composer.width,
            muted: composer.muted,
        })
    };
    // Every offset handed to this comes off `char_indices`, `place` or
    // `offset_at`, all three of which answer with boundaries of this draft.
    let moved = |cursor: usize| {
        Composed::Typing(Composer {
            draft: composer.draft.clone(),
            cursor,
            width: composer.width,
            muted: composer.muted,
        })
    };
    // A closure rather than a value because working it out folds the whole draft
    // into rows, and every key but the four row-wise ones never asks.
    let placed = || composer.place(composer.cursor, composer.width);

    match key.code {
        // Before the plain Enter below it, which is the point of the pair: the
        // modifier is what tells a new line from a submission.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => inserted('\n'),
        // Shift+Enter arrives here too, and means Enter: it is not a keystroke
        // of its own, because half the terminals in use never report it.
        KeyCode::Enter => {
            if composer.is_submittable() {
                Composed::Submit
            } else {
                unchanged()
            }
        }
        KeyCode::Esc => Composed::Leave,
        // Backspace at offset zero is still `Typing`: one press past the start
        // of the draft is a typo, and Esc is the only key that hands the
        // keyboard back.
        KeyCode::Backspace => composer.draft[..composer.cursor]
            .chars()
            .next_back()
            .map_or_else(unchanged, |character| {
                removed(composer.cursor - character.len_utf8(), composer.cursor)
            }),
        KeyCode::Delete => composer.draft[composer.cursor..]
            .chars()
            .next()
            .map_or_else(unchanged, |character| {
                removed(composer.cursor, composer.cursor + character.len_utf8())
            }),
        // The six below move the cursor and change no byte. Left and Right step
        // one `char`, not one byte — a byte of an accent is not a place in the
        // draft — and stop dead at the ends rather than wrapping round.
        KeyCode::Left => moved(
            composer.draft[..composer.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(start, _)| start),
        ),
        KeyCode::Right => moved(
            composer.draft[composer.cursor..]
                .chars()
                .next()
                .map_or(composer.cursor, |character| {
                    composer.cursor + character.len_utf8()
                }),
        ),
        // The ends of the *display* row. On a soft-wrapped row the end is the
        // break offset, which `place` draws at column zero of the row below, so
        // End there looks like a move down and Home straight after comes back to
        // the same offset. That is the row-break rule holding, not failing.
        KeyCode::Home => moved(composer.offset_at(placed().0, 0, composer.width)),
        KeyCode::End => moved(composer.offset_at(placed().0, usize::MAX, composer.width)),
        // One display row, holding the column where the destination row is long
        // enough for it. There is no row past either end to sit on, so Up on the
        // first row and Down on the last leave the cursor where it is rather
        // than clamping it to the row it is already on.
        KeyCode::Up => {
            let (row, column) = placed();
            match row.checked_sub(1) {
                Some(above) => moved(composer.offset_at(above, column, composer.width)),
                None => unchanged(),
            }
        }
        KeyCode::Down => {
            let (row, column) = placed();
            let below = row + 1;
            if below < composer.placed_rows(composer.width).len() {
                moved(composer.offset_at(below, column, composer.width))
            } else {
                unchanged()
            }
        }
        // A chord is a command somebody sent, not a character somebody typed,
        // and control characters are not text however they arrived — Ctrl-C
        // among them, which the loop above has already had its chance at.
        KeyCode::Char(character) if !key.modifiers.intersects(CHORD) && !character.is_control() => {
            inserted(character)
        }
        _ => unchanged(),
    }
}

/// One variant, and that is the promise being kept: a paste cannot submit and
/// cannot hand the keyboard back, however many newlines the block carries.
/// Pasting three lines used to send the first as a question and lose the other
/// two behind the mute that turn put up, and a negative promise held by a return
/// type is one no call site can forget.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Pasted {
    Typing(Composer),
}

/// The block is appended at the end of the draft rather than going in at
/// [`Composer::cursor`] where a keystroke does, and the cursor snaps after it.
/// That is the one place a paste and a keystroke disagree; pasting at the cursor
/// is a slice of its own and this is left as it was until that lands.
///
/// A muted field takes nothing, which is the one gate [`compose_for`] leaves to
/// its caller and this does not: a paste is a single arrival carrying however
/// much was copied, so a gate missed at a call site would land the lot.
#[must_use]
pub fn paste_for(text: &str, composer: &Composer) -> Pasted {
    if composer.muted {
        return Pasted::Typing(composer.clone());
    }

    let mut draft = composer.draft.clone();
    draft.push_str(text);

    // The width and the flag come through untouched, as they do at a keystroke.
    Pasted::Typing(Composer {
        cursor: draft.len(),
        draft,
        width: composer.width,
        muted: composer.muted,
    })
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use super::{COMPOSER_MAX_ROWS, Composed, Composer, Pasted, compose_for, paste_for};

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn alt_enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)
    }

    fn composer(draft: &str) -> Composer {
        Composer::new(draft)
    }

    fn drawn(draft: &str, width: u16) -> Composer {
        let mut composer = Composer::new(draft);
        composer.set_width(width);

        composer
    }

    fn window_rows(composer: &Composer, width: u16) -> Vec<String> {
        composer.window(width, COMPOSER_MAX_ROWS).rows
    }

    fn numbered(rows: usize) -> String {
        (0..rows)
            .map(|row| format!("row {row}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn at_row(draft: &str, row: usize) -> Composer {
        let offset = draft
            .split('\n')
            .take(row)
            // The `\n` `split` took out from the end of each line.
            .map(|line| line.len() + 1)
            .sum();

        drawn(draft, 40).at(offset)
    }

    fn scrolled(composer: &Composer, height: u16) -> (usize, usize) {
        let window = composer.window(40, height);
        let first: usize = window
            .rows
            .first()
            .expect("a window with a height in it has rows")
            .strip_prefix("row ")
            .expect("every row of the draft names its own number")
            .parse()
            .expect("every row of the draft names its own number");

        (first, first + window.row)
    }

    fn cursor_after(key: KeyEvent, composer: &Composer) -> usize {
        let next = after(key, composer);

        assert_eq!(
            next.draft(),
            composer.draft(),
            "{key:?} changed the draft it moved through"
        );
        assert_eq!(next.width(), composer.width(), "{key:?} changed the width");
        assert_eq!(
            next.is_muted(),
            composer.is_muted(),
            "{key:?} changed the muting"
        );

        next.cursor()
    }

    fn after(key: KeyEvent, composer: &Composer) -> Composer {
        match compose_for(key, composer) {
            Composed::Typing(next) => next,
            other => panic!("{key:?} should have kept the keyboard, and gave {other:?}"),
        }
    }

    fn pasted(text: &str, composer: &Composer) -> Composer {
        let Pasted::Typing(next) = paste_for(text, composer);

        next
    }

    const BINDINGS: [KeyCode; 18] = [
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('g'),
        KeyCode::Char('G'),
        KeyCode::Char(' '),
        KeyCode::Char('o'),
        KeyCode::Char('f'),
        KeyCode::Char('p'),
        KeyCode::Char('r'),
        KeyCode::Char('m'),
        KeyCode::Char('s'),
        KeyCode::Char('v'),
        KeyCode::Char('e'),
        KeyCode::Tab,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Up,
        KeyCode::Down,
    ];

    const MOVERS: [KeyCode; 6] = [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Home,
        KeyCode::End,
    ];

    fn character(code: KeyCode) -> Option<char> {
        match code {
            KeyCode::Char(character) => Some(character),
            _ => None,
        }
    }

    #[test]
    fn a_fresh_composer_is_empty_and_has_nothing_to_submit() {
        let fresh = Composer::default();

        assert_eq!(fresh.draft(), "");
        assert!(!fresh.is_submittable());
        assert_eq!(fresh, composer(""));
    }

    #[test]
    fn a_new_composer_has_its_cursor_at_the_end_of_the_draft() {
        // Where somebody who has just typed the draft would be, which is what
        // keeps every whole-value comparison written against a draft alone
        // saying what it said before there was a cursor at all.
        assert_eq!(Composer::default().cursor(), 0);
        assert_eq!(composer("").cursor(), 0);
        assert_eq!(composer("web").cursor(), 3);
        assert_eq!(composer("one\ntwo").cursor(), 7);
        // Bytes, not characters: `é` is two of them.
        assert_eq!(composer("wéb").cursor(), 4);
    }

    #[test]
    fn at_puts_the_cursor_where_it_is_asked_to() {
        assert_eq!(composer("hello").at(3).cursor(), 3);
        assert_eq!(composer("hello").at(0).cursor(), 0);
        assert_eq!(
            composer("hello").at(5),
            composer("hello"),
            "the end is where `new` already put it"
        );
        // Every boundary of a multi-byte draft, and only the boundaries: `é`
        // occupies bytes 1 and 2, so 2 is not one of them.
        let draft = "wéb";
        for offset in [0, 1, 3, 4] {
            assert_eq!(composer(draft).at(offset).cursor(), offset);
        }
    }

    #[test]
    fn the_cursor_is_part_of_the_value() {
        // Two composers holding the same characters at different insertion
        // points are two different values — which is why every draft-changing
        // arm has to land the cursor in the same place.
        assert_ne!(composer("hello").at(3), composer("hello"));
        assert_eq!(composer("hello").at(3), composer("hello").at(3));
    }

    #[test]
    #[should_panic(expected = "cursor offset 6 is past the end of \"hello\"")]
    fn at_past_the_end_of_the_draft_panics_rather_than_clamping() {
        // A test asking for an offset the draft does not have is a wrong
        // expectation, and the one slice whose job is offset arithmetic is the
        // slice where that should be loud.
        let _ = composer("hello").at(6);
    }

    #[test]
    #[should_panic(expected = "cursor offset 2 is inside a character of \"wéb\"")]
    fn at_inside_a_character_panics_rather_than_clamping() {
        // `é` is bytes 1 and 2, so 2 is halfway through it: an offset there is
        // not a place in this draft at all.
        let _ = composer("wéb").at(2);
    }

    #[test]
    fn a_paste_still_lands_at_the_end_however_far_the_cursor_has_moved() {
        // Typing is at the cursor now; a paste deliberately is not yet. It goes
        // in at the end of the draft and takes the cursor with it, so there is
        // one insertion rule for blocks in the build at a time rather than two
        // that disagree — and, since the cursor is part of the value, a pasted
        // draft carrying a stale offset would be a value nothing else in the
        // crate compares equal to.
        let from = composer("one\ntwo").at(2);
        let next = pasted("and\nmore", &from);

        assert_eq!(next.draft(), "one\ntwoand\nmore");
        assert_eq!(
            next.cursor(),
            next.draft().len(),
            "a paste should have left the cursor at the end of {:?}",
            next.draft()
        );
    }

    #[test]
    fn a_key_that_changes_nothing_moves_the_cursor_nowhere() {
        // The other half: the incoming offset comes through untouched, so a
        // key this field does not have is not a key that quietly re-homes the
        // insertion point.
        let before = composer("one\ntwo").at(2);

        for code in [KeyCode::Insert, KeyCode::BackTab] {
            assert_eq!(
                compose_for(press(code), &before),
                Composed::Typing(before.clone()),
                "{code:?} should have changed nothing at all"
            );
        }

        let mut muted = before.clone();
        muted.set_muted(true);

        assert_eq!(
            pasted("and\nmore", &muted),
            muted,
            "a muted field takes no paste, and no paste moves its cursor"
        );
    }

    #[test]
    fn a_composer_is_live_until_the_loop_says_otherwise() {
        // Muting is a fact about a turn in flight, and a session that has never
        // asked anything has never had one: the field a session starts on hears
        // every key it is given.
        let mut current = composer("web");

        assert!(!Composer::default().is_muted());
        assert!(!current.is_muted());

        current.set_muted(true);
        assert!(current.is_muted());
        current.set_muted(false);
        assert!(!current.is_muted());
    }

    #[test]
    fn muting_moves_no_character_and_changes_no_row() {
        // What somebody typed is worth more than the keystroke that stopped
        // them typing it, and a turn is not even a keystroke: the draft is the
        // same string, at the same height, in the same rows.
        let live = composer("It walks the tree and writes what it finds.");
        let mut muted = live.clone();
        muted.set_muted(true);

        assert_eq!(muted.draft(), live.draft());
        assert_eq!(muted.is_submittable(), live.is_submittable());
        assert_eq!(muted.height(18), live.height(18));
        assert_eq!(window_rows(&muted, 18), window_rows(&live, 18));
    }

    #[test]
    fn a_muted_field_carries_its_muting_through_a_keystroke() {
        // This function is not where a turn starts or ends. It is never asked
        // anything while the flag is up — the loop's gate sees to that — and if
        // it is, what comes back is the same field, still muted, rather than a
        // field that quietly went live between two keys.
        let mut before = composer("we");
        before.set_muted(true);

        let next = after(press(KeyCode::Char('b')), &before);

        assert_eq!(next.draft(), "web");
        assert!(next.is_muted(), "a keystroke unmuted the field");
    }

    #[test]
    fn printable_characters_append_in_the_order_they_are_typed() {
        let mut current = composer("");
        for character in "read the engine".chars() {
            current = after(press(KeyCode::Char(character)), &current);
        }

        assert_eq!(current, composer("read the engine"));
    }

    #[test]
    fn characters_append_exactly_as_typed_and_are_not_folded_or_refused() {
        // Nothing here judges what is being written: the draft is prose, and a
        // field that silently dropped a capital or a comma would leave somebody
        // typing at a field that ignores them.
        let mut current = composer("");
        for character in "Why, exactly, 9 passes?".chars() {
            current = after(press(KeyCode::Char(character)), &current);
        }

        assert_eq!(current.draft(), "Why, exactly, 9 passes?");
    }

    #[test]
    fn a_character_goes_in_where_the_cursor_is_and_the_cursor_follows_it() {
        // The three places there are: in front of everything, in the middle of
        // it, and at the end where `Composer::new` puts the cursor and where
        // every test written before there was a cursor types from. Whole-value
        // comparisons, because the draft and the offset have to agree — a
        // character typed at offset 3 that left the cursor at the end would put
        // the next one somewhere nobody asked for.
        let before = composer("web").at(0);
        assert_eq!(
            after(press(KeyCode::Char('x')), &before),
            composer("xweb").at(1)
        );

        let before = composer("web").at(1);
        assert_eq!(
            after(press(KeyCode::Char('x')), &before),
            composer("wxeb").at(2)
        );

        let before = composer("web");
        assert_eq!(after(press(KeyCode::Char('x')), &before), composer("webx"));
    }

    #[test]
    fn typing_from_the_middle_goes_on_from_where_the_last_character_landed() {
        // The cursor is left after what was typed, so a word typed into the
        // middle of a draft comes out as that word rather than backwards.
        let mut current = composer("read engine").at(5);
        for character in "the ".chars() {
            current = after(press(KeyCode::Char(character)), &current);
        }

        assert_eq!(current, composer("read the engine").at(9));
    }

    #[test]
    fn a_character_typed_into_a_multi_byte_draft_goes_in_whole() {
        // From every boundary of a draft with an accent in it and of one with
        // an emoji in it: an insertion done by byte rather than by `char` would
        // split one of them, and the offset it left behind would be inside a
        // character rather than in front of one.
        for draft in ["wéb", "web 🜁 fire"] {
            for offset in (0..=draft.len()).filter(|at| draft.is_char_boundary(*at)) {
                let next = after(press(KeyCode::Char('x')), &composer(draft).at(offset));

                assert_eq!(
                    next.draft(),
                    format!("{}x{}", &draft[..offset], &draft[offset..]),
                    "typing at offset {offset} of {draft:?} should splice the character in"
                );
                assert!(
                    std::str::from_utf8(next.draft().as_bytes()).is_ok(),
                    "typing at offset {offset} of {draft:?} left a draft that is not UTF-8"
                );
                assert_eq!(
                    next.draft().chars().count(),
                    draft.chars().count() + 1,
                    "typing at offset {offset} of {draft:?} should add one character"
                );
                assert!(
                    next.draft().is_char_boundary(next.cursor()),
                    "typing at offset {offset} of {draft:?} left the cursor inside a character"
                );
                assert_eq!(
                    next.cursor(),
                    offset + 1,
                    "the cursor should be after the `x` typed at offset {offset} of {draft:?}"
                );
            }
        }
    }

    #[test]
    fn a_multi_byte_character_typed_moves_the_cursor_its_own_width() {
        // The step is the character's own bytes, not one: `é` is two of them
        // and `🜁` is four, and the cursor after each is where the next one
        // goes.
        assert_eq!(
            after(press(KeyCode::Char('é')), &composer("wb").at(1)),
            composer("wéb").at(3)
        );
        assert_eq!(
            after(press(KeyCode::Char('🜁')), &composer("ab").at(1)),
            composer("a🜁b").at(5)
        );
    }

    #[test]
    fn backspace_takes_back_one_character_at_a_time() {
        // Whole values, because the offset has to follow the character out: a
        // Backspace that took the right byte and left the cursor where it was
        // would put the next keystroke a character too far along.
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("web")),
            composer("we")
        );
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("w")),
            composer("")
        );
        // And from the middle, which is the whole point of there being a
        // cursor: what goes is the character in front of it, not the last one
        // typed, and the cursor lands where that character began.
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("web").at(1)),
            composer("eb").at(0)
        );
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("web").at(2)),
            composer("wb").at(1)
        );
    }

    #[test]
    fn backspace_takes_back_a_character_and_not_a_byte() {
        // A delete that took one byte of a multi-byte character would leave the
        // buffer holding half of one.
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("wéb")),
            composer("wé")
        );
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("wé")),
            composer("w")
        );

        // From every boundary of a draft with an accent in it and of one with
        // an emoji in it, since a deletion done by byte would split one of them
        // at some offsets and not at others.
        for draft in ["wéb", "web 🜁 fire"] {
            for offset in (0..=draft.len()).filter(|at| draft.is_char_boundary(*at)) {
                let next = after(press(KeyCode::Backspace), &composer(draft).at(offset));
                let gone = draft[..offset].chars().next_back();
                let start = offset - gone.map_or(0, char::len_utf8);

                assert_eq!(
                    next.draft(),
                    format!("{}{}", &draft[..start], &draft[offset..]),
                    "Backspace at offset {offset} of {draft:?} should take the whole character before it"
                );
                assert!(
                    std::str::from_utf8(next.draft().as_bytes()).is_ok(),
                    "Backspace at offset {offset} of {draft:?} left a draft that is not UTF-8"
                );
                assert_eq!(
                    next.draft().chars().count(),
                    draft.chars().count() - usize::from(gone.is_some()),
                    "Backspace at offset {offset} of {draft:?} should take one character or none"
                );
                assert!(
                    next.draft().is_char_boundary(next.cursor()),
                    "Backspace at offset {offset} of {draft:?} left the cursor inside a character"
                );
                assert_eq!(
                    next.cursor(),
                    start,
                    "the cursor should be where the character taken at offset {offset} of {draft:?} began"
                );
            }
        }
    }

    #[test]
    fn backspace_takes_a_newline_back_like_any_other_character() {
        // One character, and the two lines it was between join up — from the
        // end of the draft and from the middle of it alike.
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("one\n")),
            composer("one")
        );
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("one\ntwo").at(4)),
            composer("onetwo").at(3)
        );
    }

    #[test]
    fn backspace_on_an_empty_draft_changes_nothing_and_keeps_the_keyboard() {
        // One press past the start is a typo, not a departure: Esc is the key
        // that hands the keyboard back, and it is the only one.
        let empty = composer("");

        assert_eq!(
            compose_for(press(KeyCode::Backspace), &empty),
            Composed::Typing(empty.clone())
        );

        // And at the front of a draft that does have something in it, which is
        // the same press with somewhere for the cursor to have come from: the
        // draft is untouched and the keyboard stays here.
        let front = composer("web").at(0);

        assert_eq!(
            compose_for(press(KeyCode::Backspace), &front),
            Composed::Typing(front.clone())
        );
    }

    #[test]
    fn delete_takes_the_character_after_the_cursor_and_leaves_the_cursor_be() {
        // Backspace's mirror: the character in front goes, and the insertion
        // point does not move, because what was behind it has not shifted.
        assert_eq!(
            after(press(KeyCode::Delete), &composer("web").at(0)),
            composer("eb").at(0)
        );
        assert_eq!(
            after(press(KeyCode::Delete), &composer("web").at(1)),
            composer("wb").at(1)
        );
        // A newline is one character to Delete too, so the line below joins on.
        assert_eq!(
            after(press(KeyCode::Delete), &composer("one\ntwo").at(3)),
            composer("onetwo").at(3)
        );
    }

    #[test]
    fn delete_at_the_end_of_the_draft_changes_nothing() {
        // There is no character in front of the cursor to take, so the press is
        // a keystroke and not a mistake: nothing moves, and nothing leaves.
        for draft in ["", "web", "one\ntwo"] {
            let before = composer(draft);

            assert_eq!(
                compose_for(press(KeyCode::Delete), &before),
                Composed::Typing(before.clone()),
                "Delete at the end of {draft:?} should have changed nothing"
            );
        }
    }

    #[test]
    fn delete_takes_a_character_and_not_a_byte() {
        // The same sweep Backspace gets, from the other side of the cursor: an
        // accent and an emoji, from every boundary there is.
        for draft in ["wéb", "web 🜁 fire"] {
            for offset in (0..=draft.len()).filter(|at| draft.is_char_boundary(*at)) {
                let next = after(press(KeyCode::Delete), &composer(draft).at(offset));
                let gone = draft[offset..].chars().next();
                let end = offset + gone.map_or(0, char::len_utf8);

                assert_eq!(
                    next.draft(),
                    format!("{}{}", &draft[..offset], &draft[end..]),
                    "Delete at offset {offset} of {draft:?} should take the whole character after it"
                );
                assert!(
                    std::str::from_utf8(next.draft().as_bytes()).is_ok(),
                    "Delete at offset {offset} of {draft:?} left a draft that is not UTF-8"
                );
                assert_eq!(
                    next.draft().chars().count(),
                    draft.chars().count() - usize::from(gone.is_some()),
                    "Delete at offset {offset} of {draft:?} should take one character or none"
                );
                assert!(
                    next.draft().is_char_boundary(next.cursor()),
                    "Delete at offset {offset} of {draft:?} left the cursor inside a character"
                );
                assert_eq!(
                    next.cursor(),
                    offset,
                    "Delete at offset {offset} of {draft:?} should have left the cursor there"
                );
            }
        }
    }

    #[test]
    fn alt_enter_puts_a_newline_in() {
        let current = after(alt_enter(), &composer("first"));

        assert_eq!(current, composer("first\n"));
        assert_eq!(
            after(press(KeyCode::Char('x')), &current),
            composer("first\nx")
        );
    }

    #[test]
    fn alt_enter_puts_its_newline_in_where_the_cursor_is() {
        // A newline is a character like any other, so it goes in at the
        // insertion point rather than at the end: Alt+Enter in the middle of a
        // line breaks that line in two, which is what somebody pressing it
        // there is asking for.
        assert_eq!(alt_enter().modifiers, KeyModifiers::ALT);
        assert_eq!(
            after(alt_enter(), &composer("onetwo").at(3)),
            composer("one\ntwo").at(4)
        );
        assert_eq!(
            after(alt_enter(), &composer("two").at(0)),
            composer("\ntwo").at(1)
        );
        // And in a multi-byte draft it is still one byte in front of a whole
        // character rather than inside one.
        let next = after(alt_enter(), &composer("wéb").at(1));
        assert_eq!(next, composer("w\néb").at(2));
        assert!(next.draft().is_char_boundary(next.cursor()));
        // And it still submits nothing from anywhere, at the drafts Enter
        // itself would refuse: `after` panics on anything but `Typing`.
        for (draft, offset) in [("", 0), ("  ", 1), ("one\ntwo", 2)] {
            let _ = after(alt_enter(), &composer(draft).at(offset));
        }
    }

    #[test]
    fn alt_enter_never_submits_however_much_is_typed() {
        for draft in ["", "  ", "a paragraph worth of words"] {
            assert_eq!(
                compose_for(alt_enter(), &composer(draft)),
                Composed::Typing(composer(&format!("{draft}\n"))),
                "Alt+Enter should have made a new line in {draft:?}"
            );
        }
    }

    #[test]
    fn enter_submits_a_draft_with_something_in_it() {
        for draft in ["why", "  leading and trailing  ", "one\ntwo"] {
            assert_eq!(
                compose_for(press(KeyCode::Enter), &composer(draft)),
                Composed::Submit,
                "Enter should submit {draft:?}"
            );
        }
    }

    #[test]
    fn enter_submits_the_whole_draft_from_wherever_the_cursor_is() {
        // The insertion point says where the next character goes and nothing
        // else: Enter offers up everything that has been typed, so a reader who
        // has gone back to fix the first line does not send that line alone.
        // Every offset of a draft with two newlines in it, the two ends
        // included.
        let draft = "one\ntwo\nsix";

        for offset in (0..=draft.len()).filter(|at| draft.is_char_boundary(*at)) {
            assert_eq!(
                compose_for(press(KeyCode::Enter), &composer(draft).at(offset)),
                Composed::Submit,
                "Enter at offset {offset} of {draft:?} should submit the lot"
            );
        }
    }

    #[test]
    fn enter_on_an_empty_or_blank_draft_does_nothing_at_all() {
        // Not a refusal with a complaint on the footer: a submission with
        // nothing in it is a keystroke, not a mistake, so it leaves the draft
        // exactly as it was and says nothing. From every offset, because
        // whether there is anything here is a question about the buffer and
        // never about where in it the cursor is sitting.
        for draft in ["", " ", "   \t  ", "\n", " \n \n "] {
            for offset in (0..=draft.len()).filter(|at| draft.is_char_boundary(*at)) {
                let before = composer(draft).at(offset);

                assert_eq!(
                    compose_for(press(KeyCode::Enter), &before),
                    Composed::Typing(before.clone()),
                    "Enter at offset {offset} should have done nothing to {draft:?}"
                );
                assert!(!before.is_submittable());
            }
        }
    }

    #[test]
    fn esc_hands_the_keyboard_back_from_any_draft() {
        // And takes nothing with it: `Composed::Leave` carries no composer, so
        // the field the caller is holding is the field it was holding — same
        // characters, same insertion point — which is what makes Esc a change
        // of focus rather than an abandonment. Asserted from the middle of the
        // draft as well as its ends.
        for draft in ["", "half a question", "one\ntwo"] {
            for offset in (0..=draft.len()).filter(|at| draft.is_char_boundary(*at)) {
                let before = composer(draft).at(offset);

                assert_eq!(
                    compose_for(press(KeyCode::Esc), &before),
                    Composed::Leave,
                    "Esc should leave from offset {offset} of {draft:?}"
                );
                assert_eq!(before.draft(), draft, "Esc took a character with it");
                assert_eq!(before.cursor(), offset, "Esc moved the cursor on its way");
            }
        }
    }

    #[test]
    fn every_tree_binding_is_swallowed_as_text_or_as_nothing() {
        // Asserted over the whole list because "nothing reaches the app" is the
        // promise: a stray `p` that pacted something behind the composer, or a
        // `j` that moved a selection nobody is looking at, would be the same
        // accident by two routes.
        for code in BINDINGS {
            let before = composer("web");
            let next = after(press(code), &before);

            let expected = character(code).map_or_else(
                || before.clone(),
                |character| composer(&format!("web{character}")),
            );

            assert_eq!(next, expected, "{code:?} should be text or nothing");
        }
    }

    #[test]
    fn nothing_but_enter_submits_and_nothing_but_esc_leaves() {
        // Said the other way round, because what must not happen is the
        // interesting half: of every key this module sees, one offers the draft
        // up and one hands the keyboard back, and no third key does either.
        for code in BINDINGS.into_iter().chain([KeyCode::Backspace]) {
            let outcome = compose_for(press(code), &composer("web"));

            assert_ne!(outcome, Composed::Submit, "{code:?} should not submit");
            assert_ne!(outcome, Composed::Leave, "{code:?} should not leave");
        }

        assert_ne!(
            compose_for(alt_enter(), &composer("web")),
            Composed::Submit,
            "Alt+Enter should not submit"
        );
    }

    #[test]
    fn ctrl_c_is_not_answered_here() {
        // It is a key event, not a signal, and the loop has to take it before it
        // consults this field. Through here it is one of the keys that change
        // nothing — which is why the loop answering it first is load-bearing
        // rather than tidy, and why a chord is never text.
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let before = composer("web");

        assert_eq!(
            compose_for(ctrl_c, &before),
            Composed::Typing(before.clone())
        );
    }

    #[test]
    fn a_chord_or_a_control_character_is_not_text() {
        // Every modifier that makes a character a command rather than a letter,
        // plus a control character arriving as itself. From a cursor somebody
        // has moved back into the draft, since a chord mistaken for text would
        // now put its letter *there* rather than harmlessly at the end.
        let before = composer("web").at(1);

        for modifiers in [
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
            KeyModifiers::HYPER,
            KeyModifiers::META,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ] {
            let key = KeyEvent::new(KeyCode::Char('u'), modifiers);

            assert_eq!(
                compose_for(key, &before),
                Composed::Typing(before.clone()),
                "{modifiers:?} makes `u` a command, not a letter"
            );
        }

        assert_eq!(
            compose_for(press(KeyCode::Char('\u{7}')), &before),
            Composed::Typing(before.clone()),
            "a control character is not text however it arrived"
        );
    }

    #[test]
    fn shift_still_types_and_shift_enter_is_not_a_keystroke_of_its_own() {
        // Terminals disagree about whether the modifier rides along with an
        // upper-case letter, and either way it is somebody typing. They disagree
        // harder about Shift+Enter, which is why it means what Enter means.
        let shifted = KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT);

        assert_eq!(after(shifted, &composer("")), composer("W"));

        let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);

        assert_eq!(compose_for(shift_enter, &composer("why")), Composed::Submit);
        assert_eq!(
            compose_for(shift_enter, &composer("why").at(1)),
            Composed::Submit,
            "and it submits the whole draft from the middle, as Enter does"
        );
        assert_eq!(
            compose_for(shift_enter, &composer("")),
            Composed::Typing(composer("")),
            "and on an empty draft it means what Enter means there too"
        );
    }

    #[test]
    fn releases_and_repeats_change_nothing() {
        // The same rule `action_for` and `edit_for` keep: acting on a release
        // would type the release of the very key that moved the focus here. The
        // movers and the two deleting keys are in the list on a value every one
        // of them would change, so "nothing happened" is a rule about the kind
        // of the event rather than about a draft that had nowhere to go — and
        // because the whole value is compared, neither a byte nor the offset
        // moved.
        let before = drawn("one\ntwo\nsix", 40).at(5);

        for code in [
            KeyCode::Char('s'),
            KeyCode::Backspace,
            KeyCode::Delete,
            KeyCode::Enter,
            KeyCode::Esc,
        ]
        .into_iter()
        .chain(MOVERS)
        {
            for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
                for modifiers in [KeyModifiers::NONE, KeyModifiers::ALT] {
                    let event = KeyEvent::new_with_kind_and_state(
                        code,
                        modifiers,
                        kind,
                        KeyEventState::NONE,
                    );

                    assert_eq!(
                        compose_for(event, &before),
                        Composed::Typing(before.clone()),
                        "{kind:?} of {code:?} with {modifiers:?} should do nothing"
                    );
                }
            }
        }
    }

    #[test]
    fn the_movers_move_and_the_editing_keys_this_field_does_not_have_do_nothing() {
        // The two halves of what the non-character keys come to, said together
        // because they used to be one thing: the six movers each put the cursor
        // somewhere else and leave every byte where it was, and Insert and
        // BackTab go on doing nothing at all — no selection and no history.
        // Delete is an editing key this field does have, and has its own
        // assertions above.
        //
        // Three rows, and the cursor starting on the middle one, so that every
        // one of the six has somewhere to go: at either end two of them would
        // stand still for the honest reason that there is no row past the draft.
        let before = drawn("one\ntwo\nsix", 40).at(5);

        for (code, cursor) in [
            (KeyCode::Left, 4),
            (KeyCode::Right, 6),
            (KeyCode::Home, 4),
            (KeyCode::End, 7),
            (KeyCode::Up, 1),
            (KeyCode::Down, 9),
        ] {
            assert_eq!(
                cursor_after(press(code), &before),
                cursor,
                "{code:?} should move the cursor there and change nothing else"
            );
        }

        // The whole value, so what is asserted is that neither the draft nor
        // the cursor moved: a key that is nothing at all is nothing to both.
        for code in [KeyCode::Insert, KeyCode::BackTab] {
            assert_eq!(
                compose_for(press(code), &before),
                Composed::Typing(before.clone()),
                "{code:?} is not an editing key this field has, and should change neither draft nor cursor"
            );
        }
    }

    #[test]
    fn the_six_movement_keys_change_no_byte_of_the_draft() {
        // The promise the whole group is built on, over a draft that has both
        // kinds of row break in it — the `\n` somebody typed and the soft wrap
        // the width made — and from every offset in it, since a key that edited
        // by accident would do it at one place rather than at all of them.
        let draft = "one two six\nfour";

        for offset in (0..=draft.len()).filter(|at| draft.is_char_boundary(*at)) {
            let before = drawn(draft, 4).at(offset);
            assert!(before.rows(4).len() > 3, "the draft should wrap and break");

            for code in MOVERS {
                let next = after(press(code), &before);

                assert_eq!(
                    next.draft(),
                    draft,
                    "{code:?} at offset {offset} changed a byte of the draft"
                );
                assert_ne!(
                    compose_for(press(code), &before),
                    Composed::Submit,
                    "{code:?} should not submit"
                );
                assert_ne!(
                    compose_for(press(code), &before),
                    Composed::Leave,
                    "{code:?} should not leave"
                );
            }
        }
    }

    #[test]
    fn left_and_right_move_one_character_and_not_one_byte() {
        // A byte of an accent is not a place in the draft, so the step is a
        // `char` — the same rule Backspace already keeps when it takes one back.
        let ascii = composer("web");
        assert_eq!(cursor_after(press(KeyCode::Left), &ascii), 2);
        assert_eq!(cursor_after(press(KeyCode::Left), &ascii.clone().at(2)), 1);
        assert_eq!(cursor_after(press(KeyCode::Right), &ascii.clone().at(1)), 2);

        // `é` is bytes 1 and 2 of "wéb", so a step over it is two bytes wide and
        // never lands on 2.
        let accented = composer("wéb");
        assert_eq!(cursor_after(press(KeyCode::Left), &accented), 3);
        assert_eq!(
            cursor_after(press(KeyCode::Left), &accented.clone().at(3)),
            1
        );
        assert_eq!(
            cursor_after(press(KeyCode::Right), &accented.clone().at(1)),
            3
        );

        // And three bytes wide over a CJK character, which draws in two columns
        // and is still one keypress.
        let cjk = composer("日本語");
        assert_eq!(cursor_after(press(KeyCode::Left), &cjk), 6);
        assert_eq!(cursor_after(press(KeyCode::Right), &cjk.clone().at(0)), 3);
    }

    #[test]
    fn left_stops_at_the_start_and_right_stops_at_the_end() {
        // Stop rather than wrap: a Left at the start that came back at the end
        // of the draft would be a cursor somebody has to watch.
        let start = composer("web").at(0);
        assert_eq!(after(press(KeyCode::Left), &start), start);

        let end = composer("web");
        assert_eq!(after(press(KeyCode::Right), &end), end);

        // Including on the draft with nothing in it, where the two ends are the
        // same place.
        let empty = composer("");
        assert_eq!(after(press(KeyCode::Left), &empty), empty);
        assert_eq!(after(press(KeyCode::Right), &empty), empty);
    }

    #[test]
    fn home_and_end_go_to_the_ends_of_the_display_row() {
        // The row the cursor is on, not the draft: on a draft of typed lines
        // that is the line, and the `\n` between two lines belongs to the row
        // above it, which is where End on that row puts the cursor.
        let lines = drawn("one\ntwo", 40);
        assert_eq!(cursor_after(press(KeyCode::Home), &lines), 4);
        assert_eq!(cursor_after(press(KeyCode::End), &lines), 7);
        assert_eq!(cursor_after(press(KeyCode::Home), &lines.clone().at(1)), 0);
        assert_eq!(cursor_after(press(KeyCode::End), &lines.clone().at(1)), 3);

        // Both are what they already are at the two ends of a row, so pressing
        // either twice is pressing it once.
        assert_eq!(cursor_after(press(KeyCode::Home), &lines.clone().at(4)), 4);
        assert_eq!(cursor_after(press(KeyCode::End), &lines.clone().at(7)), 7);
    }

    #[test]
    fn end_then_home_on_a_soft_wrapped_row_is_a_no_op() {
        // The row-break rule as a keystroke: the end of a wrapped row is the
        // break offset, and the break offset is drawn at column zero of the row
        // below — so End looks like it went down one, and Home from there comes
        // straight back to it rather than to the start of the row End left.
        let wrapped = drawn("one two", 4);
        assert_eq!(window_rows(&wrapped, 4), ["one ", "two"]);

        let ended = after(press(KeyCode::End), &wrapped.clone().at(0));
        assert_eq!(ended.cursor(), 4, "the end of the wrapped row is the break");
        assert_eq!(
            ended.place(4, 4),
            (1, 0),
            "and it is drawn on the row below"
        );

        assert_eq!(
            after(press(KeyCode::Home), &ended),
            ended,
            "Home from the break offset is the offset it is already at"
        );
    }

    #[test]
    fn up_and_down_step_the_rows_as_they_are_drawn() {
        // One paragraph, no newline in it at all, three rows on screen — and
        // three rows is what Up and Down step through, because a reader pressing
        // Down at a wrapped paragraph is asking for the row underneath rather
        // than for the end of it.
        let wrapped = drawn("one two six", 4);
        assert_eq!(window_rows(&wrapped, 4), ["one ", "two ", "six"]);

        assert_eq!(cursor_after(press(KeyCode::Up), &wrapped), 7);
        assert_eq!(cursor_after(press(KeyCode::Up), &wrapped.clone().at(7)), 3);

        assert_eq!(
            cursor_after(press(KeyCode::Down), &wrapped.clone().at(3)),
            7
        );
        assert_eq!(
            cursor_after(press(KeyCode::Down), &wrapped.clone().at(7)),
            11
        );

        // The same draft at a width that fits it whole is one row, and there is
        // nowhere for either key to go.
        let unwrapped = drawn("one two six", 40);
        assert_eq!(unwrapped.rows(40).len(), 1);
        assert_eq!(after(press(KeyCode::Up), &unwrapped), unwrapped);
        assert_eq!(after(press(KeyCode::Down), &unwrapped), unwrapped);
    }

    #[test]
    fn up_and_down_hold_the_column_where_the_row_they_land_on_is_long_enough() {
        // Columns, not characters: the row above is three CJK characters and six
        // cells wide, so a cursor four cells along it lands four cells along the
        // row below — on `e`, not on the character with the same index.
        let mixed = drawn("日本語\nabcdef", 40);
        assert_eq!(mixed.place(6, 40), (0, 4));
        assert_eq!(cursor_after(press(KeyCode::Down), &mixed.clone().at(6)), 14);
        assert_eq!(cursor_after(press(KeyCode::Up), &mixed.clone().at(14)), 6);

        // And where the destination row is too short to hold the column, the end
        // of that row is where the cursor lands.
        let ragged = drawn("abcdef\nxy", 40);
        assert_eq!(cursor_after(press(KeyCode::Down), &ragged.clone().at(6)), 9);
        assert_eq!(cursor_after(press(KeyCode::Up), &ragged.clone().at(9)), 2);
    }

    #[test]
    fn up_on_the_first_row_and_down_on_the_last_leave_the_cursor_where_it_is() {
        // There is no row past either end of the draft, so there is nowhere to
        // go: the cursor stays at the offset it was at rather than sliding to
        // the start or the end of the row it is on.
        let lines = drawn("one\ntwo", 40);

        let top = lines.clone().at(1);
        assert_eq!(after(press(KeyCode::Up), &top), top);

        let bottom = lines.clone().at(5);
        assert_eq!(after(press(KeyCode::Down), &bottom), bottom);

        // Including on a wrapped draft, where the first and last rows are rows
        // the width made rather than rows somebody typed.
        let wrapped = drawn("one two six", 4);
        let first = wrapped.clone().at(1);
        assert_eq!(after(press(KeyCode::Up), &first), first);

        let last = wrapped.clone().at(9);
        assert_eq!(after(press(KeyCode::Down), &last), last);
    }

    #[test]
    fn the_movers_move_over_the_rows_the_width_last_drawn_at_made() {
        // The width is a fact about the screen, told to the field once a round,
        // and it is what the row-wise keys are answered at: the same draft and
        // the same keystroke at two widths are two different rows and two
        // different answers. Which is why the cursor is a byte offset — a
        // resized terminal re-flows the rows under an offset that still means
        // the place it meant.
        let draft = "one two six";

        assert_eq!(cursor_after(press(KeyCode::Home), &drawn(draft, 4)), 8);
        assert_eq!(cursor_after(press(KeyCode::Home), &drawn(draft, 40)), 0);

        // A width nobody has measured is not a width to break at, so every line
        // is one row and the row-wise keys fall back to working line-wise.
        assert_eq!(cursor_after(press(KeyCode::Home), &drawn(draft, 0)), 0);
    }

    #[test]
    fn a_pasted_block_keeps_every_line_and_sends_nothing() {
        // The bug this exists for: a three-line block used to send line one as
        // a question and lose the rest behind the mute that turn put up. Every
        // line is in the draft, separated by the `\n` that was copied, and
        // nothing is submitted — which the return type sees to, since
        // `Pasted::Typing` is the only value there is.
        let block = "first line\nsecond line\nthird line";

        assert_eq!(pasted(block, &composer("")).draft(), block);
        assert_eq!(
            pasted(block, &composer("")).draft().lines().count(),
            3,
            "every line of the block should still be there"
        );
        assert_eq!(
            pasted(block, &composer("why: ")).draft(),
            "why: first line\nsecond line\nthird line",
            "a paste goes in at the end, where typing goes"
        );
        assert_eq!(
            pasted("one\ntwo\n", &composer("")).draft(),
            "one\ntwo\n",
            "a trailing newline is a byte of the block like any other"
        );
    }

    #[test]
    fn a_paste_with_no_newline_appends_exactly_as_typing_it_would() {
        // The insertion point is the same insertion point: whatever route the
        // characters came in by, the draft afterwards is the same draft.
        let text = "why nine passes?";
        let before = composer("ask: ");

        let mut typed = before.clone();
        for character in text.chars() {
            typed = after(press(KeyCode::Char(character)), &typed);
        }

        assert_eq!(pasted(text, &before).draft(), typed.draft());
        assert_eq!(pasted(text, &before), typed);
    }

    #[test]
    fn a_muted_field_takes_no_paste_and_stays_muted() {
        // One question at a time: while the answer to the last one is on its
        // way the field takes nothing, and a paste is one arrival carrying
        // however much was copied rather than a key somebody can stop pressing.
        let mut before = composer("half a question");
        before.set_muted(true);

        let next = pasted("\nand the rest of it", &before);

        assert_eq!(next.draft(), "half a question");
        assert!(next.is_muted(), "a paste unmuted the field");
        assert_eq!(next, before);
    }

    #[test]
    fn a_paste_carries_a_live_field_through_live() {
        // The other half of the flag's rule: this is not where a turn starts or
        // ends either, so a field that arrived live goes back live.
        let next = pasted("one\ntwo", &composer(""));

        assert!(!next.is_muted());
    }

    #[test]
    fn an_empty_paste_changes_nothing() {
        // A terminal that reports a paste of nothing is not a mistake anybody
        // has to hear about.
        let before = composer("one\ntwo");

        assert_eq!(pasted("", &before), before);
    }

    #[test]
    fn a_tall_paste_grows_the_field_to_the_cap_and_windows_to_the_tail() {
        // Nothing about the field's own scrolling changes: the height stops at
        // the cap, a paste snaps the cursor to the end of what it put in, and a
        // cursor on the last row leaves the window at the last rows — so the
        // end of what was pasted, where the next character will go, is what is
        // on screen.
        let block = (1..=20)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let next = pasted(&block, &composer(""));

        assert_eq!(next.height(40), COMPOSER_MAX_ROWS);
        assert_eq!(
            window_rows(&next, 40),
            [
                "line 15", "line 16", "line 17", "line 18", "line 19", "line 20"
            ]
        );
    }

    #[test]
    fn an_empty_composer_is_one_row_tall() {
        // The field is always on screen and the cursor always has a row.
        for width in [1, 20, 200] {
            assert_eq!(composer("").height(width), 1);
            assert_eq!(window_rows(&composer(""), width), [""]);
        }
    }

    #[test]
    fn a_draft_inside_the_width_is_one_row_tall() {
        assert_eq!(composer("why nine passes").height(40), 1);
        assert_eq!(
            window_rows(&composer("why nine passes"), 40),
            ["why nine passes"]
        );
    }

    #[test]
    fn a_newline_is_one_row_more() {
        assert_eq!(composer("one").height(40), 1);
        assert_eq!(composer("one\n").height(40), 2);
        assert_eq!(composer("one\ntwo").height(40), 2);
        assert_eq!(composer("one\ntwo\n").height(40), 3);
        assert_eq!(composer("one\ntwo\nthree").height(40), 3);

        assert_eq!(window_rows(&composer("one\ntwo"), 40), ["one", "two"]);
        assert_eq!(
            window_rows(&composer("one\n"), 40),
            ["one", ""],
            "the row after a newline is where the cursor is sitting"
        );
    }

    #[test]
    fn a_wrap_is_one_row_more_and_the_rows_are_the_draft_in_order() {
        let rows = window_rows(&composer("It walks the tree and writes what it finds."), 18);

        // Every row is filled to the width and cut there, wherever in a word or
        // a space that lands: the draft is text somebody is still typing, so
        // every character typed has a cell of its own to put the cursor in and
        // the rows join back up to the draft byte for byte.
        assert_eq!(
            rows,
            ["It walks the tree ", "and writes what it", " finds."]
        );
        assert_eq!(rows.concat(), "It walks the tree and writes what it finds.");
        assert_eq!(
            composer("It walks the tree and writes what it finds.").height(18),
            3
        );
    }

    #[test]
    fn a_run_with_nowhere_to_break_in_it_fills_the_row_it_started_on() {
        // A word, a space, and then a run with nowhere to break in it, at the
        // two lengths that matter: one that would fit a row of its own and one
        // past that. A field that broke at the space would leave `test ` sitting
        // alone with the rest of the row standing for no offset at all, and the
        // run would jump the draft a row at the keystroke that outgrew it. Here
        // the row goes on filling, and the character typed lands in the next
        // cell along whichever length the run is.
        let inside = format!("test {}", "h".repeat(10));
        let past = format!("test {}", "h".repeat(30));

        assert_eq!(window_rows(&composer(&inside), 12), ["test hhhhhhh", "hhh"]);
        assert_eq!(
            window_rows(&composer(&past), 12),
            ["test hhhhhhh", "hhhhhhhhhhhh", "hhhhhhhhhhh"],
        );
        // Every byte of the draft is still in a row to put the cursor in.
        assert_eq!(window_rows(&composer(&past), 12).concat(), past);
        assert_eq!(composer(&inside).height(12), 2);
        assert_eq!(composer(&past).height(12), 3);
    }

    #[test]
    fn growing_the_draft_grows_the_field_a_row_at_a_time_up_to_the_cap() {
        // Typed a character at a time through a narrow field: the height climbs
        // by one and never by two, and stops climbing at the cap.
        let mut current = composer("");
        let mut heights = vec![current.height(10)];
        for character in
            "abcdefghij klmnopqrst uvwxyzabcd efghijklmn opqrstuvwx yzabcdefgh ijklmnopqr".chars()
        {
            current = after(press(KeyCode::Char(character)), &current);
            let height = current.height(10);
            let last = *heights.last().expect("the first height is already in");
            assert!(
                height == last || height == last + 1,
                "{:?} went from {last} rows to {height}",
                current.draft()
            );
            heights.push(height);
        }

        assert_eq!(*heights.first().expect("an empty draft has a height"), 1);
        assert_eq!(current.height(10), COMPOSER_MAX_ROWS);
    }

    #[test]
    fn a_draft_past_the_cap_stops_at_the_cap() {
        let long = composer(&"line\n".repeat(40));

        assert_eq!(long.height(40), COMPOSER_MAX_ROWS);
        assert_eq!(window_rows(&long, 40).len(), usize::from(COMPOSER_MAX_ROWS));

        let wide = composer(&"word ".repeat(200));

        assert_eq!(wide.height(20), COMPOSER_MAX_ROWS);
        assert_eq!(window_rows(&wide, 20).len(), usize::from(COMPOSER_MAX_ROWS));
    }

    #[test]
    fn the_window_ends_at_the_cursor_so_the_row_typing_lands_on_is_always_in_it() {
        // Typing lands where the cursor is, and a draft being written has it at
        // the end: the row it appears on is the last row, and the window that
        // follows the cursor there is the tail the field drew before it followed
        // anything, since there are no rows below to keep.
        let draft = (1..=20)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let composer = composer(&draft);
        let window = window_rows(&composer, 40);

        assert_eq!(
            window,
            ["line 16", "line 17", "line 18", "line 19", "line 20", "",]
        );
        assert_eq!(
            window.last().map(String::as_str),
            Some(""),
            "the last row is the one the next character typed would appear on"
        );
    }

    #[test]
    fn a_cursor_on_the_last_row_starts_the_window_at_the_tail() {
        // The margin's degenerate case, stated in the numbers the rule is
        // written in: ten rows, a field of six, the cursor on the last row —
        // the first row drawn is the fourth, which is `rows - height` and the
        // tail exactly. The ceiling wins over the margin here, and that is why
        // a draft being typed at its end draws what it always drew.
        let draft = numbered(10);
        let window = composer(&draft).window(40, 6);

        assert_eq!(
            window.rows,
            ["row 4", "row 5", "row 6", "row 7", "row 8", "row 9"]
        );
        assert_eq!(scrolled(&composer(&draft), 6), (4, 9));
        assert_eq!(
            window.row, 5,
            "the caret is on the last drawn row, which is the last row of the draft"
        );
        assert_eq!(
            window.column,
            "row 9".len(),
            "and in the cell after the last character, where the next one goes"
        );
    }

    #[test]
    fn the_cursor_travels_three_rows_before_the_window_follows_it_down() {
        // Twelve rows in a field of six, walked from the top with the Down key.
        // The first three rows are travelled with the content standing still —
        // the margin is above the cursor, and at the top of a draft there is
        // nothing above to keep — and from there every row takes the window
        // with it, until the last window there are rows for.
        let draft = numbered(12);
        let mut current = drawn(&draft, 40).at(0);
        let mut firsts = vec![scrolled(&current, 6).0];
        for _ in 1..12 {
            current = after(press(KeyCode::Down), &current);
            firsts.push(scrolled(&current, 6).0);
        }

        assert_eq!(firsts, [0, 0, 0, 1, 2, 3, 4, 5, 6, 6, 6, 6]);
    }

    #[test]
    fn the_window_follows_the_cursor_up_a_row_at_a_time_as_well_as_down() {
        // The same twelve rows walked the other way with Up, from the last row
        // to the first: a cursor moved above the six rows on screen brings them
        // up to it rather than being moved somewhere nobody can see. The
        // standing still is at the bottom this time, where the ceiling holds the
        // window at the last six rows however far up the cursor is inside them.
        let draft = numbered(12);
        let mut current = drawn(&draft, 40);
        let mut firsts = vec![scrolled(&current, 6).0];
        for _ in 1..12 {
            current = after(press(KeyCode::Up), &current);
            firsts.push(scrolled(&current, 6).0);
        }

        assert_eq!(firsts, [6, 6, 6, 6, 5, 4, 3, 2, 1, 0, 0, 0]);
    }

    #[test]
    fn the_caret_is_on_the_cursors_own_row_of_every_window_it_comes_back_in() {
        // The window and the caret are one answer, so there is no window the
        // caret is not a row of: at every row of a long draft, at every height
        // a field is ever drawn at, the caret's row is a row of the rows that
        // came back with it, and it is the row the cursor is on.
        let draft = numbered(12);
        for row in 0..12 {
            for height in 1..=COMPOSER_MAX_ROWS {
                let composer = at_row(&draft, row);
                let window = composer.window(40, height);
                let (first, caret) = scrolled(&composer, height);

                assert_eq!(caret, row, "the caret is on the cursor's row");
                assert!(
                    window.row < window.rows.len(),
                    "row {row} at height {height} started at {first} and put the caret on row {} \
                     of {} rows",
                    window.row,
                    window.rows.len()
                );
            }
        }
    }

    #[test]
    fn a_field_with_no_rows_inside_it_comes_back_with_none() {
        // A border squeezed to nothing: there is no row to draw and no row to
        // put a caret on, and the arithmetic says so rather than slicing past
        // the end of the draft.
        let window = composer(&numbered(12)).window(40, 0);

        assert!(window.rows.is_empty());
        assert_eq!(window.row, 0);
    }

    #[test]
    fn the_window_is_every_row_while_the_draft_is_inside_the_cap() {
        // Nothing has to be reset when a long draft is backspaced short again,
        // because there is no offset to reset.
        for lines in 1..usize::from(COMPOSER_MAX_ROWS) {
            let draft = (1..=lines)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n");
            let composer = composer(&draft);

            assert_eq!(composer.height(40), u16::try_from(lines).expect("small"));
            assert_eq!(window_rows(&composer, 40).len(), lines);
        }
    }

    #[test]
    fn a_column_nobody_has_measured_wraps_nothing() {
        // A width of zero is the state of a composer between being built and
        // being drawn, and is not a width to break at — the same rule the
        // panel's own wrapping keeps.
        let long = "It walks the tree and writes what it finds.";

        assert_eq!(composer(long).height(0), 1);
        assert_eq!(window_rows(&composer(long), 0), [long]);
    }

    #[test]
    fn the_height_is_never_zero_and_never_past_the_cap_at_any_width() {
        let drafts = ["", " ", "one", "one\ntwo", &"word ".repeat(60)];

        for draft in drafts {
            for width in 0..40 {
                let height = composer(draft).height(width);

                assert!(height >= 1, "{draft:?} at {width} came back {height}");
                assert!(
                    height <= COMPOSER_MAX_ROWS,
                    "{draft:?} at {width} came back {height}"
                );
                assert_eq!(
                    usize::from(height),
                    window_rows(&composer(draft), width).len()
                );
            }
        }
    }

    const DRAFTS: [&str; 7] = [
        "one\ntwo",
        "It walks the tree and writes what it finds.",
        "It walks the tree\nand writes what it finds.",
        "日本語のテキスト",
        "caf\u{e9} au lait",
        "",
        "one\n",
    ];

    #[test]
    fn every_offset_is_drawn_somewhere_and_comes_back_from_there() {
        for draft in DRAFTS {
            let composer = composer(draft);
            for width in [0, 1, 4, 40] {
                for offset in (0..=draft.len()).filter(|at| draft.is_char_boundary(*at)) {
                    let (row, column) = composer.place(offset, width);

                    assert_eq!(
                        composer.offset_at(row, column, width),
                        offset,
                        "{draft:?} at width {width}: offset {offset} is drawn at row {row}, \
                         column {column}, and that cell came back somewhere else"
                    );
                    assert!(
                        row < composer.rows(width).len(),
                        "{draft:?} at width {width}: offset {offset} is on row {row}, and there \
                         are {} rows",
                        composer.rows(width).len()
                    );
                }
            }
        }
    }

    #[test]
    fn an_offset_at_a_row_break_is_column_zero_of_the_row_below() {
        // The rule stated on `Composer::place`, asserted rather than left to the
        // round trip: the round trip would pass just as well with the offset
        // drawn at the end of the row above.
        let wrapped = composer("one two");
        assert_eq!(window_rows(&wrapped, 4), ["one ", "two"]);

        // Offset 4 is both the cell after "one " and column zero of "two", and
        // it is the lower one.
        assert_eq!(wrapped.place(4, 4), (1, 0));
        assert_eq!(wrapped.place(3, 4), (0, 3));
        // The inverse keeps the same rule from the other side: the cell past the
        // end of the row above is the pair `place` never comes back with, and
        // asking for it gives the break offset — the lower place again.
        assert_eq!(wrapped.offset_at(0, 4, 4), 4);
        assert_eq!(wrapped.offset_at(0, 40, 4), 4);

        // A newline is not a row break: its own byte lies between the rows, so
        // the cell after "one" is the offset of the `\n` and belongs to the row
        // above, which is where a cursor before a newline sits.
        let lined = composer("one\ntwo");
        assert_eq!(lined.place(3, 40), (0, 3));
        assert_eq!(lined.offset_at(0, 3, 40), 3);
        assert_eq!(lined.place(4, 40), (1, 0));
    }

    #[test]
    fn a_wide_character_is_the_cells_it_draws_in_and_not_one_column() {
        // Counting characters would put the end of this draft at column 3.
        let cjk = composer("日本語");
        assert_eq!(cjk.draft().chars().count(), 3);
        assert_eq!(cjk.draft().len(), 9);

        assert_eq!(cjk.place(0, 40), (0, 0));
        assert_eq!(cjk.place(3, 40), (0, 2));
        assert_eq!(cjk.place(6, 40), (0, 4));
        assert_eq!(cjk.place(9, 40), (0, 6));

        // A column inside a wide character is that character's own offset: the
        // two cells it draws in are both its.
        assert_eq!(cjk.offset_at(0, 1, 40), 0);
        assert_eq!(cjk.offset_at(0, 2, 40), 3);
        assert_eq!(cjk.offset_at(0, 3, 40), 3);

        // And the columns are still cells where narrow and wide are mixed.
        let mixed = composer("a日b");
        assert_eq!(mixed.place(1, 40), (0, 1));
        assert_eq!(mixed.place(4, 40), (0, 3));
        assert_eq!(mixed.offset_at(0, 3, 40), 4);
    }

    #[test]
    fn no_row_and_no_column_at_any_width_lands_inside_a_character() {
        // The inverse is total — a row past the last row is the last row, and a
        // column past the end of a row is the end of that row — and nothing it
        // answers with ever cuts a character in half.
        for draft in DRAFTS {
            let composer = composer(draft);
            for width in 0..12 {
                let rows = composer.rows(width).len();
                for row in 0..rows + 2 {
                    for column in 0..12 {
                        let offset = composer.offset_at(row, column, width);

                        assert!(
                            offset <= draft.len() && draft.is_char_boundary(offset),
                            "{draft:?} at width {width}: row {row}, column {column} came back \
                             with offset {offset}"
                        );
                    }
                }
            }
        }
    }
}
