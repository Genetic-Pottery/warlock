//! `/pull`, from the command word to the project it would cut, read back off
//! the board.
//!
//! Everything a reader would be asked about a pull — the project's name, the
//! status it is sitting in, how many slices it has — is on the wire.
//! `.warlock/filed.toml` keeps an id and a URL and nothing else, so there is no
//! question to put up until a request has been answered: the fetch comes first
//! and what it has to say is a line on the thread.
//!
//! [`Pulls`] is [`crate::pushing::Pushes`]'s shape — the [`Opens`] seam held for
//! the life of the process, the home resolved once, an [`Option`] that is its
//! own say-no to a second run, a channel drained at the bottom of the loop —
//! with one difference: the board is resolved on the worker rather than on the
//! event loop's thread. A push resolves it first because a dialog is about to
//! name the team it found; a pull has nothing to put up yet, so the sequence
//! from the sigils to the slice count is one thing that fails in one place, and
//! the manifest crosses with the rest of the work as it does for a pact.
//!
//! The key is read on exactly one line in this file — the one that builds the
//! client — and what the worker holds either side of it is a manifest, two
//! paths and a spelling. No target outlives that line, so there is nowhere in
//! [`Pulls`], in [`Work`], in the channel's message or in a `Debug` rendering
//! for a key value to be.
//!
//! Nothing here writes. No mutation is sent, no status is moved and no record
//! is appended, which is what makes every refusal below cost nothing but the
//! line it is said in — and every one of those lines is the sentence
//! [`crate::error::Error`] already words for `warlock pull`, so a reader who
//! has met one at a shell meets the same words in the panel.
//!
//! The run a Yes starts is one [`Drafting`] session per uncut slice, in cut
//! order and one at a time, and every turn of it happens on a worker: a
//! [`Drafting`] is stateful across turns and each turn blocks on a subprocess,
//! so the session is *moved* onto the thread and handed back through the
//! channel with its reply. That is what keeps the panel drawing through a run
//! that takes minutes, with no second long-lived session and no runtime to
//! carry it. A turn driven from [`Pulls::keep_up`] instead would freeze the
//! frame for as long as `claude` took to think.
//!
//! A slice that asks something is a *state of that run* — [`Stage::Waiting`] —
//! and never a [`Mode`](warlock_tui::Mode): a mode is the panel's one chat
//! session said at a different level, and this is a second session under its own
//! prompt with a question of its own out. So the relay is held here, beside the
//! session that asked, and the loop routes the field to it for exactly as long
//! as it lives. Nothing in this module knows what a composer is: it says what is
//! being answered ([`Pulls::answering`]), it takes an answer
//! ([`Pulls::answered`]) and it hands back warlock's attempt for the field
//! ([`Pulls::keep_up`]), and which value holds the draft is the loop's business.
//!
//! That attempt runs on a worker of its own, off a second conversation at the
//! register the brief was written in — one turn, read-only, and nothing of the
//! reader's talk in it. It is offered as an ordinary draft and never sent: what
//! reaches the slice is whatever the field holds when somebody presses Enter, so
//! a proposal that failed, that settled nothing, or that somebody cleared and
//! typed over costs the question nothing at all.

use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

use warlock_engine::{Manifest, agent, filed_path, resolve_filing};
use warlock_tui::{
    Answer, App, Cancel, ChatAgent, Converses, Drafted, Drafting, NOTHING_SETTLES_IT, PullConfirm,
    Replied, Slice, fetch_project, propose_answer, scope_block_in,
};

use crate::cut::listed;
use crate::error::{Error, one_line};
use crate::pacting::CancelGuard;
use crate::pull::{counted, heading, is_planned, named};
use crate::push::records;
use crate::pushing::{Linear, Opens};
use crate::standing::Standing;

// Said to a `/pull` typed with one already running, and it is the whole of that
// refusal: no board is resolved, no record is read and no request is made. Two
// pulls at once would be two sets of drafting sessions spending on one
// conversation's behalf, which is why the one-at-a-time rule is worth a line of
// its own rather than a queue.
pub(crate) const ALREADY_PULLING: &str = "a pull is already running; this one read nothing";

// The worker sends on every path it takes, so a channel that closes with
// nothing on it is a panic. The hook has already printed it; what is left to
// say is that nothing was read — which is the honest answer for a sequence that
// only reads, and the reason this sentence promises more than a lost push's
// does.
const PULL_LOST: &str = "the pull stopped without saying how it went; nothing was read or changed";

// The same for a slice's worker, which also sends on every path it takes. One
// slice is one session and one thread, so a channel that closed with nothing on
// it took that slice down and nothing else — which is why this is a line about
// the slice and the run carries on to the next.
//
// A proposal's worker is worded the same way for the same reason, and costs even
// less: the question is still up and the field is still somebody's to type into.
const SLICE_LOST: &str = "it stopped without saying how it went";

