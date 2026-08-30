//! What the reader asked, and what came back.
//!
//! The panel's other two cards are a ledger and a file: an [`Account`] is one
//! pact writing documents, and a document is what it wrote. This one is the
//! conversation — the third card, and the only place in warlock where a model's
//! prose is ever shown. It is an ordered list of turns, one per message somebody
//! typed at the foot of the panel, each holding that message, the work the model
//! was seen doing while it answered, and the answer — and, among them, the runs
//! that happened while the conversation was going on.
//!
//! It is plain data like the account is: no terminal, no channel, no child
//! process, and no clock of its own. [`Instant::now`] is never called in this
//! file, so a whole conversation can be driven through it in a test off `base +
//! Duration::from_secs` with nothing attached to stdout, no `claude` installed
//! and no thread started.
//!
//! # A turn is a section that somebody asked for
//!
//! The clock rule is the account's, to the letter, and is the account's *code*
//! as well — both are built on [`Log`], so a work line here counts from the
//! start of the turn it belongs to, the newest one ticks against whatever `now`
//! the caller hands in, and it freezes at the instant the next line arrives.
//! A turn that has heard nothing yet shows the same `waiting` placeholder, for
//! the same reason: the first thing a model does with a question is spend a
//! while not answering it, and a still screen is what a hang looks like.
//!
//! The work lines are the account's shape too — a tool call is its name and the
//! one detail it carries, thinking is the word `thinking`, writing is the word
//! `writing` — because a `Grep` is a `Grep` whichever card it turns up on. And
//! what is kept out is kept out just as firmly: no tool results, no model
//! reasoning, no fragment of the answer arriving early. The answer is the one
//! piece of prose a turn has, it is a value of its own, and it lands whole.
//!
//! # A turn nobody typed
//!
//! A pact or a refresh started while the conversation is on screen is a third
//! thing that happened, in the same sequence as the questions around it, so it
//! is a turn too: [`Thread::open_run`] opens one at the instant the key was
//! pressed, and it holds an [`Account`] — the very value the account card holds
//! for the same run — fed through [`Thread::run_mut`] as the run's events
//! arrive.
//!
//! It holds one rather than copying anything out of one, and that is the whole
//! design. Every row a run has here is a row [`Account`] made: the directory
//! headings, `thinking`, a tool and its one detail, `summarising … (2/3)`, `wrote
//! … — 2341 bytes, $0.21`, `refused — …`, `cancelled — $0.03 spent`, `pact
//! finished — …`. Nothing in this file re-words any of it, because a second
//! spelling of a run's line is a second thing to keep in step with the card, and
//! two cards disagreeing about what one run did is worse than either of them
//! being wrong.
//!
//! What a run turn does *not* borrow is the conversation's vocabulary. It has no
//! message above it, no answer under it, and no [`Ending`] — a cancelled run
//! says `cancelled — $0.03 spent` where a cancelled question says `the turn was
//! cancelled`, because one of them is a pass over a directory and the other is
//! somebody's question, and a reader should never have to work out which they
//! are looking at. The money keeps the same distance: a run's is its account's,
//! per pass and totalled in `pact finished — …`, and a question's is the
//! [`Turn::cost`] line below, which says out loud that it belongs to no total.
//! Neither is ever added to the other, on screen or anywhere else.
//!
//! # Where this one differs
//!
//! Two things, both deliberate.
//!
//! A turn's money is said in words that cannot be mistaken for a pact's. The
//! account spells a cost as part of what a pass produced — `wrote … — 2341
//! bytes, $0.21` — and totals those into `pact finished — …`; a turn says
//! [`this turn cost $0.02 — chat, never added to a pact's total`][Turn::cost],
//! on a line of its own, which is a sentence no arithmetic can turn into a
//! subtotal of the other. The two numbers are never added together on screen,
//! and this is where that promise is kept, because the wording is the only
//! place it *can* be kept once both are rows in the same panel.
//!
//! And a turn ends in one line however it ends. A pact has three
//! [`Outcome`](crate::Outcome)s because a pass either writes a document, is
//! refused, or is stopped; a turn has an answer or an [`Ending`] — cancelled,
//! no `claude` to ask, a non-zero exit, a timeout, or a model that finished with
//! nothing to say. Each is one line, said about the turn rather than about the
//! model, so a failed turn costs the reader a row and not a screen. Nothing here
//! returns an error to anybody: a turn that could not be run is a line in the
//! panel, which is the whole reason the endings are values.

use std::time::{Duration, Instant};

use warlock_engine::AgentError;

use crate::account::{Account, Line, Log, THINKING, WRITING, money, tool_line};
use crate::claude::Activity;

/// One thing that stopped a turn short of an answer, in the words it ends with.
///
/// Five ways and no sixth that a caller has to word for itself: the four the
/// model seam fails in ([`AgentError`], mapped by [`ending_for`]), plus the one
/// warlock does on purpose. [`Ending::Broke`] is the catch — `AgentError` is
/// `#[non_exhaustive]`, so a variant added over there arrives here as whatever
/// it says about itself rather than as a panic or a silent nothing.
///
/// Every line it makes is about *the turn*: `the turn was cancelled`, `the turn
/// failed — …`. Never about the model, because none of these is a thing the
/// model did — it was stopped, or never reached, or not installed — and never in
/// an [`Account`](crate::Account)'s words, because a reader looking at the panel
/// should not have to work out which card they are on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// Somebody stopped the turn before the answer landed.
    ///
    /// Carries nothing: what was said before the cancel is already in the turn,
    /// and what it had spent by then is already the cost line's business.
    Cancelled,
    /// There is no such program to ask, which on a fresh machine is the
    /// ordinary state of the world rather than a fault.
    NoModel {
        /// The command that was looked for, e.g. `claude`.
        program: String,
    },
    /// The turn ran and exited non-zero: something went wrong on the far side.
    Failed {
        /// The exit code, or `None` if it was killed before it could set one.
        code: Option<i32>,
        /// What it wrote to stderr, verbatim and possibly over several lines —
        /// flattened into one line on the way to the panel, because a row is a
        /// row.
        stderr: String,
    },
    /// The turn ran past the time it was given and was stopped.
    TimedOut {
        /// How long it was given.
        after: Duration,
    },
    /// The turn finished cleanly and said nothing, so there is no answer to
    /// draw. Its own ending rather than an empty answer, because a blank turn
    /// on screen is indistinguishable from one that is still going.
    NothingSaid,
    /// The turn could not be run for some other reason: a pipe broke, a handle
    /// could not be opened, or the seam grew a failure this list has not been
    /// taught yet.
    Broke {
        /// What went wrong, in whatever words it came in.
        reason: String,
    },
}

impl Ending {
    /// The one line a turn ends with.
    ///
    /// One line and never two, whatever it carries: a stderr with a stack trace
    /// in it is flattened to a single row here and cut to the panel's width by
    /// whoever knows the width, the same division of labour the account keeps.
    ///
    /// Public because a failed turn is said in two places — the row under the
    /// question, and the footer, for a reader who is looking at another card —
    /// and those two have to be one sentence rather than two spellings that
    /// happen to agree. Whoever ends a turn puts this on the footer and hands
    /// the [`Ending`] itself to [`Thread::end`], which files the very same
    /// string as a line.
    ///
    /// ```
    /// use warlock_tui::Ending;
    ///
    /// assert_eq!(Ending::Cancelled.line(), "the turn was cancelled");
    /// ```
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::Cancelled => "the turn was cancelled".to_owned(),
            Self::NoModel { program } => {
                format!("the turn could not run — no {program} on PATH")
            }
            Self::Failed { code, stderr } => {
                let said = one_line(stderr);
                let said = if said.is_empty() {
                    String::new()
                } else {
                    format!(": {said}")
                };
                match code {
                    Some(code) => format!("the turn failed — exit status {code}{said}"),
                    None => format!("the turn failed — stopped before it exited{said}"),
                }
            }
            Self::TimedOut { after } => {
                format!("the turn ran past {}s and was stopped", after.as_secs())
            }
            Self::NothingSaid => "the turn ended with nothing to say".to_owned(),
            Self::Broke { reason } => format!("the turn could not run — {reason}"),
        }
    }
}

