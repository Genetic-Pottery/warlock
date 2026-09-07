//! Every variant prints as a single line, because `main` prints exactly one
//! after the terminal is back and a message wrapping onto a second line in a
//! restored shell looks like a crash. `one_line` is the flattening that rule
//! leans on, and other modules borrow it because the footer is one line too.

use std::path::PathBuf;
use std::{fmt, io};

use warlock_engine::{claude_md, load, manifest, pact, scope, sigils};

use crate::boundary::{blocking_scopes_message, closed_scope_message};

// One vocabulary for the panel and every subcommand rather than one enum
// each: they fail in the same ways and are printed by the same line of `main`,
// so a second enum would be a second wording of the same sentences.
#[derive(Debug)]
pub(crate) enum Error {
    WorkingDirectory {
        source: io::Error,
    },
    Load {
        source: load::Error,
    },
    Problems {
        first: String,
        rest: usize,
    },
    Manifest {
        source: manifest::Error,
    },
    // Kept apart from `Manifest` even though both carry the engine's
    // `manifest::Error`: that one is a file that would not read or write, this
    // one never opens a file at all. A path with no repository-relative form is
    // refused rather than quietly dropped from a listing, because an answer with
    // it left out would tell a script nothing is stale there.
    Unspellable {
        source: manifest::Error,
    },
    // The boundary is asked before the path is spelled and before the manifest
    // is looked into (see `crate::edits`), so neither this nor `NoPact` below
    // can be prised out of warlock from outside a scope it does not open.
    ClosedScope {
        path: String,
        scope: String,
    },
    // `ClosedScope`'s question aimed downwards, and the only refusal an
    // un-pact has that the other writes do not: coverage walks up, so that one
    // answers whether this machine may act *at* the path, while an un-pact drops
    // every entry below as well and an entry is the only home a scope has.
    // Without this a boundary could be erased by aiming at its parent. Argued in
    // `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`.
    ClosedScopeBelow {
        path: String,
        scopes: Vec<String>,
    },
    // The engine's `scope::Rule` and nothing wrapped around it: the sentence a
    // rule renders as is already the whole answer, and a preamble of warlock's
    // own would be a second voice saying the same thing less precisely.
    Scope {
        rule: scope::Rule,
    },
    NoPact {
        module: String,
    },
    Pact {
        source: pact::Error,
    },
    Failures {
        failed: usize,
        total: usize,
    },
    Cancelled,
    // A refusal rather than a shrug: a `warlock pact` is minutes of somebody's
    // tokens with no panel to press Esc in, and the signal is the only say-when
    // a shell has over it. Free to refuse here — the handler is installed after
    // the boundary is asked and before the first pass is spent.
    Signal {
        source: ctrlc::Error,
    },
    NoRepository {
        start: PathBuf,
        wanted: &'static str,
    },
    ClaudeMd {
        source: claude_md::Error,
    },
    NoHome,
    Prompt {
        source: io::Error,
    },
    Sigil {
        entered: String,
        rule: scope::Rule,
    },
    Sigils {
        source: sigils::Error,
    },
    Terminal {
        source: io::Error,
    },
}

impl Error {
    // Only the first problem is quoted. One unreadable file usually means a
    // whole directory of them, and a message per file would scroll the useful
    // one off the screen; the count says how much was left out.
    pub(crate) fn from_problems(problems: &[load::Problem]) -> Option<Self> {
        let first = problems.first()?;
        Some(Self::Problems {
            first: one_line(&first.to_string()),
            rest: problems.len() - 1,
        })
    }
}

