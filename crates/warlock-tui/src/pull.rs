//! `warlock pull <SCOPE>`: the scope's queue gives up a ticket, the loop in
//! [`mod@crate::pulling`] works it, and what comes back is a pull request, a halt
//! or a refusal.
//!
//! The split is [`mod@crate::push`]'s: everything that can refuse before a
//! request is asked by [`prepare`], which opens no socket and runs no `git`, so a
//! refusal costs nothing. [`pulled`] then reads the queue, and only a ticket that
//! came out of it reaches [`Pulling`].
//!
//! The order of the four refusals is the promise rather than an arrangement, and
//! the reason the record question and the open question are asked separately
//! here is the register they answer in. [`resolve_filing`] filters the recorded
//! scopes by the open ones before it looks for the name, so a scope this
//! repository records that is closed on this machine comes back from it as "no
//! such candidate" — an ordinary **1** — where the honest answer is the boundary's
//! **3**. Asked in this order the closed scope keeps its own status, and
//! `resolve_filing` is still what answers the key question, so an unbound
//! checkout is refused in the very words `push` and `draft` refuse it in.
//!
//! No `--json`, matching every other subcommand that spends something: what a
//! script reads afterwards is the run record under the home directory, which is a
//! file rather than a stream to be caught. No `--any` either, and there is no way
//! here to work somebody else's ticket — [`take_named`] checks the assignee by id
//! against the user the key belongs to, and nothing takes a flag past it.
//!
//! No key value is printed here and none can be: [`Prepared`] carries one as a
//! [`Key`], whose `Debug` redacts it, it is read on one line per door — the opener's, here and in
//! [`mod@crate::puller`] — and nothing that prints is given it.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use warlock_engine::{
    LinearScope, Manifest, PullRun, ScopeRecord, brief_path, held_runs, resolve_filing,
    scope_opens_to,
};

use crate::account::{THINKING, WRITING, tool_line};
use crate::board::{Board, Key, Opener as LinearOpener, Opens, QueuedTicket};
use crate::claude::{Activities, Activity};
use crate::error::{Error, NO_QUEUE, files_to_linear, one_line, say};
use crate::git::{Forge, Gh, Git, Repository};
use crate::puller::{Splitter, Stopping, Worker};
use crate::pulling::{Heading, PullEvent, Pulled, Pulling, Splits, Ticket, Works};
use crate::queue::{Chosen, Named, Refusal, Skipped, choose, take_named};
use crate::settings::Settings;
use crate::standing::{FOR_PULL, Standing};

/// The subcommand, and the only function here that reads the environment.
///
/// The two directories, the socket, the checkout, the forge and the two kinds of
/// session are resolved on these few lines and handed down as parameters, which
/// is what lets the whole command be driven against a temporary repository and a
/// temporary home with no board, no `git` and no `claude` on the machine.
///
/// [`prepare`] is called up here rather than inside [`pulled`] for one reason: the
/// open scopes it answers with are what the sub-task sessions' system prompt is
/// built from, and reading them a second time to build it could disagree with the
/// boundary this run was allowed through on.
pub fn pull(scope: &str, ticket: Option<&str>, dry_run: bool) -> Result<(), Error> {
    let standing = Standing::here(FOR_PULL)?;
    // The error rather than `check`'s `.ok()`, for [`mod@crate::push`]'s reason:
    // the config under the home says whether this scope is open on this machine
    // and the key store beside it is what reads the queue, so a machine with no
    // home has nothing to pull with rather than an answer of "no scope open".
    let home = Standing::home()?;
    let root = standing.repo_root().to_path_buf();

    let manifest = standing.manifest()?;
    let prepared = prepare(&manifest, &root, &home, scope)?;

    // Built before the seams, because the sessions report into it: the activity
    // port below is handed a clone, so a line a `claude` writes reaches the same
    // stdout — and the same execution log — as the headings around it.
    let progress = shared(Progress::new(io::stdout(), &home, &root));

    pulled(
        &manifest,
        &prepared,
        ticket,
        dry_run,
        &Ports {
            open: &LinearOpener,
            repo: &Git::at(&root),
            forge: &Gh::at(&root),
            // A shell has no stop button, so nothing ever reads the handles
            // these publish.
            split: &Splitter::new(watching(&progress), Stopping::default()),
            sessions: &Worker::new(
                scope,
                prepared.held(),
                watching(&progress),
                Stopping::default(),
            ),
        },
        &progress,
    )
}

