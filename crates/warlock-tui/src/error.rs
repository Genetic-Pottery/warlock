//! What can stop warlock showing a tree, said in one line.
//!
//! [`Error`] is the binary's whole error vocabulary: everything that reaches
//! `main` is one of its variants, and every variant prints as a single line,
//! because `main` prints exactly one after the terminal is back and a message
//! wrapping onto a second line in a restored shell is a message that looks
//! like a crash. [`one_line`] is the flattening that rule leans on, and other
//! modules borrow it for the same reason: the footer is one line too.

use std::path::PathBuf;
use std::{fmt, io};

use warlock_engine::{AgentsMdError, LoadError, LoadProblem, ManifestError};

/// Everything that can stop warlock showing a tree, or writing an `AGENTS.md`.
///
/// Richer than the `io::Error` this used to return, because loading brings
/// failures that are not I/O — a directory outside any repository, a manifest
/// that will not parse — and each of them is a different sentence. Every one
/// of those sentences is a single line: `main` prints exactly one, after the
/// terminal is back, and a message wrapping onto a second line in a restored
/// shell is a message that looks like a crash.
///
/// `warlock init` shares the vocabulary rather than having one of its own: it
/// fails in the same two ways the tree does — a working directory outside any
/// repository, and a file that will not write — and it is printed by the same
/// line of `main`, so a second enum would be a second wording of the same
/// sentences.
#[derive(Debug)]
pub(crate) enum Error {
    /// The working directory could not be read, so there is nothing to scope
    /// the run to.
    WorkingDirectory {
        /// What the operating system said.
        source: io::Error,
    },
    /// The engine refused to load a tree for the working directory.
    Load {
        /// Which of the load's fatal cases it was.
        source: LoadError,
    },
    /// The load finished, but could not colour every node it was asked to.
    ///
    /// Kept as finished text rather than as the [`LoadProblem`]s themselves:
    /// what this variant needs to carry is one line, and formatting it at the
    /// point the problems are still in hand keeps [`fmt::Display`] free of the
    /// question of how many of them to mention.
    Problems {
        /// The first problem, as it worded itself: node, file and reason.
        first: String,
        /// How many further problems went unnamed.
        rest: usize,
    },
    /// The pact manifest could not be read at startup, or written when a pact
    /// was toggled.
    ///
    /// The write is the interesting half: a read-only `.warlock/`, a full
    /// disk. It reaches `main` the same way every other failure does, which is
    /// the point — the terminal is restored by the guard on the way out, one
    /// line goes to stderr, and the exit status says it did not work.
    Manifest {
        /// Which of the manifest's cases it was, with the path it names.
        source: ManifestError,
    },
    /// `warlock init` was run somewhere with no repository above it, so there
    /// is no root to write an `AGENTS.md` at.
    ///
    /// The tree's own version of this refusal comes through [`Error::Load`] in
    /// the engine's words; `init` never loads a tree, so it asks
    /// [`repository_root`](warlock_engine::repository_root) directly and words
    /// the same fact here, in the same shape: what was looked for, where the
    /// looking started, and what that means for what was asked.
    NoRepository {
        /// The working directory the search upwards started from.
        start: PathBuf,
    },
    /// `warlock init` could not write the `AGENTS.md`: it is there and cannot
    /// be read, it is not text, or the write itself failed.
    ///
    /// Nothing is half-written when this arrives — the engine writes beside and
    /// renames over — so the file named is either untouched or whole.
    AgentsMd {
        /// Which of the writer's cases it was, with the path it names.
        source: AgentsMdError,
    },
    /// The terminal could not be set up, drawn to, or read from.
    Terminal {
        /// What the terminal said.
        source: io::Error,
    },
}

