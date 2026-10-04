//! A brief becomes a project on the board a scope files to: `warlock push
//! <SCOPE> <PATH>` here, and the panel's `/push` in [`mod@crate::pushing`], both
//! through [`prepare`] and [`file`]. Nothing is written on this machine: the URL
//! printed is the whole record, and the brief can be deleted once it is filed.
//!
//! The split is the promise rather than an arrangement: everything that can
//! refuse without asking the board — a scope that is not a board, an unbound
//! checkout, a document that is not a brief — is asked by [`prepare`], which
//! opens no socket, so a refusal reaches nobody's workspace and costs nothing. A
//! dry run and the panel's dialog are both answered from its [`Prepared`], and
//! the socket is opened inside [`file`] and nowhere earlier.
//!
//! No key value is printed here and none can be. [`Prepared`] carries one with a
//! redacting `Debug`, it is read on exactly one line — the opener's — and
//! [`sent`], the half that prints, takes a [`Destination`], which holds the key
//! by name only.

use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use warlock_engine::{Destination, Manifest, manifest_path, resolve_filing};

use crate::account::size;
use crate::brief::{Brief, brief_at};
use crate::error::Error;
use crate::linear::{Board, NewProject, Opener as LinearOpener, Opens};
use crate::standing::{FOR_PUSH, Standing};

pub fn push(scope: &str, path: &Path, dry_run: bool) -> Result<(), Error> {
    let standing = Standing::here(FOR_PUSH)?;
    // The error rather than `check`'s `.ok()`: the sigils under the home pick
    // the board and the key store beside them is what files to it, so a machine
    // with no home has nothing to push with rather than an answer of "nothing
    // held".
    let home = Standing::home()?;

    pushed(
        &standing,
        &home,
        scope,
        path,
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
    scope: &str,
    path: &Path,
    dry_run: bool,
    open: &O,
    out: &mut W,
) -> Result<(), Error> {
    let manifest = standing.manifest()?;
    let prepared = prepare(
        &manifest,
        standing.repo_root(),
        home,
        scope,
        &standing.target(path),
    )?;

    if dry_run {
        drop(writeln!(out, "{}", would(&prepared)));
        return Ok(());
    }

    // The address is dropped here and nowhere else: `file` has already printed
    // it, and it comes back for the panel's sake, which has no `out` to read.
    file(&prepared, open, out).map(drop)
}

/// A push with every refusal that costs nothing already asked: the brief read
/// and the board resolved.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Prepared {
    root: PathBuf,
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
            .field("brief", &self.brief)
            .field("destination", &self.destination)
            .field("value", &"<redacted>")
            .finish()
    }
}

// The board is resolved before the brief is read, so a machine that cannot say
// where it would file is told that first.
pub(crate) fn prepare(
    manifest: &Manifest,
    root: &Path,
    home: &Path,
    scope: &str,
    path: &Path,
) -> Result<Prepared, Error> {
    let target = resolve_filing(manifest, root, home, Some(scope))
        .map_err(|source| Error::Filing { source })?;

    let brief = brief_at(root, path).map_err(|source| Error::Brief { source })?;

    Ok(Prepared {
        root: root.to_path_buf(),
        brief,
        destination: target.destination(),
        value: target.value().to_owned(),
    })
}

pub(crate) fn file<O: Opens, W: Write>(
    prepared: &Prepared,
    open: &O,
    out: &mut W,
) -> Result<String, Error> {
    let linear = open.open(&prepared.value);
    sent(
        &linear,
        &prepared.root,
        &prepared.destination,
        &prepared.brief,
        out,
    )
}

// Split from [`file`] over the [`Board`] seam and past the opener, so the order
// — team, name, status, project — is assertable against an in-memory stand-in.
//
// The address comes back for the panel, which has no `out` to read afterwards,
// and the URL is the one thing a push must not lose.
//
// The name is asked of the board before anything is created: with nothing on
// this machine remembering a push, a project of the same name in the team is
// the only sign this brief was filed before. Asked and then created, so two
// pushes racing each other can both pass it; that is two projects a person
// deletes one of, and a lock on the board was not worth building for it.
//
// The label is resolved inside `create_project`, as its first request, and is
// deliberately not resolved here as well. That ordering is `linear.rs`'s
// decision: the label is the only mark saying warlock filed a project, nothing
// here can take a project back, so the label exists before the project does.
// Resolving it here too would ask for the same name a second time, and a label
// that will not resolve is already a failed `create_project` with nothing
// created and no URL.
pub(crate) fn sent<W: Write>(
    linear: &impl Board,
    root: &Path,
    destination: &Destination,
    brief: &Brief,
    out: &mut W,
) -> Result<String, Error> {
    let team = linear
        .team_id(destination.team_key())?
        .ok_or_else(|| Error::UnknownTeam {
            team: destination.team_key().to_owned(),
            path: manifest_path(root),
        })?;
    if let Some(url) = linear.project_named(destination.team_key(), brief.name())? {
        return Err(Error::AlreadyFiled {
            name: brief.name().to_owned(),
            url,
        });
    }
    // `None` is a workspace with no status by that name, which is a project
    // filed with no status at all rather than a failure.
    let status = linear.backlog_status()?;

    let project = linear.create_project(
        &NewProject::new(brief.name(), brief.content(), &team, destination.label())
            .with_status(status.as_deref()),
    )?;

    drop(writeln!(
        out,
        "warlock: filed `{}` to `{}`, labelled `{}`: {}",
        brief.name(),
        destination.team_key(),
        destination.label(),
        project.url()
    ));

    Ok(project.url().to_owned())
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
        destination.team_key(),
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