/// What a pull has to say for itself once the board has answered: one line, or
/// the facts a line is worded from.
///
/// A `String` for the failure, because the failure is worded on the worker
/// where it happens — out of `error.rs`, flattened — and a line is what the
/// thread takes.
type Landing = Result<Fetched, String>;

/// The project a `/pull` found, gated and parsed: what is on the board, what
/// state it is in, and how much of it is left to cut.
///
/// Held as the facts rather than as the line they are said in because they are
/// what a reader is asked to confirm next, and none of them can be read again
/// without a second request.
///
/// The board is in here twice over — the team the project sits on and the
/// *name* the key that read it is held under — because the dialog names both
/// and the worker is the only thing that resolved them. The key value is not
/// among them and cannot be: it lives on one line of this module (see the
/// module's note), and there is nowhere in this value, in the channel's message
/// or in a `Debug` rendering for its bytes to sit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fetched {
    project: String,
    // The board's own spelling and not `Planned`, because that is what somebody
    // sent to look will find written on the project: the gate folds case and
    // trims, so the two need not match.
    status: String,
    slices: usize,
    team: String,
    key: String,
    ready: Ready,
}

/// What a Yes would run: the brief the slices were cut out of, and the slices
/// no cut record claims, in the order a run walks them.
///
/// Carried from the fetch rather than read again when the question is answered,
/// because there is no second reading: the project's description came off the
/// one request, and asking the board again between the question and the answer
/// would be a second project to disagree with the one the reader confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ready {
    brief: String,
    slices: Vec<Slice>,
}

impl Fetched {
    fn line(&self) -> String {
        format!(
            "`{}` is `{}` — {}, {} still to cut",
            self.project,
            self.status,
            counted(self.slices),
            self.ready.slices.len()
        )
    }

    // The question these facts are asked as. The count it names is the one the
    // line above names first — how many slices the project has — because that
    // is what the reader is being asked about the project, and how much of it
    // is left is the line's to say.
    fn asking(&self) -> PullConfirm {
        PullConfirm::open(
            &self.project,
            &self.status,
            self.slices,
            &self.team,
            &self.key,
        )
    }
}

/// The pull a session is doing, if it is doing one, and where its client comes
/// from.
///
/// The home is here for [`crate::pushing::Pushes`]'s reason: it cannot move
/// under a running warlock, and a second reading per keystroke would be a second
/// answer. It is also what keeps every test in this crate off the developer's
/// own — a `Pulls` is built with the home it is to use, so nothing below this
/// line asks the environment.
#[derive(Debug)]
pub(crate) struct Pulls<O: Opens, A: Converses> {
    open: O,
    home: Option<PathBuf>,
    // The conversation every slice's session is opened off, built once for the
    // session and cheap to build: an agent is a command line and a timeout, so
    // no `claude` exists until a confirmed question asks a slice for drafts.
    // `warlock pull` builds its one the same way and for the same reason.
    agent: A,
    // And the conversation warlock's attempt at a question is asked in, which is
    // a different one: not the slice's own session, whose next turn is the
    // answer, and not the panel's chat, which has heard the reader's talk and
    // none of the brief. See [`ChatAgent::proposing`].
    proposer: A,
    fetching: Option<Fetching>,
    // The question between the fetch and the run, held here rather than beside
    // the session's other windows because it is a state of the pull and not of
    // the app: what it is asked about came off the wire on the round it went
    // up, and nothing else in the panel can answer it.
    confirm: PullConfirm,
    // The material the question above is asked about, parked beside it: the
    // dialog names counts and a board, and a Yes needs the slices themselves.
    // Taken by the Yes and dropped by the No, so it lives exactly as long as the
    // question does.
    ready: Option<Ready>,
    // The run, which is its own say-no to a second `/pull` for as long as it
    // lasts: a slice in flight is a session spending on this conversation's
    // behalf, and two sets of them would be two runs cutting one project.
    slicing: Option<Slicing<A>>,
}

// The channel, and the handle that is the whole of how a pull is stopped.
//
// Dropping the session drops this, which cancels: no exit path has to remember
// to stop a pull, exactly as none has to remember to stop a pact. What a cancel
// can reach is either side of the request rather than the request itself —
// there is one request, one timeout and no retry (see `linear.rs`), and nothing
// on this side can interrupt a socket that is already waiting.
#[derive(Debug)]
struct Fetching {
    events: Receiver<Landing>,
    // Never read, and that is the whole of what it does: the guard's `Drop` is
    // the cancel, so the field being here is the session's exit path. A handle
    // somebody had to remember to call would be one somebody could forget to.
    #[expect(
        dead_code,
        reason = "held for its drop, which is what cancels the fetch in flight"
    )]
    cancel: CancelGuard,
}

/// The run a Yes started: the slices left to cut, which one of them is being
/// drafted, and where that slice's session has got to.
///
/// There is always exactly one slice under way while this lives — the reply
/// that ends one slice starts the next, or ends the run — so "a pull is
/// drafting" needs no flag beside it.
#[derive(Debug)]
struct Slicing<A: Converses> {
    brief: String,
    slices: Vec<Slice>,
    // Into `slices`, so the fraction on the thread and the session in flight
    // cannot come apart: a count kept beside the index would be a second answer
    // to which slice this is.
    at: usize,
    stage: Stage<A>,
}