impl Error {
    /// The error for a load's non-fatal `problems`, or `None` when it had
    /// none.
    ///
    /// Only the first problem is quoted. One unreadable file usually means a
    /// whole directory of them, and a message per file would scroll the useful
    /// one off the screen; the count says how much was left out, and the named
    /// file is enough to go and look at.
    pub(crate) fn from_problems(problems: &[LoadProblem]) -> Option<Self> {
        let first = problems.first()?;
        Some(Self::Problems {
            first: one_line(&first.to_string()),
            rest: problems.len() - 1,
        })
    }
}

/// `message` as a single line: what it says first, and, when it ran to several
/// lines, why it says so last.
///
/// A parser's diagnostic is laid out for a compiler's output — the location on
/// the first line, then the offending source with a caret under it, then the
/// explanation — and those middle lines mean nothing once they are not in a
/// fixed-width block under each other. What survives is the two lines that are
/// sentences: `TOML parse error at line 1, column 5` and the explanation under
/// the caret, which between them say where to look and what is wrong.
///
/// Rejoining rather than truncating, because dropping the last line would
/// throw away the only part that says *why*. Single-line messages — every I/O
/// error, and every error this workspace writes itself — come back untouched.
pub(crate) fn one_line(message: &str) -> String {
    let mut lines = message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let Some(first) = lines.next() else {
        return String::new();
    };
    match lines.next_back() {
        Some(last) => format!("{first}: {last}"),
        None => first.to_owned(),
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkingDirectory { source } => {
                write!(f, "could not read the working directory: {source}")
            }
            // The engine's own wording, which is already the sentence to show
            // a user — flattened, because a manifest that will not parse
            // carries the TOML parser's multi-line diagnostic inside it.
            Self::Load { source } => write!(f, "{}", one_line(&source.to_string())),
            // Flattened for the same reason as a load: the manifest's own
            // errors carry the TOML parser's multi-line diagnostic.
            Self::Manifest { source } => write!(f, "{}", one_line(&source.to_string())),
            // The engine's `.git` wording, with what it cost the caller on the
            // end: this is a refusal to write a file rather than a refusal to
            // draw a tree, and the reader asked for the file.
            Self::NoRepository { start } => write!(
                f,
                "no `.git` directory in `{}` or any of its parents, so there is no \
                 repository root to write `AGENTS.md` at",
                start.display()
            ),
            // Flattened like the two above it: what the filesystem says can run
            // to more than one line, and this prints as one.
            Self::AgentsMd { source } => write!(f, "{}", one_line(&source.to_string())),
            Self::Problems { first, rest: 0 } => write!(f, "{first}"),
            Self::Problems { first, rest } => {
                write!(f, "{first} (and {rest} more like it)")
            }
            Self::Terminal { source } => write!(f, "{source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::WorkingDirectory { source } | Self::Terminal { source } => Some(source),
            Self::Load { source } => Some(source),
            Self::Manifest { source } => Some(source),
            Self::AgentsMd { source } => Some(source),
            Self::Problems { .. } | Self::NoRepository { .. } => None,
        }
    }
}

