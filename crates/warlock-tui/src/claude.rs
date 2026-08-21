//! The transport half of the agent seam: running `claude` as a child process.
//!
//! The engine defines what a model pass *is* — [`Agent`], its request, its
//! response and its failure vocabulary — and spawns nothing. This module is
//! the other half: it takes the request the engine built, hands the prompt to
//! a child process on its stdin, reads what the child writes to stdout, and
//! translates however that went into the engine's words. It is the only place
//! in this crate that runs anything; everything else here is data and
//! functions over data.
//!
//! Nothing about a prompt is decided here. A [`ClaudeAgent`] never inspects
//! the text it is given and never adds to it, because the moment this file
//! starts composing prompts, domain logic has crossed to the wrong side of the
//! seam.
//!
//! # Why this is fiddlier than "spawn, wait, read"
//!
//! Three ways the obvious code deadlocks, and what is done about each:
//!
//! * **A chatty child fills a pipe.** A pipe holds something like 64KiB; a
//!   child that writes more than that blocks until somebody drains it. Waiting
//!   for exit *before* reading therefore hangs on exactly the passes worth
//!   having — the long ones. So stdout and stderr are each read by their own
//!   thread, concurrently with the wait: stderr drained whole, since nothing
//!   reads it until the pass is judged, and stdout a line at a time, because
//!   every line of it is news.
//! * **A child waits for EOF.** `claude` reads its prompt from stdin until the
//!   stream closes. The write happens on its own thread which then drops the
//!   handle, so the child sees EOF whether or not the prompt is bigger than a
//!   pipe.
//! * **`wait()` holds the child.** [`Child::wait`](std::process::Child::wait)
//!   takes `&mut self`, so a waiter thread that blocks in it owns the only
//!   handle there is, leaving the caller with nothing to kill when the clock
//!   runs out. The std-only answer is a waiter thread that *polls*
//!   [`try_wait`](std::process::Child::try_wait) on a shared
//!   [`Mutex<Child>`](std::sync::Mutex), releasing the lock between polls and
//!   reporting the status over an [`mpsc`] channel, so the calling side does
//!   [`recv_timeout`](std::sync::mpsc::Receiver::recv_timeout) and still has a
//!   handle to kill with.
//!
//! A fourth thing the obvious code cannot do is *stop*. A pact is minutes of
//! passes driven from a worker thread, and the person watching it has to be
//! able to say "enough" from the thread drawing the screen. That is
//! [`Cancel`]: a clonable handle over the same [`Mutex<Child>`](std::sync::Mutex)
//! the timeout path already holds, so cancelling reaches inside a pass that is
//! in flight instead of waiting politely for it to end.
//!
//! A fifth thing it cannot do is *say what it is doing*. A pass is minutes of
//! reading and writing that the person watching sees nothing of until it ends,
//! which is why pressing the pact key looks like a hang. That is
//! [`Activities`]: a port out of the transport, built exactly like [`Cancel`] —
//! a clonable handle over an [`Arc`], attached by whoever wants to listen with
//! [`with_activities`](ClaudeAgent::with_activities), and a no-op for an agent
//! nobody attached one to. What goes out through it is [`Activity`]: the name of
//! a tool and at most one argument, the bare fact of thinking, and what the pass
//! cost. Never a tool's result, never the model's prose, never the content of a
//! thought.
//!
//! That port is why stdout is read the way it is. The child is asked for
//! `--output-format stream-json`, one JSON object per line, and the reader
//! thread hands each line to [`stream::read_line`] and reports what it said the
//! moment it says it — so an activity reaches the listener while the pass is
//! still running, which is the entire point of having one. The reader keeps the
//! document as it goes and gives it back when the stream ends; it is otherwise
//! the same thread doing the same job it did when it drained stdout whole, and
//! it is still not joined on the cancel and timeout paths, for the same reason
//! as ever: a grandchild the kill did not reach can hold the pipe open, and
//! waiting on that is the wait a cancel exists to end.
//!
//! Threads and channels, no async runtime, and no dependency: this crate's
//! `Cargo.toml` gains nothing for any of it.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, BufRead, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use warlock_engine::{Agent, AgentError, AgentRequest, AgentResponse};

/// How long one invocation is given before it is killed.
///
/// Per *invocation*, not per pact: a pact over a subtree is many passes, and
/// each one gets its own five minutes rather than sharing a budget with its
/// siblings.
///
/// Five minutes because the two failures this guards against sit far apart in
/// time. A model pass over one directory is seconds to a minute or two of
/// reading and writing, and a slow one — a big directory, a loaded machine, a
/// retried request — is still comfortably inside five. A genuine hang, by
/// contrast, is forever: a `claude` waiting on a login prompt nobody can see,
/// or a network that will never answer. Anything from roughly two minutes up
/// separates those two cleanly, and five is picked from that range because the
/// cost of the two mistakes is not symmetric. Killing a pass that would have
/// answered throws away real work and shows the user a hang that never
/// happened; waiting three minutes longer than strictly necessary before
/// reporting a hang is an annoyance. So the number errs towards patience.
pub const INVOCATION_TIMEOUT: Duration = Duration::from_mins(5);

/// The command run when nothing else is asked for.
const PROGRAM: &str = "claude";

/// What `claude` is asked to do when nothing else is asked for: print mode, in
/// the shape that narrates itself.
///
/// `--print` is the non-interactive form — hand it a prompt, read its stdout —
/// and is the minimum that makes a piped, terminal-less run work at all.
/// `--output-format stream-json` changes only *how* that stdout arrives: one
/// JSON object per line as the pass happens, instead of the finished document
/// in one go at the end. The document is the same either way; what the stream
/// adds is everything before it, which is what [`Activities`] carries. And
/// `--verbose` is not a preference: the CLI refuses `stream-json` in print mode
/// without it, so the three arguments are one decision, not three.
///
/// Still not a decision about invocation *mode* — headless per directory
/// against one long session is a later slice's call, and section 11 of the
/// design doc leaves it open. It lives in a field so that later slice can
/// change it without touching a line of this file.
const ARGS: [&str; 4] = ["--print", "--output-format", "stream-json", "--verbose"];

/// How often the waiter thread asks whether the child has exited.
///
/// It is the granularity of the whole mechanism: a pass that finishes is
/// noticed within this long, and the lock is free the rest of the time so a
/// timing-out caller can take it and kill. Ten milliseconds is invisible next
/// to a pass measured in seconds and costs a few hundred wakeups a minute.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// A say-when handle for whoever is not running the pass.
///
/// One flag and one slot for the pass in flight, behind an [`Arc`], so cloning
/// is a refcount bump and every clone speaks for the same run: the worker
/// thread gives its [`ClaudeAgent`] one with
/// [`with_cancel`](ClaudeAgent::with_cancel) and the event loop keeps another,
/// which is the whole reason this is [`Send`] + [`Sync`] and not a `&mut bool`.
///
/// [`cancel`](Cancel::cancel) is final and does two things at once: it latches
/// the flag, so a pass started afterwards spawns nothing at all, and it kills
/// and reaps whatever child is running right now, so the answer arrives in
/// milliseconds rather than at the end of a five-minute
/// [`INVOCATION_TIMEOUT`]. There is no un-cancel — a run that was stopped is
/// over, and the next one gets a fresh handle.
///
/// The slot holds *one* child, because one agent runs one pass at a time: a
/// pact is a sequence of passes, not a fan-out. Two passes sharing a handle
/// concurrently would leave the first unreachable, so don't.
///
/// ```
/// use warlock_tui::Cancel;
///
/// let cancel = Cancel::new();
/// let watcher = cancel.clone();
/// assert!(!cancel.is_cancelled());
///
/// // Whoever holds a clone can stop the run, from any thread.
/// std::thread::spawn(move || watcher.cancel()).join().expect("the thread ran");
///
/// assert!(cancel.is_cancelled());
/// ```
#[derive(Debug, Clone, Default)]
pub struct Cancel {
    /// Shared with every clone; the point of the type.
    state: Arc<State>,
}

/// What a [`Cancel`] and its clones share.
#[derive(Debug, Default)]
struct State {
    /// Latched once and never cleared: has somebody said stop?
    cancelled: AtomicBool,
    /// The child of the pass in flight, or `None` between passes.
    ///
    /// Registered before the wait and cleared after it, so a cancel can only
    /// ever kill the pass that is actually running — a stale handle left here
    /// would let a late cancel kill an unrelated later pass.
    running: Mutex<Option<Arc<Mutex<Child>>>>,
}

impl Cancel {
    /// A handle nobody has cancelled yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stop the run: latch the flag, and kill and reap the child of any pass
    /// in flight.
    ///
    /// Safe to call from any thread, more than once, and with no pass running.
    pub fn cancel(&self) {
        // The flag is set *before* the slot is read, and
        // [`Cancel::register`] reads the flag while holding the slot. Between
        // them there is no interleaving where a child is both registered after
        // this read and started before this write: either this call finds the
        // child in the slot and kills it, or the registering call sees the
        // flag and refuses.
        self.state.cancelled.store(true, Ordering::SeqCst);
        let running = lock(&self.state.running).clone();
        if let Some(child) = running {
            kill_and_reap(&child);
        }
    }

