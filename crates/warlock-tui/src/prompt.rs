//! The two prompts the `s` key puts up: the scope itself — the directory being
//! scoped, the text typed into the field, and the one line under it saying why
//! the last submit was refused — and, when that name is new, the record it
//! routes to, which is three of those fields and a cursor saying which one is
//! being typed in.
//!
//! [`ScopePrompt`] and [`RecordPrompt`] are values of their own and *not* fields
//! on [`App`](crate::App), because Esc has to leave the app exactly as it was and
//! an app that never heard of the prompt is a cheaper guarantee of that than
//! putting every field back. The cursor is a byte offset into that text and
//! every key here keeps it on a character boundary, because the text is sliced
//! at it — by the edits below and by the frame, which draws the caret on the
//! character it names. Nothing here judges the text either — Enter comes back as
//! [`Edited::Submit`] whatever has been typed, empty included, because that is
//! how a scope is cleared.
//!
//! Ctrl-C is deliberately not answered here. Raw mode is exactly the mode in
//! which the terminal stops turning it into `SIGINT`, so the loop takes it
//! before consulting this module; treated as text it would put a `c` in the
//! field.

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
    // Where the next character lands, as a byte offset into `text` and never
    // anywhere but a character boundary — see the module docs.
    cursor: usize,
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
        let text = text.into();
        Self {
            directory: directory.into(),
            // At the end of what is already there, which is where a reader who
            // opened the prompt to add to a scope wants it and one press of
            // Home from where the other one does.
            cursor: text.len(),
            text,
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

    /// A byte offset into [`ScopeField::text`], on a character boundary, so the
    /// frame can slice the text at it to put the caret down.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
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
/// press past the start is a typo and not an abandonment, and Left at the start
/// and Right at the end are the same kind of nothing. An edit clears the rule
/// line, because that line names a rule the text broke and the text has just
/// changed; a key that changes nothing — a move included — leaves it up,
/// complaint and all.
///
/// The cursor answers the keys the composer's does and no others: Left, Right,
/// Home, End, Backspace behind it and Delete in front of it. There is no Up or
/// Down, because this field is one line however long the text on it gets.
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
    let edited = |text: String, cursor: usize| {
        Edited::Open(ScopeField {
            directory: field.directory.clone(),
            text,
            cursor,
            rule: None,
        })
    };
    // A move is not an edit: the rule line names something wrong with the text,
    // and walking the cursor over it leaves it just as wrong. Only the arms that
    // change the text go through `edited`, which is what drops that line.
    let moved = |cursor: usize| {
        Edited::Open(ScopeField {
            cursor,
            ..field.clone()
        })
    };
    let before = || field.text[..field.cursor].chars().next_back();
    let after = || field.text[field.cursor..].chars().next();
    let removed = |at: usize| {
        let mut text = field.text.clone();
        text.remove(at);
        text
    };

    match key.code {
        KeyCode::Enter => Edited::Submit,
        KeyCode::Esc => Edited::Close,
        KeyCode::Left => match before() {
            Some(character) => moved(field.cursor - character.len_utf8()),
            None => unchanged(),
        },
        KeyCode::Right => match after() {
            Some(character) => moved(field.cursor + character.len_utf8()),
            None => unchanged(),
        },
        KeyCode::Home => moved(0),
        KeyCode::End => moved(field.text.len()),
        KeyCode::Backspace => match before() {
            // Whole characters on both sides of the cursor, never bytes: half a
            // character left in the buffer would not be a `String` at all, and
            // an offset landing inside one would panic the next slice.
            Some(character) => {
                let start = field.cursor - character.len_utf8();
                edited(removed(start), start)
            }
            None => unchanged(),
        },
        KeyCode::Delete => match after() {
            Some(_) => edited(removed(field.cursor), field.cursor),
            None => unchanged(),
        },
        // A chord is a command somebody sent, not a character somebody typed,
        // and control characters are not text however they arrived — Ctrl-C
        // among them, which the loop above has already had its chance at.
        KeyCode::Char(character) if !key.modifiers.intersects(CHORD) && !character.is_control() => {
            let mut text = field.text.clone();
            text.insert(field.cursor, character);
            edited(text, field.cursor + character.len_utf8())
        }
        _ => unchanged(),
    }
}

/// Which of the record's three values is being typed in.
///
/// The order is the order [`ScopeRecord::new`](warlock_engine::ScopeRecord::new)
/// takes them and the order the frame draws them, so a reader tabbing down the
/// window is filling the record top to bottom.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RecordField {
    #[default]
    Team,
    ReviewState,
    Label,
}

impl RecordField {
    pub const ALL: [Self; 3] = [Self::Team, Self::ReviewState, Self::Label];

    /// What the frame prints beside the field, and what a refusal about it
    /// names.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Team => "team",
            Self::ReviewState => "review state",
            Self::Label => "label",
        }
    }

    // Wrapping rather than stopping at the ends: there is no submit button to
    // fall off the bottom onto, so the only thing past the last field is the
    // first one.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Team => Self::ReviewState,
            Self::ReviewState => Self::Label,
            Self::Label => Self::Team,
        }
    }

    #[must_use]
    pub const fn previous(self) -> Self {
        match self {
            Self::Team => Self::Label,
            Self::ReviewState => Self::Team,
            Self::Label => Self::ReviewState,
        }
    }
}

