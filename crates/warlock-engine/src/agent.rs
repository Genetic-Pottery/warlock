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
//! * **This is transport, not payload.** A [`Request`] carries the prompt, the
//!   directory to run it in, that directory's own files ([`File`]), and the
//!   `WARLOCK.md` of each immediate child ([`ChildDocument`]) — the context one
//!   pass is scoped to, and nothing about how that scope was decided. Which
//!   files a walk gathers, what it does with one too large to send, and how any
//!   of it is spelled into prompt text are decisions elsewhere. The fields are
//!   private behind [`Request::new`] and builder methods that only ever add, so
//!   the next thing a pass needs lands here without touching [`Agent::run`] or
//!   any implementation of it.
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
/// answer — anything a later slice needs to send belongs on [`agent::Request`](crate::agent::Request), where
/// adding it breaks nobody.
///
/// `&self` rather than `&mut self` so an implementation can be shared, and the
/// method is free to be called more than once: nothing here says a pass is
/// unrepeatable.
///
/// ```
/// use warlock_engine::{agent, Agent};
///
/// /// The engine's own tests reach a model exactly like this: they don't.
/// struct Canned;
///
/// impl Agent for Canned {
///     fn run(&self, _request: &agent::Request) -> Result<agent::Response, agent::Error> {
///         Ok(agent::Response::new("# engine\n\nCore engine for warlock.\n"))
///     }
/// }
///
/// let request = agent::Request::new("describe this directory", "crates/warlock-engine");
/// let response = Canned.run(&request)?;
///
/// assert!(response.text().starts_with("# engine"));
/// # Ok::<(), warlock_engine::agent::Error>(())
/// ```
pub trait Agent {
    /// Run one pass and return what the model said.
    ///
    /// # Errors
    ///
    /// [`agent::Error`](crate::agent::Error), in the engine's vocabulary rather than the transport's: the
    /// agent command was not found, it exited non-zero, it wrote nothing, it
    /// did not finish in time, or the attempt failed with some other I/O
    /// error.
    fn run(&self, request: &Request) -> Result<Response, Error>;

    /// How many tokens of context a pass on this agent can actually read.
    ///
    /// The one thing the engine cannot work out for itself. A budget exists
    /// because a context window does, and the window is a property of the model
    /// — which is the front end's business, not the engine's: the engine has
    /// never known what it is talking to, and this keeps that true by asking
    /// rather than guessing.
    ///
    /// **Answer low rather than high.** [`fitting`](crate::fitting) turns this into a byte
    /// budget and gives files up until the request meets it, so an answer that
    /// is too large does not fail — it sends more than the model can read, and
    /// whatever silently drops the excess downstream does it with none of the
    /// care this crate takes: no order, no ladder, no [`Problem`](crate::fitting::Problem) naming what
    /// went. Over-reporting the window converts warlock's disclosed policy into
    /// somebody else's undisclosed one, which is the failure the budget is
    /// there to prevent.
    ///
    /// The default is [`DEFAULT_CONTEXT_TOKENS`], deliberately modest, so an
    /// agent that has not thought about it is merely thrifty rather than
    /// wrong.
    fn context_tokens(&self) -> u64 {
        DEFAULT_CONTEXT_TOKENS
    }
}

/// The context window assumed of an [`Agent`] that does not name its own.
///
/// Small on purpose. Every model worth pointing warlock at has a larger window
/// than this, so the cost of the default is a little summarising nobody needed
/// — while the cost of a default set optimistically would be requests quietly
/// too big for the model, which is the failure this number exists to make
/// impossible for an agent whose author never considered it.
pub const DEFAULT_CONTEXT_TOKENS: u64 = 128_000;

