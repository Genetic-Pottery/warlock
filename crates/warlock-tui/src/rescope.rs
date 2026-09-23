use std::fmt;

use warlock_engine::{Manifest, ScopeRecord, scope, validate_scope};
use warlock_tui::RecordField;

// The three values of a `[[scope]]` record exactly as a door was handed them:
// absent, or a value with nothing done to it. Whether they are required,
// forbidden or blank is a fact about what the manifest already records, so it
// is judged inside [`rescope`] and never before it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RecordFields<'a> {
    pub(crate) team: Option<&'a str>,
    pub(crate) review_state: Option<&'a str>,
    pub(crate) label: Option<&'a str>,
}

impl<'a> RecordFields<'a> {
    // In the record window's field order, so a refusal naming several of them
    // names them in the order a reader meets them in either door.
    const fn given(self) -> [(RecordField, Option<&'a str>); 3] {
        [
            (RecordField::Team, self.team),
            (RecordField::ReviewState, self.review_state),
            (RecordField::Label, self.label),
        ]
    }

    fn named(self, wrong: impl Fn(Option<&str>) -> bool) -> Vec<RecordField> {
        self.given()
            .into_iter()
            .filter(|(_, value)| wrong(*value))
            .map(|(field, _)| field)
            .collect()
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
pub(crate) enum ScopeRefusal {
    Rule {
        rule: scope::Rule,
    },
    NoPact {
        module: String,
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
    // and saying "you did not pass `--team`" to somebody who just typed
    // `--team ''` sends them looking for a shell problem they do not have.
    BlankRecord {
        fields: Vec<RecordField>,
    },
    Recorded {
        scope: String,
    },
}

// The order is load-bearing, and both doors depend on it. The scope is judged
// before the entry is looked for, so a run with two things wrong answers about
// what was typed. The pact comes next and above everything about the record,
// because three values for a directory nobody has pacted would never have been
// written whatever they said. And what the file already records is asked before
// any value is judged blank: a value handed to a record that exists is refused
// for being there at all, not for its spelling.
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
        .entry(module)
        .ok_or_else(|| ScopeRefusal::NoPact {
            module: module.to_owned(),
        })?
        .scope()
        .map(str::to_owned);

    let Some(scope) = scope else {
        return Ok(Rescoped {
            manifest: with_scope_on(manifest, module, None),
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
        with_scope_on(manifest, module, Some(&scope))
    } else {
        let (Some(team), Some(review_state), Some(label)) =
            (record.team, record.review_state, record.label)
        else {
            return Err(ScopeRefusal::NeedsRecord {
                missing: record.named(|value| value.is_none()),
                scope,
            });
        };
        // Judged on a trimmed copy while the untrimmed string is what gets
        // stored: a team, a review state and a label belong to somebody's
        // tracker, and warlock is in no position to correct their spelling.
        let blank = record.named(|value| value.is_some_and(|value| value.trim().is_empty()));
        if !blank.is_empty() {
            return Err(ScopeRefusal::BlankRecord { fields: blank });
        }
        with_scope_recorded(manifest, module, &scope, team, review_state, label)
    };

    Ok(Rescoped {
        manifest: next,
        scope: Some(scope),
        was,
    })
}

// The same comparison [`route_facts`](warlock_engine::route_facts) routes by,
// and it has to stay that way: a lookup that folded, trimmed or matched loosely
// here would answer "no record" for a name `warlock check` then routes through,
// and a second record would be written that the router never reads.
pub(crate) fn records_scope(manifest: &Manifest, name: &str) -> bool {
    manifest.scopes().iter().any(|record| record.name() == name)
}

// A rebuild rather than a mutation, because [`Manifest`] has no mutating scope
// setter and should not grow one for this. [`Manifest::rebuilt_with`] and not
// `Manifest::with_entries`: that one starts from an empty manifest and would
// drop the `[[scope]]` records out of the file. Every other entry is cloned as
// it stands and the map preserves order, so the saved file differs from the one
// on disk in one place; the edited entry keeps its document, granted hash and
// granted timestamp, none of which are this edit's to move.
fn with_scope_on(manifest: &Manifest, module: &str, scope: Option<&str>) -> Manifest {
    manifest.rebuilt_with(manifest.entries().iter().map(|entry| {
        let entry = entry.clone();
        if entry.module() != module {
            return entry;
        }
        match scope {
            Some(scope) => entry.with_scope(scope),
            None => entry.without_scope(),
        }
    }))
}

// `scope` is written in both places from the one string, so the pact cannot come
// to name a record spelled differently from the one this call created, and both
// halves are one returned `Manifest` so the caller saves once: a scope on disk
// whose record failed to write is the half-state this exists to make
// impossible. The three record values go to `ScopeRecord::new` exactly as
// given, which is what its own comment requires.
fn with_scope_recorded(
    manifest: &Manifest,
    module: &str,
    scope: &str,
    team: &str,
    review_state: &str,
    label: &str,
) -> Manifest {
    let recorded = manifest.scopes().iter().cloned().chain([ScopeRecord::new(
        scope,
        team,
        review_state,
        label,
    )]);

    with_scope_on(manifest, module, Some(scope)).with_scopes(recorded)
}

const fn flag(field: RecordField) -> &'static str {
    match field {
        RecordField::Team => "--team",
        RecordField::ReviewState => "--review-state",
        RecordField::Label => "--label",
    }
}

// `` `--a`, `--b` and `--c` ``, in [`writing::missing_line`](crate::writing)'s
// shape: a refusal that names more than one thing is read as a sentence, and a
// comma before the last of them would be read as a fourth flag.
fn naming(fields: &[RecordField]) -> String {
    let named: Vec<String> = fields
        .iter()
        .map(|field| format!("`{}`", flag(*field)))
        .collect();
    let Some((last, rest)) = named.split_last() else {
        return String::new();
    };
    if rest.is_empty() {
        last.clone()
    } else {
        format!("{} and {last}", rest.join(", "))
    }
}

// The shell's wording, because the shell is the door that prints every one of
// these: the panel puts `NeedsRecord` and `BlankRecord` into the record window
// instead of saying them. `Recorded` is the exception, worded for both doors. "nothing was written" ends the three record
// sentences, because a refusal about a record is the one place a reader might
// assume the scope went in and only the record did not.
impl fmt::Display for ScopeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The engine's sentence about the one rule that was broken, alone:
            // it is already the whole answer and the whole fix.
            Self::Rule { rule } => write!(f, "{rule}"),
            Self::NoPact { module } => write!(
                f,
                "`{module}` is not in the manifest, so there is no pact to carry a \
                 scope; pact it in warlock first, with `p`"
            ),
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
                "`{scope}` already has a record in `.warlock/pacts.toml`, and warlock does not \
                 rewrite one"
            ),
        }
    }
}

impl std::error::Error for ScopeRefusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Rule { rule } => Some(rule),
            Self::NoPact { .. }
            | Self::NeedsRecord { .. }
            | Self::BlankRecord { .. }
            | Self::Recorded { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "tests/rescope.rs"]
mod tests;
