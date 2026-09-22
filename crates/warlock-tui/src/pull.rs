//! `warlock pull <PATH>`: the project a push recorded for that brief is read
//! back, its scope block is parsed into slices, and the slices no cut record
//! names are the work.
//!
//! The order in [`pulled`] is the promise rather than an arrangement, as it is
//! in [`mod@crate::push`]: the board and the brief's record are resolved before
//! the fetch, a project that is not `Planned` stops the sequence where the
//! answer that said so arrived, and a project whose every slice is already cut
//! is refused before the repository is read by anything. Nothing above the
//! drafting half writes — no mutation is issued, and the status is not moved in
//! either direction.
//!
//! No `--json`, matching [`mod@crate::push`] and the other verbs that spend
//! something: the answer worth parsing is the record, which is a file rather
//! than a stream to be caught.
//!
//! No key value is printed here and none can be. The target [`resolve_filing`]
//! hands back carries one, it is read on exactly one line — the constructor —
//! and everything below that takes the board as [`Board`]'s three strings and
//! never that target.
//!
//! [`resolve_filing`]: warlock_engine::resolve_filing

use std::io::{self, Write};
use std::path::Path;

use warlock_engine::{filed_path, resolve_filing};
use warlock_tui::{
    ChatAgent, Converses, FetchedProject, Posts, Slice, fetch_project, scope_block_in,
};

use crate::cut::listed;
use crate::error::Error;
use crate::push::{Board, client, records};
use crate::standing::{FOR_PULL, Standing};

// The one status a project is read back from, and the only spelling accepted:
// the comparison below trims and folds case, so `planned` and ` Planned ` are
// this and `Backlog` is not.
const PLANNED: &str = "Planned";

// Unconditional rather than `cfg_attr(not(test), …)`, unlike the refusals this
// module raises: the tests below drive [`pulled`], which is the half with the
// environment as parameters, so nothing reaches this one until the clap
// subcommand does.
#[expect(
    dead_code,
    reason = "the verb lands before the door that dispatches it: the clap \
              subcommand is a later slice of brief 23, and this is what its arm \
              will call"
)]
pub(crate) fn pull(path: &Path, scope: Option<&str>, dry_run: bool) -> Result<(), Error> {
    let standing = Standing::here(FOR_PULL)?;
    // The error rather than `check`'s `.ok()`, for [`mod@crate::push`]'s reason:
    // the sigils under the home pick the board and the key store beside them is
    // what reads it back, so a machine with no home has nothing to pull with
    // rather than an answer of "nothing held".
    let home = Standing::home()?;

    pulled(
        &standing,
        &home,
        path,
        scope,
        dry_run,
        client,
        // Its own conversation at the register the brief was written in, built
        // once for the run and cheap to build: an agent is a command line and a
        // timeout, so no `claude` exists until a slice asks for a draft. One
        // agent rather than one per slice would be one conversation carrying
        // every earlier slice's talk into the next, which is the opposite of
        // what a per-slice session is for — so the sessions below are opened off
        // this one.
        &ChatAgent::drafting(),
        &mut io::stdout(),
    )
}

