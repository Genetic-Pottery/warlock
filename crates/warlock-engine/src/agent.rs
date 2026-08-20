//! The agent seam: how the engine reaches a model without reaching for one.
//!
//! Section 11 of the design doc fixes the mechanism for talking to a model:
//! Warlock runs the `claude` CLI, hands it a prompt, and reads what it writes
//! to stdout. It holds no credentials of its own and is inert without a
//! logged-in `claude` on `PATH`. That mechanism is a subprocess, and this crate
//! promises in [`lib.rs`](crate)'s first paragraph that it spawns none.
//!
//! Both stay true because the two halves are split at a port. The engine owns
//! the domain — what to ask, what an answer is, and what each way of not
//! getting one means — and states it here as a trait ([`Agent`]) over a request
//! ([`Request`]) and a response ([`Response`]). The binary owns the transport:
//! the child process, its stdin, its stdout, its stderr, its exit status and
//! its timeout. Nothing in this module runs anything.
//!
//! Three consequences are worth stating, because they are the reasons the seam
//! is drawn here rather than anywhere else:
//!
//! * **The engine's tests need no `claude`, no network and no terminal.** A
//!   hand-written fake implementing [`Agent`] returns canned markdown, so every
//!   decision built on top of a model pass is exercised in memory. That is the
//!   standing rule for this crate's tests, and it is why no mocking framework
//!   is needed.
//! * **No process type crosses the seam.** [`Error::Failed`] carries an exit
//!   code as an [`Option<i32>`] and captured stderr as a [`String`], not a
//!   [`std::process::ExitStatus`]. Those variants describe transport failures
//!   the binary reports back in the engine's vocabulary; a caller reading them
//!   never learns that a process was involved, and a future implementation that
//!   is not a process still fits.
//! * **This is transport, not payload.** A [`Request`] carries only what
//!   running a pass needs — the prompt text, and the directory to run it in.
//!   What goes *into* a prompt, and which files a request should scope, is a
//!   separate decision; the request's fields are private behind a constructor
//!   so that decision can add to it without touching [`Agent::run`].
//!
//! The failure vocabulary is deliberately not one `Io` bucket. Each variant is
//! a different thing for a caller to say or do: a missing `claude` is the
//! ordinary state of a fresh machine and deserves a message naming the binary
//! rather than "No such file or directory (os error 2)"; a non-zero exit means
//! the model was reached and refused, and its stderr is the only clue why;
//! empty stdout means it was reached, said nothing, and there is no document to
//! write; a timeout is a hang, distinguishable from a refusal because the
//! answer to it is "try again or ask for less", not "read the error".

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How much of a failed pass's stderr goes into its [`Display`](fmt::Display).
///
/// A `Display` that has to fit one line of a footer cannot carry a megabyte of
/// backtrace, and the whole text is still on the error for anything that wants
/// it — the excerpt is a rendering choice, not a lossy capture.
const STDERR_EXCERPT: usize = 200;

/// One pass of a model over one directory.
///
/// The engine defines this and never implements it: the implementation is the
/// binary's, because running a model means running the `claude` CLI and this
/// crate spawns nothing. One method, because a pact is one question and one
/// answer — anything a later slice needs to send belongs on [`Request`], where
/// adding it breaks nobody.
///
/// `&self` rather than `&mut self` so an implementation can be shared, and the
/// method is free to be called more than once: nothing here says a pass is
/// unrepeatable.
///
/// ```
/// use warlock_engine::{Agent, AgentError, AgentRequest, AgentResponse};
///
/// /// The engine's own tests reach a model exactly like this: they don't.
/// struct Canned;
///
/// impl Agent for Canned {
///     fn run(&self, _request: &AgentRequest) -> Result<AgentResponse, AgentError> {
///         Ok(AgentResponse::new("# engine\n\nCore engine for warlock.\n"))
///     }
/// }
///
/// let request = AgentRequest::new("describe this directory", "crates/warlock-engine");
/// let response = Canned.run(&request)?;
///
/// assert!(response.text().starts_with("# engine"));
/// # Ok::<(), warlock_engine::AgentError>(())
/// ```
pub trait Agent {
    /// Run one pass and return what the model said.
    ///
    /// # Errors
    ///
    /// [`Error`], in the engine's vocabulary rather than the transport's: the
    /// agent command was not found, it exited non-zero, it wrote nothing, it
    /// did not finish in time, or the attempt failed with some other I/O
    /// error.
    fn run(&self, request: &Request) -> Result<Response, Error>;
}

