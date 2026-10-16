//! `/pull` on the panel: the scope's queue gives up a ticket, a dialog names it,
//! and a Yes drives the very loop [`mod@crate::pulling`] runs for `warlock pull`.
//!
//! This is [`mod@crate::cutting`]'s shape with one more seam and one more worker.
//! The refusals that cost nothing are asked on the event loop's thread —
//! [`prepare`] opens no socket and runs no `git`, so a second pull, a machine with
//! no home, an unrecorded scope and a scope closed on this machine are each one
//! line and no work at all. Selection is a worker of its own, because it reads a
//! tree and a queue; the answer is what the dialog is opened about, and only a Yes
//! starts the run.
//!
//! The run is a third worker, and it is the whole of [`Pulling::pull`]: nothing
//! here re-decides what a sub-task is, what a halt is or what goes on the ticket.
//! What this module owns is the reporting. [`Pulling::progress`] is a `&mut dyn
//! FnMut` and [`Activities`] wants a `Fn + Send + Sync + 'static`, so the worker
//! joins the two onto one channel of [`Step`] and [`Puller::keep_up`] routes it,
//! all of it onto the thread, as `forman pull` prints all of it to the terminal:
//! each heading is a line with a live work turn under it, activities tick in that
//! turn, and milestones — the ticket taken, each commit, each repair, the halt or
//! the pull request — are lines between them. A run read anywhere else looked
//! hung to somebody watching the conversation.
//!
//! Two things a shell never needs are bought here. A commit is a milestone and the
//! loop reports none, so the checkout the run is given is wrapped
//! ([`Committing`]): every commit it makes says so on its way through. And a
//! session in flight has to be stoppable from the thread that quits, which the
//! agents cannot carry: [`Working::on`] and [`Splitting::for_ticket`] each mint a
//! [`Cancel`] of their own and re-wire the agent to it, so a handle attached before
//! the session was raised is not the one the child answers. [`Stopping`] is that
//! handle published as the session is raised, and [`StopGuard`]'s `Drop` is what
//! latches it — quitting the panel drops the run, and dropping the run kills the
//! `claude` a sub-task is running.
//!
//! One question comes before the queue, for [`mod@crate::pushing`]'s reason and
//! ported from it. [`prepare`] is also what resolves the key, so a scope this
//! checkout binds to nothing is a pull it can only refuse — and a reader who has
//! just cloned the repository would be sent to a shell to find `warlock key
//! use`. So the binding is asked for first, between the home and [`prepare`]:
//! the key window goes up over the names this machine holds, the `/pull` waits
//! in [`Parked`], and that answer decides whether there is a queue to read at
//! all. The window is the one the `s` key puts up, held by the session —
//! nothing here keeps a second copy of it — and nothing on the road reads a key:
//! the question is about names, and the binding is written by
//! [`mod@crate::scoping`].
//!
//! Nothing in the run asks the panel anything. A sub-task that needs a decision
//! comes back `blocked`, the loop halts on it, and the reason is a line on the
//! thread: there is no relay into a session that is editing the tree.

use std::mem;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use warlock_engine::{LinearScope, Manifest, PullRun, load_key_binding};

use crate::app::App;
use crate::claude::{
    Activities, Activity, Cancel, CancelGuard, ChatAgent, Split, Splitting, Worked, Working,
    working_system_prompt,
};
use crate::confirm::{PullAnswered, PullConfirm, Undertaking};
use crate::error::{Error, files_to_linear, one_line};
use crate::git::{Commit, Dirty, Error as GitError, Forge, Gh, Git, Repository, branch_name};
use crate::inflight::{Lost, Once, Port, Stream, Workers, settled};
use crate::linear::{
    Board, Error as LinearError, FetchedProject, Issue as LinearIssue, IssueProject, Listing,
    NamedIssue, NewIssue, NewProject, Opener as LinearOpener, Opens, Project as LinearProject,
    Queue,
};
use crate::prompt::KeyPrompt;
use crate::pull::{
    Prepared, clean, nothing_ready, number_in, opened, passed_over, prepare, selection, unchanged,
};
use crate::pulling::{PullEvent, Pulled, Pulling, Splits, Ticket, Works, next_runnable};
use crate::standing::Standing;
use crate::submission::Taking;

// Said of a lost selection: nothing was read, which is the honest answer for a
// sequence that reads a tree and a queue and writes neither.
const CHOICE_LOST: &str = "the pull stopped while choosing a ticket; nothing was read or changed";

// Said of a lost run, and it promises far less: a run that died half-way
// through has a branch with commits on it and a record under the home saying
// how far it got, which is what `/resume` and the next `/pull` read.
const PULL_LOST: &str = "the pull stopped without saying how it went; the run record under \
                         your home says how far it got";

/// The pull a session is doing, if it is doing one, and every seam it spends.
///
/// Built with its home rather than reading one, for [`Cutter`]'s reason: it cannot
/// move under a running warlock, a second reading per keystroke would be a second
/// answer, and a value built with the home it is to use is what keeps every test
/// in this crate off the developer's own.
///
/// No `Debug`, for [`Pulling`]'s reason twice over: these are seams and a
/// checkout, and the [`Work`] parked below carries the key the board is opened
/// with.
///
/// [`Cutter`]: crate::cutting::Cutter
pub(crate) struct Puller<O: Opens, R: Repository, F: Forge, M: Raises> {
    open: O,
    repo: R,
    forge: F,
    /// The three sessions a run spends, built when the run starts rather than
    /// here: see [`Raises`].
    raises: M,
    home: Option<PathBuf>,
    /// The `/pull` waiting on a binding, if one is. Never more than one: a key
    /// window takes every keystroke while it is up, so no second `/pull` can be
    /// typed behind this one.
    parked: Option<Parked>,
    /// The queue being read, which is its own say-no to a second `/pull` for as
    /// long as it lasts.
    choosing: Option<Choosing>,
    /// The question between the selection and the run, held here rather than
    /// beside the session's other windows because it is a state of the pull and
    /// not of the app: what it names came off the board on the round it went up,
    /// and nothing else in the panel can answer it.
    confirm: PullConfirm,
    /// What that question is asked about, parked beside it and taken by the Yes:
    /// the ticket selection chose and the scope it was chosen under. Never
    /// chosen again at the Yes — a second reading would be a second ticket to
    /// disagree with the one the reader confirmed.
    ready: Option<Ready>,
    /// The run, which is its own say-no to a second `/pull`, to a pass, to a
    /// scope write and to a `/draft`: a sub-task is a session editing this
    /// working tree, and anything else that reads or writes it while that
    /// happens is reading somebody else's half-finished edit.
    underway: Option<Underway>,
    workers: Workers,
}