    /// Whether anybody has cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::SeqCst)
    }

    /// Offer `child` up to be killed, unless the run is already cancelled.
    ///
    /// `false` means the caller lost the race and owns a child nobody else
    /// will ever stop, so the caller has to.
    fn register(&self, child: &Arc<Mutex<Child>>) -> bool {
        // Taken before the flag is read, so a concurrent `cancel` either
        // already stored `true` (and this refuses) or blocks here and finds
        // the child (and kills it).
        let mut running = lock(&self.state.running);
        if self.is_cancelled() {
            return false;
        }
        *running = Some(Arc::clone(child));
        true
    }

    /// The pass is over; there is nothing left to kill.
    fn finished(&self) {
        *lock(&self.state.running) = None;
    }
}

/// One thing a pass was seen doing, in the fewest words it can be said in.
///
/// Facts, not prose. A pass produces a document, and the document is the
/// product; this is the sign of life that runs alongside it, so every variant
/// is something that fits on one line of a panel and is true without
/// interpretation. What is *not* here is as deliberate as what is: no tool
/// result, no assistant text, and no thought — only that thinking happened.
/// Rendering a model's reasoning back at the user is prose, and prose is the
/// thing this front end refuses to show.
///
/// ```
/// use warlock_tui::Activity;
///
/// let read = Activity::Tool {
///     name: "Read".to_owned(),
///     detail: Some("src/lib.rs".to_owned()),
/// };
/// let unlisted = Activity::Tool {
///     name: "WebFetch".to_owned(),
///     detail: None,
/// };
///
/// assert_ne!(read, unlisted);
/// assert_eq!(Activity::Thinking, Activity::Thinking);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Activity {
    /// The model called a tool.
    Tool {
        /// The tool's name exactly as the stream spells it: `Read`, `Bash`,
        /// `Grep`, or whatever the vendor adds next.
        name: String,
        /// The one argument worth putting on a line, or `None` for a tool
        /// nothing is known about.
        ///
        /// One, not some, and chosen per tool rather than taken from whatever
        /// the call happened to carry: the alternative is an arbitrary input
        /// dict on somebody's screen. A tool that is not on the whitelist is
        /// shown by name alone, which is honest and short, and adding an entry
        /// is a one-line change to be made when a real stream shows a tool
        /// that matters.
        detail: Option<String>,
    },
    /// The model thought.
    ///
    /// Carries nothing on purpose — see the type's docs. A stretch of thinking
    /// is visible as a stretch of *this*, which is all a reader needs to know
    /// the run is alive.
    Thinking,
    /// What the pass cost, in US dollars, as the pass itself reported it.
    ///
    /// The one number the transport keeps out of the run's own accounting, and
    /// the reason it rides this port rather than the response: it is a fact
    /// about the *pass*, not part of the document, and it arrives at the end of
    /// the stream like everything else here.
    Cost {
        /// The cost of this one pass. Whatever the pass said it was — nothing
        /// here checks it, converts it or adds it up.
        usd: f64,
    },
}

/// Where a pass says what it is doing, for whoever is not running it.
///
/// [`Cancel`]'s twin, pointing the other way: that one carries a stop *into* a
/// pass from the thread drawing the screen, and this one carries facts *out* of
/// a pass to it. Same construction, for the same reasons — one shared thing
/// behind an [`Arc`], so cloning is a refcount bump and every clone reports to
/// the same place, and [`Send`] + [`Sync`] because the pass runs on the pact
/// worker's thread and the listener is somewhere else entirely.
///
/// The shared thing is a function rather than a channel, so that this file
/// keeps its one job. A pact worker already owns a channel to the event loop;
/// handing it a closure lets it forward an activity as one of its own events
/// over the route it already has, instead of the transport inventing a second
/// one and the loop growing a second thing to poll.
///
/// The default is a handle nobody listens to, which is what an agent gets when
/// no caller attached one, so reporting is a no-op rather than a `None` every
/// call site has to remember to check.
///
/// ```
/// use std::sync::mpsc;
///
/// use warlock_tui::{Activities, Activity};
///
/// let (sender, received) = mpsc::channel();
/// let activities = Activities::new(move |activity| {
///     let _ = sender.send(activity);
/// });
///
/// activities.report(Activity::Thinking);
///
/// assert_eq!(received.recv(), Ok(Activity::Thinking));
///
/// // And one nobody listens to swallows whatever it is told.
/// Activities::none().report(Activity::Thinking);
/// ```
#[derive(Clone)]
pub struct Activities {
    /// Shared with every clone; the point of the type.
    sink: Arc<dyn Fn(Activity) + Send + Sync>,
}

impl Activities {
    /// A handle that hands every activity to `sink`.
    ///
    /// `sink` is called on whichever thread the pass is running on, in the
    /// order the stream produced things, and it is called while the pass is
    /// still going — that being the whole point. So it should be short: send
    /// it somewhere and return.
    #[must_use]
    pub fn new(sink: impl Fn(Activity) + Send + Sync + 'static) -> Self {
        Self {
            sink: Arc::new(sink),
        }
    }

    /// A handle nobody listens to.
    ///
    /// What an agent has until a caller attaches one with
    /// [`with_activities`](ClaudeAgent::with_activities): reporting to it does
    /// nothing, costs a call through a pointer, and changes no behaviour of the
    /// pass at all.
    #[must_use]
    pub fn none() -> Self {
        Self::new(|_| {})
    }

    /// Say that the pass did `activity`.
    ///
    /// Safe to call from any thread and from any clone, and never fails: a
    /// listener that has gone away is not a reason to fail a model pass, so
    /// there is nothing here for a caller to handle.
    pub fn report(&self, activity: Activity) {
        (self.sink)(activity);
    }
}

impl Default for Activities {
    fn default() -> Self {
        Self::none()
    }
}

impl fmt::Debug for Activities {
    /// A closure has nothing to print, and a handle is not distinguishable from
    /// another by looking at it; what a test failure needs from this is the
    /// name.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Activities").finish_non_exhaustive()
    }
}

/// An [`Agent`] that runs the `claude` CLI as a child process.
///
/// Owns the child, its stdin, its stdout, its stderr, its exit status and its
/// clock, and gives the engine back nothing but an [`AgentResponse`] or an
/// [`AgentError`] — no process type crosses the seam in either direction.
///
/// The program, its arguments and the timeout are all fields rather than
/// constants baked into the call, which is what makes this testable: a test
/// points it at a stand-in that exits non-zero, or writes nothing, or sleeps
/// forever, and every failure path is exercised on a machine with no `claude`
/// installed. The defaults are the real thing: `claude`, print mode, and
/// [`INVOCATION_TIMEOUT`].
///
/// ```no_run
/// use warlock_engine::{Agent, AgentRequest};
/// use warlock_tui::ClaudeAgent;
///
/// // Runs a real `claude`, so this example is not executed by the test suite.
/// let response = ClaudeAgent::new().run(&AgentRequest::new("say hello", "."))?;
///
/// println!("{}", response.text());
/// # Ok::<(), warlock_engine::AgentError>(())
/// ```
#[derive(Debug, Clone)]
pub struct ClaudeAgent {
    /// The command to run: `claude`, or whatever a test points it at.
    program: OsString,
    /// The arguments it is run with, before any prompt — which never becomes
    /// an argument, because it goes in on stdin.
    args: Vec<OsString>,
    /// How long a single invocation gets before it is killed.
    timeout: Duration,
    /// Whoever is allowed to say stop. Its own handle by default, which nobody
    /// else holds and so nothing ever cancels.
    cancel: Cancel,
    /// Where the pass says what it is doing. A handle nobody listens to by
    /// default, so an agent no caller wired up reports into nothing.
    activities: Activities,
}

