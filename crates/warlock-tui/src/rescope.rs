use std::fmt;

use warlock_engine::{Manifest, ScopeRecord, TrackerName, scope, validate_scope};

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
/// Linear by default, which is a default of *nothing given* rather than a claim
/// about a board: the two callers that take it hand over no value at all — a
/// clear, which never looks at a record, and the scope window, which goes to the
/// tracker picker the moment a name turns out to need one — so the shape the
/// default happens to wear is never written and never named in a sentence.
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

// The values `warlock scope add` was handed, which is a tracker word and a flag
// for every tracker warlock knows: flat, because that is the shape of a command
// line — clap hands over each flag it has a field for, and nothing yet says
// which of them belong together.
//
// Which do, and whether the word names a tracker at all, is settled by
// [`TypedRecord::fields`] inside [`rescope`]: not here, where there is nothing
// to judge it against, and not in `main`, which would be judging it ahead of the
// boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TypedRecord<'a> {
    pub tracker: Option<&'a str>,
    pub team_key: Option<&'a str>,
    pub base_url: Option<&'a str>,
    pub project_key: Option<&'a str>,
    pub review_state: Option<&'a str>,
    pub label: Option<&'a str>,
}

/// What a door hands [`rescope`] to make a `[[scope]]` record out of.
///
/// Two shapes because the two doors know different amounts by the time they
/// call. The record window was built off the tracker picker, so it already
/// holds one tracker's fields and nothing else; `warlock scope add` holds a
/// word and every flag, and which of them are the record's is the word's
/// answer. Resolving the second into the first is [`rescope`]'s work rather
/// than either door's, because it is the third question that function asks and
/// the first two have to be answered first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Record<'a> {
    Filled(RecordFields<'a>),
    Typed(TypedRecord<'a>),
}

impl<'a> From<RecordFields<'a>> for Record<'a> {
    fn from(fields: RecordFields<'a>) -> Self {
        Self::Filled(fields)
    }
}

impl<'a> From<TypedRecord<'a>> for Record<'a> {
    fn from(typed: TypedRecord<'a>) -> Self {
        Self::Typed(typed)
    }
}

impl<'a> Record<'a> {
    // Whether this door handed over anything at all, which is what a write at a
    // name something already records is refused for. The tracker word counts
    // among the values: a board named for a record warlock will not rewrite is
    // as much a value dropped on the floor as a team key would be.
    fn anything_given(self) -> bool {
        match self {
            Self::Filled(fields) => fields.given().iter().any(|(_, value)| value.is_some()),
            Self::Typed(typed) => typed.anything_given(),
        }
    }

    fn fields(self, scope: &str) -> Result<RecordFields<'a>, ScopeRefusal> {
        match self {
            Self::Filled(fields) => Ok(fields),
            Self::Typed(typed) => typed.fields(scope),
        }
    }
}

impl<'a> TypedRecord<'a> {
    fn anything_given(self) -> bool {
        self.tracker.is_some()
            || [
                self.team_key,
                self.base_url,
                self.project_key,
                self.review_state,
                self.label,
            ]
            .iter()
            .any(Option::is_some)
    }

    // The word, then the flags that word has no use for, and only then the
    // fields themselves — every refusal here is about the command's shape, and
    // a reader has to be told the shape is wrong before they are told what is
    // missing from it. A run that handed Jira's values to `linear` is a tracker
    // typed wrong far more often than a pair of flags typed by mistake, and a
    // refusal naming what `linear` still needs would send it the wrong way.
    //
    // `TrackerName::parse` and nothing else: the words a record may spell are
    // the manifest reader's, so `linear` and `jira` cannot come to mean one set
    // of words at a shell and another in a file.
    fn fields(self, scope: &str) -> Result<RecordFields<'a>, ScopeRefusal> {
        let Some(word) = self.tracker else {
            return Err(ScopeRefusal::NeedsTracker {
                scope: scope.to_owned(),
            });
        };
        // The engine's sentence carried rather than reworded, for the reason the
        // parse is the engine's: a wording of its own here would be a second
        // list of the two words warlock knows.
        let named = TrackerName::parse(word).map_err(|fault| ScopeRefusal::UnknownTracker {
            said: fault.to_string(),
        })?;

        let (tracker, spare) = match named {
            TrackerName::Linear => (
                TrackerFields::Linear {
                    team_key: self.team_key,
                },
                vec![
                    (RecordField::BaseUrl, self.base_url),
                    (RecordField::ProjectKey, self.project_key),
                ],
            ),
            TrackerName::Jira => (
                TrackerFields::Jira {
                    base_url: self.base_url,
                    project_key: self.project_key,
                },
                vec![(RecordField::TeamKey, self.team_key)],
            ),
        };

        let spare: Vec<RecordField> = spare
            .into_iter()
            .filter(|(_, value)| value.is_some())
            .map(|(field, _)| field)
            .collect();
        if !spare.is_empty() {
            return Err(ScopeRefusal::WrongTracker {
                tracker: named,
                fields: spare,
            });
        }

