//! The composer: the several-line draft at the foot of the panel's column, and
//! the one pure function saying what a key does to it.
//!
//! Every warlock command is a single letter — `p` pacts, `r` refreshes, `s`
//! scopes, `v` views — so the moment a field is on screen and holding the
//! keyboard, `p` has to be able to mean the letter p. This module is the field
//! that makes that true. It is [`prompt`](crate::prompt)'s shape grown to
//! several lines: a value holding what has been typed, and
//! [`compose_for`], which takes a key event and that value and comes back with
//! one of three consequences.
//!
//! ## A buffer, and deliberately nothing more
//!
//! Printable characters append, Backspace takes the last character back,
//! Alt+Enter starts a new line, Enter offers the draft up and Esc hands the
//! keyboard back. That is the entire editor. There is no history, no selection,
//! no completion and no arrow-key editing, so the cursor is always after the
//! last character of the last row and is therefore not a field anybody has to
//! keep true — whoever draws the composer puts the cursor at the end and is
//! right by construction. The arrows, Home, End, Delete, Insert and `BackTab`
//! leave the draft byte-for-byte as it was, exactly as
//! [`edit_for`](crate::edit_for) has it, because every key a mover claims is a
//! key that cannot be one of the characters being typed.
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
//! rather than the head, because the cursor is at the end — a window that
//! followed the top of the draft would scroll the thing being typed off the
//! bottom of itself.
//!
//! Rows are counted with [`wrapped`](crate::wrap), the same wrapper the panel
//! breaks a document's lines with, so a row that fits here fits when it is
//! drawn and the composer never asks for a height the frame then disagrees with.
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
//! Muting, too. A field is muted for as long as the answer to the last question
//! is on its way, and for no other reason — one question at a time. That is
//! somewhere else: the loop owns the turn, so it owns the flag
//! ([`Composer::set_muted`]), and the keyboard's gate is what declines to ask
//! this function anything while the flag is up. So [`compose_for`] behaves
//! identically either way and simply carries the flag through, which is what
//! keeps "what a key does to a draft" one set of rules rather than two.
//!
//! Where the draft is kept between keystrokes, which pane has the focus, and
//! what a submitted draft is *for* are all somebody else's business. Nothing
//! here reads a terminal, draws anything, or takes an [`App`](crate::App): a key
//! event and the current draft go in, and one of three consequences comes out,
//! so every rule below is one assertion with nothing attached to stdout.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::wrap::wrapped;

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

/// What has been typed into the composer, with the cursor after its last
/// character.
///
/// One string and one flag. There is no cursor field because nothing can
/// move a cursor, no scroll offset because the window is always the tail (see
/// [`Composer::window`]), and no width because the width belongs to the frame
/// and is handed in by whoever is drawing — so a composer can be driven through
/// every width a terminal has in one test without a terminal.
///
/// The flag is [`Composer::is_muted`], and it is a fact about the session rather
/// than about the draft: one question at a time, so while an answer is on its
/// way the field takes no keys and is drawn to say so. Which is also why it is
/// carried here rather than worked out where it is read — the loop knows whether
/// a turn is in flight, and it tells the field once a round, exactly as it tells
/// the app what the terminal is doing with the pointer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Composer {
    /// What has been typed. Newlines are in it as `\n`; the cursor sits after
    /// its last character, always.
    draft: String,
    /// Whether the field is taking keys at all. `false` for the whole of a
    /// session that never asks anything and never runs a pact; `true` only for
    /// as long as a turn is being answered, or a run writing documents,
    /// somewhere else.
    muted: bool,
}