// First line and last, rejoined rather than truncated. A parser's diagnostic is
// laid out for a compiler's output — location, then the offending source with a
// caret under it, then the explanation — and the middle lines mean nothing once
// they are not in a fixed-width block. Dropping the last line instead would
// throw away the only part that says *why*. Single-line messages, which is every
// I/O error and everything this workspace writes itself, come back untouched.
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
            // The walker's own sentence, flattened like the manifest's: it names
            // the directory it could not list and what the filesystem said,
            // which is the whole of what happened.
            Self::Pact { source } => write!(f, "{}", one_line(&source.to_string())),
            // The count, under the lines that named each of them, and the fact
            // a reader most needs next: the run's record is on disk, so the
            // directories that did work are granted and re-running describes
            // the ones that did not. Singular when the run was one directory,
            // because "1 of 1 directories" is a sentence nobody writes.
            Self::Failures { failed, total } => {
                let directories = if *total == 1 {
                    "directory"
                } else {
                    "directories"
                };
                write!(
                    f,
                    "{failed} of {total} {directories} failed — the manifest holds what the \
                     rest earned"
                )
            }
            // What was stopped and what survives it, in that order, because the
            // second half is the thing a reader wonders about a run they killed
            // half way through: the documents that were written are still
            // written and the manifest records them, so the next run picks up
            // from there rather than starting again. The footer says the same
            // two things about the same event over a pact stopped with Esc.
            Self::Cancelled => write!(
                f,
                "the run was cancelled; what it finished first is recorded"
            ),
            // What could not be arranged, then what warlock did about it: the
            // crate's own sentence is "Ctrl-C signal handler already
            // registered" or the system's complaint, neither of which says on
            // its own that no pass was spent — and that is the half a reader at
            // a shell prompt needs, because it is the difference between
            // re-running and going to look at a subtree.
            Self::Signal { source } => write!(
                f,
                "{source}, so no run was started — a pact nobody could stop with Ctrl-C is not \
                 one warlock will spend passes on"
            ),
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
            Self::Pact { source } => Some(source),
            Self::ClaudeMd { source } => Some(source),
            Self::Sigil { rule, .. } | Self::Scope { rule } => Some(rule),
            Self::Sigils { source } => Some(source),
            Self::Signal { source } => Some(source),
            // No source, and there is none to have: a boundary this machine
            // does not hold, and a directory nobody has pacted, are facts about
            // two files agreeing rather than failures anything underneath
            // reported.
            Self::Problems { .. }
            | Self::NoRepository { .. }
            | Self::NoHome
            | Self::ClosedScope { .. }
            | Self::ClosedScopeBelow { .. }
            | Self::NoPact { .. }
            // Nor here, and there could not be one: a run's failures are N
            // errors rather than one, they have already been printed in full,
            // and picking a first to be "the" cause would be the summary
            // pretending to be a failure. A cancel has no cause underneath it
            // at all — somebody pressed Ctrl-C.
            | Self::Failures { .. }
            | Self::Cancelled => None,
        }
    }
}

impl From<io::Error> for Error {
    // Everything reached by `?` once the terminal is up is the terminal:
    // entering raw mode, drawing a frame, reading an event. The load path names
    // its own errors and never comes through here.
    fn from(source: io::Error) -> Self {
        Self::Terminal { source }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use warlock_engine::{claude_md, manifest, scope, sigils};

    use super::{Error, one_line};
    // The tails themselves, not copies of them: these used to be re-typed here
    // as literals, so rewording either original left this suite passing on a
    // sentence nothing said any more.
    use crate::standing::{FOR_CLAUDE_MD, FOR_SIGILS};

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
                source: manifest::Error::Io {
                    path: PathBuf::from("/repo/.warlock/pacts.toml"),
                    source: std::io::Error::other("boom"),
                },
            },
            Error::NoRepository {
                start: PathBuf::from("/elsewhere"),
                wanted: FOR_CLAUDE_MD,
            },
            Error::Unspellable {
                source: manifest::Error::PathOutsideRoot {
                    root: PathBuf::from("/repo"),
                    path: PathBuf::from("/elsewhere"),
                },
            },
            Error::Unspellable {
                source: manifest::Error::NonUtf8Path {
                    path: PathBuf::from("/repo/odd"),
                },
            },
            Error::NoRepository {
                start: PathBuf::from("/elsewhere"),
                wanted: FOR_SIGILS,
            },
            Error::ClaudeMd {
                source: claude_md::Error::Write {
                    path: PathBuf::from("/repo/CLAUDE.md"),
                    source: std::io::Error::other("boom"),
                },
            },
            Error::ClaudeMd {
                source: claude_md::Error::NotText {
                    path: PathBuf::from("/repo/CLAUDE.md"),
                },
            },
            Error::NoHome,
            Error::Prompt {
                source: std::io::Error::other("boom"),
            },
            Error::Sigil {
                entered: "Data Plane!".to_owned(),
                rule: scope::Rule::Character { character: '!' },
            },
            Error::Sigils {
                source: sigils::Error::NotFound {
                    path: PathBuf::from("/home/someone/.warlock/repo-abc/config.toml"),
                },
            },
            Error::Sigils {
                source: sigils::Error::Io {
                    path: PathBuf::from("/home/someone/.warlock/repo-abc/config.toml"),
                    source: std::io::Error::other("boom"),
                },
            },
            Error::ClosedScope {
                path: "crates/engine".to_owned(),
                scope: "data-plane".to_owned(),
            },
            Error::Scope {
                rule: scope::Rule::Empty,
            },
            Error::NoPact {
                module: "crates/engine".to_owned(),
            },
            Error::Failures {
                failed: 3,
                total: 12,
            },
            Error::Failures {
                failed: 1,
                total: 1,
            },
            Error::Cancelled,
            Error::Signal {
                source: ctrlc::Error::MultipleHandlers,
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
            source: manifest::Error::Io {
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
            source: manifest::Error::PathOutsideRoot {
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
            rule: scope::Rule::Character { character: '!' },
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
                rule: scope::Rule::Empty
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
            source: sigils::Error::Io {
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
            source: claude_md::Error::Write {
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