/// Everything one pull reaches the world through, so no function below this line
/// resolves one for itself.
///
/// A struct rather than five parameters on [`pulled`], which already takes the
/// environment and the three words that were typed: the seams are one thing — the
/// outside — and a test hands in stand-ins that record what they were asked and
/// panic when they are reached on a road that promised not to.
pub(crate) struct Ports<'a, O: Opens, R: Repository, F: Forge, S: Splits, W: Works> {
    pub(crate) open: &'a O,
    pub(crate) repo: &'a R,
    pub(crate) forge: &'a F,
    pub(crate) split: &'a S,
    pub(crate) sessions: &'a W,
}

/// The whole subcommand, less the environment: the two directories arrive on the
/// [`Prepared`], and the seams and the writer are parameters rather than reads, as
/// [`pushed`](crate::push) has them.
///
/// The order is the load-bearing part, and it is the order the ticket words it
/// in. [`prepare`] has already refused without a request; the tree is read next,
/// so a dirty checkout is refused before the board is asked anything; the queue is
/// read after both, and only then is a session raised.
///
/// A dry run stops at the queue by design: it is the one road that asks the board
/// and runs no `git` at all, because what it answers is "which ticket would be
/// taken", and a `git status` on the way to that answer would be a command warlock
/// promised not to run.
pub(crate) fn pulled<O: Opens, R: Repository, F: Forge, S: Splits, W: Works, P: Write>(
    manifest: &Manifest,
    prepared: &Prepared,
    named: Option<&str>,
    dry_run: bool,
    ports: &Ports<'_, O, R, F, S, W>,
    progress: &Shared<P>,
) -> Result<(), Error> {
    let (root, home) = (prepared.root(), prepared.home());
    // Before the tree is read and before the board is opened: a scope warlock
    // cannot file to is refused with nothing run and no client built.
    let scope = LinearScope::of(prepared.record())?;

    // Before the board is opened and before the queue is read: what a dirty tree
    // costs is a run that would fold somebody's uncommitted edit into a sub-task's
    // commit, and the cheapest place to refuse it is here.
    if !dry_run {
        clean(ports.repo)?;
    }

    let board = ports.open.open(prepared.record(), prepared.value());
    let selected = selection(&board, scope, home, root, named, &mut |line| {
        tell(progress, &line);
    })?;

    if dry_run {
        for line in would(scope.name(), named, &selected) {
            tell(progress, &line);
        }
        return Ok(());
    }

    for skipped in selected.skipped() {
        tell(progress, &passed_over(skipped));
    }

    let Some(issue) = selected.issue(named)? else {
        tell(progress, &nothing_ready(scope.name()));
        return Ok(());
    };

    let ticket = Ticket {
        id: issue.id(),
        identifier: issue.identifier(),
        number: number_in(issue.identifier()),
        title: issue.title(),
        description: issue.description(),
    };
    held(progress).about(ticket.identifier);

    let pulled = {
        let mut sink = |event: PullEvent| held(progress).on(&event);
        Pulling {
            board: &board,
            repo: ports.repo,
            forge: ports.forge,
            split: ports.split,
            sessions: ports.sessions,
            scope,
            manifest,
            held: prepared.held(),
            base: prepared.base(),
            root,
            home,
            progress: &mut sink,
        }
        .pull(&ticket)
    }
    .map_err(|source| Error::Pull {
        source: Box::new(source),
    })?;

    match pulled {
        Pulled::Opened { ticket, url } => {
            tell(progress, &opened(&ticket, url.as_deref()));
            Ok(())
        }
        Pulled::Unchanged { ticket } => {
            tell(progress, &unchanged(&ticket));
            Ok(())
        }
        // Both carry what the ticket's own comment already says at length, because
        // this is the line a shell prints and the status a script reads. Neither is
        // the loop failing: the branch holds what the run did commit.
        Pulled::Halted { ticket } => Err(Error::Halted { ticket }),
        Pulled::Crossed { ticket, subtask } => Err(Error::Crossed { ticket, subtask }),
    }
}