/// A `/pull` waiting on a binding: the scope it named and the ticket it named,
/// if it named one, exactly as that command spelled them.
///
/// Parked rather than read again when the window is answered, for the reason the
/// dialog holds its [`Ready`]: a second reading of the command could name
/// another scope, or another ticket, from the one the question was asked about.
/// And those two names only — no key, no binding and nothing off the store —
/// because the window the reader is looking at is what holds the choice.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Parked {
    scope: String,
    /// `None` takes the queue's own choice, exactly as [`Work::named`] does.
    ticket: Option<String>,
}

impl Parked {
    // The command as it was typed, handed back whole to the one road that reads
    // a queue: what was parked is what is picked up.
    fn taking(self) -> Taking {
        Taking {
            scope: self.scope,
            ticket: self.ticket,
        }
    }
}

// The selection worker, and the handle that is the whole of how it is stopped.
// What a cancel can reach is either side of the requests rather than a socket
// already waiting; see [`crate::cutting`]'s `Fetching`.
struct Choosing {
    /// Everything the run will need, kept while the queue is read: the worker has
    /// a clone, and this is the copy a Yes starts from. Parked rather than sent
    /// back through the channel, so what the reader confirmed is what was
    /// prepared.
    work: Work,
    landings: Once<Landing>,
    // Never read, and that is the whole of what it does: the guard's `Drop` is
    // the cancel, so the field being here is the session's exit path.
    #[expect(
        dead_code,
        reason = "held for its drop, which is what cancels the selection in flight"
    )]
    cancel: CancelGuard,
}

/// The ticket a confirmed question is about, with everything the run needs to
/// work it.
struct Ready {
    work: Work,
    undertook: Undertook,
}

/// The run in flight, from the event loop's side.
struct Underway {
    /// The ticket, for the line every other keystroke is refused with: a refusal
    /// that named nothing would leave the reader to guess which pull is holding
    /// the tree.
    ticket: String,
    events: Stream<Step>,
    #[expect(
        dead_code,
        reason = "held for its drop, which latches the run's say-when"
    )]
    cancel: CancelGuard,
    #[expect(
        dead_code,
        reason = "held for its drop, which kills the session in flight"
    )]
    sessions: StopGuard,
}

/// Everything one pull is allowed to know, owned, because all of it crosses onto
/// a worker.
///
/// The manifest is cloned rather than shared. The scope record is the one
/// [`Prepared`] cloned rather than one asked of the manifest again on the
/// worker, which could disagree with the boundary the reader was shown.
#[derive(Clone)]
struct Work {
    prepared: Prepared,
    manifest: Manifest,
    /// The ticket somebody named, or `None` for the queue's own choice.
    named: Option<String>,
}

/// The ticket one pull works, owned for [`Work`]'s reason.
struct Held {
    id: String,
    identifier: String,
    number: u32,
    title: String,
    description: String,
}

impl Held {
    // The loop's own view of it, borrowed back out.
    fn ticket(&self) -> Ticket<'_> {
        Ticket {
            id: &self.id,
            identifier: &self.identifier,
            number: self.number,
            title: &self.title,
            description: &self.description,
        }
    }
}

/// What selection chose: the ticket, the branch the run will be on, and the
/// sub-task a held run carries on from.
///
/// The branch is worked out here rather than at the run, because the dialog names
/// it before anything is created: a fresh pull's is [`branch_name`]'s, and a run
/// this checkout already holds keeps the one its record names.
struct Undertook {
    ticket: Held,
    branch: String,
    resuming: Option<String>,
}

/// What selection came to: the lines it owes the thread either way, and the
/// ticket — or `None` for a queue with nothing ready to work, which is an answer
/// rather than a refusal.
struct Chose {
    lines: Vec<String>,
    undertook: Option<Undertook>,
}

/// A `String` for the failure, because it is worded on the worker where it
/// happens — out of `error.rs`, flattened — and a line is what the thread takes.
type Landing = Result<Chose, String>;

/// What a run is seen doing, as one stream: the loop's own events, the two things
/// only this door can see, and the ending.
///
/// One channel and not three, so the order things happened in is the order they
/// are drawn in: an activity belongs under the heading that was open when it
/// arrived, and two channels drained in sequence would file a session's first
/// tool call under the section before it.
pub(crate) enum Step {
    Pull(PullEvent),
    /// One commit, as the checkout was asked to make it. Reported here rather
    /// than by the loop because the loop reports none.
    Committed(String),
    Finished(Result<Pulled, String>),
}

impl Puller<LinearOpener, Git, Gh, Claudes> {
    /// The live pull: the one [`Opens`] that opens a socket, `git` and `gh` in the
    /// repository this session was started in, and sessions raised off `claude`.
    ///
    /// Cheap to build and builds nothing: a checkout is a directory and a program
    /// name, an agent is a command line and a timeout, so no socket, no `git` and
    /// no `claude` exists until a confirmed question starts a run. The home under
    /// which the config, the key store and the run records sit is
    /// read here, once for the session.
    pub(crate) fn new(root: &Path) -> Self {
        Self::with_seams(
            LinearOpener,
            Git::at(root),
            Gh::at(root),
            Claudes,
            Standing::home().ok(),
        )
    }
}

