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
//! The half that spends is [`filed_each`]: one one-shot session per slice left
//! to cut, its drafts filed through [`cut::cut`], and the project's one comment
//! after the last of them. It is the only thing here that sends a mutation, and
//! the status is not moved on that road either — an issue is created, an edge is
//! written and a comment is said, and nothing else.
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

use warlock_engine::drafting::Draft;
use warlock_engine::{filed_path, resolve_filing};
use warlock_tui::{
    ChatAgent, Converses, Drafted, Drafting, FetchedProject, LinearIssue, Posts, Replied, Slice,
    fetch_project, scope_block_in,
};

// The module rather than its `cut` and `Slice`, which would both be a second
// name for something this file already has: the slices here are the document's,
// and `cut::Slice` is one slice's drafts on their way to a board.
use crate::cut::{self, Cut, Filing, listed};
use crate::error::Error;
use crate::push::{Board, client, records};
use crate::standing::{FOR_PULL, Standing};

// The one status a project is read back from, and the only spelling accepted:
// the comparison below trims and folds case, so `planned` and ` Planned ` are
// this and `Backlog` is not.
const PLANNED: &str = "Planned";

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
            say(out, &line);
        }
        return Ok(());
    }

    // Read off the same file the skips came from rather than threaded out of
    // `planned`, for the reason `spelled` is spelled again above. The refusal
    // cannot happen — `planned` has already refused a brief no record names —
    // and it is that same sentence rather than a panic, because a `warlock
    // pull` that aborted the process over its own impossible branch would be a
    // worse answer than the one it would have printed.
    let project_id = filed
        .record(&spelled)
        .ok_or_else(|| Error::NoRecord {
            path: spelled.clone(),
        })?
        .project_id();
    let filing = Filing {
        brief: &spelled,
        project: project_id,
        team: board.team,
        label: board.label,
    };

    let created = filed_each(
        &linear,
        standing.repo_root(),
        filing,
        block.brief(),
        &cutting,
        agent,
        out,
    )?;

    // After the last slice and only when something was filed: a run that skipped
    // every slice it could and drafted nothing usable has nothing to say on the
    // project, and saying it anyway would be a comment per run rather than a
    // comment per cut.
    //
    // A comment Linear turns down is a reported line and not a failure: the
    // issues exist and are recorded by the time it is said, and losing a run
    // over a note on a project would be the tail wagging the cut.
    if !created.is_empty()
        && let Some(line) = cut::announce(&linear, project_id, &created)
    {
        say(out, &line);
    }

    Ok(())
}

