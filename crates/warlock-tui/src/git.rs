use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
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
    /// The command ran and said no. Carries the command as it was run, so a
    /// reader can run it themselves, and what `git` said about it.
    Refused {
        command: String,
        code: Option<i32>,
        message: String,
    },
    /// The command worked and the answer warlock needed was not in what it
    /// printed. Its own variant because the answer to this is never to carry on
    /// with a default: the one thing it is raised for is a default branch
    /// nothing names, and `main` is a guess that cuts a branch from the wrong
    /// place on the repositories where it is wrong.
    Unreadable {
        what: String,
        saw: String,
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
            Self::Refused {
                command,
                code,
                message,
            } => {
                let code = code.map_or_else(|| "a signal".to_owned(), |code| format!("{code}"));
                write!(f, "`{command}` exited with {code}: {message}")
            }
            Self::Unreadable { what, saw } => write!(f, "{what} could not be read: {saw}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source } => Some(source),
            Self::NotFound { .. }
            | Self::TimedOut { .. }
            | Self::Refused { .. }
            | Self::Unreadable { .. } => None,
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

/// The program every operation below runs, and the only one they run.
const GIT: &str = "git";

/// The remote a pull is against.
///
/// `git` has no notion of *the* remote — `origin` is only the name `clone`
/// gives the place a checkout came from — but a branch has to be pushed
/// somewhere and a pull request opened against something. One name in one
/// place, so a repository that calls its remote something else is a setting
/// somebody can add here rather than a string to go hunting for.
const REMOTE: &str = "origin";

/// One path `git status` named, under the two-letter code it named it with.
///
/// The code is kept as the two characters `git` printed rather than parsed into
/// "modified", "added", "untracked" and the rest: the reader is about to run
/// `git status` themselves, and a halt that renamed the codes would make them
/// translate back before they could look.
///
/// [`from`](Dirty::from) is the other side of a rename or a copy, which `git`
/// reports as a second field — the pull halts on a dirty tree and the halt has
/// to name both, because "`R` `src/route.rs`" alone does not say which file has
/// gone.
///
/// Paths are text, not bytes, and this is the one place in the module that
/// converts lossily: a path is not required to be UTF-8, but nothing here ever
/// opens one of these — they are read by a person deciding what to do with
/// their own working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirty {
    pub code: String,
    pub path: String,
    pub from: Option<String>,
}

impl fmt::Display for Dirty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.code, self.path)?;
        if let Some(from) = &self.from {
            write!(f, " (was {from})")?;
        }
        Ok(())
    }
}

/// One commit, by the identifier `git` gave it.
///
/// ```
/// use warlock_tui::Commit;
///
/// let commit = Commit::new("4e724822589a889678ef4d920a03a69e67d977e8");
///
/// assert_eq!(commit.short(), "4e724822");
/// assert_eq!(commit.to_string(), "4e724822589a889678ef4d920a03a69e67d977e8");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit(String);

impl Commit {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.0
    }

    /// Enough of the identifier to paste into a `git show`, for a halt message
    /// that has to fit on a line. Eight characters because that is what `git`
    /// abbreviates to on a repository of this size; ambiguity is the reader's
    /// to resolve, and [`id`](Commit::id) is still whole.
    #[must_use]
    pub fn short(&self) -> &str {
        let end = self
            .0
            .char_indices()
            .nth(8)
            .map_or(self.0.len(), |(index, _)| index);
        &self.0[..end]
    }
}

impl fmt::Display for Commit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Whether `HEAD` is where the caller left it.
///
/// A value rather than an error, and rather than a panic: warlock owns every
/// commit on a pull's branch, so a `HEAD` that moved while a session was
/// running means the session committed something. That is a thing the caller
/// halts and reports — with the commit, so the reader can go and look at it —
/// not a thing this module decides.
///
/// ```
/// use warlock_tui::{Commit, Head};
///
/// let before = Commit::new("aaaa1111");
/// let after = Commit::new("bbbb2222");
///
/// assert_eq!(Head::between(&before, &before), Head::Unmoved);
/// assert_eq!(
///     Head::between(&before, &after),
///     Head::Moved {
///         from: before,
///         to: after
///     }
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Head {
    Unmoved,
    Moved { from: Commit, to: Commit },
}

