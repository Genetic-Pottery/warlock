//! The composer: the several-line draft at the foot of the panel's column, and
//! the two pure functions saying what a key and what a paste do to it.
//!
//! Every warlock command is a single letter — `p` pacts, `r` refreshes, `s`
//! scopes, `v` views — so the moment a field is on screen and holding the
//! keyboard, `p` has to be able to mean the letter p. This module is the field
//! that makes that true. It is [`prompt`](crate::prompt)'s shape grown to
//! several lines: a value holding what has been typed, and
//! [`compose_for`], which takes a key event and that value and comes back with
//! one of three consequences.
//!
//! ## A buffer, an insertion point, and deliberately nothing more
//!
//! Printable characters append, Backspace takes the last character back,
//! Alt+Enter starts a new line, Enter offers the draft up and Esc hands the
//! keyboard back. That is the entire editor. There is no history, no selection
//! and no completion.
//!
//! There is an insertion point, though: [`Composer::cursor`], a byte offset into
//! the draft, and the six keys that move it. Left and Right step a character,
//! Home and End go to the ends of the display row, and Up and Down step the rows
//! as they are drawn — so a wrapped paragraph is walked visually rather than by
//! line. None of the six changes a byte, and Delete, Insert and `BackTab` go on
//! doing nothing at all, exactly as [`edit_for`](crate::edit_for) has it,
//! because every key a mover claims is a key that cannot be one of the
//! characters being typed.
//!
//! What has *not* arrived yet is editing at that insertion point: a character
//! typed, Alt+Enter, Backspace and a paste all still land at the end of the
//! draft and snap the cursor there with them, so there is one insertion rule in
//! the build at a time rather than two disagreeing ones. The drawing has not
//! caught up either — the caret is still drawn after the last character — which
//! is why nothing here has to be true of the screen yet.
//!
//! Because four of those six are row-wise, the value carries the width it was
//! last drawn at ([`Composer::set_width`]), told to it once a round the way its
//! muting is. That keeps [`compose_for`] a function of a key and a composer and
//! nothing else.
//!
//! A paste is the fourth road in, and it is deliberately the narrowest.
//! [`paste_for`] takes a block of text the terminal handed over whole and puts
//! it in the draft where typing goes — at the end, since that is the only place
//! this field has — and what comes back is a [`Pasted`], which has one variant
//! and cannot say "submit" or "leave" however many newlines the block carries.
//! That is the whole of why it is a second function rather than another arm of
//! [`compose_for`]: a pasted `\n` is a character of somebody's paragraph, and
//! without bracketed paste it arrives as `KeyCode::Enter` and sends the first
//! line as a question nobody finished asking. Nothing about the insertion point
//! moves here either — a paste appends, exactly as typing the same characters
//! one at a time would, and there is still no cursor to put anywhere else.
//!
//! Enter and Alt+Enter are the pair, and Shift+Enter is deliberately not a third
//! keystroke: terminals disagree about whether they report it at all, so binding
//! it would be binding a key half the readers of warlock do not have. A draft
//! that is empty or nothing but whitespace is not offered up — Enter on it
//! changes nothing rather than sending a message with no message in it — which
//! is the one judgement this module does make, and it makes it because "is there
//! anything here" is a question about the buffer rather than about what the
//! buffer is for.
//!
//! ## Why the value knows its own height
//!
//! The composer is one row tall when it is empty and grows a row at a time as
//! the draft wraps or a newline is inserted, up to [`COMPOSER_MAX_ROWS`], and
//! the panel above it loses exactly the rows it takes. That arithmetic has to be
//! done before the frame is cut, so it lives here rather than in the drawing:
//! [`Composer::height`] is the number the layout asks for, and
//! [`Composer::window`] is the tail of rows that number has room for. The tail
//! rather than the head, because the end of the draft is where typing lands — a
//! window that followed the top of the draft would scroll the thing being typed
//! off the bottom of itself. A window that follows the *cursor* is the drawing
//! slice's, and is deliberately not here yet.
//!
//! Rows are counted with [`folded`](crate::wrap::folded), which breaks where the
//! panel's own wrapper breaks, so a row counted here is a row the frame agrees
//! with and the composer never asks for a height the drawing then disagrees
//! with. It is the byte-preserving wrapper rather than
//! [`wrapped`](crate::wrap::wrapped) because a draft is text somebody is still
//! typing: the space a row broke at stays on that row, so the rows join back up
//! to the draft byte for byte and nothing typed goes missing between the buffer
//! and the screen. The row keeping its break character is one column wider than
//! the width, and the drawing truncates at the pane edge.
//!
//! ## What this deliberately does not answer
//!
//! Ctrl-C. It is a key event and not a signal — raw mode is exactly the mode in
//! which the terminal stops turning it into `SIGINT` — so the event loop answers
//! it before it consults this module, with the composer focused and without.
//! Through here it is one of the keys that change nothing, because a character
//! carrying Ctrl is not text somebody typed; if it were, the one keystroke every
//! reader trusts to get them out would put a `c` in the draft.
//!
//! Muting, too — as a fact. A field is muted for as long as the answer to the
//! last question is on its way, and for no other reason — one question at a
//! time. Setting it is somewhere else: the loop owns the turn, so it owns the
//! flag ([`Composer::set_muted`]), and the keyboard's gate is what declines to
//! ask [`compose_for`] anything while the flag is up. So [`compose_for`] behaves
//! identically either way and simply carries the flag through, which is what
//! keeps "what a key does to a draft" one set of rules rather than two.
//! [`paste_for`] carries the flag through in exactly the same way and adds one
//! thing the keys do not need: a muted field takes no paste, because a block of
//! bytes the terminal delivered while an answer is in flight is not something
//! anybody typed at this field, and it is one arrival rather than a key at a
//! time — a gate missed at a call site would land the lot.
//!
//! Where the draft is kept between keystrokes, which pane has the focus, and
//! what a submitted draft is *for* are all somebody else's business. Nothing
//! here reads a terminal, draws anything, or takes an [`App`](crate::App): a key
//! event or a pasted block goes in with the current draft, one consequence comes
//! out, and every rule below is one assertion with nothing attached to stdout.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::ui::display_width;
use crate::wrap::folded;

