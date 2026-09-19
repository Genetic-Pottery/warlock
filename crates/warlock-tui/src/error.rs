//! Every variant prints as a single line, because `main` prints exactly one
//! after the terminal is back and a message wrapping onto a second line in a
//! restored shell looks like a crash. `one_line` is the flattening that rule
//! leans on, and other modules borrow it because the footer is one line too.

use std::path::PathBuf;
use std::{fmt, io};

use warlock_engine::{claude_md, keys, load, manifest, pact, route, scope, sigils};

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
    // The one failure here that is another program's: the selection owner, the
    // compositor's helper, or nothing at all on a session with no display.
    // Source-carrying like `Signal` above and for the same reason — the crate's
    // own sentence is the whole of what went wrong, and what it cost is
    // warlock's to say.
    Clipboard {
        source: arboard::Error,
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
    // The name a key is stored under, judged before anything is read — so this
    // is the one key failure raised with no line typed. It carries the name and
    // the rule and could not carry a value if it wanted to.
    KeyName {
        name: String,
        rule: scope::Rule,
    },
    // A line that was typed and held nothing, which is not the EOF that changes
    // nothing: `name` is what a person asked to store under, never what they
    // typed.
    NoKey {
        name: String,
    },
    // A name no key is stored under, raised by `key use` before it writes and
    // by `key forget` after the engine reports it removed nothing. `wanted` is
    // one of [`crate::key`]'s tails, following `NoRepository`: the fact is one
    // fact and only what it cost differs between the two verbs. A store that is
    // missing entirely is this refusal as well — it holds no key by that name
    // either, and a sentence about an absent file answers a question nobody
    // asked.
    UnknownKey {
        name: String,
        wanted: &'static str,
    },
    // The engine's key store, and the one variant here that wraps an error from
    // a module holding secrets. It is safe to carry and to print because
    // `keys::Error` carries paths, names and a line number and never a value —
    // which is also why `keys::Unparseable` exists instead of the TOML parse
    // error, whose diagnostic would quote the line the key is on.
    Keys {
        source: keys::Error,
    },
    // Only the engine's *reporting* route errors reach this, and neither of
    // them is an absence: `route_facts` answers "no scope", "no record",
    // "nothing bound" and "no such key" as values, so what is left is a path
    // with no place in the manifest and a key store that will not read. Which
    // is why `warlock check` can carry this variant and still exit 0 on every
    // route a person has yet to finish setting up.
    Route {
        source: route::Error,
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
            // The crate's sentence about the clipboard, with what it cost on
            // the end of it, in `Signal`'s shape: "the native clipboard is not
            // accessible due to being held by another party" does not say by
            // itself that the text a reader asked for is not on the clipboard,
            // and that is the half they need. Flattened, because an unknown
            // failure carries whatever some other program printed.
            Self::Clipboard { source } => {
                write!(f, "nothing was copied: {}", one_line(&source.to_string()))
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
            // `Sigil`'s shape, with the other half of what a reader needs on the
            // end: the rule is a sentence of its own, and what is worth adding
            // to it is that the prompt never happened, so no key is anywhere.
            Self::KeyName { name, rule } => {
                write!(f, "`{name}` is not a key name, so nothing was read: {rule}")
            }
            // Names the pipe rather than only the emptiness, because the person
            // who typed a blank line at this prompt is the person who has a key
            // in a file and does not want it on their screen.
            Self::NoKey { name } => write!(
                f,
                "nothing was typed, so no key is stored under `{name}`: pipe one in with \
                 `warlock key add {name} < key.txt`"
            ),
            // The one place a reader is sent, because the names are the one
            // thing `warlock key list` will tell them and a misremembered name
            // is what this usually is.
            Self::UnknownKey { name, wanted } => write!(
                f,
                "this machine holds no key called `{name}`, so there was nothing to {wanted}: \
                 `warlock key list` names the keys it does hold"
            ),
            // Flattened like the sigil config's: a store that will not parse
            // carries a position rather than the parser's own diagnostic, and
            // the rest is the filesystem's, which can still run to two lines.
            Self::Keys { source } => write!(f, "{}", one_line(&source.to_string())),
            // Flattened for the same reason again: what reaches here wraps a
            // manifest or key-store error whose text can run to two lines.
            Self::Route { source } => write!(f, "{}", one_line(&source.to_string())),
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
            Self::Sigil { rule, .. } | Self::Scope { rule } | Self::KeyName { rule, .. } => {
                Some(rule)
            }
            Self::Sigils { source } => Some(source),
            Self::Keys { source } => Some(source),
            Self::Route { source } => Some(source),
            Self::Signal { source } => Some(source),
            Self::Clipboard { source } => Some(source),
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
            // Nor here: a prompt answered with a blank line, and a name nobody
            // stored a key under, are a person and not a failure underneath.
            | Self::NoKey { .. }
            | Self::UnknownKey { .. }
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
#[path = "tests/error.rs"]
mod tests;