/// The two things a slice can be doing: talking to the model, or waiting on
/// somebody.
///
/// One value rather than a session and a flag beside it, because the session is
/// in exactly one of the two places — on a worker, or parked here with a
/// question out — and a pair would have a fourth state for every caller to read
/// and none of them to produce.
#[derive(Debug)]
enum Stage<A: Converses> {
    /// A turn on a worker, with the session riding along.
    Drafting(Asking<A>),
    /// A question relayed, the session parked until somebody answers it.
    Waiting(Waiting<A>),
}

/// A question put to whoever is at the panel: the session that asked it, and
/// warlock's attempt at it while that attempt is still being made.
///
/// The session is held rather than dropped and re-opened, because it is the
/// conversation that asked: a slice answered by a fresh session would be one
/// answering a question nobody in it had heard. Nothing is in flight for it
/// here — its next turn starts when the answer arrives.
#[derive(Debug)]
struct Waiting<A> {
    session: Drafting<A>,
    /// The proposal, for as long as it is being made. `None` once it has landed
    /// however it landed, so a question is only ever attempted once: the
    /// failures that reach here are a missing binary, a cancel and a timeout,
    /// and none of the three is better the second time.
    ///
    /// There is no cancel handle beside it. [`propose_answer`] mints its own and
    /// keeps it, which is the honest shape for one turn nothing else can reach:
    /// quitting drops the receiver and the worker finishes into nowhere, having
    /// read a brief, a slice and a repository and written nothing.
    proposing: Option<Receiver<Result<String, agent::Error>>>,
}

// One slice's session, from the panel's side: what the worker will say, and the
// handle that stops the turn it is in.
//
// The handle is the session's own — see [`CancelGuard::over`] — because the
// session wired its agent to it before the turn started, so this is the flag the
// child is listening to. Dropping the guard is the whole of how quitting stops
// a run: losing the session loses the run, and losing the run cancels.
#[derive(Debug)]
struct Asking<A> {
    replies: Receiver<Turned<A>>,
    #[expect(
        dead_code,
        reason = "held for its drop, which is what cancels the slice in flight"
    )]
    cancel: CancelGuard,
}

/// A turn's answer with the session that gave it: the session rides onto the
/// worker, because a [`Drafting`] is stateful across turns and each turn blocks
/// on a subprocess, and it has to come back for the next one.
type Turned<A> = (Drafting<A>, Result<Replied, agent::Error>);

impl Pulls<Linear, ChatAgent> {
    pub(crate) fn new() -> Self {
        // Two conversations of its own at the register the brief was written in,
        // neither of them the panel's: a drafting session has heard none of the
        // reader's talk and answers in JSON, and a proposing one is read-only
        // and one turn long. `warlock pull` opens its sessions off the same
        // value for the same reason.
        Self::with_client(
            Linear,
            Standing::home().ok(),
            ChatAgent::drafting(),
            ChatAgent::proposing(),
        )
    }
}

impl<O: Opens, A: Converses> Pulls<O, A> {
    // The seam a test drives the real value over a stand-in client and two
    // stand-in models through, rather than assembling the pieces underneath and
    // proving something about an arrangement the event loop never has.
    //
    // Two agents and not one, because the two are two conversations: a test that
    // scripted them as one would be proving something about a session warlock
    // does not open.
    pub(crate) const fn with_client(open: O, home: Option<PathBuf>, agent: A, proposer: A) -> Self {
        Self {
            open,
            home,
            agent,
            proposer,
            fetching: None,
            confirm: PullConfirm::Closed,
            ready: None,
            slicing: None,
        }
    }

    // Read once a round by the loop, to draw the window and to decide which
    // window a keystroke belongs to.
    pub(crate) const fn confirm(&self) -> &PullConfirm {
        &self.confirm
    }

    // Read once a round by the loop and once per `/pull` by this value itself,
    // off the one run it keeps: a flag beside it would be a second record of
    // whether a project is being read back.
    pub(crate) const fn fetching(&self) -> bool {
        self.fetching.is_some()
    }

    // Read once a round by the loop and once per `/pull` by this value itself,
    // off the one run it keeps, for the reason above.
    pub(crate) const fn drafting(&self) -> bool {
        self.slicing.is_some()
    }

    // What a second `/pull` is refused against: a project being read back and a
    // project being cut are both this session's one pull, and the run is by far
    // the longer of the two.
    const fn running(&self) -> bool {
        self.fetching() || self.drafting()
    }

