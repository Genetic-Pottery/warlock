//! Every variant prints as a single line, because `main` prints exactly one
//! after the terminal is back and a message wrapping onto a second line in a
//! restored shell looks like a crash. `one_line` is the flattening that rule
//! leans on, and other modules borrow it because the footer is one line too.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::{fmt, io};

use warlock_engine::{
    Manifest, RunStatus, briefs, config, filing, keys, load, manifest, pulls, route, scope,
};

use crate::boundary::{closed_scope_message, unscoped_message};
use crate::brief::{Error as BriefError, ScopeBlockError};
use crate::cut::listed;
use crate::git::{Dirty, Error as GitError};
use crate::linear::Error as LinearError;
use crate::pulling;
use crate::queue::Refusal;
use crate::rescope::ScopeRefusal;
use crate::template::Error as TemplateError;

// The two tails of `Error::UnrecordedScope`'s sentence, spelled here beside the
// variant rather than beside the two verbs that raise it: `Standing`'s `FOR_*`
// tails were re-typed as literals in this file's own tests once, where rewording
// the original failed nothing.
pub(crate) const NO_QUEUE: &str = "there is no queue to read";

pub(crate) const NOT_BOUND: &str = "nothing was written";

pub(crate) const NOT_NAMED: &str = "there is no binding to name";

