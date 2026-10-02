//! One scope slice's validated drafts become issues on the board, and what was
//! created becomes a cut record beside the brief's own.
//!
//! The order in [`cut`] is the promise rather than an arrangement, as it is in
//! [`mod@crate::push`]: a slice the record already names sends nothing at all,
//! a team with no `Backlog` state is refused while the slice is still nothing
//! rather than half filed, and no relation is written until every issue it
//! could name exists. The record is saved for this slice as soon as its issues
//! exist and not once at the end, because an issue nothing records is exactly
//! what the next run files a second time.
//!
//! Its own module rather than [`mod@crate::planned`]'s, which sequences the slices
//! and issues no write of its own.
//! No key is read here and none can be: the seam arrives built, as it does for
//! a cut.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use warlock_engine::drafting::Draft;
use warlock_engine::{CutRecord, Destination, fold_title, manifest_path, now_rfc3339};

use crate::error::Error;
use crate::linear::{Board, Issue as LinearIssue, NewIssue};
use crate::push::records;

/// Where one slice's issues go, which is what [`resolve_filing`] and the
/// brief's own record between them answered: the board, the project a push
/// made, and the brief as `.warlock/filed.toml` spells it.
///
/// `assignee` is the user the key belongs to, resolved once for the run by
/// [`prepare`] rather than here: this is one slice of several, and the answer is
/// the same for all of them.
///
/// [`resolve_filing`]: warlock_engine::resolve_filing
/// [`prepare`]: crate::planned::prepare
#[derive(Debug, Clone, Copy)]
pub(crate) struct Filing<'a> {
    pub(crate) brief: &'a str,
    pub(crate) project: &'a str,
    pub(crate) destination: &'a Destination,
    pub(crate) assignee: &'a str,
}

/// One slice of the project's scope, with the drafts a session settled for it
/// and the issues it waits on.
///
/// `needs` is one entry per slice this slice's `depends_on` names, holding the
/// issues that slice became: the references are written as positions in a
/// document this module never reads, and a cut record keeps identifiers rather
/// than the ids a relation is written with — so the run that filed them hands
/// the issues over itself rather than anything here reading the board back.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Slice<'a> {
    pub(crate) title: &'a str,
    pub(crate) drafts: &'a [Draft],
    pub(crate) needs: &'a [&'a [LinearIssue]],
    /// The open tickets the drafting session was shown, which are the only ones
    /// a draft's `waits_on` can name.
    pub(crate) open: &'a [LinearIssue],
}

/// What filing one slice came to.
#[derive(Debug)]
pub(crate) enum Cut {
    /// The record already names this slice, so nothing was sent: the
    /// identifiers are the ones that record holds, which is all a cut record
    /// keeps of an issue.
    Already(Vec<String>),
    /// The issues created now, whole rather than as identifiers, because the
    /// relations a later slice asks for are written by issue *id*, with one
    /// line per edge Linear turned down.
    ///
    /// The lines are the caller's to say: an issue that exists with a missing
    /// edge is something a person can fix on the board, so a refused relation
    /// is reported beside the identifiers rather than taking them down with it.
    Filed {
        issues: Vec<LinearIssue>,
        reported: Vec<String>,
    },
}