/// A pull with every refusal that costs nothing already asked: the scope found in
/// a `[[scope]]` record, open on this machine, and a key bound to this checkout.
#[derive(Debug, Clone)]
pub(crate) struct Prepared {
    /// The repository the run commits in, and what the run record's directory is
    /// keyed by.
    root: PathBuf,
    /// Where that record goes, never under [`root`](Self::root).
    home: PathBuf,
    record: ScopeRecord,
    /// The flattened open scopes, which the crossing check after
    /// every session is judged against and the session's own system prompt names.
    held: Vec<String>,
    /// The branch this checkout set with `warlock branch use`, or `None` for the
    /// remote's default.
    base: Option<String>,
    value: Key,
}

impl Prepared {
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn home(&self) -> &Path {
        &self.home
    }

    pub(crate) const fn record(&self) -> &ScopeRecord {
        &self.record
    }

    pub(crate) fn held(&self) -> &[String] {
        &self.held
    }

    pub(crate) fn base(&self) -> Option<&str> {
        self.base.as_deref()
    }

    /// The key the board is opened with, read on exactly one line per door — the
    /// opener's. `pub(crate)` for the panel's sake: its run opens the board on a
    /// worker of its own, so the value has to cross out of here, and the type it
    /// sits on redacts it in `Debug`.
    pub(crate) fn value(&self) -> &str {
        self.value.as_str()
    }
}

/// The four questions a pull answers before it asks the world anything, in the
/// order the answers are worth having.
///
/// The record first, because a scope no `[[scope]]` record holds is a manifest
/// one line short and the fix is in this repository. Open second, because a
/// recorded scope closed on this machine is a boundary and its own status. The
/// tracker third, because a scope that files to Jira is a pull warlock can only
/// refuse, and refusing it before the key is read is what makes "nothing was
/// sent" true of it. The key last, through the very [`resolve_filing`] a push
/// and a draft go through, so an unbound checkout is told the same sentence by
/// all three verbs.
pub(crate) fn prepare(
    manifest: &Manifest,
    root: &Path,
    home: &Path,
    scope: &str,
) -> Result<Prepared, Error> {
    let Some(record) = manifest.scope_record(scope) else {
        return Err(Error::unrecorded(manifest, scope, NO_QUEUE));
    };

    let settings = Settings::at(Some(home.to_path_buf()), root);
    let held = settings.open_scopes()?;
    // `scope_opens_to` and nothing written here, so the wildcard is honoured by
    // the one function that knows about it: a comparison of this module's own
    // would be a second copy of the boundary rule, and it is the copy that would
    // forget `*`.
    if !scope_opens_to(Some(scope), &held) {
        return Err(Error::UnheldScope {
            scope: scope.to_owned(),
            held,
        });
    }

    // Above `resolve_filing`, which loads the key value: a scope warlock cannot
    // file to is refused with no key read, no `git` run and no client built.
    // `pulled` reads the same record through the same view a moment later, so
    // this is where that refusal is asked rather than a second rule.
    files_to_linear(manifest, scope)?;

    // Reached only with the record and open questions above already answered, so
    // the board refusals it can raise cannot fire: the name is a candidate by construction.
    // What is left of it is the key half, which is why it is asked at all.
    let target = resolve_filing(manifest, root, home, Some(scope))
        .map_err(|source| Error::Filing { source })?;
    let base = settings.base()?;

    Ok(Prepared {
        root: root.to_path_buf(),
        home: home.to_path_buf(),
        record: record.clone(),
        held,
        base,
        value: Key::new(target.value()),
    })
}

pub(crate) fn clean<R: Repository>(repo: &R) -> Result<(), Error> {
    let dirty = repo.dirty().map_err(|source| Error::Git { source })?;
    if dirty.is_empty() {
        Ok(())
    } else {
        Err(Error::DirtyTree { dirty })
    }
}