// Split from `pull` the way `pushed` is split from `push`: the environment — the
// working directory, the repository root, the home the sigils and the key store
// sit under — is three parameters rather than three reads, so every refusal
// below can be run against a temporary repository and a temporary home, and no
// test in this crate can reach the developer's real key store by standing in
// the wrong directory.
//
// `open` is the socket, as a parameter, and `agent` is the model: both are
// called on exactly one line each, and a test hands in stand-ins that panic
// when they are reached — which is what makes "this refusal sends nothing" and
// "a dry run drafts nothing" assertions about the order here rather than
// readings of it.
//
// The order is the ticket's: the board and the brief's own record are resolved
// before the fetch, so a machine that cannot say which board it is standing at
// refuses without a request; and a project with no slice left to cut refuses
// before anything reads the repository, because a run with nothing to draft is
// not a run that succeeded quietly.
#[expect(
    clippy::too_many_arguments,
    reason = "the environment, the seam and the model are parameters rather \
              than reads, which is the whole of what lets every refusal run \
              against a temporary repository and a temporary home"
)]
#[expect(
    unused_variables,
    reason = "the agent is the drafting half's, which is the next slice of \
              brief 23; `expect` so that the half reaching for it has to take \
              this off"
)]
fn pulled<P: Posts, O: FnOnce(&str) -> P, A: Converses, W: Write>(
    standing: &Standing,
    home: &Path,
    path: &Path,
    scope: Option<&str>,
    dry_run: bool,
    open: O,
    agent: &A,
    out: &mut W,
) -> Result<(), Error> {
    let manifest = standing.manifest()?;

    let target = resolve_filing(&manifest, standing.repo_root(), home, scope)
        .map_err(|source| Error::Filing { source })?;
    let record = target.record();
    let board = Board {
        scope: target.scope(),
        team: record.team(),
        label: record.label(),
    };

    // The key is read here and nowhere else in this module, and unlike a push's
    // dry run a pull's has a request to make: what it reports is the project on
    // the board, so the client exists either way. Nothing is sent by building
    // it, which is why the record refusal inside `planned` still costs nothing.
    let linear = open(target.value());
    let project = planned(standing, &linear, path)?;

    let block = scope_block_in(project.content()).map_err(|source| Error::ScopeBlock { source })?;
    // `ordered` and not `slices`: the order is not a display detail but the
    // order tickets are filed and blocked in, so a slice is only reached once
    // everything it waits on has been.
    let slices = block.ordered();

    // Spelled again rather than threaded out of `planned`, which hands back the
    // project: this is the key the cut records are filed under, it is one join
    // and one strip of the root, and a `planned` that answered with it as well
    // would be an operation reporting two things so that its caller can skip a
    // line.
    let spelled = standing.spelled(&standing.target(path))?;
    let filed = records(standing.repo_root())?;
    // Asked of the file rather than worked out here: `cut_state` matches on the
    // key a record spells and not on a fresh fold of the title beside it, which
    // is the difference between a slice somebody renamed and one warlock quietly
    // treats as already filed.
    let state = filed.cut_state(&spelled, slices.iter().map(|slice| slice.heading()));
    if state.uncut().is_empty() {
        return Err(Error::AllCut { path: spelled });
    }

    // Paired here, above both halves, because the drafting half needs the same
    // answer: a slice that was skipped is still a slice the ones after it may
    // depend on, and the issues it became come off its cut record.
    let cutting: Vec<Cutting<'_>> = slices
        .iter()
        .map(|slice| Cutting {
            slice,
            already: state
                .cut()
                .iter()
                .find(|(title, _)| *title == slice.heading())
                .map(|(_, cut)| cut.issues()),
        })
        .collect();

    if dry_run {
        for line in would(board, &project, &cutting) {
            drop(writeln!(out, "warlock: {line}"));
        }
        return Ok(());
    }

    // The half that spends, and the one this slice of brief 23 does not carry:
    // one drafting session per uncut slice, its drafts filed through
    // [`cut::cut`], and the project's one comment after the last of them. It
    // lands here, with the board, the project, the order and the skips all
    // resolved above it and the seam and the agent in hand.
    //
    // A panic rather than a quiet `Ok(())`: `warlock pull` is not a clap
    // subcommand yet, so nothing can reach this line, and a run that reported
    // success having drafted nothing is the one outcome worse than a loud stop.
    //
    // [`cut::cut`]: crate::cut::cut
    todo!(
        "draft and file the {} slices left to cut",
        state.uncut().len()
    )
}

