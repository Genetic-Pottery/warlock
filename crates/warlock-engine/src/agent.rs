// The transport stays in the binary: this crate spawns no subprocess, so the
// errors a model session can end in are stated here as plain values. No
// `std::process` type crosses the seam — exit codes and stderr arrive as
// `Option<i32>` and `String`.
use std::fmt;
use std::time::Duration;

const STDERR_EXCERPT: usize = 200;

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
// only clue, and empty output means there is nothing to write.
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
            Self::EmptyOutput => write!(f, "the model pass wrote nothing"),
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
    let flattened = crate::fill::flattened(text);
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