impl Head {
    #[must_use]
    pub fn between(before: &Commit, after: &Commit) -> Self {
        if before == after {
            Self::Unmoved
        } else {
            Self::Moved {
                from: before.clone(),
                to: after.clone(),
            }
        }
    }
}

/// Everything a pull asks of the checkout it runs in, said in the pull's words
/// rather than in `git`'s.
///
/// [`Runs`] is the seam that starts a process; this is the seam the pull is
/// written against, the way `Board` sits above `Posts` in `linear.rs`. A caller
/// is driven in a test by a stand-in that records the argument vectors and
/// scripts the answers, with no repository, no network and no `git` on the
/// machine.
///
/// What is missing is the point of it. There is no stash, no reset, no clean,
/// no merge, no rebase, no amend, no force and no delete — not as an option a
/// caller may pass, and not spelled some other way further down. A tree with
/// work in it is [reported](Repository::dirty) and the run stops, because that
/// work is somebody's and warlock did not put it there.
pub trait Repository {
    /// What is dirty in the working tree; empty is a clean tree.
    fn dirty(&self) -> Result<Vec<Dirty>, Error>;

    /// The branch the remote points its `HEAD` at — detected, and reported
    /// rather than guessed when nothing names one.
    fn default_branch(&self) -> Result<String, Error>;

    /// Move onto a branch that is already there. What a resumed run does with
    /// the branch its record names, instead of cutting it a second time.
    fn switch_to(&self, branch: &str) -> Result<(), Error>;

    /// Bring the checked-out `branch` up to the remote, refusing anything that
    /// is not a fast-forward.
    fn catch_up(&self, branch: &str) -> Result<(), Error>;

    /// Cut `branch` from `from` and move onto it.
    fn cut_branch(&self, branch: &str, from: &str) -> Result<(), Error>;

    /// The commit `HEAD` is on.
    fn head(&self) -> Result<Commit, Error>;

    /// Stage everything the working tree has and make one commit carrying
    /// exactly `message`.
    fn commit_all(&self, message: &str) -> Result<(), Error>;

    /// Push `branch` to the remote and set it as the upstream.
    fn publish(&self, branch: &str) -> Result<(), Error>;
}

/// The one [`Repository`] that speaks `git`, over whatever [`Runs`] it holds
/// and in whatever directory it was given.
///
/// ```no_run
/// use warlock_tui::{Git, Repository};
///
/// // Runs a real `git`, so this example is not executed by the test suite.
/// let checkout = Git::at(".");
///
/// for dirty in checkout.dirty()? {
///     println!("{dirty}");
/// }
/// # Ok::<(), warlock_tui::GitError>(())
/// ```
#[derive(Debug, Clone)]
pub struct Git<R = Spawner> {
    runs: R,
    directory: PathBuf,
}

impl Git<Spawner> {
    /// A checkout run by a real `git` under the module's clock.
    #[must_use]
    pub fn at(directory: impl Into<PathBuf>) -> Self {
        Self::new(Spawner::new(), directory)
    }
}

