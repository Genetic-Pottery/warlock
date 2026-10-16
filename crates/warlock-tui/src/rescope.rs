use std::fmt;

use warlock_engine::{Manifest, ScopeRecord, scope, validate_scope};

use crate::prompt::RecordField;
use crate::writing::listing;

// The values of a `[[scope]]` record exactly as a door was handed them: absent,
// or a value with nothing done to it. Whether they are required, forbidden or
// blank is a fact about what the manifest already records, so it is judged
// inside [`rescope`] and never before it.
//
// Tracker-shaped for the reason [`Tracker`](warlock_engine::Tracker) is: a flat
// `team_key` beside the other two is a Linear field every set of fields carries,
// and a door filling one in for Jira had nowhere to put a base URL. The shape
// here is what decides which [`ScopeRecord`] constructor the write goes through.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordFields<'a> {
    pub tracker: TrackerFields<'a>,
    pub review_state: Option<&'a str>,
    pub label: Option<&'a str>,
}

/// The values that address one board, as a door was handed them.
///
/// Linear by default, because that is the shape both of today's doors fill in:
/// the record window draws Linear's one field and `warlock scope add` takes
/// Linear's one flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerFields<'a> {
    Linear {
        team_key: Option<&'a str>,
    },
    Jira {
        base_url: Option<&'a str>,
        project_key: Option<&'a str>,
    },
}

impl Default for TrackerFields<'_> {
    fn default() -> Self {
        Self::Linear { team_key: None }
    }
}

impl<'a> TrackerFields<'a> {
    // In the order the tracker's own constructor takes them, which for Linear
    // is also the record window's field order.
    fn given(self) -> Vec<(RecordField, Option<&'a str>)> {
        match self {
            Self::Linear { team_key } => vec![(RecordField::TeamKey, team_key)],
            Self::Jira {
                base_url,
                project_key,
            } => vec![
                (RecordField::BaseUrl, base_url),
                (RecordField::ProjectKey, project_key),
            ],
        }
    }
}

impl<'a> RecordFields<'a> {
    // The tracker's own values first and then the two every record carries, so
    // a refusal naming several of them names them in the order a reader meets
    // them in the door they came from.
    fn given(self) -> Vec<(RecordField, Option<&'a str>)> {
        let mut given = self.tracker.given();
        given.push((RecordField::ReviewState, self.review_state));
        given.push((RecordField::Label, self.label));
        given
    }

    fn named(self, wrong: impl Fn(Option<&str>) -> bool) -> Vec<RecordField> {
        self.given()
            .into_iter()
            .filter(|(_, value)| wrong(*value))
            .map(|(field, _)| field)
            .collect()
    }