/// Which ticket the queue gives up, with the records this checkout holds for the
/// scope read first: they are what tells a ticket in `In Progress` here from one
/// in progress somewhere else.
///
/// An unreadable record is a line handed to `unreadable` and not a failure: the
/// scan names it, and the run it describes may be holding uncommitted work on a
/// branch. The line goes out before the queue is read, so a refusal from the
/// board does not swallow it.
pub(crate) fn selection(
    board: &impl Board,
    scope: LinearScope<'_>,
    home: &Path,
    root: &Path,
    named: Option<&str>,
    unreadable: &mut dyn FnMut(String),
) -> Result<Selected, Error> {
    let assignee = board.viewer()?;
    let runs = held_runs(home, root, scope.name()).map_err(|source| Error::Runs { source })?;
    for record in runs.unreadable() {
        unreadable(one_line(record));
    }
    select(board, scope, &assignee, named, runs.runs())
}

fn select(
    board: &impl Board,
    scope: LinearScope<'_>,
    assignee: &str,
    named: Option<&str>,
    runs: &[PullRun],
) -> Result<Selected, Error> {
    Ok(match named {
        Some(ticket) => Selected::Named(take_named(board, scope, assignee, ticket, runs)?),
        None => Selected::Chosen(choose(
            &board.scope_queue(assignee)?,
            scope.review_state(),
            runs,
        )),
    })
}

/// What selection came to, in the two shapes the two doors produce.
///
/// One type for both because everything after it — the dry run, the skip lines
/// and the ticket to work — reads the same three facts off either: what was
/// taken, what was passed over, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Selected {
    Chosen(Chosen),
    Named(Named),
}

/// The ticket selection gave up, or the reason there is none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Taken<'a> {
    Issue(&'a QueuedTicket),
    /// The queue answered and nothing in it was available, which is an answer and
    /// not a refusal.
    Nothing,
    /// A ticket somebody named that cannot be worked.
    Refused(&'a Refusal),
}

impl Selected {
    /// The ticket to work, `None` for a queue with nothing available, and a
    /// refusal for a ticket somebody named that cannot be worked.
    pub(crate) fn issue(&self, named: Option<&str>) -> Result<Option<&QueuedTicket>, Error> {
        match self.taken() {
            Taken::Issue(issue) => Ok(Some(issue)),
            Taken::Nothing => Ok(None),
            Taken::Refused(refusal) => Err(Error::NotPulled {
                ticket: named.unwrap_or_default().to_owned(),
                refusal: refusal.clone(),
            }),
        }
    }

    pub(crate) fn taken(&self) -> Taken<'_> {
        match self {
            Self::Chosen(chosen) => chosen.taken().map_or(Taken::Nothing, Taken::Issue),
            Self::Named(Named::Taken(issue)) => Taken::Issue(issue),
            Self::Named(Named::Refused(refusal)) => Taken::Refused(refusal),
        }
    }

    /// Every ticket the pass walked past with the reason it did, in the queue's
    /// order. A named ticket has walked past nothing: it was the only one asked
    /// about, and its own refusal is on [`Taken::Refused`].
    pub(crate) fn skipped(&self) -> &[Skipped] {
        match self {
            Self::Chosen(chosen) => chosen.skipped(),
            Self::Named(_) => &[],
        }
    }
}

/// What a dry run prints: the ticket that would be taken, and every ticket the
/// pass walked past with the reason.
///
/// A list rather than one sentence, for the reason a cut's dry run is a list: what
/// a pull is about to do is a choice among tickets, and the tickets it did not
/// choose are the half a person reads it for.
fn would(scope: &str, named: Option<&str>, selected: &Selected) -> Vec<String> {
    let mut lines = match (selected.taken(), named) {
        (Taken::Issue(issue), _) => vec![format!(
            "would pull `{}` — {}, and nothing was written, no `git` ran and no session was raised",
            issue.identifier(),
            issue.title()
        )],
        // The two "nothing" endings word differently because the question was
        // different: a queue pass says the queue is empty of work, and a named
        // ticket says that ticket would not be taken and why.
        (Taken::Nothing, _) => vec![format!("would pull nothing: {}", nothing_ready(scope))],
        (Taken::Refused(refusal), ticket) => vec![format!(
            "would not pull `{}`: {refusal}",
            ticket.unwrap_or_default()
        )],
    };
    lines.extend(selected.skipped().iter().map(passed_over));
    lines
}