impl<R: Runs> Git<R> {
    #[must_use]
    pub fn new(runs: R, directory: impl Into<PathBuf>) -> Self {
        Self {
            runs,
            directory: directory.into(),
        }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// One `git`, whatever it exited with: for the calls that ask a question
    /// and read the answer off a refusal as readily as off a success.
    fn ran(&self, args: &[&str]) -> Result<Ran, Error> {
        let vector: Vec<OsString> = args.iter().map(OsString::from).collect();
        self.runs.run(GIT.as_ref(), &vector, &self.directory)
    }

    /// One `git` that has to have worked, so every caller below is spared the
    /// same four lines and no operation can forget to look at the status.
    fn done(&self, args: &[&str]) -> Result<Ran, Error> {
        let ran = self.ran(args)?;
        if ran.success() {
            Ok(ran)
        } else {
            Err(Error::Refused {
                command: format!("{GIT} {}", args.join(" ")),
                code: ran.code(),
                message: complaint(&ran),
            })
        }
    }
}

impl<R: Runs> Repository for Git<R> {
    fn dirty(&self) -> Result<Vec<Dirty>, Error> {
        // `--porcelain=v1` pins the format against a `git` that one day makes
        // v2 the default, `-z` is what makes a path with a newline or a quote
        // in it readable at all, and `--untracked-files=all` names the files
        // inside a new directory rather than the directory: a halt says which
        // files are in the way, and "`?? src/`" is not that.
        let ran = self.done(&["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
        Ok(dirty_in(ran.stdout()))
    }

    fn default_branch(&self) -> Result<String, Error> {
        // The local answer first, because it is a file read and the other one
        // is a round trip to the forge.
        let local = self.ran(&["symbolic-ref", "--quiet", "--short", &origin_head()])?;
        if local.success()
            && let Some(branch) = branch_named(&local.stdout_text())
        {
            return Ok(branch);
        }

        // `origin/HEAD` is unset in a checkout cloned before `git` set it, and
        // in one where somebody deleted it; the remote still knows.
        let remote = self.ran(&["remote", "show", REMOTE])?;
        if remote.success()
            && let Some(branch) = head_branch_in(&remote.stdout_text())
        {
            return Ok(branch);
        }

        // No guess. `main` would be right nearly every time, and the times it
        // was wrong a branch would be cut from `master` or `develop` behind
        // somebody's back and a pull request opened against a branch nobody
        // merges into.
        Err(Error::Unreadable {
            what: format!("the default branch of `{REMOTE}`"),
            saw: undetected(&local, &remote),
        })
    }

    fn switch_to(&self, branch: &str) -> Result<(), Error> {
        self.done(&["switch", branch])?;
        Ok(())
    }

    fn catch_up(&self, branch: &str) -> Result<(), Error> {
        // `--ff-only` and not a plain pull: a default branch that has diverged
        // from the remote is a checkout somebody is in the middle of something
        // in, and the answer to that is to stop, not to make a merge commit on
        // it. The remote and the branch are named rather than left to the
        // upstream configuration, so what this pulls does not depend on how the
        // checkout was set up.
        self.done(&["pull", "--ff-only", REMOTE, branch])?;
        Ok(())
    }

    fn cut_branch(&self, branch: &str, from: &str) -> Result<(), Error> {
        // The start point is named rather than taken from whatever is checked
        // out: this is the one call that decides where a ticket's work begins,
        // and it should not depend on a switch somewhere above it having
        // happened.
        self.done(&["switch", "--create", branch, from])?;
        Ok(())
    }

    fn head(&self) -> Result<Commit, Error> {
        let ran = self.done(&["rev-parse", "HEAD"])?;
        let id = ran.stdout_text().trim().to_owned();
        if id.is_empty() {
            return Err(Error::Unreadable {
                what: "the commit `HEAD` is on".to_owned(),
                saw: "`git rev-parse HEAD` printed nothing".to_owned(),
            });
        }
        Ok(Commit::new(id))
    }

    /// `git add -A` and then one commit. Everything, because a sub-task's work
    /// is whatever it left in the tree and a warlock that picked paths would
    /// commit half of it.
    ///
    /// A `git` that refuses because there is nothing to commit is an
    /// [`Error::Refused`] like any other, carrying what `git` said: whether a
    /// sub-task that changed nothing is a failure is the caller's to decide.
    fn commit_all(&self, message: &str) -> Result<(), Error> {
        self.done(&["add", "-A"])?;
        // The message as its own argument, never interpolated into one: a goal
        // with a quote, a newline or a `$` in it is a commit message, not a
        // shell word, and nothing here goes through a shell anyway.
        self.done(&["commit", "-m", message])?;
        Ok(())
    }

    fn publish(&self, branch: &str) -> Result<(), Error> {
        // `--set-upstream` every time rather than only the first: a resumed run
        // pushes a branch that already has one, and setting it again to the
        // same place costs nothing and spares the caller having to know which
        // kind of run it is in.
        self.done(&["push", "--set-upstream", REMOTE, branch])?;
        Ok(())
    }
}

/// The branch a pull's work goes on: the ticket, lowercased, then the title.
///
/// The team key is a parameter because it belongs to the scope record in
/// `.warlock/pacts.toml`, and nothing in this module reads a file.
///
/// ```
/// use warlock_tui::branch_name;
///
/// assert_eq!(
///     branch_name("WAR", 137, "Run git and gh behind a seam: branch, commits, push"),
///     "war-137/run-git-and-gh-behind-a-seam-branch-commits-push"
/// );
/// ```
#[must_use]
pub fn branch_name(team_key: &str, number: u32, title: &str) -> String {
    format!(
        "{}-{number}/{}",
        team_key.trim().to_lowercase(),
        branch_slug(title)
    )
}

/// The message on every commit warlock makes: `<TICKET> <TICKET>.NN: <goal>`.
///
/// The ticket twice, because the two halves are read by different things. The
/// first word is what a `git log --oneline | grep WAR-137` matches, so a whole
/// ticket's history comes back whichever sub-task each commit belongs to; the
/// second is the sub-task itself, which is what a halted run resumes from.
///
/// ```
/// use warlock_tui::commit_message;
///
/// assert_eq!(
///     commit_message("WAR-137", "WAR-137.02", "Render the pull request title and body"),
///     "WAR-137 WAR-137.02: Render the pull request title and body"
/// );
/// ```
#[must_use]
pub fn commit_message(ticket: &str, sub_task: &str, goal: &str) -> String {
    // Trimmed on every side: a goal arrives from a brief, where it is a line of
    // markdown that may well end in a newline, and a commit message with a
    // blank second line is a body as far as `git log` is concerned.
    format!(
        "{} {}: {}",
        ticket.trim(),
        sub_task.trim(),
        goal.trim().replace('\n', " ")
    )
}

/// About this many characters of title in a branch name.
///
/// A branch name is read in `git branch`, in a shell prompt and in the URL of a
/// pull request, all beside the ticket half and whatever else is on the line.
const BRANCH_SLUG_MAX: usize = 50;

/// What a title with nothing sluggable in it becomes, so there is always a
/// second half: `war-137/` is not a branch name.
const UNNAMED: &str = "untitled";

// A second copy of the rule in `writing.rs`'s `slugged`, on purpose, and the
// only two places it is written.
//
// It cannot be shared as the code stands: `writing.rs` is a `main.rs` module and
// `pub(crate)` to the binary, so the library cannot call it, and lifting it here
// would leave the document writer taking its filename rule from the module that
// runs `git`.
//
// It should not be shared even then, because the two answer to different things.
// A filename is a `docs/` name a person reads and renames at will; a branch name
// is a ref that a run record, a pushed branch and an open pull request all have
// to agree on, so tuning the filename rule must not quietly move every branch
// and leave a halted run resuming onto a name that no longer exists. They
// already differ twice over: the cap is this one's own, and a title that slugs
// to nothing is `untitled.md` there — a name a reader fixes — and
// `<ticket>/untitled` here, a name nobody ever types.
fn branch_slug(title: &str) -> String {
    let mut slug = String::new();
    for character in title.chars() {
        if character.is_alphanumeric() {
            slug.extend(character.to_lowercase());
        } else if !slug.ends_with('-') {
            // Every run of punctuation and whitespace, however long, is one
            // hyphen: `git` takes a double hyphen in a ref, but a reader asked
            // to type one is reading a typo.
            slug.push('-');
        }
    }
    let slug = capped(slug.trim_matches('-'));
    if slug.is_empty() {
        return UNNAMED.to_owned();
    }
    slug.to_owned()
}

// Three cases, and the third is why the cap is "about". A slug that fits comes
// back whole; one that does not is cut back to the last hyphen inside the cap;
// and one whose first word is itself longer than the cap is cut after that word,
// however long it is, because there is nowhere to break it and half a word in a
// branch name is a name nobody can guess the rest of.
fn capped(slug: &str) -> &str {
    let Some((cut, _)) = slug.char_indices().nth(BRANCH_SLUG_MAX) else {
        return slug;
    };
    if slug[cut..].starts_with('-') {
        return &slug[..cut];
    }
    if let Some(hyphen) = slug[..cut].rfind('-') {
        return &slug[..hyphen];
    }
    match slug[cut..].find('-') {
        Some(end) => &slug[..cut + end],
        None => slug,
    }
}

// `XY PATH\0`, and for a rename or a copy `XY PATH\0ORIG_PATH\0` — the new name
// first and the old one as a field of its own, which is the whole reason the
// entries are walked with an iterator rather than mapped over.
fn dirty_in(payload: &[u8]) -> Vec<Dirty> {
    // The payload ends in a NUL, so the split leaves an empty tail; a field in
    // the middle is never empty, because every one of them starts with a status
    // or is a path.
    let mut fields = payload
        .split(|&byte| byte == 0)
        .filter(|field| !field.is_empty());
    let mut dirty = Vec::new();

    while let Some(field) = fields.next() {
        // `XY` and the space after it: three bytes, all of them ASCII, before
        // the first byte of the path.
        if field.len() < 4 {
            continue;
        }
        let code = String::from_utf8_lossy(&field[..2]).into_owned();
        let path = String::from_utf8_lossy(&field[3..]).into_owned();
        let renamed = code.contains('R') || code.contains('C');
        let from = renamed
            .then(|| fields.next())
            .flatten()
            .map(|field| String::from_utf8_lossy(field).into_owned());
        dirty.push(Dirty { code, path, from });
    }

    dirty
}

fn origin_head() -> String {
    format!("refs/remotes/{REMOTE}/HEAD")
}

// `--short` answers `origin/main`; the unabbreviated ref is stripped as well, so
// a `git` that ever stops shortening is read rather than taken for a branch
// called `refs`.
fn branch_named(text: &str) -> Option<String> {
    let name = text.trim();
    let name = name.strip_prefix("refs/remotes/").unwrap_or(name);
    let name = name.strip_prefix(&format!("{REMOTE}/")).unwrap_or(name);
    (!name.is_empty()).then(|| name.to_owned())
}

// What to say when neither probe named a branch: whatever `git` complained,
// and the one command that sets `origin/HEAD` for good. A halt with something
// to do in it, rather than one that only refuses.
fn undetected(local: &Ran, remote: &Ran) -> String {
    let said = [complaint(remote), complaint(local)]
        .into_iter()
        .find(|said| !said.is_empty())
        .unwrap_or_else(|| format!("`{REMOTE}/HEAD` names no branch"));
    format!("{said}; run `git remote set-head {REMOTE} --auto`")
}

// The one line of `git remote show origin` that matters. `(unknown)` is what it
// prints when the remote has no HEAD at all, and that is not a branch name.
fn head_branch_in(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("HEAD branch:"))
        .map(str::trim)
        .filter(|branch| !branch.is_empty() && *branch != "(unknown)")
        .map(ToOwned::to_owned)
}

// What `git` said about a refusal. Its stderr, which is where it complains, and
// its stdout when it did not: `git commit` says "nothing to commit" on stdout
// and exits non-zero, and an error carrying an empty string would be a halt
// nobody can act on.
fn complaint(ran: &Ran) -> String {
    let stderr = ran.stderr_text();
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_owned();
    }
    ran.stdout_text().trim().to_owned()
}

/// The last line of every body, and the only claim in it warlock makes about
/// itself.
///
/// One line and not a wrapped paragraph: with no `gh` on the machine this body
/// is commented on the ticket instead, and Linear's editor reads a single
/// newline inside a paragraph as a line break — the reason
/// `brief::for_the_board` exists — so a sentence hard wrapped at this file's
/// column width arrives ragged on a page with a width of its own.
pub const HUMAN_GATE: &str = "The pull request and the ticket's review state are the human gate: warlock merges nothing, closes nothing and moves nothing past review.";

// Named under the heading rather than left to the reader, because a list of
// scopes with no sentence over it reads as a confession: these are directories
// this machine was entitled to edit, and the section exists so a reviewer sees
// the work reached past the one scope it was pulled under.
const TOUCHED_NOTE: &str = "Held on this machine, and not the scope this ticket was pulled under.";

/// A sub-task the run finished, as the body names it.
///
/// Borrowed and flat on purpose. The run record is another ticket's to own, and a
/// body that took its types would make rendering wait on them; mapping a record
/// into these three strings is a line at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finished<'a> {
    pub id: &'a str,
    pub goal: &'a str,
    pub summary: &'a str,
}

