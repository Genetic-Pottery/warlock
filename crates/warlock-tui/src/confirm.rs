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
pub(crate) enum Answer {
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
pub(crate) enum QuitConfirm {
    #[default]
    Closed,
    Open(Answer),
}

impl QuitConfirm {
    // A named constructor rather than `Open(Answer::No)` at the call site, so
    // which answer a fresh dialog starts on is decided here rather than wherever
    // Esc happens to be handled.
    #[must_use]
    pub(crate) const fn open() -> Self {
        Self::Open(Answer::No)
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn is_open(self) -> bool {
        matches!(self, Self::Open(_))
    }

    // The one way into `answer_for`: the `Option` is what keeps the key handler
    // from having to invent an answer for a dialog that is not up.
    #[must_use]
    pub(crate) const fn highlighted(self) -> Option<Answer> {
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
pub(crate) enum Answered {
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
pub(crate) fn answer_for(key: KeyEvent, highlighted: Answer) -> Answered {
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
pub(crate) struct Filing {
    project: String,
    destination: Destination,
    answer: Answer,
}

impl Filing {
    #[must_use]
    pub(crate) fn project(&self) -> &str {
        &self.project
    }

    #[must_use]
    pub(crate) const fn destination(&self) -> &Destination {
        &self.destination
    }

    #[must_use]
    pub(crate) const fn answer(&self) -> Answer {
        self.answer
    }

    // The one way the highlight moves, so answering re-lights the same question
    // rather than building a second one from strings it would have to be handed
    // again.
    #[must_use]
    pub(crate) fn with_answer(&self, answer: Answer) -> Self {
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
pub(crate) enum PushConfirm {
    #[default]
    Closed,
    Open(Filing),
}

impl PushConfirm {
    /// No is lit on open, for the reason [`Answer::No`] is the default: the
    /// keystroke that opened the dialog and an Enter straight after it come to
    /// nothing at all.
    #[must_use]
    pub(crate) fn open(project: impl Into<String>, destination: Destination) -> Self {
        Self::Open(Filing {
            project: project.into(),
            destination,
            answer: Answer::No,
        })
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    /// The one way into [`push_answer_for`] and into the drawing, for the
    /// reason [`QuitConfirm::highlighted`] is: the caller cannot invent a
    /// question that is not up.
    #[must_use]
    pub(crate) const fn filing(&self) -> Option<&Filing> {
        match self {
            Self::Closed => None,
            Self::Open(filing) => Some(filing),
        }
    }

    /// The same question with the other answer lit, and a closed dialog left
    /// closed: an arrow key pressed at nothing lights nothing.
    #[must_use]
    pub(crate) fn lit(&self, answer: Answer) -> Self {
        match self {
            Self::Closed => Self::Closed,
            Self::Open(filing) => Self::Open(filing.with_answer(answer)),
        }
    }
}

/// [`Answered`] in this dialog's vocabulary: a confirmed question here means
/// send, and warlock goes on running either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PushAnswered {
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
pub(crate) fn push_answer_for(key: KeyEvent, highlighted: Answer) -> PushAnswered {
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
pub(crate) struct Cutting {
    project: String,
    status: String,
    slices: usize,
    team: String,
    key: String,
    answer: Answer,
}

impl Cutting {
    #[must_use]
    pub(crate) fn project(&self) -> &str {
        &self.project
    }

    /// The status as the board spelled it: see the type's own note.
    #[must_use]
    pub(crate) fn status(&self) -> &str {
        &self.status
    }

    /// How many slices the project's scope has, which is how much work the
    /// answer is about.
    #[must_use]
    pub(crate) const fn slices(&self) -> usize {
        self.slices
    }

    #[must_use]
    pub(crate) fn team(&self) -> &str {
        &self.team
    }

    /// The *name* the key is held under, never a key value: see the type's own
    /// note.
    #[must_use]
    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub(crate) const fn answer(&self) -> Answer {
        self.answer
    }

    // The one way the highlight moves, for [`Filing::with_answer`]'s reason:
    // answering re-lights the same question rather than building a second one
    // from facts that would have to be fetched again.
    #[must_use]
    pub(crate) fn with_answer(&self, answer: Answer) -> Self {
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
pub(crate) enum CutConfirm {
    #[default]
    Closed,
    Open(Cutting),
}

impl CutConfirm {
    /// No is lit on open, for the reason [`Answer::No`] is the default: the
    /// round that puts this up and an Enter straight after it come to nothing
    /// at all.
    #[must_use]
    pub(crate) fn open(
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
    #[cfg(test)]
    pub(crate) const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    /// The one way into [`cut_answer_for`] and into the drawing, for the
    /// reason [`QuitConfirm::highlighted`] is: the caller cannot invent a
    /// question that is not up.
    #[must_use]
    pub(crate) const fn cutting(&self) -> Option<&Cutting> {
        match self {
            Self::Closed => None,
            Self::Open(cutting) => Some(cutting),
        }
    }

    /// The same question with the other answer lit, and a closed dialog left
    /// closed: an arrow key pressed at nothing lights nothing.
    #[must_use]
    pub(crate) fn lit(&self, answer: Answer) -> Self {
        match self {
            Self::Closed => Self::Closed,
            Self::Open(cutting) => Self::Open(cutting.with_answer(answer)),
        }
    }
}

/// [`Answered`] in this dialog's vocabulary: a confirmed question here starts
/// the run, and warlock goes on running either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CutAnswered {
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
pub(crate) fn cut_answer_for(key: KeyEvent, highlighted: Answer) -> CutAnswered {
    match answer_for(key, highlighted) {
        Answered::Open(answer) => CutAnswered::Open(answer),
        Answered::Close => CutAnswered::Cancel,
        Answered::Leave => CutAnswered::Cut,
    }
}

// The ticket a `/pull` is about to work and where the work will land. Named for
// what the answer commits to — undertaking a whole ticket — rather than for the
// command, because the run itself is `Pulling` elsewhere and one word for both
// would read as one thing.
//
// Every fact here was read off the board or worked out from the run record a
// moment ago, and is parked for [`Cutting`]'s reason: none of them can be had
// again without a second request, and a Yes should not have to ask twice.
//
// There is no key field at all, which is [`Filing`]'s note taken one step
// further: a pull resolves the board the machine's own way, so this value has
// neither a key nor a name for one, and a dialog with nowhere to put a key
// cannot draw one, print one or grow one in a `Debug` rendering.
//
// `resuming` is `Some` exactly when this is a run being carried on, and what it
// carries is the sub-task the next session starts from. Two facts in one field
// because they are one fact: a fresh pull resumes nothing and so has no sub-task
// to name, and "resuming, from nowhere" is not a state a run is ever in.
//
// The lit answer rides along inside it for [`QuitConfirm`]'s reason — it exists
// exactly as long as the question does — which is why this is only ever built
// through [`PullConfirm::open`] or [`PullConfirm::resuming`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Undertaking {
    ticket: String,
    title: String,
    scope: String,
    team: String,
    branch: String,
    resuming: Option<String>,
    answer: Answer,
}

impl Undertaking {
    /// The ticket's identifier, in the spelling every line about this run names
    /// it in.
    #[must_use]
    pub(crate) fn ticket(&self) -> &str {
        &self.ticket
    }

    #[must_use]
    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    /// The scope the ticket was taken from, which is the boundary the run is
    /// allowed to work inside.
    #[must_use]
    pub(crate) fn scope(&self) -> &str {
        &self.scope
    }

    #[must_use]
    pub(crate) fn team(&self) -> &str {
        &self.team
    }

    /// The branch the run will create, named before anything is created: see
    /// the type's own note about why it is parked here.
    #[must_use]
    pub(crate) fn branch(&self) -> &str {
        &self.branch
    }

    /// The sub-task a resumed run carries on from, and `None` for a fresh pull:
    /// see the field's own note.
    #[must_use]
    pub(crate) fn resuming(&self) -> Option<&str> {
        self.resuming.as_deref()
    }

    #[must_use]
    pub(crate) const fn answer(&self) -> Answer {
        self.answer
    }

    // The one way the highlight moves, for [`Filing::with_answer`]'s reason:
    // answering re-lights the same question rather than building a second one
    // from facts that would have to be fetched again.
    #[must_use]
    pub(crate) fn with_answer(&self, answer: Answer) -> Self {
        Self {
            answer,
            ..self.clone()
        }
    }
}

/// The question a `/pull` asks between choosing the ticket and starting the
/// run, drawn over the frame the way the other dialogs are and answered by the
/// very same rules — [`pull_answer_for`] is [`answer_for`] with the answers
/// renamed.
///
/// A separate value from [`CutConfirm`] because the two are answered about
/// different things and say so in their types: a confirmed question there
/// drafts tickets for a project, and a confirmed question here checks out a
/// branch and works one ticket to a pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub(crate) enum PullConfirm {
    #[default]
    Closed,
    Open(Undertaking),
}

impl PullConfirm {
    /// A ticket taken fresh: No is lit on open, for the reason [`Answer::No`]
    /// is the default — the round that puts this up and an Enter straight after
    /// it come to nothing at all.
    #[must_use]
    pub(crate) fn open(
        ticket: impl Into<String>,
        title: impl Into<String>,
        scope: impl Into<String>,
        team: impl Into<String>,
        branch: impl Into<String>,
    ) -> Self {
        Self::Open(Undertaking {
            ticket: ticket.into(),
            title: title.into(),
            scope: scope.into(),
            team: team.into(),
            branch: branch.into(),
            resuming: None,
            answer: Answer::No,
        })
    }

    /// The same question about a run being carried on, which names the sub-task
    /// the next session starts from. A constructor of its own rather than an
    /// `Option` on [`PullConfirm::open`], so a caller resuming a run cannot
    /// forget to say where it resumes from.
    #[must_use]
    pub(crate) fn resuming(
        ticket: impl Into<String>,
        title: impl Into<String>,
        scope: impl Into<String>,
        team: impl Into<String>,
        branch: impl Into<String>,
        subtask: impl Into<String>,
    ) -> Self {
        Self::Open(Undertaking {
            ticket: ticket.into(),
            title: title.into(),
            scope: scope.into(),
            team: team.into(),
            branch: branch.into(),
            resuming: Some(subtask.into()),
            answer: Answer::No,
        })
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn is_open(&self) -> bool {
        matches!(self, Self::Open(_))
    }

    /// The one way into [`pull_answer_for`] and into the drawing, for the
    /// reason [`QuitConfirm::highlighted`] is: the caller cannot invent a
    /// question that is not up.
    #[must_use]
    pub(crate) const fn undertaking(&self) -> Option<&Undertaking> {
        match self {
            Self::Closed => None,
            Self::Open(undertaking) => Some(undertaking),
        }
    }

    /// The same question with the other answer lit, and a closed dialog left
    /// closed: an arrow key pressed at nothing lights nothing.
    #[must_use]
    pub(crate) fn lit(&self, answer: Answer) -> Self {
        match self {
            Self::Closed => Self::Closed,
            Self::Open(undertaking) => Self::Open(undertaking.with_answer(answer)),
        }
    }
}

/// [`Answered`] in this dialog's vocabulary: a confirmed question here starts
/// the pull, and warlock goes on running either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PullAnswered {
    Open(Answer),
    Cancel,
    Pull,
}

/// The quit dialog's rules, renamed rather than restated: Esc and `n` cancel,
/// Left then Enter starts the run, an immediate Enter cancels, a release
/// changes nothing, and every other key leaves the question exactly as it was.
/// Written over [`answer_for`] so the four cannot drift — a key that moves one
/// moves them all.
#[must_use]
pub(crate) fn pull_answer_for(key: KeyEvent, highlighted: Answer) -> PullAnswered {
    match answer_for(key, highlighted) {
        Answered::Open(answer) => PullAnswered::Open(answer),
        Answered::Close => PullAnswered::Cancel,
        Answered::Leave => PullAnswered::Pull,
    }
}

/// What one slice's drafts are answered with, in the order of Forman's prompt:
/// file them, edit them, leave the slice alone, or say what is wrong with them.
///
/// Four answers and so a value of its own rather than [`Answer`] renamed:
/// the windows above ask one thing and take a yes or a no, and this asks which
/// of four things to do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) enum Choice {
    /// File this slice's drafts as issues. The default: see [`Review::open`].
    #[default]
    Create,
    /// Open the drafts in `$EDITOR`, and ask again about what was saved.
    Edit,
    /// Leave the slice, recorded so no later run offers it.
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
            Self::Create | Self::Edit => Self::Create,
            Self::Skip => Self::Edit,
            Self::Feedback => Self::Skip,
        }
    }

    const fn rightwards(self) -> Self {
        match self {
            Self::Create => Self::Edit,
            Self::Edit => Self::Skip,
            Self::Skip | Self::Feedback => Self::Feedback,
        }
    }
}

/// One slice's drafts, waiting to be answered about: what was drafted, for which
/// slice, and which of the answers is lit.
///
/// The window shows how many drafts there are and nothing of them: every draft
/// is on the panel's document card behind it, whole, so the window stays small
/// and off the text it is asking about.
///
/// There is no `Closed` variant beside this the way there is for the three
/// dialogs above, and deliberately: those are fields of the session, which is
/// always there, and this is a state of one slice of one run — it exists exactly
/// as long as that slice is waiting, and a caller with no run has nothing to
/// hold.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Review {
    /// The slice, named in the words every line about it is named in: the window
    /// and the thread are talking about the same slice and say so alike.
    slice: String,
    titles: Vec<String>,
    choice: Choice,
}

impl Review {
    /// [`Choice::Create`] is lit on open, as Forman reads an empty line at its
    /// review: somebody who has read the drafts and pressed Enter has agreed
    /// with them. The drafts are on the panel behind this window before it is
    /// up.
    #[must_use]
    pub(crate) fn open(slice: impl Into<String>, titles: Vec<String>) -> Self {
        Self {
            slice: slice.into(),
            titles,
            choice: Choice::Create,
        }
    }

    #[must_use]
    pub(crate) fn slice(&self) -> &str {
        &self.slice
    }

    #[must_use]
    pub(crate) fn titles(&self) -> &[String] {
        &self.titles
    }

    #[must_use]
    pub(crate) const fn choice(&self) -> Choice {
        self.choice
    }

    // The one way the highlight moves, for [`Filing::with_answer`]'s reason:
    // answering re-lights the same window rather than building a second one from
    // drafts that would have to be handed over again.
    #[must_use]
    pub(crate) fn with_choice(&self, choice: Choice) -> Self {
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
pub(crate) enum Reviewed {
    Open(Choice),
    Create,
    Edit,
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
#[must_use]
pub(crate) fn review_answer_for(key: KeyEvent, review: &Review) -> Reviewed {
    let lit = review.choice();
    if key.kind != KeyEventKind::Press {
        return Reviewed::Open(lit);
    }

    match key.code {
        KeyCode::Left => Reviewed::Open(lit.leftwards()),
        KeyCode::Right => Reviewed::Open(lit.rightwards()),
        KeyCode::Enter => match lit {
            Choice::Create => Reviewed::Create,
            Choice::Edit => Reviewed::Edit,
            Choice::Skip => Reviewed::Skip,
            Choice::Feedback => Reviewed::Feedback,
        },
        KeyCode::Char('c' | 'C') => Reviewed::Create,
        KeyCode::Char('e' | 'E') => Reviewed::Edit,
        KeyCode::Char('s' | 'S') | KeyCode::Esc => Reviewed::Skip,
        KeyCode::Char('f' | 'F') => Reviewed::Feedback,
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
pub(crate) struct Carry {
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
    pub(crate) fn open(left: impl Into<String>) -> Self {
        Self {
            left: left.into(),
            answer: Answer::No,
        }
    }

    #[must_use]
    pub(crate) fn left(&self) -> &str {
        &self.left
    }

    #[must_use]
    pub(crate) const fn answer(&self) -> Answer {
        self.answer
    }

    // The one way the highlight moves, for [`Filing::with_answer`]'s reason.
    #[must_use]
    pub(crate) fn with_answer(&self, answer: Answer) -> Self {
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
pub(crate) enum CarryAnswered {
    Open(Answer),
    Stop,
    Carry,
}

/// The quit dialog's rules again, renamed rather than restated, so that the one
/// two-answer question inside a cut is answered by the very keys every other
/// one is: Esc and `n` stop, Left then Enter carries on, an immediate Enter
/// stops, a release changes nothing.
#[must_use]
pub(crate) fn carry_answer_for(key: KeyEvent, highlighted: Answer) -> CarryAnswered {
    match answer_for(key, highlighted) {
        Answered::Open(answer) => CarryAnswered::Open(answer),
        Answered::Close => CarryAnswered::Stop,
        Answered::Leave => CarryAnswered::Carry,
    }
}

#[cfg(test)]
#[path = "tests/confirm.rs"]
mod tests;