impl ClaudeAgent {
    /// An agent that runs `claude --print --output-format stream-json
    /// --verbose` with the five-minute [`INVOCATION_TIMEOUT`].
    ///
    /// ```
    /// use warlock_tui::{ClaudeAgent, INVOCATION_TIMEOUT};
    ///
    /// assert_eq!(ClaudeAgent::new().timeout(), INVOCATION_TIMEOUT);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            program: OsString::from(PROGRAM),
            args: ARGS.iter().map(OsString::from).collect(),
            timeout: INVOCATION_TIMEOUT,
            cancel: Cancel::new(),
            activities: Activities::none(),
        }
    }

    /// The same agent, running `program` instead of `claude`.
    ///
    /// For tests: a stand-in that fails in a chosen way is how the failure
    /// paths are covered without installing anything.
    #[must_use]
    pub fn with_program(mut self, program: impl Into<OsString>) -> Self {
        self.program = program.into();
        self
    }

    /// The same agent, passing `args` instead of the default arguments.
    #[must_use]
    pub fn with_args<A: Into<OsString>>(mut self, args: impl IntoIterator<Item = A>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// The same agent, giving each invocation `timeout` instead of
    /// [`INVOCATION_TIMEOUT`].
    ///
    /// For tests, which cannot afford to wait five minutes to prove that
    /// waiting stops.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The same agent, answering to `cancel`.
    ///
    /// The caller keeps a clone: that is how a pass running on a worker thread
    /// is stopped from the thread reading the keyboard. Without this, an agent
    /// still has a handle — its own, which nobody else holds, so nothing can
    /// ever cancel it.
    ///
    /// ```
    /// use warlock_tui::{Cancel, ClaudeAgent};
    ///
    /// let cancel = Cancel::new();
    /// let agent = ClaudeAgent::new().with_cancel(cancel.clone());
    ///
    /// // Nothing has been asked to stop yet.
    /// assert!(!cancel.is_cancelled());
    /// # let _ = agent;
    /// ```
    #[must_use]
    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = cancel;
        self
    }

    /// The same agent, reporting what each pass does to `activities`.
    ///
    /// The mirror of [`with_cancel`](ClaudeAgent::with_cancel), and used the
    /// same way: the caller keeps a clone, and what the pass does on the worker
    /// thread reaches the thread drawing the screen. Without this an agent
    /// still has a handle — one nobody listens to, so a pass runs exactly as it
    /// did before and reporting costs nothing.
    ///
    /// ```
    /// use std::sync::mpsc;
    ///
    /// use warlock_tui::{Activities, ClaudeAgent};
    ///
    /// let (sender, received) = mpsc::channel();
    /// let agent = ClaudeAgent::new().with_activities(Activities::new(move |activity| {
    ///     let _ = sender.send(activity);
    /// }));
    ///
    /// // Nothing has run, so nothing has been reported.
    /// assert!(received.try_recv().is_err());
    /// # let _ = agent;
    /// ```
    #[must_use]
    pub fn with_activities(mut self, activities: Activities) -> Self {
        self.activities = activities;
        self
    }

    /// The command this agent runs.
    #[must_use]
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// The arguments it is run with, before any prompt.
    #[must_use]
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// Where this agent reports what a pass is doing.
    #[must_use]
    pub fn activities(&self) -> &Activities {
        &self.activities
    }

    /// How long one invocation is given before it is killed.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Start the child with all three streams piped, in the request's
    /// directory.
    fn spawn(&self, request: &AgentRequest) -> Result<Child, AgentError> {
        Command::new(&self.program)
            .args(&self.args)
            .current_dir(request.directory())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| self.spawn_error(error, request))
    }

    /// Which [`AgentError`] a failed spawn is.
    ///
    /// `NotFound` is ambiguous at the syscall: the operating system says the
    /// same thing whether the *program* is missing or the working directory
    /// is. Only the first deserves the message naming `claude`, so the
    /// directory is checked before the blame is assigned, and a missing
    /// directory goes back as ordinary I/O.
    fn spawn_error(&self, error: io::Error, request: &AgentRequest) -> AgentError {
        if error.kind() == io::ErrorKind::NotFound && request.directory().is_dir() {
            AgentError::NotFound {
                program: self.program.to_string_lossy().into_owned(),
            }
        } else {
            AgentError::Io { source: error }
        }
    }
}

impl Default for ClaudeAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl Agent for ClaudeAgent {
    fn run(&self, request: &AgentRequest) -> Result<AgentResponse, AgentError> {
        // Asked before anything is started: a pass begun after the cancel is a
        // process the user already said they did not want, and the cheapest
        // way not to kill it is not to spawn it.
        if self.cancel.is_cancelled() {
            return Err(cancelled());
        }

        let mut child = self.spawn(request)?;

        // Configured as pipes just above, so all three are `Some`; taking them
        // hands each stream to the thread that owns it for the rest of the
        // call, and leaves the `Child` itself holding nothing but the process.
        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        // The prompt goes out on its own thread, and the handle is dropped the
        // moment it is written so the child sees EOF. Errors are deliberately
        // dropped: a child that exits without reading its stdin — every
        // stand-in below, and any real `claude` that rejects the run early —
        // breaks the pipe, and that is not the failure worth reporting. Its
        // exit status and stderr are.
        let prompt = request.prompt().to_owned();
        let writer = thread::spawn(move || {
            let mut stdin = stdin;
            let _ = stdin.write_all(prompt.as_bytes());
            let _ = stdin.flush();
        });

        // Read concurrently with the wait, or a child that writes more than a
        // pipeful blocks forever and so does this call. Stdout goes through
        // [`read`], which reports as it reads; stderr is only ever looked at
        // once the pass is over, so it is drained whole.
        let out = read(stdout, self.activities.clone());
        let err = drain(stderr);

        let child = Arc::new(Mutex::new(child));
        // Registered before the wait and cleared after it, so a cancel arriving
        // now reaches this child and one arriving later reaches nothing rather
        // than a pass that has moved on.
        if !self.cancel.register(&child) {
            kill_and_reap(&child);
            return Err(cancelled());
        }
        let (waiter, exited) = watch(&child);

        let outcome = match exited.recv_timeout(self.timeout) {
            Ok(Ok(status)) if !status.success() && self.cancel.is_cancelled() => {
                // The exit a cancel caused. Its output is not read, for the
                // same reason the timeout arm below does not read it: the
                // child is gone, but a grandchild the kill did not reach can
                // still hold the pipes open, and a join here would sit out
                // whatever that grandchild is doing — the very wait the cancel
                // was pressed to end. There is nothing in that output worth
                // waiting for anyway, because a pass killed mid-word has no
                // document to judge, and the rewrite below is what this
                // failure would come back as regardless.
                let _ = waiter.join();
                Err(cancelled())
            }
            Ok(Ok(status)) => {
                // Exited on its own — including the pass that beat a cancel by
                // a hair, which is why success is not read as a cancel above.
                // The child is gone, so every pipe it held is closed and each
                // join returns: the readers with what they read, the writer
                // with nothing.
                let _ = waiter.join();
                let _ = writer.join();
                // Not `?`: every way out of this wait has to reach the
                // clearing below, and an early return would leave a finished
                // child registered for a later cancel to find.
                match (collect(out), collect(err)) {
                    (Ok(document), Ok(stderr)) => judge(status, document, &stderr),
                    (Err(error), _) | (_, Err(error)) => Err(error),
                }
            }
            // The waiter could not tell whether it had exited; treat it like
            // any other I/O failure, but not before cleaning up after it.
            Ok(Err(error)) => {
                kill_and_reap(&child);
                let _ = waiter.join();
                Err(AgentError::Io { source: error })
            }
            Err(RecvTimeoutError::Timeout) => {
                // Killed *and* reaped: an abandoned child is an orphan holding
                // a subscription's worth of tokens, and one waited on by
                // nobody is a zombie in the process table.
                kill_and_reap(&child);
                // The waiter sees the exit it caused within one poll, so this
                // join is bounded. The reader and writer threads deliberately
                // are not joined: their pipes could still be held open by a
                // grandchild the kill did not reach, and this call has already
                // decided what it returns. They end on their own when the last
                // writer of each pipe closes it.
                let _ = waiter.join();
                Err(AgentError::TimedOut {
                    after: self.timeout,
                })
            }
            // Unreachable in practice: the waiter sends before it returns.
            Err(RecvTimeoutError::Disconnected) => {
                kill_and_reap(&child);
                Err(AgentError::Io {
                    source: io::Error::other("the process waiter stopped without an exit status"),
                })
            }
        };
        self.cancel.finished();

        match outcome {
            // A cancel kills the child, so the wait above ends the ordinary
            // way and judges a signalled exit — which would go back as
            // [`AgentError::Failed`], blaming the model for a run the user
            // stopped. Only a failed outcome is rewritten: a pass that beat
            // the cancel by a hair produced a real document, and throwing it
            // away would be a lie in the other direction.
            Err(_) if self.cancel.is_cancelled() => Err(cancelled()),
            outcome => outcome,
        }
    }
}

/// What a cancelled pass comes back as.
///
/// No variant of its own, because [`AgentError`] is the engine's vocabulary and
/// the engine has no opinion about people pressing Esc: a run that was
/// interrupted before it could produce a document is exactly
/// [`AgentError::Io`], and [`ErrorKind::Interrupted`](io::ErrorKind::Interrupted)
/// is what that is called. The message is what a footer shows, so it says who
/// stopped it rather than what a signal was.
fn cancelled() -> AgentError {
    AgentError::Io {
        source: io::Error::new(
            io::ErrorKind::Interrupted,
            "the model pass was cancelled before it finished",
        ),
    }
}