/// Every slice in the cut order: the skips said, the rest drafted and filed, and
/// the identifiers this run created handed back for the project's one comment.
///
/// Split from [`pulled`] because it is the half that spends: everything above it
/// resolves where the work goes and what is left to do, and everything here
/// sends. Nothing in it reads the environment — the root, the board and the seam
/// all arrive — so the whole of it runs against a temporary repository.
fn filed_each<P: Posts, A: Converses, W: Write>(
    linear: &P,
    root: &Path,
    filing: Filing<'_>,
    brief: &str,
    cutting: &[Cutting<'_>],
    agent: &A,
    out: &mut W,
) -> Result<Vec<String>, Error> {
    // One entry per slice of the project, in document order, holding the issues
    // that slice came to: what `depends_on` names is a position, so a position
    // is what this is indexed by. A slice skipped as already cut fills its entry
    // from its record and a slice that was filed now fills it with whole issues,
    // which is what lets a relation be written from either.
    //
    // Empty is "nothing this run can name as a blocker" — a slice that never
    // drafted — and an empty entry is left out of `needs` rather than written as
    // an edge to nothing.
    let mut became: Vec<Vec<LinearIssue>> = vec![Vec::new(); cutting.len()];
    // What was created here, for the project's one comment. The skips are not in
    // it: the comment says what this run filed, and an earlier run's issues were
    // named by an earlier run's comment.
    let mut created: Vec<String> = Vec::new();

    for (place, slice) in cutting.iter().enumerate() {
        let at = slice.slice.position() - 1;
        let heading = heading(place, cutting.len(), slice.slice);

        if let Some(issues) = slice.already {
            say(
                out,
                &format!(
                    "{heading} — already cut as {}, so nothing was sent",
                    listed(issues)
                ),
            );
            became[at] = issues
                .iter()
                .map(|issue| LinearIssue::recorded(issue))
                .collect();
            continue;
        }

        say(out, &format!("{heading} — drafting"));
        let Some(drafts) = drafted(agent, brief, slice.slice, out) else {
            continue;
        };

        // Built here and not held across the loop: the references are into
        // `became`, which the line after the cut writes to, and a list kept any
        // longer than the call it is made for would be a borrow of the thing
        // this run exists to fill in.
        let needs: Vec<&[LinearIssue]> = slice
            .slice
            .depends_on()
            .iter()
            .filter_map(|position| became.get(position - 1))
            .map(Vec::as_slice)
            .filter(|issues| !issues.is_empty())
            .collect();

        // `?`, and not a reported line: what reaches here is a team with no
        // `Backlog` state, a create Linear turned down, or a record that would
        // not save — and carrying on to the next slice after any of the three
        // would be warlock filing a second slice into the same wall, or
        // recording nothing about issues that now exist.
        match cut::cut(
            linear,
            root,
            filing,
            cut::Slice {
                title: slice.slice.heading(),
                drafts: &drafts,
                needs: &needs,
            },
            out,
        )? {
            // Unreachable while the skips above are read off the same file
            // `cut` matches against, and handled rather than asserted: the two
            // readings agreeing is worth nothing to assert and a panic in the
            // middle of a run that has filed issues is worth avoiding.
            Cut::Already(issues) => {
                became[at] = issues
                    .iter()
                    .map(|issue| LinearIssue::recorded(issue))
                    .collect();
            }
            Cut::Filed { issues, reported } => {
                // The edges Linear turned down, said here because `cut` hands
                // them back rather than printing them: an issue that exists with
                // a missing edge is a thing a person can fix on the board, and
                // it is only fixable if they are told.
                for line in reported {
                    say(out, &line);
                }
                created.extend(issues.iter().map(|issue| issue.identifier().to_owned()));
                became[at] = issues;
            }
        }
    }

    Ok(created)
}

/// One slice drafted in one session, or `None` with what went wrong already
/// said.
///
/// [`Drafting::one_shot`] rather than [`Drafting::for_slice`]: there is nobody
/// at a shell to put a question to, so the session is told up front that it
/// cannot ask one and is held to no rounds at all. One session per slice, opened
/// here and dropped at the end of this call, so nothing a slice said reaches the
/// next one.
///
/// Every way a slice can fail to draft is a reported line and the next slice,
/// not the end of the run: the slices left are other work, they were ordered so
/// that nothing is filed before what it waits on, and a run that stopped would
/// leave the operator re-running it to reach them anyway.
fn drafted<A: Converses, W: Write>(
    agent: &A,
    brief: &str,
    slice: &Slice,
    out: &mut W,
) -> Option<Vec<Draft>> {
    let mut session = Drafting::one_shot(agent, brief, slice.heading(), slice.prose());

    let replied = match session.open() {
        Ok(replied) => replied,
        // A missing binary, a timeout or a cancel, none of which is better the
        // second time — see [`Drafting`]'s own note — so the slice is left
        // uncut rather than asked again.
        Err(error) => {
            say(out, &format!("{} was not drafted: {error}", named(slice)));
            return None;
        }
    };

    match replied {
        Replied::Answer(Drafted::Drafts { fill, repairs }) => {
            // On stdout with the progress for `running.rs`'s reason: a repaired
            // draft is a ticket that was filed, not one that was missed, and a
            // log read tomorrow should be able to tell the two apart.
            for repair in repairs {
                say(out, &format!("{} — {repair}", named(slice)));
            }
            Some(fill.drafts)
        }
        // Four answers and not an object among them. Reported and left uncut
        // rather than filed as the stand-in ticket the document road's floor
        // would supply: a supplied ticket is warlock putting work nobody planned
        // on somebody's board.
        Replied::Answer(Drafted::Unusable(defect)) => {
            say(out, &format!("{} was not drafted: {defect}", named(slice)));
            None
        }
        // A session with no rounds hands a question back to nobody, so this is
        // unreachable — and said rather than panicked on for the reason
        // `Cut::Already` is handled above.
        Replied::Question(_) => {
            say(
                out,
                &format!(
                    "{} was not drafted: the session asked a question and there is nobody to \
                     answer it",
                    named(slice)
                ),
            );
            None
        }
    }
}

// The prefix of every line about one slice that is not its place in the run: the
// position in the document and the heading, which is what a reader takes back to
// the brief.
fn named(slice: &Slice) -> String {
    format!("slice {} `{}`", slice.position(), slice.heading())
}

// A failed write is ignored, exactly as `running.rs`'s `Progress` ignores one
// and for its reason: `warlock pull docs/brief.md | head -1` is a closed stdout,
// and failing a run of drafting sessions over the state of a pipe would spend
// somebody's tokens and then throw away what they bought.
fn say<W: Write>(out: &mut W, fact: &str) {
    drop(writeln!(out, "warlock: {fact}"));
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
        let heading = heading(place, cutting.len(), slice.slice);
        lines.push(match slice.already {
            Some(issues) => format!("{heading} — already cut as {}", listed(issues)),
            None => heading,
        });
    }

    lines
}

// One slice's line, shared by the dry run's report and the run itself so that
// the two read alike: a person who has read a `--dry-run` is looking for the
// same slices in the same order when they take the flag off.
//
// The fraction is the place in the cut order, one-based as `running.rs`'s is,
// and the position is where the slice sits in the document — the two differ
// exactly when a `depends_on` line moved something, and the second is what finds
// the slice in the brief.
fn heading(place: usize, total: usize, slice: &Slice) -> String {
    format!("[{}/{total}] {}", place + 1, named(slice))
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