/// Which [`Ending`] a failed turn is.
///
/// The one place the model seam's failure vocabulary becomes the panel's, so
/// whoever runs a turn hands the error over rather than wording it — the same
/// arrangement as [`answer_for`](crate::answer_for) and
/// [`compose_for`](crate::compose_for), a value in and a value out.
///
/// A cancel does not come through here. It reaches a caller as an ordinary I/O
/// error — a killed child is a broken pipe, and nothing about the errno says who
/// killed it — so whether a turn was cancelled is a fact the caller holds and
/// [`Ending::Cancelled`] is theirs to choose.
#[must_use]
pub fn ending_for(error: &AgentError) -> Ending {
    match error {
        AgentError::NotFound { program } => Ending::NoModel {
            program: program.clone(),
        },
        AgentError::Failed { code, stderr } => Ending::Failed {
            code: *code,
            stderr: stderr.clone(),
        },
        AgentError::TimedOut { after } => Ending::TimedOut { after: *after },
        AgentError::EmptyOutput => Ending::NothingSaid,
        // `AgentError` is `#[non_exhaustive]`: `Io` lands here today, and so
        // does whatever the seam learns to fail with next. Its own `Display` is
        // one line already, which is exactly what a row needs.
        other => Ending::Broke {
            reason: other.to_string(),
        },
    }
}

/// One entry of the conversation: a question somebody typed, or a run nobody
/// did.
///
/// A typed turn is opened by [`Thread::ask`] and closed by [`Thread::answer`]
/// or [`Thread::end`]; a run turn is opened by [`Thread::open_run`], fed
/// through [`Thread::run_mut`] and closed by [`Thread::close_run`]. Between the
/// two ends it is the live turn — the one whose newest work line ticks and the
/// only one anything can be filed under — and a closed turn never moves again,
/// however long the session goes on.
///
/// Holds an `f64` cost, so it is [`PartialEq`] and not [`Eq`], as everything
/// that comes to hold one of these is.
#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    /// Which of the two kinds of turn this is, and everything that kind holds.
    kind: Kind,
}

/// The two kinds of turn, and the whole difference between them.
///
/// Two and no third: either a reader typed something at the foot of the panel
/// and a model answered it, or warlock ran a pact and wrote an account of it.
/// They are one type because they are one sequence — the reader reads down the
/// card and finds what happened in the order it happened — and they are two
/// variants because nothing about them is shared except that order: a question
/// has an answer and a cost line of its own, and a run has an [`Account`], which
/// already knows how to word every line it will ever have.
#[derive(Debug, Clone, PartialEq)]
enum Kind {
    /// A turn somebody typed.
    Said(Said),
    /// A turn nobody typed: a run, appearing in the conversation as itself.
    Ran(Ran),
}

/// One question, everything the model was seen doing about it, and the answer.
#[derive(Debug, Clone, PartialEq)]
struct Said {
    /// The message the reader typed, exactly as they typed it, newlines and
    /// all. Never wrapped here: the width is a fact about a terminal.
    message: String,
    /// When the turn was asked, what has been filed under it, and whether its
    /// clock is still moving.
    log: Log,
    /// What came back, whole and unwrapped, once it has.
    answer: Option<String>,
    /// How it ended instead, if it did. Never both this and an answer: the
    /// first of the two to arrive closes the turn.
    ending: Option<Ending>,
    /// What the turn reported spending, summed over however many times it said
    /// so. `None` means it never said, which is not the same as zero.
    cost: Option<f64>,
}

/// A run of warlock's own, as the conversation holds it: the account it is
/// writing, and whether it is over.
///
/// The account is the whole of it. Every line a run has in the thread is a line
/// the account made — [`Section`](crate::Section), the outcome wordings, the
/// summary, the clocks — so the rows here and the rows on the account card are
/// the same rows from the same code, and there is no second spelling of a run's
/// lines for the two to drift apart in.
#[derive(Debug, Clone, PartialEq)]
struct Ran {
    /// What the run has done, in the run's own words.
    account: Account,
    /// When the run stopped, if it has. The account cannot answer this for
    /// itself — a run that has finished and a run that has opened no directory
    /// yet both have nothing live in them — and the answer is what says whether
    /// the composer is muted, so it is kept here rather than guessed at.
    closed: Option<Instant>,
}

impl Turn {
    /// The message this turn was asked in, or `None` for a turn nobody typed.
    ///
    /// `None` is the run turns: a pact is started with a keystroke and says
    /// nothing on its way in, so there is no message to draw above its lines and
    /// an empty string would be a row of the reader's own words that they never
    /// said.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        match &self.kind {
            Kind::Said(said) => Some(&said.message),
            Kind::Ran(_) => None,
        }
    }

    /// The account of the run this turn is, or `None` for a turn somebody
    /// typed.
    ///
    /// The run's own record, whole: what a reader sees on this turn is what
    /// they see on the account card, because it is this.
    #[must_use]
    pub const fn account(&self) -> Option<&Account> {
        match &self.kind {
            Kind::Said(_) => None,
            Kind::Ran(ran) => Some(&ran.account),
        }
    }

    /// What came back, or `None` while the turn is still going — and for good
    /// on a turn that ended without answering.
    ///
    /// Whole and unwrapped, as it arrived. Breaking it into the rows a panel of
    /// some width can draw it in happens on the way to the screen, so a terminal
    /// made narrower re-flows an answer the reader is looking at rather than
    /// asking the model again.
    ///
    /// Always `None` on a run turn: a pact answers nobody, it writes documents,
    /// and what it wrote is its account's outcome line.
    #[must_use]
    pub fn answer(&self) -> Option<&str> {
        match &self.kind {
            Kind::Said(said) => said.answer.as_deref(),
            Kind::Ran(_) => None,
        }
    }

    /// How this turn ended short of an answer, or `None` if it did not.
    ///
    /// Always `None` on a run turn, and deliberately: an [`Ending`] is worded
    /// about a *turn* — `the turn was cancelled` — and a cancelled run says
    /// `cancelled — $0.03 spent` under the directory it was stopped in, in the
    /// account's words. Two wordings for one run is exactly what the account
    /// being the run's only voice avoids.
    #[must_use]
    pub const fn ending(&self) -> Option<&Ending> {
        match &self.kind {
            Kind::Said(said) => said.ending.as_ref(),
            Kind::Ran(_) => None,
        }
    }

    /// What this turn reported spending, or `None` if it never reported.
    ///
    /// Its own number, said on its own line — `this turn cost $0.02 — chat,
    /// never added to a pact's total` — and deliberately not summed with any
    /// other turn's or with a pact's. A conversation is not a budget.
    ///
    /// Always `None` on a run turn, for the reason [`Turn::ending`] is: a run's
    /// money is the account's arithmetic, said per pass in its outcome lines and
    /// totalled in `pact finished — …`, and a number handed out here would be
    /// the one somebody adds to a chat's.
    #[must_use]
    pub const fn cost(&self) -> Option<f64> {
        match &self.kind {
            Kind::Said(said) => said.cost,
            Kind::Ran(_) => None,
        }
    }

    /// When this turn was asked, which is where its clocks count from — or,
    /// for a run turn, when the run started.
    #[must_use]
    pub const fn started(&self) -> Instant {
        match &self.kind {
            Kind::Said(said) => said.log.started(),
            Kind::Ran(ran) => ran.account.started(),
        }
    }

    /// Whether this turn has stopped moving: answered, ended, run to a finish,
    /// or overtaken by a newer turn.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        match &self.kind {
            Kind::Said(said) => said.log.is_closed(),
            Kind::Ran(ran) => ran.closed.is_some(),
        }
    }

    /// How many rows this turn draws as, before anything is wrapped to a width.
    ///
    /// A typed turn is the message's own lines, the work lines — at least one,
    /// since a turn that has heard nothing draws the `waiting` placeholder — the
    /// answer's own lines, and the cost line where there is a cost. A run turn
    /// is however many rows its account has, counted by the account, because
    /// they are the account's rows.
    fn line_count(&self) -> usize {
        match &self.kind {
            Kind::Said(said) => said.line_count(),
            Kind::Ran(ran) => ran.account.line_count(),
        }
    }

    /// Every row of this turn, in the order a reader reads them, with clocks
    /// measured against `now`.
    ///
    /// Boxed because the two kinds of turn draw out of two different iterators
    /// and the whole point of the second one is that it is the account's: a run
    /// turn yields [`Account`]'s own rows, unaltered and unre-worded, and one
    /// allocation per turn per frame is the price of there being one place in
    /// warlock that says what a run's line reads like.
    fn rows(&self, now: Instant) -> Box<dyn Iterator<Item = Line> + '_> {
        match &self.kind {
            Kind::Said(said) => Box::new(said.rows(now)),
            Kind::Ran(ran) => Box::new(ran.account.rows(now)),
        }
    }

    /// Stop this turn moving as of `at`, if it has not stopped already.
    ///
    /// What a newer turn does to the one above it, and what closes a run. It
    /// adds no line either way: a typed turn overtaken by a newer question is
    /// simply frozen where it got to, and a run's last word is its account's
    /// summary rather than anything the thread has to say about it.
    ///
    /// A run turn's account is frozen along with the turn, so the run's newest
    /// line stops counting up rather than ticking on under a conversation that
    /// has moved past it. `Account::freeze` is idempotent, so a run whose
    /// account was already finished keeps the instant it finished at.
    fn freeze(&mut self, at: Instant) {
        match &mut self.kind {
            Kind::Said(said) => said.log.freeze(at),
            Kind::Ran(ran) => {
                ran.account.freeze(at);
                ran.closed.get_or_insert(at);
            }
        }
    }

    /// This turn as the typed turn it is, while it is still live.
    ///
    /// The one gate everything a conversation does goes through: a message's
    /// activity, its answer and its ending are all filed here or dropped. A run
    /// turn is never it — nothing the model says in a chat belongs under a run
    /// nobody typed — and neither is a turn that is over, for the reason
    /// [`Thread::record`] gives.
    fn live_said(&mut self) -> Option<&mut Said> {
        if self.is_closed() {
            return None;
        }
        match &mut self.kind {
            Kind::Said(said) => Some(said),
            Kind::Ran(_) => None,
        }
    }

    /// This turn as the run it is, while that run is still going.
    ///
    /// [`Turn::live_said`]'s counterpart, and the same rule the other way
    /// round: a run's events reach its account through here and nothing else,
    /// so they cannot land on a typed turn or on a run that has been closed.
    fn live_run(&mut self) -> Option<&mut Account> {
        if self.is_closed() {
            return None;
        }
        match &mut self.kind {
            Kind::Said(_) => None,
            Kind::Ran(ran) => Some(&mut ran.account),
        }
    }
}

