//! The accident that costs a session is the reflex second Esc — the first
//! cancels a pact, the run is still tidying up, and the next press lands in a
//! shell nobody meant to be in. Which is why Esc answers No here: the key that
//! means "not this" cannot also be the key that leaves. For the same reason the
//! mode is a value of its own and *not* a field on `App`, so an app compared
//! before opening and after closing is equal because nothing about it was
//! touched rather than because every field was carefully put back.
//!
//! Yes is drawn on the left and No on the right, which makes Left and Right
//! positional rather than a toggle — a toggle would move the highlight *away*
//! from the side the arrow points at once it is already there. Ctrl-C is not
//! answered here at all: raw mode is exactly the mode in which the terminal
//! stops turning it into `SIGINT`, so the loop takes it before consulting this
//! mode, and coming through here the dialog would swallow it.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};
use warlock_engine::Destination;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Answer {
    Yes,
    // The default, so the keystroke that opens the dialog and an Enter straight
    // after it come to nothing at all: the dangerous answer is never the one
    // already under the reader's finger.
    #[default]
    No,
}

// The lit answer lives inside `Open` rather than beside a `bool`, so "closed,
// with Yes highlighted" is not a state that can be written down: the highlight
// exists exactly as long as the question does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum QuitConfirm {
    #[default]
    Closed,
    Open(Answer),
}

impl QuitConfirm {
    // A named constructor rather than `Open(Answer::No)` at the call site, so
    // which answer a fresh dialog starts on is decided here rather than wherever
    // Esc happens to be handled.
    #[must_use]
    pub const fn open() -> Self {
        Self::Open(Answer::No)
    }

    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open(_))
    }

    // The one way into `answer_for`: the `Option` is what keeps the key handler
    // from having to invent an answer for a dialog that is not up.
    #[must_use]
    pub const fn highlighted(self) -> Option<Answer> {
        match self {
            Self::Closed => None,
            Self::Open(answer) => Some(answer),
        }
    }
}

// Three variants is the whole of what can happen to a two-answer question, and
// there is deliberately no variant for "the key meant nothing": a key that means
// nothing here leaves the question where it was, which is `Open` with the same
// answer in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Answered {
    Open(Answer),
    Close,
    Leave,
}

// Every key not matched below leaves the question byte-for-byte as it was,
// including the tree's own bindings: while this is up the loop consults this
// instead of the app rather than as well as it, so `j`, `k`, `p`, Tab and the
// rest reach nothing underneath.
//
// Only presses count. Crossterm reports releases and auto-repeats on some
// platforms and not others, and a release acted on here would answer the
// question with the release of the very key that opened it — Esc pressed once
// would open the dialog and immediately close it again, which is a gate that is
// not there.
#[must_use]
pub fn answer_for(key: KeyEvent, highlighted: Answer) -> Answered {
    if key.kind != KeyEventKind::Press {
        return Answered::Open(highlighted);
    }

    match key.code {
        KeyCode::Left => Answered::Open(Answer::Yes),
        KeyCode::Right => Answered::Open(Answer::No),
        KeyCode::Enter => match highlighted {
            Answer::Yes => Answered::Leave,
            Answer::No => Answered::Close,
        },
        // By character rather than by `SHIFT`, like the tree's `g`/`G` pair:
        // terminals disagree about whether the modifier rides along with an
        // upper-case letter, and a reader with caps lock on is still answering.
        KeyCode::Char('y' | 'Y') => Answered::Leave,
        KeyCode::Char('n' | 'N') | KeyCode::Esc => Answered::Close,
        _ => Answered::Open(highlighted),
    }
}

// The project a `/push` is about to make and the board it goes to. The board is
// a [`Destination`], which holds the key *by name* and has nowhere for its bytes
// to sit: a dialog that cannot hold a key cannot draw one, print one or grow one
// in a `Debug` rendering.
//
// The lit answer rides along inside it for `QuitConfirm`'s reason — it exists
// exactly as long as the question does — which is why this is only ever built
// through [`PushConfirm::open`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Filing {
    project: String,
    destination: Destination,
    answer: Answer,
}

impl Filing {
    #[must_use]
    pub fn project(&self) -> &str {
        &self.project
    }

    #[must_use]
    pub const fn destination(&self) -> &Destination {
        &self.destination
    }

    #[must_use]
    pub const fn answer(&self) -> Answer {
        self.answer
    }

    // The one way the highlight moves, so answering re-lights the same question
    // rather than building a second one from strings it would have to be handed
    // again.
    #[must_use]
    pub fn with_answer(&self, answer: Answer) -> Self {
        Self {
            answer,
            ..self.clone()
        }
    }
}