// One vocabulary for the panel and every subcommand rather than one enum
// each: they fail in the same ways and are printed by the same line of `main`,
// so a second enum would be a second wording of the same sentences.
#[derive(Debug)]
pub enum Error {
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
    // refused rather than quietly dropped from an answer, because an answer with
    // it left out would tell a script nothing is there.
    Unspellable {
        source: manifest::Error,
    },
    // The boundary is asked before the path is spelled and before the manifest
    // is looked into (see `crate::edits`), so neither this nor a `Scope`
    // refusal below can be prised out of warlock from outside a scope it does
    // not open.
    ClosedScope {
        path: String,
        scope: String,
    },
    // `ClosedScope` with no scope to name: a path nothing covers is closed to
    // every machine, so the fix is drawing a scope rather than opening one.
    Unscoped {
        path: String,
    },
    // Every refusal `warlock scope add` and `remove` have past the boundary,
    // worded once by `ScopeRefusal` for both doors.
    Scope {
        refusal: ScopeRefusal,
    },
    Cancelled,
    // The one failure here that is another program's: the selection owner, the
    // compositor's helper, or nothing at all on a session with no display.
    // Source-carrying because the crate's own sentence is the whole of what went
    // wrong, and what it cost is warlock's to say.
    Clipboard {
        source: arboard::Error,
    },
    NoRepository {
        start: PathBuf,
        wanted: &'static str,
    },
    NoHome,
    Prompt {
        source: io::Error,
    },
    ScopeName {
        entered: String,
        rule: scope::Rule,
    },
    Config {
        source: config::Error,
    },
    NotOpen {
        scope: String,
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
    // A recorded scope `warlock key <SCOPE>` or `/key <SCOPE>` could name no
    // stored key for: bound to nothing when `key` is `None`, bound to a name
    // the store does not hold when it is `Some`. An error rather than a line on
    // stdout, so a script reading the bare name off a 0 exit never reads a
    // sentence as the name.
    NoBinding {
        scope: String,
        key: Option<String>,
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
    // answer: no scope open, nothing recorded, several candidates, a name that
    // is not one of them, an unbound checkout, a bound name the store has never
    // heard of. One variant for all of them because the engine has already
    // worded them and this carries the sentence rather than re-writing it —
    // `filing::Error` wraps the two key refusals from `route.rs` for that same
    // reason, and a person meets those first from `warlock check`.
    Filing {
        source: filing::Error,
    },
    // A scope whose record files somewhere warlock cannot file to, carried
    // rather than reworded: the engine's sentence already names the scope and
    // ends in nothing having been sent, which is the whole of what a caller on
    // its way to a board has to say.
    NotLinear {
        source: manifest::NotLinear,
    },
    // A file that is not a brief: absent, unreadable, untitled, or missing a
    // section of the repository's shape. Raised before a socket is opened, like
    // every other refusal a push has.
    Brief {
        source: BriefError,
    },
    // The repository's brief shape, refused before a conversation is opened:
    // `warlock brief` reads the template first, and a file that is there and
    // will not read is never quietly replaced by the built-in default, which is
    // `template.rs`'s own reasoning. Absent is not a failure, so nothing but an
    // unreadable file reaches this.
    Template {
        source: TemplateError,
    },
    // And the file beside it, which says where a written brief goes. Refused
    // beside the template and for its reason: both are read before the first
    // turn, so a conversation is never opened on a setting warlock could not
    // read. A missing file is the default rather than this.
    Briefs {
        source: briefs::Error,
    },
    // The URL is the point of this refusal: a brief is filed once, so the
    // answer to pushing it again is the address of the project it already made.
    AlreadyFiled {
        name: String,
        url: String,
    },
    // The team key is the manifest's, so the file to fix is named with it. A
    // team Linear does not know is not a client failure and `linear.rs` does not
    // word it: that module holds no sentence about `.warlock/scopes.toml`.
    UnknownTeam {
        team: String,
        path: PathBuf,
    },
    Linear {
        source: LinearError,
    },
    // The two refusals a cut has before it reads a slice: a slug the workspace
    // does not have, and a project that is not planned. The scope rides along
    // so the sentence can name the command that lists the right slugs.
    UnknownProject {
        slug: String,
        scope: String,
    },
    // `None` is a project with no status at all, which a workspace without the
    // status warlock files into leaves behind: it is not `Planned` either, so it
    // refuses with the rest, and the sentence has to be able to say that as well
    // as name a wrong one.
    NotPlanned {
        name: String,
        status: Option<String>,
    },
    // A project whose content is not a scope to cut: no `## Scope` heading, a
    // heading with no slices under it, or slices that wait on each other. One
    // variant for all three because `ScopeBlockError` has already worded them
    // and this carries the sentence rather than re-writing it, the way `Filing`
    // above carries the engine's. What it is about is the project's content on
    // the board rather than a file here, so there is no path to name with it.
    ScopeBlock {
        source: ScopeBlockError,
    },
    // Every slice of a project already carrying a note, refused before the
    // repository is read: a run with nothing to draft is not a run that
    // succeeded quietly.
    AllCut {
        name: String,
    },
    // The team key again, because a workflow state belongs to a team rather
    // than to the workspace: a `Backlog` on one team says nothing about
    // another, so the refusal is only useful with the team in it. Raised before
    // any issue is created, so what it cost is nothing.
    NoBacklog {
        team: String,
    },
    // A scope `warlock pull`, `warlock key use` or `warlock key <SCOPE>` was
    // given that no `[[scope]]` record in this repository holds. The recorded
    // names are carried because the usual cause is a near miss against one of
    // them, and they are the answer either way. `cost` is one of [`NO_QUEUE`],
    // [`NOT_BOUND`] and [`NOT_NAMED`], following `UnknownKey`'s `wanted` — one
    // fact costing each verb something different — and it carries the whole
    // clause rather than a verb because the costs are not the same shape of
    // phrase.
    UnrecordedScope {
        scope: String,
        recorded: Vec<String>,
        cost: &'static str,
    },
    // The same question one file along, and the boundary's **3** rather than the
    // ordinary **1**: the scope is recorded and is closed on this machine. Kept
    // apart from `ClosedScope`, which is about a directory that is scoped — here
    // the scope *is* what was named, so that sentence would say it twice — and it
    // carries the open scopes, because the usual cause is a typo against one
    // this machine already has open.
    UnheldScope {
        scope: String,
        held: Vec<String>,
    },
    // The tree a pull was asked to work in, with somebody's uncommitted work in
    // it. Every entry is named rather than a count: the operator's next move is to
    // look at all of them, and `git status` is what they would otherwise run to
    // find out what warlock meant.
    DirtyTree {
        dirty: Vec<Dirty>,
    },
    // Every way the loop itself stopped that is a failure rather than a halt: a
    // `git` that refused, a board that would not answer, a run record that would
    // not write, and the one refusal the loop makes itself — a resumed run whose
    // tree is not clean. Boxed because the variant is this enum's largest
    // otherwise, and every `Result<_, Error>` in the workspace would be widened
    // by a failure only one command can reach.
    Pull {
        source: Box<pulling::Error>,
    },
    // A run that did as much as it could and stopped, which is not the loop
    // failing: the branch holds one commit per finished sub-task and the ticket
    // carries the account. Nothing but the ticket is named here, because that
    // comment is the account and repeating it on one line would be a worse copy.
    Halted {
        ticket: String,
    },
    // The one halt that is a boundary and so the one that spends the **3**: a
    // session wrote outside the scopes open on this machine, nothing was
    // committed, and the tree was left exactly as the session left it.
    Crossed {
        ticket: String,
        subtask: String,
    },
    // A ticket somebody named with `--ticket` that the queue's own rules leave
    // unworkable, refused in the words the chooser would have skipped it with —
    // including the `warlock resume` that releases a halted run.
    NotPulled {
        ticket: String,
        refusal: Refusal,
    },
    // The run records under the home directory, when the directory holding them
    // cannot be listed at all. One record that will not read is not this: the scan
    // hands those back to be named, and the run carries on.
    Runs {
        source: pulls::Error,
    },
    // `warlock resume` asked about a ticket this checkout has never pulled. An
    // ordinary **1** and not the boundary's **3** — a resume writes nothing but a
    // machine-local record, so it has no scope question to fail.
    NoRun {
        ticket: String,
        directory: PathBuf,
    },
    // A resume that found the run and had nothing in it to put back, refused with
    // the record left byte-identical to what was read: the reset is in memory
    // until it is saved, and `PullRun::resume` leaves the run's own status alone
    // when it released nothing. Beside `NoRun` and a **1** for its reason.
    NothingToResume {
        ticket: String,
        status: RunStatus,
        // Which mode was asked, because the same record answers the two
        // differently: a halt held by a blocker alone has nothing for
        // `--failed-only` to release and everything for a plain resume.
        failed_only: bool,
    },
    // `git` itself, for the one command a pull runs before the loop: the tree read
    // to see whether there is anything in it.
    Git {
        source: GitError,
    },
    // The one failure a cut has after something was spent: the issues exist,
    // no note on the project names them, and the identifiers are what nothing
    // else now knows. They are carried rather than only printed because the
    // caller filing the next slice has no `out` to read them back off.
    Uncut {
        issues: Vec<String>,
        // Boxed because the variant is this enum's largest otherwise, and every
        // `Result<_, Error>` in the workspace — which is most of them — would be
        // widened by a failure only one command can reach.
        source: Box<LinearError>,
    },
    Unskipped {
        title: String,
        // Boxed for `Uncut`'s reason.
        source: Box<LinearError>,
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
            first: one_line(first),
            rest: problems.len() - 1,
        })
    }

