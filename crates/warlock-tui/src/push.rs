//! A brief becomes a project on the board this machine's sigil names, and the
//! address of that project is written into `.warlock/filed.toml`: `warlock push
//! <PATH>` here, and the panel's `/push` in [`mod@crate::pushing`], both through
//! [`prepare`] and [`file`].
//!
//! The split is the promise rather than an arrangement: everything that can
//! refuse — no board, several boards, an unbound checkout, a brief already
//! filed, a document that is not a brief — is asked by [`prepare`], which opens
//! no socket, so a refusal reaches nobody's workspace and costs nothing. A dry
//! run and the panel's dialog are both answered from its [`Prepared`], and the
//! socket is opened inside [`file`] and nowhere earlier.
//!
//! No `--json`, matching the other writing subcommands: the answer worth
//! parsing is the record, which is a file rather than a stream to be caught.
//!
//! No key value is printed here and none can be. [`Prepared`] carries one with a
//! redacting `Debug`, it is read on exactly one line — the opener's — and
//! [`sent`], the half that prints, takes a [`Destination`], which holds the key
//! by name only.

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use warlock_engine::{
    Destination, Filed, FiledRecord, Manifest, filed, manifest_path, now_rfc3339, resolve_filing,
    to_manifest_path,
};
use warlock_tui::{Board, Brief, LinearOpener, NewProject, Opens, brief_at, size};

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
        &LinearOpener,
        &mut io::stdout(),
    )
}

// Split from `push` so that the environment — the working directory, the
// repository root, the home the sigils and the key store sit under — is three
// parameters rather than three reads, the way `check` splits `checked_onto`:
// the whole command can then be run against a temporary repository and a
// temporary home, and no test in this crate can reach the developer's real key
// store by standing in the wrong directory.
fn pushed<O: Opens, W: Write>(
    standing: &Standing,
    home: &Path,
    path: &Path,
    scope: Option<&str>,
    dry_run: bool,
    open: &O,
    out: &mut W,
) -> Result<(), Error> {
    let manifest = standing.manifest()?;
    let prepared = prepare(
        &manifest,
        standing.repo_root(),
        home,
        &standing.target(path),
        scope,
    )?;

    if dry_run {
        drop(writeln!(out, "{}", would(&prepared)));
        return Ok(());
    }

    // The address is dropped here and nowhere else: `file` has already printed
    // it, and it comes back for the panel's sake, which has no `out` to read.
    file(&prepared, open, out).map(drop)
}

/// A push with every refusal already asked: the brief read, the board resolved,
/// and the brief's manifest spelling checked against `.warlock/filed.toml`.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Prepared {
    root: PathBuf,
    path: PathBuf,
    spelled: String,
    brief: Brief,
    destination: Destination,
    value: String,
}

impl Prepared {
    pub(crate) const fn brief(&self) -> &Brief {
        &self.brief
    }

    pub(crate) const fn destination(&self) -> &Destination {
        &self.destination
    }
}

// Hand-written for `Target`'s reason: this holds the key value, and the panel
// keeps one in the window its dialog is drawn from, which a failing assertion
// anywhere in the suite may print.
impl fmt::Debug for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Prepared")
            .field("root", &self.root)
            .field("path", &self.path)
            .field("spelled", &self.spelled)
            .field("brief", &self.brief)
            .field("destination", &self.destination)
            .field("value", &"<redacted>")
            .finish()
    }
}

// `path` is joined onto nothing here: the subcommand hands in its argument
// joined onto the working directory and the panel its manifest spelling joined
// onto the root, and both are spelled back against the root below, which is
// what a record is keyed by.
//
// The board is resolved before the records are read and the records before the
// brief, so a machine that cannot say where it would file is told that first,
// and a brief already filed is answered with its address even if it has since
// stopped being a brief.
pub(crate) fn prepare(
    manifest: &Manifest,
    root: &Path,
    home: &Path,
    path: &Path,
    scope: Option<&str>,
) -> Result<Prepared, Error> {
    let target =
        resolve_filing(manifest, root, home, scope).map_err(|source| Error::Filing { source })?;

    let spelled = to_manifest_path(root, path).map_err(|source| Error::Unspellable { source })?;
    unfiled(&records(root)?, &spelled)?;

    let brief = brief_at(root, path).map_err(|source| Error::Brief { source })?;

    Ok(Prepared {
        root: root.to_path_buf(),
        path: path.to_path_buf(),
        spelled,
        brief,
        destination: target.destination(),
        value: target.value().to_owned(),
    })
}