/// The question a `/push` asks before anything leaves the machine, drawn over
/// the frame the way the quit dialog is and answered by the very same rules —
/// [`push_answer_for`] is [`answer_for`] with the answers renamed.
///
/// A separate value from [`QuitConfirm`] because the two are answered about
/// different things and say so in their types: a confirmed question here sends
/// a brief and leaves the session exactly where it was.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum PushConfirm {
    #[default]
    Closed,
    Open(Filing),
}

impl PushConfirm {
    /// No is lit on open, for the reason [`Answer::No`] is the default: the
    /// keystroke that opened the dialog and an Enter straight after it come to
    /// nothing at all.
    #[must_use]
    pub fn open(project: impl Into<String>, destination: Destination) -> Self {
        Self::Open(Filing {
            project: project.into(),
            destination,
            answer: Answer::No,
        })
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    /// The one way into [`push_answer_for`] and into the drawing, for the
    /// reason [`QuitConfirm::highlighted`] is: the caller cannot invent a
    /// question that is not up.
    #[must_use]
    pub const fn filing(&self) -> Option<&Filing> {
        match self {
            Self::Closed => None,
            Self::Open(filing) => Some(filing),
        }
    }

    /// The same question with the other answer lit, and a closed dialog left
    /// closed: an arrow key pressed at nothing lights nothing.
    #[must_use]
    pub fn lit(&self, answer: Answer) -> Self {
        match self {
            Self::Closed => Self::Closed,
            Self::Open(filing) => Self::Open(filing.with_answer(answer)),
        }
    }
}

/// [`Answered`] in this dialog's vocabulary: a confirmed question here means
/// send, and warlock goes on running either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PushAnswered {
    Open(Answer),
    Cancel,
    Send,
}

/// The quit dialog's rules, renamed rather than restated: Esc and `n` cancel,
/// Left then Enter sends, an immediate Enter cancels, a release changes
/// nothing, and every other key leaves the question exactly as it was. Written
/// over [`answer_for`] so the two cannot drift — a key that moves one moves the
/// other.
#[must_use]
pub fn push_answer_for(key: KeyEvent, highlighted: Answer) -> PushAnswered {
    match answer_for(key, highlighted) {
        Answered::Open(answer) => PushAnswered::Open(answer),
        Answered::Close => PushAnswered::Cancel,
        Answered::Leave => PushAnswered::Send,
    }
}

// The project a `/draft` is about to cut, as the five facts the reader is being
// asked about. Every one of them was read back off the board a moment ago and
// none can be read again without a second request, which is why they are parked
// here rather than looked up when the answer comes in.
//
// The key is here *by name*, for [`Filing`]'s reason and with the same
// consequence: there is nowhere in this value for its bytes to sit, so a dialog
// that cannot hold a key cannot draw one, print one or grow one in a `Debug`
// rendering. There is no scope beside the name either — a cut resolves the
// board the machine's own way, with no field in front of it to name a second
// one, so a Yes asks that same question again from nothing this value carries.
//
// The status is the board's own spelling and not `Planned`: the gate folds case
// and trims, and what a reader is shown is what somebody sent to look would
// find written on the project.
//
// The lit answer rides along inside it for [`QuitConfirm`]'s reason — it exists
// exactly as long as the question does — which is why this is only ever built
// through [`CutConfirm::open`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cutting {
    project: String,
    status: String,
    slices: usize,
    team: String,
    key: String,
    answer: Answer,
}

impl Cutting {
    #[must_use]
    pub fn project(&self) -> &str {
        &self.project
    }

    /// The status as the board spelled it: see the type's own note.
    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    /// How many slices the project's scope has, which is how much work the
    /// answer is about.
    #[must_use]
    pub const fn slices(&self) -> usize {
        self.slices
    }

    #[must_use]
    pub fn team(&self) -> &str {
        &self.team
    }

    /// The *name* the key is held under, never a key value: see the type's own
    /// note.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub const fn answer(&self) -> Answer {
        self.answer
    }

    // The one way the highlight moves, for [`Filing::with_answer`]'s reason:
    // answering re-lights the same question rather than building a second one
    // from facts that would have to be fetched again.
    #[must_use]
    pub fn with_answer(&self, answer: Answer) -> Self {
        Self {
            answer,
            ..self.clone()
        }
    }
}

/// The question a `/draft` asks between the fetch and the run, drawn over the
/// frame the way the other two are and answered by the very same rules —
/// [`cut_answer_for`] is [`answer_for`] with the answers renamed.
///
/// A separate value from [`PushConfirm`] because the two are answered about
/// different things and say so in their types: a confirmed question there sends
/// one brief, and a confirmed question here starts a run over every uncut slice
/// of a project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum CutConfirm {
    #[default]
    Closed,
    Open(Cutting),
}