/// What one pass needs in order to run: a prompt, where to run it, and the
/// context it is scoped to.
///
/// The context is two lists and one optional document. The directory's own
/// files ([`agent::File`](crate::agent::File)) — its whole listing, each either carrying its bytes,
/// standing in as a name and a size, or standing in as a name, a size and an
/// account of what it contains — the `WARLOCK.md` of each immediate child
/// ([`agent::ChildDocument`](crate::agent::ChildDocument)), which is how a directory learns what is underneath it
/// without reading a single source file down there, and this directory's *own*
/// previous `WARLOCK.md` where it already has one.
///
/// # Why the previous document has a slot of its own
///
/// It used to be one of [`agent::Request::files`](crate::agent::Request::files), listed like any other file, and
/// this doc said it deliberately always would be: giving it a slot was said to
/// bake a refresh workflow into the transport before anything had decided what
/// a refresh was. Both halves of that have expired. A refresh is decided and
/// shipped ([`refresh_subtree`](crate::refresh_subtree)), and the arrangement
/// turned out to have a defect that is not about workflow at all.
///
/// A file in the listing is *evidence*: it is the directory as it currently is.
/// The previous document is a *claim*, written by an earlier pass against code
/// that may since have moved. Handed over as an ordinary file the two are
/// indistinguishable, so a claim carries the authority of evidence — and at any
/// directory holding no source of its own, which is every interior directory of
/// a workspace, there is nothing in the request that can contradict it. A false
/// sentence written once is then re-copied by every later pass, reading more
/// established each time, and the ledger stamps the result granted. Warlock had
/// four such sentences in its own documents when this was found.
///
/// Separating the two is what lets the prompt say which is which, and lets a
/// pass be told to check a carried claim against the files rather than trust
/// its own predecessor.
///
/// The fields are private and reached through [`agent::Request::new`](crate::agent::Request::new), the
/// builder-style `with_*` methods and the accessors. Every widening so far has
/// been additive for exactly that reason: an existing [`Agent`] implementation
/// and every existing call site keep compiling.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Request {
    /// The whole text handed to the model.
    prompt: String,
    /// The directory the pass runs in, so relative paths in the prompt mean
    /// what the model would see if it looked.
    directory: PathBuf,
    /// The files sitting directly in that directory, in whatever order the
    /// caller added them.
    files: Vec<File>,
    /// The `WARLOCK.md` of each immediate child directory that has one. A
    /// child without one contributes no entry.
    child_documents: Vec<ChildDocument>,
    /// This directory's own `WARLOCK.md` as it stands before this pass, where
    /// it has one. `None` for a directory being described for the first time,
    /// which is the whole of the difference between a pact and a refresh as
    /// this transport sees it.
    previous_document: Option<String>,
}

impl Request {
    /// A pass that asks `prompt`, run from `directory`, carrying no files and
    /// no child documents.
    ///
    /// Infallible: nothing is validated here. Whether the directory exists is
    /// the transport's problem, and it reports back as [`agent::Error::Io`](crate::agent::Error::Io). Context
    /// is added afterwards with [`agent::Request::with_files`](crate::agent::Request::with_files) and
    /// [`agent::Request::with_child_documents`](crate::agent::Request::with_child_documents), so a caller that has none — a test,
    /// or a question about a directory rather than about its contents — says
    /// nothing extra.
    ///
    /// ```
    /// use std::path::Path;
    /// use warlock_engine::agent;
    ///
    /// let request = agent::Request::new("summarise this module", "crates/warlock-engine");
    ///
    /// assert_eq!(request.prompt(), "summarise this module");
    /// assert_eq!(request.directory(), Path::new("crates/warlock-engine"));
    /// assert!(request.files().is_empty());
    /// assert!(request.child_documents().is_empty());
    /// ```
    #[must_use]
    pub fn new(prompt: impl Into<String>, directory: impl Into<PathBuf>) -> Self {
        Self {
            prompt: prompt.into(),
            directory: directory.into(),
            files: Vec::new(),
            child_documents: Vec::new(),
            previous_document: None,
        }
    }

