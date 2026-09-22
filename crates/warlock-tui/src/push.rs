//! `warlock push <PATH>`: the brief at that path becomes a project on the board
//! this machine's sigil names, and the address of that project is written into
//! `.warlock/filed.toml`.
//!
//! The order in [`push`] is the promise rather than an arrangement: everything
//! that can refuse — no board, several boards, an unbound checkout, a brief
//! already filed, a document that is not a brief — is asked before the client
//! is built, so a refusal reaches nobody's workspace and costs nothing. The
//! socket is opened on the line that constructs the client and on no earlier
//! one.
//!
//! No `--json`, matching the other writing subcommands: the answer worth
//! parsing is the record, which is a file rather than a stream to be caught.
//!
//! No key value is printed here and none can be. [`resolve_filing`] hands back
//! a target carrying one, it is read on exactly one line — the constructor —
//! and [`sent`], which is the half that prints and the half a test drives,
//! takes the board as three strings and never that target.

use std::io::{self, Write};
use std::path::Path;

use warlock_engine::{Filed, FiledRecord, filed, manifest_path, now_rfc3339, resolve_filing};
use warlock_tui::{
    Brief, LinearClient, NewProject, Posts, backlog_status, brief_at, create_project, size, team_id,
};

use crate::error::Error;
use crate::standing::{FOR_PUSH, Standing};

pub(crate) fn push(path: &Path, scope: Option<&str>, dry_run: bool) -> Result<(), Error> {
    let standing = Standing::here(FOR_PUSH)?;
    // The error rather than `check`'s `.ok()`: the sigils under the home pick
    // the board and the key store beside them is what files to it, so a machine
    // with no home has nothing to push with rather than an answer of "nothing
    // held".
    let home = Standing::home()?;

    pushed(
        &standing,
        &home,
        path,
        scope,
        dry_run,
        client,
        &mut io::stdout(),
    )
}

// Named rather than passed as `LinearClient::new`, which is generic over what it
// is handed and so is only ever a function of one lifetime: the seam below has
// to take a key borrowed for however long the target lives, and this is the
// one-line function that says so.
//
// `pub(crate)` for [`mod@crate::pull`], which reads a project back off the same
// board with the same key: one spelling of "the socket opens here", so the one
// line in warlock that turns a stored key into a client cannot become two.
pub(crate) fn client(key: &str) -> LinearClient {
    LinearClient::new(key)
}

// Split from `push` so that the environment — the working directory, the
// repository root, the home the sigils and the key store sit under — is three
// parameters rather than three reads, the way `check` splits `checked_onto`:
// every refusal below can then be run against a temporary repository and a
// temporary home, and no test in this crate can reach the developer's real key
// store by standing in the wrong directory.
//
// `open` is the socket, as a parameter. It is called on exactly one line — the
// one marked below — and a test hands in a stand-in that panics when it is
// called, which is what makes "this refusal opens no socket" an assertion about
// the order here rather than a reading of it.
fn pushed<P: Posts, O: FnOnce(&str) -> P, W: Write>(
    standing: &Standing,
    home: &Path,
    path: &Path,
    scope: Option<&str>,
    dry_run: bool,
    open: O,
    out: &mut W,
) -> Result<(), Error> {
    let manifest = standing.manifest()?;

    let target = resolve_filing(&manifest, standing.repo_root(), home, scope)
        .map_err(|source| Error::Filing { source })?;
    let record = target.record();

    let brief_path = standing.target(path);
    let spelled = standing.spelled(&brief_path)?;
    let filed = records(standing.repo_root())?;
    if let Some(already) = filed.record(&spelled) {
        return Err(Error::AlreadyFiled {
            path: spelled,
            url: already.url().to_owned(),
        });
    }

    let brief =
        brief_at(standing.repo_root(), &brief_path).map_err(|source| Error::Brief { source })?;

    let board = Board {
        scope: target.scope(),
        team: record.team(),
        label: record.label(),
    };
    if dry_run {
        drop(writeln!(out, "{}", would(board, target.key(), &brief)));
        return Ok(());
    }

    // The key is read here and nowhere else in this module, on the last line
    // before anything leaves the machine.
    let linear = open(target.value());
    // The address is dropped here and nowhere else: the line above has already
    // carried it to whoever ran the command, and it comes back for the panel's
    // sake, which has no `out` to read afterwards. See [`mod@crate::pushing`].
    sent(
        &linear,
        standing.repo_root(),
        board,
        &brief,
        &brief_path,
        filed,
        out,
    )
    .map(drop)
}