impl Composer {
    /// A composer holding `draft`, live.
    ///
    /// [`Composer::default`] is the empty one a session starts on. This is for
    /// putting a composer back where it was — and for tests, which is most of
    /// what a value this small wants a constructor for. Muting is not a
    /// constructor's business: it is set and unset as turns and runs come and
    /// go, by [`Composer::set_muted`].
    #[must_use]
    pub fn new(draft: impl Into<String>) -> Self {
        Self {
            draft: draft.into(),
            muted: false,
        }
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

    /// What has been typed so far, newlines included, with the cursor at its
    /// end.
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
    /// of the scrolling this field does: the cursor is after the last character,
    /// so the row it is on is the last row, so a window ending at the last row
    /// is a window the cursor is always in. Every row when the draft is inside
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
    /// [`wrapped`], which never comes back empty, so a draft of nothing is one
    /// blank row and a draft ending in a newline has a blank row under it for
    /// the cursor to sit on. A `width` of zero is a column nobody has measured
    /// and wraps nothing, exactly as it does for a document.
    fn rows(&self, width: u16) -> Vec<String> {
        self.draft
            .split('\n')
            .flat_map(|line| wrapped(line, usize::from(width)))
            .collect()
    }
}

/// What a keystroke comes to while the composer holds the keyboard.
///
/// Named apart from the keys that produce it for the reason [`Edited`](crate::Edited)
/// is: it keeps [`compose_for`] a pure function of a key event and leaves the
/// loop above reading as a list of consequences. Three variants is the whole of
/// what can happen to a draft — it goes on being typed into, the keyboard is
/// handed back, or the draft is offered up — and there is deliberately no
/// variant for "the key meant nothing", because a key that means nothing here
/// leaves the composer exactly where it was, which is [`Composed::Typing`] with
/// the same draft in it.
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
/// since the cursor is at the end and nothing moves it. Backspace takes the last
/// character back, by character rather than by byte, so a draft with an accent
/// or an emoji in it is still deletable one keypress at a time; on an empty
/// draft it does nothing rather than handing the keyboard back, because a
/// Backspace one press past the start is a typo and not a departure. Alt+Enter
/// puts a newline in. Enter offers up a draft that has something in it, and does
/// nothing at all to one that is empty or nothing but whitespace. Esc hands the
/// keyboard back and leaves the draft where it is.
///
/// Every other key leaves the draft byte-for-byte as it was, the tree's own
/// bindings included — while this has the keyboard, `j`, `k`, `g`, `G`, `f`,
/// `p`, `r`, `s`, `v` and `e` are letters somebody is typing, and the arrows and
/// the page keys are nothing at all, because the loop consults this instead of
/// the app rather than as well as it.
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

    let unchanged = || Composed::Typing(composer.clone());
    // Muted or not comes through with the draft: this function is not where a
    // turn starts or ends, so a field that arrived muted goes back muted.
    let typed = |draft| {
        Composed::Typing(Composer {
            draft,
            muted: composer.muted,
        })
    };

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

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use super::{COMPOSER_MAX_ROWS, Composed, Composer, compose_for};

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

    /// The composer `key` leaves behind, or a panic naming what came out
    /// instead: most rules here are about what is still in the draft afterwards.
    fn after(key: KeyEvent, composer: &Composer) -> Composer {
        match compose_for(key, composer) {
            Composed::Typing(next) => next,
            other => panic!("{key:?} should have kept the keyboard, and gave {other:?}"),
        }
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
        // would type the release of the very key that moved the focus here.
        let before = composer("web");

        for code in [
            KeyCode::Char('s'),
            KeyCode::Backspace,
            KeyCode::Enter,
            KeyCode::Esc,
        ] {
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
    fn the_movement_and_editing_keys_this_field_does_not_have_do_nothing() {
        // No selection, no history, no left-arrow editing: the cursor is at the
        // end because nothing here can put it anywhere else. Asserted on a draft
        // with a newline in it, since a mover that did work would show up as a
        // changed line rather than a changed character.
        let before = composer("one\ntwo");

        for code in [
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Delete,
            KeyCode::Insert,
            KeyCode::BackTab,
            KeyCode::Up,
            KeyCode::Down,
        ] {
            assert_eq!(
                compose_for(press(code), &before),
                Composed::Typing(before.clone()),
                "{code:?} is not an editing key this field has"
            );
            assert_eq!(
                after(press(code), &before).draft(),
                "one\ntwo",
                "{code:?} should leave the draft byte for byte"
            );
        }
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

        assert_eq!(rows, ["It walks the tree", "and writes what it", "finds."]);
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
}
