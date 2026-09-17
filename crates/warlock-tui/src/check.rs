//! `warlock check <path>`: which boundary a path sits inside, what this machine
//! holds, and whether the two meet — printed, and nothing written anywhere.
//!
//! Both halves of the answer are the engine's [`scope_covering`] and
//! [`scope_opens_to`], called once each and neither re-implemented here. That
//! is the point of the subcommand: the alternative for a script is walking
//! `.warlock/pacts.toml` upwards by hand, which is the boundary rule written a
//! second time somewhere it will drift from the first.
//!
//! A closed scope is an answer, not a failure: `opens` is `false` and the exit
//! status is 0, which is what makes `warlock check <path> --json | jq -e
//! '.opens'` the CI recipe, with `jq` and not warlock spending the non-zero
//! status on the verdict. So is a config that will not read — three-valued for
//! that reason, because printing `[]` would tell an operator they hold nothing
//! when the truth is warlock could not read what they hold.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;
use warlock_engine::{Manifest, scope_covering, scope_opens_to, sigils_path};
use warlock_tui::Sigils;

use crate::error::Error;
use crate::query::{envelope, spelled, write_object};
use crate::session::sigils_under;
use crate::standing::{FOR_CHECK, Standing};

const CHECK: &str = "check";

const PATH: &str = "path";

const SCOPE: &str = "scope";

const SIGILS: &str = "sigils";

const OPENS: &str = "opens";

// A value rather than four things printed as they are worked out, so the prose
// and the object are two renderings of one answer and cannot disagree about it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Checked {
    // Repository-root-relative and never the absolute path typed: an absolute
    // machine path is the one non-reproducible thing in output meant to be
    // diffable across machines.
    path: String,
    scope: Option<String>,
    sigils: Sigils,
    // Named in the prose for `Sigils::Unknown` and unused otherwise: a file
    // that would not read is only useful to a reader told which file it is.
    config: Option<PathBuf>,
    opens: bool,
}

// Nothing on disk has to exist for this to answer: coverage is a walk up the
// manifest's stored paths and never a walk of the filesystem, so a check about a
// file somebody is *about* to write answers exactly as one about a file that is
// there. `path` is joined onto the working directory, which leaves an absolute
// one as it stands, and a `..` that climbs out of the repository is refused by
// the spelling below rather than resolved into something inside it.
pub(crate) fn check(path: PathBuf, json: bool) -> Result<(), Error> {
    checked_onto(&Standing::here(FOR_CHECK)?, path, json, &mut io::stdout())
}

// Split from `check` so the order — manifest, then home, then the answer, then
// one line — is something a test can run against a temporary repository and a
// temporary home. It is where the two readings that are easy to get backwards
// live: a *missing* manifest is an empty one and answers "nothing covers this",
// while a manifest that will not *parse* is a failure; and a home that will not
// resolve is nothing held, which is a state of the answer.
fn checked_onto<W: Write>(
    standing: &Standing,
    path: PathBuf,
    json: bool,
    out: &mut W,
) -> Result<(), Error> {
    // A missing manifest is an empty one and not a failure: a repository that
    // has never pacted anything has never scoped anything either, and "nothing
    // covers this path" is the answer rather than the absence of one.
    let manifest = standing.manifest()?;
    // A home that cannot be resolved is `None` and reads as nothing held — see
    // the module docs for why it is not `Unknown`.
    let home = Standing::home().ok();

    let checked = checked(
        standing.repo_root(),
        home.as_deref(),
        &manifest,
        &standing.target(path),
    )?;

    if json {
        write_object(out, &object(&checked));
    } else {
        drop(writeln!(out, "{}", prose(&checked)));
    }
    Ok(())
}

