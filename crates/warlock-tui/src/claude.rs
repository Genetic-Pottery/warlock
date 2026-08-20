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
//!   having — the long ones. So stdout and stderr are each drained by their own
//!   thread, concurrently with the wait.
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
//! Threads and channels, no async runtime, and no dependency: this crate's
//! `Cargo.toml` gains nothing for any of it.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
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

/// What `claude` is asked to do when nothing else is asked for: print mode,
/// which is the non-interactive shape of "hand it a prompt, read its stdout".
///
/// Not a decision about invocation *mode* — headless per directory against one
/// long session is a later slice's call, and section 11 of the design doc
/// leaves it open. It is the minimum that makes a piped, terminal-less run work
/// at all, and it lives in a field so that later slice can change it without
/// touching a line of this file.
const ARGS: [&str; 1] = ["--print"];

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
}

impl ClaudeAgent {
    /// An agent that runs `claude --print` with the five-minute
    /// [`INVOCATION_TIMEOUT`].
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

    /// The command this agent runs.
    #[must_use]
    pub fn program(&self) -> &OsStr {
        &self.program
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

        // Drained concurrently with the wait, or a child that writes more than
        // a pipeful blocks forever and so does this call.
        let out = drain(stdout);
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
            Ok(Ok(status)) => {
                // The child is gone, so every pipe it held is closed and each
                // join returns: the readers with what they read, the writer
                // with nothing.
                let _ = waiter.join();
                let _ = writer.join();
                // Not `?`: every way out of this wait has to reach the
                // clearing below, and an early return would leave a finished
                // child registered for a later cancel to find.
                match (collect(out), collect(err)) {
                    (Ok(stdout), Ok(stderr)) => judge(status, &stdout, &stderr),
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

/// What an exit status, its stdout and its stderr mean in the engine's
/// vocabulary.
///
/// Order matters: a non-zero exit is reported as a failure even if it printed
/// something, and silence is only [`AgentError::EmptyOutput`] when the run
/// itself went fine. Whitespace counts as silence — a document of blank lines
/// is no document.
fn judge(status: ExitStatus, stdout: &[u8], stderr: &[u8]) -> Result<AgentResponse, AgentError> {
    if !status.success() {
        return Err(AgentError::Failed {
            code: status.code(),
            stderr: String::from_utf8_lossy(stderr).into_owned(),
        });
    }
    // Lossy rather than a decode error: a stray byte in a model's markdown is
    // not worth failing a pass over, and the engine's vocabulary has no variant
    // for it.
    let text = String::from_utf8_lossy(stdout).into_owned();
    if text.trim().is_empty() {
        return Err(AgentError::EmptyOutput);
    }
    Ok(AgentResponse::new(text))
}

/// Read everything `source` produces, on a thread of its own.
fn drain<R: Read + Send + 'static>(source: R) -> JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut source = source;
        let mut buffer = Vec::new();
        source.read_to_end(&mut buffer)?;
        Ok(buffer)
    })
}

/// Wait for what [`drain`] read.
///
/// A panicked reader is a bug rather than a transport failure, but it is not
/// worth panicking the caller over: it comes back as I/O like anything else
/// that stopped the pass being read.
fn collect(handle: JoinHandle<io::Result<Vec<u8>>>) -> Result<Vec<u8>, AgentError> {
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
    use std::thread;
    use std::time::{Duration, Instant};

    use warlock_engine::{Agent, AgentError, AgentRequest};

    use super::{Cancel, ClaudeAgent, INVOCATION_TIMEOUT};

    /// A name no directory on `PATH` can hold, so the lookup is guaranteed to
    /// fail the way a machine without `claude` fails.
    const NOT_A_PROGRAM: &str = "warlock-test-no-such-program-8f3a1c";

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
        assert_eq!(ClaudeAgent::default().timeout(), agent.timeout());
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
        use std::time::{Duration, Instant};
        use std::{env, fs, process, thread};

        use warlock_engine::{Agent, AgentError, AgentRequest};

        use super::super::{Cancel, ClaudeAgent};

        /// How long a test waits for a child to announce itself before giving
        /// up and cancelling anyway. Generous, because it is only reached when
        /// something is already wrong; the wait itself ends as soon as the pid
        /// file appears.
        const AT_MOST: Duration = Duration::from_secs(5);

        /// An agent whose `claude` is `sh -c script`.
        fn stand_in(script: &str) -> ClaudeAgent {
            ClaudeAgent::new()
                .with_program("/bin/sh")
                .with_args(["-c", script])
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
        fn a_clean_run_comes_back_as_the_text_it_printed() {
            // `cat` is the smallest possible model: it answers with the prompt
            // it was given, which proves the prompt reached stdin *and* that
            // stdin was closed — without EOF, `cat` would never return.
            let agent = ClaudeAgent::new()
                .with_program("/bin/cat")
                .with_args(Vec::<&str>::new());

            let response = agent
                .run(&AgentRequest::new("# module\n\nWhat it does.\n", "."))
                .expect("cat exits cleanly and prints what it was given");

            assert_eq!(response.text(), "# module\n\nWhat it does.\n");
        }

        #[test]
        fn the_pass_runs_in_the_directory_the_request_names() {
            let directory = scratch("cwd");
            fs::write(directory.join("marker.txt"), "here").expect("a file to look for");

            let response = stand_in("ls")
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
            for script in ["exit 0", "printf '\\n  \\n'"] {
                let error = stand_in(script)
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
            // than one, and so is the answer. Draining on threads is what
            // makes this return at all.
            let prompt = "x".repeat(200_000);

            let response = stand_in("cat > /dev/null; yes hello | head -n 20000")
                .run(&AgentRequest::new(prompt, "."))
                .expect("a chatty stand-in still exits cleanly");

            assert_eq!(response.text().lines().count(), 20_000);
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
            let agent = ClaudeAgent::new()
                .with_program("/bin/cat")
                .with_args(Vec::<&str>::new())
                .with_cancel(cancel.clone());

            let response = agent
                .run(&AgentRequest::new("# module\n\nWhat it does.\n", "."))
                .expect("attaching a handle does not change a clean run");

            assert_eq!(response.text(), "# module\n\nWhat it does.\n");
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