/// The most rows the composer is ever drawn in, however long the draft gets.
///
/// A cap rather than a field, because the number is a judgement about the screen
/// and not about the draft: the panel above is where warlock says what it did,
/// and a composer allowed to grow without limit would eat the record it is being
/// typed next to. Six rows is enough to see a paragraph while writing it and
/// little enough that the account keeps most of the column; past it the composer
/// scrolls within itself and the newest row stays at the bottom.
pub const COMPOSER_MAX_ROWS: u16 = 6;

/// The modifiers that mean a character is a command rather than something
/// somebody typed.
///
/// The same set [`prompt`](crate::prompt) keeps, for the same reason and stated
/// again rather than shared: a field's idea of what is text is part of that
/// field. Shift is deliberately absent — an upper-case letter arrives with it on
/// some terminals and without it on others, and either way it is text.
/// Everything here is a chord, Ctrl-C first among them, and a chord is left to
/// the loop above rather than put in the draft.
const CHORD: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::HYPER)
    .union(KeyModifiers::META);

/// What has been typed into the composer, and where the insertion point is in
/// it.
///
/// One string, one offset, one width and one flag. No scroll offset, because the
/// window is always the tail (see [`Composer::window`]).
///
/// The offset is [`Composer::cursor`], and it is a byte index into the draft
/// rather than a row and a column, so that it goes on meaning the same place
/// when the terminal is resized and the draft re-flows underneath it. It is
/// always on a `char` boundary and never past the end of the draft: every value
/// built in this module holds that by construction, and the one way to set it
/// from outside — [`Composer::at`] — panics rather than clamp.
///
/// The width is [`Composer::width`], and it is the width the field was last
/// *drawn* at rather than a width the draft has an opinion about. It is here for
/// one reason: Home, End, Up and Down move by display row, and which row an
/// offset is on is a question only a width can answer (see [`compose_for`]).
/// [`Composer::height`], [`Composer::window`] and the maps go on taking a width
/// as an argument, so a composer can still be driven through every width a
/// terminal has in one test without a terminal; this is the one the *keys* are
/// answered at, told once a round by whoever is about to draw
/// ([`Composer::set_width`]), exactly as the muting is.
///
/// The flag is [`Composer::is_muted`], and it is a fact about the session rather
/// than about the draft: one question at a time, so while an answer is on its
/// way the field takes no keys and is drawn to say so. Which is also why it is
/// carried here rather than worked out where it is read — the loop knows whether
/// a turn is in flight, and it tells the field once a round, exactly as it tells
/// the app what the terminal is doing with the pointer.
///
/// Every field takes part in `PartialEq` and `Hash` — this is a whole value and
/// is compared as one — so two composers holding the same characters at
/// different insertion points are two different values, and so are two last
/// drawn at different widths. Which is why every value built here carries the
/// incoming width through untouched: a keystroke is not a redraw.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Composer {
    /// What has been typed. Newlines are in it as `\n`.
    draft: String,
    /// Where the next character goes: a byte offset into `draft`, on a `char`
    /// boundary, at most `draft.len()`. While nothing edits anywhere but the
    /// end it is `draft.len()` after every keystroke and every paste that
    /// changes the draft.
    cursor: usize,
    /// How many columns the field was last drawn in, as last set by
    /// [`Composer::set_width`]. Zero until somebody draws, which is a column
    /// nobody has measured and folds every line to one row.
    width: u16,
    /// Whether the field is taking keys at all. `false` for the whole of a
    /// session that never asks anything and never runs a pact; `true` only for
    /// as long as a turn is being answered, or a run writing documents,
    /// somewhere else.
    muted: bool,
}

impl Composer {
    /// A composer holding `draft`, live, with the cursor at the end of it.
    ///
    /// [`Composer::default`] is the empty one a session starts on — empty draft,
    /// cursor at zero. This is for putting a composer back where it was — and
    /// for tests, which is most of what a value this small wants a constructor
    /// for. The end rather than the start because the end is where somebody who
    /// has just typed `draft` would be, so every whole-value comparison written
    /// against a draft alone goes on saying what it said. Somewhere else in the
    /// draft is [`Composer::at`]. Neither the muting nor the width is a
    /// constructor's business: both are told to the field from outside as the
    /// session goes on, by [`Composer::set_muted`] and [`Composer::set_width`],
    /// and a composer nobody has drawn yet is a composer nobody has measured.
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

