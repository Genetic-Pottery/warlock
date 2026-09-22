//! Every variant prints as a single line, because `main` prints exactly one
//! after the terminal is back and a message wrapping onto a second line in a
//! restored shell looks like a crash. `one_line` is the flattening that rule
//! leans on, and other modules borrow it because the footer is one line too.

use std::path::{Path, PathBuf};
use std::{fmt, io};

use warlock_engine::{claude_md, filed, filing, keys, load, manifest, pact, route, scope, sigils};
use warlock_tui::{BriefError, LinearError};

use crate::boundary::{blocking_scopes_message, closed_scope_message};
use crate::cut::listed;

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
    // The three refusals `warlock scope add` has about a `[[scope]]` record,
    // and all three are raised past the boundary for `NoPact`'s reason: which
    // of them applies depends on what the manifest already records, and that is
    // a fact about the inside of a manifest a closed scope must not leak.
    //
    // A name nothing records is written with its record or not at all, so the
    // missing flags are carried rather than the one that was noticed first: the
    // fix is to retype the command, and a refusal naming a flag at a time is
    // three runs.
    UnrecordedScope {
        scope: String,
        missing: Vec<&'static str>,
    },
    // Blank kept apart from missing even though the fix rhymes: a flag that was
    // never passed and a flag passed an empty string are different mistakes,
    // and saying "you did not pass `--team`" to somebody who just typed
    // `--team ''` sends them looking for a shell problem they do not have.
    BlankRecord {
        flags: Vec<&'static str>,
    },
    // The other side of the same rule the `s` key follows: a record already in
    // the file is never rewritten, merged or deleted from here. A flag passed
    // at one is refused rather than dropped, because a value silently ignored
    // is a run that believes it wrote something it did not.
    RecordedScope {
        scope: String,
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
    // Which board a push files to, and every way that question has no single
    // answer: nothing held, nothing recorded, several candidates, a name that
    // is not one of them, an unbound checkout, a bound name the store has never
    // heard of. One variant for all of them because the engine has already
    // worded them and this carries the sentence rather than re-writing it —
    // `filing::Error` wraps the two key refusals from `route.rs` for that same
    // reason, and a person meets those first from `warlock check`.
    Filing {
        source: filing::Error,
    },
    // A file that is not a brief: absent, unreadable, untitled, or missing a
    // section of the repository's shape. Raised before a socket is opened, like
    // every other refusal a push has.
    Brief {
        source: BriefError,
    },
    Filed {
        source: filed::Error,
    },
    // The URL is the point of this refusal: a brief is filed once, so the
    // answer to pushing it again is the address of the project it already made.
    AlreadyFiled {
        path: String,
        url: String,
    },
    // The team key is the manifest's, so the file to fix is named with it. A
    // team Linear does not know is not a client failure and `linear.rs` does not
    // word it: that module holds no sentence about `.warlock/pacts.toml`.
    UnknownTeam {
        team: String,
        path: PathBuf,
    },
    Linear {
        source: LinearError,
    },
    // A project that exists with no record of it, which is the one failure here
    // that comes after something was spent. It carries the URL because that is
    // the thing nothing else on the machine now knows.
    Unfiled {
        url: String,
        // Boxed for the reason `filed::Record` boxes the parser's error: the
        // variant is this enum's largest otherwise, and every `Result<_, Error>`
        // in the workspace — which is most of them — would be widened by a
        // failure only one command can reach.
        source: Box<filed::Error>,
    },
    // The three refusals a pull has before it reads anything: a brief nothing
    // filed, an id the workspace does not have, and a project that is not
    // planned. Each names what it read and where that came from, because all
    // three are a file on this machine and the board disagreeing rather than a
    // failure underneath.
    NoRecord {
        path: String,
    },
    UnknownProject {
        id: String,
        path: PathBuf,
    },
    // `None` is a project with no status at all, which a workspace without the
    // status warlock files into leaves behind: it is not `Planned` either, so it
    // refuses with the rest, and the sentence has to be able to say that as well
    // as name a wrong one.
    NotPlanned {
        path: String,
        status: Option<String>,
    },
    // The team key again, because a workflow state belongs to a team rather
    // than to the workspace: a `Backlog` on one team says nothing about
    // another, so the refusal is only useful with the team in it. Raised before
    // any issue is created, so what it cost is nothing.
    NoBacklog {
        team: String,
    },
    // `Unfiled`'s shape for the same event one layer down: the issues exist,
    // nothing on this machine records them, and the identifiers are what
    // nothing else now knows. They are carried rather than only printed because
    // the caller filing the next slice has no `out` to read them back off.
    Uncut {
        issues: Vec<String>,
        // Boxed for `Unfiled`'s reason.
        source: Box<filed::Error>,
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

// `` `--a`, `--b` and `--c` ``, in [`writing::missing_line`](crate::writing)'s
// shape: a refusal that names more than one thing is read as a sentence, and a
// comma before the last of them would be read as a fourth flag.
//
// An empty slice is the empty string and no caller reaches it: both variants
// that call this are raised with at least one flag named, since a list of
// nothing wrong with the command is not a refusal.
fn naming(flags: &[&str]) -> String {
    let named: Vec<String> = flags.iter().map(|flag| format!("`{flag}`")).collect();
    let Some((last, rest)) = named.split_last() else {
        return String::new();
    };
    if rest.is_empty() {
        last.clone()
    } else {
        format!("{} and {last}", rest.join(", "))
    }
}

// The fact first, then the whole of the fix: the flags that were not given,
// named together so the command can be retyped once rather than three times.
// "nothing was written" is on the end of all three of these sentences, because
// a refusal about a record is the one place a reader might assume the scope
// went in and only the record did not.
fn unrecorded_message(scope: &str, missing: &[&str]) -> String {
    format!(
        "nothing records `{scope}` yet, so nothing was written: writing a scope by that name \
         needs a team, a review state and a label, given as {}",
        naming(missing)
    )
}

// The record window's own wording about an empty field — `a team cannot be
// blank` — said about whichever flags were handed one here.
fn blank_message(flags: &[&str]) -> String {
    format!("{} cannot be blank, so nothing was written", naming(flags))
}

// What the file already holds, and then both roads out: the scope on its own is
// one flagless run, and the record is the file's to edit. Warlock does not
// rewrite, merge or delete a record from here, so there is no flag to reach for.
fn recorded_message(scope: &str) -> String {
    format!(
        "`{scope}` already has a record in `.warlock/pacts.toml`, and warlock does not rewrite \
         one: run without `--team`, `--review-state` and `--label` to write the scope, or edit \
         the file to change the record"
    )
}

// The URL leads, because it is what the reader wants from this line: the brief
// is filed, and the road from here is the project rather than a second push.
fn already_filed_message(path: &str, url: &str) -> String {
    format!(
        "`{path}` is already filed at {url}, so nothing was sent: a brief that changed after it \
         was filed is edited where it is"
    )
}

// Names the file the key is written in rather than only the key: a `team` in a
// `[[scope]]` record is a Linear team key — `WAR` — and the usual cause of this
// is a record carrying a team's *name*.
fn unknown_team_message(team: &str, path: &Path) -> String {
    format!(
        "Linear knows no team with the key `{team}`, so nothing was filed: the `[[scope]]` \
         record in `{}` is where that key is written",
        path.display()
    )
}

// The URL leads again, and for more than the last one's reason: the project
// exists, nothing on this machine records it, and this line is the last place
// that address appears. Flattened like the manifest's — filed records are TOML,
// and a file that will not parse carries the parser's diagnostic.
fn unfiled_message(url: &str, source: &filed::Error) -> String {
    format!(
        "the project is at {url}, and warlock could not record it: {}",
        one_line(&source.to_string())
    )
}

// The command that would make the record is the whole of the fix, so it is
// spelled out with the path already in it rather than named in the abstract.
fn no_record_message(path: &str) -> String {
    format!(
        "nothing in `.warlock/filed.toml` records `{path}`, so there is no project to read: \
         `warlock push {path}` files it"
    )
}

// Named against the file the id is written in, like the unknown team above: the
// usual cause is a record for a project somebody deleted in Linear, and that
// file is the only place this machine keeps the id.
fn unknown_project_message(id: &str, path: &Path) -> String {
    format!(
        "Linear knows no project with the id `{id}`, so nothing was read: the record in `{}` is \
         where that id is written",
        path.display()
    )
}

fn not_planned_message(path: &str, status: Option<&str>) -> String {
    let found = match status {
        Some(status) => format!("is in `{status}`"),
        None => "has no status".to_owned(),
    };
    format!(
        "the project filed for `{path}` {found} rather than `Planned`, so nothing was read: \
         warlock reads a project back once it is planned"
    )
}

// Named against the team and against Linear's own settings, like the unknown
// team above: the state is the team's, so a workspace that files its issues
// into a column spelled something else is a workflow to edit rather than a
// warlock to configure.
fn no_backlog_message(team: &str) -> String {
    format!(
        "the team `{team}` has no workflow state called `Backlog`, so no issue was created: a cut \
         slice is filed into that state, and the team's workflow in Linear is where it is named"
    )
}

// The identifiers lead for the URL's reason in `unfiled_message`: the issues
// exist, nothing on this machine records them, and this line is the last place
// they are named. Flattened for that function's reason as well.
fn uncut_message(issues: &[String], source: &filed::Error) -> String {
    format!(
        "the issues {} were created, and warlock could not record them: {}",
        listed(issues),
        one_line(&source.to_string())
    )
}

impl fmt::Display for Error {
    // Over the pedantic line count because it is one arm per variant and the
    // enum is the whole vocabulary: the split the lint asks for would put half
    // the sentences behind a name, and a reader asking what warlock says about
    // one failure would have two places to look. The wordings themselves are
    // already free functions above.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per variant, and every variant of this enum prints"
    )]
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
            // The three record refusals, worded below rather than here: they
            // are one question asked three ways and read as a set, and the
            // boundary's two sentences already reach this match through a
            // function of their own.
            Self::UnrecordedScope { scope, missing } => {
                write!(f, "{}", unrecorded_message(scope, missing))
            }
            Self::BlankRecord { flags } => write!(f, "{}", blank_message(flags)),
            Self::RecordedScope { scope } => write!(f, "{}", recorded_message(scope)),
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
            // The engine's sentence and nothing around it, for `Scope`'s
            // reason: each of these already says what is missing and what
            // writes it, and two of them are `route.rs`'s own words about a
            // key, which a person has already met from `warlock check`.
            // Flattened because the key store and the manifest underneath can
            // carry a parser's diagnostic.
            Self::Filing { source } => write!(f, "{}", one_line(&source.to_string())),
            // The reader's own sentence about the document, which already ends
            // in what it cost — nothing was pushed.
            Self::Brief { source } => write!(f, "{source}"),
            // Flattened like the manifest's: filed records are TOML and a file
            // that will not parse carries the parser's diagnostic.
            Self::Filed { source } => write!(f, "{}", one_line(&source.to_string())),
            // The three push refusals with wording of their own, said below
            // rather than here for the record refusals' reason.
            Self::AlreadyFiled { path, url } => write!(f, "{}", already_filed_message(path, url)),
            Self::UnknownTeam { team, path } => write!(f, "{}", unknown_team_message(team, path)),
            // Flattened for the transport's sake: Linear's own refusals are one
            // line, and what a socket failure carries is whatever the network
            // stack said.
            Self::Linear { source } => write!(f, "{}", one_line(&source.to_string())),
            Self::Unfiled { url, source } => write!(f, "{}", unfiled_message(url, source)),
            // The three pull refusals with wording of their own, said below for
            // the push refusals' reason.
            Self::NoRecord { path } => write!(f, "{}", no_record_message(path)),
            Self::UnknownProject { id, path } => {
                write!(f, "{}", unknown_project_message(id, path))
            }
            Self::NotPlanned { path, status } => {
                write!(f, "{}", not_planned_message(path, status.as_deref()))
            }
            // The two cut refusals with wording of their own, said above for
            // the reason the rest are.
            Self::NoBacklog { team } => write!(f, "{}", no_backlog_message(team)),
            Self::Uncut { issues, source } => write!(f, "{}", uncut_message(issues, source)),
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
            Self::Filing { source } => Some(source),
            Self::Brief { source } => Some(source),
            Self::Filed { source } => Some(source),
            Self::Unfiled { source, .. } | Self::Uncut { source, .. } => Some(source.as_ref()),
            Self::Linear { source } => Some(source),
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
            // Nor here: a command line missing a flag, carrying a blank one, or
            // carrying one over a record warlock will not rewrite, is a person
            // and two files agreeing rather than a failure underneath.
            | Self::UnrecordedScope { .. }
            | Self::BlankRecord { .. }
            | Self::RecordedScope { .. }
            // Nor here: a prompt answered with a blank line, and a name nobody
            // stored a key under, are a person and not a failure underneath.
            | Self::NoKey { .. }
            | Self::UnknownKey { .. }
            // Nor here: a brief this repository has already filed, and a team
            // key Linear does not know, are two files disagreeing rather than
            // a failure underneath. The Linear call that answered the second
            // one worked.
            | Self::AlreadyFiled { .. }
            | Self::UnknownTeam { .. }
            // Nor here, for that reason again: a brief no record names, an id
            // the workspace does not have and a project that is not planned are
            // this machine and the board disagreeing. The Linear call that
            // answered the last two worked.
            | Self::NoRecord { .. }
            | Self::UnknownProject { .. }
            | Self::NotPlanned { .. }
            // Nor here: a team whose workflow has no `Backlog` is that same
            // disagreement, and the call that answered it worked.
            | Self::NoBacklog { .. }
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
