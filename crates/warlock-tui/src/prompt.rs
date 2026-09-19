//! The two prompts the `s` key puts up: the scope prompt — the directory being
//! scoped and the text typed into its one field — and the record prompt behind
//! it, which asks for the three values a brand-new scope name routes work by.
//! Both carry a line under the field saying why the last submit was refused.
//!
//! [`ScopePrompt`] and [`RecordPrompt`] are values of their own and *not* fields
//! on [`App`](crate::App), because Esc has to leave the app exactly as it was and
//! an app that never heard of the prompt is a cheaper guarantee of that than
//! putting every field back. The cursor is a byte offset into that text and
//! every key here keeps it on a character boundary, because the text is sliced
//! at it — by the edits below and by the frame, which draws the caret on the
//! character it names. Nothing here judges the text either — Enter comes back as
//! [`Edited::Submit`] or [`Recorded::Submit`] whatever has been typed, empty
//! included, because that is how a scope is cleared and because blankness in a
//! record is the caller's refusal to make, alongside the engine's.
//!
//! There is one cursor implementation, [`edit_for`], and [`record_edit_for`]
//! hands it the field being typed in rather than keeping a second copy: a
//! Backspace that took a byte in one window and a character in the other would
//! be the same bug twice.
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

/// Which of the record's three fields the keys are going into.
///
/// The order is the order they are asked in and the order they are drawn in,
/// and [`RecordAsk::ORDER`] is the one place it is written down.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum RecordAsk {
    #[default]
    Team,
    ReviewState,
    Label,
}

impl RecordAsk {
    pub const ORDER: [Self; 3] = [Self::Team, Self::ReviewState, Self::Label];

    /// What the field is called on screen.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Team => "team",
            Self::ReviewState => "review state",
            Self::Label => "label",
        }
    }

    // Both wrap, because these two keys are the whole of the movement: a Tab
    // that stopped dead on the last field would leave a reader who overshot
    // with nowhere to go but Shift-Tab, which not every terminal sends.
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

/// The scope being recorded and the three values it will route work by.
///
/// `name` is the folded scope name already accepted by the first prompt, held
/// so the record that is written carries the same string that went onto the
/// pact; nothing typed here can move it, exactly as nothing typed into a
/// [`ScopeField`] can move the directory it is about.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct RecordFields {
    module: String,
    name: String,
    // Three [`ScopeField`]s rather than three texts and three cursors, so the
    // editing keys are `edit_for`'s and there is no second implementation of
    // them. Each is built over `module`, because that is the directory a
    // `ScopeField` is about and `edit_for` hands it back untouched; what is
    // drawn over each field is [`RecordAsk::heading`], so there is no second
    // spelling of the three names to keep in step.
    team: ScopeField,
    review_state: ScopeField,
    label: ScopeField,
    asking: RecordAsk,
}

impl RecordFields {
    // Empty, because there is nothing true to open on: the manifest holds no
    // record for this name, which is the only reason this question is being
    // asked at all.
    #[must_use]
    pub fn new(module: impl Into<String>, name: impl Into<String>) -> Self {
        let module = module.into();
        Self {
            team: ScopeField::new(module.as_str(), ""),
            review_state: ScopeField::new(module.as_str(), ""),
            label: ScopeField::new(module.as_str(), ""),
            module,
            name: name.into(),
            asking: RecordAsk::default(),
        }
    }

