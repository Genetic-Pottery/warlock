use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::claude::{drain, kill_and_reap, watch};

/// The clock one `git` or `gh` call runs under.
///
/// Not [`INVOCATION_TIMEOUT`](crate::INVOCATION_TIMEOUT): five minutes is the
/// measure of a model pass thinking, and the slowest thing here is a push or a
/// pull request over somebody's network. Two minutes rather than thirty seconds
/// because that network may be a phone tether pushing a repository's first
/// branch.
pub const COMMAND_TIMEOUT: Duration = Duration::from_mins(2);

/// One child process: a program, the whole of its argument vector, the directory
/// it runs in, and what it said.
///
/// The seam every `git` and `gh` operation goes through, so a caller is driven in
/// a test by a stand-in that records the vector and scripts the answer — with no
/// repository, no network and neither program on the machine. Nothing about what
/// a command *means* is decided here: a non-zero exit is a [`Ran`] and not an
/// error, because which statuses are failures is the operation's business and
/// `git diff --quiet` answers with one.
pub trait Runs {
    fn run(&self, program: &OsStr, args: &[OsString], directory: &Path) -> Result<Ran, Error>;
}

/// What a child that ran came back with.
///
/// The exit status is its code and not a
/// [`ExitStatus`](std::process::ExitStatus), which is why a stand-in can build
/// one of these at all: there is no portable way to mint an `ExitStatus`, so a
/// seam carrying one would be a seam only a real child could cross. A child
/// killed by a signal has no code, and [`success`](Ran::success) is false for it.
///
/// Output is bytes, not [`String`]: `git status -z` names paths, a path is not
/// required to be UTF-8, and a lossy conversion here would hand the caller a
/// filename that no longer exists.
///
/// ```
/// use warlock_tui::Ran;
///
/// let ran = Ran::new(Some(0), "refs/remotes/origin/main\n", "");
///
/// assert!(ran.success());
/// assert_eq!(ran.stdout_text().trim(), "refs/remotes/origin/main");
/// assert!(!Ran::new(Some(1), "", "fatal: not a git repository").success());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ran {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Ran {
    #[must_use]
    pub fn new(code: Option<i32>, stdout: impl Into<Vec<u8>>, stderr: impl Into<Vec<u8>>) -> Self {
        Self {
            code,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[must_use]
    pub fn code(&self) -> Option<i32> {
        self.code
    }

    #[must_use]
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }

    #[must_use]
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    #[must_use]
    pub fn stdout_text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    #[must_use]
    pub fn stderr_text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }
}

/// Why a child did not run, or did not finish.
#[derive(Debug)]
pub enum Error {
    /// The program is not on the `PATH`. Its own variant, and never folded into
    /// [`Error::Io`]: `gh` being absent is an outcome the pull reports and
    /// carries on from, and a caller that cannot tell it from a broken pipe has
    /// to treat every I/O failure as a machine without `gh` on it.
    NotFound {
        program: String,
    },
    Io {
        source: io::Error,
    },
    TimedOut {
        after: Duration,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { program } => write!(f, "`{program}` is not on the PATH"),
            Self::Io { source } => write!(f, "the command could not be run: {source}"),
            Self::TimedOut { after } => {
                write!(f, "the command did not finish within {after:?}")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source } => Some(source),
            Self::NotFound { .. } | Self::TimedOut { .. } => None,
        }
    }
}

/// The one implementation that starts a process, and the only thing in this
/// module that touches the machine.
///
/// ```no_run
/// use std::ffi::OsString;
/// use std::path::Path;
///
/// use warlock_tui::{Runs, Spawner};
///
/// // Runs a real `git`, so this example is not executed by the test suite.
/// let ran = Spawner::new().run(
///     "git".as_ref(),
///     &[OsString::from("rev-parse"), OsString::from("HEAD")],
///     Path::new("."),
/// )?;
///
/// println!("{}", ran.stdout_text().trim());
/// # Ok::<(), warlock_tui::GitError>(())
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Spawner {
    timeout: Duration,
}