    // The recorded names are read off the manifest here rather than at the two
    // doors that raise this, so a pull and a `key use` cannot grow two different
    // ideas of what "recorded" means — and so the refusal cannot be built
    // without one of the two tails above.
    pub(crate) fn unrecorded(manifest: &Manifest, scope: &str, cost: &'static str) -> Self {
        Self::UnrecordedScope {
            scope: scope.to_owned(),
            recorded: manifest
                .scopes()
                .iter()
                .map(|record| record.name().to_owned())
                .collect(),
            cost,
        }
    }
}

/// The one question every door to a board asks first: does the scope it was
/// given file somewhere warlock can file to.
///
/// Written once here rather than at each of the doors — `push`, `draft` and
/// `pull`, at the shell and at the panel — because a tracker read a second way
/// is the reading that would let one of them through. Every one of them calls it
/// before it reads a key, opens a client or parks anything behind a window, so a
/// Jira scope costs a manifest lookup and nothing else, which is what the
/// refusal's own "nothing was sent" is promising.
///
/// A scope no `[[scope]]` record holds is `Ok` here and not this refusal: there
/// is no tracker to read off a record that is not there, and the door behind
/// this has the sentence that names the recorded scopes. Nothing reaches a board
/// on that road either.
pub(crate) fn files_to_linear(manifest: &Manifest, scope: &str) -> Result<(), Error> {
    match manifest.scope_record(scope) {
        // The engine's view is the only reader of a record's tracker, so this is
        // the same answer the flows past it get — and its refusal is the
        // engine's sentence, carried by `Error::NotLinear` and not reworded.
        Some(record) => manifest::LinearScope::of(record)
            .map(drop)
            .map_err(Error::from),
        None => Ok(()),
    }
}