impl Said {
    /// How many rows this turn draws as, before anything is wrapped to a width.
    fn line_count(&self) -> usize {
        broken(&self.message).count()
            + self.log.row_count()
            + self
                .answer
                .as_deref()
                .map_or(0, |answer| broken(answer).count())
            + usize::from(self.cost.is_some())
    }

    /// Every row of this turn, in the order a reader reads them, with clocks
    /// measured against `now`.
    ///
    /// The question, then the work, then the answer, then the money. An ending
    /// needs no arm of its own: it is filed as an ordinary line when the turn
    /// closes, so it clocks and freezes like everything else and sits exactly
    /// where a reader is already looking.
    fn rows(&self, now: Instant) -> impl Iterator<Item = Line> + '_ {
        let said = broken(&self.message).map(|text| Line::Said {
            text: text.to_owned(),
        });
        let answer = self
            .answer
            .iter()
            .flat_map(|answer| broken(answer))
            .map(|text| Line::Text {
                text: text.to_owned(),
            });
        let spent = self.cost.map(|usd| Line::Summary { text: spend(usd) });

        said.chain(self.log.rows(now)).chain(answer).chain(spent)
    }

    /// Close this turn at `at` with the line `ending` makes.
    ///
    /// Reached only through [`Turn::live_said`], which is what makes the first
    /// ending win: a turn that is closed already is not a live one, so a cancel
    /// that lands a moment before the answer does is still the cancel that is on
    /// screen.
    fn word(&mut self, ending: &Ending, at: Instant) {
        self.log.push(ending.line(), at);
        self.ending = Some(ending.clone());
        self.log.freeze(at);
    }
}

/// The conversation, from the first question to the last answer — and every run
/// that happened while it was going on.
///
/// One session, one thread: warlock's chat is one conversation for the life of
/// the process, so turns are appended and nothing is ever dropped or trimmed —
/// a reader can scroll back to what they asked ten minutes ago, and the model
/// remembers it too, which is [`ChatAgent`](crate::ChatAgent)'s half of the same
/// arrangement.
///
/// Driven by four calls for a question, all of which take the instant they
/// happened at: [`Thread::ask`] when a message is submitted, [`Thread::record`]
/// for every activity the turn reports, and [`Thread::answer`] or
/// [`Thread::end`] when it is over. And three for a run, which is a turn nobody
/// typed: [`Thread::open_run`] when it starts, [`Thread::run_mut`] to feed its
/// [`Account`] as the run's own events arrive, and [`Thread::close_run`] when it
/// is over however it went. Read back as rows with [`Thread::lines`] or
/// [`Thread::window`], which take the `now` the newest clock is measured
/// against.
///
/// ```
/// use std::time::{Duration, Instant};
///
/// use warlock_tui::{Activity, Line, Thread};
///
/// let base = Instant::now();
/// let mut thread = Thread::new();
///
/// thread.ask("what is in crates/warlock-engine?", base);
/// thread.record(&Activity::Thinking, base + Duration::from_secs(1));
///
/// // Nothing newer has arrived, so the newest line counts up with `now`.
/// assert_eq!(
///     thread.lines(base + Duration::from_secs(9)),
///     vec![
///         Line::Said { text: "what is in crates/warlock-engine?".to_owned() },
///         Line::Clocked { clock: "0:09".to_owned(), text: "thinking".to_owned() },
///     ],
/// );
/// ```
///
/// A run started under that question takes a turn of its own, and what it draws
/// as is its account — the same rows, from the same code, as the account card
/// shows for the same run:
///
/// ```
/// use std::time::{Duration, Instant};
///
/// use warlock_tui::{Activity, Line, Thread};
///
/// let base = Instant::now();
/// let mut thread = Thread::new();
///
/// thread.open_run(base);
/// let run = thread.run_mut().expect("the run is the live turn");
/// run.open_section("crates/engine", base);
/// run.record(&Activity::Thinking, base + Duration::from_secs(1));
///
/// assert_eq!(
///     thread.lines(base + Duration::from_secs(9)),
///     vec![
///         Line::Directory { path: "crates/engine".into() },
///         Line::Clocked { clock: "0:09".to_owned(), text: "thinking".to_owned() },
///     ],
/// );
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Thread {
    /// The turns, in the order they were asked — a run among them at the
    /// position it started at.
    turns: Vec<Turn>,
}

impl Thread {
    /// A conversation nobody has said anything in yet.
    ///
    /// Empty means empty: no turns, no lines, nothing to draw. A panel whose
    /// thread card has never been asked a question draws warlock's mark, which
    /// is the same fact said one level up.
    #[must_use]
    pub const fn new() -> Self {
        Self { turns: Vec::new() }
    }

    /// Ask `message` at `at`, and freeze the turn above it.
    ///
    /// The clock under this question counts from `at`, so every turn starts
    /// again at `0:00` — and it is on screen from this call, as the `waiting`
    /// placeholder, rather than from whenever the model first says something.
    /// Whatever turn was live stops here: one question at a time is the rule
    /// the event loop keeps, and a turn still ticking under a newer one would
    /// be a second answer to that question.
    pub fn ask(&mut self, message: impl Into<String>, at: Instant) {
        self.freeze_last(at);
        self.turns.push(Turn {
            kind: Kind::Said(Said {
                message: message.into(),
                log: Log::opened_at(at),
                answer: None,
                ending: None,
                cost: None,
            }),
        });
    }