impl Spawner {
    /// ```
    /// use warlock_tui::{COMMAND_TIMEOUT, Spawner};
    ///
    /// assert_eq!(Spawner::new().timeout(), COMMAND_TIMEOUT);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            timeout: COMMAND_TIMEOUT,
        }
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl Default for Spawner {
    fn default() -> Self {
        Self::new()
    }
}

impl Runs for Spawner {
    fn run(&self, program: &OsStr, args: &[OsString], directory: &Path) -> Result<Ran, Error> {
        let mut child = Command::new(program)
            .args(args)
            // Two deliberate absences. Nothing is added to the environment —
            // warlock's keys reach a `git` or `gh` child neither in the vector
            // nor here — and nothing is taken away either: `gh` authenticates
            // from the ambient environment and its own config, so an
            // `env_clear` would make every pull request fail on a machine where
            // `gh` works perfectly from the shell.
            .current_dir(directory)
            // Closed rather than piped, which is what makes the writer thread
            // `claude.rs` needs unnecessary: a `git` that wants a passphrase, a
            // merge message or a credential reads stdin, and a child holding a
            // pipe nobody writes to waits for the timeout instead of failing at
            // once.
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| spawn_error(error, program, directory))?;

        // Piped by the lines above, so both are `Some`.
        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");

        // Read concurrently with the wait, or a `git diff` of anything real
        // fills its pipe and blocks forever. Both whole rather than a line at a
        // time: a command's output is read once it has exited, so there is
        // nothing to report as it arrives.
        let out = drain(stdout);
        let err = drain(stderr);

        let child = Arc::new(Mutex::new(child));
        let (waiter, exited) = watch(&child);

        match exited.recv_timeout(self.timeout) {
            Ok(Ok(status)) => {
                // The child is gone, so both pipes are closed and both joins
                // return.
                let _ = waiter.join();
                Ok(Ran {
                    code: status.code(),
                    stdout: collected(out)?,
                    stderr: collected(err)?,
                })
            }
            // The waiter could not tell whether it had exited; clean up after it
            // and report it as the I/O failure it is.
            Ok(Err(source)) => {
                kill_and_reap(&child);
                let _ = waiter.join();
                Err(Error::Io { source })
            }
            Err(RecvTimeoutError::Timeout) => {
                // Killed *and* reaped: a killed child nobody waits on is a
                // zombie in the process table, and an abandoned one is a `git
                // push` still running against a branch this call has given up
                // on. The waiter sees the exit it caused within one poll, so
                // joining it is bounded; the readers are deliberately not
                // joined, because a grandchild the kill did not reach can hold
                // their pipes open and this call has already decided what it
                // returns.
                kill_and_reap(&child);
                let _ = waiter.join();
                Err(Error::TimedOut {
                    after: self.timeout,
                })
            }
            // Unreachable in practice: the waiter sends before it returns.
            Err(RecvTimeoutError::Disconnected) => {
                kill_and_reap(&child);
                Err(Error::Io {
                    source: io::Error::other("the process waiter stopped without an exit status"),
                })
            }
        }
    }
}

/// A `NotFound` is only ever the program when the directory is there. A call
/// naming a working directory that has since gone would otherwise be reported as
/// a missing `git`, and somebody would go looking for it on their `PATH`.
fn spawn_error(error: io::Error, program: &OsStr, directory: &Path) -> Error {
    if error.kind() == io::ErrorKind::NotFound && directory.is_dir() {
        Error::NotFound {
            program: program.to_string_lossy().into_owned(),
        }
    } else {
        Error::Io { source: error }
    }
}

fn collected(handle: JoinHandle<io::Result<Vec<u8>>>) -> Result<Vec<u8>, Error> {
    match handle.join() {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(source)) => Err(Error::Io { source }),
        Err(_) => Err(Error::Io {
            source: io::Error::other("the thread reading the command's output panicked"),
        }),
    }
}

#[cfg(test)]
#[path = "tests/git.rs"]
mod tests;