pub(crate) fn cut<W: Write>(
    linear: &impl Board,
    root: &Path,
    filing: Filing<'_>,
    slice: Slice<'_>,
    out: &mut W,
) -> Result<Cut, Error> {
    let mut filed = records(root)?;
    // Held across the requests below rather than looked up twice: the record
    // this cut is appended to has to exist before anything is sent, and a
    // second lookup afterwards would be a second answer to what happens when it
    // does not — with the issues already created by then.
    let record = filed
        .record_mut(filing.brief)
        .ok_or_else(|| Error::NoRecord {
            path: filing.brief.to_owned(),
        })?;

    // Matched on the key the record spells rather than on its title, for the
    // reason `Filed::cut_state` gives: the fold is stored, so a key somebody
    // edited is a record that no longer matches rather than one silently
    // re-matched from the title beside it.
    let key = fold_title(slice.title);
    if let Some(already) = record.cuts().iter().find(|cut| cut.key() == key) {
        let issues = already.issues().to_vec();
        let said = if issues.is_empty() {
            "was skipped in an earlier run".to_owned()
        } else {
            format!("is already cut as {}", listed(&issues))
        };
        drop(writeln!(
            out,
            "warlock: `{}` {said}, so nothing was sent",
            slice.title
        ));
        return Ok(Cut::Already(issues));
    }

    let team = linear
        .team_id(filing.destination.team_key())?
        .ok_or_else(|| Error::UnknownTeam {
            team: filing.destination.team_key().to_owned(),
            path: manifest_path(root),
        })?;
    // Before the label and before every create: a team with nowhere to put an
    // issue is a refusal that costs nothing, and the same question asked after
    // the first create would leave a slice half filed on the board.
    let state = linear
        .backlog_state(&team)?
        .ok_or_else(|| Error::NoBacklog {
            team: filing.destination.team_key().to_owned(),
        })?;
    let label = linear.issue_label_id(filing.destination.label(), &team)?;

    let mut issues = Vec::with_capacity(slice.drafts.len());
    for draft in slice.drafts {
        // A create that fails partway is a refusal and not a short cut record:
        // a record says the slice is filed, so writing one for the drafts that
        // landed would be warlock promising never to file the rest.
        let issue = linear.create_issue(&NewIssue::new(
            &draft.title,
            &draft.body,
            &team,
            filing.project,
            &label,
            &state,
            filing.assignee,
        ))?;
        issues.push(issue);
    }

    // After the loop above and never inside it: an edge can only be written
    // between two issues that exist, and a draft is blocked by drafts on either
    // side of it in the slice.
    let mut reported = relate(linear, &edges(slice, &issues));
    reported.extend(unshown(slice));

    let identifiers: Vec<String> = issues
        .iter()
        .map(|issue| issue.identifier().to_owned())
        .collect();

    // Printed before the record is saved, not after, for the reason `push`'s
    // URL is: the issues exist from here on and their identifiers are the one
    // thing that must not be lost, so they go out whatever the save does next.
    drop(writeln!(
        out,
        "warlock: cut `{}` into {}",
        slice.title,
        listed(&identifiers)
    ));

    record.push_cut(CutRecord::new(
        slice.title,
        identifiers.iter().map(String::as_str),
        now_rfc3339(),
    ));
    filed.save(root).map_err(|source| Error::Uncut {
        issues: identifiers,
        source: Box::new(source),
    })?;

    Ok(Cut::Filed { issues, reported })
}

/// A slice somebody said no to at the review, recorded as a cut that filed
/// nothing, so the next run passes it over as it passes over a filed one. Red's
/// rule: a skip is a real answer and is never retried on its own; retitling the
/// slice in the brief makes it a new slice, and that is how it comes back.
///
/// An empty `issues` is the whole of the record's saying so. A real cut always
/// files at least one issue, because the drafting repairs always leave a draft.
pub(crate) fn skip(root: &Path, brief: &str, title: &str) -> Result<(), Error> {
    let mut filed = records(root)?;
    let record = filed.record_mut(brief).ok_or_else(|| Error::NoRecord {
        path: brief.to_owned(),
    })?;
    let key = fold_title(title);
    if record.cuts().iter().any(|cut| cut.key() == key) {
        return Ok(());
    }
    record.push_cut(CutRecord::new(
        title,
        std::iter::empty::<&str>(),
        now_rfc3339(),
    ));
    filed.save(root).map_err(|source| Error::Unskipped {
        title: title.to_owned(),
        source: Box::new(source),
    })
}

/// How a settled slice is said after its heading and a dash: by the issues it
/// became, or as skipped when the record names none. See [`skip`].
pub(crate) fn settled_as(issues: &[String]) -> String {
    if issues.is_empty() {
        "skipped in an earlier run".to_owned()
    } else {
        format!("already cut as {}", listed(issues))
    }
}

