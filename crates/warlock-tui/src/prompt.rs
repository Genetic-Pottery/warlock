//! The scope prompt: which directory is being scoped, what has been typed into
//! the field, and the one line under it saying why the last submit was refused.
//!
//! A scope is a fact about a directory — `data-plane`, `billing`, `web` —
//! committed in `.warlock/pacts.toml` beside the module's document. Setting one
//! used to mean opening the manifest in an editor. This module is the whole of
//! what the `s` key opens instead: a mode with a text field, and one pure
//! function saying what a key does to it.
//!
//! ## The shape, because the drawing and the loop both depend on it
//!
//! The mode is a value of its own — [`ScopePrompt`] — and *not* a field on
//! [`App`](crate::App), for the reasons [`QuitConfirm`](crate::QuitConfirm) is
//! not one either. Esc has to leave the app exactly as it was, and the cheapest
//! way to be sure of that is for the app never to have heard of the prompt: an
//! `App` compared before opening and after closing is equal because nothing
//! about it was touched, rather than because every field was carefully put back.
//! And a half-typed scope is not state about the tree — it is state about this
//! keystroke and the next one — so keeping it out of `App` keeps `App` the
//! answer to "what is warlock showing" rather than "what is warlock in the
//! middle of asking".
//!
//! ## A buffer, and deliberately nothing more
//!
//! Printable characters append, Backspace takes the last character back, Enter
//! submits and Esc closes. That is the entire editor. There is no history, no
//! completion, no selection and no arrow-key editing, so the cursor is always at
//! the end of the text and is therefore not a field anybody has to keep true —
//! whoever draws the field puts the cursor after the last character and is
//! right by construction. A scope is at most twenty-four characters of lowercase
//! letters, digits, `-` and `_`; a line editor for a string that short is more
//! code than the thing it edits, and every key it would claim is a key that
//! could instead be one of the characters being typed.
//!
//! ## This module judges nothing
//!
//! [`edit_for`] never asks whether the text is a scope. Enter comes back as
//! [`Edited::Submit`] whatever has been typed, including nothing at all — an
//! empty field is how a scope is *cleared*, so it is a legitimate answer rather
//! than an error — and the caller puts the question to the engine's
//! [`validate_scope`](warlock_engine::validate_scope), which is the workspace's
//! only judge of what a scope may be. A refusal comes back here as a
//! [`String`] the caller hands to [`ScopeField::refused`], so the field reopens
//! with the same text and one line underneath it. Nothing in this crate knows
//! how long a scope may be or which characters it may hold, and this module is
//! not the exception.
//!
//! ## What this deliberately does not answer
//!
//! Ctrl-C. It is a key event and not a signal — raw mode is exactly the mode in
//! which the terminal stops turning it into `SIGINT` — so it has to be answered
//! by the event loop *before* the loop consults this mode, both with the prompt
//! closed and with it open. Through here it is one of the keys that change
//! nothing, because a character carrying Ctrl is not text somebody typed; if it
//! were treated as text, the one keystroke every reader trusts to get them out
//! would put a `c` in the field.
//!
//! Nothing here reads a terminal, draws anything, or takes an
//! [`App`](crate::App): a key event and the open field go in, and one of three
//! consequences comes out, so every rule below is one assertion with nothing
//! attached to stdout.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// The modifiers that mean a character is a command rather than something
/// somebody typed.
///
/// Shift is deliberately absent: an upper-case letter arrives with it on some
/// terminals and without it on others, and either way it is text. Everything
/// here is a chord — Ctrl-C first among them — and a chord is not text, so a
/// character wearing one of these is left to the loop above rather than put in
/// the field.
const CHORD: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::HYPER)
    .union(KeyModifiers::META);

/// The field while the prompt is up: what is being scoped, what has been typed,
/// and why the last submit was refused.
///
/// One value rather than three parallel variables in the event loop, because
/// the three are only ever meaningful together — there is no directory being
/// scoped without a field to type in, and no rule line without a submit that
/// broke one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ScopeField {
    /// The directory being scoped, as the window names it. A manifest path
    /// (`"crates/warlock-engine"`, or `"."` for the root) in every real use, but
    /// held as a plain [`String`] because this module only ever prints it.
    directory: String,
    /// What has been typed. The cursor sits after its last character, always.
    text: String,
    /// The rule the last submit broke, if one did: one line, put here by the
    /// caller after the engine refused, and worded by the engine.
    rule: Option<String>,
}