    /// Open a turn nobody typed at `at`: a run, appended in order like any
    /// other turn.
    ///
    /// The counterpart of [`Thread::ask`], and the same call in every way but
    /// the message there is none of. A pact or a refresh starting while the
    /// conversation is on screen has to go *into* the conversation — a card
    /// swapped away under the reader, or a second history beside this one, is
    /// the single sequence of what happened coming apart — so a run takes a turn
    /// of its own, at the position it started at, and whatever was live above it
    /// freezes exactly as a new question would have frozen it.
    ///
    /// What the turn holds is an [`Account`], starting at `at`, which is the
    /// same value the account card holds for the same run. Nothing about the run
    /// is worded here and nothing ever will be: the lines are the account's,
    /// from the account's own code, so a run reads the same in the conversation
    /// as it does on its own card, down to the character.
    ///
    /// Feed it with [`Thread::run_mut`] and close it with [`Thread::close_run`].
    pub fn open_run(&mut self, at: Instant) {
        self.freeze_last(at);
        self.turns.push(Turn {
            kind: Kind::Ran(Ran {
                account: Account::new(at),
                closed: None,
            }),
        });
    }

    /// The account of the run still under way, to be fed as its events arrive.
    ///
    /// Handed out whole rather than wrapped in a method per kind of event,
    /// because every one of those methods would be a second name for something
    /// [`Account`] already does and a second place a run's line could come to be
    /// worded: a directory opening, an activity, a summarising pass, an outcome
    /// and the run's summary are all calls the caller already knows how to make.
    /// What they make them on is the run's own account, and the rows the reader
    /// sees are what those calls produced.
    ///
    /// `None` when the newest turn is not a run, or when the run it is has been
    /// closed — the same silence [`Thread::record`] keeps, and for the same
    /// reason: a line filed under a run that is over would contradict a line
    /// already on screen.
    pub fn run_mut(&mut self) -> Option<&mut Account> {
        self.turns.last_mut().and_then(Turn::live_run)
    }

    /// Close the run turn at `at`.
    ///
    /// The run is over — it wrote its documents, it was refused, it failed, or
    /// somebody stopped it — and this is the call that says so: the turn stops
    /// being the live one, so the composer is the reader's again, and the run's
    /// account stops counting up.
    ///
    /// It adds no line. How a run went is the account's to say, in the outcome
    /// lines and the summary a caller has already put there through
    /// [`Thread::run_mut`], and a sentence of the thread's own about a run would
    /// be the second wording this whole arrangement exists to avoid.
    ///
    /// Does nothing when the newest turn is not a live run.
    pub fn close_run(&mut self, at: Instant) {
        if self.run_mut().is_some() {
            self.freeze_last(at);
        }
    }

    /// Record what the live turn was seen doing at `at`.
    ///
    /// A tool becomes its name plus the one detail [`Activity`] chose to carry,
    /// thinking becomes the word `thinking`, writing the word `writing` — the
    /// account's wording, from the account's own function, because they are the
    /// same facts. A cost becomes no line at all: it is a fact about the turn
    /// rather than a thing the turn did, so it is added up and said once, at the
    /// end, on a line of its own.
    ///
    /// Does nothing when there is no live *typed* turn — before the first
    /// question, after the current one has been answered or ended, or while the
    /// newest turn is a run. A line cannot be filed under a turn that is already
    /// over without contradicting a line already on screen, and a chat's
    /// activity filed under a run nobody typed would put the model's work in the
    /// middle of a pact's account; dropping it is the honest way to fail. What a
    /// run reports goes to the run's own account, through
    /// [`Thread::run_mut`].
    pub fn record(&mut self, activity: &Activity, at: Instant) {
        let Some(said) = self.live_said() else {
            return;
        };

        match activity {
            Activity::Cost { usd } => *said.cost.get_or_insert(0.0) += usd,
            // One line per stretch, however many times the stream says the
            // stretch is still going: the line already there goes on ticking,
            // and its clock is the count of how long the model has been at it.
            Activity::Thinking => said.log.extend_or_open(THINKING, at),
            Activity::Writing => said.log.extend_or_open(WRITING, at),
            Activity::Tool { name, detail } => {
                said.log.push(tool_line(name, detail.as_ref()), at);
            }
        }
    }

    /// Land `answer` on the live turn at `at`, and close it.
    ///
    /// The answer is stored as it arrived, whole and unwrapped, and the turn
    /// stops moving: `at` is the instant its last work line freezes at.
    ///
    /// An answer with nothing in it is not an answer. A model that finished and
    /// said nothing ends the turn with [`Ending::NothingSaid`] instead, because
    /// a turn drawn as a question with nothing under it is indistinguishable
    /// from one still going, and a reader would sit and wait for it.
    ///
    /// Does nothing when there is no live typed turn, for [`Thread::record`]'s
    /// reason — a run does not answer anybody.
    pub fn answer(&mut self, answer: impl Into<String>, at: Instant) {
        let Some(said) = self.live_said() else {
            return;
        };

        let answer = answer.into();
        if answer.trim().is_empty() {
            said.word(&Ending::NothingSaid, at);
            return;
        }
        said.answer = Some(answer);
        said.log.freeze(at);
    }

    /// End the live turn at `at` with the one line `ending` makes.
    ///
    /// The cancel and every failure come through here, and every one of them is
    /// one line: whatever arrived before it stays exactly where it was, so a
    /// turn cancelled after two tool calls still shows those two tool calls.
    ///
    /// Does nothing when there is no live typed turn, when the newest one is
    /// over already — the first ending wins, and a failure reported twice is
    /// still one line — or when the newest turn is a run, whose endings are its
    /// account's [`Outcome`](crate::Outcome)s and not the thread's to word.
    pub fn end(&mut self, ending: &Ending, at: Instant) {
        if let Some(said) = self.live_said() {
            said.word(ending, at);
        }
    }

    /// The turns, in the order they were asked — the runs among them included,
    /// at the position each run started at.
    #[must_use]
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// Whether nothing has been asked and nothing has been run.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// The turn still going, or `None` when none is.
    ///
    /// The live turn is the last one, and only while it is un-frozen: a turn
    /// stops being live the moment it answers, ends, is run to a finish, or is
    /// overtaken by a newer one. What a caller does with it is decide whether the
    /// composer is muted — one turn at a time, whether that turn is a question
    /// somebody asked or a run they started — without keeping a second flag that
    /// could disagree with the thread.
    #[must_use]
    pub fn in_flight(&self) -> Option<&Turn> {
        self.turns.last().filter(|turn| !turn.is_closed())
    }

    /// Stop the newest turn moving as of `at`, whatever kind of turn it is.
    ///
    /// What every turn opening does to the one above it, and what closes a run:
    /// see [`Turn::freeze`], which is where the difference between freezing a
    /// question and freezing a run lives.
    fn freeze_last(&mut self, at: Instant) {
        if let Some(previous) = self.turns.last_mut() {
            previous.freeze(at);
        }
    }

    /// The live typed turn, which is the only turn a conversation's own events
    /// can be filed under.
    fn live_said(&mut self) -> Option<&mut Said> {
        self.turns.last_mut().and_then(Turn::live_said)
    }

    /// How many rows the whole thread draws as, before anything is wrapped.
    ///
    /// What a scroll offset is clamped against — at a width of nothing, which is
    /// every card's count until the frame says otherwise. The card is the one
    /// that knows the width, and it is the one that wraps.
    #[must_use]
    pub fn line_count(&self) -> usize {
        self.turns.iter().map(Turn::line_count).sum()
    }

    /// Every row of the thread, with clocks measured against `now`.
    ///
    /// `now` is the caller's: this reads no clock, so the same thread and the
    /// same instant give the same rows every time. Only the newest work line of
    /// a live turn depends on it.
    #[must_use]
    pub fn lines(&self, now: Instant) -> Vec<Line> {
        self.window(0, self.line_count(), now)
    }

