//! One scope slice's validated drafts become issues on the board, and what was
//! created becomes a note on the project.
//!
//! The order in [`cut`] is the promise rather than an arrangement, as it is in
//! [`mod@crate::push`]: a team with no `Backlog` state is refused while the
//! slice is still nothing rather than half filed, and no relation is written
//! until every issue it could name exists. The note is said for this slice as
//! soon as its issues exist and not once at the end, because an issue no note
//! names is exactly what the next run files a second time.
//!
//! The notes are the whole ledger. Nothing on this machine records a cut, so a
//! fresh clone with a key and a scope drafts on from where the board says.
//!
//! Its own module rather than [`mod@crate::planned`]'s, which sequences the slices
//! and issues no write of its own.
//! No key is read here and none can be: the seam arrives built, as it does for
//! a cut.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

use warlock_engine::drafting::{Draft, is_identifier};
use warlock_engine::{Destination, manifest_path};

use crate::error::Error;
use crate::linear::{Board, CUT_NOTE, Issue as LinearIssue, NewIssue, SKIP_NOTE};
use crate::queue::IN_PROGRESS;

/// Where one slice's issues go: the board [`resolve_filing`] answered and the
/// project the draft was named with, by Linear's own id.
///
/// `assignee` is the user the key belongs to, resolved once for the run by
/// [`prepare`] rather than here: this is one slice of several, and the answer is
/// the same for all of them.
///
/// [`resolve_filing`]: warlock_engine::resolve_filing
/// [`prepare`]: crate::planned::prepare
#[derive(Debug, Clone, Copy)]
pub(crate) struct Filing<'a> {
    pub(crate) project: &'a str,
    pub(crate) destination: &'a Destination,
    pub(crate) assignee: &'a str,
}

/// One slice of the project's scope, with the drafts a session settled for it
/// and the issues it waits on.
///
/// `needs` is one entry per slice this slice's `depends_on` names, holding the
/// issues that slice became: the references are written as positions in a
/// document this module never reads, so the run hands the issues over itself
/// rather than anything here reading the board back.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Slice<'a> {
    pub(crate) title: &'a str,
    pub(crate) drafts: &'a [Draft],
    pub(crate) needs: &'a [&'a [LinearIssue]],
    /// The open tickets the drafting session was shown, which are the only ones
    /// a draft's `waits_on` can name.
    pub(crate) open: &'a [LinearIssue],
}

/// What filing one slice came to: the issues created, whole rather than as
/// identifiers, because the relations a later slice asks for are written by
/// issue *id*, with one line per edge Linear turned down.
///
/// The lines are the caller's to say: an issue that exists with a missing edge
/// is something a person can fix on the board, so a refused relation is
/// reported beside the identifiers rather than taking them down with it.
#[derive(Debug)]
pub(crate) struct Cut {
    pub(crate) issues: Vec<LinearIssue>,
    pub(crate) reported: Vec<String>,
}

pub(crate) fn cut<W: Write>(
    linear: &impl Board,
    root: &Path,
    filing: Filing<'_>,
    slice: Slice<'_>,
    out: &mut W,
) -> Result<Cut, Error> {
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
        // A create that fails partway is a refusal and not a short note: a note
        // says the slice is filed, so writing one for the drafts that landed
        // would be warlock promising never to file the rest.
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

    // Printed before the note is said, not after, for the reason `push`'s URL
    // is: the issues exist from here on and their identifiers are the one thing
    // that must not be lost, so they go out whatever the comment does next.
    drop(writeln!(
        out,
        "warlock: cut `{}` into {}",
        slice.title,
        listed(&identifiers)
    ));

    linear
        .comment_on_project(filing.project, &cut_note(slice.title, &identifiers))
        .map_err(|source| Error::Uncut {
            issues: identifiers,
            source: Box::new(source),
        })?;

    Ok(Cut { issues, reported })
}

/// The project moved to `In Progress`, said as one line however it went: the
/// issues exist whatever happens here, so a workspace with no such status or a
/// move Linear turns down is reported rather than failed.
pub(crate) fn finish(linear: &impl Board, project: &str) -> String {
    let moved = linear.project_status(IN_PROGRESS).and_then(|status| {
        status
            .map(|status| linear.move_project(project, &status))
            .transpose()
    });
    match moved {
        Ok(Some(_)) => {
            format!("every slice is settled, so the project moved to `{IN_PROGRESS}`")
        }
        Ok(None) => format!(
            "every slice is settled, and the workspace has no project status called \
             `{IN_PROGRESS}`, so the project was not moved"
        ),
        Err(error) => format!(
            "every slice is settled, and the project was not moved to `{IN_PROGRESS}`: {error}"
        ),
    }
}

/// A slice somebody said no to at the review, noted on the project as a cut that
/// filed nothing, so the next run passes it over as it passes over a filed one.
/// Red's rule: a skip is a real answer and is never retried on its own;
/// retitling the slice in the brief makes it a new slice, and that is how it
/// comes back.
pub(crate) fn skip(linear: &impl Board, project: &str, title: &str) -> Result<(), Error> {
    linear
        .comment_on_project(project, &skip_note(title))
        .map(drop)
        .map_err(|source| Error::Unskipped {
            title: title.to_owned(),
            source: Box::new(source),
        })
}

// The two note shapes and their reader sit together because they are one wire
// format: a body [`noted`] cannot read back is a slice the next run files again.
// The title goes in backticks so Linear's markdown keeps it literal, and the
// identifiers too, so Linear does not turn them into issue mentions. The join
// is its own rather than [`listed`]'s, which is a display string free to change.
fn cut_note(title: &str, issues: &[String]) -> String {
    let issues: Vec<String> = issues.iter().map(|issue| format!("`{issue}`")).collect();
    format!("{CUT_NOTE}`{title}` into {}.", issues.join(", "))
}

fn skip_note(title: &str) -> String {
    format!("{SKIP_NOTE}`{title}`.")
}

/// Every slice the project's notes settle, as its folded title and the issues it
/// became — none for a skip. A body that is not one of the two shapes is left
/// out rather than refused: it is a person's comment that happens to start the
/// same way.
pub(crate) fn noted(notes: &[String]) -> Vec<(String, Vec<String>)> {
    notes
        .iter()
        .filter_map(|body| {
            let body = body.trim();
            if let Some(rest) = body.strip_prefix(CUT_NOTE) {
                let (title, issues) = rest.strip_prefix('`')?.rsplit_once("` into ")?;
                let issues: Vec<String> = issues
                    .split(',')
                    .map(|issue| issue.trim().trim_end_matches('.').trim_matches('`'))
                    .filter(|issue| is_identifier(issue))
                    .map(str::to_owned)
                    .collect();
                return (!issues.is_empty()).then(|| (fold_title(title), issues));
            }
            let title = body
                .strip_prefix(SKIP_NOTE)?
                .trim_end_matches('.')
                .strip_prefix('`')?
                .strip_suffix('`')?;
            Some((fold_title(title), Vec::new()))
        })
        .collect()
}

/// A slice title as a note is matched by: case and runs of whitespace folded, so
/// a brief edited only in its spacing or capitalisation still finds its notes.
pub(crate) fn fold_title(title: &str) -> String {
    title
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// How a settled slice is said after its heading and a dash: by the issues it
/// became, or as skipped when its note names none. See [`skip`].
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
