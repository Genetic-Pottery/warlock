//! `/pull`, from the command word to the project cut into issues, driving
//! [`mod@crate::pull`] with somebody at the panel to ask.
//!
//! What this file adds to that module is the asking and the workers. The
//! project is read back on a worker — [`prepare`] makes a request, and
//! everything a reader is asked to confirm is on the wire — and the answer is a
//! line on the thread and the question over it. Every refusal is the sentence
//! [`crate::error::Error`] words for `warlock pull`, so a reader who has met one
//! at a shell meets the same words here.
//!
//! [`Pulls`] is [`crate::pushing::Pushes`]'s shape — the [`Opens`] seam held for
//! the life of the process, the home resolved once, an [`Option`] that is its
//! own say-no to a second run, a channel drained at the bottom of the loop —
//! with one difference: the board is resolved on the worker rather than on the
//! event loop's thread. A push resolves it first because a dialog is about to
//! name the team it found; a pull has nothing to put up until the board has
//! answered, so the sequence from the sigils to the slice count is one thing
//! that fails in one place.
//!
//! The run a Yes starts is one [`Drafting`] session per uncut slice, in cut
//! order and one at a time, and every turn of it happens on a worker: a
//! [`Drafting`] is stateful across turns and each turn blocks on a subprocess,
//! so the session is *moved* onto the thread and handed back through the
//! channel with its reply. A turn driven from [`Pulls::keep_up`] instead would
//! freeze the frame for as long as `claude` took to think.
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
//!
//! Nothing a slice drafts becomes an issue on its own. The drafts land on the
//! thread as titles and then wait behind a [`Review`], which is answered
//! create, skip or feedback: create files that slice on a worker of its own,
//! skip records nothing and asks whether to carry on, and feedback takes
//! whatever is typed into the field and redrafts that one slice exactly once
//! more. When the run ends, however it ends, the project's one comment is said
//! on a worker too if the run created anything.

use std::io;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Instant;

use warlock_engine::drafting::Draft;
use warlock_engine::{Manifest, agent, from_manifest_path};
use warlock_tui::{
    Answer, App, Cancel, Carry, CarryAnswered, ChatAgent, Choice, Converses, Drafting,
    LinearOpener, NOTHING_SETTLES_IT, Opens, PullAnswered, PullConfirm, Replied, Review, Reviewed,
    Slice, propose_answer,
};

use crate::cut::{Cut, listed};
use crate::error::{Error, one_line};
use crate::pacting::CancelGuard;
use crate::pull::{
    self, Announcement, Next, Pull, Reply, Settled, counted, named, not_drafted, prepare, replied,
};
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

// And the comment's, which may or may not have reached the project before the
// worker went down: the issues are recorded either way, so what is owed is a
// look at the project rather than a second comment.
const ANNOUNCE_LOST: &str =
    "the project's comment stopped without saying how it went; look at the project to see";

/// What a pull has to say for itself once the board has answered: the project
/// read back and gated, or the line that says why not.
///
/// A `String` for the failure, because the failure is worded on the worker
/// where it happens — out of `error.rs`, flattened — and a line is what the
/// thread takes.
type Landing = Result<Pull, String>;

// The count it names first is how many slices the project has, because that is
// what the reader is being asked about; how much of it is left is the line's
// second half.
fn fetched_line(pull: &Pull) -> String {
    format!(
        "`{}` is `{}` — {}, {} still to cut",
        pull.name(),
        pull.status(),
        counted(pull.total()),
        pull.left()
    )
}