// A failed write is dropped on purpose: `warlock pull … | head -1` is a closed
// stdout, and a run of sessions lost over the state of a pipe spends somebody's
// tokens for nothing.
pub(crate) fn say<W: Write>(out: &mut W, fact: &str) {
    drop(writeln!(out, "warlock: {fact}"));
}

// First line and last, rejoined rather than truncated. A parser's diagnostic is
// laid out for a compiler's output — location, then the offending source with a
// caret under it, then the explanation — and the middle lines mean nothing once
// they are not in a fixed-width block. Dropping the last line instead would
// throw away the only part that says *why*. Single-line messages, which is every
// I/O error and everything this workspace writes itself, come back untouched.
pub(crate) fn one_line(message: impl fmt::Display) -> String {
    let message = message.to_string();
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

// The URL leads, because it is what the reader wants from this line: the brief
// is filed, and the road from here is the project rather than a second push.
fn already_filed_message(f: &mut fmt::Formatter<'_>, name: &str, url: &str) -> fmt::Result {
    write!(
        f,
        "a project named `{name}` is already filed at {url}, so nothing was sent: a brief that \
         changed after it was filed is edited on the project"
    )
}

// Names the file the key is written in rather than only the key: a `team` in a
// `[[scope]]` record is a Linear team key — `WAR` — and the usual cause of this
// is a record carrying a team's *name*.
fn unknown_team_message(f: &mut fmt::Formatter<'_>, team: &str, path: &Path) -> fmt::Result {
    write!(
        f,
        "Linear knows no team with the key `{team}`, so nothing was filed: the `[[scope]]` \
         record in `{}` is where that key is written",
        path.display()
    )
}

// The command that lists the slugs is the whole of the fix, so it is spelled out
// with the scope already in it.
fn unknown_project_message(f: &mut fmt::Formatter<'_>, slug: &str, scope: &str) -> fmt::Result {
    write!(
        f,
        "Linear knows no project with the slug `{slug}`, so nothing was read: `warlock draft \
         {scope}` lists the planned ones"
    )
}

fn not_planned_message(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    status: Option<&str>,
) -> fmt::Result {
    let found = match status {
        Some(status) => format!("is in `{status}`"),
        None => "has no status".to_owned(),
    };
    write!(
        f,
        "the project `{name}` {found} rather than `Planned`, so nothing was read: \
         warlock reads a project back once it is planned"
    )
}

// The fact, then where it is written, then the rule underneath: a slice is cut
// once, so a project every note already covers has nowhere left to go here and
// the issues it made are where the work goes on.
fn all_cut_message(f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
    write!(
        f,
        "every slice of `{name}` is already cut or skipped, so there is nothing to draft: the \
         project's comments hold a note for each of them, and warlock offers a slice once — \
         retitle a skipped slice in the brief to have it offered again"
    )
}

// Named against the team and against Linear's own settings, like the unknown
// team above: the state is the team's, so a workspace that files its issues
// into a column spelled something else is a workflow to edit rather than a
// warlock to configure.
fn no_backlog_message(f: &mut fmt::Formatter<'_>, team: &str) -> fmt::Result {
    write!(
        f,
        "the team `{team}` has no workflow state called `Backlog`, so no issue was created: a cut \
         slice is filed into that state, and the team's workflow in Linear is where it is named"
    )
}

// The names lead, because they are the answer: the scope that was typed is not one
// of them, and one of them is almost certainly what was meant. A repository that
// records none is told that in words rather than handed an empty list.
fn unrecorded_scope_message(
    f: &mut fmt::Formatter<'_>,
    scope: &str,
    recorded: &[String],
    cost: &str,
) -> fmt::Result {
    let named = if recorded.is_empty() {
        "no `[[scope]]` record at all".to_owned()
    } else {
        format!("records {}", listed(recorded))
    };
    write!(
        f,
        "nothing in `.warlock/scopes.toml` records the scope `{scope}`, so {cost}: this \
         repository {named}"
    )
}

// Names the scope wanted and the open scopes, and ends where
// `closed_scope_message` ends — at `warlock open` — because the fix is the same
// one: opening the scope is what opens the work.
fn unheld_scope_message(f: &mut fmt::Formatter<'_>, scope: &str, held: &[String]) -> fmt::Result {
    let holding = if held.is_empty() {
        "AI can work in no scope on this machine".to_owned()
    } else {
        format!("AI can work in {} on this machine", listed(held))
    };
    write!(
        f,
        "the scope `{scope}` is closed on this machine, so nothing was pulled: {holding} — \
         `warlock open {scope}` lets AI work there"
    )
}

// Every entry, on one line, in `git status`'s own two-letter codes: the reader is
// about to run `git status` themselves, and a refusal that renamed the codes would
// make them translate back before they could look.
fn dirty_tree_message(f: &mut fmt::Formatter<'_>, dirty: &[Dirty]) -> fmt::Result {
    write!(
        f,
        "the working tree is not clean, so no ticket was pulled: {} — warlock commits what a \
         session writes, and what is already there is yours",
        listed(dirty)
    )
}

// Names the branch nothing, because the ticket's comment names it: this line says
// what happened and which of the two commands comes first, and the comment is the
// account of the whole run.
fn halted_message(f: &mut fmt::Formatter<'_>, ticket: &str) -> fmt::Result {
    write!(
        f,
        "the run for `{ticket}` halted, so the ticket stays in progress and was not moved to \
         review: its comment lists what finished and what did not, and `warlock resume {ticket}` \
         releases it"
    )
}

// The tree is the half a reader needs first: it still holds the work, warlock did
// not commit it and will not, and what happens to it is theirs to decide.
fn crossed_message(f: &mut fmt::Formatter<'_>, ticket: &str, subtask: &str) -> fmt::Result {
    write!(
        f,
        "`{subtask}` wrote outside the scopes open on this machine, so the run for `{ticket}` \
         stopped with nothing committed: the working tree is exactly as that session left it, and \
         the ticket's comment names the paths"
    )
}

// Names the directory rather than the `state.json` inside it: the whole run lives
// there — the record, the manifest and a brief per sub-task — and a reader told
// there is no run wants to know where warlock went looking for one.
fn no_run_message(f: &mut fmt::Formatter<'_>, ticket: &str, directory: &Path) -> fmt::Result {
    write!(
        f,
        "this machine holds no run for `{ticket}`, so there is nothing to resume: warlock looked \
         in `{}`, and a record is written there by the `warlock pull` that starts the ticket",
        directory.display()
    )
}

// The status leads, because it is the answer: a run in `in_review` has finished
// and one in `pulled` has not stopped. Which mode was asked is on the end for the
// variant's reason — the flag a reader typed is half of why the run had nothing to
// put back.
fn nothing_to_resume_message(
    f: &mut fmt::Formatter<'_>,
    ticket: &str,
    status: RunStatus,
    failed_only: bool,
) -> fmt::Result {
    let releases = if failed_only {
        "`--failed-only` puts a `failed` sub-task back and leaves a `blocked` or `crossed` one as \
         it is"
    } else {
        "a resume puts the `failed`, `blocked` and `crossed` sub-tasks back"
    };
    write!(
        f,
        "the run for `{ticket}` is `{status}` and has no sub-task to put back, so nothing was \
         written: {releases}"
    )
}

// The identifiers lead: the issues exist, no note names them, and this line is
// the last place they are named. The next draft offers the slice again, so the
// reader has to know to skip it there or to delete these.
fn uncut_message(
    f: &mut fmt::Formatter<'_>,
    issues: &[String],
    source: &LinearError,
) -> fmt::Result {
    write!(
        f,
        "the issues {} were created, and warlock could not note them on the project, so the next \
         draft offers this slice again: {}",
        listed(issues),
        one_line(source)
    )
}

impl fmt::Display for Error {
    // Over the pedantic line count because it is one arm per variant and the
    // enum is the whole vocabulary: the split the lint asks for would put half
    // the sentences behind a name, and a reader asking what warlock says about
    // one failure would have two places to look. The longest wordings are free
    // functions above.
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
            Self::Load { source } => write!(f, "{}", one_line(source)),
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
                write!(f, "{}", one_line(source))
            }
            // What was stopped and what survives it, in that order, because the
            // second half is the thing a reader wonders about a run they killed
            // half way through.
            Self::Cancelled => write!(
                f,
                "the run was cancelled; what it finished first is recorded"
            ),
            // The crate's sentence about the clipboard, with what it cost on
            // the end of it: "the native clipboard is not
            // accessible due to being held by another party" does not say by
            // itself that the text a reader asked for is not on the clipboard,
            // and that is the half they need. Flattened, because an unknown
            // failure carries whatever some other program printed.
            Self::Clipboard { source } => {
                write!(f, "nothing was copied: {}", one_line(source))
            }
            // The footer's own sentence, to the letter: the same fact refused
            // at a keystroke and at a shell prompt says the same thing, names
            // the same scope and points at the same `warlock open`.
            Self::ClosedScope { path, scope } => {
                write!(f, "{}", closed_scope_message(path, scope))
            }
            Self::Unscoped { path } => write!(f, "{}", unscoped_message(path)),
            // The shell's road out of a record it will not rewrite, which the
            // panel has no flags to take.
            Self::Scope {
                refusal: refusal @ ScopeRefusal::Recorded { .. },
            } => write!(
                f,
                "{refusal}: run with the path and the scope alone — no tracker, no `--team-key`, \
                 `--review-state` or `--label` — to write the scope, or edit the file to change \
                 the record"
            ),
            Self::Scope { refusal } => write!(f, "{refusal}"),
            // The engine's `.git` wording, with what it cost the caller on the
            // end: this is a refusal to do the thing that was typed rather than
            // a refusal to draw a tree, and the reader asked for that thing.
            Self::NoRepository { start, wanted } => write!(
                f,
                "no `.git` directory in `{}` or any of its parents, so there is no \
                 repository root to {wanted}",
                start.display()
            ),
            // Says which variables were looked at and what to do about it: a
            // reader whose `HOME` is unset is in an unusual shell and needs the
            // name of the thing to set rather than a fact about warlock.
            Self::NoHome => write!(
                f,
                "neither `HOME` nor `USERPROFILE` is set, so there is no home \
                 directory to keep the settings for this repository under: set \
                 `HOME` and run the command again"
            ),
            Self::Prompt { source } => {
                write!(f, "could not read the line that was typed: {source}")
            }
            // The rule is a sentence of its own, so this says what it is about
            // and, because the reader has just typed a whole line, that the rest
            // of that line has not been written either.
            Self::ScopeName { entered, rule } => write!(
                f,
                "`{entered}` is not a scope name, so nothing was written: {rule}"
            ),
            // Flattened like the manifest's, and for the same reason: a config
            // that will not parse carries the TOML parser's diagnostic.
            Self::Config { source } => write!(f, "{}", one_line(source)),
            Self::NotOpen { scope } => write!(
                f,
                "`{scope}` is not open on this machine, so nothing was written"
            ),
            // `ScopeName`'s shape, with the other half of what a reader needs on the
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
            Self::NoBinding { scope, key: None } => write!(
                f,
                "no key is bound to the scope `{scope}` in this checkout: `warlock key use \
                 {scope} <name>` binds one of the names `warlock key list` prints"
            ),
            Self::NoBinding {
                scope,
                key: Some(key),
            } => write!(
                f,
                "the scope `{scope}` is bound to the key `{key}`, which this machine does not \
                 store: `warlock key add {key}` stores it"
            ),
            // Flattened like the config's: a store that will not parse
            // carries a position rather than the parser's own diagnostic, and
            // the rest is the filesystem's, which can still run to two lines.
            Self::Keys { source } => write!(f, "{}", one_line(source)),
            // Flattened for the same reason again: what reaches here wraps a
            // manifest or key-store error whose text can run to two lines.
            Self::Route { source } => write!(f, "{}", one_line(source)),
            // The engine's sentence and nothing around it, for `Scope`'s
            // reason: each of these already says what is missing and what
            // writes it, and two of them are `route.rs`'s own words about a
            // key, which a person has already met from `warlock check`.
            // Flattened because the key store and the manifest underneath can
            // carry a parser's diagnostic.
            Self::Filing { source } => write!(f, "{}", one_line(source)),
            // The engine's sentence to the letter, for `Filing`'s reason: it
            // names the scope and what it cost, and a second wording here would
            // be a second answer to one question.
            Self::NotLinear { source } => write!(f, "{source}"),
            // The reader's own sentence about the document, which already ends
            // in what it cost — nothing was pushed.
            Self::Brief { source } => write!(f, "{source}"),
            // Both flattened: what the filesystem says about a file it would not
            // read can run to more than one line, and a `briefs.toml` that will
            // not parse carries the TOML parser's own multi-line diagnostic.
            Self::Template { source } => write!(f, "{}", one_line(source)),
            Self::Briefs { source } => write!(f, "{}", one_line(source)),
            Self::AlreadyFiled { name, url } => already_filed_message(f, name, url),
            Self::UnknownTeam { team, path } => unknown_team_message(f, team, path),
            // Flattened for the transport's sake: Linear's own refusals are one
            // line, and what a socket failure carries is whatever the network
            // stack said.
            Self::Linear { source } => write!(f, "{}", one_line(source)),
            Self::UnknownProject { slug, scope } => unknown_project_message(f, slug, scope),
            Self::NotPlanned { name, status } => not_planned_message(f, name, status.as_deref()),
            // The parser's own sentence and nothing around it, for `Brief`'s
            // reason: it names the heading the project is missing, or the
            // slices that wait on each other, and ends in there being nothing
            // to cut. Unflattened because it can only be one line — this is the
            // one wrapped error here that wraps nothing itself.
            Self::ScopeBlock { source } => write!(f, "{source}"),
            Self::AllCut { name } => all_cut_message(f, name),
            Self::NoBacklog { team } => no_backlog_message(f, team),
            Self::Uncut { issues, source } => uncut_message(f, issues, source),
            Self::Unskipped { title, source } => write!(
                f,
                "`{title}` was skipped, and warlock could not note the skip on the project, so \
                 the next draft offers it again: {}",
                one_line(source)
            ),
            Self::UnrecordedScope {
                scope,
                recorded,
                cost,
            } => unrecorded_scope_message(f, scope, recorded, cost),
            Self::UnheldScope { scope, held } => unheld_scope_message(f, scope, held),
            Self::DirtyTree { dirty } => dirty_tree_message(f, dirty),
            Self::Halted { ticket } => halted_message(f, ticket),
            Self::Crossed { ticket, subtask } => crossed_message(f, ticket, subtask),
            // The queue's own sentence about the ticket, which already says what is
            // in the way and names the command that frees a halted run.
            Self::NotPulled { ticket, refusal } => {
                write!(f, "`{ticket}` was not pulled: {refusal}")
            }
            // Flattened like the rest of the carried failures: the loop's `Dirty`
            // lists a tree an entry to a line, and `git`, Linear and the run record
            // each carry their own multi-line news.
            Self::Pull { source } => write!(f, "{}", one_line(source)),
            Self::Runs { source } => write!(f, "{}", one_line(source)),
            Self::NoRun { ticket, directory } => no_run_message(f, ticket, directory),
            Self::NothingToResume {
                ticket,
                status,
                failed_only,
            } => nothing_to_resume_message(f, ticket, *status, *failed_only),
            Self::Git { source } => write!(f, "{}", one_line(source)),
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
            Self::ScopeName { rule, .. } | Self::KeyName { rule, .. } => Some(rule),
            // The refusal's own cause rather than the refusal, which is the
            // rule for a scope the engine refuses and nothing for the others:
            // the refusal's sentence is this error's sentence, so naming it as
            // the cause would print it twice.
            Self::Scope { refusal } => std::error::Error::source(refusal),
            Self::Config { source } => Some(source),
            Self::Keys { source } => Some(source),
            Self::Route { source } => Some(source),
            Self::Filing { source } => Some(source),
            Self::NotLinear { source } => Some(source),
            Self::Brief { source } => Some(source),
            Self::Template { source } => Some(source),
            Self::Briefs { source } => Some(source),
            Self::ScopeBlock { source } => Some(source),
            Self::Uncut { source, .. } | Self::Unskipped { source, .. } => Some(source.as_ref()),
            Self::Linear { source } => Some(source),
            Self::Runs { source } => Some(source),
            Self::Git { source } => Some(source),
            Self::Pull { source } => Some(source.as_ref()),
            Self::Clipboard { source } => Some(source),
            // No source, and there is none to have: a boundary closed on this
            // machine is a fact about two files agreeing rather than a
            // failure anything underneath reported.
            Self::Problems { .. }
            | Self::NoRepository { .. }
            | Self::NoHome
            | Self::ClosedScope { .. }
            | Self::Unscoped { .. }
            // Nor here: a prompt answered with a blank line, and a name nobody
            // stored a key under, are a person and not a failure underneath.
            | Self::NoKey { .. }
            | Self::UnknownKey { .. }
            | Self::NoBinding { .. }
            | Self::NotOpen { .. }
            // Nor here: a brief the board already holds, and a team key Linear
            // does not know, are a disagreement rather than a failure
            // underneath. The Linear calls that answered both worked.
            | Self::AlreadyFiled { .. }
            | Self::UnknownTeam { .. }
            // Nor here, for that reason again: a slug the workspace does not
            // have and a project that is not planned are a command and the
            // board disagreeing. The Linear call that answered both worked.
            | Self::UnknownProject { .. }
            | Self::NotPlanned { .. }
            // Nor here: a team whose workflow has no `Backlog` is that same
            // disagreement, and a project every note already covers is the
            // board answering completely.
            | Self::NoBacklog { .. }
            | Self::AllCut { .. }
            // Nor here, and for that reason once more: a scope no record holds, a
            // scope closed on this machine and a ticket the queue's rules turn down are
            // facts about files agreeing, and a dirty tree is a person's own work.
            // A halt and a crossing have no cause underneath either — the run did
            // what it could, and what stopped it is on the ticket.
            | Self::UnrecordedScope { .. }
            | Self::UnheldScope { .. }
            | Self::DirtyTree { .. }
            | Self::NotPulled { .. }
            | Self::Halted { .. }
            | Self::Crossed { .. }
            // Nor here: a ticket this checkout never pulled, and a run with
            // nothing left to put back, are this machine's own records answering
            // completely rather than anything underneath failing. A record that
            // would not read is `Runs` above, which does carry its cause.
            | Self::NoRun { .. }
            | Self::NothingToResume { .. }
            // Nor here: a cancel has no cause underneath it at all — somebody
            // pressed Ctrl-C.
            | Self::Cancelled => None,
        }
    }
}

