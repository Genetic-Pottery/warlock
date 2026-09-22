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
//! keyboard the loop consults it *instead of* `input::action_for` rather than
//! as well: `p` is the letter p.

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
/// `width`, `muted` and `answering` are facts about the session rather than
/// about the draft, told in once a round by whoever is about to draw. All three
/// take part in [`PartialEq`] and [`Hash`] with everything else, so every value
/// built here must carry the incoming three through untouched — a keystroke
/// that dropped one would read as a redraw to any whole-value comparison.
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
    /// Who the draft is for, when it is not for this conversation: the one
    /// sentence naming whatever the next submission answers. `None` is the
    /// ordinary field, which is the conversation's own.
    ///
    /// A line to draw and not a destination: nothing here knows where a
    /// submission goes, and whoever routes it is the same value that says this.
    answering: Option<String>,
}

impl Composer {
    /// A field holding that draft with the cursor at the end of it, which is
    /// where somebody about to edit what is there starts from: every editing
    /// key works on it exactly as it works on something they typed, because it
    /// is the same value a typed draft is.
    #[must_use]
    pub fn new(draft: impl Into<String>) -> Self {
        let draft = draft.into();
        let cursor = draft.len();

        Self {
            draft,
            cursor,
            width: 0,
            muted: false,
            answering: None,
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

    /// Told every round for [`Composer::set_muted`]'s reason, off whatever is
    /// waiting on a draft: a field that had to be told at the two edges would
    /// be one somebody could leave labelled for a question that is over.
    ///
    /// The draft does not move. What this changes is what the field is drawn as
    /// and nothing about what is in it, because a question relayed into the
    /// field is answered with whatever somebody sends — theirs or warlock's.
    pub fn set_answering(&mut self, answering: Option<String>) {
        self.answering = answering;
    }

    #[must_use]
    pub fn answering(&self) -> Option<&str> {
        self.answering.as_deref()
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
        let height = usize::from(height);
        if height == 0 {
            return ComposerWindow {
                rows: Vec::new(),
                row: 0,
                column,
            };
        }

        let rows = self.rows(width);
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
        let (row, (start, text)) = placed
            .iter()
            .enumerate()
            .rfind(|(_, (start, _))| *start <= offset)
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
    // The builders below are written literally rather than through
    // `Composer::at`, and each holds that constructor's invariant by
    // construction: `composer.cursor` is a boundary of the draft, so the halves
    // it splits into are whole strings and the offset after a whole character of
    // the result is a boundary of the result. `typing` is the one place
    // `width`, `muted` and `answering` are carried through, and it carries them
    // untouched — this function is neither a redraw, nor where a turn starts or
    // ends, nor where a question is relayed or answered.
    let typing = |draft: String, cursor: usize| {
        Composed::Typing(Composer {
            draft,
            cursor,
            width: composer.width,
            muted: composer.muted,
            answering: composer.answering.clone(),
        })
    };
    let inserted = |character: char| {
        let cursor = composer.cursor;
        let mut draft = String::with_capacity(composer.draft.len() + character.len_utf8());
        draft.push_str(&composer.draft[..cursor]);
        draft.push(character);
        draft.push_str(&composer.draft[cursor..]);

        typing(draft, cursor + character.len_utf8())
    };
    // `start` and `end` are the two ends of a whole `char`, which is what makes
    // this deletion by character rather than by byte: half a character left
    // behind would not be a `String` at all. Which character goes is the only
    // thing Backspace and Delete disagree about.
    let removed = |start: usize, end: usize| {
        let mut draft = String::with_capacity(composer.draft.len() - (end - start));
        draft.push_str(&composer.draft[..start]);
        draft.push_str(&composer.draft[end..]);

        typing(draft, start)
    };
    // Every offset handed to this comes off `char_indices`, `place` or
    // `offset_at`, all three of which answer with boundaries of this draft.
    let moved = |cursor: usize| typing(composer.draft.clone(), cursor);
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

    // The width and the two flags come through untouched, as they do at a
    // keystroke.
    Pasted::Typing(Composer {
        cursor: draft.len(),
        draft,
        width: composer.width,
        muted: composer.muted,
        answering: composer.answering.clone(),
    })
}

#[cfg(test)]
#[path = "tests/composer.rs"]
mod tests;