    /// `/pull` typed into the composer, with the brief it is about already
    /// spelled the manifest's way.
    ///
    /// The two refusals are asked in the order they have to be: a pull already
    /// running is answered before anything is read, because a machine with no
    /// home is a fact about a request that is not going to be made anyway.
    /// Everything past them is the worker's, so this returns having read
    /// nothing but the clock.
    pub(crate) fn press(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        brief: &str,
        now: Instant,
    ) {
        if self.running() {
            app.panel_mut().note(ALREADY_PULLING, now);
            return;
        }
        // `Standing::home`'s own sentence, asked of the error that words it
        // rather than written again here: the session holds the answer to that
        // reading and not the failure, so this is where the failure is said.
        let Some(home) = self.home.clone() else {
            app.panel_mut()
                .note(one_line(&Error::NoHome.to_string()), now);
            return;
        };

        let work = Work {
            manifest: manifest.clone(),
            root: repo_root.to_path_buf(),
            home,
            brief: brief.to_owned(),
        };
        // Before the worker starts, so the thread says which document is being
        // read back from the instant it is: the answer is a request away and a
        // reader who has just typed the command is looking at the conversation.
        app.panel_mut().note(reading_line(brief), now);
        let cancel = CancelGuard::new();
        self.fetching = Some(Fetching {
            events: spawn_fetch(self.open.clone(), work, cancel.handle()),
            cancel,
        });
    }

    /// What the pull has said since the last round: what the board answered, or
    /// what the slice being drafted came to.
    ///
    /// Drained rather than received, for [`crate::pacting`]'s reason: nothing
    /// here blocks, so frames keep being drawn, the tree keeps scrolling and the
    /// composer stays usable while a request is in flight and while a slice is
    /// being drafted — which for a slice is minutes rather than seconds.
    ///
    /// What comes back is warlock's attempt at a question, on the one round the
    /// attempt landed in and on no other. It is handed up rather than put
    /// anywhere, because the field it goes in is the conversation's and this
    /// value has no business knowing that; the loop puts it there as an ordinary
    /// draft. Every other ending — nothing settled it, the attempt failed, its
    /// worker said nothing — is a line on the thread and no draft at all, so the
    /// field is left empty and the question is still somebody's to answer.
    pub(crate) fn keep_up(&mut self, app: &mut App, now: Instant) -> Option<String> {
        self.landed(app, now);
        self.drafted(app, now)
    }

    fn landed(&mut self, app: &mut App, now: Instant) {
        let Some(fetching) = self.fetching.as_ref() else {
            return;
        };

        let landing = match fetching.events.try_recv() {
            Ok(landing) => landing,
            // Still in flight, and nothing new to say.
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(PULL_LOST.to_owned()),
        };
        let line = match &landing {
            Ok(fetched) => fetched.line(),
            // The worker's own sentence, which is `error.rs`'s wording of
            // whatever stopped it: a machine that cannot say which board it
            // stands at, a brief nothing filed, a project Linear no longer has,
            // one that is not planned, content that is not a scope, or a
            // project with nothing left to cut.
            Err(line) => line.clone(),
        };
        // Taken before the line is put on the thread, so the round that reports
        // a pull is a round on which the next `/pull` is already allowed.
        self.fetching = None;
        app.panel_mut().note(line, now);
        // The question goes up on the round the answer landed, over the line
        // that reports it: what a reader is being asked to confirm is what they
        // have just read. A fetch that failed put its own sentence on the
        // thread and there is nothing to ask about, so nothing opens.
        if let Ok(fetched) = landing {
            self.confirm = fetched.asking();
            self.ready = Some(fetched.ready);
        }
    }

    // Whichever of the two things the slice under way is doing. One drain per
    // round and never both: a session is on a worker or it is parked with a
    // question out, and the stage is the one record of which.
    fn drafted(&mut self, app: &mut App, now: Instant) -> Option<String> {
        if matches!(self.slicing.as_ref()?.stage, Stage::Waiting(_)) {
            return self.proposed(app, now);
        }
        self.turned(app, now);
        None
    }

    // One slice's turn, when its worker has one to report, and the next slice
    // started on the same round: a run that waited for the round after would
    // spend a poll interval of nothing between every two sessions.
    //
    // The run is taken out rather than borrowed because the agent the next
    // session is opened off sits beside it on this value.
    fn turned(&mut self, app: &mut App, now: Instant) {
        let Some(mut slicing) = self.slicing.take() else {
            return;
        };
        let Stage::Drafting(asking) = &slicing.stage else {
            self.slicing = Some(slicing);
            return;
        };
        let turned = match asking.replies.try_recv() {
            Ok(turned) => Some(turned),
            // Still drafting, and nothing new to say.
            Err(TryRecvError::Empty) => {
                self.slicing = Some(slicing);
                return;
            }
            // The worker panicked: the hook has already printed it, and what is
            // left to say is that this slice came to nothing.
            Err(TryRecvError::Disconnected) => None,
        };

        match ended(&slicing.slices[slicing.at], turned) {
            // A question, which is the one ending that leaves the slice where it
            // is: the session is parked with the question on the thread in the
            // words it was asked, and warlock starts on an attempt at it. The
            // run goes no further until somebody has answered, because the next
            // thing this slice says depends on what they say.
            Ended::Asked { session, question } => {
                let slice = &slicing.slices[slicing.at];
                app.panel_mut().note(question_line(slice, &question), now);
                let proposing = spawn_proposal(&self.proposer, &slicing.brief, slice, &question);
                slicing.stage = Stage::Waiting(Waiting {
                    session,
                    proposing: Some(proposing),
                });
                self.slicing = Some(slicing);
                return;
            }
            Ended::Over(lines) => {
                for line in lines {
                    app.panel_mut().note(line, now);
                }
            }
        }

        slicing.at += 1;
        let at = slicing.at;
        let total = slicing.slices.len();
        // The last slice leaves the run taken down, which is what makes the
        // next `/pull` allowed on the round the last one reported.
        if at < total {
            slicing.stage = Stage::Drafting(started(
                &self.agent,
                app,
                &slicing.brief,
                &slicing.slices[at],
                at,
                total,
                now,
            ));
            self.slicing = Some(slicing);
        }
    }