    #[must_use]
    pub fn module(&self) -> &str {
        &self.module
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn asking(&self) -> RecordAsk {
        self.asking
    }

    #[must_use]
    pub const fn field(&self, ask: RecordAsk) -> &ScopeField {
        match ask {
            RecordAsk::Team => &self.team,
            RecordAsk::ReviewState => &self.review_state,
            RecordAsk::Label => &self.label,
        }
    }

    /// The field the keys are going into, which is the one the caret is drawn
    /// in.
    #[must_use]
    pub const fn current(&self) -> &ScopeField {
        self.field(self.asking)
    }

    // The refusal moves the question to the field it is about as well as
    // putting the line under it: a complaint about the label while the caret
    // sits in the team field is one a reader fixes by typing in the wrong
    // place. Text and cursor are `ScopeField::refused`'s to keep, and the other
    // two fields are carried across whatever was typed into them, so a blank
    // third field costs nobody the first two.
    #[must_use]
    pub fn refused(self, ask: RecordAsk, rule: impl Into<String>) -> Self {
        let refused = self.field(ask).clone().refused(rule);
        Self {
            asking: ask,
            ..self.with(ask, refused)
        }
    }

    fn with(self, ask: RecordAsk, field: ScopeField) -> Self {
        match ask {
            RecordAsk::Team => Self {
                team: field,
                ..self
            },
            RecordAsk::ReviewState => Self {
                review_state: field,
                ..self
            },
            RecordAsk::Label => Self {
                label: field,
                ..self
            },
        }
    }

    fn asked(self, asking: RecordAsk) -> Self {
        Self { asking, ..self }
    }
}

// The fields live inside `Open` for the reason [`ScopePrompt`]'s do: half a
// record typed into a closed prompt is not a state that can be written down.
#[allow(
    clippy::large_enum_variant,
    reason = "three text fields, moved once per keystroke on the event loop's own thread; a `Box` would buy a heap allocation and an indirection between the prompt and the text it is about"
)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum RecordPrompt {
    #[default]
    Closed,
    Open(RecordFields),
}

impl RecordPrompt {
    #[must_use]
    pub fn open(module: impl Into<String>, name: impl Into<String>) -> Self {
        Self::Open(RecordFields::new(module, name))
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    // The one way into `record_edit_for`, as `ScopePrompt::field` is into
    // `edit_for`.
    #[must_use]
    pub const fn fields(&self) -> Option<&RecordFields> {
        match self {
            Self::Closed => None,
            Self::Open(fields) => Some(fields),
        }
    }
}

/// What a keystroke comes to while the record prompt is open.
///
/// The same three answers as [`Edited`] and for the same reasons — a key that
/// means nothing leaves the prompt where it was, and a submit carries no text
/// of its own so there is one copy of what was typed.
#[allow(
    clippy::large_enum_variant,
    reason = "the same three fields as `RecordPrompt`, and boxed here for the size of `Close` would be a heap allocation per keystroke"
)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Recorded {
    Open(RecordFields),
    Close,
    Submit,
}

/// What `key` does to a record prompt open over `fields`.
///
/// Tab and Down go to the next field, Shift-Tab and Up to the previous one, and
/// both wrap; everything else is [`edit_for`]'s answer for the field being typed
/// in, Enter and Esc included. So there is no new global binding here and none
/// is wanted: while this is up the loop consults this module *instead of* the
/// app, so Tab is not the tree's Tab any more than `j` is the tree's `j`.
///
/// Moving is not an edit, so it leaves the refusal line where it is — the line
/// names something wrong with text that walking away from has not changed. Only
/// Enter submits and only Esc closes, whichever of the three fields is being
/// typed in: a record is written or abandoned whole, because a scope routing
/// work by two of the three values is the state this question exists to prevent.
#[must_use]
pub fn record_edit_for(key: KeyEvent, fields: &RecordFields) -> Recorded {
    if key.kind != KeyEventKind::Press {
        return Recorded::Open(fields.clone());
    }

    let asking = fields.asking;
    match key.code {
        KeyCode::Tab | KeyCode::Down => Recorded::Open(fields.clone().asked(asking.next())),
        KeyCode::BackTab | KeyCode::Up => Recorded::Open(fields.clone().asked(asking.previous())),
        _ => match edit_for(key, fields.current()) {
            Edited::Open(field) => Recorded::Open(fields.clone().with(asking, field)),
            Edited::Close => Recorded::Close,
            Edited::Submit => Recorded::Submit,
        },
    }
}

#[cfg(test)]
#[path = "tests/prompt.rs"]
mod tests;