impl<O, R, F, M> Puller<O, R, F, M>
where
    O: Opens,
    R: Repository + Clone + Send + 'static,
    F: Forge + Clone + Send + 'static,
    M: Raises + Clone + Send + 'static,
{
    // The seam a test drives the real value over stand-in seams through, rather
    // than assembling the pieces underneath and proving something about an
    // arrangement the event loop never has.
    pub(crate) const fn with_seams(
        open: O,
        repo: R,
        forge: F,
        raises: M,
        home: Option<PathBuf>,
    ) -> Self {
        Self {
            open,
            repo,
            forge,
            raises,
            home,
            parked: None,
            choosing: None,
            confirm: PullConfirm::Closed,
            ready: None,
            underway: None,
            workers: Workers::Threaded,
        }
    }

    #[cfg(test)]
    pub(crate) fn inline(self) -> Self {
        Self {
            workers: Workers::Inline,
            ..self
        }
    }

    // Read once a round by the loop, to draw the window and to decide which
    // window a keystroke belongs to.
    pub(crate) const fn confirm(&self) -> &PullConfirm {
        &self.confirm
    }

    /// Where the run records sit, for the one other command that reads them: a
    /// `/resume` releases a halt of a run this value would pick up, so the two
    /// read the same directory or they are talking about two different halts.
    /// Read off here rather than resolved a second time in the loop, for the
    /// reason this value was built with it — see [`Puller::with_seams`].
    pub(crate) fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    // Read once a round by the loop and once per `/pull` by this value itself,
    // off the one run it keeps: a flag beside it would be a second record of
    // whether a ticket is being worked.
    pub(crate) const fn pulling(&self) -> bool {
        self.underway.is_some()
    }

    // Read by the tests alone, which wait on the selection worker through it. The
    // loop never needs to ask: `in_flight` already words the queue being read.
    #[cfg(test)]
    pub(crate) const fn choosing(&self) -> bool {
        self.choosing.is_some()
    }

    // `in_flight().is_some()` without wording a line on every draw; the two must
    // answer alike.
    pub(crate) fn busy(&self) -> bool {
        self.underway.is_some() || self.confirm.undertaking().is_some() || self.choosing.is_some()
    }

    /// The line a keystroke that races a pull is refused with, and `None` when
    /// nothing is in flight.
    ///
    /// One sentence for every caller — the second `/pull`, a pass, a scope write,
    /// a `/draft`, a `/resume`, the composer — because they are all refused for
    /// the one reason: a session is editing this working tree, and everything
    /// else here reads or writes it. Each names what it did not do itself; this
    /// names the pull.
    pub(crate) fn in_flight(&self) -> Option<String> {
        if let Some(underway) = self.underway.as_ref() {
            return Some(format!("`{}` is being pulled", underway.ticket));
        }
        // Reachable only from a road that does not go through the keyboard: with
        // the question up every key belongs to it. Worded anyway rather than
        // asserted, because a `/pull` that started a second run from behind a
        // dialog would be two runs in one tree.
        if let Some(undertaking) = self.confirm.undertaking() {
            return Some(format!(
                "the question about `{}` is still up",
                undertaking.ticket()
            ));
        }
        let choosing = self.choosing.as_ref()?;
        Some(format!(
            "a pull of `{}` is choosing a ticket",
            choosing.work.prepared.record().name()
        ))
    }

    /// `/pull <SCOPE>` typed into the composer. A bare `/pull` never reaches
    /// here: the loop answers it from this checkout's settings.
    ///
    /// The refusals are asked in the order they have to be. A pull already
    /// in flight is answered before anything is read; a machine with no home is a
    /// fact about a request that is not going to be made anyway; and the four
    /// [`prepare`] asks are the ones `warlock pull` asks, in its order and its
    /// words, so a reader who has met one at a shell meets the same sentence here.
    ///
    /// Two of the four are asked ahead of [`prepare`] rather than by it. The
    /// tracker, off the manifest, because a scope whose queue warlock cannot
    /// read must not cost a key read or a window; and the key, because a scope
    /// this checkout binds to nothing is a question somebody looking at the
    /// panel can answer. What comes back is the key window the session is to put
    /// up, which is [`KeyPrompt::Closed`] on every road but that one — the
    /// session holds the window, and this value holds the `/pull` waiting behind
    /// it.
    pub(crate) fn press(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        taking: &Taking,
        now: Instant,
    ) -> KeyPrompt {
        // Whatever an earlier `/pull` parked goes here: this command says which
        // scope and which ticket the next question is about. Nothing can be
        // parked in practice — a key window up takes every keystroke, so no
        // `/pull` reaches the composer behind one — and it is dropped anyway,
        // because a pull that outlived its own window would read a queue on an
        // Enter meant for another one.
        self.parked = None;
        if let Some(line) = self.in_flight() {
            app.panel_mut().note(refused(&line), now);
            return KeyPrompt::Closed;
        }
        // `Standing::home`'s own sentence, asked of the error that words it
        // rather than written again here.
        let Some(home) = self.home.clone() else {
            app.panel_mut().note(one_line(Error::NoHome), now);
            return KeyPrompt::Closed;
        };

        // Before the binding is read and before anything is parked behind the
        // key window: a scope whose queue warlock cannot read is a line on the
        // thread, and asking somebody to bind a key for a `/pull` that can only
        // be refused would be a window spent on a question that does not matter.
        // `prepare` asks the same question again on the road past here.
        if let Err(error) = files_to_linear(manifest, &taking.scope) {
            app.panel_mut().note(one_line(error), now);
            return KeyPrompt::Closed;
        }

        // After the home, which is where the config sits, and before
        // `prepare`, which is what resolves the key: a scope bound to nothing
        // gets the question rather than the refusal.
        match load_key_binding(&home, repo_root, &taking.scope) {
            // Bound, which is every `/pull` that worked before this question
            // existed: on to `prepare`, the reading line and the queue. The
            // name is not kept — what the run opens the board with comes back
            // from `prepare`, and this read is only whether there is one.
            Ok(Some(_)) => {}
            // Bound to nothing, so nothing on this machine could read this
            // scope's queue: the window, and the pull parked behind it. No
            // reading line either, because nothing is being read.
            Ok(None) => return self.binding(app, &home, taking, now),
            // A checkout nobody has configured, and a config that will not
            // read. The engine's own sentence on the thread, as every other
            // `/pull` refusal is, and no window: the answer would be written
            // into the very file that could not be read.
            Err(source) => {
                app.panel_mut()
                    .note(one_line(Error::Config { source }), now);
                return KeyPrompt::Closed;
            }
        }

        self.choose(app, manifest, repo_root, &home, taking, now);
        KeyPrompt::Closed
    }

    /// The parked `/pull` picked up over the binding the key window has just
    /// written: straight to [`prepare`], which is the road a `/pull` with a key
    /// bound takes on the round it is typed — the reading line on the thread,
    /// the choice on a worker, and the dialog on the round the queue answers.
    ///
    /// Taken rather than read and then cleared, so one question leads to one
    /// reading and no later Enter at the key window — from the `s` key's road,
    /// or from a session with no home — can pick this pull up a second time.
    /// Does nothing at all when no `/pull` is waiting, which is every other way
    /// that window is answered.
    ///
    /// A binding the submit could not write is not asked about again:
    /// [`prepare`] refuses an unbound scope in the engine's own words, and that
    /// line on the thread is the answer rather than a second window.
    pub(crate) fn bound(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        now: Instant,
    ) {
        let Some(parked) = self.parked.take() else {
            return;
        };
        // The home this value was built with, as the press that parked the pull
        // read it: a home cannot move under a running warlock, and this is the
        // one it was resolved against.
        let Some(home) = self.home.clone() else {
            app.panel_mut().note(one_line(Error::NoHome), now);
            return;
        };

        self.choose(app, manifest, repo_root, &home, &parked.taking(), now);
    }

    /// The key window abandoned, and the `/pull` that was waiting on it with
    /// it: nothing is prepared, no queue is read, no worker starts and no dialog
    /// opens.
    ///
    /// The binding is the window's own to not write — Esc never opens the config
    /// — so this is the whole of what an Esc costs on this side.
    pub(crate) fn put_down(&mut self) {
        self.parked = None;
    }

    // The key window over this machine's key names, with the pull parked behind
    // it.
    //
    // Names and no values, which is [`KeyPrompt::open`]'s whole shape: it lists
    // the store through `load_key_names`, exactly as the `s` key's own road
    // does, and there is no field on it a value could sit in.
    fn binding(&mut self, app: &mut App, home: &Path, taking: &Taking, now: Instant) -> KeyPrompt {
        match KeyPrompt::open(home, &taking.scope) {
            Ok(prompt) => {
                self.parked = Some(Parked {
                    scope: taking.scope.clone(),
                    ticket: taking.ticket.clone(),
                });
                prompt
            }
            // A store that will not read, in the engine's own sentence on the
            // thread: there is nothing to pick from, so no window goes up and
            // no pull waits for one.
            Err(source) => {
                app.panel_mut().note(one_line(Error::Keys { source }), now);
                KeyPrompt::Closed
            }
        }
    }

    // Where both roads to a reading end: the `/pull` typed over a scope with a
    // key bound, and the one picked up behind the key window.
    fn choose(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        repo_root: &Path,
        home: &Path,
        taking: &Taking,
        now: Instant,
    ) {
        let prepared = match prepare(manifest, repo_root, home, &taking.scope) {
            Ok(prepared) => prepared,
            // The scope no `[[scope]]` record holds, the scope closed on this
            // machine, and the checkout the key window could not
            // bind, each in `error.rs`'s words and flattened as the thread
            // takes a line.
            Err(error) => {
                app.panel_mut().note(one_line(error), now);
                return;
            }
        };

        let work = Work {
            prepared,
            manifest: manifest.clone(),
            named: taking.ticket.clone(),
        };
        // Before the worker starts, so the thread says which queue is being read
        // from the instant it is: the answer is a request away and a reader who
        // has just typed the command, or just bound the scope, is looking at the
        // conversation.
        app.panel_mut()
            .note(reading_line(&taking.scope, taking.ticket.as_deref()), now);
        let cancel = CancelGuard::new();
        self.choosing = Some(Choosing {
            landings: spawn_choice(
                self.workers,
                self.open.clone(),
                self.repo.clone(),
                work.clone(),
                cancel.handle(),
            ),
            work,
            cancel,
        });
    }

    /// What the pull has said since the last round: what selection chose, and
    /// whatever the run in flight has done.
    ///
    /// Drained rather than received: nothing here blocks, so frames keep being
    /// drawn, the tree keeps scrolling and the
    /// conversation stays readable while a run that takes an hour runs.
    pub(crate) fn keep_up(&mut self, app: &mut App, now: Instant) {
        self.chosen(app, now);
        self.stepped(app, now);
    }

    // What the queue answered: the tickets passed over, then the question about
    // the one taken — or one line, for a queue with nothing ready and for every
    // way the reading failed.
    fn chosen(&mut self, app: &mut App, now: Instant) {
        let Some((choosing, landing)) =
            settled(&mut self.choosing, |choosing| choosing.landings.landed())
        else {
            return;
        };

        let chose = match landing.unwrap_or_else(|Lost| Err(CHOICE_LOST.to_owned())) {
            Ok(chose) => chose,
            Err(line) => {
                app.panel_mut().note(line, now);
                return;
            }
        };
        for line in chose.lines {
            app.panel_mut().note(line, now);
        }
        // The question goes up over the lines that report the reading: what a
        // reader is being asked to confirm is what they have just read.
        if let Some(undertook) = chose.undertook {
            match LinearScope::of(choosing.work.prepared.record()) {
                Ok(scope) => self.confirm = asking(&undertook, scope),
                // The reading that got this far was made through the same view,
                // so nothing reaches this arm today. It is a line and not an
                // unwrap because a panic here takes the terminal with it.
                Err(refusal) => {
                    app.panel_mut().note(one_line(refusal), now);
                    return;
                }
            }
            self.ready = Some(Ready {
                work: choosing.work,
                undertook,
            });
        }
    }

    // Everything the run has done since the last round, in the order it did it.
    fn stepped(&mut self, app: &mut App, now: Instant) {
        let Some((_, ending)) = settled(&mut self.underway, |underway| {
            underway.events.drained(|step| {
                match step {
                    Step::Pull(event) => said(app, event, now),
                    Step::Committed(message) => {
                        app.panel_mut().note(committed_line(&message), now);
                    }
                    Step::Finished(ending) => return ControlFlow::Break(ending),
                }
                ControlFlow::Continue(())
            })
        }) else {
            return;
        };
        let ending = ending.unwrap_or_else(|Lost| Err(PULL_LOST.to_owned()));
        // The last phase's clock stops here; every earlier one stopped when the
        // phase after it opened.
        app.panel_mut().settle_turn(now);
        // One line either way, and nothing else comes down: a halt, a crossing
        // and a failure are all a pull that stopped, and the panel goes on
        // running. What each of them left behind is the branch, the run record
        // and the ticket's own comment.
        let line = match &ending {
            Ok(pulled) => ended_line(pulled),
            Err(line) => line.clone(),
        };
        app.panel_mut().note(line, now);
    }

    /// The pull dialog, moved or answered. An arrow re-lights the question that is
    /// up — the facts it was opened with ride along unchanged, since they are what
    /// is being answered about — and either answer takes it down.
    ///
    /// A No starts nothing at all: no branch is cut, no ticket moves, no session is
    /// raised, and the ticket selection chose is left exactly where it was for the
    /// next `/pull` to choose again.
    pub(crate) fn answered(&mut self, app: &mut App, answered: PullAnswered, now: Instant) {
        match answered {
            // [`PullConfirm::lit`]'s rule and not a second one here: a closed
            // dialog stays closed.
            PullAnswered::Open(answer) => self.confirm = self.confirm.lit(answer),
            PullAnswered::Cancel => {
                self.confirm = PullConfirm::Closed;
                self.ready = None;
            }
            PullAnswered::Pull => self.start(app, now),
        }
    }

    /// The confirmed question: the window down, the panel turned into the run's
    /// output window, and the run running on a worker.
    ///
    /// Taken rather than read and then closed, so there is no round on which both
    /// the question and its own run are up.
    fn start(&mut self, app: &mut App, now: Instant) {
        let confirm = mem::replace(&mut self.confirm, PullConfirm::Closed);
        let (Some(undertaking), Some(ready)) = (confirm.undertaking(), self.ready.take()) else {
            return;
        };
        // The one milestone said before anything happens, because it is the fact
        // every line under it belongs to: this run, of this ticket.
        app.panel_mut().note(taking_line(undertaking), now);

        let cancel = CancelGuard::new();
        let stopping = Stopping::default();
        let events = spawn_run(
            self.workers,
            Spending {
                open: self.open.clone(),
                repo: self.repo.clone(),
                forge: self.forge.clone(),
                raises: self.raises.clone(),
            },
            ready,
            cancel.handle(),
            stopping.clone(),
        );
        self.underway = Some(Underway {
            ticket: undertaking.ticket().to_owned(),
            events,
            cancel,
            sessions: StopGuard(stopping),
        });
    }
}