        Ok(RecordFields {
            tracker,
            review_state: self.review_state,
            label: self.label,
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
    // A name nothing records, handed no tracker to record it against. Apart
    // from `NeedsRecord` because the two are answered differently: that one
    // knows which values a record of this shape is short of, and this one
    // cannot know until the word arrives, so its sentence spells the whole
    // command instead of listing what is missing from one.
    //
    // No door but the shell can raise it — the record window is built off the
    // picker's answer, so it always has a tracker.
    NeedsTracker {
        scope: String,
    },
    // A tracker word that is neither of the two, carrying the engine's sentence
    // about it rather than the word: [`TrackerName::parse`] already names the
    // value that was typed and the words a record may spell, and a second
    // wording of that here is the one that would go stale.
    UnknownTracker {
        said: String,
    },
    // Values handed to a tracker that holds none of them — `--base-url` behind
    // `linear`, `--team-key` behind `jira`. Every one of them rather than the
    // first, for `NeedsRecord`'s reason, and the tracker rides along because
    // the mistake is as often the word as the flags.
    WrongTracker {
        tracker: TrackerName,
        fields: Vec<RecordField>,
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
// what was typed. And what the file already records is asked before a tracker
// word is parsed or any value is judged blank: a value handed to a record that
// exists is refused for being there at all, not for its spelling, and a name
// nothing records is the only name a tracker has anything to say about.
//
// Fold, then judge, and nothing else is done to what was typed: `Data-Plane`
// and `data-plane` are one boundary, while nothing is trimmed, split on a comma
// or repaired, so `control-plane, data-plane` is one refused string rather than
// two scopes somebody might have meant. `to_ascii_lowercase` rather than
// `to_lowercase`, because a scope is drawn from ASCII and folding a non-ASCII
// capital would produce a character the judge refuses anyway.
//
// `None` clears the scope, and clearing asks nothing about records.
//
// The record arrives as whichever shape its door holds — an [`Into`] rather
// than two functions, so neither door has to know the other exists and the
// order above is kept in one place.
pub(crate) fn rescope<'a>(
    manifest: &Manifest,
    module: &str,
    scope: Option<&str>,
    record: impl Into<Record<'a>>,
) -> Result<Rescoped, ScopeRefusal> {
    let record = record.into();
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
        if record.anything_given() {
            return Err(ScopeRefusal::Recorded { scope });
        }
        manifest.rescoped(module, Some(&scope))
    } else {
        // Which tracker, and so which values this record is made of, settled
        // here: past the name, past what the file already holds, and before
        // anything is said about a value being missing or blank.
        let record = record.fields(&scope)?;
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

// The same values in words rather than as flags, which is the half of the
// sentence that says what is wanted: `--team-key` does not tell a reader on its
// own that the key is wanted and the team's name is not, and a record is short
// of a team key before it is short of a flag. Every one of the five takes "a".
fn wanted(fields: &[RecordField]) -> String {
    let named: Vec<String> = fields
        .iter()
        .map(|field| format!("a {}", field.name()))
        .collect();
    listing(&named)
}

// The shell's wording, because the shell is the door that prints every one of
// these: the panel puts `NeedsRecord` and `BlankRecord` into the record window
// instead of saying them, the three tracker refusals are the shell's alone, and
// `Recorded` is worded for both doors. Every refusal about a record says
// "nothing was written", because a refusal about a record is the one place a
// reader might assume the scope went in and only the record did not.
impl fmt::Display for ScopeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The engine's sentence about the one rule that was broken, alone:
            // it is already the whole answer and the whole fix.
            Self::Rule { rule } => write!(f, "{rule}"),
            // The whole command rather than a list of what is missing, which
            // is the one thing this refusal cannot work out: without the word
            // there is no saying whether this record wants a team key or a
            // base URL, and naming the tracker alone would cost a second run
            // to be told the rest.
            Self::NeedsTracker { scope } => write!(
                f,
                "nothing records `{scope}` yet and no tracker was named, so nothing was written: \
                 writing a scope by that name takes `{linear}` or `{jira}` after it — `{linear}` \
                 with {team}, `{jira}` with {site} — beside {both}",
                linear = TrackerName::Linear.word(),
                jira = TrackerName::Jira.word(),
                team = naming(&[RecordField::TeamKey]),
                site = naming(&[RecordField::BaseUrl, RecordField::ProjectKey]),
                both = naming(&[RecordField::ReviewState, RecordField::Label]),
            ),
            Self::UnknownTracker { said } => write!(f, "{said}, and nothing was written"),
            Self::WrongTracker { tracker, fields } => write!(
                f,
                "a `{word}` record has no use for {named}, so nothing was written",
                word = tracker.word(),
                named = naming(fields),
            ),
            Self::NeedsRecord { scope, missing } => write!(
                f,
                "nothing records `{scope}` yet, so nothing was written: writing a scope by that \
                 name needs {wanted}, given as {named}",
                wanted = wanted(missing),
                named = naming(missing),
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
            // None for the tracker refusals either, the unknown word included:
            // what the engine's parser handed back is this refusal's own
            // sentence, so naming it as the cause would print it twice.
            Self::NeedsTracker { .. }
            | Self::UnknownTracker { .. }
            | Self::WrongTracker { .. }
            | Self::NeedsRecord { .. }
            | Self::BlankRecord { .. }
            | Self::Recorded { .. } => None,
        }
    }
}

#[cfg(test)]
#[path = "tests/rescope.rs"]
mod tests;