// The records are read again rather than carried from `prepare`: the panel's
// dialog can sit open while another shell's push appends to the file, and
// saving a copy read before that would drop that push's record — and file this
// brief a second time if the record was its own.
pub(crate) fn file<O: Opens, W: Write>(
    prepared: &Prepared,
    open: &O,
    out: &mut W,
) -> Result<String, Error> {
    let filed = records(&prepared.root)?;
    unfiled(&filed, &prepared.spelled)?;

    let linear = open.open(&prepared.value);
    sent(
        &linear,
        &prepared.root,
        &prepared.destination,
        &prepared.brief,
        &prepared.path,
        filed,
        out,
    )
}

fn unfiled(filed: &Filed, spelled: &str) -> Result<(), Error> {
    match filed.record(spelled) {
        Some(already) => Err(Error::AlreadyFiled {
            path: spelled.to_owned(),
            url: already.url().to_owned(),
        }),
        None => Ok(()),
    }
}

// Split from [`file`] over the [`Board`] seam and past the opener, so the order
// — team, status, project, record — is assertable against an in-memory stand-in,
// and so a test can hand it a root whose record cannot be saved without having
// to get a brief read from under one.
//
// The address comes back for the panel, which has no `out` to read afterwards,
// and the URL is the one thing a push must not lose.
//
// The label is resolved inside `create_project`, as its first request, and is
// deliberately not resolved here as well. That ordering is `linear.rs`'s
// decision: the label is the only mark saying warlock filed a project, nothing
// here can take a project back, so the label exists before the project does.
// Resolving it here too would ask for the same name a second time and would not
// reach the case the ticket describes — a label that fails after a create that
// landed — because with the label first there is no such create. So a label that
// will not resolve is a failed `create_project`: a refusal with nothing created,
// nothing recorded and no URL, and that is the honest report of what happened.
pub(crate) fn sent<W: Write>(
    linear: &impl Board,
    root: &Path,
    destination: &Destination,
    brief: &Brief,
    path: &Path,
    mut filed: Filed,
    out: &mut W,
) -> Result<String, Error> {
    let team = linear
        .team_id(destination.team())
        .map_err(|source| Error::Linear { source })?
        .ok_or_else(|| Error::UnknownTeam {
            team: destination.team().to_owned(),
            path: manifest_path(root),
        })?;
    // `None` is a workspace with no status by that name, which is a project
    // filed with no status at all rather than a failure.
    let status = linear
        .backlog_status()
        .map_err(|source| Error::Linear { source })?;

    let project = linear
        .create_project(
            &NewProject::new(brief.name(), brief.content(), &team, destination.label())
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
        destination.team(),
        destination.label(),
        project.url()
    ));

    filed.push(
        FiledRecord::new(
            root,
            path,
            project.id(),
            project.url(),
            destination.scope(),
            destination.team(),
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
fn would(prepared: &Prepared) -> String {
    let (brief, destination) = (&prepared.brief, &prepared.destination);
    let bytes = u64::try_from(brief.content().len()).unwrap_or(u64::MAX);
    format!(
        "warlock: would file `{}` to `{}`, under the scope `{}`, with the key `{}` — {} of \
         content, and nothing was sent",
        brief.name(),
        destination.team(),
        destination.scope(),
        destination.key(),
        size(bytes)
    )
}

// Every refusal above is driven through the [`Board`] seam and a temporary home,
// so no test of this module opens a socket, reads a key store that is not its
// own, or names a board anybody holds.
#[cfg(test)]
#[path = "tests/push.rs"]
mod tests;