    /// The `height` rows starting at `offset`, with clocks measured against
    /// `now`.
    ///
    /// Asking for more rows than there are, or starting past the end, gives back
    /// what is there rather than failing: a viewport is a request, not an
    /// assertion about the conversation's length.
    #[must_use]
    pub fn window(&self, offset: usize, height: usize, now: Instant) -> Vec<Line> {
        self.turns
            .iter()
            .flat_map(move |turn| turn.rows(now))
            .skip(offset)
            .take(height)
            .collect()
    }
}

/// What a turn's cost line says.
///
/// Worded so that no reader and no arithmetic can mistake it for a pact's. The
/// account says what a pass produced and what that cost — `wrote … — 2341
/// bytes, $0.21` — and adds those up into `pact finished — …`; this says whose
/// money it is and, out loud, that it belongs to no total. The panel shows both
/// kinds of row, so the sentence is the only thing keeping them apart.
fn spend(usd: f64) -> String {
    format!(
        "this turn cost {} — chat, never added to a pact's total",
        money(usd)
    )
}

/// `text` as the rows it is drawn in before any wrapping: its own lines, and one
/// empty row for a text with nothing in it.
///
/// Never empty, for [`wrapped`](crate::wrap)'s reason: a blank line is a
/// paragraph break, and a message or an answer that came to no rows at all would
/// be a turn with a hole in it.
fn broken(text: &str) -> impl Iterator<Item = &str> {
    let mut lines = text.lines().peekable();
    let empty = lines.peek().is_none().then_some("");
    empty.into_iter().chain(lines)
}