impl ScopeField {
    /// A field for `directory`, opened on `text`.
    ///
    /// `text` is the scope the directory carries right now — read from the
    /// manifest by the caller — or the empty string when it carries none, so
    /// the prompt opens on what is true rather than on a blank that would
    /// silently clear it. No rule line: nothing has been submitted yet, and a
    /// prompt that opens already complaining is complaining about somebody
    /// else's typing.
    #[must_use]
    pub fn new(directory: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            directory: directory.into(),
            text: text.into(),
            rule: None,
        }
    }

    /// The same field with `rule` under it: what the caller reopens the prompt
    /// with when the engine refused the submit.
    ///
    /// The text is untouched, which is the point — a refusal leaves whoever
    /// typed it looking at what they typed, one character away from fixing it,
    /// rather than at an empty field and a complaint about a string that is no
    /// longer on screen.
    #[must_use]
    pub fn refused(self, rule: impl Into<String>) -> Self {
        Self {
            rule: Some(rule.into()),
            ..self
        }
    }

    /// The directory being scoped, for the window's heading.
    #[must_use]
    pub fn directory(&self) -> &str {
        &self.directory
    }

    /// What has been typed so far, and what a submit hands to the engine. The
    /// cursor is at its end.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The line under the field, or `None` when nothing has been refused.
    #[must_use]
    pub fn rule(&self) -> Option<&str> {
        self.rule.as_deref()
    }
}

/// Whether the scope prompt is up, and the field being typed into while it is.
///
/// The field lives inside [`ScopePrompt::Open`] rather than beside a `bool`, so
/// "closed, with half a scope typed" is not a state that can be written down:
/// there is one place the text can be, and it exists exactly as long as the
/// question does.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum ScopePrompt {
    /// No scope is being asked for. Keys mean what they have always meant, and
    /// there is nothing over the frame.
    #[default]
    Closed,
    /// The prompt is on screen, over `.0`.
    Open(ScopeField),
}

impl ScopePrompt {
    /// The prompt as it opens: up, over `directory`, holding the scope it
    /// carries now.
    ///
    /// A named constructor rather than `Open(ScopeField::new(..))` written at
    /// the call site, for the reason [`QuitConfirm::open`](crate::QuitConfirm)
    /// is one: what a fresh prompt starts on is decided here rather than
    /// wherever the `s` key happens to be handled.
    #[must_use]
    pub fn open(directory: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Open(ScopeField::new(directory, text))
    }

    /// Whether a scope is being asked for right now.
    ///
    /// What the event loop branches on before it consults [`edit_for`], and what
    /// the renderer branches on before it draws anything over the frame.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    /// The field being typed into, or `None` when nothing is being asked.
    ///
    /// The one way into [`edit_for`]: an `Option` here is what keeps the key
    /// handler from having to invent a field for a prompt that is not up.
    #[must_use]
    pub const fn field(&self) -> Option<&ScopeField> {
        match self {
            Self::Closed => None,
            Self::Open(field) => Some(field),
        }
    }
}

/// What a keystroke comes to while the scope prompt is open.
///
/// Named apart from the keys that produce it for the reason the binary's
/// `Action` is: it keeps [`edit_for`] a pure function of a key event, and leaves
/// the loop above reading as a list of consequences. Three variants is the whole
/// of what can happen to a field — it goes on being typed into, it is abandoned,
/// or it is offered up — and there is deliberately no variant for "the key meant
/// nothing", because a key that means nothing here leaves the prompt exactly
/// where it was, which is [`Edited::Open`] with the same field in it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Edited {
    /// Still being typed into: the prompt stays up over `.0`, which is either
    /// the field as it was or the field with one character more or less.
    Open(ScopeField),
    /// Abandoned: take the prompt down and change nothing. Nothing was written
    /// and the app was never touched, so there is nothing to put back.
    Close,
    /// Offered up: the caller takes [`ScopeField::text`] from the field it is
    /// holding, asks the engine whether it is a scope, and either writes the
    /// manifest or reopens the prompt through [`ScopeField::refused`].
    ///
    /// It carries no text of its own, so there is one copy of what was typed
    /// and no way for a submit to disagree with the field it came from.
    Submit,
}

