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
}

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
    ///     .with_files([agent::File::omitted("Cargo.lock", 4_200_000)]);
    ///
    /// assert_eq!(request.files().len(), 2);
    /// assert_eq!(request.files()[0].bytes(), Some(&b"//! Core engine.\n"[..]));
    /// // An omitted file is still listed, by name and size, never truncated.
    /// assert_eq!(request.files()[1].bytes(), None);
    /// assert_eq!(request.files()[1].size(), 4_200_000);
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

// There is no truncated file. Sending the first n bytes of a source file was
// rejected because it invites confident wrong conclusions about the part that
// never arrived, where a name and a size is accurate. `Elided` is the file's own
// lines with named regions dropped on line boundaries, which is why it comes
// back through `File::kept` and is never reachable through `File::bytes`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Content {
    Bytes(Vec<u8>),
    Omitted(u64),
    Elided { size: u64, kept: String },
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

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.content {
            Content::Bytes(bytes) => Some(bytes),
            Content::Omitted(_) | Content::Elided { .. } => None,
        }
    }

    #[must_use]
    pub fn kept(&self) -> Option<&str> {
        match &self.content {
            Content::Elided { kept, .. } => Some(kept),
            Content::Bytes(_) | Content::Omitted(_) => None,
        }
    }

    #[must_use]
    pub fn size(&self) -> u64 {
        match &self.content {
            Content::Bytes(bytes) => bytes.len() as u64,
            Content::Omitted(size) | Content::Elided { size, .. } => *size,
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
    NotFound { program: String },
    Failed { code: Option<i32>, stderr: String },
    EmptyOutput,
    TimedOut { after: Duration },
    Io { source: std::io::Error },
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
#[path = "tests/agent.rs"]
mod tests;