/// The three sessions one pull spends, built when the run starts.
///
/// A seam of its own rather than three values on the [`Puller`], because none of
/// the three can be built before there is a run: the activity port is this run's
/// channel, the say-when is this run's guard, and the sub-task sessions' system
/// prompt names the scope the ticket was pulled under and the scopes open on
/// this machine.
///
/// The associated types are the three seams [`Pulling`] takes, so a test hands in
/// stand-ins that answer out of memory and a run spends no model pass at all.
pub(crate) trait Raises {
    type Split: Splits + Send + 'static;
    type Sessions: Works + Send + 'static;

    fn raise(&self, asked: Raising<'_>) -> Raised<Self::Split, Self::Sessions>;
}

/// Everything the sessions of one run are built from.
pub(crate) struct Raising<'a> {
    /// The scope the ticket was pulled under, which the sub-task prompt names.
    pub(crate) scope: &'a str,
    /// And the open scopes, which it names beside it.
    pub(crate) held: &'a [String],
    /// Where everything a session is seen doing goes.
    pub(crate) events: Port<Step>,
    /// The sessions' own handles, published as each is raised — see
    /// [`Stopping`].
    pub(crate) stopping: Stopping,
}

/// What [`Raises::raise`] answers with.
pub(crate) struct Raised<S: Splits, W: Works> {
    pub(crate) split: S,
    pub(crate) sessions: W,
}