    /// The same request with `files` appended to the directory's listing.
    ///
    /// Appends rather than replaces, and can be called more than once: a
    /// caller gathering a directory in passes never has to hold the whole
    /// listing in one iterator. Order is the caller's — nothing here sorts,
    /// because the order files reach a prompt in is the prompt's business.
    ///
    /// ```
    /// use warlock_engine::{agent};
    ///
    /// let request = agent::Request::new("summarise this module", "crates/engine")
    ///     .with_files([agent::File::present("src/lib.rs", *b"//! Core engine.\n")])
    ///     .with_files([agent::File::omitted("Cargo.lock", 4_200_000)])
    ///     .with_files([agent::File::summarised(
    ///         "src/schema.rs",
    ///         900_000,
    ///         "Generated request and response types for the public API.",
    ///     )]);
    ///
    /// assert_eq!(request.files().len(), 3);
    /// assert_eq!(request.files()[0].bytes(), Some(&b"//! Core engine.\n"[..]));
    /// // An omitted file is still listed, by name and size, never truncated.
    /// assert_eq!(request.files()[1].bytes(), None);
    /// assert_eq!(request.files()[1].size(), 4_200_000);
    /// // A summarised file adds an account of its contents — and still no bytes.
    /// assert_eq!(
    ///     request.files()[2].summary(),
    ///     Some("Generated request and response types for the public API."),
    /// );
    /// assert_eq!(request.files()[2].bytes(), None);
    /// assert_eq!(request.files()[2].size(), 900_000);
    /// ```
    #[must_use]
    pub fn with_files(mut self, files: impl IntoIterator<Item = File>) -> Self {
        self.files.extend(files);
        self
    }

    /// The same request with `documents` appended to the child documents.
    ///
    /// Appends rather than replaces, on the same terms as
    /// [`agent::Request::with_files`](crate::agent::Request::with_files).
    ///
    /// ```
    /// use warlock_engine::{agent};
    ///
    /// let request = agent::Request::new("summarise this module", "crates/engine")
    ///     .with_child_documents([agent::ChildDocument::new("src", "# src\n\nThe code.\n")]);
    ///
    /// assert_eq!(request.child_documents()[0].directory(), "src");
    /// assert!(request.child_documents()[0].text().starts_with("# src"));
    /// ```
    #[must_use]
    pub fn with_child_documents(
        mut self,
        documents: impl IntoIterator<Item = ChildDocument>,
    ) -> Self {
        self.child_documents.extend(documents);
        self
    }

    /// The same request carrying `text` as this directory's previous
    /// `WARLOCK.md`.
    ///
    /// Replaces rather than appends, unlike the two `with_*` methods above:
    /// there is one previous document or there is none, and a second call is a
    /// caller correcting itself rather than adding a second opinion.
    ///
    /// ```
    /// use warlock_engine::agent;
    ///
    /// let request = agent::Request::new("describe this module", "crates/engine")
    ///     .with_previous_document("# engine\n\nWhat an earlier pass concluded.\n");
    ///
    /// assert!(request.previous_document().is_some());
    /// // It is not one of the files: the claim and the evidence stay apart.
    /// assert!(request.files().is_empty());
    /// ```
    #[must_use]
    pub fn with_previous_document(mut self, text: impl Into<String>) -> Self {
        self.previous_document = Some(text.into());
        self
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

    /// The files sitting directly in that directory.
    ///
    /// The directory's own `WARLOCK.md` is *not* among them — it is
    /// [`agent::Request::previous_document`](crate::agent::Request::previous_document), for the reason given on the type.
    #[must_use]
    pub fn files(&self) -> &[File] {
        &self.files
    }

    /// The `WARLOCK.md` of each immediate child directory that has one.
    #[must_use]
    pub fn child_documents(&self) -> &[ChildDocument] {
        &self.child_documents
    }

    /// This directory's own `WARLOCK.md` as it stood before this pass, where it
    /// had one.
    #[must_use]
    pub fn previous_document(&self) -> Option<&str> {
        self.previous_document.as_deref()
    }
}

/// One file of the directory a pass is about: its path, and its bytes if they
/// were sent.
///
/// Bytes rather than text, because a directory's files are whatever is in it —
/// a PNG, a binary fixture, a latin-1 CSV — and a type that could only hold
/// UTF-8 would make those unrepresentable rather than merely awkward.
///
/// A file the caller chose not to send is here all the same, as its path and
/// its size ([`agent::File::omitted`](crate::agent::File::omitted)). That is the whole vocabulary for leaving
/// something out: there is no half-sent file, because a truncated source file
/// invites confident wrong conclusions about the part that never arrived,
/// while a name and a size is accurate information a model can document
/// honestly.
///
/// Between the two sits a third state ([`agent::File::summarised`](crate::agent::File::summarised)): a name, a size,
/// and an account of what the file contains, written by an earlier pass that
/// did read the whole thing. It is not a shorter version of the file and not
/// the beginning of it — it is prose *about* the file, which is why it is
/// reached through [`agent::File::summary`](crate::agent::File::summary) and never through [`File::bytes`].
/// Truncation stays forbidden and omit-and-list stays the floor: a summary is
/// something better than a bare name and a size, not something less than the
/// whole file.
///
/// Above the summary, and below the whole file, sits one more
/// ([`agent::File::elided`](crate::agent::File::elided)): the file's own lines, verbatim and in order,
/// with whole regions a documenter has no use for — test bodies — replaced by a
/// marker saying how many lines stood there. It is still not truncation, and
/// the difference is the one that matters: truncation stops at an arbitrary
/// byte and says nothing, while an elision drops named regions on whole-line
/// boundaries and leaves a marker where each one was. Nothing here is
/// paraphrased, so unlike a summary it may be quoted as the file's own text —
/// which is why it comes back through [`agent::File::kept`](crate::agent::File::kept) rather than through
/// [`agent::File::summary`](crate::agent::File::summary), and why it is text rather than bytes: a file that is
/// not UTF-8 has no lines to keep.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct File {
    /// Where the file is, relative to the request's directory, spelled with
    /// forward slashes — the same spelling the manifest uses, so one path
    /// means one thing on every platform.
    path: String,
    /// Its bytes, the size standing in for them, or an account of them.
    content: Content,
}