    // `None` for any field this shape needs and was not handed, which is what
    // [`rescope`] reads as the cue to refuse rather than write. The one place a
    // record is built out of these fields, so the written record's tracker is
    // the shape the door filled in and cannot be anything else.
    fn recorded(self, scope: &str) -> Option<ScopeRecord> {
        let review_state = self.review_state?;
        let label = self.label?;
        Some(match self.tracker {
            TrackerFields::Linear { team_key } => {
                ScopeRecord::new(scope, team_key?, review_state, label)
            }
            TrackerFields::Jira {
                base_url,
                project_key,
            } => ScopeRecord::jira(scope, base_url?, project_key?, review_state, label),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rescoped {
    pub(crate) manifest: Manifest,
    // The scope as written, which is the folded one.
    pub(crate) scope: Option<String>,
    pub(crate) was: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeRefusal {
    Rule {
        rule: scope::Rule,
    },
    // Every missing field rather than the first one noticed: the shell's fix is
    // to retype the command, and a refusal naming a field at a time is three
    // runs. The panel reads this as the cue to put the record window up.
    NeedsRecord {
        scope: String,
        missing: Vec<RecordField>,
    },
    // Kept apart from `NeedsRecord` even though the fix rhymes: a flag that was
    // never passed and a flag passed an empty string are different mistakes,
    // and saying "you did not pass `--team-key`" to somebody who just typed
    // `--team-key ''` sends them looking for a shell problem they do not have.
    BlankRecord {
        fields: Vec<RecordField>,
    },
    Recorded {
        scope: String,
    },
}

// The order is load-bearing, and both doors depend on it. The scope is judged
// before anything about the record, so a run with two things wrong answers about
// what was typed. And what the file already records is asked before any value
// is judged blank: a value handed to a record that exists is refused for being
// there at all, not for its spelling.
//
// Fold, then judge, and nothing else is done to what was typed: `Data-Plane`
// and `data-plane` are one boundary, while nothing is trimmed, split on a comma
// or repaired, so `control-plane, data-plane` is one refused string rather than
// two scopes somebody might have meant. `to_ascii_lowercase` rather than
// `to_lowercase`, because a scope is drawn from ASCII and folding a non-ASCII
// capital would produce a character the judge refuses anyway.
//
// `None` clears the scope, and clearing asks nothing about records.
pub(crate) fn rescope(
    manifest: &Manifest,
    module: &str,
    scope: Option<&str>,
    record: RecordFields<'_>,
) -> Result<Rescoped, ScopeRefusal> {
    let scope = match scope {
        None => None,
        Some(typed) => {
            let folded = typed.to_ascii_lowercase();
            validate_scope(&folded).map_err(|rule| ScopeRefusal::Rule { rule })?;
            Some(folded)
        }
    };

    let was = manifest
        .directory(module)
        .map(|directory| directory.scope().to_owned());

    let Some(scope) = scope else {
        return Ok(Rescoped {
            manifest: manifest.rescoped(module, None),
            scope: None,
            was,
        });
    };

    let next = if records_scope(manifest, &scope) {
        // Warlock does not rewrite, merge or delete a record, so a value
        // handed to one would be a value dropped on the floor.
        if record.given().iter().any(|(_, value)| value.is_some()) {
            return Err(ScopeRefusal::Recorded { scope });
        }
        manifest.rescoped(module, Some(&scope))
    } else {
        let Some(written) = record.recorded(&scope) else {
            return Err(ScopeRefusal::NeedsRecord {
                missing: record.named(|value| value.is_none()),
                scope,
            });
        };
        // Judged on a trimmed copy while the untrimmed string is what gets
        // stored: a team, a base URL, a project key, a review state and a label
        // belong to somebody's tracker, and warlock is in no position to
        // correct their spelling.
        let blank = record.named(|value| value.is_some_and(|value| value.trim().is_empty()));
        if !blank.is_empty() {
            return Err(ScopeRefusal::BlankRecord { fields: blank });
        }
        with_scope_recorded(manifest, module, &scope, written)
    };

    Ok(Rescoped {
        manifest: next,
        scope: Some(scope),
        was,
    })
}

pub(crate) fn records_scope(manifest: &Manifest, name: &str) -> bool {
    manifest.scope_record(name).is_some()
}

// `scope` is written in both places from the one string, so the row cannot come
// to name a record spelled differently from the one this call created, and both
// halves are one returned `Manifest` so the caller saves once: a scope on disk
// whose record failed to write is the half-state this exists to make
// impossible. The record arrives built, out of the values it was handed exactly
// as given, which is what `ScopeRecord`'s own comment requires.
fn with_scope_recorded(
    manifest: &Manifest,
    module: &str,
    scope: &str,
    record: ScopeRecord,
) -> Manifest {
    let recorded = manifest.scopes().iter().cloned().chain([record]);

    manifest.rescoped(module, Some(scope)).with_scopes(recorded)
}

const fn flag(field: RecordField) -> &'static str {
    match field {
        RecordField::TeamKey => "--team-key",
        RecordField::BaseUrl => "--base-url",
        RecordField::ProjectKey => "--project-key",
        RecordField::ReviewState => "--review-state",
        RecordField::Label => "--label",
    }
}

fn naming(fields: &[RecordField]) -> String {
    let named: Vec<String> = fields
        .iter()
        .map(|field| format!("`{}`", flag(*field)))
        .collect();
    listing(&named)
}

// The shell's wording, because the shell is the door that prints every one of
// these: the panel puts `NeedsRecord` and `BlankRecord` into the record window
// instead of saying them, and `Recorded` is worded for both doors. Both record
// refusals say "nothing was written", because a refusal about a record is the
// one place a reader might assume the scope went in and only the record did
// not.
impl fmt::Display for ScopeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The engine's sentence about the one rule that was broken, alone:
            // it is already the whole answer and the whole fix.
            Self::Rule { rule } => write!(f, "{rule}"),
            Self::NeedsRecord { scope, missing } => write!(
                f,
                "nothing records `{scope}` yet, so nothing was written: writing a scope by that \
                 name needs a team, a review state and a label, given as {}",
                naming(missing)
            ),
            Self::BlankRecord { fields } => write!(
                f,
                "{} cannot be blank, so nothing was written",
                naming(fields)
            ),
            // The fact only: what to do about it differs by door, and the
            // shell appends its flag advice in `Error`'s `Display`.
            Self::Recorded { scope } => write!(
                f,
                "`{scope}` already has a record in `.warlock/scopes.toml`, and warlock does not \
                 rewrite one"
            ),
        }
    }
}

impl std::error::Error for ScopeRefusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rule { rule } => Some(rule),
            Self::NeedsRecord { .. } | Self::BlankRecord { .. } | Self::Recorded { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "tests/rescope.rs"]
mod tests;