/// A scope the run edited under while holding it, other than the one the ticket
/// was pulled under, and the paths it touched there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Touched<'a> {
    pub scope: &'a str,
    pub paths: Vec<&'a str>,
}

/// A pacted directory the branch made stale that the refresh did not put back,
/// and why it did not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeftStale<'a> {
    pub directory: &'a str,
    pub reason: &'a str,
}

/// ```
/// use warlock_tui::pull_request_title;
///
/// assert_eq!(
///     pull_request_title("WAR-131", "The Linear queue query"),
///     "WAR-131: The Linear queue query"
/// );
/// ```
#[must_use]
pub fn pull_request_title(ticket: &str, title: &str) -> String {
    // Trimmed because a ticket title arrives from the board, and a newline or a
    // trailing space in it survives all the way into `gh pr create`'s `--title`,
    // where it is a pull request nobody can search for by name.
    format!("{}: {}", ticket.trim(), title.trim())
}

/// What the pull request says, and what gets commented on the ticket when there
/// is no `gh` to open one — so it stands alone, with no pull request around it.
///
/// A section with nothing in it is absent rather than an empty heading: a run
/// that crossed nothing and left nothing stale is the ordinary run, and three
/// bare headings saying so would be the bulk of its body.
///
/// ```
/// use warlock_tui::{HUMAN_GATE, pull_request_body};
///
/// // The emptiest run there is: no description, no sub-task, nothing crossed
/// // and nothing stale.
/// let body = pull_request_body("", &[], &[], &[]);
///
/// assert_eq!(body, format!("{HUMAN_GATE}\n"));
/// ```
#[must_use]
pub fn pull_request_body(
    description: &str,
    finished: &[Finished<'_>],
    touched: &[Touched<'_>],
    stale: &[LeftStale<'_>],
) -> String {
    let mut blocks: Vec<String> = Vec::new();

    let description = description.trim();
    if !description.is_empty() {
        blocks.push(description.to_owned());
    }

    if !finished.is_empty() {
        let mut block = String::from("## Sub-tasks");
        for task in finished {
            let _ = write!(block, "\n\n### {} {}", task.id.trim(), task.goal.trim());
            let summary = task.summary.trim();
            if !summary.is_empty() {
                let _ = write!(block, "\n\n{summary}");
            }
        }
        blocks.push(block);
    }

    if !touched.is_empty() {
        let mut block = format!("## Scopes touched\n\n{TOUCHED_NOTE}");
        for scope in touched {
            let _ = write!(block, "\n\n### {}", scope.scope.trim());
            if !scope.paths.is_empty() {
                // The blank line that opens the list, written once here so every
                // item below writes the same thing. Under the guard because a
                // scope with no path under it would otherwise end the block on a
                // blank line, and the join would make three.
                block.push('\n');
                for path in &scope.paths {
                    let _ = write!(block, "\n- `{}`", path.trim());
                }
            }
        }
        blocks.push(block);
    }

    if !stale.is_empty() {
        let mut block = String::from("## Directories left stale\n");
        for directory in stale {
            let _ = write!(
                block,
                "\n- `{}` — {}",
                directory.directory.trim(),
                directory.reason.trim()
            );
        }
        blocks.push(block);
    }

    blocks.push(HUMAN_GATE.to_owned());

    let mut body = blocks.join("\n\n");
    body.push('\n');
    body
}

/// The program the pull request goes through, and the only one this half runs.
const GH: &str = "gh";

/// The pull request to open, once everything about it has been decided.
///
/// Four strings of the same type, so they are named rather than positional: the
/// two branches are the pair that a call swapping them would still compile and
/// still run, and the pull request would be the default branch merged into the
/// work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PullRequest<'a> {
    /// The branch it merges into — the detected default branch, never a guess.
    pub base: &'a str,
    /// The branch it merges: the one this run's commits are on.
    pub head: &'a str,
    /// [`pull_request_title`]'s answer.
    pub title: &'a str,
    /// [`pull_request_body`]'s answer.
    pub body: &'a str,
}

/// What came of asking for a pull request.
///
/// [`NoGh`](Opened::NoGh) is a variant of the success and not an
/// [`Error`], and that is a decision rather than an oversight. A machine with
/// no `gh` on it has still done the work: the branch is cut, the commits are
/// made and the push has happened, and the caller's answer to this variant is
/// to comment [the same body](pull_request_body) on the ticket, name the
/// branch, and count the run finished. Folding it into an error would halt a
/// run that succeeded, and turn "install `gh`" into "the pull failed".
///
/// A `gh` that is there and says no is an [`Error::Refused`], which is the
/// distinction the variant exists for.
///
/// ```
/// use warlock_tui::Opened;
///
/// let opened = Opened::At {
///     url: "https://github.com/team/repo/pull/12".to_owned(),
/// };
///
/// assert_eq!(opened.url(), Some("https://github.com/team/repo/pull/12"));
/// assert_eq!(Opened::NoGh.url(), None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    At { url: String },
    NoGh,
}

impl Opened {
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        match self {
            Self::At { url } => Some(url),
            Self::NoGh => None,
        }
    }
}

