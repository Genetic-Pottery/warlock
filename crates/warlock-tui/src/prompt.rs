//! The scope prompt: which directory is being scoped, what has been typed into
//! the field, and the one line under it saying why the last submit was refused.
//!
//! [`ScopePrompt`] is a value of its own and *not* a field on
//! [`App`](crate::App), for the reason [`QuitConfirm`](crate::QuitConfirm) is
//! not one: Esc has to leave the app exactly as it was, and an app that never
//! heard of the prompt is a cheaper guarantee of that than putting every field
//! back.
//!
//! Printable characters append, Backspace takes one back, Enter submits and Esc
//! closes; that is the entire editor. Because nothing moves the cursor, it is
//! always at the end of the text and so is not a field anybody has to keep true
//! — whoever draws the field puts the caret after the last character and is
//! right by construction. Adding arrow-key editing would break that and would
//! claim keys that are otherwise characters being typed into a string of at most
//! twenty-four.
//!
//! Nothing here judges the text. Enter comes back as [`Edited::Submit`] whatever
//! has been typed, empty included — that is how a scope is *cleared* — and the
//! caller asks the engine's
//! [`validate_scope`](warlock_engine::validate_scope), which is the workspace's
//! only judge of what a scope may be.
//!
//! Ctrl-C is deliberately not answered here. It is a key event and not a signal
//! — raw mode is exactly the mode in which the terminal stops turning it into
//! `SIGINT` — so the loop has to take it before it consults this module, both
//! with the prompt open and closed. Through here it is one of the keys that
//! change nothing, which is why a chord is never treated as text: the one
//! keystroke every reader trusts to get them out would otherwise put a `c` in
//! the field.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

// Shift is deliberately absent: an upper-case letter arrives with it on some
// terminals and without it on others, and either way it is text. Everything here
// is a chord — Ctrl-C first among them — and a chord is not text.
const CHORD: KeyModifiers = KeyModifiers::CONTROL
    .union(KeyModifiers::ALT)
    .union(KeyModifiers::SUPER)
    .union(KeyModifiers::HYPER)
    .union(KeyModifiers::META);

// One value rather than three parallel variables in the event loop: the three
// are only ever meaningful together — there is no directory being scoped without
// a field to type in, and no rule line without a submit that broke one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ScopeField {
    // A manifest path in every real use, held as a plain `String` because this
    // module only ever prints it.
    directory: String,
    text: String,
    // One line, put here by the caller after the engine refused, and worded by
    // the engine.
    rule: Option<String>,
}

impl ScopeField {
    // `text` is the scope the directory carries right now, or empty when it
    // carries none, so the prompt opens on what is true rather than on a blank
    // that would silently clear it. No rule line: a prompt that opens already
    // complaining is complaining about somebody else's typing.
    #[must_use]
    pub fn new(directory: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            directory: directory.into(),
            text: text.into(),
            rule: None,
        }
    }

    // The text is untouched, which is the point: a refusal leaves whoever typed
    // it looking at what they typed, one character away from fixing it, rather
    // than at an empty field and a complaint about a string no longer on screen.
    #[must_use]
    pub fn refused(self, rule: impl Into<String>) -> Self {
        Self {
            rule: Some(rule.into()),
            ..self
        }
    }

    #[must_use]
    pub fn directory(&self) -> &str {
        &self.directory
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn rule(&self) -> Option<&str> {
        self.rule.as_deref()
    }
}

// The field lives inside `Open` rather than beside a `bool`, so "closed, with
// half a scope typed" is not a state that can be written down: there is one
// place the text can be, and it exists exactly as long as the question does.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum ScopePrompt {
    #[default]
    Closed,
    Open(ScopeField),
}

impl ScopePrompt {
    // A named constructor rather than `Open(ScopeField::new(..))` at the call
    // site: what a fresh prompt starts on is decided here rather than wherever
    // the `s` key happens to be handled.
    #[must_use]
    pub fn open(directory: impl Into<String>, text: impl Into<String>) -> Self {
        Self::Open(ScopeField::new(directory, text))
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    // The one way into `edit_for`: an `Option` here is what keeps the key
    // handler from having to invent a field for a prompt that is not up.
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
/// There is deliberately no variant for "the key meant nothing": a key that
/// means nothing here leaves the prompt exactly where it was, which is
/// [`Edited::Open`] with the same field in it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Edited {
    Open(ScopeField),
    Close,
    /// Offered up. It carries no text of its own, so there is one copy of what
    /// was typed and no way for a submit to disagree with the field it came
    /// from: the caller takes [`ScopeField::text`], asks the engine whether it
    /// is a scope, and either writes the manifest or reopens the prompt through
    /// [`ScopeField::refused`].
    Submit,
}

/// What `key` does to a prompt open over `field`.
///
/// Backspace on an empty field does nothing rather than closing the prompt: one
/// press past the start is a typo and not an abandonment. An edit clears the
/// rule line, because that line names a rule the text broke and the text has
/// just changed; a key that changes nothing leaves it up, complaint and all.
///
/// Every other key leaves the field byte for byte as it was, the tree's own
/// bindings included — while this is up, `j`, `k`, `p`, `r` and the rest are
/// letters somebody is typing, because the loop consults this *instead of* the
/// app rather than as well as it. Only presses count, exactly as `action_for`
/// has it: acting on a release would type the release of the very key that
/// opened the prompt.
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

    // Every field below is opened over this, so a test that meant to change the
    // text cannot pass by changing the directory instead.
    const DIRECTORY: &str = "crates/warlock-engine";

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn field(text: &str) -> ScopeField {
        ScopeField::new(DIRECTORY, text)
    }

    fn after(key: KeyEvent, field: &ScopeField) -> ScopeField {
        match edit_for(key, field) {
            Edited::Open(next) => next,
            other => panic!("{key:?} should have left the prompt open, and gave {other:?}"),
        }
    }

    // Every key the tree answers to, plus a character bound to nothing anywhere:
    // the list the prompt has to swallow whole, so no keystroke reaches the app
    // underneath while the field is up.
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