/// One slice of the project's scope with what `.warlock/filed.toml` already
/// says about it.
///
/// `already` is the identifiers a cut record names, which is all such a record
/// keeps of an issue, and `None` is a slice still to draft. Kept as a pair
/// rather than as two lists because the order is the thing being reported and
/// acted on: a reader watching a dry run and the half that files are both
/// walking the project's slices in the order they will be cut.
#[derive(Debug)]
struct Cutting<'a> {
    slice: &'a Slice,
    already: Option<&'a [String]>,
}

// One line for the run and one per slice, rather than push's single sentence:
// what a pull is about to do is an order, and an order is not a thing one line
// can say. The fraction is the place in the cut order and the position is where
// the slice sits in the document, so a reader can find it in the brief — the
// two differ exactly when a `depends_on` line moved something.
//
// Handed back rather than printed, for the reason everything else in this crate
// takes its writer as a parameter: the lines are the contract and a function
// that printed could only be tested by spawning a process to read.
fn would(board: Board<'_>, project: &FetchedProject, cutting: &[Cutting<'_>]) -> Vec<String> {
    let cut = cutting
        .iter()
        .filter(|slice| slice.already.is_some())
        .count();
    let mut lines = vec![format!(
        "would cut `{}`, which is `{}`, into `{}` under the scope `{}` — {}, {} already cut, and \
         nothing was drafted",
        project.name(),
        // `planned` has already refused anything else, so the fallback is
        // unreachable; the status is printed as the board spelled it rather
        // than as this module spells it, because that is what a reader will
        // find when they go and look.
        project.status().unwrap_or(PLANNED),
        board.team,
        board.scope,
        counted(cutting.len()),
        cut
    )];

    for (place, slice) in cutting.iter().enumerate() {
        let heading = format!(
            "[{}/{}] slice {} `{}`",
            place + 1,
            cutting.len(),
            slice.slice.position(),
            slice.slice.heading()
        );
        lines.push(match slice.already {
            Some(issues) => format!("{heading} — already cut as {}", listed(issues)),
            None => heading,
        });
    }

    lines
}

// `1 slice`, `9 slices`, so the line above is not worded twice or read as
// `1 slices`.
fn counted(count: usize) -> String {
    let noun = if count == 1 { "slice" } else { "slices" };
    format!("{count} {noun}")
}

// Split from any caller the way `push`'s `pushed` is split from `push`: the
// environment is the standing and the path, so every refusal here runs against a
// temporary repository, and the seam is a parameter, so a refusal that must send
// nothing is asserted with a stand-in that panics when it is posted to.
//
// `records` rather than a second `Filed::load`: the file a push appends to and
// the file a pull resolves against are one file, and a second loader here would
// be a second reading of what a missing one means.
pub(crate) fn planned(
    standing: &Standing,
    linear: &impl Posts,
    path: &Path,
) -> Result<FetchedProject, Error> {
    let spelled = standing.spelled(&standing.target(path))?;
    let filed = records(standing.repo_root())?;
    let Some(record) = filed.record(&spelled) else {
        return Err(Error::NoRecord { path: spelled });
    };

    let project = fetch_project(linear, record.project_id())
        .map_err(|source| Error::Linear { source })?
        .ok_or_else(|| Error::UnknownProject {
            id: record.project_id().to_owned(),
            path: filed_path(standing.repo_root()),
        })?;

    if !is_planned(project.status()) {
        return Err(Error::NotPlanned {
            path: spelled,
            status: project.status().map(ToOwned::to_owned),
        });
    }

    Ok(project)
}

// Trimmed and case-folded because the name is typed into Linear by a person and
// read back over a wire, and neither end promises the capitalisation warlock
// filed it under. Nothing wider than that: `Planned` is the gate, so a workspace
// that spells its planned column something else is a refusal rather than a
// guess.
fn is_planned(status: Option<&str>) -> bool {
    status.is_some_and(|status| status.trim().eq_ignore_ascii_case(PLANNED))
}

#[cfg(test)]
#[path = "tests/pull.rs"]
mod tests;