    /// The same composer with the cursor at `offset` instead.
    ///
    /// A consuming builder, so `Composer::new("hello").at(3)` is one expression
    /// and there is no half-built composer to leave lying around:
    /// `Composer::new` puts the cursor where typing would have left it, and this
    /// is how a test says somewhere else.
    ///
    /// # Panics
    ///
    /// If `offset` is past the end of the draft, or falls inside a character
    /// rather than on a `char` boundary. A panic rather than a clamp, because
    /// there is no production path here — the values this module builds itself
    /// hold the invariant by construction and do not come through here — so
    /// every caller is a test, and a test asking for an offset the draft does
    /// not have is a wrong expectation that should be loud rather than quietly
    /// answered with the nearest offset that does exist.
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

    /// Where the insertion point is: a byte offset into [`Composer::draft`], on
    /// a `char` boundary, at most the draft's length.
    ///
    /// A byte offset rather than a row and a column so that it survives a
    /// resize: the rows the draft draws as depend on the width the frame hands
    /// in, and this does not.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Tell the field how many columns it is being drawn in.
    ///
    /// Told rather than worked out, for [`Composer::set_muted`]'s reason and in
    /// its shape: the frame is cut somewhere else, so the width is somewhere
    /// else's fact, and it is handed over once a round by whoever is about to
    /// draw — [`panel_width`](crate::panel_width), which is the width the
    /// composer's own pane is drawn at.
    ///
    /// It is here because four of the keys are row-wise. Home and End are the
    /// ends of the *display* row, and Up and Down step the rows as they are
    /// drawn, so a wrapped paragraph is stepped through visually — and which row
    /// an offset is on is a question that has no answer until something has said
    /// how wide the field is. Carrying it on the value rather than passing it to
    /// [`compose_for`] keeps that function a pure function of a key and a
    /// composer, which is the shape every keyboard function in warlock has.
    ///
    /// Nothing about the draft or the cursor moves here. A width is a fact about
    /// the screen: the same bytes re-flow into different rows, and the cursor
    /// goes on being the byte offset it was, which is the whole reason it is a
    /// byte offset (see [`Composer::cursor`]).
    ///
    /// Zero until the first frame, and zero again for a composer built fresh
    /// after a submission — a column nobody has measured, which folds every line
    /// to one row, so the row-wise keys fall back to working line-wise until the
    /// next frame. The loop draws before it waits for a key, so that is a state
    /// no keystroke is ever answered in.
    pub const fn set_width(&mut self, width: u16) {
        self.width = width;
    }

    /// How many columns the field was last drawn in, and the width the keys are
    /// answered at.
    ///
    /// Zero for a composer nobody has drawn. Not the width [`Composer::height`],
    /// [`Composer::window`] and the maps take — those are asked by the frame,
    /// which knows its own width and hands it in.
    #[must_use]
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Say whether the field is taking keys.
    ///
    /// Told rather than worked out, and told by the one thread that knows: the
    /// event loop holds the turn and the run in flight, so it holds this fact,
    /// and it hands it over once a round the way it hands the app what the
    /// terminal is doing with the pointer (see
    /// [`App::set_mouse_captured`](crate::App::set_mouse_captured)).
    /// Every round rather than at the keystrokes that change it, so a turn that
    /// ended in any of its five ways, or a run that ended in any of its four,
    /// leaves a live field behind it without each of those ways having to
    /// remember to say so.
    ///
    /// Nothing about the draft moves here. A muted field is the same characters
    /// in the same order, still there when the answer lands — muting is about
    /// which keys it hears, not about what somebody has written.
    pub const fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Whether the field is muted: `true` while a turn is being answered or a
    /// pact or refresh is in flight.
    ///
    /// Asked twice a round and nowhere else. The keyboard asks so that a key
    /// pressed at a muted field neither types nor acts (see `press_for` in the
    /// binary), and the drawing asks so that the field is visibly dim while it
    /// is not listening — a border lit over a field that swallows keys would be
    /// warlock pointing at somewhere nothing happens.
    #[must_use]
    pub const fn is_muted(&self) -> bool {
        self.muted
    }

    /// What has been typed so far, newlines included.
    ///
    /// What a submit hands on, and the only text there is: [`Composed::Submit`]
    /// carries none of its own, so there is no way for a submission to disagree
    /// with the composer it came from.
    #[must_use]
    pub fn draft(&self) -> &str {
        &self.draft
    }

    /// Whether the draft would be offered up if Enter were pressed now.
    ///
    /// `false` for an empty draft and for one that is nothing but spaces,
    /// newlines and tabs. The drawing asks so it can say whether Enter means
    /// anything; [`compose_for`] asks so that it does not.
    #[must_use]
    pub fn is_submittable(&self) -> bool {
        !self.draft.trim().is_empty()
    }

    /// How many rows the composer needs at `width`.
    ///
    /// One when the draft is empty — the field is always on screen and always
    /// has a row for the cursor to sit on — and one more for every newline and
    /// every wrap, up to [`COMPOSER_MAX_ROWS`], past which the draft scrolls
    /// within the rows it has. This is the number the layout takes off the panel
    /// above, so it is asked for before the frame is cut and answered against
    /// the width that frame will draw at.
    #[must_use]
    pub fn height(&self, width: u16) -> u16 {
        // At most `COMPOSER_MAX_ROWS`, and the row count is at least one, so
        // this never truncates and never comes back zero.
        u16::try_from(self.rows(width).len())
            .unwrap_or(COMPOSER_MAX_ROWS)
            .min(COMPOSER_MAX_ROWS)
    }