/// What an exit status, the document its stdout carried and its stderr mean in
/// the engine's vocabulary.
///
/// Order matters: a non-zero exit is reported as a failure even if it printed
/// something, and silence is only [`AgentError::EmptyOutput`] when the run
/// itself went fine. Whitespace counts as silence — a document of blank lines
/// is no document, and so is a stream that never carried one.
fn judge(status: ExitStatus, document: String, stderr: &[u8]) -> Result<AgentResponse, AgentError> {
    if !status.success() {
        return Err(AgentError::Failed {
            code: status.code(),
            stderr: String::from_utf8_lossy(stderr).into_owned(),
        });
    }
    if document.trim().is_empty() {
        return Err(AgentError::EmptyOutput);
    }
    // Moved, not copied or re-encoded: what the result line said is what the
    // engine gets, byte for byte.
    Ok(AgentResponse::new(document))
}

/// Reading the stream, and nothing to do with running anything.
///
/// Everything in here is a pure function over [`Value`]: a line of text goes
/// in, a [`Reading`] comes out, and no part of it can spawn, block, fail or
/// panic. That is deliberate and it is the whole design of this half — the
/// process plumbing above has to be tested with stand-in programs and real
/// pipes, while the schema of a vendor's JSON is tested from string literals
/// in microseconds, and mixing the two would mean testing the second the
/// expensive way forever.
mod stream {
    use serde_json::Value;

    use super::Activity;

    /// The tools whose one interesting argument is known, and which key holds
    /// it.
    ///
    /// Six entries, copied verbatim from the same table in `forman.spawn`'s
    /// `describe_activity`, and an abbreviation of nothing: a tool that is not
    /// here is reported by name alone. The alternative — printing whatever the
    /// call happened to carry — puts an arbitrary input dict on somebody's
    /// screen, which is exactly the prose this front end refuses to show.
    /// Adding a row is a one-line change, to be made when a real stream turns
    /// up a tool that matters, not in advance of one.
    ///
    /// A list rather than a map because six pairs scanned linearly is faster
    /// than hashing the name, and this reads as the table it is.
    const DETAILS: [(&str, &str); 6] = [
        ("Read", "file_path"),
        ("Edit", "file_path"),
        ("Write", "file_path"),
        ("Glob", "pattern"),
        ("Grep", "pattern"),
        ("Bash", "command"),
    ];

    /// Everything one line of the stream had to say.
    ///
    /// A line is not one thing: an assistant message carries a list of content
    /// blocks and so can be several activities at once, and the final line
    /// carries both what the pass cost and the document it produced. So the
    /// parse returns what it found rather than an enum of what it was, and a
    /// line that meant nothing to us returns the default — empty, which is not
    /// an error.
    #[derive(Debug, Default, PartialEq)]
    pub(super) struct Reading {
        /// What the line said the pass was doing, in the order the line said
        /// it.
        pub(super) activities: Vec<Activity>,
        /// The document, if this was the line that carried it.
        pub(super) text: Option<String>,
    }

    /// What one line of `--output-format stream-json` means.
    ///
    /// Every level of every line is treated as optional, and nothing here can
    /// fail: a line that is not JSON, or is JSON of a shape this code has
    /// never seen, reads as [`Reading::default`] — no activities, no text, no
    /// error. That is a deliberate posture rather than laziness. The stream's
    /// schema belongs to a vendor who will add to it, and the cost of the two
    /// mistakes is not symmetric: missing an activity costs a line of a panel
    /// nobody was promised, while failing a pass over an unrecognised field
    /// throws away minutes of work and a written document over a *decoration*.
    /// Hence [`Value`] and `.get(...).and_then(...)` throughout, and no
    /// `Deserialize` struct that would turn tomorrow's extra field into
    /// today's hard error.
    ///
    /// Pure, and takes a `&str`: everything about reading the stream is
    /// testable from a string literal, with no child process anywhere near it.
    pub(super) fn read_line(line: &str) -> Reading {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // Not JSON at all. `claude` is entitled to print a warning, and a
            // warning is not a reason to fail a pass.
            return Reading::default();
        };
        match value.get("type").and_then(Value::as_str) {
            Some("assistant") => Reading {
                activities: read_activities(&value),
                text: None,
            },
            Some("result") => read_result(&value),
            // `system`, `user` — which is where tool results come back — and
            // whatever is added next: all of it is somebody else's business.
            _ => Reading::default(),
        }
    }

    /// The activities in an assistant message's content blocks.
    ///
    /// Two of the block types say something worth showing and the rest say
    /// nothing: a `tool_result` is the output of a command, which can be a
    /// megabyte of file, and a `text` block is the model's prose. Neither is a
    /// sign of life, both are unbounded, and so both come back as no activity
    /// at all.
    fn read_activities(value: &Value) -> Vec<Activity> {
        value
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .map(|blocks| blocks.iter().filter_map(read_block).collect())
            .unwrap_or_default()
    }

    /// What one content block is doing, if it is doing anything.
    fn read_block(block: &Value) -> Option<Activity> {
        match block.get("type").and_then(Value::as_str)? {
            "tool_use" => {
                let name = block.get("name").and_then(Value::as_str)?;
                Some(Activity::Tool {
                    name: name.to_owned(),
                    detail: read_detail(block, name),
                })
            }
            // The bare fact, never the thought: see [`Activity::Thinking`].
            "thinking" => Some(Activity::Thinking),
            _ => None,
        }
    }

    /// The one argument of `name`'s call worth putting on a line, if there is
    /// one.
    ///
    /// `None` three ways, all of them ordinary: the tool is not in
    /// [`DETAILS`], the call did not carry the key the table names, or what it
    /// carried was not a string. A number or an object where a path was
    /// expected is a tool whose shape has changed, and the honest answer to
    /// that is the tool's name by itself.
    fn read_detail(block: &Value, name: &str) -> Option<String> {
        let (_, key) = DETAILS.iter().find(|(tool, _)| *tool == name)?;
        block
            .get("input")
            .and_then(|input| input.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    }

    /// The document and the cost the final line carries.
    ///
    /// The text is the result line's own `result` field rather than the
    /// assistant `text` blocks accumulated along the way, because that field
    /// is *literally* what `--print` prints: same run, same field, so the
    /// document is byte identical by construction instead of by a reassembly
    /// this file would have to get right — joining blocks with the separator
    /// the vendor happens to use, across as many assistant messages as the
    /// pass took, minus the ones that were only a tool call. Nothing is gained
    /// by rebuilding what the stream already hands over whole.
    fn read_result(value: &Value) -> Reading {
        let cost = value
            .get("total_cost_usd")
            .and_then(Value::as_f64)
            .map(|usd| Activity::Cost { usd });
        Reading {
            activities: cost.into_iter().collect(),
            text: value
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    }
}

/// Read the stream `source` is writing, a line at a time, on a thread of its
/// own; report what each line says as it says it, and keep the document.
///
/// Incremental on purpose, and the reason this is not [`drain`]: reading to EOF
/// and parsing afterwards would produce exactly the same document and exactly
/// the same activities, all of them arriving after the only moment anybody
/// wanted them. So each line is parsed and reported the moment its newline
/// lands, which is the difference between a panel that shows a pass happening
/// and one that shows a pass that happened.
///
/// The document is the `result` field of whichever result line arrived last,
/// not the assistant `text` blocks accumulated along the way — see
/// `stream::read_result` for why that is byte identical to `--print` by
/// construction. A stream that never carried one leaves this empty, which
/// [`judge`] reads as [`AgentError::EmptyOutput`], the same answer a silent
/// child got before.
///
/// Lines are split on bytes and converted lossily rather than read through
/// [`BufRead::lines`](io::BufRead::lines), which fails a whole read on invalid
/// UTF-8: a stray byte in a model's markdown is not worth failing a pass over,
/// and the engine's vocabulary has no variant for it. Nothing here bounds a
/// line's length — a whole document arrives as one — so the buffer is reused
/// rather than grown per line, and it is cleared as it goes.
fn read<R: Read + Send + 'static>(
    source: R,
    activities: Activities,
) -> JoinHandle<io::Result<String>> {
    thread::spawn(move || {
        let mut source = io::BufReader::new(source);
        let mut line = Vec::new();
        let mut document = String::new();
        loop {
            line.clear();
            if source.read_until(b'\n', &mut line)? == 0 {
                return Ok(document);
            }
            let reading = stream::read_line(&String::from_utf8_lossy(&line));
            for activity in reading.activities {
                activities.report(activity);
            }
            if let Some(text) = reading.text {
                document = text;
            }
        }
    })
}

/// Read everything `source` produces, on a thread of its own.
///
/// Stderr's reader, now that stdout has one of its own: nothing looks at stderr
/// until the pass is over and its exit status is known, so there is nothing to
/// be gained by reading it in pieces.
fn drain<R: Read + Send + 'static>(source: R) -> JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut source = source;
        let mut buffer = Vec::new();
        source.read_to_end(&mut buffer)?;
        Ok(buffer)
    })
}

/// Wait for what [`read`] or [`drain`] read.
///
/// A panicked reader is a bug rather than a transport failure, but it is not
/// worth panicking the caller over: it comes back as I/O like anything else
/// that stopped the pass being read.
fn collect<T>(handle: JoinHandle<io::Result<T>>) -> Result<T, AgentError> {
    match handle.join() {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(source)) => Err(AgentError::Io { source }),
        Err(_) => Err(AgentError::Io {
            source: io::Error::other("the thread reading the model pass's output panicked"),
        }),
    }
}