/// What one pass needs in order to run: a prompt, and where to run it.
///
/// Nothing more, on purpose. This is the transport's half of the contract —
/// the prompt's *content*, and which files it should describe, is a decision
/// this type is shaped to absorb rather than one it makes. The fields are
/// private and reached through [`Request::new`] and the accessors, so a later
/// slice adds what it needs here and every implementation of [`Agent`] keeps
/// compiling.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Request {
    /// The whole text handed to the model.
    prompt: String,
    /// The directory the pass runs in, so relative paths in the prompt mean
    /// what the model would see if it looked.
    directory: PathBuf,
}

impl Request {
    /// A pass that asks `prompt`, run from `directory`.
    ///
    /// Infallible: neither field is validated here. Whether the directory
    /// exists is the transport's problem, and it reports back as
    /// [`Error::Io`].
    ///
    /// ```
    /// use std::path::Path;
    /// use warlock_engine::AgentRequest;
    ///
    /// let request = AgentRequest::new("summarise this module", "crates/warlock-engine");
    ///
    /// assert_eq!(request.prompt(), "summarise this module");
    /// assert_eq!(request.directory(), Path::new("crates/warlock-engine"));
    /// ```
    #[must_use]
    pub fn new(prompt: impl Into<String>, directory: impl Into<PathBuf>) -> Self {
        Self {
            prompt: prompt.into(),
            directory: directory.into(),
        }
    }

    /// The text to hand the model.
    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// The directory the pass runs in.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

/// What one pass produced: the text the model wrote.
///
/// Unparsed, and it stays that way — Warlock does not read `WARLOCK.md`, it
/// cares that one exists and what its bytes hash to. A response that reached
/// this type is a response the transport already judged usable: non-empty
/// output from a run that exited cleanly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Response {
    /// Everything the model said, verbatim.
    text: String,
}

impl Response {
    /// A response holding `text`.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    /// What the model said.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// What the model said, taking ownership.
    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }
}

/// Everything that can stop a model pass producing a document.
///
/// Hand-rolled like [`ManifestError`](crate::ManifestError) and
/// [`HashError`](crate::HashError), for the same reason: a handful of variants
/// do not pay for an error-handling dependency. Every variant's
/// [`Display`](fmt::Display) is a single line, because these are read in a
/// one-line footer as often as in a log — which is why [`Error::Failed`]
/// flattens and excerpts the stderr it carries instead of printing it whole.
///
/// `#[non_exhaustive]`: this is the list of failures the transport reports
/// today, not a claim that a transport can fail in no other way.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The agent command is not on `PATH`. The common case on a machine that
    /// has never installed it, so the message names the command rather than
    /// passing on an errno nobody can act on.
    NotFound {
        /// The command that was looked for, e.g. `claude`.
        program: String,
    },
    /// The pass ran and exited non-zero: the model was reached and something
    /// went wrong on the far side.
    Failed {
        /// The exit code, or `None` if it was killed before it could set one.
        /// A plain [`i32`] rather than a [`std::process::ExitStatus`] so no
        /// process type crosses the seam.
        code: Option<i32>,
        /// Everything the pass wrote to stderr, verbatim and possibly
        /// multi-line. Usually the only explanation there is.
        stderr: String,
    },
    /// The pass exited cleanly and wrote nothing to stdout, so there is no
    /// document. Distinct from [`Error::Failed`] because nothing failed:
    /// there is simply no answer to write.
    EmptyOutput,
    /// The pass did not finish inside the time it was given and was stopped.
    /// Its own variant rather than an exit code, because a hang and a refusal
    /// call for different answers.
    TimedOut {
        /// How long it was given.
        after: Duration,
    },
    /// The pass could not be run, or could not be read from, for some other
    /// reason: the directory is gone, a pipe broke, a handle could not be
    /// opened.
    Io {
        /// What the operating system said.
        source: std::io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { program } => write!(
                f,
                "`{program}` is not on PATH, so no model can be reached; install it and log in"
            ),
            Self::Failed { code, stderr } => {
                let said = one_line(stderr);
                let said = if said.is_empty() {
                    "it wrote nothing to stderr".to_owned()
                } else {
                    format!("stderr: {said}")
                };
                match code {
                    Some(code) => write!(f, "the model pass exited with status {code} ({said})"),
                    None => write!(f, "the model pass was killed before it exited ({said})"),
                }
            }
            Self::EmptyOutput => write!(f, "the model pass wrote nothing, so there is no document"),
            Self::TimedOut { after } => write!(
                f,
                "the model pass did not finish within {}s and was stopped",
                after.as_secs()
            ),
            Self::Io { source } => write!(f, "could not run the model pass: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source } => Some(source),
            Self::NotFound { .. }
            | Self::Failed { .. }
            | Self::EmptyOutput
            | Self::TimedOut { .. } => None,
        }
    }
}