/// What a [`agent::File`](crate::agent::File) has to say about its contents: all of them, how many there
/// were, or what they amount to.
///
/// Private, and reached through [`agent::File::bytes`], [`File::size`] and
/// [`agent::File::summary`](crate::agent::File::summary), so "omitted" and "summarised" stay bits of the public
/// surface rather than variants callers match on and grow special cases
/// around.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Content {
    /// The file, whole.
    Bytes(Vec<u8>),
    /// The file's size in bytes, its contents left out.
    Omitted(u64),
    /// The file's size in bytes, and the file's own lines with regions a
    /// documenter does not need dropped and marked.
    Elided {
        /// How many bytes the file has on disk — what it costs to store, not
        /// what it costs to send.
        size: u64,
        /// The surviving lines, verbatim, with a marker where each dropped
        /// region was.
        kept: String,
    },
    /// The file's size in bytes, and an account of its contents that is
    /// explicitly not its text — never a prefix, never an excerpt, never
    /// something to quote as what the file says.
    Summarised {
        /// How many bytes the file has on disk, unaffected by how long the
        /// account of it happens to be.
        size: u64,
        /// What an earlier pass over the whole file reported it contains.
        summary: String,
    },
}

impl File {
    /// A file sent whole: `path`, carrying `bytes`.
    #[must_use]
    pub fn present(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            content: Content::Bytes(bytes.into()),
        }
    }

    /// A file listed but not sent: `path`, and the `size` in bytes it has on
    /// disk.
    ///
    /// The size is given rather than measured, because the point of this
    /// constructor is that nobody is holding the bytes.
    #[must_use]
    pub fn omitted(path: impl Into<String>, size: u64) -> Self {
        Self {
            path: path.into(),
            content: Content::Omitted(size),
        }
    }

    /// A file sent with regions dropped: `path`, the `size` in bytes it has on
    /// disk, and `kept` — its own lines, in order, with a marker standing where
    /// each dropped region was.
    ///
    /// The size is the file's size on disk rather than the length of `kept`,
    /// for [`agent::File::summarised`](crate::agent::File::summarised)'s reason exactly: how big the file is and how
    /// much of it was sent are two facts, and collapsing them would hide the
    /// second.
    ///
    /// Every line in `kept` is a line the file really contains. That is what
    /// separates this from truncation and what lets a reader quote it, which a
    /// summary may never be.
    ///
    /// ```
    /// use warlock_engine::agent;
    ///
    /// let file = agent::File::elided(
    ///     "scope.rs",
    ///     40_000,
    ///     "pub fn covering() -> Option<&str> {\n    None\n}\n… 900 lines of test bodies elided …",
    /// );
    ///
    /// assert_eq!(file.size(), 40_000, "the size on disk, not the size sent");
    /// assert!(file.kept().is_some_and(|text| text.contains("pub fn covering")));
    /// assert_eq!(file.summary(), None, "kept text is not prose about the file");
    /// assert!(!file.is_omitted(), "nothing about it is a bare name");
    /// ```
    #[must_use]
    pub fn elided(path: impl Into<String>, size: u64, kept: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            content: Content::Elided {
                size,
                kept: kept.into(),
            },
        }
    }

    /// A file described rather than sent: `path`, the `size` in bytes it has
    /// on disk, and `summary` — an account of its contents written by an
    /// earlier pass that read the whole thing.
    ///
    /// The size is given rather than derived from the summary, because the two
    /// measure different things: the file is as big as it is on disk however
    /// briefly it can be described.
    ///
    /// The summary is prose about the file, not any part of the file. Nothing
    /// reading it back may present it as the file's text — which is why it
    /// comes out of [`agent::File::summary`](crate::agent::File::summary) and [`File::bytes`] still answers
    /// `None`.
    ///
    /// ```
    /// use warlock_engine::agent;
    ///
    /// let file = agent::File::summarised(
    ///     "vendor/schema.json",
    ///     2_400_000,
    ///     "A JSON Schema for the public API: 180 object definitions, no code.",
    /// );
    ///
    /// assert_eq!(file.size(), 2_400_000);
    /// assert!(file.summary().is_some_and(|said| said.contains("JSON Schema")));
    /// assert_eq!(file.bytes(), None, "an account of a file is not its text");
    /// ```
    #[must_use]
    pub fn summarised(path: impl Into<String>, size: u64, summary: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            content: Content::Summarised {
                size,
                summary: summary.into(),
            },
        }
    }

    /// Where the file is, relative to the request's directory.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The file's bytes, or `None` if it was listed or summarised rather than
    /// sent.
    ///
    /// A summarised file answers `None` here like an omitted one: an account
    /// of a file's contents is not its contents, and there is nowhere else it
    /// could be mistaken for them.
    #[must_use]
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.content {
            Content::Bytes(bytes) => Some(bytes),
            Content::Omitted(_) | Content::Elided { .. } | Content::Summarised { .. } => None,
        }
    }

    /// The file's surviving lines, or `None` if it was sent whole, summarised
    /// or merely listed.
    ///
    /// Verbatim text of the file, unlike [`agent::File::summary`](crate::agent::File::summary): what comes back
    /// here may be quoted as what the file says, as long as nothing reads the
    /// absence of a line as the absence of the thing — which is what the
    /// markers in it are for.
    #[must_use]
    pub fn kept(&self) -> Option<&str> {
        match &self.content {
            Content::Elided { kept, .. } => Some(kept),
            Content::Bytes(_) | Content::Omitted(_) | Content::Summarised { .. } => None,
        }
    }

    /// How many bytes the file has — answered whether or not they were sent,
    /// which is what makes an omitted file a fact rather than a hole.
    ///
    /// For a summarised file this is its size on disk, not the length of the
    /// account of it.
    #[must_use]
    pub fn size(&self) -> u64 {
        match &self.content {
            Content::Bytes(bytes) => bytes.len() as u64,
            Content::Omitted(size)
            | Content::Elided { size, .. }
            | Content::Summarised { size, .. } => *size,
        }
    }

    /// What an earlier pass reported this file contains, or `None` if it was
    /// sent whole or merely listed.
    ///
    /// An account of the file, never a piece of it: a caller may report what
    /// it says, and may not quote it as the file's text.
    #[must_use]
    pub fn summary(&self) -> Option<&str> {
        match &self.content {
            Content::Summarised { summary, .. } => Some(summary),
            Content::Bytes(_) | Content::Omitted(_) | Content::Elided { .. } => None,
        }
    }

    /// Whether the file was listed rather than sent.
    ///
    /// A summarised file is *not* an omitted one and answers `false`: nothing
    /// was left out of it, because a pass read the whole thing and what came
    /// back is on [`agent::File::summary`](crate::agent::File::summary). Only a file nobody has anything to say
    /// about — a name and a size — is omitted.
    #[must_use]
    pub fn is_omitted(&self) -> bool {
        matches!(self.content, Content::Omitted(_))
    }
}