/// The sessions on the real road: a splitting pass and one sub-task session per
/// sub-task, each off `claude`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Claudes;

impl Raises for Claudes {
    type Split = Splitter;
    type Sessions = Worker;

    fn raise(&self, asked: Raising<'_>) -> Raised<Splitter, Worker> {
        let activities = activity_port(&asked.events);
        Raised {
            split: Splitter::new(activities.clone(), asked.stopping.clone()),
            sessions: Worker::new(asked.scope, asked.held, activities, asked.stopping),
        }
    }
}

/// The splitting session, and the ports it reports and answers through.
///
/// One agent for the run rather than one per attempt: an agent is a command line
/// and a timeout, so nothing is spawned until a split is asked for.
pub(crate) struct Splitter {
    agent: ChatAgent,
    activities: Activities,
    stopping: Stopping,
}

impl Splitter {
    pub(crate) fn new(activities: Activities, stopping: Stopping) -> Self {
        Self {
            agent: ChatAgent::splitting(),
            activities,
            stopping,
        }
    }
}

impl Splits for Splitter {
    fn split(&self, ticket: &str, title: &str, description: &str) -> Split {
        let mut session = Splitting::for_ticket(&self.agent, ticket, title, description)
            .reporting(self.activities.clone());
        self.stopping.raising(session.cancel());
        session.run()
    }
}

/// One sub-task session per call, at the system prompt the scope and the open
/// scopes make.
///
/// The prompt is built once, because it says which scope the work is under and
/// which scopes are open on this machine, and neither moves for the length of a run.
pub(crate) struct Worker {
    agent: ChatAgent,
    activities: Activities,
    stopping: Stopping,
}

impl Worker {
    pub(crate) fn new(
        scope: &str,
        held: &[String],
        activities: Activities,
        stopping: Stopping,
    ) -> Self {
        Self {
            agent: ChatAgent::working(&working_system_prompt(scope, held)),
            activities,
            stopping,
        }
    }
}