pub(crate) fn passed_over(skipped: &Skipped) -> String {
    format!(
        "passed over `{}` — {}",
        skipped.issue().identifier(),
        skipped.reason()
    )
}

pub(crate) fn nothing_ready(scope: &str) -> String {
    format!("nothing in the queue for `{scope}` is ready to work")
}

/// The one line a finished pull prints, and the URL leads it for the reason a
/// push's does: the pull request is what the run was for, and its address is the
/// thing that must not be lost.
///
/// No `gh` on the machine is still a finish, said as what happened: the account of
/// the run is on the ticket instead, which is where somebody opening the request
/// by hand will look.
pub(crate) fn opened(ticket: &str, url: Option<&str>) -> String {
    match url {
        Some(url) => format!("`{ticket}` is in review: {url}"),
        None => format!(
            "`{ticket}` is in review, and there is no `gh` on this machine — the branch is pushed \
             and the pull request's body is a comment on the ticket"
        ),
    }
}

pub(crate) fn unchanged(ticket: &str) -> String {
    format!(
        "`{ticket}` is in review with nothing to merge: the work was already there, so nothing \
         was pushed and what each sub-task found is a comment on the ticket"
    )
}

/// `WAR-140` is 140, which is the middle of the branch name.
///
/// Zero for an identifier not built that way, rather than a refusal: the
/// identifier came off the board, the branch name also carries the folded title,
/// and refusing to work a ticket over how its team spells identifiers would be
/// warlock deciding what Linear may call an issue.
pub(crate) fn number_in(identifier: &str) -> u32 {
    identifier
        .trim()
        .rsplit_once('-')
        .and_then(|(_, number)| number.trim().parse().ok())
        .unwrap_or(0)
}

/// The progress of one pull, shared: the door's own lines, the loop's headings,
/// and the activities of whatever session is running.
///
/// Shared because the last of the three arrives from inside a session.
/// [`Activities`] takes a `Fn(Activity) + Send + Sync + 'static`, so the port a
/// `claude` reports into cannot borrow anything the run owns — it holds a clone of
/// this, and what it writes lands between the headings it happened under rather
/// than in a burst after the session ends.
pub(crate) type Shared<W> = Arc<Mutex<Progress<W>>>;

pub(crate) fn shared<W: Write>(progress: Progress<W>) -> Shared<W> {
    Arc::new(Mutex::new(progress))
}

/// The lock, taken for the length of one line.
///
/// A poisoned lock is a panic in something that was writing a progress line, and
/// there is nothing sensible to do about one from here, so this is the one place
/// that says so.
pub(crate) fn held<W: Write>(progress: &Shared<W>) -> MutexGuard<'_, Progress<W>> {
    progress
        .lock()
        .expect("nothing panics holding the progress")
}

fn tell<W: Write>(progress: &Shared<W>, fact: &str) {
    say(&mut held(progress).out, fact);
}

/// What a pull is seen doing, on stdout in `running.rs`'s shape: `warlock: ` a
/// line, a header per section, and the session's own lines under the header they
/// belong to.
///
/// A failed write is ignored, exactly as `running.rs`'s `Progress` ignores one and
/// for its reason: `warlock pull warlock-team | head -1` is a closed stdout, and
/// losing a run of sessions over the state of a pipe would spend somebody's tokens
/// and then throw away what they bought. The append below is ignored for the same
/// reason on a stronger case — the run record's brief is a courtesy copy of what
/// is already on stdout and in `state.json`.
///
/// No `Debug`, for the reason [`Pulling`] has none: a writer is not a value to
/// print.
pub(crate) struct Progress<W: Write> {
    out: W,
    home: PathBuf,
    root: PathBuf,
    /// The ticket being worked, which the door says once the queue has answered:
    /// a resumed run holding its split never reaches the splitting heading, so the
    /// events alone cannot be relied on to name it.
    ticket: Option<String>,
    /// The sub-task whose section is open, and the whole of what says where an
    /// activity is appended. `None` outside a sub-task — the split and the pull
    /// request have no brief of their own to write into.
    subtask: Option<String>,
    watched: Watched,
}