// Three [`ScopeField`]s and not three bare strings: the cursor, the character
// boundaries and the rule line are the same problem three times over, and
// reusing the field is what keeps one set of answers to it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct RecordForm {
    path: String,
    // Already folded by the caller, and stored so the window can show which name
    // is being recorded: the record written from this form has to be the one the
    // pact names, and a second read of the first prompt could disagree.
    scope: String,
    team: ScopeField,
    review_state: ScopeField,
    label: ScopeField,
    focus: RecordField,
}

impl RecordForm {
    // Empty fields, not defaults: nothing here is guessable from the scope name,
    // and a pre-filled team would be somebody else's team saved by an Enter.
    #[must_use]
    pub fn new(path: impl Into<String>, scope: impl Into<String>) -> Self {
        let path = path.into();
        let empty = ScopeField::new(path.as_str(), "");
        Self {
            scope: scope.into(),
            team: empty.clone(),
            review_state: empty.clone(),
            label: empty,
            focus: RecordField::Team,
            path,
        }
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    #[must_use]
    pub const fn focus(&self) -> RecordField {
        self.focus
    }

    #[must_use]
    pub const fn field(&self, which: RecordField) -> &ScopeField {
        match which {
            RecordField::Team => &self.team,
            RecordField::ReviewState => &self.review_state,
            RecordField::Label => &self.label,
        }
    }

    #[must_use]
    pub const fn focused(&self) -> &ScopeField {
        self.field(self.focus)
    }

    // The cursor moves to the field complained about, so the first keystroke
    // after a refusal fixes the thing that was refused rather than editing
    // whichever field the reader happened to leave it in.
    #[must_use]
    pub fn refused(mut self, which: RecordField, rule: impl Into<String>) -> Self {
        let field = std::mem::take(self.slot(which)).refused(rule);
        *self.slot(which) = field;
        self.focus = which;
        self
    }

    const fn slot(&mut self, which: RecordField) -> &mut ScopeField {
        match which {
            RecordField::Team => &mut self.team,
            RecordField::ReviewState => &mut self.review_state,
            RecordField::Label => &mut self.label,
        }
    }
}

// `RecordForm` is some 300 bytes against `Closed`'s nothing, which clippy flags
// and which would be worth boxing in a `Result` or a collection. This is one
// field of the event loop, alive between two frames of one keystroke, so a `Box`
// would buy an allocation and a pointer chase — and cost `form` its `const`,
// because a `Box` cannot be dereferenced in a const fn.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum RecordPrompt {
    #[default]
    Closed,
    Open(RecordForm),
}

impl RecordPrompt {
    #[must_use]
    pub fn open(path: impl Into<String>, scope: impl Into<String>) -> Self {
        Self::Open(RecordForm::new(path, scope))
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    #[must_use]
    pub const fn form(&self) -> Option<&RecordForm> {
        match self {
            Self::Closed => None,
            Self::Open(form) => Some(form),
        }
    }
}

/// What a keystroke comes to while the record prompt is open.
///
/// [`RecordEdited::Submit`] carries no text for the reason [`Edited::Submit`]
/// does not: the caller reads the three fields off the form it already holds, so
/// there is no second copy of them to disagree with it. Whether any of them is
/// blank is the caller's refusal to make, through [`RecordForm::refused`].
// Unboxed for the reason given over `RecordPrompt`, and this one is shorter
// lived still: it is read by the caller that asked for it and dropped.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RecordEdited {
    Open(RecordForm),
    Close,
    Submit,
}

/// What `key` does to a prompt open over `form`.
///
/// Every key that is text, a cursor move or a delete is [`edit_for`]'s answer
/// about the focused field, so the three fields here type exactly as the scope
/// field does and cannot drift from it. Tab and Down go to the next field, and
/// `BackTab` and Up to the previous one: none of the four is text in `edit_for`,
/// which is what lets the record be filled in without a keystroke that could
/// have been a letter.
///
/// Changing field is a move and not an edit, so the rule line under the field
/// being left stays up — it names something wrong with text that has not
/// changed.
#[must_use]
pub fn record_edit_for(key: KeyEvent, form: &RecordForm) -> RecordEdited {
    if key.kind != KeyEventKind::Press {
        return RecordEdited::Open(form.clone());
    }

    let refocused = |focus: RecordField| {
        RecordEdited::Open(RecordForm {
            focus,
            ..form.clone()
        })
    };

    match key.code {
        KeyCode::Tab | KeyCode::Down => refocused(form.focus.next()),
        KeyCode::BackTab | KeyCode::Up => refocused(form.focus.previous()),
        _ => match edit_for(key, form.focused()) {
            Edited::Close => RecordEdited::Close,
            Edited::Submit => RecordEdited::Submit,
            Edited::Open(field) => {
                let mut next = form.clone();
                *next.slot(form.focus) = field;
                RecordEdited::Open(next)
            }
        },
    }
}

#[cfg(test)]
#[path = "tests/prompt.rs"]
mod tests;