impl Works for Worker {
    fn work(&self, opening: &str) -> Worked {
        let mut session = Working::on(&self.agent, opening).reporting(self.activities.clone());
        // Published before the first turn, which is the whole of how quitting
        // reaches the `claude` this sub-task is running: `Working::on` mints a
        // handle of its own and re-wires the agent to it, so the run's own
        // say-when is not the flag this child answers.
        self.stopping.raising(session.cancel());
        session.run()
    }
}

/// The say-when of the session a run has raised, and the whole of how the panel
/// stops one.
///
/// [`Working::on`] and [`Splitting::for_ticket`] each mint a [`Cancel`] of their
/// own and re-wire the agent to it, so a handle wired on before the session was
/// raised is not the flag the child answers. The worker publishes each session's
/// handle in here as it raises it, and the thread that quits latches whatever is
/// in it through [`StopGuard`].
///
/// A session raised after the stop is cancelled as it is published, which is what
/// keeps a run that was stopped between two sub-tasks from spending the next one.
#[derive(Debug, Clone, Default)]
pub(crate) struct Stopping {
    session: Arc<Mutex<Option<Cancel>>>,
    stopped: Arc<AtomicBool>,
}

impl Stopping {
    /// Called by every session as it is raised, from the worker: the handle the
    /// child is really listening to, published where the thread that quits can
    /// reach it. `pub(crate)` because a stand-in session in a test publishes its
    /// own the same way — that is how a quit is driven at all.
    pub(crate) fn raising(&self, cancel: Cancel) {
        let mut session = self.held();
        if self.stopped.load(Ordering::SeqCst) {
            cancel.cancel();
        }
        *session = Some(cancel);
    }

    fn stop(&self) {
        // The flag first, so a session being raised right now is cancelled by
        // whichever of the two gets the lock second.
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(cancel) = self.held().as_ref() {
            cancel.cancel();
        }
    }

    fn held(&self) -> std::sync::MutexGuard<'_, Option<Cancel>> {
        self.session
            .lock()
            .expect("nothing panics holding the session's say-when")
    }
}

/// [`Stopping`] held for its `Drop`, which is the session half of what quitting
/// does: no exit path has to remember to stop a pull, because losing the
/// [`Puller`]'s run loses this.
#[derive(Debug)]
struct StopGuard(Stopping);

impl Drop for StopGuard {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// The seams one run is handed, gathered so the worker takes one value rather than
/// four.
struct Spending<O: Opens, R: Repository, F: Forge, M: Raises> {
    open: O,
    repo: R,
    forge: F,
    raises: M,
}

/// The checkout the run is given, reporting every commit it makes.
///
/// A wrapper rather than an event on [`PullEvent`], and the reason is which door
/// wants it: the shell's account of a run is its headings, where a commit is
/// implied by the sub-task that ends in one, and the panel's thread is a list of
/// milestones where it is not.
struct Committing<R: Repository> {
    inner: R,
    events: Port<Step>,
}

impl<R: Repository> Committing<R> {
    fn said(&self, message: &str) {
        self.events.send(Step::Committed(message.to_owned()));
    }
}

impl<R: Repository> Repository for Committing<R> {
    fn dirty(&self) -> Result<Vec<Dirty>, GitError> {
        self.inner.dirty()
    }

    fn default_branch(&self) -> Result<String, GitError> {
        self.inner.default_branch()
    }

    fn switch_to(&self, branch: &str) -> Result<(), GitError> {
        self.inner.switch_to(branch)
    }

    fn catch_up(&self, branch: &str) -> Result<(), GitError> {
        self.inner.catch_up(branch)
    }

    fn cut_branch(&self, branch: &str, from: &str) -> Result<(), GitError> {
        self.inner.cut_branch(branch, from)
    }

    fn head(&self) -> Result<Commit, GitError> {
        self.inner.head()
    }

    fn changed_against(&self, base: &str) -> Result<Vec<String>, GitError> {
        self.inner.changed_against(base)
    }

    // Reported only after the commit is made: a commit that `git` refused is not
    // a milestone.
    fn commit_all(&self, message: &str) -> Result<(), GitError> {
        self.inner.commit_all(message)?;
        self.said(message);
        Ok(())
    }

    fn publish(&self, branch: &str) -> Result<(), GitError> {
        self.inner.publish(branch)
    }
}

/// The board the run is given: asked nothing at all once the run has been
/// stopped.
///
/// A wrapper rather than a question inside the loop, for [`Committing`]'s reason —
/// which door wants it. A shell has no stop button, so `warlock pull` only ever
/// reaches a halt with somebody waiting on it; the panel reaches one on its way
/// out, because quitting cancels the session in flight and a cancelled session is
/// a sub-task that failed. The record is written before the ticket is told
/// anything, so the halt is recorded either way and `/resume` releases it — what
/// this stops is the comment after it, which would be the ticket's account of a
/// run nobody let finish.
///
/// Every method answers through [`asked`](Quiet::asked) rather than each one
/// asking for itself, so the gate is one line and belongs to the type.
struct Quiet<B: Board> {
    inner: B,
    cancel: Cancel,
}

impl<B: Board> Quiet<B> {
    fn asked<T>(&self, ask: impl FnOnce(&B) -> Result<T, LinearError>) -> Result<T, LinearError> {
        if self.cancel.is_cancelled() {
            return Err(LinearError::Stopped);
        }
        ask(&self.inner)
    }
}

impl<B: Board> Board for Quiet<B> {
    fn viewer(&self) -> Result<String, LinearError> {
        self.asked(Board::viewer)
    }

    fn team_id(&self, key: &str) -> Result<Option<String>, LinearError> {
        self.asked(|board| board.team_id(key))
    }

    fn backlog_status(&self) -> Result<Option<String>, LinearError> {
        self.asked(Board::backlog_status)
    }

    fn project_status(&self, name: &str) -> Result<Option<String>, LinearError> {
        self.asked(|board| board.project_status(name))
    }

    fn move_project(&self, project: &str, status: &str) -> Result<String, LinearError> {
        self.asked(|board| board.move_project(project, status))
    }

    fn issue_project(&self, issue: &str) -> Result<Option<IssueProject>, LinearError> {
        self.asked(|board| board.issue_project(issue))
    }

