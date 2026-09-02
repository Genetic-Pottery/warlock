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

use warlock_engine::{ClaudeMdError, LoadError, LoadProblem, ManifestError, ScopeRule, SigilError};

use crate::session::{blocking_scopes_message, closed_scope_message};

/// Everything that can stop warlock showing a tree, or writing a `CLAUDE.md`.
///
/// Richer than the `io::Error` this used to return, because loading brings
/// failures that are not I/O — a directory outside any repository, a manifest
/// that will not parse — and each of them is a different sentence. Every one
/// of those sentences is a single line: `main` prints exactly one, after the
/// terminal is back, and a message wrapping onto a second line in a restored
/// shell is a message that looks like a crash.
///
/// Every subcommand shares the vocabulary rather than having one each: they
/// fail in the same ways the tree does — a working directory outside any
/// repository, a manifest that will not parse, a file that will not write — and
/// they are printed by the same line of `main`, so a second enum would be a
/// second wording of the same sentences. Where one of them needs a sentence of
/// its own, it is a variant here beside the others: [`Error::ClaudeMd`] is
/// `init`'s, [`Error::NoHome`] and [`Error::Sigil`] are `config`'s,
/// [`Error::Unspellable`] belongs to the queries — the two listings and the
/// check, which refuse a path with no repository-relative form on the same
/// grounds and in the same words — and [`Error::ClosedScope`],
/// [`Error::ClosedScopeBelow`], [`Error::Scope`] and [`Error::NoPact`] belong to
/// the headless writes, which refuse a boundary this machine does not hold in
/// the footer's own words, an un-pact that would drop one it does not hold in
/// the footer's other ones, a scope that is not one in the engine's, and a
/// directory nobody has pacted in the manifest's.
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
    /// A path a query was pointed at, or a directory it would have listed, has
    /// no form the manifest could spell: it is outside the repository root, or
    /// some component of it is not valid UTF-8.
    ///
    /// A listing prints repository-root-relative paths, so a directory it
    /// cannot spell is a directory it cannot report — and an answer with that
    /// directory quietly left out would tell a script that nothing is stale
    /// there, which is the one thing warlock does not know about it. So it is a
    /// refusal with an exit status rather than a line dropped.
    ///
    /// `warlock check` refuses the same path on the same grounds, and the
    /// reasoning is
    /// [`scope_covering`](warlock_engine::scope_covering)'s own: a path the
    /// manifest cannot spell is not an unscoped path, it is one this manifest
    /// has nothing whatever to say about, and reporting it as `scope: null`
    /// would tell a script it is open to anyone.
    ///
    /// Kept apart from [`Error::Manifest`] even though both carry the engine's
    /// [`ManifestError`]: that one is a file that would not read or write, and
    /// this one never opens a file at all.
    Unspellable {
        /// Which of the engine's two path cases it was, naming the path.
        source: ManifestError,
    },
    /// A headless write was pointed at a path inside a boundary this machine's
    /// sigils do not open.
    ///
    /// The one refusal in this enum that is about *who* rather than about what
    /// is on disk, and the only one whose sentence is written somewhere else:
    /// [`closed_scope_message`] is what the TUI's footer says when `p`, `r` or
    /// `s` is refused over the same boundary, and one rule refused in two
    /// registers must not be refused in two wordings. Borrowed rather than
    /// retyped, so the day that sentence changes it changes in one place.
    ///
    /// Nothing has been read past this point and nothing at all has been
    /// written: the boundary is asked before the path is spelled and before the
    /// manifest is looked into (see [`mod@crate::edits`]), so this refusal
    /// cannot have leaked what the manifest holds on the far side of it.
    ClosedScope {
        /// The path, as the manifest spells it: repository-root-relative with
        /// forward slashes, and `.` for the root itself. Never the absolute
        /// path typed, so the sentence reads the way the TUI's does over a row.
        path: String,
        /// The scope covering it — the sigil to go and ask for, which is the
        /// whole social half of the refusal.
        scope: String,
    },
    /// `warlock unpact` was pointed at a subtree carrying boundaries this
    /// machine's sigils do not open.
    ///
    /// [`Error::ClosedScope`]'s question aimed downwards, and the only refusal
    /// in this enum that an un-pact has and the other two writes do not.
    /// Coverage walks up, so the variant above answers whether this machine may
    /// act *at* the path; this one answers what the act would **reach**, which
    /// only an un-pact raises — it drops every entry below as well, and an entry
    /// is the only home a scope has, so without this a boundary could be erased
    /// by aiming at its parent. The decision is argued in
    /// `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`.
    ///
    /// Its sentence is written elsewhere for [`Error::ClosedScope`]'s reason:
    /// [`blocking_scopes_message`] is what the TUI's footer says when `p` is
    /// refused un-pacting-ward over the same subtree, and one rule refused in
    /// two registers must not be refused in two wordings.
    ///
    /// The path has been spelled and the boundary *over* it has already said
    /// yes; nothing has been written, and nothing the manifest holds has been
    /// disclosed beyond the scopes named — which are committed inside the
    /// repository and visible to everyone who clones it.
    ClosedScopeBelow {
        /// The path the un-pact was aimed at, as the manifest spells it. Not the
        /// paths underneath: what a reader must hold to proceed is the scopes,
        /// and `warlock check` is what locates the directories carrying them.
        path: String,
        /// Every distinct scope at or below it that this machine does not open,
        /// deduplicated and in the manifest's own order — see
        /// [`closed_scopes_at_or_below`](warlock_engine::closed_scopes_at_or_below).
        /// Never empty: no blocking scope is nothing to refuse.
        scopes: Vec<String>,
    },
    /// `warlock scope add` was handed something that is not a scope.
    ///
    /// The engine's own [`ScopeRule`] and nothing wrapped around it: the
    /// sentence a rule renders as is already the whole answer — what a scope may
    /// hold, and what this one held instead — and a preamble of warlock's own
    /// would be a second voice saying the same thing less precisely. It is the
    /// line the TUI puts under the scope field, printed where a shell can read
    /// it.
    ///
    /// Nothing has been written when this arrives. The judging happens before
    /// the manifest is rebuilt, and what is judged is the ASCII-lower-cased
    /// text — folding case is the one thing done to it, and there is no
    /// trimming, repairing or splitting anywhere on this road.
    Scope {
        /// The one rule it broke, in the engine's words.
        rule: ScopeRule,
    },
    /// A headless scope write was pointed at a directory the manifest has no
    /// entry for.
    ///
    /// A scope is written on a pact's entry, so a directory with no entry has
    /// nowhere to keep one — the same fact
    /// [`no_pact_message`](crate::scoping) states on the footer when the prompt
    /// is answered over a directory that has been hand-edited out of the file,
    /// worded for a reader at a shell rather than at a keyboard.
    ///
    /// Only ever reached past an *open* boundary: the boundary is asked before
    /// the manifest is looked into (see [`mod@crate::edits`]), so this sentence
    /// — which is a fact about what the manifest holds — cannot be prised out of
    /// warlock from outside a scope it does not open.
    NoPact {
        /// The directory, as the manifest spells it: repository-root-relative
        /// with forward slashes, and `.` for the root itself.
        module: String,
    },
    /// A subcommand was run somewhere with no repository above it, so there is
    /// no root to write a `CLAUDE.md` at or to hold sigils for.
    ///
    /// The tree's own version of this refusal comes through [`Error::Load`] in
    /// the engine's words; neither subcommand loads a tree, so each asks
    /// [`repository_root`](warlock_engine::repository_root) directly and words
    /// the same fact here, in the same shape: what was looked for, where the
    /// looking started, and what that means for what was asked.
    ///
    /// One variant for both subcommands, with the consequence carried as
    /// `wanted`: the sentence up to that point is the same fact about `.git`
    /// twice over, and two variants would be two wordings of it to keep in step.
    NoRepository {
        /// The working directory the search upwards started from.
        start: PathBuf,
        /// What the root was wanted for, as the tail of the sentence: "write
        /// `CLAUDE.md` at", "hold sigils for". A `&'static str` because there
        /// are exactly as many of these as there are subcommands, each spelled
        /// where its subcommand is written.
        wanted: &'static str,
    },
    /// `warlock init` could not write the `CLAUDE.md`: it is there and cannot
    /// be read, it is not text, or the write itself failed.
    ///
    /// Nothing is half-written when this arrives — the engine writes beside and
    /// renames over — so the file named is either untouched or whole.
    ClaudeMd {
        /// Which of the writer's cases it was, with the path it names.
        source: ClaudeMdError,
    },
    /// `warlock config` could not work out where this machine's home directory
    /// is, so it does not know where the sigils would be kept.
    ///
    /// Carries nothing: the two variables that were looked at are the whole of
    /// the fact, and they are named in the sentence rather than stored.
    NoHome,
    /// The line `warlock config` prints its prompt for could not be read.
    ///
    /// Deliberately not [`Error::Terminal`]: `warlock config` never enters the
    /// alternate screen and never leaves cooked mode, so a failure here is
    /// stdin being closed or unreadable — a pipe that broke — rather than
    /// anything the terminal was asked to do.
    Prompt {
        /// What the read said.
        source: io::Error,
    },
    /// Something typed at that prompt is not a sigil.
    ///
    /// Nothing has been written when this arrives: the whole line is judged
    /// before anything is saved, so the set held is the set that was held. The
    /// string is kept as it was typed rather than as it was folded, because
    /// that is what the reader can see on their screen.
    Sigil {
        /// The offending string, exactly as it was entered.
        entered: String,
        /// The one rule it broke, in the engine's words.
        rule: ScopeRule,
    },
    /// The machine-local sigil config could not be read or written.
    ///
    /// Nothing is half-written when this arrives — the engine writes beside and
    /// renames over — and nothing inside the repository is touched either way:
    /// the file named is under the home directory.
    Sigils {
        /// Which of the engine's cases it was, with the path it names.
        source: SigilError,
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
            //
            // One arm for two variants, because the engine's wording is already
            // the sentence to show in both cases and there is nothing either
            // could add to it — a listing has nothing to say beyond
            // "`/elsewhere` is not inside the manifest root `/repo`". What the
            // two do not share is what happened, and that is on the variants
            // themselves rather than in this line.
            Self::Manifest { source } | Self::Unspellable { source } => {
                write!(f, "{}", one_line(&source.to_string()))
            }
            // The footer's own sentence, to the letter: the same fact refused
            // at a keystroke and at a shell prompt says the same thing, names
            // the same scope and points at the same `warlock config`.
            Self::ClosedScope { path, scope } => {
                write!(f, "{}", closed_scope_message(path, scope))
            }
            // The footer's other boundary sentence, to the letter and for the
            // same reason: `p` un-pacting-ward over this subtree is refused by
            // the same engine answer, names the same scopes in the same order,
            // and offers the same two roads out.
            Self::ClosedScopeBelow { path, scopes } => {
                let scopes: Vec<&str> = scopes.iter().map(String::as_str).collect();
                write!(f, "{}", blocking_scopes_message(path, &scopes))
            }
            // The engine's sentence about the one rule that was broken, alone
            // on the line: it says what a scope may be and what this one was,
            // which is the whole of the answer and the whole of the fix.
            Self::Scope { rule } => write!(f, "{rule}"),
            // The manifest's own fact, and then what would help: there is no
            // `warlock pact`, so the road from here to a scope is the `p` key
            // over that directory.
            Self::NoPact { module } => write!(
                f,
                "`{module}` is not in the manifest, so there is no pact to carry a \
                 scope; pact it in warlock first, with `p`"
            ),
            // The engine's `.git` wording, with what it cost the caller on the
            // end: this is a refusal to do the thing that was typed rather than
            // a refusal to draw a tree, and the reader asked for that thing.
            Self::NoRepository { start, wanted } => write!(
                f,
                "no `.git` directory in `{}` or any of its parents, so there is no \
                 repository root to {wanted}",
                start.display()
            ),
            // Flattened like the two above it: what the filesystem says can run
            // to more than one line, and this prints as one.
            Self::ClaudeMd { source } => write!(f, "{}", one_line(&source.to_string())),
            // Says which variables were looked at and what to do about it: a
            // reader whose `HOME` is unset is in an unusual shell and needs the
            // name of the thing to set rather than a fact about warlock.
            Self::NoHome => write!(
                f,
                "neither `HOME` nor `USERPROFILE` is set, so there is no home \
                 directory to keep the sigils for this repository under: set \
                 `HOME` and run `warlock config` again"
            ),
            Self::Prompt { source } => {
                write!(f, "could not read the line that was typed: {source}")
            }
            // The rule is a sentence of its own, so this says what it is about
            // and, because the reader has just typed a whole line, that the rest
            // of that line has not been written either.
            Self::Sigil { entered, rule } => write!(
                f,
                "`{entered}` is not a sigil, so nothing was written: {rule}"
            ),
            // Flattened like the manifest's, and for the same reason: a config
            // that will not parse carries the TOML parser's diagnostic.
            Self::Sigils { source } => write!(f, "{}", one_line(&source.to_string())),
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
            Self::WorkingDirectory { source }
            | Self::Terminal { source }
            | Self::Prompt { source } => Some(source),
            Self::Load { source } => Some(source),
            Self::Manifest { source } | Self::Unspellable { source } => Some(source),
            Self::ClaudeMd { source } => Some(source),
            Self::Sigil { rule, .. } | Self::Scope { rule } => Some(rule),
            Self::Sigils { source } => Some(source),
            // No source, and there is none to have: a boundary this machine
            // does not hold, and a directory nobody has pacted, are facts about
            // two files agreeing rather than failures anything underneath
            // reported.
            Self::Problems { .. }
            | Self::NoRepository { .. }
            | Self::NoHome
            | Self::ClosedScope { .. }
            | Self::ClosedScopeBelow { .. }
            | Self::NoPact { .. } => None,
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

    use warlock_engine::{ClaudeMdError, ManifestError, ScopeRule, SigilError};

    use super::{Error, one_line};

    /// What `warlock init` wants a repository root for, as `main` spells it.
    const FOR_CLAUDE_MD: &str = "write `CLAUDE.md` at";

    /// What `warlock config` wants one for, as `config` spells it.
    const FOR_SIGILS: &str = "hold sigils for";

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
                wanted: FOR_CLAUDE_MD,
            },
            Error::Unspellable {
                source: ManifestError::PathOutsideRoot {
                    root: PathBuf::from("/repo"),
                    path: PathBuf::from("/elsewhere"),
                },
            },
            Error::Unspellable {
                source: ManifestError::NonUtf8Path {
                    path: PathBuf::from("/repo/odd"),
                },
            },
            Error::NoRepository {
                start: PathBuf::from("/elsewhere"),
                wanted: FOR_SIGILS,
            },
            Error::ClaudeMd {
                source: ClaudeMdError::Write {
                    path: PathBuf::from("/repo/CLAUDE.md"),
                    source: std::io::Error::other("boom"),
                },
            },
            Error::ClaudeMd {
                source: ClaudeMdError::NotText {
                    path: PathBuf::from("/repo/CLAUDE.md"),
                },
            },
            Error::NoHome,
            Error::Prompt {
                source: std::io::Error::other("boom"),
            },
            Error::Sigil {
                entered: "Data Plane!".to_owned(),
                rule: ScopeRule::Character { character: '!' },
            },
            Error::Sigils {
                source: SigilError::NotFound {
                    path: PathBuf::from("/home/someone/.warlock/repo-abc/config.toml"),
                },
            },
            Error::Sigils {
                source: SigilError::Io {
                    path: PathBuf::from("/home/someone/.warlock/repo-abc/config.toml"),
                    source: std::io::Error::other("boom"),
                },
            },
            Error::ClosedScope {
                path: "crates/engine".to_owned(),
                scope: "data-plane".to_owned(),
            },
            Error::Scope {
                rule: ScopeRule::Empty,
            },
            Error::NoPact {
                module: "crates/engine".to_owned(),
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
    fn a_path_with_no_repository_relative_form_says_which_root_it_is_not_inside() {
        // What `warlock stale /elsewhere` prints. The engine's sentence is
        // already the whole fact, so nothing is wrapped around it.
        let error = Error::Unspellable {
            source: ManifestError::PathOutsideRoot {
                root: PathBuf::from("/repo"),
                path: PathBuf::from("/elsewhere"),
            },
        };

        assert_eq!(
            error.to_string(),
            "`/elsewhere` is not inside the manifest root `/repo`"
        );
    }

    #[test]
    fn init_outside_a_repository_says_what_was_looked_for_and_where() {
        let error = Error::NoRepository {
            start: PathBuf::from("/elsewhere"),
            wanted: FOR_CLAUDE_MD,
        };

        assert_eq!(
            error.to_string(),
            "no `.git` directory in `/elsewhere` or any of its parents, so there is \
             no repository root to write `CLAUDE.md` at"
        );
    }

    #[test]
    fn config_outside_a_repository_says_the_same_thing_about_sigils() {
        // One fact about `.git`, worded once, with what the reader asked for on
        // the end of it.
        let error = Error::NoRepository {
            start: PathBuf::from("/elsewhere"),
            wanted: FOR_SIGILS,
        };

        assert_eq!(
            error.to_string(),
            "no `.git` directory in `/elsewhere` or any of its parents, so there is \
             no repository root to hold sigils for"
        );
    }

    #[test]
    fn a_string_that_is_not_a_sigil_names_itself_and_the_rule_it_broke() {
        let error = Error::Sigil {
            entered: "Data-Plane!".to_owned(),
            rule: ScopeRule::Character { character: '!' },
        };

        assert_eq!(
            error.to_string(),
            "`Data-Plane!` is not a sigil, so nothing was written: a scope holds \
             only lowercase letters, digits, `-` and `_`, and this one holds `!`"
        );
    }

    #[test]
    fn a_scope_that_is_not_one_is_the_engines_rule_and_nothing_wrapped_round_it() {
        // What `warlock scope add crates 'Control Plane'` prints, after the
        // fold: the rule's own sentence, so the shell and the scope field say
        // one thing about one rule. Asked of the judge rather than retyped.
        let rule = warlock_engine::validate_scope("control plane")
            .expect_err("a space is not a scope character");

        assert_eq!(
            Error::Scope { rule: rule.clone() }.to_string(),
            rule.to_string()
        );
        assert_eq!(
            Error::Scope {
                rule: ScopeRule::Empty
            }
            .to_string(),
            "a scope cannot be empty"
        );
    }

    #[test]
    fn a_directory_nobody_pacted_is_named_and_pointed_at_the_key_that_pacts_it() {
        let error = Error::NoPact {
            module: "crates/engine".to_owned(),
        };

        assert_eq!(
            error.to_string(),
            "`crates/engine` is not in the manifest, so there is no pact to carry a \
             scope; pact it in warlock first, with `p`"
        );
    }

    #[test]
    fn no_home_says_which_variables_were_looked_at_and_what_to_do() {
        assert_eq!(
            Error::NoHome.to_string(),
            "neither `HOME` nor `USERPROFILE` is set, so there is no home directory \
             to keep the sigils for this repository under: set `HOME` and run \
             `warlock config` again"
        );
    }

    #[test]
    fn a_sigil_config_that_cannot_be_read_says_so_in_the_engines_words() {
        let error = Error::Sigils {
            source: SigilError::Io {
                path: PathBuf::from("/home/someone/.warlock/repo-abc/config.toml"),
                source: std::io::Error::other("permission denied"),
            },
        };

        assert_eq!(
            error.to_string(),
            "could not read or write `/home/someone/.warlock/repo-abc/config.toml`: \
             permission denied"
        );
    }

    #[test]
    fn a_claude_md_that_cannot_be_written_says_so_in_the_engines_words() {
        let error = Error::ClaudeMd {
            source: ClaudeMdError::Write {
                path: PathBuf::from("/repo/CLAUDE.md"),
                source: std::io::Error::other("permission denied"),
            },
        };

        assert_eq!(
            error.to_string(),
            "could not write `/repo/CLAUDE.md`: permission denied"
        );
    }
}