/// `text` as one line: runs of whitespace collapsed to single spaces.
///
/// A row is a row, and a stderr with a stack trace in it would otherwise be
/// several. Nothing is cut — the account refuses to truncate for the same
/// reason, since the width is a fact about a terminal and this is not.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use warlock_engine::AgentError;

    use super::{Ending, Thread, Turn, ending_for};
    use crate::account::{Account, Line, Outcome};
    use crate::claude::Activity;

    /// The instant `seconds` after `base`, so a whole conversation can be driven
    /// without anything ever reading a clock.
    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    /// The activity a turn reports for a tool call with one detail worth saying.
    fn tool(name: &str, detail: &str) -> Activity {
        Activity::Tool {
            name: name.to_owned(),
            detail: Some(detail.to_owned()),
        }
    }

    /// What the thread's rows say, as plain text, for the tests that care about
    /// the wording rather than the shape.
    fn said(thread: &Thread, now: Instant) -> Vec<String> {
        thread
            .lines(now)
            .into_iter()
            .map(|line| match line {
                // Only a run turn yields a directory heading, and it yields it
                // as the account wrote it: the path, on its own row.
                Line::Directory { path } => path.display().to_string(),
                Line::Clocked { clock, text } => format!("{clock} {text}"),
                Line::Summary { text } | Line::Text { text } | Line::Said { text } => text,
            })
            .collect()
    }

    /// Only the clocked rows: the work lines, which is where the promises about
    /// what never reaches the panel are kept.
    fn work(thread: &Thread, now: Instant) -> Vec<String> {
        thread
            .lines(now)
            .into_iter()
            .filter_map(|line| match line {
                Line::Clocked { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_conversation_nobody_has_started_draws_nothing() {
        let base = Instant::now();
        let thread = Thread::new();

        assert!(thread.is_empty());
        assert_eq!(thread.line_count(), 0);
        assert_eq!(thread.lines(at(base, 30)), Vec::new());
        assert!(thread.turns().is_empty());
        assert!(thread.in_flight().is_none());
    }

    #[test]
    fn a_whole_turn_reads_as_question_then_work_then_answer_then_money() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("what does the engine do?", base);
        thread.record(&Activity::Thinking, at(base, 2));
        thread.record(&tool("grep", "pact_directory"), at(base, 8));
        thread.record(
            &tool("read", "crates/warlock-engine/src/pact.rs"),
            at(base, 11),
        );
        thread.record(&Activity::Writing, at(base, 30));
        thread.record(&Activity::Cost { usd: 0.02 }, at(base, 31));
        thread.answer("It pacts a tree.", at(base, 42));

        assert_eq!(
            said(&thread, at(base, 90)),
            vec![
                "what does the engine do?".to_owned(),
                // Each clock is the instant the next line arrived, measured
                // from the question rather than from the session.
                "0:08 thinking".to_owned(),
                "0:11 grep pact_directory".to_owned(),
                "0:30 read crates/warlock-engine/src/pact.rs".to_owned(),
                "0:42 writing".to_owned(),
                "It pacts a tree.".to_owned(),
                "this turn cost $0.02 — chat, never added to a pact's total".to_owned(),
            ],
        );
        assert_eq!(thread.line_count(), 7);
        assert_eq!(thread.lines(at(base, 90)).len(), thread.line_count());
    }

    #[test]
    fn a_turn_that_has_heard_nothing_ticks_waiting_until_the_model_speaks() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("why?", base);

        // On screen from the moment the question is submitted: the clock is
        // already moving with nothing recorded, which is what says the silence
        // is a model and not a hang.
        assert_eq!(thread.line_count(), 2);
        assert_eq!(
            said(&thread, at(base, 12)),
            vec!["why?".to_owned(), "0:12 waiting".to_owned()],
        );

        // The first real line takes its place, on the same clock.
        thread.record(&Activity::Thinking, at(base, 58));
        assert_eq!(
            said(&thread, at(base, 60)),
            vec!["why?".to_owned(), "1:00 thinking".to_owned()],
        );
    }

    #[test]
    fn the_newest_work_line_ticks_with_now_and_freezes_when_the_answer_lands() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("go on", base);
        thread.record(&Activity::Thinking, at(base, 1));

        assert_eq!(work(&thread, at(base, 5)), vec!["thinking".to_owned()]);
        assert_eq!(
            said(&thread, at(base, 5))[1],
            "0:05 thinking",
            "a live turn's newest line counts up with the caller's now",
        );
        assert_eq!(said(&thread, at(base, 9))[1], "0:09 thinking");

        thread.answer("done", at(base, 20));

        // Frozen where the answer landed, however long the session runs on.
        assert_eq!(said(&thread, at(base, 600))[1], "0:20 thinking");
        assert!(thread.in_flight().is_none());
    }

    #[test]
    fn every_turn_starts_its_clock_again_at_zero() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("first", at(base, 10));
        thread.record(&Activity::Thinking, at(base, 12));
        thread.ask("second", at(base, 70));
        thread.record(&Activity::Thinking, at(base, 71));

        // The first turn's line froze when the second question was asked, one
        // minute in; the second's counts from its own question.
        assert_eq!(
            said(&thread, at(base, 75)),
            vec![
                "first".to_owned(),
                "1:00 thinking".to_owned(),
                "second".to_owned(),
                "0:05 thinking".to_owned(),
            ],
        );
        assert_eq!(thread.turns().len(), 2);
        assert_eq!(thread.turns()[0].message(), Some("first"));
        assert_eq!(thread.in_flight().map(Turn::started), Some(at(base, 70)));
    }

    #[test]
    fn a_stretch_of_thinking_is_one_line_however_often_it_is_reported() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("hello", base);
        for second in [2, 5, 9, 40] {
            thread.record(&Activity::Thinking, at(base, second));
        }

        assert_eq!(
            said(&thread, at(base, 55)),
            vec!["hello".to_owned(), "0:55 thinking".to_owned()],
            "one line, counting from when thinking started",
        );

        // Something else happening ends the stretch, and thinking after it is a
        // new one — the two are separated by what came between them.
        thread.record(&tool("read", "src/lib.rs"), at(base, 60));
        thread.record(&Activity::Thinking, at(base, 61));
        thread.record(&Activity::Thinking, at(base, 65));

        assert_eq!(
            work(&thread, at(base, 70)),
            vec![
                "thinking".to_owned(),
                "read src/lib.rs".to_owned(),
                "thinking".to_owned(),
            ],
        );
    }

    #[test]
    fn a_tool_without_a_detail_is_its_bare_name() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("look around", base);
        thread.record(
            &Activity::Tool {
                name: "glob".to_owned(),
                detail: None,
            },
            at(base, 3),
        );

        assert_eq!(work(&thread, at(base, 4)), vec!["glob".to_owned()]);
    }

    #[test]
    fn a_cost_is_arithmetic_and_never_a_work_line() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("what is this?", base);
        thread.record(&Activity::Cost { usd: 0.01 }, at(base, 1));
        thread.record(&Activity::Cost { usd: 0.014 }, at(base, 2));

        // Every report is added up, and none of them put a row under the
        // question: the work lines are still just the placeholder.
        assert_eq!(work(&thread, at(base, 3)), vec!["waiting".to_owned()]);
        assert_eq!(thread.turns()[0].cost(), Some(0.024));

        thread.answer("this.", at(base, 4));
        assert_eq!(
            said(&thread, at(base, 5)).last().map(String::as_str),
            Some("this turn cost $0.02 — chat, never added to a pact's total"),
        );
    }

    #[test]
    fn a_turn_that_never_reported_a_cost_draws_no_cost_line() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("cheap?", base);
        thread.answer("free.", at(base, 2));

        assert_eq!(thread.turns()[0].cost(), None);
        assert_eq!(
            said(&thread, at(base, 3)),
            vec![
                "cheap?".to_owned(),
                "0:02 waiting".to_owned(),
                "free.".to_owned(),
            ],
        );
    }

    #[test]
    fn a_turns_money_cannot_be_read_as_a_pacts() {
        let base = Instant::now();

        let mut thread = Thread::new();
        thread.ask("how much?", base);
        thread.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
        thread.answer("that much.", at(base, 2));

        let mut account = Account::new(base);
        account.open_section("crates/engine", base);
        account.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
        account.close_section(
            &Outcome::Wrote {
                document: "crates/engine/WARLOCK.md".into(),
                bytes: 2341,
            },
            at(base, 2),
        );
        account.finish(at(base, 3));

        let turn_money = said(&thread, at(base, 4));
        let turn_money = turn_money
            .last()
            .expect("a turn that reported a cost has a cost line");
        let pact_money: Vec<String> = account
            .lines(at(base, 4))
            .into_iter()
            .filter_map(|line| match line {
                Line::Clocked { text, .. } | Line::Summary { text } => {
                    text.contains("$0.21").then_some(text)
                }
                _ => None,
            })
            .collect();

        // The account says what a pass produced and totals those; the turn says
        // whose money it is and, out loud, that it belongs to no total. Neither
        // sentence can be mistaken for the other, and nothing on screen invites
        // adding them up.
        assert_eq!(
            pact_money,
            vec![
                "wrote crates/engine/WARLOCK.md — 2341 bytes, $0.21".to_owned(),
                "pact finished — 1 directory, 0:03, $0.21".to_owned(),
            ],
        );
        assert!(turn_money.contains("this turn cost"));
        assert!(turn_money.contains("never added to a pact's total"));
        assert!(!turn_money.contains("wrote"));
        assert!(!turn_money.contains("pact finished"));
        assert!(!pact_money.iter().any(|line| line.contains("this turn")));
    }

    #[test]
    fn no_tool_result_and_no_model_prose_ever_reaches_a_work_line() {
        let base = Instant::now();
        let mut thread = Thread::new();

        // Everything a turn can be told, including a tool whose detail is the
        // only thing it carries: none of it is a result, and none of it is
        // reasoning. The answer is the one piece of prose, and it is a text row
        // rather than a clocked one.
        thread.ask("summarise the engine", base);
        thread.record(&Activity::Thinking, at(base, 1));
        thread.record(&tool("grep", "fn pact_directory"), at(base, 4));
        thread.record(&tool("read", "src/pact.rs"), at(base, 6));
        thread.record(&Activity::Writing, at(base, 9));
        thread.answer(
            "The engine walks a tree and writes a document per directory.",
            at(base, 20),
        );

        assert_eq!(
            work(&thread, at(base, 21)),
            vec![
                "thinking".to_owned(),
                "grep fn pact_directory".to_owned(),
                "read src/pact.rs".to_owned(),
                "writing".to_owned(),
            ],
        );
        for line in work(&thread, at(base, 21)) {
            assert!(
                !line.contains("engine walks"),
                "the answer's prose never appears among the work lines: {line}",
            );
        }
    }

    #[test]
    fn an_answer_is_kept_whole_and_unwrapped() {
        let base = Instant::now();
        let mut thread = Thread::new();

        let long = "a ".repeat(120);
        thread.ask("say something long", base);
        thread.answer(format!("{long}\n\nand a second paragraph"), at(base, 5));

        // Stored exactly as it arrived: wrapping is the frame's business, and a
        // terminal made narrower should re-flow this rather than ask again.
        let answer = thread.turns()[0]
            .answer()
            .expect("the turn answered")
            .to_owned();
        assert_eq!(answer, format!("{long}\n\nand a second paragraph"));

        // Its own newlines are rows, though, because they are the answer's.
        assert_eq!(
            thread
                .lines(at(base, 6))
                .into_iter()
                .filter(|line| matches!(line, Line::Text { .. }))
                .count(),
            3,
        );
    }

    #[test]
    fn a_cancelled_turn_keeps_what_arrived_and_gains_one_line() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("read the tree", base);
        thread.record(&tool("read", "a.rs"), at(base, 2));
        thread.record(&tool("read", "b.rs"), at(base, 4));
        thread.end(&Ending::Cancelled, at(base, 6));

        assert_eq!(
            said(&thread, at(base, 60)),
            vec![
                "read the tree".to_owned(),
                "0:04 read a.rs".to_owned(),
                "0:06 read b.rs".to_owned(),
                "0:06 the turn was cancelled".to_owned(),
            ],
        );
        assert_eq!(thread.turns()[0].ending(), Some(&Ending::Cancelled));
        assert!(thread.turns()[0].answer().is_none());
        assert!(thread.in_flight().is_none());
    }

    #[test]
    fn every_failure_ends_the_turn_in_exactly_one_line() {
        let base = Instant::now();
        let endings = [
            (
                Ending::NoModel {
                    program: "claude".to_owned(),
                },
                "the turn could not run — no claude on PATH",
            ),
            (
                Ending::Failed {
                    code: Some(2),
                    stderr: "not logged in\n  try `claude login`".to_owned(),
                },
                "the turn failed — exit status 2: not logged in try `claude login`",
            ),
            (
                Ending::Failed {
                    code: None,
                    stderr: String::new(),
                },
                "the turn failed — stopped before it exited",
            ),
            (
                Ending::TimedOut {
                    after: Duration::from_secs(90),
                },
                "the turn ran past 90s and was stopped",
            ),
            (Ending::NothingSaid, "the turn ended with nothing to say"),
            (
                Ending::Broke {
                    reason: "broken pipe".to_owned(),
                },
                "the turn could not run — broken pipe",
            ),
        ];

        for (ending, wording) in endings {
            let mut thread = Thread::new();
            thread.ask("go", base);
            thread.end(&ending, at(base, 3));

            assert_eq!(
                said(&thread, at(base, 30)),
                vec!["go".to_owned(), format!("0:03 {wording}")],
                "one question, one line, and nothing else",
            );
            assert_eq!(thread.turns()[0].ending(), Some(&ending));
            assert!(thread.in_flight().is_none(), "the session is free again");
        }
    }

    #[test]
    fn a_model_that_finished_with_nothing_to_say_ends_rather_than_answers() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("well?", base);
        thread.answer("   \n  ", at(base, 4));

        // A blank turn on screen is indistinguishable from one still going, so
        // an empty answer is a failure with a line of its own.
        assert!(thread.turns()[0].answer().is_none());
        assert_eq!(thread.turns()[0].ending(), Some(&Ending::NothingSaid));
        assert_eq!(
            said(&thread, at(base, 30)),
            vec![
                "well?".to_owned(),
                "0:04 the turn ended with nothing to say".to_owned()
            ],
        );
    }

    #[test]
    fn the_first_ending_wins_and_nothing_lands_on_a_closed_turn() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("stop", base);
        thread.end(&Ending::Cancelled, at(base, 2));

        // Everything after the cancel is dropped: a second ending, an answer
        // that was already on its way, and any activity still in the channel.
        thread.end(&Ending::NothingSaid, at(base, 3));
        thread.answer("too late", at(base, 4));
        thread.record(&Activity::Thinking, at(base, 5));
        thread.record(&Activity::Cost { usd: 9.99 }, at(base, 6));

        assert_eq!(
            said(&thread, at(base, 30)),
            vec!["stop".to_owned(), "0:02 the turn was cancelled".to_owned()],
        );
        assert_eq!(thread.turns()[0].ending(), Some(&Ending::Cancelled));
        assert_eq!(thread.turns()[0].cost(), None);
    }

    #[test]
    fn nothing_can_be_filed_before_the_first_question() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.record(&Activity::Thinking, base);
        thread.answer("out of nowhere", at(base, 1));
        thread.end(&Ending::Cancelled, at(base, 2));

        assert!(thread.is_empty());
        assert_eq!(thread.lines(at(base, 3)), Vec::new());
    }

    #[test]
    fn a_cancelled_turn_still_says_what_it_spent() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("expensive?", base);
        thread.record(&Activity::Cost { usd: 0.4 }, at(base, 1));
        thread.end(&Ending::Cancelled, at(base, 2));

        assert_eq!(
            said(&thread, at(base, 30)),
            vec![
                "expensive?".to_owned(),
                "0:02 the turn was cancelled".to_owned(),
                "this turn cost $0.40 — chat, never added to a pact's total".to_owned(),
            ],
        );
    }

    #[test]
    fn a_message_with_newlines_in_it_is_the_rows_it_was_typed_as() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("first line\nsecond line", base);

        assert_eq!(
            thread.lines(at(base, 1))[..2],
            [
                Line::Said {
                    text: "first line".to_owned()
                },
                Line::Said {
                    text: "second line".to_owned()
                },
            ],
        );
        assert_eq!(thread.line_count(), 3);
    }

    #[test]
    fn a_window_gives_back_what_is_there_rather_than_what_was_asked_for() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("one", base);
        thread.answer("uno", at(base, 1));
        thread.ask("two", at(base, 2));
        thread.answer("dos", at(base, 3));

        assert_eq!(thread.line_count(), 6);
        assert_eq!(
            thread.window(3, 2, at(base, 4)),
            vec![
                Line::Said {
                    text: "two".to_owned()
                },
                Line::Clocked {
                    clock: "0:01".to_owned(),
                    text: "waiting".to_owned()
                },
            ],
        );
        assert_eq!(thread.window(0, 99, at(base, 4)).len(), 6);
        assert_eq!(thread.window(99, 4, at(base, 4)), Vec::new());
    }

    /// A whole run, fed to whichever account it is handed: two directories, a
    /// pass over a big file, a document written, a refusal, and the summary.
    ///
    /// One script, so the account card's account and the thread's run turn are
    /// driven by the same calls in the same order — which is the only honest way
    /// to ask whether they come out saying the same thing.
    fn drive_run(account: &mut Account, base: Instant) {
        account.open_section("crates/engine", base);
        account.record(&Activity::Thinking, at(base, 2));
        account.record_summarising("crates/engine/Cargo.lock", 1, 2, at(base, 10));
        account.record(&tool("Read", "src/lib.rs"), at(base, 40));
        account.record(&Activity::Writing, at(base, 50));
        account.record(&Activity::Cost { usd: 0.21 }, at(base, 55));
        account.open_section("crates/tui", at(base, 60));
        account.record(&Activity::Thinking, at(base, 61));
        account.close_open_sections(at(base, 90), |section| {
            if section.directory() == Path::new("crates/engine") {
                Outcome::Wrote {
                    document: "crates/engine/WARLOCK.md".into(),
                    bytes: 2_341,
                }
            } else {
                Outcome::Refused {
                    reason: "the model returned an empty document".to_owned(),
                }
            }
        });
        account.finish(at(base, 90));
    }

    #[test]
    fn a_run_turn_draws_exactly_what_the_account_card_draws() {
        let base = Instant::now();

        // The same run, twice: once as the card holds it, once as the thread
        // does. The thread's is an account too — that is the whole design — so
        // there is no second wording of a directory heading, a tool call, a
        // summarising pass, an outcome or the summary to drift from this one.
        let mut card = Account::new(base);
        drive_run(&mut card, base);

        let mut thread = Thread::new();
        thread.open_run(base);
        drive_run(thread.run_mut().expect("the run is the live turn"), base);
        thread.close_run(at(base, 90));

        let now = at(base, 120);
        assert_eq!(thread.lines(now), card.lines(now));
        assert_eq!(thread.line_count(), card.line_count());
        assert_eq!(
            said(&thread, now),
            vec![
                "crates/engine".to_owned(),
                "0:10 thinking".to_owned(),
                "0:40 summarising crates/engine/Cargo.lock (1/2)".to_owned(),
                "0:50 Read src/lib.rs".to_owned(),
                "1:00 writing".to_owned(),
                "1:00 wrote crates/engine/WARLOCK.md — 2341 bytes, $0.21".to_owned(),
                "crates/tui".to_owned(),
                "0:30 thinking".to_owned(),
                "0:30 refused — the model returned an empty document".to_owned(),
                "pact finished — 2 directories, 1:30, $0.21 (incomplete: 1 pass reported no cost)"
                    .to_owned(),
            ],
        );

        // One line per action, and not one word of prose or of a tool's result
        // among them: the run says what it did, exactly as it does on its card.
        for line in work(&thread, now) {
            assert!(
                !line.contains("this turn"),
                "a chat's wording reached a run's line: {line}",
            );
        }
    }

    #[test]
    fn a_run_takes_its_turn_where_it_started_among_the_questions() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.ask("what is in the engine?", base);
        thread.record(&Activity::Thinking, at(base, 1));
        thread.answer("A pact.", at(base, 5));

        // A pact started while the conversation is on screen goes into the
        // conversation, in order, rather than swapping a card away or opening a
        // second history somewhere else.
        thread.open_run(at(base, 10));
        let run = thread.run_mut().expect("the run is the live turn");
        run.open_section("crates/engine", at(base, 10));
        run.record(&Activity::Writing, at(base, 12));
        run.finish(at(base, 40));
        thread.close_run(at(base, 40));

        thread.ask("and how long did that take?", at(base, 50));
        thread.answer("Half a minute.", at(base, 55));

        assert_eq!(
            said(&thread, at(base, 90)),
            vec![
                "what is in the engine?".to_owned(),
                "0:05 thinking".to_owned(),
                "A pact.".to_owned(),
                // The run's own clock starts again at zero, as every turn's
                // does, and counts from the key press rather than the session.
                "crates/engine".to_owned(),
                "0:30 writing".to_owned(),
                "pact finished — 1 directory, 0:30, $0.00 (incomplete: 1 pass reported no cost)"
                    .to_owned(),
                "and how long did that take?".to_owned(),
                "0:05 waiting".to_owned(),
                "Half a minute.".to_owned(),
            ],
        );

        // Three turns, in the order they happened, and the middle one is the
        // one nobody typed.
        assert_eq!(thread.turns().len(), 3);
        assert_eq!(thread.turns()[0].message(), Some("what is in the engine?"));
        assert_eq!(thread.turns()[1].message(), None);
        assert_eq!(thread.turns()[1].started(), at(base, 10));
        assert!(thread.turns()[1].account().is_some());
        assert!(thread.turns()[0].account().is_none());
        assert_eq!(
            thread.turns()[2].message(),
            Some("and how long did that take?"),
        );

        // Counted as well as drawn, so the panel can scroll to the end of it.
        assert_eq!(thread.line_count(), 9);
        assert_eq!(thread.lines(at(base, 90)).len(), thread.line_count());
        assert_eq!(
            thread.window(3, 3, at(base, 90)),
            vec![
                Line::Directory {
                    path: "crates/engine".into(),
                },
                Line::Clocked {
                    clock: "0:30".to_owned(),
                    text: "writing".to_owned(),
                },
                Line::Summary {
                    text: "pact finished — 1 directory, 0:30, $0.00 (incomplete: 1 pass reported \
                           no cost)"
                        .to_owned(),
                },
            ],
        );
    }

    #[test]
    fn a_run_that_ends_badly_words_its_own_turn_and_leaves_the_ones_above_it_alone() {
        let base = Instant::now();

        for (outcome, wording) in [
            (Outcome::Cancelled, "0:20 cancelled — $0.03 spent"),
            (
                Outcome::Refused {
                    reason: "the model returned an empty document".to_owned(),
                },
                "0:20 refused — the model returned an empty document",
            ),
            // A pass that failed outright is a refusal with the failure as its
            // reason — see `pacting`, which words every way a pass can not
            // produce a document this way — so it lands here like the rest.
            (
                Outcome::Refused {
                    reason: "claude exited with status 1".to_owned(),
                },
                "0:20 refused — claude exited with status 1",
            ),
        ] {
            let mut thread = Thread::new();
            thread.ask("what is in the engine?", base);
            thread.answer("A pact.", at(base, 5));
            let before = said(&thread, at(base, 900));

            thread.open_run(at(base, 10));
            let run = thread.run_mut().expect("the run is the live turn");
            run.open_section("crates/engine", at(base, 10));
            run.record(&Activity::Cost { usd: 0.03 }, at(base, 11));
            run.close_section(&outcome, at(base, 30));
            run.finish(at(base, 30));
            thread.close_run(at(base, 30));

            let after = said(&thread, at(base, 900));
            assert_eq!(
                after[..before.len()],
                before[..],
                "the turns above the run are exactly as they were",
            );
            assert_eq!(after[before.len() + 1], wording);

            // The outcome is the account's, in the account's words: nothing
            // here ends a run with `the turn was cancelled`.
            assert!(!after.iter().any(|line| line.contains("the turn")));
            assert_eq!(thread.turns()[1].ending(), None);
            assert!(thread.in_flight().is_none(), "the session is free again");
        }
    }

    #[test]
    fn a_runs_money_and_a_turns_money_are_never_each_others_words() {
        let base = Instant::now();
        let mut thread = Thread::new();

        // The same twenty-one cents, spent by a question and by a pact, in one
        // conversation: the two lines have to be unmistakable for each other,
        // because the panel now draws them one under the other.
        thread.ask("how much?", base);
        thread.record(&Activity::Cost { usd: 0.21 }, at(base, 1));
        thread.answer("that much.", at(base, 2));

        thread.open_run(at(base, 10));
        let run = thread.run_mut().expect("the run is the live turn");
        run.open_section("crates/engine", at(base, 10));
        run.record(&Activity::Cost { usd: 0.21 }, at(base, 11));
        run.close_section(
            &Outcome::Wrote {
                document: "crates/engine/WARLOCK.md".into(),
                bytes: 2_341,
            },
            at(base, 30),
        );
        run.finish(at(base, 30));
        thread.close_run(at(base, 30));

        let money: Vec<String> = said(&thread, at(base, 60))
            .into_iter()
            .filter(|line| line.contains("$0.21"))
            .collect();
        assert_eq!(
            money,
            vec![
                "this turn cost $0.21 — chat, never added to a pact's total".to_owned(),
                "0:20 wrote crates/engine/WARLOCK.md — 2341 bytes, $0.21".to_owned(),
                "pact finished — 1 directory, 0:20, $0.21".to_owned(),
            ],
        );

        // And no arithmetic anywhere has added them: the chat's money is the
        // turn's own and the run's total is the run's own, both $0.21 rather
        // than one $0.42 between them.
        assert_eq!(thread.turns()[0].cost(), Some(0.21));
        assert_eq!(
            thread.turns()[1].cost(),
            None,
            "a run's money is its account's, never a turn's number",
        );
        assert_eq!(
            thread.turns()[1]
                .account()
                .and_then(|account| account.sections()[0].cost()),
            Some(0.21),
        );
    }

    #[test]
    fn nothing_a_conversation_says_lands_on_a_turn_nobody_typed() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.open_run(base);
        let run = thread.run_mut().expect("the run is the live turn");
        run.open_section("crates/engine", base);
        run.record(&Activity::Writing, at(base, 2));

        let during = said(&thread, at(base, 20));

        // A chat's activity, answer or ending has nowhere to go while the run
        // is the newest turn: a model's prose in the middle of a pact's account
        // is exactly what the account card refuses, and this card is that card.
        thread.record(&Activity::Thinking, at(base, 21));
        thread.record(&Activity::Cost { usd: 9.99 }, at(base, 22));
        thread.answer("out of nowhere", at(base, 23));
        thread.end(&Ending::Cancelled, at(base, 24));

        assert_eq!(said(&thread, at(base, 20)), during);
        assert_eq!(thread.turns().len(), 1);
        assert_eq!(thread.turns()[0].answer(), None);
        assert_eq!(thread.turns()[0].ending(), None);
        assert!(
            thread.in_flight().is_some(),
            "and none of it ended the run either",
        );
    }

    #[test]
    fn a_run_is_the_live_turn_until_it_is_closed() {
        let base = Instant::now();
        let mut thread = Thread::new();

        assert!(thread.run_mut().is_none(), "no run, nothing to feed");

        thread.open_run(base);
        assert!(thread.in_flight().is_some(), "the composer is muted");
        assert_eq!(thread.in_flight().map(Turn::started), Some(base));

        let run = thread.run_mut().expect("the run is the live turn");
        run.open_section("crates/engine", base);
        run.record(&Activity::Writing, at(base, 2));
        assert_eq!(said(&thread, at(base, 30))[1], "0:30 writing");

        // Closing a run stops its clocks with it, however the run went and
        // whether or not its account was finished: a line still counting up
        // under a conversation that has moved on is a run that looks alive.
        thread.close_run(at(base, 40));
        assert!(thread.in_flight().is_none(), "the composer is the reader's");
        assert!(thread.run_mut().is_none(), "and nothing more is filed");
        assert_eq!(said(&thread, at(base, 900))[1], "0:40 writing");

        // A question after it opens a turn of its own, under the run.
        thread.ask("what did that write?", at(base, 50));
        thread.answer("A document.", at(base, 55));
        assert_eq!(
            said(&thread, at(base, 900)),
            vec![
                "crates/engine".to_owned(),
                "0:40 writing".to_owned(),
                "what did that write?".to_owned(),
                "0:05 waiting".to_owned(),
                "A document.".to_owned(),
            ],
        );
    }

    #[test]
    fn a_run_left_open_stops_when_the_next_turn_starts() {
        let base = Instant::now();
        let mut thread = Thread::new();

        thread.open_run(base);
        let run = thread.run_mut().expect("the run is the live turn");
        run.open_section("crates/engine", base);
        run.record(&Activity::Writing, at(base, 2));

        // Nobody closed the run, and a question was asked anyway: the run
        // freezes where the question landed, exactly as a turn overtaken by a
        // newer one does.
        thread.ask("what is happening?", at(base, 20));

        assert!(thread.turns()[0].is_closed());
        assert_eq!(said(&thread, at(base, 900))[1], "0:20 writing");
        assert_eq!(thread.in_flight().map(Turn::started), Some(at(base, 20)));
    }

    #[test]
    fn the_seams_failures_become_the_panels_endings() {
        assert_eq!(
            ending_for(&AgentError::NotFound {
                program: "claude".to_owned()
            }),
            Ending::NoModel {
                program: "claude".to_owned()
            },
        );
        assert_eq!(
            ending_for(&AgentError::Failed {
                code: Some(1),
                stderr: "boom".to_owned(),
            }),
            Ending::Failed {
                code: Some(1),
                stderr: "boom".to_owned(),
            },
        );
        assert_eq!(
            ending_for(&AgentError::TimedOut {
                after: Duration::from_secs(30)
            }),
            Ending::TimedOut {
                after: Duration::from_secs(30)
            },
        );
        assert_eq!(ending_for(&AgentError::EmptyOutput), Ending::NothingSaid);

        // Anything the seam grows later arrives as whatever it says about
        // itself, rather than as a panic or a silent nothing.
        let broken = AgentError::Io {
            source: std::io::Error::other("broken pipe"),
        };
        assert_eq!(
            ending_for(&broken),
            Ending::Broke {
                reason: broken.to_string()
            },
        );
    }
}