/// `text` as one line: runs of whitespace collapsed to single spaces, and no
/// more than [`STDERR_EXCERPT`] characters of it.
///
/// Cut on a character boundary, counting characters rather than bytes, so a
/// stack trace full of arrows and box drawing cannot panic the formatter.
fn one_line(text: &str) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flattened.char_indices().nth(STDERR_EXCERPT) {
        Some((cut, _)) => format!("{}…", &flattened[..cut]),
        None => flattened,
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{Agent, Error, Request, Response};

    /// The whole point of the seam, in one struct: an [`Agent`] that answers
    /// with canned markdown. No `claude`, no terminal, no network, no mocking
    /// framework — and every test below runs on a machine that has none of
    /// them.
    struct Canned {
        /// What every pass returns.
        markdown: &'static str,
        /// Where each pass was asked to run, in call order.
        seen: std::cell::RefCell<Vec<(String, PathBuf)>>,
    }

    impl Canned {
        /// A fake answering `markdown` to anything.
        fn new(markdown: &'static str) -> Self {
            Self {
                markdown,
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl Agent for Canned {
        fn run(&self, request: &Request) -> Result<Response, Error> {
            self.seen.borrow_mut().push((
                request.prompt().to_owned(),
                request.directory().to_path_buf(),
            ));
            Ok(Response::new(self.markdown))
        }
    }

    /// The other half of a fake: one that never reaches a model. Written as a
    /// separate type rather than a mode flag on [`Canned`] so a test reading
    /// it can see which behaviour it asked for.
    struct Refuses;

    impl Agent for Refuses {
        fn run(&self, _request: &Request) -> Result<Response, Error> {
            Err(Error::NotFound {
                program: "claude".to_owned(),
            })
        }
    }

    /// One of every variant, for the properties that hold across all of them.
    fn every_variant() -> Vec<Error> {
        vec![
            Error::NotFound {
                program: "claude".to_owned(),
            },
            Error::Failed {
                code: Some(2),
                stderr: "Invalid API key\nRun `claude login` first\n".to_owned(),
            },
            Error::Failed {
                code: None,
                stderr: String::new(),
            },
            Error::EmptyOutput,
            Error::TimedOut {
                after: Duration::from_mins(5),
            },
            Error::Io {
                source: std::io::Error::other("broken pipe"),
            },
        ]
    }

    #[test]
    fn a_fake_agent_answers_with_canned_markdown() {
        let agent = Canned::new("# engine\n\nCore engine for warlock.\n");
        let request = Request::new("describe this directory", "crates/warlock-engine");

        let response = agent.run(&request).expect("the fake always answers");

        assert_eq!(response.text(), "# engine\n\nCore engine for warlock.\n");
        assert_eq!(
            response.clone().into_text(),
            response.text(),
            "owned and borrowed are the same text"
        );
        assert_eq!(
            agent.seen.borrow().as_slice(),
            [(
                "describe this directory".to_owned(),
                PathBuf::from("crates/warlock-engine"),
            )],
            "the request reaches the implementation whole"
        );
    }

    #[test]
    fn the_same_agent_runs_more_than_one_pass() {
        // `&self`, not `&mut self`: pacting a subtree is many passes through
        // one implementation.
        let agent = Canned::new("# module\n");
        for directory in ["crates/warlock-engine", "crates/warlock-tui"] {
            let response = agent
                .run(&Request::new("summarise", directory))
                .expect("the fake always answers");
            assert_eq!(response.text(), "# module\n");
        }
        assert_eq!(agent.seen.borrow().len(), 2);
    }

    #[test]
    fn a_failing_agent_comes_back_as_an_error_not_a_document() {
        let error = Refuses
            .run(&Request::new("summarise", "."))
            .expect_err("this fake reaches nothing");

        assert!(matches!(error, Error::NotFound { .. }), "{error:?}");
    }

    #[test]
    fn a_request_carries_its_prompt_and_directory() {
        let request = Request::new("summarise this module", "/repo/crates/engine");

        assert_eq!(request.prompt(), "summarise this module");
        assert_eq!(request.directory(), Path::new("/repo/crates/engine"));
        assert_eq!(request, request.clone());
    }

    #[test]
    fn every_error_variant_displays_on_one_line() {
        for error in every_variant() {
            let rendered = error.to_string();
            assert!(!rendered.is_empty(), "{error:?} says nothing");
            assert!(
                !rendered.contains('\n'),
                "{error:?} renders across lines: {rendered}"
            );
        }
    }

    #[test]
    fn the_missing_binary_is_named_rather_than_an_errno() {
        let error = Error::NotFound {
            program: "claude".to_owned(),
        };
        let rendered = error.to_string();

        assert!(rendered.contains("claude"), "{rendered}");
        assert!(rendered.contains("PATH"), "{rendered}");
        assert!(
            !rendered.contains("os error"),
            "a bare io::Error is exactly what this variant exists to avoid: {rendered}"
        );
    }

    #[test]
    fn a_non_zero_exit_shows_its_status_and_flattens_its_stderr() {
        let error = Error::Failed {
            code: Some(2),
            stderr: "Invalid API key\nRun `claude login` first\n".to_owned(),
        };
        let rendered = error.to_string();

        assert!(rendered.contains('2'), "{rendered}");
        assert!(
            rendered.contains("Invalid API key Run `claude login` first"),
            "multi-line stderr is flattened, not dropped: {rendered}"
        );
    }

    #[test]
    fn an_enormous_stderr_is_excerpted() {
        let error = Error::Failed {
            code: Some(1),
            stderr: "boom ".repeat(10_000),
        };
        let rendered = error.to_string();

        assert!(rendered.chars().count() < 300, "{rendered}");
        assert!(
            rendered.contains('…'),
            "the excerpt says it was cut: {rendered}"
        );
    }

    #[test]
    fn a_timeout_says_how_long_it_waited_and_is_not_an_exit() {
        let error = Error::TimedOut {
            after: Duration::from_mins(5),
        };

        assert!(error.to_string().contains("300"), "{error}");
        assert!(
            !matches!(error, Error::Failed { .. }),
            "a hang is its own answer, not an exit code"
        );
    }

    #[test]
    fn only_the_io_variant_has_a_source() {
        for error in every_variant() {
            let expected = matches!(error, Error::Io { .. });
            assert_eq!(
                error.source().is_some(),
                expected,
                "{error:?} disagrees about having a source"
            );
        }
        let io = Error::Io {
            source: std::io::Error::other("broken pipe"),
        };
        assert_eq!(
            io.source().expect("an io error has a source").to_string(),
            "broken pipe"
        );
    }
}