/// Watch `child` from a thread, reporting its exit over a channel.
///
/// Polls rather than blocking in [`Child::wait`] precisely so the lock is free
/// almost all the time: whoever is holding the clock has to be able to take it
/// and kill.
fn watch(child: &Arc<Mutex<Child>>) -> (JoinHandle<()>, mpsc::Receiver<io::Result<ExitStatus>>) {
    let (sender, receiver) = mpsc::channel();
    let child = Arc::clone(child);
    let waiter = thread::spawn(move || {
        loop {
            // Scoped so the guard is dropped before the sleep, not held across
            // it.
            let polled = lock(&child).try_wait();
            match polled {
                Ok(Some(status)) => {
                    let _ = sender.send(Ok(status));
                    return;
                }
                Ok(None) => thread::sleep(POLL_INTERVAL),
                Err(error) => {
                    let _ = sender.send(Err(error));
                    return;
                }
            }
        }
    });
    (waiter, receiver)
}

/// Stop `child` and collect it, so nothing of it outlives the call.
///
/// Both halves matter: [`kill`](Child::kill) ends the process,
/// [`wait`](Child::wait) collects the status the kernel is holding for it.
/// Errors are dropped because there is nothing left to do about either — a
/// child that has already exited reports one, and that is the good case.
fn kill_and_reap(child: &Arc<Mutex<Child>>) {
    let mut child = lock(child);
    let _ = child.kill();
    let _ = child.wait();
}