    /// The rows of the draft that fit in [`Composer::height`] at `width`, in
    /// order, top row first.
    ///
    /// The *tail* of the draft's rows rather than the head, which is the whole
    /// of the scrolling this field does: typing lands at the end of the draft,
    /// so a window ending at the last row is a window the next character always
    /// appears in. A window that follows [`Composer::cursor`] instead is the
    /// drawing slice's and is not here yet. Every row when the draft is inside
    /// the cap, which is the ordinary case and is why nothing has to be reset
    /// when a long draft is backspaced short again.
    #[must_use]
    pub fn window(&self, width: u16) -> Vec<String> {
        let rows = self.rows(width);
        let from = rows.len().saturating_sub(usize::from(COMPOSER_MAX_ROWS));

        rows[from..].to_vec()
    }

    /// Every row the draft draws as at `width`, however many that is.
    ///
    /// Each line of the draft — the pieces between its newlines — broken by
    /// [`folded`], which never comes back empty, so a draft of nothing is one
    /// blank row and a draft ending in a newline has a blank row under it for
    /// the cursor to sit on. A `width` of zero is a column nobody has measured
    /// and wraps nothing, exactly as it does for a document.
    ///
    /// [`folded`] rather than [`wrapped`](crate::wrap::wrapped) because a space
    /// the reader typed is a character of the draft: the space a row breaks at
    /// stays on that row, so the rows join back up to the draft byte for byte
    /// and every byte typed has a cell of its own on screen. The row that keeps
    /// its break character is a column wider than `width`, which the drawing
    /// truncates at the pane edge.
    ///
    /// The rows of [`Composer::placed_rows`] without the offsets they start at,
    /// so there is one account of what the draft breaks into and the maps below
    /// cannot come to break it somewhere else.
    fn rows(&self, width: u16) -> Vec<String> {
        self.placed_rows(width)
            .into_iter()
            .map(|(_, row)| row)
            .collect()
    }

    /// Every row the draft draws as at `width`, each with the byte offset of the
    /// draft its first character is at.
    ///
    /// What the two maps are built on, and the only place the arithmetic between
    /// bytes and rows is done. It works because [`folded`] keeps every byte: the
    /// rows of one line join back up to that line, so a row's offset is the row
    /// before it plus that row's length. The only bytes not on any row are the
    /// `\n`s [`str::split`] took out — one between the last row of a line and
    /// the first row of the next — which is what the extra byte at the end of
    /// each line is.
    ///
    /// Never empty, because [`folded`] never comes back empty and a draft always
    /// has at least one line: there is always a row for the cursor to be on.
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

    /// Where the byte `offset` is drawn at `width`: the row of
    /// [`Composer::rows`] it is on, and the column of that row it is at.
    ///
    /// Columns are cells rather than characters — [`display_width`] of the row
    /// up to the offset — so a wide character is two columns along and a row of
    /// CJK is twice as far across as it is long. The inverse is
    /// [`Composer::offset_at`], and every offset the draft has round-trips
    /// through the pair.
    ///
    /// # The row-break rule
    ///
    /// A soft wrap is one offset with two places it could be drawn: the cell
    /// after the last character of the row above, and column zero of the row
    /// below. It is always the lower one. So an offset at a row break resolves
    /// to `(row + 1, 0)` and never to a cell past the end of the row above —
    /// which is what makes the row a cursor is on the row its next character
    /// would be drawn on, and stops End on a wrapped row parking the cursor in
    /// a cell the next keystroke would not appear in.
    ///
    /// This map keeps the rule by taking the *last* row starting at or before
    /// the offset, and [`Composer::offset_at`] keeps the same one: the cell past
    /// the end of a soft-wrapped row is the one (row, column) pair this map
    /// never comes back with, and asking the inverse for it gives the break
    /// offset, which is that same lower place.
    ///
    /// A newline is not a row break in this sense. Its own byte lies between the
    /// two rows, so the cell after the last character of the row above is the
    /// offset of the `\n` itself and belongs to that upper row — which is where
    /// a cursor before a newline should be, and where End on such a row puts it.
    ///
    /// A character that draws in no cells of its own — a combining accent — puts
    /// two offsets in one column, and this module has no grapheme segmentation
    /// to join it to the character it sits on. The pair still answers, and the
    /// inverse answers such a column with the offset after the whole run, so the
    /// offset *inside* it is the one offset that does not come back from a
    /// round trip.
    ///
    /// # Panics
    ///
    /// If `offset` is past the end of the draft or is not on a `char` boundary,
    /// which is the same thing [`Composer::at`] refuses: the cursor holds that
    /// invariant by construction, so an offset without it is a caller's mistake
    /// rather than a place on the screen.
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

