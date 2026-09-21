//! The read side of a brief: the project a push recorded for it, fetched by the
//! id that record holds and handed back only when the board says it is planned.
//!
//! The order in [`planned`] is the promise rather than an arrangement, as it is
//! in [`mod@crate::push`]: a brief no record names is refused before the seam is
//! touched at all, and a project whose status is not `Planned` is refused with
//! nothing read after the answer that said so. Nothing here writes — no
//! mutation is issued, and the status is not moved in either direction.
//!
//! No key is read on this path and none can be: the seam arrives built, so the
//! whole module is one lookup, one request and a comparison.

use std::path::Path;

use warlock_engine::filed_path;
use warlock_tui::{FetchedProject, Posts, fetch_project};

use crate::error::Error;
use crate::push::records;
use crate::standing::Standing;

// The one status a project is read back from, and the only spelling accepted:
// the comparison below trims and folds case, so `planned` and ` Planned ` are
// this and `Backlog` is not.
const PLANNED: &str = "Planned";

// Split from any caller the way `push`'s `pushed` is split from `push`: the
// environment is the standing and the path, so every refusal here runs against a
// temporary repository, and the seam is a parameter, so a refusal that must send
// nothing is asserted with a stand-in that panics when it is posted to.
//
// `records` rather than a second `Filed::load`: the file a push appends to and
// the file a pull resolves against are one file, and a second loader here would
// be a second reading of what a missing one means.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the operation lands before the door that calls it: `warlock \
                  pull` and the panel are later slices of brief 23, and this is \
                  what they will both call"
    )
)]
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