impl CutConfirm {
    /// No is lit on open, for the reason [`Answer::No`] is the default: the
    /// round that puts this up and an Enter straight after it come to nothing
    /// at all.
    #[must_use]
    pub fn open(
        project: impl Into<String>,
        status: impl Into<String>,
        slices: usize,
        team: impl Into<String>,
        key: impl Into<String>,
    ) -> Self {
        Self::Open(Cutting {
            project: project.into(),
            status: status.into(),
            slices,
            team: team.into(),
            key: key.into(),
            answer: Answer::No,
        })
    }

    #[must_use]
    pub const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    /// The one way into [`cut_answer_for`] and into the drawing, for the
    /// reason [`QuitConfirm::highlighted`] is: the caller cannot invent a
    /// question that is not up.
    #[must_use]
    pub const fn cutting(&self) -> Option<&Cutting> {
        match self {
            Self::Closed => None,
            Self::Open(cutting) => Some(cutting),
        }
    }

    /// The same question with the other answer lit, and a closed dialog left
    /// closed: an arrow key pressed at nothing lights nothing.
    #[must_use]
    pub fn lit(&self, answer: Answer) -> Self {
        match self {
            Self::Closed => Self::Closed,
            Self::Open(cutting) => Self::Open(cutting.with_answer(answer)),
        }
    }
}

/// [`Answered`] in this dialog's vocabulary: a confirmed question here starts
/// the run, and warlock goes on running either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CutAnswered {
    Open(Answer),
    Cancel,
    Cut,
}

/// The quit dialog's rules, renamed rather than restated: Esc and `n` cancel,
/// Left then Enter cuts, an immediate Enter cancels, a release changes nothing,
/// and every other key leaves the question exactly as it was. Written over
/// [`answer_for`] so the three cannot drift — a key that moves one moves them
/// all.
#[must_use]
pub fn cut_answer_for(key: KeyEvent, highlighted: Answer) -> CutAnswered {
    match answer_for(key, highlighted) {
        Answered::Open(answer) => CutAnswered::Open(answer),
        Answered::Close => CutAnswered::Cancel,
        Answered::Leave => CutAnswered::Cut,
    }
}

/// What one slice's drafts are answered with: file them, leave the slice alone,
/// or say what is wrong with them.
///
/// Three answers and so a value of its own rather than [`Answer`] renamed a
/// fourth time: the windows above ask one thing and take a yes or a no, and this
/// asks which of three things to do. [`answer_for`] cannot be written over for
/// the same reason — there is no third answer for it to hand back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Choice {
    /// File this slice's drafts as issues.
    Create,
    /// The default, for [`Answer::No`]'s reason: the round that puts the window
    /// up and an Enter straight after it file nothing at all. It is also the
    /// answer in the middle, so the one that spends is a deliberate press
    /// either way.
    #[default]
    Skip,
    /// Redraft this slice with something typed into the composer.
    Feedback,
}

impl Choice {
    // One step along the row rather than a toggle, and clamped at both ends: a
    // highlight that wrapped would put the answer that files issues under the
    // finger of somebody pressing Right twice.
    const fn leftwards(self) -> Self {
        match self {
            Self::Create | Self::Skip => Self::Create,
            Self::Feedback => Self::Skip,
        }
    }

    // `offered` is whether this slice still has its one redraft. A third answer
    // that is not drawn is not one a key can land on.
    const fn rightwards(self, offered: bool) -> Self {
        match self {
            Self::Create => Self::Skip,
            Self::Skip | Self::Feedback if offered => Self::Feedback,
            answer => answer,
        }
    }
}

/// One slice's drafts, waiting to be answered about: what was drafted, for which
/// slice, and which of the answers is lit.
///
/// The titles are what the model settled on and the whole of what the window
/// shows of a draft: a body is paragraphs and a window that drew them would be a
/// document card with three answers under it. They are on the thread as well,
/// said as they arrived, so a reader who wants more than a title scrolls.
///
/// There is no `Closed` variant beside this the way there is for the three
/// dialogs above, and deliberately: those are fields of the session, which is
/// always there, and this is a state of one slice of one run — it exists exactly
/// as long as that slice is waiting, and a caller with no run has nothing to
/// hold.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Review {
    /// The slice, named in the words every line about it is named in: the window
    /// and the thread are talking about the same slice and say so alike.
    slice: String,
    titles: Vec<String>,
    /// Whether the third answer is offered at all. A slice is redrafted once —
    /// see [`Choice::Feedback`]'s caller — so the second time round this window
    /// goes up with two answers on it.
    feedback: bool,
    choice: Choice,
}

impl Review {
    /// [`Choice::Skip`] is lit on open, for the reason [`Answer::No`] is the
    /// default elsewhere: the round that puts this up and an Enter straight
    /// after it cost nothing.
    #[must_use]
    pub fn open(slice: impl Into<String>, titles: Vec<String>, feedback: bool) -> Self {
        Self {
            slice: slice.into(),
            titles,
            feedback,
            choice: Choice::Skip,
        }
    }

