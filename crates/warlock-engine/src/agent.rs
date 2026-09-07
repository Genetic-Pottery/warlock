// The port, and nothing behind it. Reaching a model means running the `claude`
// CLI, and this crate promises to spawn no subprocess, so the engine states the
// domain here and the binary owns the transport. Implementing `Agent` in this
// module — however small the implementation looked — is the edit that breaks
// that promise, and with it the rule that the engine's tests need no `claude`,
// no network and no terminal.
//
// The corollary is that no transport type crosses the seam: exit codes and
// stderr arrive as `Option<i32>` and `String`, never as a `std::process` type,
// so a future agent that is not a process still fits.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const STDERR_EXCERPT: usize = 200;

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
    fn run(&self, request: &Request) -> Result<Response, Error>;

    // Answer low rather than high. `fitting` turns this into a byte budget and
    // stops giving files up once the request meets it, so an over-reported
    // window does not fail — it sends more than the model can read and lets
    // something downstream drop the excess with no order, no ladder and no
    // `Problem` naming what went.
    fn context_tokens(&self) -> u64 {
        DEFAULT_CONTEXT_TOKENS
    }
}

// Modest on purpose, for the reason above: an agent whose author never thought
// about the window should end up thrifty, not wrong.
pub const DEFAULT_CONTEXT_TOKENS: u64 = 128_000;

// No slot here for the directory's own previous document, and that absence is
// deliberate. A request used to carry it, labelled as a claim to be checked;
// the instruction was unfollowable whenever the evidence for a claim was not in
// the request — which for anything about another directory is always — so one
// false sentence survived every later pass, read more established each time,
// and the ledger stamped the result granted. Warlock's own documents carried
// several. Every pass is now written from the files and the children's
// documents alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Request {
    prompt: String,
    directory: PathBuf,
    files: Vec<File>,
    child_documents: Vec<ChildDocument>,
}

impl Request {
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
        }
    }

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

    /// ```
    /// use warlock_engine::agent;
    ///
    /// let request = agent::Request::new("placeholder", "crates/engine")
    ///     .with_files([agent::File::present("lib.rs", *b"//! Core engine.\n")])
    ///     .with_prompt("describe these files");
    ///
    /// assert_eq!(request.prompt(), "describe these files");
    /// assert_eq!(request.files().len(), 1, "everything else is untouched");
    /// ```
    #[must_use]
    pub fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = prompt.into();
        self
    }

    #[must_use]
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn files(&self) -> &[File] {
        &self.files
    }

    #[must_use]
    pub fn child_documents(&self) -> &[ChildDocument] {
        &self.child_documents
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct File {
    path: String,
    content: Content,
}

// Four states, and no fifth: there is no truncated file. Sending the first n
// bytes of a source file was rejected because it invites confident wrong
// conclusions about the part that never arrived, where a name and a size is
// accurate. `Elided` is the file's own lines with named regions dropped on
// line boundaries, so it may be quoted; `Summarised` is prose *about* the file
// and may not be, which is why the two come back through separate accessors and
// why neither is reachable through `File::bytes`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Content {
    Bytes(Vec<u8>),
    Omitted(u64),
    Elided {
        size: u64,
        kept: String,
    },
    Summarised {
        size: u64,
        summary: String,
    },
}

impl File {
    #[must_use]
    pub fn present(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            content: Content::Bytes(bytes.into()),
        }
    }

    #[must_use]
    pub fn omitted(path: impl Into<String>, size: u64) -> Self {
        Self {
            path: path.into(),
            content: Content::Omitted(size),
        }
    }

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

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.content {
            Content::Bytes(bytes) => Some(bytes),
            Content::Omitted(_) | Content::Elided { .. } | Content::Summarised { .. } => None,
        }
    }

    #[must_use]
    pub fn kept(&self) -> Option<&str> {
        match &self.content {
            Content::Elided { kept, .. } => Some(kept),
            Content::Bytes(_) | Content::Omitted(_) | Content::Summarised { .. } => None,
        }
    }

    #[must_use]
    pub fn size(&self) -> u64 {
        match &self.content {
            Content::Bytes(bytes) => bytes.len() as u64,
            Content::Omitted(size)
            | Content::Elided { size, .. }
            | Content::Summarised { size, .. } => *size,
        }
    }

    #[must_use]
    pub fn summary(&self) -> Option<&str> {
        match &self.content {
            Content::Summarised { summary, .. } => Some(summary),
            Content::Bytes(_) | Content::Omitted(_) | Content::Elided { .. } => None,
        }
    }

    #[must_use]
    pub fn is_omitted(&self) -> bool {
        matches!(self.content, Content::Omitted(_))
    }
}

// Immediate children only. Descending further was rejected as redundant rather
// than merely expensive: a grandchild is already described by its own parent's
// document, which is what lets a directory say what it contains without any
// pass reading a source file below it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChildDocument {
    directory: String,
    text: String,
}

impl ChildDocument {
    #[must_use]
    pub fn new(directory: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            directory: directory.into(),
            text: text.into(),
        }
    }

    #[must_use]
    pub fn directory(&self) -> &str {
        &self.directory
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Response {
    text: String,
}

impl Response {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }
}

// Not one `Io` bucket, because each of these is a different thing for a caller
// to say or do: a missing `claude` is the ordinary state of a fresh machine, a
// non-zero exit means the model was reached and refused and its stderr is the
// only clue, empty output means there is nothing to write, and a timeout is
// answered by asking for less rather than by reading an error.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    NotFound {
        program: String,
    },
    Failed {
        code: Option<i32>,
        stderr: String,
    },
    EmptyOutput,
    TimedOut {
        after: Duration,
    },
    Io {
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

fn one_line(text: &str) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // Cut on a character boundary rather than at byte `STDERR_EXCERPT`: a stack
    // trace full of arrows and box drawing would otherwise panic the formatter.
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

    struct Canned {
        markdown: &'static str,
        seen: std::cell::RefCell<Vec<(String, PathBuf)>>,
    }

    impl Canned {
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

    struct Refuses;

    impl Agent for Refuses {
        fn run(&self, _request: &Request) -> Result<Response, Error> {
            Err(Error::NotFound {
                program: "claude".to_owned(),
            })
        }
    }

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
    fn a_request_has_no_slot_for_the_directorys_own_previous_document() {
        // The assertion that has to keep holding: a pass is shown the files
        // and the children's documents and nothing its predecessor wrote. The
        // type has no field for one, so the only way a previous document could
        // reach a pass is as an ordinary file — and `fitting` drops it from the
        // listing. What is checked here is the half this type owns.
        let request = Request::new("summarise", "/repo/crates/engine")
            .with_files([File::present("src/lib.rs", *b"//! Core engine.\n")])
            .with_child_documents([ChildDocument::new("src", "# src\n")]);

        assert_eq!(request.files().len(), 1);
        assert!(
            request
                .child_documents()
                .iter()
                .all(|child| child.directory() != "." && child.directory() != "WARLOCK.md"),
            "a directory's own document is not one of its children's"
        );
    }

    #[test]
    fn the_prompt_can_be_replaced_once_the_context_is_known() {
        let request = Request::new("placeholder", "/repo")
            .with_files([File::present("a.rs", *b"a")])
            .with_prompt("the real instructions");
        assert_eq!(request.prompt(), "the real instructions");
        assert_eq!(request.files().len(), 1);
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