/// The lock, poisoned or not.
///
/// Poisoning means some thread panicked while holding the child; the child is
/// still a child and still needs killing and reaping, so recovering the guard
/// is strictly better here than panicking a second time.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use warlock_engine::{Agent, AgentError, AgentRequest};

    use super::stream;
    use super::{Activities, Activity, Cancel, ClaudeAgent, INVOCATION_TIMEOUT};

    /// A name no directory on `PATH` can hold, so the lookup is guaranteed to
    /// fail the way a machine without `claude` fails.
    const NOT_A_PROGRAM: &str = "warlock-test-no-such-program-8f3a1c";

    /// `agent`'s arguments as plain strings, which is the shape a test can say
    /// out loud.
    fn args(agent: &ClaudeAgent) -> Vec<String> {
        agent
            .args()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_defaults_are_the_real_thing() {
        let agent = ClaudeAgent::new();

        assert_eq!(agent.program(), "claude");
        assert_eq!(agent.timeout(), INVOCATION_TIMEOUT);
        assert_eq!(
            INVOCATION_TIMEOUT.as_secs(),
            300,
            "five minutes, per invocation"
        );
        // Exactly this, in this order: print mode, the streaming output format,
        // and the `--verbose` the CLI insists on before it will stream at all.
        assert_eq!(
            args(&agent),
            ["--print", "--output-format", "stream-json", "--verbose"]
        );
        assert_eq!(args(&ClaudeAgent::default()), args(&agent));
        assert_eq!(ClaudeAgent::default().timeout(), agent.timeout());
    }

    #[test]
    fn the_arguments_are_a_field_a_caller_can_replace_outright() {
        // Not appended to and not merged with: what a caller asks for is what
        // is run, which is how every stand-in below works and how a later
        // slice changes the invocation without touching this file.
        let agent = ClaudeAgent::new().with_args(["-c", "echo hello"]);

        assert_eq!(args(&agent), ["-c", "echo hello"]);
        assert!(args(&ClaudeAgent::new().with_args(Vec::<&str>::new())).is_empty());
    }

    #[test]
    fn a_missing_binary_is_reported_by_name_not_as_an_errno() {
        // No `claude` needed to test the no-`claude` case, which is the point:
        // this is the state of every machine that has never installed it.
        let agent = ClaudeAgent::new().with_program(NOT_A_PROGRAM);

        let error = agent
            .run(&AgentRequest::new("anything", "."))
            .expect_err("nothing by that name can be on PATH");

        match error {
            AgentError::NotFound { program } => assert_eq!(program, NOT_A_PROGRAM),
            other => panic!("expected a missing binary, got {other:?}"),
        }
    }

    #[test]
    fn a_cancel_handle_is_one_flag_shared_by_every_clone() {
        // The property the whole mechanism rests on: the thread that cancels
        // is never the thread that is running the pass.
        fn held_across_threads<T: Send + Sync + 'static>(_: &T) {}

        let cancel = Cancel::new();
        held_across_threads(&cancel);
        let watcher = cancel.clone();
        assert!(!cancel.is_cancelled());
        assert!(!Cancel::default().is_cancelled(), "and a fresh one is live");

        thread::spawn(move || {
            watcher.cancel();
            // Latched, not toggled, and saying it twice is not an error.
            watcher.cancel();
        })
        .join()
        .expect("the cancelling thread ran");

        assert!(cancel.is_cancelled());
    }

    #[test]
    fn a_port_nobody_listens_to_swallows_everything_reported_to_it() {
        // What an agent has until a caller attaches one: reporting is a no-op,
        // not a panic and not a failure, so the parsing side can report
        // unconditionally.
        for activities in [Activities::none(), Activities::default()] {
            activities.report(Activity::Thinking);
            activities.report(Activity::Tool {
                name: "Read".to_owned(),
                detail: Some("src/lib.rs".to_owned()),
            });
            activities.report(Activity::Cost { usd: 0.03 });
        }

        // And that is what an agent nobody wired up has.
        ClaudeAgent::new().activities().report(Activity::Thinking);
    }

    #[test]
    fn every_clone_of_a_port_reports_to_the_same_place() {
        // The property the whole port rests on, and [`Cancel`]'s in reverse:
        // the thread running the pass is never the thread listening to it.
        fn held_across_threads<T: Send + Sync + 'static>(_: &T) {}

        let (sender, received) = mpsc::channel();
        let activities = Activities::new(move |activity| {
            sender.send(activity).expect("the test is still listening");
        });
        held_across_threads(&activities);
        let passing = activities.clone();

        thread::spawn(move || {
            passing.report(Activity::Thinking);
            passing.report(Activity::Tool {
                name: "Bash".to_owned(),
                detail: Some("cargo test".to_owned()),
            });
            passing.report(Activity::Cost { usd: 0.25 });
        })
        .join()
        .expect("the reporting thread ran");

        // In the order they were reported, through the clone, from the other
        // thread.
        assert_eq!(received.recv(), Ok(Activity::Thinking));
        assert_eq!(
            received.recv(),
            Ok(Activity::Tool {
                name: "Bash".to_owned(),
                detail: Some("cargo test".to_owned()),
            })
        );
        assert_eq!(received.recv(), Ok(Activity::Cost { usd: 0.25 }));
        // The original still reports to the same place after the clone is gone.
        activities.report(Activity::Thinking);
        assert_eq!(received.recv(), Ok(Activity::Thinking));
    }

    #[test]
    fn an_attached_port_is_the_one_the_agent_reports_to() {
        let (sender, received) = mpsc::channel();
        let agent = ClaudeAgent::new().with_activities(Activities::new(move |activity| {
            let _ = sender.send(activity);
        }));

        agent.activities().report(Activity::Thinking);

        assert_eq!(received.recv(), Ok(Activity::Thinking));
        // Attaching one changes nothing else about the agent.
        assert_eq!(agent.program(), "claude");
        assert_eq!(agent.timeout(), INVOCATION_TIMEOUT);
    }

    /// One assistant line carrying `blocks` as its content, the shape a real
    /// stream uses.
    fn assistant(blocks: &str) -> String {
        format!(r#"{{"type":"assistant","message":{{"role":"assistant","content":[{blocks}]}}}}"#)
    }

    #[test]
    fn each_whitelisted_tool_carries_its_one_argument_and_the_rest_carry_none() {
        // The table verbatim, and the point of the last row: a tool nobody
        // wrote down is shown by name, not by dumping whatever its call
        // carried.
        let expected = [
            ("Read", r#"{"file_path":"src/lib.rs"}"#, Some("src/lib.rs")),
            (
                "Edit",
                r#"{"file_path":"src/main.rs"}"#,
                Some("src/main.rs"),
            ),
            (
                "Write",
                r#"{"file_path":"docs/plan.md"}"#,
                Some("docs/plan.md"),
            ),
            ("Glob", r#"{"pattern":"**/*.rs"}"#, Some("**/*.rs")),
            ("Grep", r#"{"pattern":"fn main"}"#, Some("fn main")),
            ("Bash", r#"{"command":"cargo test"}"#, Some("cargo test")),
            ("WebFetch", r#"{"url":"https://example.invalid"}"#, None),
        ];

        for (name, input, detail) in expected {
            let line = assistant(&format!(
                r#"{{"type":"tool_use","id":"toolu_1","name":"{name}","input":{input}}}"#
            ));

            let reading = stream::read_line(&line);

            assert_eq!(
                reading.activities,
                vec![Activity::Tool {
                    name: name.to_owned(),
                    detail: detail.map(str::to_owned),
                }],
                "one activity for {name}, with exactly the whitelisted detail"
            );
            assert_eq!(reading.text, None, "a tool call is not the document");
        }
    }

    #[test]
    fn a_whitelisted_tool_missing_its_argument_is_still_the_bare_name() {
        // Three ways the key is not there, none of them a reason to lose the
        // activity or to reach for some other key.
        for input in [r"{}", r#"{"offset":12}"#, r#"{"file_path":7}"#] {
            let line = assistant(&format!(
                r#"{{"type":"tool_use","name":"Read","input":{input}}}"#
            ));

            assert_eq!(
                stream::read_line(&line).activities,
                vec![Activity::Tool {
                    name: "Read".to_owned(),
                    detail: None,
                }],
                "Read with input {input}"
            );
        }

        // And a block with no `input` at all.
        assert_eq!(
            stream::read_line(&assistant(r#"{"type":"tool_use","name":"Bash"}"#)).activities,
            vec![Activity::Tool {
                name: "Bash".to_owned(),
                detail: None,
            }]
        );
    }

    #[test]
    fn a_thought_reaches_the_panel_as_the_fact_that_it_happened_and_nothing_else() {
        let secret = "the user's code is beyond saving and I shall say so gently";
        let line = assistant(&format!(
            r#"{{"type":"thinking","thinking":"{secret}","signature":"abc"}}"#
        ));

        let reading = stream::read_line(&line);

        assert_eq!(reading.activities, vec![Activity::Thinking]);
        // The whole point of the bare variant: there is nowhere for the text to
        // be, so it cannot be printed by accident later.
        assert!(
            !format!("{reading:?}").contains("beyond saving"),
            "no part of a thought survives the parse"
        );
    }

    #[test]
    fn tool_results_and_the_models_own_prose_are_not_activities() {
        let enormous = "x".repeat(200_000);
        let lines = [
            // A tool result comes back on a `user` line, which is not a line
            // type this reads at all...
            format!(
                r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_1","content":"{enormous}"}}]}}}}"#
            ),
            // ...and would still be nothing if it arrived on one that is.
            assistant(&format!(
                r#"{{"type":"tool_result","tool_use_id":"toolu_1","content":"{enormous}"}}"#
            )),
            // The model's prose is the document, not a sign of life.
            assistant(r#"{"type":"text","text":"Here is the summary you asked for."}"#),
        ];

        for line in lines {
            let reading = stream::read_line(&line);

            assert_eq!(
                reading,
                stream::Reading::default(),
                "nothing from {line:.60}"
            );
        }
    }

    #[test]
    fn a_line_this_code_does_not_understand_is_skipped_rather_than_fatal() {
        let lines = [
            "",
            "   ",
            "not json at all",
            "{",
            "[1, 2, 3]",
            "null",
            r#""a bare string""#,
            // JSON, well formed, and about something else entirely.
            r#"{"type":"system","subtype":"init","tools":["Read","Bash"]}"#,
            r#"{"type":"kraken","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
            r#"{"message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
            // The right type, with the levels below it missing or the wrong
            // shape.
            r#"{"type":"assistant"}"#,
            r#"{"type":"assistant","message":{"content":"not a list"}}"#,
            &assistant(r#"{"type":"tool_use"}"#),
            &assistant(r#"{"no":"type"}"#),
        ];

        for line in lines {
            assert_eq!(
                stream::read_line(line),
                stream::Reading::default(),
                "nothing, and no panic, from {line:?}"
            );
        }
    }

    #[test]
    fn the_final_line_carries_the_document_and_what_the_pass_cost() {
        let line = r##"{"type":"result","subtype":"success","is_error":false,"duration_ms":8123,"result":"# Warlock\n\nThe freshness ledger.\n","total_cost_usd":0.0342,"usage":{"input_tokens":11}}"##;

        let reading = stream::read_line(line);

        assert_eq!(reading.activities, vec![Activity::Cost { usd: 0.0342 }]);
        // Verbatim, including the trailing newline: this field is what
        // `--print` prints.
        assert_eq!(
            reading.text.as_deref(),
            Some("# Warlock\n\nThe freshness ledger.\n")
        );
    }

    #[test]
    fn a_result_line_missing_a_half_still_gives_up_the_other_one() {
        let costless = stream::read_line(r#"{"type":"result","result":"a document"}"#);
        assert_eq!(costless.activities, vec![]);
        assert_eq!(costless.text.as_deref(), Some("a document"));

        let textless = stream::read_line(r#"{"type":"result","total_cost_usd":1.5}"#);
        assert_eq!(textless.activities, vec![Activity::Cost { usd: 1.5 }]);
        assert_eq!(textless.text, None);

        // A cost that is not a number is no cost, not a failed pass.
        let nonsense = stream::read_line(r#"{"type":"result","total_cost_usd":"lots"}"#);
        assert_eq!(nonsense, stream::Reading::default());
    }

    #[test]
    fn one_line_of_several_blocks_is_several_activities_in_order() {
        // What a real assistant message looks like when the model thinks, says
        // something, then calls two tools.
        let line = assistant(concat!(
            r#"{"type":"thinking","thinking":"which file"},"#,
            r#"{"type":"text","text":"Let me look."},"#,
            r#"{"type":"tool_use","name":"Grep","input":{"pattern":"TODO","path":"src"}},"#,
            r#"{"type":"tool_use","name":"Read","input":{"file_path":"src/app.rs"}}"#
        ));

        let reading = stream::read_line(&line);

        assert_eq!(
            reading.activities,
            vec![
                Activity::Thinking,
                Activity::Tool {
                    name: "Grep".to_owned(),
                    // The whitelisted key, not the first key, and not both.
                    detail: Some("TODO".to_owned()),
                },
                Activity::Tool {
                    name: "Read".to_owned(),
                    detail: Some("src/app.rs".to_owned()),
                },
            ]
        );
        assert_eq!(reading.text, None);
    }

    #[test]
    fn cancelling_with_no_pass_running_is_a_no_op_that_still_latches() {
        // No child registered, so there is nothing to kill; the flag is the
        // whole effect, and it is the half that stops the *next* pass.
        let cancel = Cancel::new();

        cancel.cancel();

        assert!(cancel.is_cancelled());
    }

    /// Everything below runs a real child, and the stand-ins it runs are shell
    /// scripts, so the whole module is Unix-only. What is being tested — the
    /// pipes, the timeout, the kill — is not, but a portable stand-in would
    /// have to be a second binary to build, and that costs more than the
    /// coverage it adds.
    #[cfg(unix)]
    mod unix {
        use std::io::ErrorKind;
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        use std::{env, fs, process, thread};

        use warlock_engine::{Agent, AgentError, AgentRequest};

        use super::super::{Activities, Activity, Cancel, ClaudeAgent};

        /// How long a test waits for a child to announce itself before giving
        /// up and cancelling anyway. Generous, because it is only reached when
        /// something is already wrong; the wait itself ends as soon as the pid
        /// file appears.
        const AT_MOST: Duration = Duration::from_secs(5);

        /// The lines of a pass, in miniature: the session's opening line, a
        /// tool call, a thought and the model's own prose, then the result line
        /// carrying the document and what the pass cost.
        ///
        /// The stand-in every test that wants a *plausible* pass runs, so that
        /// what a real stream looks like is written down once. It is the shape
        /// of the thing, not a transcript: four lines rather than four hundred,
        /// and one of each kind that matters.
        const PASS: [&str; 4] = [
            r#"{"type":"system","subtype":"init","tools":["Read","Bash"]}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"a thought nobody is entitled to"},{"type":"text","text":"Here is the summary you asked for."}]}}"#,
            r##"{"type":"result","subtype":"success","result":"# module\n\nWhat it does.\n","total_cost_usd":0.0342}"##,
        ];

        /// The document [`PASS`]'s result line carries, spelled the way Rust
        /// spells it.
        const DOCUMENT: &str = "# module\n\nWhat it does.\n";

        /// What [`PASS`] reports, in order: the tool with its one whitelisted
        /// argument, the bare fact of the thought, and the cost. Not the
        /// thought's text, not the model's prose, and nothing from the `system`
        /// line.
        fn reported() -> Vec<Activity> {
            vec![
                Activity::Tool {
                    name: "Read".to_owned(),
                    detail: Some("src/lib.rs".to_owned()),
                },
                Activity::Thinking,
                Activity::Cost { usd: 0.0342 },
            ]
        }

        /// An agent whose `claude` is `sh -c script`.
        fn stand_in(script: &str) -> ClaudeAgent {
            ClaudeAgent::new()
                .with_program("/bin/sh")
                .with_args(["-c", script])
        }

        /// A shell script that prints `lines`, one per line, and exits.
        ///
        /// `printf '%s\n' a b c` repeats its format once per argument, so this
        /// is one process and no loop and every line arrives whole. Quoting is
        /// single quotes around JSON that contains none, which is a property of
        /// every canned line in this module and worth keeping.
        fn printing(lines: &[&str]) -> String {
            let arguments: Vec<String> = lines.iter().map(|line| format!("'{line}'")).collect();
            format!("printf '%s\\n' {}", arguments.join(" "))
        }

        /// The same agent, reporting into a channel this test can read.
        fn listening(agent: ClaudeAgent) -> (ClaudeAgent, mpsc::Receiver<Activity>) {
            let (sender, received) = mpsc::channel();
            let agent = agent.with_activities(Activities::new(move |activity| {
                let _ = sender.send(activity);
            }));
            (agent, received)
        }

        /// Everything reported before the sender went away, in order.
        fn drained(received: &mpsc::Receiver<Activity>) -> Vec<Activity> {
            received.try_iter().collect()
        }

        /// Whether `error` is how a cancelled pass comes back: interrupted I/O
        /// in the engine's vocabulary, and not a model that refused.
        fn is_cancelled(error: &AgentError) -> bool {
            matches!(error, AgentError::Io { source } if source.kind() == ErrorKind::Interrupted)
        }

        /// The pid a stand-in wrote, once it has written one.
        ///
        /// `None` while the file is missing or still empty, which is how a
        /// test waits for a child to be genuinely running rather than guessing
        /// at a sleep.
        fn pid(path: &Path) -> Option<String> {
            let text = fs::read_to_string(path).ok()?;
            let pid = text.trim().to_owned();
            (!pid.is_empty()).then_some(pid)
        }

        /// A directory of this test's own, removed at the end of the test that
        /// made it. Hand-rolled rather than pulled in as a dependency: this
        /// crate's manifest gains nothing for this ticket.
        fn scratch(name: &str) -> PathBuf {
            static NEXT: AtomicUsize = AtomicUsize::new(0);

            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let directory =
                env::temp_dir().join(format!("warlock-claude-{}-{name}-{unique}", process::id()));
            fs::create_dir_all(&directory).expect("a scratch directory under the temp directory");
            directory
        }

        /// Best effort: a leftover under `/tmp` is untidy, not a test failure.
        fn clean_up(directory: &Path) {
            let _ = fs::remove_dir_all(directory);
        }

        #[test]
        fn a_clean_run_comes_back_as_the_document_the_stream_carried() {
            // `cat` is still the smallest possible model: it answers with the
            // prompt it was given, which proves the prompt reached stdin *and*
            // that stdin was closed — without EOF, `cat` would never return.
            // What it is given is now the result line of a stream, so the same
            // test also shows the parse working on bytes out of a real pipe
            // rather than on a string literal.
            let (agent, received) = listening(
                ClaudeAgent::new()
                    .with_program("/bin/cat")
                    .with_args(Vec::<&str>::new()),
            );

            let response = agent
                .run(&AgentRequest::new(format!("{}\n", PASS[3]), "."))
                .expect("cat exits cleanly and prints what it was given");

            assert_eq!(response.text(), DOCUMENT);
            assert_eq!(drained(&received), vec![Activity::Cost { usd: 0.0342 }]);
        }

        #[test]
        fn a_whole_pass_reports_what_it_did_and_returns_its_document() {
            let (agent, received) = listening(stand_in(&printing(&PASS)));

            let response = agent
                .run(&AgentRequest::new("anything", "."))
                .expect("the canned pass exits cleanly and prints a document");

            // Byte for byte the result line's own field, newlines and all.
            assert_eq!(response.text(), DOCUMENT);
            let activities = drained(&received);
            assert_eq!(activities, reported());
            // Said once more, because it is the promise the port is for: none
            // of the thought and none of the prose came with it.
            let seen = format!("{activities:?}");
            assert!(!seen.contains("entitled"), "{seen}");
            assert!(!seen.contains("summary"), "{seen}");
        }

        #[test]
        fn an_activity_reaches_the_port_while_the_pass_is_still_running() {
            // The whole reason for reading a line at a time: the tool call is
            // reported, and only then does the child get around to finishing.
            // A drain-to-EOF reader passes every other test in this module and
            // fails this one.
            let script = format!(
                "{}; sleep 1; {}",
                printing(&PASS[..2]),
                printing(&PASS[3..])
            );
            let (agent, received) = listening(stand_in(&script));

            let started = Instant::now();
            let pass = thread::spawn(move || agent.run(&AgentRequest::new("anything", ".")));
            let first = received
                .recv_timeout(AT_MOST)
                .expect("the tool call is reported as it happens");
            let reported_after = started.elapsed();
            let response = pass
                .join()
                .expect("the pass ran")
                .expect("the canned pass exits cleanly");
            let finished_after = started.elapsed();

            assert_eq!(first, reported()[0]);
            assert!(
                reported_after + Duration::from_millis(300) < finished_after,
                "the activity arrived at {reported_after:?} and the pass ended at \
                 {finished_after:?}: that is not streaming"
            );
            assert_eq!(response.text(), DOCUMENT);
        }

        #[test]
        fn garbage_in_the_stream_costs_neither_the_document_nor_an_activity() {
            // A warning on stdout, a half-written line, and an event from a
            // future version of the CLI. None of it is a reason to throw away
            // minutes of work and a written document.
            let lines = [
                "Warning: something the CLI felt like mentioning",
                PASS[1],
                "{not json",
                r#"{"type":"kraken","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
                PASS[2],
                "",
                PASS[3],
            ];
            let (agent, received) = listening(stand_in(&printing(&lines)));

            let response = agent
                .run(&AgentRequest::new("anything", "."))
                .expect("a stream with junk in it still produced a document");

            assert_eq!(response.text(), DOCUMENT);
            assert_eq!(drained(&received), reported());
        }

        #[test]
        fn the_pass_runs_in_the_directory_the_request_names() {
            let directory = scratch("cwd");
            fs::write(directory.join("marker.txt"), "here").expect("a file to look for");

            // `ls`, wrapped in the result line a pass would have wrapped it in.
            let response =
                stand_in(r#"printf '{"type":"result","result":"%s"}\n' "$(ls | tr '\n' ' ')""#)
                    .run(&AgentRequest::new("ignored", &directory))
                    .expect("ls exits cleanly and prints a name");

            assert!(
                response.text().contains("marker.txt"),
                "the child ran somewhere else: {}",
                response.text()
            );
            clean_up(&directory);
        }

        #[test]
        fn a_non_zero_exit_carries_its_status_and_its_stderr() {
            let error = stand_in("echo boom >&2; exit 3")
                .run(&AgentRequest::new("anything", "."))
                .expect_err("this stand-in refuses");

            match error {
                AgentError::Failed { code, stderr } => {
                    assert_eq!(code, Some(3));
                    assert_eq!(stderr.trim(), "boom", "stderr is captured, not dropped");
                }
                other => panic!("expected a non-zero exit, got {other:?}"),
            }
        }

        #[test]
        fn a_clean_run_that_says_nothing_is_empty_output() {
            // Four ways to say nothing: no output at all, blank lines, a whole
            // stream that never carried a result line, and a result line whose
            // document is whitespace. The last two are new shapes of the same
            // old answer — a document of blank lines is no document, and so is
            // a pass that produced none.
            let scripts = [
                "exit 0".to_owned(),
                "printf '\\n  \\n'".to_owned(),
                printing(&PASS[..3]),
                printing(&[r#"{"type":"result","result":"  \n\t"}"#]),
            ];

            for script in scripts {
                let error = stand_in(&script)
                    .run(&AgentRequest::new("anything", "."))
                    .expect_err("there is no document in silence");

                assert!(
                    matches!(error, AgentError::EmptyOutput),
                    "`{script}` gave {error:?}"
                );
            }
        }

        #[test]
        fn a_missing_directory_is_io_rather_than_a_missing_binary() {
            // The syscall says `NotFound` for both; only one of them deserves
            // the message telling the user to install `claude`.
            let error = stand_in("true")
                .run(&AgentRequest::new("anything", "/warlock/no/such/directory"))
                .expect_err("nothing can run in a directory that is not there");

            assert!(matches!(error, AgentError::Io { .. }), "{error:?}");
        }

        #[test]
        fn a_big_prompt_and_a_chatty_child_do_not_deadlock() {
            // Both directions past a pipe buffer at once: the prompt is bigger
            // than one, and so is the stream. Reading on a thread is what makes
            // this return at all, and reading by line rather than to EOF must
            // not have quietly reintroduced the block — a reader that stopped
            // consuming would leave this child wedged on a full pipe forever.
            let prompt = "x".repeat(200_000);
            let chatter = PASS[2];
            let script = format!(
                "cat > /dev/null; yes '{chatter}' | head -n 20000; {}",
                printing(&PASS[3..])
            );

            let (agent, received) = listening(stand_in(&script));
            let response = agent
                .run(&AgentRequest::new(prompt, "."))
                .expect("a chatty stand-in still exits cleanly");

            // Roughly two megabytes of stream, every line of it read and every
            // thought in it reported, then the document at the end.
            assert_eq!(response.text(), DOCUMENT);
            let activities = drained(&received);
            assert_eq!(activities.len(), 20_001);
            assert!(
                activities[..20_000]
                    .iter()
                    .all(|activity| *activity == Activity::Thinking)
            );
            assert_eq!(activities[20_000], Activity::Cost { usd: 0.0342 });
        }

        #[test]
        fn a_hanging_pass_times_out_and_its_child_stops() {
            let directory = scratch("hang");
            let ticks = directory.join("ticks");
            // Never exits on its own, and says so in a file: whether it is
            // still running after the call is a question the test can ask.
            let agent = stand_in("while :; do echo tick >> ticks; sleep 0.05; done")
                .with_timeout(Duration::from_millis(250));

            let started = Instant::now();
            let error = agent
                .run(&AgentRequest::new("anything", &directory))
                .expect_err("this stand-in never finishes");
            let elapsed = started.elapsed();

            match error {
                AgentError::TimedOut { after } => assert_eq!(after, Duration::from_millis(250)),
                other => panic!("expected a timeout, got {other:?}"),
            }
            assert!(
                elapsed < Duration::from_secs(10),
                "the call waited {elapsed:?}, far past its timeout"
            );

            let before = fs::metadata(&ticks).map_or(0, |file| file.len());
            thread::sleep(Duration::from_millis(300));
            let after = fs::metadata(&ticks).map_or(0, |file| file.len());
            assert_eq!(
                before, after,
                "the child outlived the call that gave up on it"
            );
            clean_up(&directory);
        }

        /// The kill is only half of it: a child nobody waits on stays in the
        /// process table as a zombie. `/proc` is where that is visible, so this
        /// one test is Linux-only — the kill itself is covered above on every
        /// Unix.
        #[cfg(target_os = "linux")]
        #[test]
        fn a_timed_out_child_is_reaped_not_left_a_zombie() {
            let directory = scratch("reap");
            let agent =
                stand_in("echo $$ > pid; sleep 30").with_timeout(Duration::from_millis(250));

            let started = Instant::now();
            let error = agent
                .run(&AgentRequest::new("anything", &directory))
                .expect_err("this stand-in sleeps far past its timeout");
            let elapsed = started.elapsed();

            assert!(matches!(error, AgentError::TimedOut { .. }), "{error:?}");
            assert!(
                elapsed < Duration::from_secs(20),
                "the call outlasted the sleep it was supposed to cut short: {elapsed:?}"
            );

            let pid = fs::read_to_string(directory.join("pid")).expect("the child wrote its pid");
            let pid = pid.trim();
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "process {pid} is still in the table: killed but never reaped"
            );
            clean_up(&directory);
        }

        #[test]
        fn a_cancel_from_another_thread_ends_the_pass_promptly() {
            let directory = scratch("cancel");
            let pid_file = directory.join("pid");
            let cancel = Cancel::new();
            // The real five-minute timeout: the only thing that can end this
            // call in time is the cancel.
            let agent = stand_in("echo $$ > pid; sleep 30").with_cancel(cancel.clone());

            let stopper = {
                let pid_file = pid_file.clone();
                thread::spawn(move || {
                    // Stopped once it is genuinely running, which it says by
                    // writing its pid — a sleep here would be a race dressed
                    // up as a delay.
                    let waited = Instant::now();
                    while pid(&pid_file).is_none() && waited.elapsed() < AT_MOST {
                        thread::sleep(Duration::from_millis(10));
                    }
                    cancel.cancel();
                })
            };

            let started = Instant::now();
            let error = agent
                .run(&AgentRequest::new("anything", &directory))
                .expect_err("a cancelled pass has no document");
            let elapsed = started.elapsed();
            stopper.join().expect("the cancelling thread ran");

            assert!(is_cancelled(&error), "{error:?}");
            assert!(
                elapsed < Duration::from_secs(20),
                "the call sat out the sleep it was told to cut short: {elapsed:?}"
            );
            clean_up(&directory);
        }

        /// A cancel ends the call even when something the kill did not reach
        /// is still holding the child's pipes open.
        ///
        /// The stand-ins above are at the mercy of whichever `/bin/sh` the
        /// machine has: `sh -c "echo $$ > pid; sleep 30"` is one process under
        /// a shell that execs its last command, and two under one that forks,
        /// and only in the second case does anything outlive the kill. This
        /// one forks on purpose — `wait` is a builtin, so no shell can exec
        /// away — and pins the behaviour on both. It is the shape a real
        /// `claude` has: a tool subprocess of its own, inheriting the pipes it
        /// was given.
        #[test]
        fn a_cancel_does_not_wait_on_output_a_survivor_still_holds() {
            let directory = scratch("cancel-survivor");
            let pid_file = directory.join("pid");
            let survivor_file = directory.join("survivor");
            let cancel = Cancel::new();
            let agent = stand_in("sleep 30 & echo $! > survivor; echo $$ > pid; wait")
                .with_cancel(cancel.clone());

            let stopper = {
                let pid_file = pid_file.clone();
                thread::spawn(move || {
                    let waited = Instant::now();
                    while pid(&pid_file).is_none() && waited.elapsed() < AT_MOST {
                        thread::sleep(Duration::from_millis(10));
                    }
                    cancel.cancel();
                })
            };

            let started = Instant::now();
            let error = agent
                .run(&AgentRequest::new("anything", &directory))
                .expect_err("a cancelled pass has no document");
            let elapsed = started.elapsed();
            stopper.join().expect("the cancelling thread ran");

            assert!(is_cancelled(&error), "{error:?}");
            assert!(
                elapsed < Duration::from_secs(20),
                "the call waited on output the survivor was still holding: {elapsed:?}"
            );
            // The survivor is the point of the test, so it is this test's to
            // clear up. Nothing else can: the kill reaches the child, and this
            // one was never the child.
            if let Some(survivor) = pid(&survivor_file) {
                let _ = process::Command::new("/bin/kill").arg(survivor).status();
            }
            clean_up(&directory);
        }

        /// Killing is half of it here too: see
        /// [`a_timed_out_child_is_reaped_not_left_a_zombie`], which is
        /// Linux-only for the same reason — `/proc` is where the process table
        /// is visible.
        #[cfg(target_os = "linux")]
        #[test]
        fn a_cancelled_childs_process_is_gone_afterwards() {
            let directory = scratch("cancel-reap");
            let pid_file = directory.join("pid");
            let cancel = Cancel::new();
            let agent = stand_in("echo $$ > pid; sleep 30").with_cancel(cancel.clone());

            let stopper = {
                let pid_file = pid_file.clone();
                thread::spawn(move || {
                    let waited = Instant::now();
                    while pid(&pid_file).is_none() && waited.elapsed() < AT_MOST {
                        thread::sleep(Duration::from_millis(10));
                    }
                    cancel.cancel();
                })
            };

            let error = agent
                .run(&AgentRequest::new("anything", &directory))
                .expect_err("a cancelled pass has no document");
            stopper.join().expect("the cancelling thread ran");

            assert!(is_cancelled(&error), "{error:?}");
            let pid = pid(&pid_file).expect("the child wrote its pid before it was stopped");
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "process {pid} survived the cancel, or was killed and never reaped"
            );
            clean_up(&directory);
        }

        #[test]
        fn a_pass_started_after_a_cancel_spawns_nothing_at_all() {
            let directory = scratch("never-started");
            let marker = directory.join("marker");
            let cancel = Cancel::new();
            cancel.cancel();
            // Anything that ran would leave a file behind, and the call only
            // returns once its child has exited — so a missing marker is a
            // child that never existed, not one that has not got there yet.
            let agent = stand_in("touch marker").with_cancel(cancel);

            let error = agent
                .run(&AgentRequest::new("anything", &directory))
                .expect_err("a cancelled agent runs nothing");

            assert!(is_cancelled(&error), "{error:?}");
            assert!(!marker.exists(), "a cancelled agent spawned a child anyway");
            clean_up(&directory);
        }

        #[test]
        fn a_handle_nobody_cancels_leaves_the_run_exactly_as_it_was() {
            let cancel = Cancel::new();
            let agent = stand_in(&printing(&PASS)).with_cancel(cancel.clone());

            let response = agent
                .run(&AgentRequest::new("anything", "."))
                .expect("attaching a handle does not change a clean run");

            assert_eq!(response.text(), DOCUMENT);
            // The pass is over and the handle knows it: this reaches for a
            // child that is no longer registered, and returns rather than
            // killing whatever came next.
            cancel.cancel();
            assert!(cancel.is_cancelled());
        }
    }

    #[test]
    fn a_failure_arrives_quickly_rather_than_after_the_timeout() {
        // The timeout is a backstop, not a delay every failure pays.
        let agent = ClaudeAgent::new()
            .with_program(NOT_A_PROGRAM)
            .with_timeout(INVOCATION_TIMEOUT);

        let started = Instant::now();
        let _ = agent.run(&AgentRequest::new("anything", "."));

        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