    /// Which byte offset of the draft is drawn at `column` of `row` at `width`:
    /// the inverse of [`Composer::place`], and it keeps that map's row-break
    /// rule.
    ///
    /// Total, so that a caller stepping rows never has to check its arithmetic
    /// twice: a `row` past the last row is answered by the last row, and a
    /// `column` past the end of a row is answered by the offset at the end of
    /// that row — which is where End lands, and on a soft-wrapped row is the
    /// break offset. A column that falls *inside* a wide character is that
    /// character's own offset, since the cells it draws in are its.
    ///
    /// Always a `char` boundary of the draft, and never past its end.
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

/// What a keystroke comes to while the composer holds the keyboard.
///
/// Named apart from the keys that produce it for the reason [`Edited`](crate::Edited)
/// is: it keeps [`compose_for`] a pure function of a key event and leaves the
/// loop above reading as a list of consequences. Three variants is the whole of
/// what can happen to a draft *at a keystroke* — it goes on being typed into,
/// the keyboard is handed back, or the draft is offered up — and there is
/// deliberately no variant for "the key meant nothing", because a key that means
/// nothing here leaves the composer exactly where it was, which is
/// [`Composed::Typing`] with the same draft in it.
///
/// A paste is the fourth thing that can reach the draft and it does not come
/// through here: it has [`Pasted`] of its own, which can only say the first of
/// these three, so a block of text can never start a turn or hand the keyboard
/// back whatever is in it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Composed {
    /// Still being typed into: the composer keeps the keyboard, holding `.0`,
    /// which is either the draft as it was or the draft with one character more
    /// or less.
    Typing(Composer),
    /// The keyboard is handed back: focus moves off the composer and the draft
    /// is left exactly as it is. Esc here is not an abandonment — nothing is
    /// thrown away, because what was typed is worth more than the keystroke that
    /// stopped typing it.
    Leave,
    /// Offered up: the caller takes [`Composer::draft`] from the composer it is
    /// holding and does whatever a submitted draft is for.
    ///
    /// It carries no text of its own, so there is one copy of what was typed and
    /// no way for a submission to disagree with the field it came from. It
    /// arrives only for a draft with something in it: see
    /// [`Composer::is_submittable`].
    Submit,
}

/// What `key` does to a composer holding `composer`.
///
/// The counterpart of [`edit_for`](crate::edit_for) and of the binary's
/// `action_for`, and the same shape: a key and a situation in, one intention
/// out, no terminal and no [`App`](crate::App).
///
/// Five things a key can be. A printable character appends — and *only* appends,
/// since editing at the cursor is not built yet. Backspace takes the last
/// character back, by character rather than by byte, so a draft with an accent
/// or an emoji in it is still deletable one keypress at a time; on an empty
/// draft it does nothing rather than handing the keyboard back, because a
/// Backspace one press past the start is a typo and not a departure. Alt+Enter
/// puts a newline in. Enter offers up a draft that has something in it, and does
/// nothing at all to one that is empty or nothing but whitespace. Esc hands the
/// keyboard back and leaves the draft where it is.
///
/// # The six keys that move the cursor
///
/// Left, Right, Home, End, Up and Down change no byte of the draft. Every one of
/// them comes back with the same string carrying a different
/// [`Composer::cursor`], which is why they are a group rather than six rules:
/// what they move is where the *next* thing typed will go, and nothing here
/// types anything.
///
/// Left and Right step one `char` — not one byte, since a byte of an accent is
/// not a place in the draft — and stop dead at offset 0 and at the end rather
/// than wrapping round to the other end of the draft.
///
/// The other four are row-wise, and a row is a row *as drawn*: they are answered
/// at [`Composer::width`], the width the field was last drawn in, over
/// [`Composer::place`] and [`Composer::offset_at`]. So a paragraph that soft-wraps
/// into three rows is stepped through in three, which is what somebody pressing
/// Down at a wrapped paragraph is asking for; and because the cursor is a byte
/// offset, the same keystroke at a resized terminal steps the rows that terminal
/// has. Home and End go to the two ends of the row the cursor is on. Up and Down
/// go one row, holding the column where the destination row is long enough for
/// it and landing at the end of that row where it is not — and on the first row
/// Up does nothing, as Down does on the last, because there is no row past the
/// draft to sit on.
///
/// Both ends of a row obey [`Composer::place`]'s row-break rule, which is the
/// one place this can surprise: the end of a *soft-wrapped* row is the break
/// offset, and the break offset is drawn at column zero of the row below. So End
/// on such a row leaves the cursor looking like it moved down one, and Home
/// straight after it comes back to the same offset — End then Home is a no-op
/// there. That is the rule holding rather than failing: the cursor sits where
/// the next character it types would be drawn, and on a wrapped row that cell is
/// on the row below.
///
/// Every other key leaves the composer exactly as it was, the tree's own
/// bindings included — while this has the keyboard, `j`, `k`, `g`, `G`, `f`,
/// `p`, `r`, `s`, `v` and `e` are letters somebody is typing, and Delete, Insert,
/// `BackTab` and the page keys are nothing at all, because the loop consults
/// this instead of the app rather than as well as it. No movement key can submit
/// or leave: Enter is the one key that offers the draft up and Esc the one that
/// hands the keyboard back.
///
/// Only presses count, exactly as `action_for` and [`edit_for`](crate::edit_for)
/// have it. Crossterm reports releases and auto-repeats on some platforms and
/// not on others, and a release acted on here would type the release of the very
/// key that moved the focus into the field.
#[must_use]
pub fn compose_for(key: KeyEvent, composer: &Composer) -> Composed {
    if key.kind != KeyEventKind::Press {
        return Composed::Typing(composer.clone());
    }

    // The incoming cursor comes through untouched, along with the draft and the
    // flag: a key that changes nothing moves nothing.
    let unchanged = || Composed::Typing(composer.clone());
    // Muted or not comes through with the draft, and so does the width: this
    // function is not where a turn starts or ends, and it is not a redraw. The
    // cursor does not come through — a draft that changed here changed at its
    // end, so the insertion point snaps there. Built literally rather than
    // through `Composer::at`, since `draft.len()` is a boundary by
    // construction.
    let typed = |draft: String| {
        Composed::Typing(Composer {
            cursor: draft.len(),
            draft,
            width: composer.width,
            muted: composer.muted,
        })
    };
    // The other way round for the movement keys: the draft, the width and the
    // flag come through and only the offset is new. Every offset handed to this
    // comes off `char_indices`, `Composer::place` or `Composer::offset_at`, all
    // three of which answer with boundaries of this draft, so the invariant
    // holds by construction here too.
    let moved = |cursor: usize| {
        Composed::Typing(Composer {
            draft: composer.draft.clone(),
            cursor,
            width: composer.width,
            muted: composer.muted,
        })
    };
    // Where the cursor is on screen, at the width the field was last drawn in:
    // what the four row-wise keys work from. A closure rather than a value
    // because working it out folds the whole draft into rows, and the other
    // keys — every character typed among them — do not ask.
    let placed = || composer.place(composer.cursor, composer.width);

    match key.code {
        // Before the plain Enter below it, which is the point of the pair: the
        // modifier is what tells a new line from a submission.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
            let mut draft = composer.draft.clone();
            draft.push('\n');
            typed(draft)
        }
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
        KeyCode::Backspace => {
            let mut draft = composer.draft.clone();
            // `pop` takes a whole character, not a byte: half a character left
            // in the buffer would not be a `String` at all.
            if draft.pop().is_some() {
                typed(draft)
            } else {
                unchanged()
            }
        }
        // The six that move the cursor and change no byte. Left and Right by
        // `char`: the offset before the one the cursor is at, and the offset
        // after it, or the end they are already at.
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
        // The two ends of the row the cursor is on. `offset_at` is total, so the
        // column past every column there is answers with the end of the row —
        // which on a soft-wrapped row is the break offset, drawn at column zero
        // of the row below, so End then Home there comes back here.
        KeyCode::Home => moved(composer.offset_at(placed().0, 0, composer.width)),
        KeyCode::End => moved(composer.offset_at(placed().0, usize::MAX, composer.width)),
        // One display row, holding the column: `offset_at` lands at the end of
        // the destination row when that row is not long enough to hold it. The
        // row past the ends is not a row, so the first row's Up and the last
        // row's Down leave the cursor exactly where it is rather than clamping
        // to the row it is already on.
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
            let mut draft = composer.draft.clone();
            draft.push(character);
            typed(draft)
        }
        _ => unchanged(),
    }
}