    fn backlog_state(&self, team: &str) -> Result<Option<String>, LinearError> {
        self.asked(|board| board.backlog_state(team))
    }

    fn workflow_state(&self, team: &str, name: &str) -> Result<Option<String>, LinearError> {
        self.asked(|board| board.workflow_state(team, name))
    }

    fn move_issue(&self, issue: &str, state: &str) -> Result<String, LinearError> {
        self.asked(|board| board.move_issue(issue, state))
    }

    fn issue_label_id(&self, name: &str, team: &str) -> Result<String, LinearError> {
        self.asked(|board| board.issue_label_id(name, team))
    }

    fn fetch_project(&self, slug: &str) -> Result<Option<FetchedProject>, LinearError> {
        self.asked(|board| board.fetch_project(slug))
    }

    fn planned_projects(&self, team: &str, label: &str) -> Result<Listing, LinearError> {
        self.asked(|board| board.planned_projects(team, label))
    }

    fn project_named(&self, team: &str, name: &str) -> Result<Option<String>, LinearError> {
        self.asked(|board| board.project_named(team, name))
    }

    fn scope_queue(&self, team: &str, label: &str, assignee: &str) -> Result<Queue, LinearError> {
        self.asked(|board| board.scope_queue(team, label, assignee))
    }

    fn named_issue(&self, team: &str, number: u64) -> Result<Option<NamedIssue>, LinearError> {
        self.asked(|board| board.named_issue(team, number))
    }

    fn create_project(&self, project: &NewProject<'_>) -> Result<LinearProject, LinearError> {
        self.asked(|board| board.create_project(project))
    }

    fn create_issue(&self, issue: &NewIssue<'_>) -> Result<LinearIssue, LinearError> {
        self.asked(|board| board.create_issue(issue))
    }

    fn create_relation(&self, blocker: &str, waiting: &str) -> Result<String, LinearError> {
        self.asked(|board| board.create_relation(blocker, waiting))
    }

    fn comment_on_project(&self, project: &str, body: &str) -> Result<String, LinearError> {
        self.asked(|board| board.comment_on_project(project, body))
    }

    fn comment_on_issue(&self, issue: &str, body: &str) -> Result<String, LinearError> {
        self.asked(|board| board.comment_on_issue(issue, body))
    }
}

/// The activity port of every session in one run: a clone of the channel, written
/// into from wherever the session reports.
///
/// [`Activities`] takes a `Fn(Activity) + Send + Sync + 'static`, so the port a
/// `claude` reports into cannot borrow anything the run owns.
pub(crate) fn activity_port(events: &Port<Step>) -> Activities {
    let events = events.clone();
    Activities::new(move |activity: Activity| {
        events.send(Step::Pull(PullEvent::Activity(activity)));
    })
}

// The tree, the board and the queue, on a worker: a `git status`, one request for
// the user the key belongs to, the run records this checkout holds and the queue
// itself.
fn spawn_choice<O: Opens, R: Repository + Send + 'static>(
    workers: Workers,
    open: O,
    repo: R,
    work: Work,
    cancel: Cancel,
) -> Once<Landing> {
    workers.once(move || chose(&open, &repo, &work, &cancel).map_err(one_line))
}

/// Which ticket this pull would work, in `pulled`'s own order: the tree before the
/// board, so a dirty checkout is refused before anything is asked of Linear, and
/// the records this checkout holds before the queue, so a ticket in `In Progress`
/// here is told from one in progress somewhere else.
///
/// The cancel is asked either side of the requests and nowhere else: a socket
/// already waiting cannot be interrupted, so what the guard buys is a worker that
/// neither opens one after the session has gone nor hands back an answer nobody
/// will hear.
fn chose<O: Opens, R: Repository>(
    open: &O,
    repo: &R,
    work: &Work,
    cancel: &Cancel,
) -> Result<Chose, Error> {
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    // Before the tree is read and before the board is opened, as the headless
    // door has it: a scope warlock cannot file to is refused having run nothing.
    let scope = LinearScope::of(work.prepared.record())?;
    clean(repo)?;

    let board = open.open(work.prepared.value());
    let mut lines = Vec::new();
    let selected = selection(
        &board,
        scope,
        work.prepared.home(),
        work.prepared.root(),
        work.named.as_deref(),
        &mut |line| lines.push(line),
    )?;
    for skipped in selected.skipped() {
        lines.push(passed_over(skipped));
    }

    let Some(issue) = selected.issue(work.named.as_deref())? else {
        lines.push(nothing_ready(scope.name()));
        return Ok(Chose {
            lines,
            undertook: None,
        });
    };
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }

    let ticket = Held {
        id: issue.id().to_owned(),
        identifier: issue.identifier().to_owned(),
        number: number_in(issue.identifier()),
        title: issue.title().to_owned(),
        description: issue.description().to_owned(),
    };
    // What the dialog names, and the one thing here that is not the queue's: a run
    // this checkout already holds keeps the branch its record names and carries on
    // from the sub-task the loop would run next, and a ticket with no record is
    // told the branch it will be given. A record that cannot be read is `Runs`,
    // the failure the scan above has, and the loop words its own over the same
    // `pulls::Error`, so the two say the same sentence.
    let held = PullRun::find(
        work.prepared.home(),
        work.prepared.root(),
        &ticket.identifier,
    )
    .map_err(|source| Error::Runs { source })?;
    let (branch, resuming) = match held.as_ref() {
        Some(run) => (
            run.branch().to_owned(),
            next_runnable(run).map(|next| next.id().to_owned()),
        ),
        None => (
            branch_name(scope.team_key(), ticket.number, &ticket.title),
            None,
        ),
    };

    Ok(Chose {
        lines,
        undertook: Some(Undertook {
            ticket,
            branch,
            resuming,
        }),
    })
}