    // Warlock's attempt at the question that is up, when its worker has one to
    // report. The attempt is over however it went — there is no second try —
    // and the question is not: whatever lands here, the field is somebody's and
    // the session is still parked waiting on it.
    fn proposed(&mut self, app: &mut App, now: Instant) -> Option<String> {
        let slicing = self.slicing.as_mut()?;
        let Stage::Waiting(waiting) = &mut slicing.stage else {
            return None;
        };
        let proposal = match waiting.proposing.as_ref()?.try_recv() {
            Ok(Ok(proposal)) => Ok(proposal),
            Ok(Err(error)) => Err(one_line(&error.to_string())),
            // Still thinking, and nothing new to say.
            Err(TryRecvError::Empty) => return None,
            // The worker panicked: the hook has already printed it, and the
            // question is still up with nothing in the field.
            Err(TryRecvError::Disconnected) => Err(SLICE_LOST.to_owned()),
        };
        waiting.proposing = None;

        let slice = &slicing.slices[slicing.at];
        match proposal {
            // Recognised by [`propose_answer`] and not re-read here: the one
            // place that sentence is told from a proposal is the one that asked
            // for it, and a second reader would eventually disagree with it.
            Ok(proposal) if proposal == NOTHING_SETTLES_IT => {
                app.panel_mut().note(settled_line(slice, &proposal), now);
                None
            }
            // Handed up for the field and deliberately not said on the thread:
            // it is a draft nobody has sent, and a thread that reported it would
            // read tomorrow as though warlock had answered the question itself.
            Ok(proposal) => Some(proposal),
            Err(why) => {
                app.panel_mut().note(unproposed_line(slice, &why), now);
                None
            }
        }
    }

    /// Whether a slice is waiting on an answer, which is the whole of what the
    /// loop asks before it routes a submitted draft here rather than into the
    /// conversation.
    ///
    /// A state of the pull and not a mode: it is true for exactly as long as one
    /// session has one question out, and it goes false on the round the answer
    /// is taken.
    pub(crate) fn relaying(&self) -> bool {
        self.waiting().is_some()
    }

    /// What the field is answering for, as a sentence to draw on it, or `None`
    /// while nothing is waiting.
    ///
    /// Named the way every other line about a slice names it, so a reader whose
    /// eye is on the border and a reader whose eye is on the thread are being
    /// told about the same slice in the same words.
    pub(crate) fn answering(&self) -> Option<String> {
        self.waiting()
            .map(|slicing| format!("answering {}", named(&slicing.slices[slicing.at])))
    }

    fn waiting(&self) -> Option<&Slicing<A>> {
        self.slicing
            .as_ref()
            .filter(|slicing| matches!(slicing.stage, Stage::Waiting(_)))
    }

    /// The answer somebody sent, put to the session that asked for it.
    ///
    /// The text goes up exactly as it was handed over: nothing here reads it,
    /// no command in it is recognised and warlock's own attempt has no standing
    /// over anything typed in its place — what was in the field is what was
    /// sent, and the thread says so.
    ///
    /// A no-op with nothing waiting, which is how the loop's one question
    /// ([`Pulls::relaying`]) stays the only one: a submitted draft that arrived
    /// a round late cannot start a turn of a session that has moved on.
    pub(crate) fn answered(&mut self, app: &mut App, answer: &str, now: Instant) {
        let Some(mut slicing) = self.slicing.take() else {
            return;
        };
        slicing.stage = match slicing.stage {
            // Not waiting on anybody, so there is nothing this answers: the
            // stage goes back exactly as it was.
            stage @ Stage::Drafting(_) => stage,
            Stage::Waiting(waiting) => {
                let slice = &slicing.slices[slicing.at];
                app.panel_mut().note(answer_line(slice, answer), now);
                Stage::Drafting(asked(waiting.session, answer))
            }
        };
        self.slicing = Some(slicing);
    }

    /// The same question with the other answer lit. A closed dialog stays
    /// closed, which is [`PullConfirm::lit`]'s rule and not a second one here.
    pub(crate) fn lit(&mut self, answer: Answer) {
        self.confirm = self.confirm.lit(answer);
    }