/// What `key` does to a prompt open over `field`.
///
/// The counterpart of the binary's `action_for` and of
/// [`answer_for`](crate::answer_for), and the same shape: a key and a situation
/// in, one intention out, no terminal and no [`App`](crate::App).
///
/// Four things a key can be. A printable character appends — and *only* appends,
/// since the cursor is at the end and nothing moves it. Backspace takes the last
/// character back, by character rather than by byte, so a name typed in a script
/// warlock will refuse anyway is still deletable one keypress at a time; on an
/// empty field it does nothing rather than closing the prompt, because a
/// Backspace one press past the start is a typo and not an abandonment. Enter
/// submits whatever is there, empty included — that is how a scope is cleared.
/// Esc closes and changes nothing.
///
/// An edit clears the rule line. The line names a rule the *text* broke, and the
/// text has just changed, so leaving it up would have the window complaining
/// about a string that is no longer in the field. A key that changes nothing
/// leaves it exactly where it was, complaint and all.
///
/// Every other key leaves the field byte-for-byte as it was, including the
/// tree's own bindings — while this is up, `j`, `k`, `g`, `G`, space, `o`, `f`,
/// `p`, `r` and `m` are letters somebody is typing, and Tab and the page keys
/// are nothing at all, because the loop consults this instead of the app rather
/// than as well as it.
///
/// Only presses count, exactly as `action_for` has it. Crossterm reports
/// releases and auto-repeats on some platforms and not on others, and a release
/// acted on here would type the release of the very key that opened the prompt.
#[must_use]
pub fn edit_for(key: KeyEvent, field: &ScopeField) -> Edited {
    if key.kind != KeyEventKind::Press {
        return Edited::Open(field.clone());
    }

    let unchanged = || Edited::Open(field.clone());
    let edited = |text| {
        Edited::Open(ScopeField {
            directory: field.directory.clone(),
            text,
            rule: None,
        })
    };

    match key.code {
        KeyCode::Enter => Edited::Submit,
        KeyCode::Esc => Edited::Close,
        KeyCode::Backspace => {
            let mut text = field.text.clone();
            // `pop` takes a whole character, not a byte: half a character left
            // in the buffer would not be a `String` at all.
            if text.pop().is_some() {
                edited(text)
            } else {
                unchanged()
            }
        }
        // A chord is a command somebody sent, not a character somebody typed,
        // and control characters are not text however they arrived — Ctrl-C
        // among them, which the loop above has already had its chance at.
        KeyCode::Char(character) if !key.modifiers.intersects(CHORD) && !character.is_control() => {
            let mut text = field.text.clone();
            text.push(character);
            edited(text)
        }
        _ => unchanged(),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    use super::{Edited, ScopeField, ScopePrompt, edit_for};

    /// The directory every field below is opened over, so a test that meant to
    /// change the text cannot pass by changing this instead.
    const DIRECTORY: &str = "crates/warlock-engine";

    /// A plain press of `code`, as crossterm reports one with no modifiers.
    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// A field over [`DIRECTORY`] holding `text` and no rule line.
    fn field(text: &str) -> ScopeField {
        ScopeField::new(DIRECTORY, text)
    }

    /// The field `key` leaves behind, or a panic naming what came out instead:
    /// most rules here are about what the prompt still looks like afterwards.
    fn after(key: KeyEvent, field: &ScopeField) -> ScopeField {
        match edit_for(key, field) {
            Edited::Open(next) => next,
            other => panic!("{key:?} should have left the prompt open, and gave {other:?}"),
        }
    }

    /// Every key the tree answers to, plus a character bound to nothing
    /// anywhere: the list the prompt has to swallow whole, so no keystroke
    /// reaches the app underneath while the field is up.
    const BINDINGS: [KeyCode; 17] = [
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
        KeyCode::Tab,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Char('x'),
    ];

    /// The ones of [`BINDINGS`] that are characters, i.e. the ones that have to
    /// come out as text rather than as nothing.
    fn typed(code: KeyCode) -> Option<char> {
        match code {
            KeyCode::Char(character) => Some(character),
            _ => None,
        }
    }

    #[test]
    fn a_fresh_prompt_is_up_over_the_directory_and_the_scope_it_carries() {
        // It opens on what is true: the scope the directory has now, so Enter
        // straight after the key that opened this writes back what was already
        // there rather than clearing it.
        let prompt = ScopePrompt::open(DIRECTORY, "data-plane");

        assert_eq!(prompt, ScopePrompt::Open(field("data-plane")));
        assert!(prompt.is_open());
        assert_eq!(prompt.field(), Some(&field("data-plane")));
        assert_eq!(prompt.field().map(ScopeField::directory), Some(DIRECTORY));
        assert_eq!(prompt.field().map(ScopeField::text), Some("data-plane"));
        assert_eq!(prompt.field().and_then(ScopeField::rule), None);
    }

    #[test]
    fn a_directory_carrying_no_scope_opens_on_an_empty_field() {
        let prompt = ScopePrompt::open(DIRECTORY, "");

        assert_eq!(prompt.field().map(ScopeField::text), Some(""));
        assert_eq!(prompt.field().and_then(ScopeField::rule), None);
    }

    #[test]
    fn a_closed_prompt_is_the_default_and_holds_no_field() {
        // `Default` is what the event loop starts a session on, and a closed
        // prompt has no text to draw or to hand to `edit_for`.
        assert_eq!(ScopePrompt::default(), ScopePrompt::Closed);
        assert!(!ScopePrompt::Closed.is_open());
        assert_eq!(ScopePrompt::Closed.field(), None);
    }

    #[test]
    fn printable_characters_append_in_the_order_they_are_typed() {
        let mut current = field("");
        for character in "data-plane_2".chars() {
            current = after(press(KeyCode::Char(character)), &current);
        }

        assert_eq!(current, field("data-plane_2"));
    }

    #[test]
    fn characters_append_exactly_as_typed_and_are_not_folded_or_refused() {
        // Case folding belongs to the caller that asks the engine, and judging
        // belongs to the engine: a field that silently dropped a capital or a
        // comma would leave somebody typing at a prompt that ignores them.
        let mut current = field("");
        for character in "Control Plane, 9".chars() {
            current = after(press(KeyCode::Char(character)), &current);
        }

        assert_eq!(current.text(), "Control Plane, 9");
    }

    #[test]
    fn backspace_takes_back_one_character_at_a_time() {
        assert_eq!(after(press(KeyCode::Backspace), &field("web")), field("we"));
        assert_eq!(after(press(KeyCode::Backspace), &field("w")), field(""));
    }

    #[test]
    fn backspace_takes_back_a_character_and_not_a_byte() {
        // A multi-byte character is not a scope and never will be, but it can
        // be typed, and a delete that took one byte of it would leave the
        // buffer holding half a character.
        assert_eq!(after(press(KeyCode::Backspace), &field("wéb")), field("wé"));
        assert_eq!(after(press(KeyCode::Backspace), &field("wé")), field("w"));
    }

    #[test]
    fn backspace_on_an_empty_field_changes_nothing_and_does_not_close() {
        // One press past the start is a typo, not an abandonment: Esc is the
        // key that leaves, and it is the only one.
        let empty = field("");

        assert_eq!(
            edit_for(press(KeyCode::Backspace), &empty),
            Edited::Open(empty.clone())
        );

        let refused = empty.refused("a scope cannot be empty");
        assert_eq!(
            edit_for(press(KeyCode::Backspace), &refused),
            Edited::Open(refused.clone()),
            "a key that changes nothing leaves the rule line where it was"
        );
    }

    #[test]
    fn enter_submits_whatever_is_there_including_nothing() {
        // An empty field is how a scope is cleared, so it is an answer rather
        // than an error, and it is not this module's business either way.
        for text in ["data-plane", "", "  ", "control-plane, data-plane"] {
            assert_eq!(
                edit_for(press(KeyCode::Enter), &field(text)),
                Edited::Submit,
                "Enter should submit {text:?}"
            );
        }
    }

    #[test]
    fn esc_closes_from_any_text() {
        for text in ["", "data-plane", "not a scope at all"] {
            assert_eq!(
                edit_for(press(KeyCode::Esc), &field(text)),
                Edited::Close,
                "Esc should close over {text:?}"
            );
        }
    }

    #[test]
    fn a_refusal_reopens_the_field_with_the_text_untouched() {
        let refused = field("data plane").refused("a scope holds only lowercase letters");

        assert_eq!(refused.text(), "data plane");
        assert_eq!(refused.directory(), DIRECTORY);
        assert_eq!(refused.rule(), Some("a scope holds only lowercase letters"));
    }

    #[test]
    fn an_edit_clears_the_rule_line_and_a_non_edit_leaves_it() {
        // The line names a rule the text broke, and the text has just changed.
        let refused = field("data plane").refused("a scope holds only lowercase letters");

        assert_eq!(after(press(KeyCode::Backspace), &refused).rule(), None);
        assert_eq!(after(press(KeyCode::Char('x')), &refused).rule(), None);
        assert_eq!(
            after(press(KeyCode::Tab), &refused).rule(),
            Some("a scope holds only lowercase letters"),
            "a key that types nothing has not made the complaint stale"
        );
    }

    #[test]
    fn every_tree_binding_is_swallowed_as_text_or_as_nothing() {
        // Asserted over the whole list because "nothing reaches the app" is the
        // promise: a stray `p` that pacted something behind the prompt, or a
        // `j` that moved a selection nobody can see, would be the same accident
        // by two routes.
        for code in BINDINGS {
            let before = field("web");
            let next = after(press(code), &before);

            let expected = typed(code).map_or_else(
                || before.clone(),
                |character| field(&format!("web{character}")),
            );

            assert_eq!(next, expected, "{code:?} should be text or nothing");
            assert_eq!(next.directory(), DIRECTORY);
        }
    }

    #[test]
    fn nothing_but_enter_submits_and_nothing_but_esc_closes() {
        // Said the other way round, because what must not happen is the
        // interesting half: of every key this module sees, one writes and one
        // abandons, and no third key does either.
        for code in BINDINGS.into_iter().chain([KeyCode::Backspace]) {
            let outcome = edit_for(press(code), &field("web"));

            assert_ne!(outcome, Edited::Submit, "{code:?} should not submit");
            assert_ne!(outcome, Edited::Close, "{code:?} should not close");
        }
    }

    #[test]
    fn ctrl_c_is_not_answered_here() {
        // It is a key event, not a signal, and the loop has to take it before
        // it consults this mode. Through here it is one of the keys that change
        // nothing — which is why the loop answering it first is load-bearing
        // rather than tidy, and why a chord is never text.
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let before = field("web");

        assert_eq!(edit_for(ctrl_c, &before), Edited::Open(before.clone()));
    }

    #[test]
    fn a_chord_or_a_control_character_is_not_text() {
        // Every modifier that makes a character a command rather than a letter,
        // plus a control character arriving as itself.
        let before = field("web");

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
                edit_for(key, &before),
                Edited::Open(before.clone()),
                "{modifiers:?} makes `u` a command, not a letter"
            );
        }

        assert_eq!(
            edit_for(press(KeyCode::Char('\u{7}')), &before),
            Edited::Open(before.clone()),
            "a control character is not text however it arrived"
        );
    }

    #[test]
    fn shift_still_types() {
        // Terminals disagree about whether the modifier rides along with an
        // upper-case letter, and either way it is somebody typing.
        let key = KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT);

        assert_eq!(after(key, &field("")), field("W"));
    }

    #[test]
    fn releases_and_repeats_type_nothing() {
        // The same rule as `action_for`: acting on a release would type the
        // release of the very key that opened the prompt, and `s` would arrive
        // in its own field.
        let before = field("web");

        for code in [
            KeyCode::Char('s'),
            KeyCode::Backspace,
            KeyCode::Enter,
            KeyCode::Esc,
        ] {
            for kind in [KeyEventKind::Release, KeyEventKind::Repeat] {
                let event = KeyEvent::new_with_kind_and_state(
                    code,
                    KeyModifiers::NONE,
                    kind,
                    KeyEventState::NONE,
                );

                assert_eq!(
                    edit_for(event, &before),
                    Edited::Open(before.clone()),
                    "{kind:?} of {code:?} should do nothing"
                );
            }
        }
    }

    #[test]
    fn no_key_ever_changes_the_directory_being_scoped() {
        // The heading is what the window is about, and it is decided when the
        // prompt opens: nothing typed into the field can move it.
        let before = field("web").refused("a scope cannot be empty");

        for code in BINDINGS.into_iter().chain([
            KeyCode::Backspace,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::Delete,
        ]) {
            assert_eq!(after(press(code), &before).directory(), DIRECTORY);
        }
    }

    #[test]
    fn the_arrows_and_the_editing_keys_this_field_does_not_have_do_nothing() {
        // No selection, no history, no left-arrow editing: the cursor is at the
        // end because nothing here can put it anywhere else.
        let before = field("web");

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
                edit_for(press(code), &before),
                Edited::Open(before.clone()),
                "{code:?} is not an editing key this field has"
            );
        }
    }
}