    #[must_use]
    pub fn slice(&self) -> &str {
        &self.slice
    }

    #[must_use]
    pub fn titles(&self) -> &[String] {
        &self.titles
    }

    /// Whether the third answer is offered: see the field's own note.
    #[must_use]
    pub const fn feedback(&self) -> bool {
        self.feedback
    }

    #[must_use]
    pub const fn choice(&self) -> Choice {
        self.choice
    }

    // The one way the highlight moves, for [`Filing::with_answer`]'s reason:
    // answering re-lights the same window rather than building a second one from
    // drafts that would have to be handed over again.
    #[must_use]
    pub fn with_choice(&self, choice: Choice) -> Self {
        Self {
            choice,
            ..self.clone()
        }
    }
}

/// What a key did to the review window: moved the highlight, or answered.
///
/// Three answers and no cancel. A window that could be dismissed would leave the
/// run holding drafts nobody had decided about, so the least committal key —
/// Esc — is [`Reviewed::Skip`], which is the answer that files nothing and asks
/// what to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reviewed {
    Open(Choice),
    Create,
    Skip,
    Feedback,
}

/// The review window's keys, which are [`answer_for`]'s where the two windows
/// can mean the same thing: only presses count, Left and Right move the
/// highlight positionally, Enter answers with whatever is lit, and a key nothing
/// is bound to leaves the window exactly as it was.
///
/// The letters are the answers' own initials rather than `y`/`n`, matched by
/// character for [`answer_for`]'s reason: terminals disagree about whether shift
/// rides along with an upper-case letter, and a reader with caps lock on is
/// still answering.
///
/// The whole window and not just the lit answer, because what a key means
/// depends on the drafts: a slice that has spent its redraft has no third answer
/// for `f` or a Right to reach.
#[must_use]
pub fn review_answer_for(key: KeyEvent, review: &Review) -> Reviewed {
    let lit = review.choice();
    if key.kind != KeyEventKind::Press {
        return Reviewed::Open(lit);
    }

    match key.code {
        KeyCode::Left => Reviewed::Open(lit.leftwards()),
        KeyCode::Right => Reviewed::Open(lit.rightwards(review.feedback())),
        KeyCode::Enter => match lit {
            Choice::Create => Reviewed::Create,
            Choice::Skip => Reviewed::Skip,
            Choice::Feedback => Reviewed::Feedback,
        },
        KeyCode::Char('c' | 'C') => Reviewed::Create,
        KeyCode::Char('s' | 'S') | KeyCode::Esc => Reviewed::Skip,
        KeyCode::Char('f' | 'F') if review.feedback() => Reviewed::Feedback,
        _ => Reviewed::Open(lit),
    }
}

/// The question a skipped slice leaves behind: whether to go on to the ones
/// after it.
///
/// Its own value for [`Review`]'s reason — it is a state of the run rather than
/// a field of the session — and a two-answer question for the plainest one:
/// what it asks has a yes and a no.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Carry {
    /// What is left to offer, in the run's own words. A count said here would be
    /// a second place that has to know whether one slice is `1 slice` or
    /// `1 slices`.
    left: String,
    answer: Answer,
}

impl Carry {
    /// No is lit on open, and here that is the answer that stops: a run carries
    /// on by somebody saying so, and Esc after a skip leaves the rest of the
    /// project alone.
    #[must_use]
    pub fn open(left: impl Into<String>) -> Self {
        Self {
            left: left.into(),
            answer: Answer::No,
        }
    }

    #[must_use]
    pub fn left(&self) -> &str {
        &self.left
    }

    #[must_use]
    pub const fn answer(&self) -> Answer {
        self.answer
    }

    // The one way the highlight moves, for [`Filing::with_answer`]'s reason.
    #[must_use]
    pub fn with_answer(&self, answer: Answer) -> Self {
        Self {
            answer,
            ..self.clone()
        }
    }
}

/// [`Answered`] in the carry-on question's vocabulary: a confirmed question here
/// drafts the next slice, and a refused one ends the run with the rest of the
/// project untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CarryAnswered {
    Open(Answer),
    Stop,
    Carry,
}

/// The quit dialog's rules again, renamed rather than restated, so that the one
/// two-answer question inside a cut is answered by the very keys every other
/// one is: Esc and `n` stop, Left then Enter carries on, an immediate Enter
/// stops, a release changes nothing.
#[must_use]
pub fn carry_answer_for(key: KeyEvent, highlighted: Answer) -> CarryAnswered {
    match answer_for(key, highlighted) {
        Answered::Open(answer) => CarryAnswered::Open(answer),
        Answered::Close => CarryAnswered::Stop,
        Answered::Leave => CarryAnswered::Carry,
    }
}

#[cfg(test)]
#[path = "tests/confirm.rs"]
mod tests;