// The whole run on a worker, because every step of it blocks: a request, a `git`,
// a model pass.
fn spawn_run<O, R, F, M>(
    workers: Workers,
    spending: Spending<O, R, F, M>,
    ready: Ready,
    cancel: Cancel,
    stopping: Stopping,
) -> Stream<Step>
where
    O: Opens,
    R: Repository + Send + 'static,
    F: Forge + Send + 'static,
    M: Raises + Send + 'static,
{
    workers.stream(|events| {
        move || {
            let Ready { work, undertook } = ready;
            // Before a session is raised and before the board is opened: a
            // scope warlock cannot file to ends the run here, with nothing
            // spent and the line the panel prints for every other way a run
            // stops.
            let scope = match LinearScope::of(work.prepared.record()) {
                Ok(scope) => scope,
                Err(refusal) => {
                    events.send(Step::Finished(Err(one_line(refusal))));
                    return;
                }
            };
            let raised = spending.raises.raise(Raising {
                scope: scope.name(),
                held: work.prepared.held(),
                events: events.clone(),
                stopping,
            });
            // Opened over here, which is what keeps the panel drawing while Linear is
            // answering: the key crosses as a field of the work and is read on this
            // one line. Wrapped in the run's own say-when, so a run stopped by the
            // panel quitting tells the board nothing on its way out — see [`Quiet`].
            let board = Quiet {
                inner: spending.open.open(work.prepared.value()),
                cancel,
            };
            let repo = Committing {
                inner: spending.repo,
                events: events.clone(),
            };
            let reporting = events.clone();
            let mut progress = move |event: PullEvent| reporting.send(Step::Pull(event));
            let pulled = Pulling {
                board: &board,
                repo: &repo,
                forge: &spending.forge,
                split: &raised.split,
                sessions: &raised.sessions,
                scope,
                manifest: &work.manifest,
                held: work.prepared.held(),
                base: work.prepared.base(),
                root: work.prepared.root(),
                home: work.prepared.home(),
                progress: &mut progress,
            }
            .pull(&undertook.ticket.ticket());

            events.send(Step::Finished(pulled.map_err(one_line)));
        }
    })
}

/// One of the loop's events, on the card it belongs to.
///
/// A free function rather than a method, and that is the whole of why the drain
/// above compiles: the run is borrowed off the session while this is called, and
/// nothing here needs the session — a heading, an activity and a milestone are all
/// the app's.
fn said(app: &mut App, event: PullEvent, now: Instant) {
    match event {
        // The three headings, each a phase on the thread. Opening one stops the
        // clock of the one above it, so two never run at once.
        PullEvent::Heading(heading) => phase(app, &heading.line(), now),
        // Filed under the live phase, which is the one the heading before it
        // opened. What each activity comes to is the thread's business — a tool is
        // its name and its one detail, thinking and writing are the words for
        // them, and a cost is dropped. See `Thread::record`.
        PullEvent::Activity(activity) => app.panel_mut().record_turn(&activity, now),
        event => {
            if let Some(line) = event.line() {
                app.panel_mut().note(line, now);
            }
        }
    }
}

// One phase of the run on the thread, as `/draft`'s slices are: the line that
// says what it is, and a live work turn under it that the session's activity
// ticks in. The whole run is read in the conversation, as Forman prints it.
fn phase(app: &mut App, heading: &str, now: Instant) {
    app.panel_mut().note(heading, now);
    app.panel_mut().start_work(now);
}

fn asking(undertook: &Undertook, scope: LinearScope<'_>) -> PullConfirm {
    let Undertook {
        ticket,
        branch,
        resuming,
    } = undertook;
    match resuming {
        // A run this checkout already holds, named by the sub-task the next
        // session starts from: what the reader is being asked is whether to carry
        // it on, which is not the same question as starting one.
        Some(subtask) => PullConfirm::resuming(
            &ticket.identifier,
            &ticket.title,
            scope.name(),
            scope.team_key(),
            branch,
            subtask,
        ),
        None => PullConfirm::open(
            &ticket.identifier,
            &ticket.title,
            scope.name(),
            scope.team_key(),
            branch,
        ),
    }
}

// The line that opens a run, and the fact every line under it belongs to. The
// two wordings are the dialog's two: a ticket taken, and a run carried on from
// the sub-task it stopped at.
fn taking_line(undertaking: &Undertaking) -> String {
    match undertaking.resuming() {
        Some(subtask) => format!(
            "resuming `{}` from `{subtask}` on `{}`",
            undertaking.ticket(),
            undertaking.branch()
        ),
        None => format!(
            "took `{}` — {}, working it on `{}`",
            undertaking.ticket(),
            undertaking.title(),
            undertaking.branch()
        ),
    }
}

// One commit, in the message it was made with: the message already names the
// ticket, the sub-task and what the sub-task was for, so nothing is worded over
// the top of it.
fn committed_line(message: &str) -> String {
    format!("committed — {}", one_line(message))
}

// What the run came to, in the words the shell says it in: a pull request's
// address is the thing that must not be lost, and both haltings carry what the
// ticket's own comment says at length.
fn ended_line(pulled: &Pulled) -> String {
    match pulled {
        Pulled::Opened { ticket, url } => opened(ticket, url.as_deref()),
        Pulled::Unchanged { ticket } => unchanged(ticket),
        Pulled::Halted { ticket } => one_line(Error::Halted {
            ticket: ticket.clone(),
        }),
        Pulled::Crossed { ticket, subtask } => one_line(Error::Crossed {
            ticket: ticket.clone(),
            subtask: subtask.clone(),
        }),
    }
}

// The queue being read, named before the request goes out. The named ticket is
// said when there is one, because a `/pull warlock-team WAR-140` reads one ticket
// rather than a queue.
fn reading_line(scope: &str, ticket: Option<&str>) -> String {
    match ticket {
        Some(ticket) => format!("reading `{ticket}` for `{scope}`"),
        None => format!("reading the queue for `{scope}`"),
    }
}

// A keystroke turned down, in the one sentence every caller shares plus what this
// one did not do. Said here rather than in each caller so the halves cannot
// drift.
fn refused(in_flight: &str) -> String {
    format!("{in_flight}; this `/pull` read nothing")
}

// Every test drives a temporary repository and a temporary home, through the seams
// this value is built with: nothing in the suite reads the config or the key
// store of the machine it runs on, opens a socket, runs a `git` or raises
// a `claude`.
#[cfg(test)]
#[path = "tests/puller.rs"]
mod tests;
