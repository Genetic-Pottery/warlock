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
//! Threads and channels, no async runtime, and no dependency: this crate's
//! `Cargo.toml` gains nothing for any of it.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
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
        let (waiter, exited) = watch(&child);

        match exited.recv_timeout(self.timeout) {
            Ok(Ok(status)) => {
                // The child is gone, so every pipe it held is closed and each
                // join returns: the readers with what they read, the writer
                // with nothing.
                let _ = waiter.join();
                let _ = writer.join();
                let stdout = collect(out)?;
                let stderr = collect(err)?;
                judge(status, &stdout, &stderr)
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
        }
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
    use std::time::{Duration, Instant};

    use warlock_engine::{Agent, AgentError, AgentRequest};

    use super::{ClaudeAgent, INVOCATION_TIMEOUT};

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

    /// Everything below runs a real child, and the stand-ins it runs are shell
    /// scripts, so the whole module is Unix-only. What is being tested — the
    /// pipes, the timeout, the kill — is not, but a portable stand-in would
    /// have to be a second binary to build, and that costs more than the
    /// coverage it adds.
    #[cfg(unix)]
    mod unix {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};
        use std::{env, fs, process, thread};

        use warlock_engine::{Agent, AgentError, AgentRequest};

        use super::super::ClaudeAgent;

        /// An agent whose `claude` is `sh -c script`.
        fn stand_in(script: &str) -> ClaudeAgent {
            ClaudeAgent::new()
                .with_program("/bin/sh")
                .with_args(["-c", script])
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