fn asking(pull: &Pull) -> PullConfirm {
    PullConfirm::open(
        pull.name(),
        pull.status(),
        pull.total(),
        pull.destination().team(),
        pull.destination().key(),
    )
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
    // The pull the question above is asked about, parked beside it: the dialog
    // names counts and a board, and a Yes walks the slices themselves. Taken by
    // the Yes and dropped by the No, so it lives exactly as long as the question
    // does. Never fetched again at the Yes: a second reading would be a second
    // project to disagree with the one the reader confirmed.
    ready: Option<Pull>,
    // The run, which is its own say-no to a second `/pull` for as long as it
    // lasts: a slice in flight is a session spending on this conversation's
    // behalf, and two sets of them would be two runs cutting one project.
    slicing: Option<Slicing<A>>,
    // The project's one comment, in flight after the run that owed it. No
    // cancel guard, for [`Filing`]'s reason: it is a mutation, and a receiver
    // dropped by quitting leaves a worker that finishes into nowhere.
    announcing: Option<Receiver<Option<String>>>,
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

/// The run a Yes started: the pull being walked, the slice under way, and
/// where that slice's session has got to.
///
/// There is always exactly one slice under way while this lives — the reply
/// that ends one slice starts the next, or ends the run — so "a pull is
/// drafting" needs no flag beside it.
#[derive(Debug)]
struct Slicing<A: Converses> {
    pull: Pull,
    next: Next,
    // Whether this slice has spent its one redraft. Cleared as the run moves on,
    // because it is a fact about the slice under way and not about the run: one
    // feedback each, and the second review of a redrafted slice is offered with
    // two answers rather than three.
    redrafted: bool,
    stage: Stage<A>,
}

/// The things a slice can be doing: talking to the model, waiting on somebody,
/// or being filed.
///
/// One value rather than a session and flags beside it, because the slice is in
/// exactly one of them — on a worker, parked with a question out, held behind
/// the review window, being filed, or holding the carry-on question — and a
/// record per state would have combinations for every caller to read and none of
/// them to produce.
#[derive(Debug)]
enum Stage<A: Converses> {
    /// A turn on a worker, with the session riding along.
    Drafting(Asking<A>),
    /// A question relayed, the session parked until somebody answers it.
    Waiting(Waiting<A>),
    /// Drafts in hand and the review window up.
    Reviewing(Reviewing<A>),
    /// Feedback asked for, the session parked until somebody types it: the
    /// window is down, because what is being asked for is text rather than an
    /// answer to a question.
    Feedback(Drafting<A>),
    /// The drafts on their way to the board, on a worker.
    Filing(Filing),
    /// A slice skipped, with the run asking whether to go on to the next.
    Carrying(Carry),
}

/// One slice's drafts, held behind the window that decides what becomes of
/// them.
///
/// The drafts are kept whole while the titles alone are on the thread and in the
/// window: what create files is this, and a review that held titles would be a
/// window whose Yes had to ask the model again for the bodies.
///
/// The session is `Some` for exactly as long as this slice still has its one
/// redraft. It is the conversation that drafted these, so feedback given to it
/// is feedback about something it said; once spent it goes, because a second
/// redraft is not offered and a session nobody can reach is a `claude` held
/// open for nothing.
#[derive(Debug)]
struct Reviewing<A> {
    review: Review,
    drafts: Vec<Draft>,
    session: Option<Drafting<A>>,
}

// One slice being filed, from the panel's side: what the worker will say, and
// nothing else.
//
// No cancel guard, unlike every other worker here, and deliberately: a create
// that has left the machine cannot be taken back, and the cut record beside it
// is what stops the next run filing the same drafts twice. So a pull dropped
// mid-create leaves a worker that finishes filing and records what it filed,
// into a channel nobody is listening to — which is the only ending that does not
// lose issues.
#[derive(Debug)]
struct Filing {
    landings: Receiver<Result<Cut, String>>,
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

impl Pulls<LinearOpener, ChatAgent> {
    pub(crate) fn new() -> Self {
        // Two conversations of its own at the register the brief was written in,
        // neither of them the panel's: a drafting session has heard none of the
        // reader's talk and answers in JSON, and a proposing one is read-only
        // and one turn long. `warlock pull` opens its sessions off the same
        // value for the same reason.
        Self::with_client(
            LinearOpener,
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
            announcing: None,
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

    // What a second `/pull` is refused against: a project being read back, a
    // project being cut and the comment on a project just cut are all this
    // session's one pull.
    const fn running(&self) -> bool {
        self.fetching() || self.drafting() || self.announcing.is_some()
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
        self.announced(app, now);
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
            Ok(pull) => fetched_line(pull),
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
        if let Ok(pull) = landing {
            self.confirm = asking(&pull);
            self.ready = Some(pull);
        }
    }

    // The comment's one line, when there is one: a comment that was said is
    // said on the project and not again here, and one Linear turned down is a
    // line rather than a failure, because the issues it names exist either way.
    fn announced(&mut self, app: &mut App, now: Instant) {
        let Some(announcing) = self.announcing.as_ref() else {
            return;
        };
        let line = match announcing.try_recv() {
            Ok(line) => line,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Some(ANNOUNCE_LOST.to_owned()),
        };
        self.announcing = None;
        if let Some(line) = line {
            app.panel_mut().note(line, now);
        }
    }

    // Whichever of the things the slice under way is doing has a worker to
    // drain. One drain per round and never two: the stage is the one record of
    // which, and the three stages that are waiting on a person — the review
    // window, the field a feedback goes in, the carry-on question — have nothing
    // in flight to ask about.
    fn drafted(&mut self, app: &mut App, now: Instant) -> Option<String> {
        match self.slicing.as_ref()?.stage {
            Stage::Drafting(_) => self.turned(app, now),
            Stage::Waiting(_) => return self.proposed(app, now),
            Stage::Filing(_) => self.cutting(app, now),
            Stage::Reviewing(_) | Stage::Feedback(_) | Stage::Carrying(_) => {}
        }

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

        match ended(slicing.next.slice(), turned) {
            // A question, which is the one ending that leaves the slice where it
            // is: the session is parked with the question on the thread in the
            // words it was asked, and warlock starts on an attempt at it. The
            // run goes no further until somebody has answered, because the next
            // thing this slice says depends on what they say.
            Ended::Asked { session, question } => {
                let slice = slicing.next.slice();
                app.panel_mut().note(question_line(slice, &question), now);
                let proposing =
                    spawn_proposal(&self.proposer, slicing.pull.brief(), slice, &question);
                slicing.stage = Stage::Waiting(Waiting {
                    session,
                    proposing: Some(proposing),
                });
                self.slicing = Some(slicing);
                return;
            }
            // Drafts, which are not an ending either: they are said as they
            // arrived and then wait behind the window, because nothing this
            // run drafted becomes an issue until somebody says so.
            Ended::Drafted {
                session,
                drafts,
                lines,
            } => {
                for line in lines {
                    app.panel_mut().note(line, now);
                }
                let titles = drafts.iter().map(|draft| draft.title.clone()).collect();
                // The redraft is offered exactly once per slice, so the second
                // time round the window goes up with two answers and the
                // session goes with the drafts it has already given.
                let feedback = !slicing.redrafted;
                slicing.stage = Stage::Reviewing(Reviewing {
                    review: Review::open(named(slicing.next.slice()), titles, feedback),
                    drafts,
                    session: feedback.then_some(session),
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

        self.onwards(slicing, app, now);
    }

    // The slice after this one, started on the round this one settled: a run
    // that waited for the round after would spend a poll interval of nothing
    // between every two sessions.
    //
    // The run is taken by value because the agent the next session is opened off
    // sits beside it on this value, and it is put back only when there is a next
    // slice: the last one leaves the run taken down, which is what makes the
    // next `/pull` allowed once the project's comment has been said.
    fn onwards(&mut self, mut slicing: Slicing<A>, app: &mut App, now: Instant) {
        let Some(next) = slicing.pull.next_uncut() else {
            self.finished(&slicing.pull);
            return;
        };
        slicing.redrafted = false;
        slicing.stage =
            Stage::Drafting(started(&self.agent, app, slicing.pull.brief(), &next, now));
        slicing.next = next;
        self.slicing = Some(slicing);
    }

    // Every road out of a run comes through here — the last slice settled, the
    // last slice skipped, a No to carrying on — so the comment is asked for once
    // and the pull decides whether one is owed.
    fn finished(&mut self, pull: &Pull) {
        if let Some(announcement) = pull.finish() {
            self.announcing = Some(spawn_announcement(self.open.clone(), announcement));
        }
    }

    // One slice's filing, when its worker has one to report: the identifiers on
    // the thread, every edge Linear turned down beside them, and the run on to
    // the next slice.
    //
    // A refusal is one line and the next slice, not the end of the run, for the
    // reason a failed draft is: what is left was ordered so that nothing is
    // filed before what it waits on, and a run that stopped would leave the
    // reader typing `/pull` again to reach it.
    fn cutting(&mut self, app: &mut App, now: Instant) {
        let Some(mut slicing) = self.slicing.take() else {
            return;
        };
        let Stage::Filing(filing) = &slicing.stage else {
            self.slicing = Some(slicing);
            return;
        };
        let landing = match filing.landings.try_recv() {
            Ok(landing) => landing,
            // Still filing, and nothing new to say.
            Err(TryRecvError::Empty) => {
                self.slicing = Some(slicing);
                return;
            }
            // The worker panicked: the hook has already printed it, and what is
            // left to say is that this slice cannot be reported on. Whatever it
            // created is on the board with its record beside it, which is what
            // the next `/pull` will read.
            Err(TryRecvError::Disconnected) => Err(SLICE_LOST.to_owned()),
        };

        let settled = landing.map(|cut| slicing.pull.settle(&slicing.next, cut));
        let slice = slicing.next.slice();
        match settled {
            Ok(Settled::Already(issues)) => {
                app.panel_mut().note(already_line(slice, &issues), now);
            }
            Ok(Settled::Filed { issues, reported }) => {
                app.panel_mut().note(filed_line(slice, &issues), now);
                // One line each, and after the identifiers: an issue that
                // exists with a missing edge is something a person can fix on
                // the board, and it is only fixable if they are told.
                for line in reported {
                    app.panel_mut().note(refused_line(slice, &line), now);
                }
            }
            Err(why) => {
                app.panel_mut().note(unfiled_line(slice, &why), now);
            }
        }

        self.onwards(slicing, app, now);
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

        let slice = slicing.next.slice();
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

    /// Whether a slice is waiting on text from the field, which is the whole of
    /// what the loop asks before it routes a submitted draft here rather than
    /// into the conversation.
    ///
    /// Two stages answer it and not one: a question a slice asked, and the
    /// feedback a review asked for. Both are the same routing — whatever is in
    /// the field is taken whole and goes to this session — and the words on the
    /// field say which of the two it is.
    ///
    /// A state of the pull and not a mode: it is true for exactly as long as
    /// one session is waiting on somebody, and it goes false on the round the
    /// text is taken.
    pub(crate) fn relaying(&self) -> bool {
        self.waiting().is_some()
    }

    /// What the field is taking text for, as a sentence to draw on it, or
    /// `None` while nothing is waiting.
    ///
    /// Named the way every other line about a slice names it, so a reader whose
    /// eye is on the border and a reader whose eye is on the thread are being
    /// told about the same slice in the same words. The verb is the difference:
    /// `answering` is a question the slice asked, `redrafting` is the reader's
    /// own say about drafts they have just read.
    pub(crate) fn answering(&self) -> Option<String> {
        let slicing = self.waiting()?;
        let slice = named(slicing.next.slice());
        Some(match slicing.stage {
            Stage::Feedback(_) => format!("redrafting {slice}"),
            _ => format!("answering {slice}"),
        })
    }

    fn waiting(&self) -> Option<&Slicing<A>> {
        self.slicing
            .as_ref()
            .filter(|slicing| matches!(slicing.stage, Stage::Waiting(_) | Stage::Feedback(_)))
    }

    /// The window a slice's drafts are waiting behind, for the round the loop
    /// is drawing, or `None` when nothing is waiting to be reviewed.
    pub(crate) fn reviewing(&self) -> Option<&Review> {
        match &self.slicing.as_ref()?.stage {
            Stage::Reviewing(reviewing) => Some(&reviewing.review),
            _ => None,
        }
    }

    /// The carry-on question a skip left up, for the same round, or `None` when
    /// nothing was skipped.
    pub(crate) fn carrying(&self) -> Option<&Carry> {
        match &self.slicing.as_ref()?.stage {
            Stage::Carrying(carry) => Some(carry),
            _ => None,
        }
    }

    /// The text somebody sent, put to the session waiting on it.
    ///
    /// It goes up exactly as it was handed over: nothing here reads it, no
    /// command in it is recognised and warlock's own attempt has no standing
    /// over anything typed in its place — what was in the field is what was
    /// sent, and the thread says so.
    ///
    /// The two stages that take text take it identically, because it is one
    /// turn of one session either way; the line on the thread is what says
    /// whether the session was answering a question or being told to draft
    /// again.
    ///
    /// A no-op with nothing waiting, which is how the loop's one question
    /// ([`Pulls::relaying`]) stays the only one: a submitted draft that arrived
    /// a round late cannot start a turn of a session that has moved on.
    pub(crate) fn answered(&mut self, app: &mut App, answer: &str, now: Instant) {
        let Some(mut slicing) = self.slicing.take() else {
            return;
        };
        let slice = slicing.next.slice().clone();
        slicing.stage = match slicing.stage {
            // Not waiting on anybody, so there is nothing this answers: the
            // stage goes back exactly as it was.
            stage @ (Stage::Drafting(_)
            | Stage::Reviewing(_)
            | Stage::Filing(_)
            | Stage::Carrying(_)) => stage,
            Stage::Waiting(waiting) => {
                app.panel_mut().note(answer_line(&slice, answer), now);
                Stage::Drafting(asked(waiting.session, answer))
            }
            Stage::Feedback(session) => {
                app.panel_mut().note(feedback_line(&slice, answer), now);
                // Said here rather than where the answer was asked for, so the
                // slice is spent by the turn that redrafts it and not by a
                // reader who chose feedback and then thought better of it.
                slicing.redrafted = true;
                Stage::Drafting(asked(session, answer))
            }
        };
        self.slicing = Some(slicing);
    }

    /// The pull dialog, moved or answered. An arrow re-lights the question that
    /// is up — the facts it was opened with ride along unchanged, since they are
    /// what is being answered about — and either answer takes it down.
    ///
    /// A No leaves the session exactly where it was and sends nothing: the
    /// project is still `Planned`, nothing was written and the fetch that put
    /// this up has already had its say on the thread. A Yes takes the window
    /// down and starts the run. Either way warlock goes on running, which is
    /// what the dialog promised.
    pub(crate) fn confirmed(&mut self, app: &mut App, answered: PullAnswered, now: Instant) {
        match answered {
            PullAnswered::Open(answer) => self.lit(answer),
            PullAnswered::Cancel => self.cancelled(),
            PullAnswered::Cut => self.cut(app, now),
        }
    }

    /// One slice's drafts, answered about. An arrow re-lights the window — the
    /// titles it was opened with ride along, since they are what is being
    /// answered about — and each of the three answers takes it down.
    ///
    /// Create is the only one that sends anything, and it sends it from a
    /// worker; skip records nothing and asks whether to carry on; feedback
    /// leaves the field taking whatever the reader has to say. Whichever it is,
    /// the session goes on exactly where it was.
    pub(crate) fn reviewed(&mut self, app: &mut App, answered: Reviewed, now: Instant) {
        match answered {
            Reviewed::Open(choice) => self.review_lit(choice),
            Reviewed::Create => self.create(app, now),
            Reviewed::Skip => self.skip(app, now),
            Reviewed::Feedback => self.feedback(app, now),
        }
    }

    /// The question a skipped slice left up: a Yes drafts the next slice and a
    /// No ends the run with the rest of the project untouched. Neither writes
    /// anything, here or on the board.
    pub(crate) fn carried(&mut self, app: &mut App, answered: CarryAnswered, now: Instant) {
        match answered {
            CarryAnswered::Open(answer) => self.carry_lit(answer),
            CarryAnswered::Carry => self.carry_on(app, now),
            CarryAnswered::Stop => self.stop(app, now),
        }
    }

    /// The review window with another answer lit. A window that is not up stays
    /// down, which is the rule every other dialog here keeps.
    fn review_lit(&mut self, choice: Choice) {
        if let Some(slicing) = self.slicing.as_mut()
            && let Stage::Reviewing(reviewing) = &mut slicing.stage
        {
            reviewing.review = reviewing.review.with_choice(choice);
        }
    }

    /// Create: this slice's drafts on their way to the board, on a worker.
    ///
    /// The window comes down on the round the answer is given, so there is no
    /// round on which both it and its own filing are up. The session goes with
    /// it: the drafts are in hand, and a conversation kept open past the answer
    /// that spends it is a `claude` held for nothing.
    ///
    /// Nothing on this thread sends. The client is built over there, which is
    /// what keeps the panel drawing while Linear is answering — one create per
    /// draft, and the edges after them.
    fn create(&mut self, app: &mut App, now: Instant) {
        let Some(mut slicing) = self.slicing.take() else {
            return;
        };
        let reviewing = match slicing.stage {
            Stage::Reviewing(reviewing) => reviewing,
            // A key that reached the wrong window: the stage goes back exactly
            // as it was and nothing is sent.
            stage => {
                slicing.stage = stage;
                self.slicing = Some(slicing);
                return;
            }
        };

        app.panel_mut().note(filing_line(slicing.next.slice()), now);
        let landings = spawn_filing(
            self.open.clone(),
            slicing.pull.filing(&slicing.next, reviewing.drafts),
        );

        slicing.stage = Stage::Filing(Filing { landings });
        self.slicing = Some(slicing);
    }

    /// Skip: nothing is recorded for this slice, and the run asks whether to go
    /// on to the ones after it.
    ///
    /// No record, no request and no note of the refusal anywhere but the
    /// thread: a skipped slice is one the next `/pull` offers again, which is
    /// the whole difference between skipping drafts and filing them.
    ///
    /// The last slice has nothing to ask about, so it ends the run instead: a
    /// question whose only answer is "there is nothing left" is one nobody
    /// should have to press a key for.
    fn skip(&mut self, app: &mut App, now: Instant) {
        let Some(mut slicing) = self.slicing.take() else {
            return;
        };
        if !matches!(slicing.stage, Stage::Reviewing(_)) {
            self.slicing = Some(slicing);
            return;
        }

        app.panel_mut()
            .note(skipped_line(slicing.next.slice()), now);
        let left = slicing.next.left();
        if left == 0 {
            self.finished(&slicing.pull);
            return;
        }

        slicing.stage = Stage::Carrying(Carry::open(counted(left)));
        self.slicing = Some(slicing);
    }

    /// Feedback: the window down and the field taking whatever the reader has
    /// to say about these drafts.
    ///
    /// A slice that has spent its redraft has no session to hear it, and this
    /// is a no-op there rather than a second conversation: the window it was
    /// answered from does not draw the answer at all (see [`Review::feedback`]),
    /// so a call with none left is a key that reached the wrong window.
    fn feedback(&mut self, app: &mut App, now: Instant) {
        let Some(mut slicing) = self.slicing.take() else {
            return;
        };
        let reviewing = match slicing.stage {
            Stage::Reviewing(reviewing) => reviewing,
            stage => {
                slicing.stage = stage;
                self.slicing = Some(slicing);
                return;
            }
        };
        let Reviewing {
            review,
            drafts,
            session,
        } = reviewing;
        let Some(session) = session else {
            slicing.stage = Stage::Reviewing(Reviewing {
                review,
                drafts,
                session: None,
            });
            self.slicing = Some(slicing);
            return;
        };

        app.panel_mut()
            .note(asking_feedback_line(slicing.next.slice()), now);
        slicing.stage = Stage::Feedback(session);
        self.slicing = Some(slicing);
    }

    /// The carry-on question with the other answer lit, and the same rule about
    /// a window that is not up.
    fn carry_lit(&mut self, answer: Answer) {
        if let Some(slicing) = self.slicing.as_mut()
            && let Stage::Carrying(carry) = &slicing.stage
        {
            slicing.stage = Stage::Carrying(carry.with_answer(answer));
        }
    }

    /// A Yes to the carry-on question: on to the next slice.
    fn carry_on(&mut self, app: &mut App, now: Instant) {
        let Some(slicing) = self.slicing.take() else {
            return;
        };
        if !matches!(slicing.stage, Stage::Carrying(_)) {
            self.slicing = Some(slicing);
            return;
        }

        self.onwards(slicing, app, now);
    }

    /// A No: the run ends here, with the slices after this one never offered.
    ///
    /// They are left rather than refused — nothing has been drafted for them
    /// and nothing sent about them — so the next `/pull` finds them exactly as
    /// this one did.
    fn stop(&mut self, app: &mut App, now: Instant) {
        let Some(slicing) = self.slicing.take() else {
            return;
        };
        if !matches!(slicing.stage, Stage::Carrying(_)) {
            self.slicing = Some(slicing);
            return;
        }

        app.panel_mut().note(stopped_line(slicing.next.left()), now);
        self.finished(&slicing.pull);
    }

    /// The same question with the other answer lit. A closed dialog stays
    /// closed, which is [`PullConfirm::lit`]'s rule and not a second one here.
    fn lit(&mut self, answer: Answer) {
        self.confirm = self.confirm.lit(answer);
    }

    /// The question taken down with nothing started: no request is made, no
    /// record is read and the session is left exactly where it was, which is
    /// the whole of what a No costs.
    ///
    /// The slices go down with it. They were the answer to one reading of one
    /// project, and the next `/pull` reads it again.
    fn cancelled(&mut self) {
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
    fn cut(&mut self, app: &mut App, now: Instant) {
        let confirm = mem::replace(&mut self.confirm, PullConfirm::Closed);
        let (Some(cutting), Some(mut pull)) = (confirm.cutting(), self.ready.take()) else {
            return;
        };
        app.panel_mut().note(cutting_line(cutting.project()), now);

        let Some(next) = pull.next_uncut() else {
            return;
        };
        let asking = started(&self.agent, app, pull.brief(), &next, now);
        self.slicing = Some(Slicing {
            pull,
            next,
            redrafted: false,
            stage: Stage::Drafting(asking),
        });
    }
}

// One slice's session opened and handed to a worker, with the thread told which
// slice it is and how many there are first: the answer is minutes away and a
// reader who has just said yes is looking at the conversation.
//
// `for_slice` and not `one_shot`: this is the road with somebody to ask.
fn started<A: Converses>(
    agent: &A,
    app: &mut App,
    brief: &str,
    next: &Next,
    now: Instant,
) -> Asking<A> {
    app.panel_mut()
        .note(format!("{} — drafting", next.heading()), now);
    let slice = next.slice();
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

/// What one slice's turn came to: a question to put to somebody, drafts to be
/// answered about, or the lines that end this slice.
///
/// The session rides out on the first two and not on the third, which is the
/// whole distinction: a question's next turn is whatever somebody answers and
/// drafts can be asked for again once, while an ending is an ending — the slices
/// left are other work, they were ordered so that nothing is drafted before what
/// it waits on, and a run that stopped would leave the reader typing `/pull`
/// again to reach them.
enum Ended<A> {
    Asked {
        session: Drafting<A>,
        question: String,
    },
    Drafted {
        session: Drafting<A>,
        drafts: Vec<Draft>,
        /// What to say about them as they arrive: the titles, then one line per
        /// repair.
        lines: Vec<String>,
    },
    Over(Vec<String>),
}

// The session is dropped on the endings alone. Nothing after one has anything
// more to say to it: the slice is uncut, and asking again is a turn spent on an
// answer that was not better the first time.
fn ended<A>(slice: &Slice, turned: Option<Turned<A>>) -> Ended<A> {
    let Some((session, replied_with)) = turned else {
        return Ended::Over(vec![not_drafted(slice, SLICE_LOST)]);
    };

    match replied(slice, replied_with) {
        // Held rather than filed: what becomes of them is the review window's
        // answer, and the session goes with them because feedback is a turn of
        // this same conversation. The titles go first, over the repairs.
        Reply::Drafts { drafts, lines } => {
            let titles: Vec<String> = drafts.iter().map(|draft| draft.title.clone()).collect();
            let mut said = vec![format!("{} — drafted {}", named(slice), listed(&titles))];
            said.extend(lines);
            Ended::Drafted {
                session,
                drafts,
                lines: said,
            }
        }
        // The one ending that is not an ending: the session goes back to the
        // caller with it, because its next turn is whatever somebody answers.
        Reply::Question(question) => Ended::Asked { session, question },
        Reply::Over(line) => Ended::Over(vec![line]),
    }
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

// What the reader told the slice about its drafts, in their own words and the
// other half of the pair [`question_line`] and [`answer_line`] make: the verb
// says this was feedback rather than an answer to anything the slice asked.
fn feedback_line(slice: &Slice, feedback: &str) -> String {
    format!(
        "{} is being redrafted: {}",
        named(slice),
        one_line(feedback)
    )
}

// The window down and the field waiting, said before anything is typed so that a
// reader who has just pressed Feedback is told where their words are to go.
fn asking_feedback_line(slice: &Slice) -> String {
    format!("{} — say what these drafts should be instead", named(slice))
}

// A create started, over the requests it is about to make: the board is a
// network away and a reader who has just said Create is looking at the
// conversation.
fn filing_line(slice: &Slice) -> String {
    format!("{} — filing", named(slice))
}

// What a slice became, by identifier, which is the one thing about an issue that
// must not be lost: a cut record keeps identifiers and nothing else, and this is
// the same list `warlock pull` prints.
fn filed_line(slice: &Slice, issues: &[String]) -> String {
    format!("{} — cut into {}", named(slice), listed(issues))
}

// A slice the record already claims, which a run over the uncut slices cannot
// reach — said rather than asserted, because a panic in a panel that is
// otherwise running is a worse answer than a line saying what the file already
// said.
fn already_line(slice: &Slice, issues: &[String]) -> String {
    format!(
        "{} — already cut as {}, so nothing was sent",
        named(slice),
        listed(issues)
    )
}

// One edge Linear turned down, in the filing path's own words: the issues exist
// either way, and an issue with a missing edge is something a person can fix on
// the board if they are told about it.
fn refused_line(slice: &Slice, refused: &str) -> String {
    format!("{} — {refused}", named(slice))
}

// A create that came to nothing: a team with nowhere to put an issue, a request
// Linear turned down, a record that would not save. One line, and the run goes
// on to the next slice.
fn unfiled_line(slice: &Slice, why: &str) -> String {
    format!("{} was not filed: {why}", named(slice))
}

// A slice left alone. `nothing was recorded` rather than `skipped` alone,
// because what a reader wants to know tomorrow is whether the next `/pull` will
// offer this slice again — and it will.
fn skipped_line(slice: &Slice) -> String {
    format!("{} was skipped; nothing was recorded for it", named(slice))
}

// The run ended by a No to the carry-on question, counting what was never
// offered: those slices are untouched rather than refused, so the next `/pull`
// finds them exactly as this one did.
fn stopped_line(left: usize) -> String {
    format!("the run stopped; {} left for another pull", counted(left))
}

// Everything the fetch's worker owns, and the whole of what crosses the thread
// boundary beside the seam: the manifest is cloned as a pact's is, because the
// board is resolved over there.
#[derive(Debug)]
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

// One slice's drafts handed to a worker to be filed. No cancel handle, for
// [`Filing`]'s reason: what this does cannot be taken back, and the record it
// writes is what stops the next run filing the same drafts again.
//
// `io::sink` where [`cut::cut`](crate::cut::cut)'s progress would have gone,
// exactly as `pushing.rs` gives `push::file` nowhere to print: the panel's lines
// are the panel's own and worded beside the slice they are about.
//
// The `JoinHandle` is dropped on purpose, as every other worker's is.
fn spawn_filing<O: Opens>(open: O, filing: pull::Filing) -> Receiver<Result<Cut, String>> {
    let (events, received) = mpsc::channel();
    thread::spawn(move || {
        let landing = filing
            .file(&open, &mut io::sink())
            .map_err(|error| one_line(&error.to_string()));
        // Ignored for the reason every other worker's send is: a receiver that
        // has gone away is a panel nobody is looking at any more. What this
        // worker did is on the board and in the cut record beside the brief,
        // which is where the next `/pull` reads it from.
        let _ = events.send(landing);
    });

    received
}

// The project's one comment, on a worker for the reason a create is on one: it
// is a request, and the panel keeps drawing while Linear answers it.
fn spawn_announcement<O: Opens>(open: O, announcement: Announcement) -> Receiver<Option<String>> {
    let (events, received) = mpsc::channel();
    thread::spawn(move || {
        let _ = events.send(announcement.post(&open));
    });

    received
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

// The cancel is asked either side of [`prepare`] and nowhere else. A socket
// already waiting cannot be interrupted, so what the guard buys is a worker that
// neither opens one after the session has gone nor hands back an answer nobody
// will hear.
fn fetched<O: Opens>(open: &O, work: &Work, cancel: &Cancel) -> Result<Pull, Error> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    let pull = prepare(
        &work.manifest,
        &work.root,
        &work.home,
        &from_manifest_path(&work.root, &work.brief),
        None,
        open,
    )?;

    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    Ok(pull)
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
