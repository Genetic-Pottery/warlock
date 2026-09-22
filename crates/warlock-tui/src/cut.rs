//! One scope slice's validated drafts become issues on the board, and what was
//! created becomes a cut record beside the brief's own.
//!
//! The order in [`cut`] is the promise rather than an arrangement, as it is in
//! [`mod@crate::push`]: a slice the record already names sends nothing at all,
//! and a team with no `Backlog` state is refused while the slice is still
//! nothing rather than half filed. The record is saved for this slice as soon
//! as its issues exist and not once at the end, because an issue nothing
//! records is exactly what the next run files a second time.
//!
//! Its own module rather than [`mod@crate::pull`]'s, which writes nothing.
//! No key is read here and none can be: the seam arrives built, as it does for
//! a pull.

use std::io::Write;
use std::path::Path;

use warlock_engine::drafting::Draft;
use warlock_engine::{CutRecord, fold_title, manifest_path, now_rfc3339};
use warlock_tui::{
    LinearIssue, NewIssue, Posts, backlog_state, create_issue, issue_label_id, team_id,
};

use crate::error::Error;
use crate::push::records;

/// Where one slice's issues go, which is what [`resolve_filing`] and the
/// brief's own record between them answered: the two names off the
/// `[[scope]]` record, the project a push made, and the brief as
/// `.warlock/filed.toml` spells it.
///
/// [`resolve_filing`]: warlock_engine::resolve_filing
#[derive(Debug, Clone, Copy)]
pub(crate) struct Filing<'a> {
    pub(crate) brief: &'a str,
    pub(crate) project: &'a str,
    pub(crate) team: &'a str,
    pub(crate) label: &'a str,
}

/// One slice of the project's scope, with the drafts a session settled for it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Slice<'a> {
    pub(crate) title: &'a str,
    pub(crate) drafts: &'a [Draft],
}

/// What filing one slice came to.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "what `cut` answers, and read by the caller that lands in a \
                  later slice of brief 23"
    )
)]
#[derive(Debug)]
pub(crate) enum Cut {
    /// The record already names this slice, so nothing was sent: the
    /// identifiers are the ones that record holds, which is all a cut record
    /// keeps of an issue.
    Already(Vec<String>),
    /// The issues created now, whole rather than as identifiers, because the
    /// relations a slice's drafts ask for are written by issue *id*.
    Filed(Vec<LinearIssue>),
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the operation lands before the door that calls it: `warlock \
                  pull` and the panel are later slices of brief 23, and this is \
                  what they will both call once a slice has been drafted"
    )
)]
pub(crate) fn cut<W: Write>(
    linear: &impl Posts,
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
        drop(writeln!(
            out,
            "warlock: `{}` is already cut as {}, so nothing was sent",
            slice.title,
            listed(&issues)
        ));
        return Ok(Cut::Already(issues));
    }

    let team = team_id(linear, filing.team)
        .map_err(|source| Error::Linear { source })?
        .ok_or_else(|| Error::UnknownTeam {
            team: filing.team.to_owned(),
            path: manifest_path(root),
        })?;
    // Before the label and before every create: a team with nowhere to put an
    // issue is a refusal that costs nothing, and the same question asked after
    // the first create would leave a slice half filed on the board.
    let state = backlog_state(linear, &team)
        .map_err(|source| Error::Linear { source })?
        .ok_or_else(|| Error::NoBacklog {
            team: filing.team.to_owned(),
        })?;
    let label =
        issue_label_id(linear, filing.label, &team).map_err(|source| Error::Linear { source })?;

    let mut issues = Vec::with_capacity(slice.drafts.len());
    for draft in slice.drafts {
        // A create that fails partway is a refusal and not a short cut record:
        // a record says the slice is filed, so writing one for the drafts that
        // landed would be warlock promising never to file the rest.
        let issue = create_issue(
            linear,
            &NewIssue::new(
                &draft.title,
                &draft.body,
                &team,
                filing.project,
                &label,
                &state,
            ),
        )
        .map_err(|source| Error::Linear { source })?;
        issues.push(issue);
    }

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

    Ok(Cut::Filed(issues))
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