/// Every edge this slice asks for, as the pair of issues it is written between:
/// the blocker first, then the issue it holds up.
///
/// De-duplicated, because two drafts naming each other — one's `blocks` and the
/// other's `blocked_by` — are one edge said twice, and Linear would take both.
fn edges<'a>(
    slice: Slice<'a>,
    issues: &'a [LinearIssue],
) -> Vec<(&'a LinearIssue, &'a LinearIssue)> {
    let mut edges = Vec::new();

    for (position, draft) in slice.drafts.iter().enumerate() {
        // Indices into this slice's own drafts, already pruned to the ones that
        // are in it by `warlock_engine::drafting::mend` — read through `get`
        // anyway, because the alternative to an edge nobody can write is a
        // panic in the middle of a slice that is already half related.
        let Some(waiting) = issues.get(position) else {
            continue;
        };

        for blocker in draft.blocked_by.iter().filter_map(|at| issues.get(*at)) {
            edges.push((blocker, waiting));
        }
        for blocked in draft.blocks.iter().filter_map(|at| issues.get(*at)) {
            edges.push((waiting, blocked));
        }
        for blocker in draft
            .waits_on
            .iter()
            .filter_map(|name| shown(slice.open, name))
        {
            edges.push((blocker, waiting));
        }
    }

    for earlier in slice.needs {
        for blocker in *earlier {
            for waiting in issues {
                edges.push((blocker, waiting));
            }
        }
    }

    let mut seen = HashSet::new();
    edges.retain(|(blocker, waiting)| seen.insert((blocker.id(), waiting.id())));
    edges
}

fn shown<'a>(open: &'a [LinearIssue], name: &str) -> Option<&'a LinearIssue> {
    open.iter()
        .find(|issue| issue.identifier().eq_ignore_ascii_case(name.trim()))
}

// A ticket a draft waits on that the session was not shown: not resolved by
// asking the board, because what the session cannot see it cannot have judged.
fn unshown(slice: Slice<'_>) -> Vec<String> {
    slice
        .drafts
        .iter()
        .flat_map(|draft| {
            draft
                .waits_on
                .iter()
                .filter(|name| shown(slice.open, name).is_none())
                .map(move |name| {
                    format!(
                        "`{}` waits on `{name}`, which is not one of the open tickets its \
                         session was shown, so no edge was written",
                        draft.title
                    )
                })
        })
        .collect()
}

fn relate(linear: &impl Board, edges: &[(&LinearIssue, &LinearIssue)]) -> Vec<String> {
    edges
        .iter()
        .filter_map(|(blocker, waiting)| {
            linear
                .create_relation(blocker.id(), waiting.id())
                .err()
                .map(|error| {
                    format!(
                        "`{}` was not written as blocking `{}`: {error}",
                        blocker.identifier(),
                        waiting.identifier()
                    )
                })
        })
        .collect()
}

/// Say on the project what was filed out of it, answering with the one line to
/// report when Linear turns the comment down.
///
/// Nothing here decides *when* to say it: the brief's one comment lands after
/// the last slice settles and only when something was filed, which is the run's
/// question and not this operation's.
pub(crate) fn announce(linear: &impl Board, project: &str, issues: &[String]) -> Option<String> {
    let body = format!(
        "Warlock cut this project into {}.\n\nThe project's status was not moved.",
        listed(issues)
    );

    linear
        .comment_on_project(project, &body)
        .err()
        .map(|error| format!("the project was not commented on: {error}"))
}

// Shared with `error.rs`, which names the same identifiers in the refusal that
// says they were not recorded: one spelling for the list, so the line a person
// reads off a cut and the line they read off its failure name the issues the
// same way.
pub(crate) fn listed(issues: &[String]) -> String {
    issues
        .iter()
        .map(|issue| format!("`{issue}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "tests/cut.rs"]
mod tests;