/// One immediate child directory's `WARLOCK.md`, as its parent's pass sees it.
///
/// This is how a directory describes what it contains without reading it: the
/// children summarise themselves, and their parent is handed those summaries
/// instead of every source file below. Nothing deeper than the immediate
/// children ever appears — a grandchild is already described by the child's
/// document.
///
/// Text rather than bytes, unlike [`agent::File`](crate::agent::File): this is Warlock's own document,
/// the same string an [`Agent`] handed back as a [`agent::Response`](crate::agent::Response) when the child
/// was pacted, not an arbitrary file that happens to sit in a directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChildDocument {
    /// The child directory, relative to the request's directory, with forward
    /// slashes. The directory rather than the document, because the document's
    /// name is the same for every child and the directory is what a reader
    /// needs to place it.
    directory: String,
    /// The document, verbatim.
    text: String,
}

impl ChildDocument {
    /// The document `text`, belonging to the child directory `directory`.
    #[must_use]
    pub fn new(directory: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            directory: directory.into(),
            text: text.into(),
        }
    }

    /// The child directory, relative to the request's directory.
    #[must_use]
    pub fn directory(&self) -> &str {
        &self.directory
    }

    /// What that child's document says.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
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
/// Hand-rolled like [`manifest::Error`](crate::manifest::Error) and
/// [`hash::Error`](crate::hash::Error), for the same reason: a handful of variants
/// do not pay for an error-handling dependency. Every variant's
/// [`Display`](fmt::Display) is a single line, because these are read in a
/// one-line footer as often as in a log — which is why [`agent::Error::Failed`](crate::agent::Error::Failed)
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
    /// document. Distinct from [`agent::Error::Failed`](crate::agent::Error::Failed) because nothing failed:
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

    use super::{Agent, ChildDocument, Error, File, Request, Response};

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
    fn a_request_needs_no_files_or_child_documents_to_exist() {
        // `new` alone is still a whole request: everything the widening added
        // is optional, so nothing that built one before has to change.
        let request = Request::new("summarise this module", "/repo/crates/engine");

        assert!(request.files().is_empty());
        assert!(request.child_documents().is_empty());
    }

    #[test]
    fn a_request_round_trips_its_files() {
        let request = Request::new("summarise", "/repo/crates/engine").with_files([
            File::present("src/lib.rs", *b"//! Core engine.\n"),
            File::present("logo.png", vec![0x89, b'P', b'N', b'G', 0x00, 0xff]),
        ]);

        let files = request.files();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path(), "src/lib.rs");
        assert_eq!(files[0].bytes(), Some(&b"//! Core engine.\n"[..]));
        assert_eq!(files[0].size(), 17);
        assert!(!files[0].is_omitted());
        assert_eq!(
            files[0].summary(),
            None,
            "a file sent whole needs no account"
        );
        assert_eq!(
            files[1].bytes(),
            Some(&[0x89, b'P', b'N', b'G', 0x00, 0xff][..]),
            "bytes, not text: a file in a directory need not be UTF-8"
        );
    }

    #[test]
    fn an_omitted_file_is_listed_by_name_and_size_with_no_bytes() {
        let request =
            Request::new("summarise", "/repo").with_files([File::omitted("Cargo.lock", 4_200_000)]);

        let file = &request.files()[0];
        assert_eq!(file.path(), "Cargo.lock");
        assert_eq!(file.size(), 4_200_000);
        assert!(file.is_omitted());
        assert_eq!(
            file.bytes(),
            None,
            "never truncated: an omitted file has no bytes at all, not some of them"
        );
        assert_eq!(
            file.summary(),
            None,
            "nobody has read it, so there is nothing to say about it"
        );
    }

    #[test]
    fn a_summarised_file_is_a_name_a_size_and_an_account_of_its_contents() {
        let request = Request::new("summarise", "/repo").with_files([File::summarised(
            "vendor/schema.json",
            2_400_000,
            "A JSON Schema for the public API: 180 object definitions, no code.",
        )]);

        let file = &request.files()[0];
        assert_eq!(file.path(), "vendor/schema.json");
        assert_eq!(
            file.size(),
            2_400_000,
            "the size on disk, not the length of the account of it"
        );
        assert_eq!(
            file.summary(),
            Some("A JSON Schema for the public API: 180 object definitions, no code."),
        );
        assert_eq!(
            file.bytes(),
            None,
            "an account of a file is prose about it, never a piece of it"
        );
        assert!(
            !file.is_omitted(),
            "a summarised file is not an omitted one: a pass read the whole thing"
        );
    }

    #[test]
    fn the_three_states_answer_the_same_four_questions_differently() {
        // One table, so no state can quietly start answering like another.
        let present = File::present("src/lib.rs", *b"//! Core engine.\n");
        let omitted = File::omitted("Cargo.lock", 4_200_000);
        let summarised = File::summarised("Cargo.lock", 4_200_000, "The locked dependency graph.");

        assert_eq!(
            [
                present.bytes().is_some(),
                omitted.bytes().is_some(),
                summarised.bytes().is_some(),
            ],
            [true, false, false]
        );
        assert_eq!(
            [present.size(), omitted.size(), summarised.size()],
            [17, 4_200_000, 4_200_000]
        );
        assert_eq!(
            [present.summary(), omitted.summary(), summarised.summary(),],
            [None, None, Some("The locked dependency graph.")]
        );
        assert_eq!(
            [
                present.is_omitted(),
                omitted.is_omitted(),
                summarised.is_omitted(),
            ],
            [false, true, false]
        );
    }

    #[test]
    fn a_request_round_trips_its_child_documents() {
        let request = Request::new("summarise", "/repo/crates/engine").with_child_documents([
            ChildDocument::new("src", "# src\n\nThe code.\n"),
            ChildDocument::new("tests", "# tests\n"),
        ]);

        let children = request.child_documents();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].directory(), "src");
        assert_eq!(children[0].text(), "# src\n\nThe code.\n");
        assert_eq!(children[1].directory(), "tests");
    }

    #[test]
    fn the_builders_add_rather_than_replace() {
        let request = Request::new("summarise", "/repo")
            .with_files([File::present("a.rs", *b"a")])
            .with_child_documents([ChildDocument::new("one", "# one\n")])
            .with_files([File::omitted("b.bin", 9)])
            .with_child_documents([ChildDocument::new("two", "# two\n")]);

        assert_eq!(
            request
                .files()
                .iter()
                .map(File::path)
                .collect::<Vec<_>>()
                .as_slice(),
            ["a.rs", "b.bin"]
        );
        assert_eq!(
            request
                .child_documents()
                .iter()
                .map(ChildDocument::directory)
                .collect::<Vec<_>>()
                .as_slice(),
            ["one", "two"]
        );
        assert_eq!(request, request.clone(), "and the whole thing is a value");
    }

    #[test]
    fn the_directorys_own_document_is_an_ordinary_file_of_it() {
        // The one assertion that has to keep holding: `WARLOCK.md` gets no slot
        // of its own, so an existing document arrives as a file like any other
        // and no caller can special-case what it cannot find.
        let request = Request::new("summarise", "/repo/crates/engine").with_files([
            File::present("WARLOCK.md", *b"# engine\n"),
            File::present("src/lib.rs", *b"//! Core engine.\n"),
        ]);

        let document = request
            .files()
            .iter()
            .find(|file| file.path() == "WARLOCK.md")
            .expect("the existing document is in the listing");
        assert_eq!(document.bytes(), Some(&b"# engine\n"[..]));
        assert!(
            request
                .child_documents()
                .iter()
                .all(|child| child.directory() != "."),
            "a directory's own document is not one of its children's"
        );
    }

    #[test]
    fn a_request_with_context_still_reaches_an_agent_whole() {
        let agent = Canned::new("# engine\n");
        let request = Request::new("describe this directory", "crates/warlock-engine")
            .with_files([File::present("src/lib.rs", *b"//! Core engine.\n")])
            .with_child_documents([ChildDocument::new("src", "# src\n")]);

        agent.run(&request).expect("the fake always answers");

        assert_eq!(
            agent.seen.borrow().as_slice(),
            [(
                "describe this directory".to_owned(),
                PathBuf::from("crates/warlock-engine"),
            )],
            "widening the request breaks no existing implementation of the trait"
        );
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