// The three strings a push sends, split out of the `Target` they were read from
// so the sending half below cannot reach the key value that target carries.
//
// `pub(crate)` for the panel's `/push`, which resolves the same three off the
// same target and hands them to the same [`sent`]: see [`mod@crate::pushing`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct Board<'a> {
    pub(crate) scope: &'a str,
    pub(crate) team: &'a str,
    pub(crate) label: &'a str,
}

// Split from `push` over the `Posts` seam, the way `check` splits its I/O-free
// core: everything below can be driven by an in-memory stand-in, so the order —
// team, status, project, record — is assertable without a socket, a key or a
// board.
//
// It is also the whole of what the panel's `/push` sends, called on that push's
// worker thread with a sink for `out`: the four requests, their order, their
// wording and the record are one sequence with one caller's worth of reasons
// behind them, and a second composition of them for the panel would be a second
// place for the order to drift. The address comes back for that caller — the
// panel has no `out` to read afterwards, and the URL is the one thing a push
// must not lose.
//
// The label is `create_project`'s own first request and is deliberately not
// resolved here as well. That ordering is the client's decision (`linear.rs`):
// the label is the only mark saying warlock filed a project, nothing here can
// take a project back, so the label exists before the project does. A
// `label_id` call in this function would resolve the same name a second time,
// one extra request per push, and would not reach the case the ticket describes
// — a label that fails after a create that landed — because with the label
// first there is no such create. So a label that will not resolve is a refusal
// with nothing sent, nothing recorded and no URL, and that is the honest report
// of what happened.
pub(crate) fn sent<W: Write>(
    linear: &impl Posts,
    root: &Path,
    board: Board<'_>,
    brief: &Brief,
    path: &Path,
    mut filed: Filed,
    out: &mut W,
) -> Result<String, Error> {
    let team = team_id(linear, board.team)
        .map_err(|source| Error::Linear { source })?
        .ok_or_else(|| Error::UnknownTeam {
            team: board.team.to_owned(),
            path: manifest_path(root),
        })?;
    // `None` is a workspace with no status by that name, which is a project
    // filed with no status at all rather than a failure.
    let status = backlog_status(linear).map_err(|source| Error::Linear { source })?;

    let project = create_project(
        linear,
        &NewProject::new(brief.name(), brief.content(), &team, board.label)
            .with_status(status.as_deref()),
    )
    .map_err(|source| Error::Linear { source })?;

    // Printed before the record is saved, not after: the project exists from
    // here on and its address is the one thing that must not be lost, so it
    // goes out whatever the save does next.
    drop(writeln!(
        out,
        "warlock: filed `{}` to `{}`, labelled `{}`: {}",
        brief.name(),
        board.team,
        board.label,
        project.url()
    ));

    filed.push(
        FiledRecord::new(
            root,
            path,
            project.id(),
            project.url(),
            board.scope,
            board.team,
            now_rfc3339(),
        )
        .map_err(|source| Error::Unfiled {
            url: project.url().to_owned(),
            source: Box::new(source),
        })?,
    );
    filed.save(root).map_err(|source| Error::Unfiled {
        url: project.url().to_owned(),
        source: Box::new(source),
    })?;

    // The address, for a caller with no `out` to read it off. Last, so that a
    // push which hands one back is one whose record is on disk — and so that the
    // two failures either side of the create keep carrying it themselves.
    Ok(project.url().to_owned())
}

// Shared with the panel's `/push`, which loads the same file on its worker
// thread and appends through the same [`Filed`]: see [`mod@crate::pushing`].
//
// A missing file is an empty one, the reading `Standing::manifest` takes: a
// repository that has never filed anything records no filing, and that is an
// answer rather than the absence of one. A file that exists and will not read
// stays a failure — pushing over records warlock could not read is how a brief
// that already has a project gets a second one.
pub(crate) fn records(root: &Path) -> Result<Filed, Error> {
    match Filed::load(root) {
        Err(filed::Error::NotFound { .. }) => Ok(Filed::new()),
        other => other.map_err(|source| Error::Filed { source }),
    }
}

// The key by name, as everywhere else in warlock. The size is the content's own
// bytes and not the file's: what the file holds includes the title line, which
// is sent as the project's name rather than as part of its body.
fn would(board: Board<'_>, key: &str, brief: &Brief) -> String {
    let bytes = u64::try_from(brief.content().len()).unwrap_or(u64::MAX);
    format!(
        "warlock: would file `{}` to `{}`, under the scope `{}`, with the key `{}` — {} of \
         content, and nothing was sent",
        brief.name(),
        board.team,
        board.scope,
        key,
        size(bytes)
    )
}

// Every refusal above is driven through the `Posts` seam and a temporary home,
// so no test of this module opens a socket, reads a key store that is not its
// own, or names a board anybody holds.
#[cfg(test)]
#[path = "tests/push.rs"]
mod tests;