impl From<io::Error> for Error {
    /// Everything reached by `?` once the terminal is up is the terminal:
    /// entering raw mode, drawing a frame, reading an event. The load path
    /// names its own errors and never comes through here.
    fn from(source: io::Error) -> Self {
        Self::Terminal { source }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use warlock_engine::{AgentsMdError, ManifestError};

    use super::{Error, one_line};

    /// The problem text the engine hands over, standing in for a
    /// [`LoadProblem`](warlock_engine::LoadProblem) — which cannot be built
    /// outside the engine, because the hash error inside it is
    /// `#[non_exhaustive]`. What is under test here is the wrapping, not the
    /// engine's wording.
    const PROBLEM: &str = "`/repo/crates/engine` could not be hashed and is stale: \
                           could not read `/repo/crates/engine/src/lib.rs`, so the \
                           subtree has no hash: permission denied";

    #[test]
    fn a_message_that_is_already_one_line_is_left_alone() {
        assert_eq!(one_line("permission denied"), "permission denied");
        assert_eq!(one_line("  padded  \n"), "padded");
        assert_eq!(one_line(""), "");
    }

    #[test]
    fn a_parser_diagnostic_keeps_where_and_why_and_drops_the_snippet() {
        // Exactly the shape `toml` produces for a manifest that will not
        // parse, wrapped in the engine's own wording.
        let message = "could not read the pact manifest: malformed pact manifest: \
                       TOML parse error at line 1, column 5\n  |\n1 | not a manifest\n  \
                       |     ^\nkey with no value, expected `=`\n";

        assert_eq!(
            one_line(message),
            "could not read the pact manifest: malformed pact manifest: TOML parse \
             error at line 1, column 5: key with no value, expected `=`",
        );
    }

    #[test]
    fn a_load_with_no_problems_is_not_an_error() {
        assert!(Error::from_problems(&[]).is_none());
    }

    #[test]
    fn one_problem_is_reported_as_it_worded_itself() {
        let error = Error::Problems {
            first: PROBLEM.to_owned(),
            rest: 0,
        };

        assert_eq!(error.to_string(), PROBLEM);
    }

    #[test]
    fn further_problems_are_counted_rather_than_listed() {
        let error = Error::Problems {
            first: PROBLEM.to_owned(),
            rest: 3,
        };

        assert_eq!(error.to_string(), format!("{PROBLEM} (and 3 more like it)"));
    }

    #[test]
    fn every_message_is_one_line_so_it_prints_as_one() {
        let errors = [
            Error::WorkingDirectory {
                source: std::io::Error::other("boom"),
            },
            Error::Terminal {
                source: std::io::Error::other("boom"),
            },
            Error::Problems {
                first: PROBLEM.to_owned(),
                rest: 2,
            },
            Error::Manifest {
                source: ManifestError::Io {
                    path: PathBuf::from("/repo/.warlock/pacts.toml"),
                    source: std::io::Error::other("boom"),
                },
            },
            Error::NoRepository {
                start: PathBuf::from("/elsewhere"),
            },
            Error::AgentsMd {
                source: AgentsMdError::Write {
                    path: PathBuf::from("/repo/AGENTS.md"),
                    source: std::io::Error::other("boom"),
                },
            },
            Error::AgentsMd {
                source: AgentsMdError::NotText {
                    path: PathBuf::from("/repo/AGENTS.md"),
                },
            },
        ];

        for error in errors {
            let message = error.to_string();
            assert!(!message.contains('\n'), "{error:?} wrapped: {message}");
            assert!(!message.is_empty(), "{error:?} said nothing");
        }
    }

    #[test]
    fn a_manifest_that_cannot_be_saved_says_so_in_the_engines_words() {
        let error = Error::Manifest {
            source: ManifestError::Io {
                path: PathBuf::from("/repo/.warlock/pacts.toml"),
                source: std::io::Error::other("permission denied"),
            },
        };

        assert_eq!(
            error.to_string(),
            "could not read or write `/repo/.warlock/pacts.toml`: permission denied"
        );
    }

    #[test]
    fn init_outside_a_repository_says_what_was_looked_for_and_where() {
        let error = Error::NoRepository {
            start: PathBuf::from("/elsewhere"),
        };

        assert_eq!(
            error.to_string(),
            "no `.git` directory in `/elsewhere` or any of its parents, so there is \
             no repository root to write `AGENTS.md` at"
        );
    }

    #[test]
    fn an_agents_md_that_cannot_be_written_says_so_in_the_engines_words() {
        let error = Error::AgentsMd {
            source: AgentsMdError::Write {
                path: PathBuf::from("/repo/AGENTS.md"),
                source: std::io::Error::other("permission denied"),
            },
        };

        assert_eq!(
            error.to_string(),
            "could not write `/repo/AGENTS.md`: permission denied"
        );
    }
}