    /// The question taken down with nothing started: no request is made, no
    /// record is read and the session is left exactly where it was, which is
    /// the whole of what a No costs.
    ///
    /// The slices go down with it. They were the answer to one reading of one
    /// project, and the next `/pull` reads it again.
    pub(crate) fn cancelled(&mut self) {
        self.confirm = PullConfirm::Closed;
        self.ready = None;
    }

    /// The confirmed question: the window down, the first slice's session
    /// opened, and the run running.
    ///
    /// Taken rather than read and then closed, so there is no round on which
    /// both the question and its own run are up.
    ///
    /// A fetch that found nothing to cut is refused before it is ever reported,
    /// so the empty run below cannot happen; it is answered by starting nothing
    /// rather than by an `expect`, because a panic in a panel that is otherwise
    /// running is a worse answer than a Yes that did nothing.
    pub(crate) fn cut(&mut self, app: &mut App, now: Instant) {
        let confirm = mem::replace(&mut self.confirm, PullConfirm::Closed);
        let (Some(cutting), Some(ready)) = (confirm.cutting(), self.ready.take()) else {
            return;
        };
        app.panel_mut().note(cutting_line(cutting.project()), now);

        let total = ready.slices.len();
        let Some(first) = ready.slices.first() else {
            return;
        };
        let asking = started(&self.agent, app, &ready.brief, first, 0, total, now);
        self.slicing = Some(Slicing {
            brief: ready.brief,
            slices: ready.slices,
            at: 0,
            stage: Stage::Drafting(asking),
        });
    }
}

// One slice's session opened and handed to a worker, with the thread told which
// slice it is and how many there are first: the answer is minutes away and a
// reader who has just said yes is looking at the conversation.
//
// `heading` is `pull.rs`'s, so the panel and the subcommand cannot come to name
// the same work differently — a reader who has watched a `warlock pull` is
// looking for the same fractions here.
//
// `for_slice` and not `one_shot`: this is the road with somebody to ask.
fn started<A: Converses>(
    agent: &A,
    app: &mut App,
    brief: &str,
    slice: &Slice,
    place: usize,
    total: usize,
    now: Instant,
) -> Asking<A> {
    app.panel_mut()
        .note(format!("{} — drafting", heading(place, total, slice)), now);
    let session = Drafting::for_slice(agent, brief, slice.heading(), slice.prose());

    turning(session, Drafting::open)
}

// The same session told what somebody answered, back on a worker. Nothing is
// said on the thread here: the answer was put there as it was taken, and the
// slice is already named as the one being drafted.
//
// The answer is the session's to word from here on — an answer given after the
// last round carries [`Drafting`]'s own instruction to draft now — so nothing on
// this side counts rounds or decides when the asking is over.
fn asked<A: Converses>(session: Drafting<A>, answer: &str) -> Asking<A> {
    let answer = answer.to_owned();
    turning(session, move |session| session.answer(&answer))
}

// A session handed to a worker for one turn, with the handle that stops the turn
// it is in taken before it goes: the handle is the session's own — see
// [`CancelGuard::over`] — so it reaches the `claude` this turn is actually
// running.
//
// The `JoinHandle` is dropped on purpose, as every other worker's is: joining is
// waiting, and this thread exists precisely so nobody waits for it. The guard
// the caller keeps is what stops it.
//
// The session is moved in and sent back out with the reply rather than shared
// behind a lock: it is the one thing that carries what this slice's
// conversation has already said, and the event loop's thread has no business
// touching it while a turn is running.
fn turning<A, F>(session: Drafting<A>, turn: F) -> Asking<A>
where
    A: Converses,
    F: FnOnce(&mut Drafting<A>) -> Result<Replied, agent::Error> + Send + 'static,
{
    let cancel = CancelGuard::over(session.cancel());

    Asking {
        replies: spawn_turn(session, turn),
        cancel,
    }
}

fn spawn_turn<A, F>(mut session: Drafting<A>, turn: F) -> Receiver<Turned<A>>
where
    A: Converses,
    F: FnOnce(&mut Drafting<A>) -> Result<Replied, agent::Error> + Send + 'static,
{
    let (events, received) = mpsc::channel();
    thread::spawn(move || {
        let replied = turn(&mut session);
        // Ignored for the reason every other worker's send is: a receiver that
        // has gone away is an application that is quitting, which is also the
        // one thing that cancels a run.
        let _ = events.send((session, replied));
    });

    received
}