impl<W: Write> Progress<W> {
    pub(crate) fn new(out: W, home: &Path, root: &Path) -> Self {
        Self {
            out,
            home: home.to_path_buf(),
            root: root.to_path_buf(),
            ticket: None,
            subtask: None,
            watched: Watched::default(),
        }
    }

    /// What a test reads the run's output off. Nothing on the real road reads it
    /// again — the writer there is stdout, and the lines have already gone.
    #[cfg(test)]
    pub(crate) const fn written(&self) -> &W {
        &self.out
    }

    pub(crate) fn about(&mut self, ticket: &str) {
        self.ticket = Some(ticket.to_owned());
    }

    pub(crate) fn on(&mut self, event: &PullEvent) {
        if let PullEvent::Activity(activity) = event {
            if let Some(line) = self.watched.line(activity) {
                say(&mut self.out, &line);
                self.append(&line);
            }
            return;
        }
        if let PullEvent::Heading(heading) = event {
            self.subtask = match heading {
                Heading::Subtask { id, .. } => Some(id.clone()),
                Heading::Split { .. } | Heading::PullRequest { .. } => None,
            };
            self.watched = Watched::default();
        }
        if let Some(line) = event.line() {
            say(&mut self.out, &line);
        }
    }

    /// One activity line under the execution log heading of the sub-task's own
    /// brief in the run record.
    ///
    /// Appended rather than written, and that is what makes it safe:
    /// [`PullSubtask::to_brief_string`](warlock_engine::PullSubtask) re-renders
    /// everything above the log line on every save and carries whatever is below
    /// it across untouched, so the two writers never race over the same bytes.
    fn append(&mut self, line: &str) {
        let (Some(ticket), Some(subtask)) = (&self.ticket, &self.subtask) else {
            return;
        };
        let path = brief_path(&self.home, &self.root, ticket, subtask);
        // Ignored on purpose, and never created: a brief that is not there is a
        // sub-task whose record has not been saved yet, which is not a thing to
        // put a file back for.
        if let Ok(mut file) = OpenOptions::new().append(true).open(path) {
            drop(writeln!(file, "- {line}"));
        }
    }
}

/// One activity as a line, or nothing for the one that is not something a session
/// did.
///
/// A cost is no line at all: it is a fact about the pass rather than an action,
/// and a run that printed one per report would be a column of money in the
/// middle of the work.
fn activity_line(activity: &Activity) -> Option<String> {
    match activity {
        Activity::Tool { name, detail } => Some(tool_line(name, detail.as_ref())),
        Activity::Thinking => Some(THINKING.to_owned()),
        // No byte count: the count is only worth reading where it can tick up in
        // place, and here it would make every report a different line.
        Activity::Writing { .. } => Some(WRITING.to_owned()),
        Activity::Cost { .. } => None,
    }
}

/// The activity lines of one session at a shell door, with a stretch of
/// thinking or writing said once: the panel redraws its activity line in place,
/// but a session reports `Writing` again with every delta, and printed on a
/// screen that only scrolls that is a column of identical lines. A tool line
/// repeats only when the session did the same thing twice, so it is never
/// collapsed.
#[derive(Debug, Default)]
pub(crate) struct Watched {
    said: Option<String>,
}

impl Watched {
    pub(crate) fn line(&mut self, activity: &Activity) -> Option<String> {
        let line = activity_line(activity)?;
        let running = matches!(activity, Activity::Thinking | Activity::Writing { .. });
        if running && self.said.as_deref() == Some(line.as_str()) {
            return None;
        }
        self.said = Some(line.clone());
        Some(line)
    }
}

/// The activity port of a session: a clone of the progress, written into from
/// wherever the session reports.
fn watching<W: Write + Send + 'static>(progress: &Shared<W>) -> Activities {
    let progress = Arc::clone(progress);
    Activities::new(move |activity| held(&progress).on(&PullEvent::Activity(activity)))
}

// Every refusal above is asked over values against a temporary repository and a
// temporary home, with stand-ins for the board, the checkout, the forge and both
// kinds of session, so no test of this module opens a socket, runs a `git`, raises
// a `claude` or reads a key store that is not its own.
#[cfg(test)]
#[path = "tests/pull.rs"]
mod tests;