// Every input is a parameter — the manifest in hand, the home the caller
// resolved, the path it joined — so the one thing read from disk here is the
// sigil config, under the home handed in. That is what keeps the tests off the
// developer's real home.
//
// The path is spelled *before* coverage is asked for, and both refusals are the
// same one: `spelled` and `scope_covering` agree by construction, since the
// second is the first followed by a walk. Asking here means a refused path is a
// refusal before anything is printed rather than an answer with an unprintable
// path in it.
fn checked(
    repo_root: &Path,
    home: Option<&Path>,
    manifest: &Manifest,
    target: &Path,
) -> Result<Checked, Error> {
    let path = spelled(repo_root, target)?;
    let scope = scope_covering(target, repo_root, manifest)
        .map_err(|source| Error::Unspellable { source })?
        .map(str::to_owned);
    let sigils = home.map_or(Sigils::Nothing, |home| sigils_under(home, repo_root));
    // The scope and the sigils meet in the engine and nowhere else: `Nothing`
    // and `Unknown` are both the empty slice on the way in (`Sigils::as_slice`),
    // which is what makes `opens` false for both over a scoped path and true for
    // both over an unscoped one.
    let opens = scope_opens_to(scope.as_deref(), sigils.as_slice());

    Ok(Checked {
        path,
        scope,
        sigils,
        config: home.map(|home| sigils_path(home, repo_root)),
        opens,
    })
}

// Three lines rather than a paragraph, because the three facts answer three
// questions and a reader looking for one should find it on a line of its own.
fn prose(checked: &Checked) -> String {
    format!(
        "{}\n{}\n{}",
        covering_line(&checked.path, checked.scope.as_deref()),
        holding_line(&checked.sigils, checked.config.as_deref()),
        verdict_line(checked.scope.as_deref(), checked.opens),
    )
}

// "Nothing scopes" rather than "is unscoped", because the fact is about the
// whole line of ancestors and not only the directory named: an unscoped
// directory under a scoped one is covered, and this line is the answer after
// that walk.
fn covering_line(path: &str, scope: Option<&str>) -> String {
    match scope {
        Some(scope) => format!("`{path}` is scoped `{scope}`"),
        None => format!("nothing scopes `{path}`"),
    }
}

// The wording is `warlock config`'s and the header's to the letter — one fact
// should not have two wordings. What is added here is the file: `Sigils::line`
// says only "holding unknown", and a reader running a subcommand about a config
// that will not parse is owed the path to go and fix.
fn holding_line(sigils: &Sigils, config: Option<&Path>) -> String {
    match sigils {
        Sigils::Held(held) => format!(
            "holding {}",
            held.iter()
                .map(|sigil| format!("`{sigil}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Sigils::Nothing => "holding nothing".to_owned(),
        // The `None` is unreachable today and is written out rather than
        // unwrapped: `Unknown` is a file that exists and would not read, so
        // there is always a home it was looked for under. If that ever stops
        // being true, the line still says the true half of what it knows.
        Sigils::Unknown => match config {
            Some(path) => format!("holding unknown: `{}` could not be read", path.display()),
            None => "holding unknown".to_owned(),
        },
    }
}

// The unscoped case is stated as the permissive default it comes from rather
// than as a bare "open": a reader just told nothing scopes the path should not
// be left wondering which sigil did it. The closed line ends by naming `warlock
// config`, which is the one place a sigil is recorded.
fn verdict_line(scope: Option<&str>, opens: bool) -> String {
    match scope {
        None => "an unscoped path is open to anyone, so this machine may work here".to_owned(),
        Some(scope) if opens => format!("`{scope}` is open to this machine"),
        Some(scope) => format!(
            "`{scope}` is closed to this machine — hold that sigil to work here, \
             with `warlock config`"
        ),
    }
}

// The same envelope a listing prints, with this command's body in it. No `root`
// field and no home, deliberately: an absolute machine path is not reproducible
// across machines, and a home names a person.
fn object(checked: &Checked) -> Value {
    envelope(
        CHECK,
        [
            (PATH, Value::String(checked.path.clone())),
            (
                SCOPE,
                checked.scope.clone().map_or(Value::Null, Value::String),
            ),
            (SIGILS, sigils_value(&checked.sigils)),
            (OPENS, Value::Bool(checked.opens)),
        ],
    )
}

// The three-valuedness is the whole point. `[]` for `Sigils::Unknown` would tell
// an operator they hold nothing when warlock could not read what they hold, and
// those mean opposite things about what is on disk — so the broken case is
// `null`, which a consumer has to handle deliberately rather than iterate over
// by accident. Total, so a fourth state breaks this at compile time.
fn sigils_value(sigils: &Sigils) -> Value {
    match sigils {
        Sigils::Held(held) => Value::Array(held.iter().cloned().map(Value::String).collect()),
        Sigils::Nothing => Value::Array(Vec::new()),
        Sigils::Unknown => Value::Null,
    }
}

#[cfg(test)]
#[path = "tests/check.rs"]
mod tests;