// Warlock's attempt at one question, on a worker of its own for the reason a
// turn is on one: it is a whole `claude` invocation, and a panel that waited for
// it would stop drawing for as long as it thought.
//
// Everything it needs is copied in — the brief, the one slice, the question —
// because the run on this side goes on living while it thinks, and a worker
// borrowing from it would be a worker the next round could not move past.
//
// `propose_answer` is called whole rather than assembled here: it is what knows
// the register, the one turn, and which replies are the session saying it has
// nothing.
fn spawn_proposal<A: Converses>(
    agent: &A,
    brief: &str,
    slice: &Slice,
    question: &str,
) -> Receiver<Result<String, agent::Error>> {
    let (events, received) = mpsc::channel();
    let (agent, brief, question) = (agent.clone(), brief.to_owned(), question.to_owned());
    let (title, prose) = (slice.heading().to_owned(), slice.prose().to_owned());
    thread::spawn(move || {
        let proposed = propose_answer(&agent, &brief, &title, &prose, &question);
        // Ignored for the reason above: a receiver that has gone away is a
        // question nobody is waiting on any more.
        let _ = events.send(proposed);
    });

    received
}

/// What one slice's turn came to: a question to put to somebody, or the lines
/// that end this slice.
///
/// The session rides out on the question and nowhere else, which is the whole
/// distinction: an ending that is lines is an ending, and the slices left are
/// other work — they were ordered so that nothing is drafted before what it
/// waits on, and a run that stopped would leave the reader typing `/pull` again
/// to reach them.
enum Ended<A> {
    Asked {
        session: Drafting<A>,
        question: String,
    },
    Over(Vec<String>),
}

// The session is dropped on every path but the question's. Nothing after the
// asking has anything more to say to it: the drafts are in hand, or the slice is
// uncut and asking again is a turn spent on an answer that was not better the
// first time.
fn ended<A>(slice: &Slice, turned: Option<Turned<A>>) -> Ended<A> {
    let Some((session, replied)) = turned else {
        return Ended::Over(vec![format!(
            "{} was not drafted: {SLICE_LOST}",
            named(slice)
        )]);
    };

    // Questions apart, every arm below is lines about a slice that is over.
    let lines = match replied {
        Ok(Replied::Answer(Drafted::Drafts { fill, repairs })) => {
            let titles: Vec<String> = fill
                .drafts
                .iter()
                .map(|draft| draft.title.clone())
                .collect();
            let mut lines = vec![format!("{} — drafted {}", named(slice), listed(&titles))];
            // Beside the titles for `pull.rs`'s reason: a repaired draft is a
            // ticket that was drafted, not one that was missed, and a
            // conversation read back tomorrow should be able to tell the two
            // apart.
            lines.extend(repairs.iter().map(|repair| repair_line(slice, repair)));
            lines
        }
        // Four answers and not an object among them, which on this road takes
        // the whole of the asking first. Reported and left uncut rather than
        // filed as the document road's floor would supply: a supplied ticket is
        // warlock putting work nobody planned on somebody's board.
        Ok(Replied::Answer(Drafted::Unusable(defect))) => {
            vec![format!("{} was not drafted: {defect}", named(slice))]
        }
        // The one ending that is not an ending: the session goes back to the
        // caller with it, because its next turn is whatever somebody answers.
        Ok(Replied::Question(question)) => return Ended::Asked { session, question },
        // A missing binary, a timeout or a cancel, none of which is better the
        // second time — see [`Drafting`]'s own note — so the slice is left
        // uncut rather than asked again.
        Err(error) => vec![format!("{} was not drafted: {error}", named(slice))],
    };

    Ended::Over(lines)
}

fn repair_line(slice: &Slice, repair: &str) -> String {
    format!("{} — {repair}", named(slice))
}

// The question in the words it was asked, flattened as the thread takes a line.
// Whoever is at the panel answers it, so this line and [`answer_line`] are the
// pair a conversation is read back by: `asked` is the slice talking and
// `answered` is the panel, and the two verbs are the whole of how a reader
// tomorrow tells one from the other.
fn question_line(slice: &Slice, question: &str) -> String {
    format!("{} asked: {}", named(slice), one_line(question))
}

// What was sent, in the words it was sent in, and the other half of that pair.
// Warlock's attempt and something typed over it land here identically on
// purpose: what went to the session is what was in the field, and a line that
// said which of the two it was would be warlock reporting its own draft rather
// than the answer.
fn answer_line(slice: &Slice, answer: &str) -> String {
    format!("{} was answered: {}", named(slice), one_line(answer))
}

// The session having nothing to offer, said in the sentence `propose_answer`
// hands back and no other words: the question is still up, the field is still
// empty, and the answer is entirely whoever is reading's.
fn settled_line(slice: &Slice, settles: &str) -> String {
    format!("{} — {settles}", named(slice))
}

// An attempt that never came back with anything. One line and the question left
// standing: nothing was sent, the session is still waiting, and the field is
// empty for somebody to answer in their own words.
fn unproposed_line(slice: &Slice, why: &str) -> String {
    format!("{} — no answer was proposed: {why}", named(slice))
}

// Everything the worker owns, and the whole of what crosses the thread
// boundary beside the seam: the manifest is cloned as a pact's is, because the
// board is resolved over there and a `Target` borrows the manifest it was found
// in.
#[derive(Debug, Clone)]
struct Work {
    manifest: Manifest,
    root: PathBuf,
    home: PathBuf,
    // The manifest's own spelling, which is what a filed record is keyed by:
    // the composer's `/pull docs/a-brief.md` and the path `/write` remembered
    // are both spelled before they get here, so nothing below resolves a path
    // against a working directory.
    brief: String,
}