/// The one thing a pull asks of the forge, and — like [`Repository`] — as much
/// for what it does not have as for what it does.
///
/// Opening a pull request is the whole of it. There is no merge, no approval,
/// no close and no auto-merge, because the pull request is where warlock stops:
/// a `gh pr merge` added here would take a run past the human gate its own body
/// promises is there.
///
/// Its own trait beside [`Repository`] rather than a method on it, because the
/// two are answered by different programs and only one of them may be missing:
/// a caller holding a [`Repository`] holds something that has to work, and a
/// caller holding a `Forge` holds something that may come back
/// [`Opened::NoGh`].
pub trait Forge {
    fn open_pull_request(&self, request: PullRequest<'_>) -> Result<Opened, Error>;
}

/// The one [`Forge`] that speaks `gh`, over whatever [`Runs`] it holds and in
/// whatever directory it was given.
///
/// ```no_run
/// use warlock_tui::{Forge, Gh, PullRequest};
///
/// // Runs a real `gh`, so this example is not executed by the test suite.
/// let opened = Gh::at(".").open_pull_request(PullRequest {
///     base: "main",
///     head: "war-137/run-git-and-gh-behind-a-seam",
///     title: "WAR-137: Run git and gh behind a seam",
///     body: "The pull request and the ticket's review state are the human gate.\n",
/// })?;
///
/// match opened.url() {
///     Some(url) => println!("{url}"),
///     None => println!("no `gh` on this machine; comment the body on the ticket"),
/// }
/// # Ok::<(), warlock_tui::GitError>(())
/// ```
#[derive(Debug, Clone)]
pub struct Gh<R = Spawner> {
    runs: R,
    directory: PathBuf,
}

impl Gh<Spawner> {
    /// A forge reached by a real `gh` under the module's clock.
    #[must_use]
    pub fn at(directory: impl Into<PathBuf>) -> Self {
        Self::new(Spawner::new(), directory)
    }
}

impl<R: Runs> Gh<R> {
    #[must_use]
    pub fn new(runs: R, directory: impl Into<PathBuf>) -> Self {
        Self {
            runs,
            directory: directory.into(),
        }
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

impl<R: Runs> Forge for Gh<R> {
    fn open_pull_request(&self, request: PullRequest<'_>) -> Result<Opened, Error> {
        // The base named rather than left to `gh`'s own default: `gh` would ask
        // the forge for the repository's default branch, which is the question
        // `Repository::default_branch` already answered against this checkout's
        // remote.
        //
        // Every value its own argument. A body is markdown with blank lines,
        // backticks and whatever a summary contained in it, and nothing here
        // goes through a shell.
        let args = [
            "pr",
            "create",
            "--base",
            request.base,
            "--head",
            request.head,
            "--title",
            request.title,
            "--body",
            request.body,
        ];
        let vector: Vec<OsString> = args.iter().map(OsString::from).collect();

        let ran = match self.runs.run(GH.as_ref(), &vector, &self.directory) {
            Ok(ran) => ran,
            // The whole reason `NotFound` is a variant of its own. Anything
            // else — a broken pipe, a timeout, a `gh` that exited non-zero —
            // stays what it is.
            Err(Error::NotFound { .. }) => return Ok(Opened::NoGh),
            Err(other) => return Err(other),
        };

        if !ran.success() {
            return Err(Error::Refused {
                // The two branches and not the whole vector: the title and the
                // body are the rendered document, and a halt that quoted them
                // back would bury what `gh` actually said under it.
                command: format!(
                    "{GH} pr create --base {} --head {}",
                    request.base, request.head
                ),
                code: ran.code(),
                message: complaint(&ran),
            });
        }

        match url_in(&ran.stdout_text()) {
            Some(url) => Ok(Opened::At { url }),
            None => Err(Error::Unreadable {
                what: "the pull request's URL".to_owned(),
                saw: said_or_silent(&ran),
            }),
        }
    }
}

// The last URL rather than the first: `gh pr create` prints the one it made on
// its own last line, and whatever it says above that — the branch it is opening
// from, a notice about the remote — may itself carry a link.
fn url_in(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|line| line.starts_with("https://") || line.starts_with("http://"))
        .map(ToOwned::to_owned)
}

fn said_or_silent(ran: &Ran) -> String {
    let said = complaint(ran);
    if said.is_empty() {
        format!("`{GH} pr create` printed nothing")
    } else {
        said
    }
}

#[cfg(test)]
#[path = "tests/git.rs"]
mod tests;