impl From<LinearError> for Error {
    fn from(source: LinearError) -> Self {
        Self::Linear { source }
    }
}

// A `From` rather than a `map_err` at each site, because every caller that asks
// a record for its Linear view is one `?` away from this and the refusal has
// nothing to be told at any of them.
impl From<manifest::NotLinear> for Error {
    fn from(source: manifest::NotLinear) -> Self {
        Self::NotLinear { source }
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

/// The process's exit status.
///
/// Three non-zero registers of warlock's own, kept distinct so a script can
/// tell them apart without reading a word of the message, beside clap's **2**
/// for a command line it could not parse: **1** is warlock could not do it;
/// **3** is a boundary closed on this machine, refused with nothing
/// spent; and [`CANCELLED`] is a run somebody stopped.
///
/// **0** means the question was answered whatever the answer was — a closed
/// scope is `check`'s answer rather than a failure to reach one.
#[must_use]
pub const fn status_for(outcome: &Result<(), Error>) -> u8 {
    match outcome {
        Ok(()) => 0,
        // The boundary's refusals spend the same register and nothing else does:
        // a scope closed on this machine or a path no scope covers, refused
        // before a request with nothing spent, and a session that wrote outside
        // the open scopes, which is a run that stopped with nothing
        // committed and the tree left as it was. The numbers here and
        // `Pulled::status`'s are one decision, asserted against each other in
        // the tests.
        Err(
            Error::ClosedScope { .. }
            | Error::Unscoped { .. }
            | Error::UnheldScope { .. }
            | Error::Crossed { .. },
        ) => 3,
        // A run somebody stopped, and the one status here that is not warlock's
        // verdict on anything: it sits apart from the catch-all because it is
        // the number a shell already spells an interrupted process with.
        Err(Error::Cancelled) => CANCELLED,
        // Everything else, and that includes every refusal `warlock scope add`
        // has about a `[[scope]]` record — the tracker word among them,
        // deliberately, rather than for want of somewhere to put it. A **1**
        // and not clap's **2**: which tracker and which flags a run needs
        // depends on what the manifest already records, so the rule is
        // warlock's to word rather than a command line clap could have parsed,
        // and a scope name the engine refuses already spends this register. Not
        // a **3** either — that one is the boundary's alone, and a script
        // reading it as "open a scope" would be sent to `warlock open` over a
        // missing `--team-key`.
        Err(_) => 1,
    }
}

/// The status a shell already spells an interrupted process with, so warlock
/// does not invent a second one.
pub(crate) const CANCELLED: u8 = 130;

#[cfg(test)]
#[path = "tests/error.rs"]
mod tests;