// The `JoinHandle` is dropped on purpose, as a turn's and a push's are: joining
// is waiting, and this thread exists precisely so nobody waits for it. The
// guard the caller keeps is what stops it.
fn spawn_fetch<O: Opens>(open: O, work: Work, cancel: Cancel) -> Receiver<Landing> {
    let (events, received) = mpsc::channel();
    thread::spawn(move || {
        let landing = fetched(&open, &work, &cancel).map_err(|error| one_line(&error.to_string()));
        // Ignored for the reason every other worker's send is: a receiver that
        // has gone away is an application that is quitting, which is also the
        // one thing that cancels a pull.
        let _ = events.send(landing);
    });

    received
}

// The whole of what a `/pull` does before anybody is asked anything, in the
// order `pull.rs` runs it: the board, the brief's own record, the project, the
// gate, the scope block, and what is left to cut. Everything is a parameter —
// the manifest, the root, the home, the seam — so the whole sequence runs
// against a temporary repository and a temporary home, and no test in this
// crate can reach the developer's key store by standing in the wrong directory.
//
// The cancel is asked either side of the request and nowhere else. A socket
// already waiting cannot be interrupted, so what the guard buys is a worker
// that neither opens one after the session has gone nor reads the repository
// for an answer nobody will hear.
fn fetched<O: Opens>(open: &O, work: &Work, cancel: &Cancel) -> Result<Fetched, Error> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    let target = resolve_filing(&work.manifest, &work.root, &work.home, None)
        .map_err(|source| Error::Filing { source })?;
    let filed = records(&work.root)?;
    let Some(record) = filed.record(&work.brief) else {
        return Err(Error::NoRecord {
            path: work.brief.clone(),
        });
    };

    // The key is read here and on no other line in this module, on the last
    // line before anything can leave the machine — `push.rs` and `pushing.rs`
    // read it in the same one place for the same reason.
    let linear = open.open(target.value());
    let project = fetch_project(&linear, record.project_id())
        .map_err(|source| Error::Linear { source })?
        .ok_or_else(|| Error::UnknownProject {
            id: record.project_id().to_owned(),
            path: filed_path(&work.root),
        })?;

    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    // The gate is `pull.rs`'s, asked of the status the board answered with, so
    // the panel and the subcommand cannot come to disagree about what `Planned`
    // spells. What comes back out of it is that same string, for the line.
    let Some(status) = project.status().filter(|status| is_planned(status)) else {
        return Err(Error::NotPlanned {
            path: work.brief.clone(),
            status: project.status().map(ToOwned::to_owned),
        });
    };
    let status = status.to_owned();

    let block = scope_block_in(project.content()).map_err(|source| Error::ScopeBlock { source })?;
    // `ordered` and not `slices`: the count reported is of the slices a run
    // would walk, in the order it would walk them, so a `depends_on` that names
    // something unreachable is refused here rather than counted.
    let slices = block.ordered();
    // Asked of the file rather than worked out here: `cut_state` matches on the
    // key a record spells and not on a fresh fold of the title beside it, which
    // is the difference between a slice somebody renamed and one warlock
    // quietly treats as already filed.
    let state = filed.cut_state(&work.brief, slices.iter().map(|slice| slice.heading()));
    if state.uncut().is_empty() {
        return Err(Error::AllCut {
            path: work.brief.clone(),
        });
    }
    // Filtered out of `ordered` rather than collected from `cut_state`, so the
    // slices a run walks arrive in the cut order and not in the order the file
    // happened to list them: what a slice waits on has to be reached first.
    let uncut: Vec<Slice> = slices
        .iter()
        .filter(|slice| state.uncut().contains(&slice.heading()))
        .map(|slice| (*slice).clone())
        .collect();

    Ok(Fetched {
        project: project.name().to_owned(),
        status,
        slices: slices.len(),
        ready: Ready {
            brief: block.brief().to_owned(),
            slices: uncut,
        },
        team: target.record().team().to_owned(),
        // The key by name. `Target::value` is read on one line above and
        // nowhere else on this path.
        key: target.key().to_owned(),
    })
}

// The line the run opens with, over the first slice's own: the project named
// once, so everything under it is read as one cut of one project rather than as
// slices of nothing in particular.
fn cutting_line(project: &str) -> String {
    format!("cutting `{project}` into tickets")
}

// Named by the document rather than by the project, because the project has no
// name on this side of the request: what the reader typed is a brief, and the
// record that turns it into an id is the only thing warlock holds until the
// board answers.
fn reading_line(brief: &str) -> String {
    format!("reading the project filed for `{brief}`")
}

// Every test drives a temporary repository and a temporary home, through the
// half of this module that takes both as parameters: nothing in the suite can
// read the sigils, the binding or the key store of the machine it runs on.
#[cfg(test)]
#[path = "tests/pulling.rs"]
mod tests;