/// What a pasted block comes to at the composer: the one thing it can be.
///
/// A type rather than a third [`Composed`] variant, and one variant rather than
/// three, because the promise being kept is a negative one — a paste cannot
/// submit and cannot hand the keyboard back, however many newlines are in it —
/// and a negative promise held by the return type is one nobody at a call site
/// can forget. Pasting a three-line block used to send line one as a question
/// and lose the other two behind the mute that turn put up; there is now no
/// value [`paste_for`] could return that would say "send".
///
/// It reads as a `match` with a single arm, or as a `let`, since one variant is
/// an irrefutable pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Pasted {
    /// Still being typed into: the composer keeps the keyboard, holding `.0`,
    /// which is the draft with the pasted text in it, or the draft exactly as it
    /// was if the field was muted or the paste was empty.
    Typing(Composer),
}

/// What pasting `text` does to a composer holding `composer`.
///
/// [`compose_for`]'s counterpart for the other way text arrives, and the same
/// shape: what came in and the situation go in, one intention comes out, no
/// terminal and no [`App`](crate::App). The block is inserted at the insertion
/// point — the end of the draft, where typing goes and the only place this field
/// has — byte for byte, newlines and all, so a paragraph pasted in is the
/// paragraph that was copied and every line of it is still there to be read
/// before Enter is pressed. Pasting text with no newline in it leaves the same
/// draft behind as typing those characters one at a time would.
///
/// A muted field takes nothing: the draft comes back the string it was and the
/// flag comes back up. That is the one rule [`compose_for`] leaves to the gate
/// above it and this does not, because a paste is one arrival carrying however
/// much was copied rather than a key somebody can stop pressing, and because
/// bytes the terminal delivered while an answer is in flight are not somebody
/// typing at this field. Muting itself is still set nowhere near here — one
/// question at a time is the loop's fact, and it stays the loop's fact.
///
/// Nothing else moves. No turn starts, no focus changes, and an empty paste is
/// a paste that changes nothing rather than an error anybody has to hear about.
/// The cursor ends up at the end of the draft the paste left behind — the same
/// place typing those characters would have left it, and for as long as the end
/// is the only insertion point this field has, the same place it started.
#[must_use]
pub fn paste_for(text: &str, composer: &Composer) -> Pasted {
    if composer.muted {
        return Pasted::Typing(composer.clone());
    }

    let mut draft = composer.draft.clone();
    draft.push_str(text);

    // Muted or not comes through with the draft, and so does the width, exactly
    // as a keystroke has it: this function is not where a turn starts or ends,
    // and it is not a redraw. The cursor snaps to the end of what the paste left
    // behind, exactly as typing the same characters would leave it.
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

    /// A plain press of `code`, as crossterm reports one with no modifiers.
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// The newline keystroke: Enter wearing Alt.
    fn alt_enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)
    }

    /// A composer holding `draft`.
    fn composer(draft: &str) -> Composer {
        Composer::new(draft)
    }

    /// A composer holding `draft`, last drawn `width` columns wide.
    ///
    /// What the four row-wise keys are answered against: a row is only a row
    /// once something has said how wide the field is, and in the binary that is
    /// the frame, once a round. Here it is one line, and no terminal.
    fn drawn(draft: &str, width: u16) -> Composer {
        let mut composer = Composer::new(draft);
        composer.set_width(width);

        composer
    }

    /// Where `key` leaves the cursor, having first insisted that it left every
    /// byte of the draft, the width and the muting exactly as they were.
    ///
    /// The movement keys are asserted through this rather than by comparing
    /// whole values, because the offset is the only thing they are allowed to
    /// change and this is the assertion that says so at every one of them.
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

    /// The composer `key` leaves behind, or a panic naming what came out
    /// instead: most rules here are about what is still in the draft afterwards.
    fn after(key: KeyEvent, composer: &Composer) -> Composer {
        match compose_for(key, composer) {
            Composed::Typing(next) => next,
            other => panic!("{key:?} should have kept the keyboard, and gave {other:?}"),
        }
    }

    /// The composer a paste of `text` leaves behind. One arm, because
    /// [`Pasted`] has one variant — which is the rule being kept rather than a
    /// convenience.
    fn pasted(text: &str, composer: &Composer) -> Composer {
        let Pasted::Typing(next) = paste_for(text, composer);

        next
    }

    /// Every key the tree answers to, plus a character bound to nothing
    /// anywhere: the list the composer has to swallow whole, so no keystroke
    /// reaches the app underneath while the field has the keyboard.
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

    /// The six keys that move the cursor: they change no byte of the draft, and
    /// none of them submits or leaves.
    const MOVERS: [KeyCode; 6] = [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Home,
        KeyCode::End,
    ];

    /// The ones of [`BINDINGS`] that are characters, i.e. the ones that have to
    /// come out as text rather than as nothing.
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

    /// A draft that changed changed at its end, so the cursor snaps there.
    ///
    /// Transient, and deliberately one named test so that it is one thing to
    /// delete: the editing slice — slice 4 — moves typing, Alt+Enter, Backspace,
    /// Delete and paste onto the cursor, and **this test is expected to be
    /// deleted with them**. Until then the end is the only insertion point the
    /// field has, so a draft that changed anywhere changed there — and since
    /// the cursor is part of the value, a changed draft carrying a stale offset
    /// would break the whole-value `Composer::new(...)` comparisons the rest of
    /// the crate is written with.
    #[test]
    fn every_key_that_changes_the_draft_leaves_the_cursor_at_the_end() {
        let from = composer("one\ntwo").at(2);

        for (what, next) in [
            (
                "a printable character",
                after(press(KeyCode::Char('x')), &from),
            ),
            ("Alt+Enter", after(alt_enter(), &from)),
            ("Backspace", after(press(KeyCode::Backspace), &from)),
            ("a paste", pasted("and\nmore", &from)),
        ] {
            assert_eq!(
                next.cursor(),
                next.draft().len(),
                "{what} should have left the cursor at the end of {:?}",
                next.draft()
            );
        }
    }

    #[test]
    fn a_key_that_changes_nothing_moves_the_cursor_nowhere() {
        // The other half: the incoming offset comes through untouched, so a
        // key this field does not have is not a key that quietly re-homes the
        // insertion point.
        let before = composer("one\ntwo").at(2);

        for code in [KeyCode::Delete, KeyCode::Insert, KeyCode::BackTab] {
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
        assert_eq!(muted.window(18), live.window(18));
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
    fn backspace_takes_back_one_character_at_a_time() {
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("web")),
            composer("we")
        );
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("w")),
            composer("")
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
    }

    #[test]
    fn backspace_takes_a_newline_back_like_any_other_character() {
        assert_eq!(
            after(press(KeyCode::Backspace), &composer("one\n")),
            composer("one")
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
    fn enter_on_an_empty_or_blank_draft_does_nothing_at_all() {
        // Not a refusal with a complaint on the footer: a submission with
        // nothing in it is a keystroke, not a mistake, so it leaves the draft
        // exactly as it was and says nothing.
        for draft in ["", " ", "   \t  ", "\n", " \n \n "] {
            let before = composer(draft);

            assert_eq!(
                compose_for(press(KeyCode::Enter), &before),
                Composed::Typing(before.clone()),
                "Enter should have done nothing to {draft:?}"
            );
            assert!(!before.is_submittable());
        }
    }

    #[test]
    fn esc_hands_the_keyboard_back_from_any_draft() {
        for draft in ["", "half a question", "one\ntwo"] {
            assert_eq!(
                compose_for(press(KeyCode::Esc), &composer(draft)),
                Composed::Leave,
                "Esc should leave from {draft:?}"
            );
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
        // plus a control character arriving as itself.
        let before = composer("web");

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
            compose_for(shift_enter, &composer("")),
            Composed::Typing(composer("")),
            "and on an empty draft it means what Enter means there too"
        );
    }

    #[test]
    fn releases_and_repeats_change_nothing() {
        // The same rule `action_for` and `edit_for` keep: acting on a release
        // would type the release of the very key that moved the focus here. The
        // movers are in the list on a value each of them would move from, so
        // "nothing happened" is a rule about the kind of the event rather than
        // about a draft that had nowhere to go.
        let before = drawn("one\ntwo\nsix", 40).at(5);

        for code in [
            KeyCode::Char('s'),
            KeyCode::Backspace,
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
        // somewhere else and leave every byte where it was, and Delete, Insert
        // and BackTab go on doing nothing at all — no selection, no history and
        // no editing away from the end of the draft.
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

        for code in [KeyCode::Delete, KeyCode::Insert, KeyCode::BackTab] {
            assert_eq!(
                compose_for(press(code), &before),
                Composed::Typing(before.clone()),
                "{code:?} is not an editing key this field has"
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
        assert_eq!(wrapped.window(4), ["one ", "two"]);

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
        assert_eq!(wrapped.window(4), ["one ", "two ", "six"]);

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
        // the cap and the window is the last rows, so the end of what was
        // pasted — where the next character will go — is what is on screen.
        let block = (1..=20)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let next = pasted(&block, &composer(""));

        assert_eq!(next.height(40), COMPOSER_MAX_ROWS);
        assert_eq!(
            next.window(40),
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
            assert_eq!(composer("").window(width), [""]);
        }
    }

    #[test]
    fn a_draft_inside_the_width_is_one_row_tall() {
        assert_eq!(composer("why nine passes").height(40), 1);
        assert_eq!(composer("why nine passes").window(40), ["why nine passes"]);
    }

    #[test]
    fn a_newline_is_one_row_more() {
        assert_eq!(composer("one").height(40), 1);
        assert_eq!(composer("one\n").height(40), 2);
        assert_eq!(composer("one\ntwo").height(40), 2);
        assert_eq!(composer("one\ntwo\n").height(40), 3);
        assert_eq!(composer("one\ntwo\nthree").height(40), 3);

        assert_eq!(composer("one\ntwo").window(40), ["one", "two"]);
        assert_eq!(
            composer("one\n").window(40),
            ["one", ""],
            "the row after a newline is where the cursor is sitting"
        );
    }

    #[test]
    fn a_wrap_is_one_row_more_and_the_rows_are_the_words_in_order() {
        let rows = composer("It walks the tree and writes what it finds.").window(18);

        // The space each row broke at is kept, on the row above the break: the
        // draft is text somebody is still typing, so every space typed has a
        // cell of its own, and the rows join back up to the draft byte for
        // byte. The row that kept one overhangs the width by that column, and
        // the drawing truncates at the pane edge.
        assert_eq!(
            rows,
            ["It walks the tree ", "and writes what it ", "finds."]
        );
        assert_eq!(rows.concat(), "It walks the tree and writes what it finds.");
        assert_eq!(
            composer("It walks the tree and writes what it finds.").height(18),
            3
        );
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
        assert_eq!(long.window(40).len(), usize::from(COMPOSER_MAX_ROWS));

        let wide = composer(&"word ".repeat(200));

        assert_eq!(wide.height(20), COMPOSER_MAX_ROWS);
        assert_eq!(wide.window(20).len(), usize::from(COMPOSER_MAX_ROWS));
    }

    #[test]
    fn the_window_is_the_tail_so_the_cursor_line_is_always_in_it() {
        // The cursor is after the last character, so its row is the last row:
        // a window that ends there is a window it is in.
        let draft = (1..=20)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let composer = composer(&draft);
        let window = composer.window(40);

        assert_eq!(
            window,
            ["line 16", "line 17", "line 18", "line 19", "line 20", "",]
        );
        assert_eq!(
            window.last().map(String::as_str),
            Some(""),
            "the last row is the one the cursor is on"
        );
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
            assert_eq!(composer.window(40).len(), lines);
        }
    }

    #[test]
    fn a_column_nobody_has_measured_wraps_nothing() {
        // A width of zero is the state of a composer between being built and
        // being drawn, and is not a width to break at — the same rule the
        // panel's own wrapping keeps.
        let long = "It walks the tree and writes what it finds.";

        assert_eq!(composer(long).height(0), 1);
        assert_eq!(composer(long).window(0), [long]);
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
                assert_eq!(usize::from(height), composer(draft).window(width).len());
            }
        }
    }

    /// The drafts the two maps are driven over: a draft with a newline in it, a
    /// draft long enough to soft-wrap at every width tested, drafts of
    /// multi-byte characters wide and narrow, and the two edges — nothing at
    /// all, and a draft ending in a newline.
    ///
    /// The accent is precomposed (`U+00E9`) rather than an `e` and a combining
    /// mark: a combining mark draws in no cells of its own, so it would put two
    /// offsets in one column, and joining it to the character it sits on would
    /// need grapheme segmentation this module deliberately does not have — see
    /// [`Composer::place`].
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
        assert_eq!(wrapped.window(4), ["one ", "two"]);

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
