//! `warlock check <path>`: which boundary a path sits inside, where work under
//! it is filed, what this machine holds, and whether the two meet — printed,
//! and nothing written anywhere.
//!
//! The covering scope and the route are one [`route_facts`] call in its
//! reporting form, and whether this machine opens that scope is one
//! [`scope_opens_to`] call; neither is re-implemented here. That is the point
//! of the subcommand: the alternative for a script is walking
//! `.warlock/pacts.toml` upwards by hand, which is the boundary rule and the
//! `[[scope]]` lookup written a second time somewhere they will drift from the
//! first. Only the *name* a key is stored under is ever read, so there is no
//! key value in this module to print.
//!
//! A closed scope is an answer, not a failure: `opens` is `false` and the exit
//! status is 0, which is what makes `warlock check <path> --json | jq -e
//! '.opens'` the CI recipe, with `jq` and not warlock spending the non-zero
//! status on the verdict. The same 0 covers every half-finished route — a
//! scope nobody recorded, nothing bound, a bound name the store has never heard
//! of — so `warlock check <path> --json | jq -e '.opens and .key_found'` is the
//! recipe for "may work here and can file the ticket", again on `jq`'s status
//! and not warlock's. So is a config that will not read — `sigils` is
//! three-valued for that reason, because printing `[]` would tell an operator
//! they hold nothing when the truth is warlock could not read what they hold.
//!
//! `--gate` is the one exception and the reason the rest of it is the rule: it
//! asks the same question and spends the status on the verdict itself, because a
//! shell or a `PreToolUse` hook stopping a write cannot read five lines of prose
//! and has nothing but the status to stop on. It prints nothing, it invents no
//! status — a closed scope is `Error::ClosedScope` and the **3** the headless
//! writes already refuse with — and it is the same [`checked`] answer underneath,
//! so a gate that refuses and a check that says "closed" can never disagree.
//!
//! With no path at all, `--gate` is the same question asked by a Claude Code
//! `PreToolUse` hook, which hands the path over on stdin and reads an object back
//! rather than a status: [`hook`]. That form is the one thing here that prints
//! something a person never reads, and it is still [`gated`]'s verdict — the two
//! forms differ in where the path comes from and in what a refusal is written as,
//! and in nothing else.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use warlock_engine::{Manifest, route_facts, scope_opens_to, sigils_path};
use warlock_tui::Sigils;

use crate::boundary::closed_scope_message;
use crate::error::Error;
use crate::query::{envelope, spelled, write_object};
use crate::session::sigils_under;
use crate::standing::{FOR_CHECK, Standing};

const CHECK: &str = "check";

const PATH: &str = "path";

const SCOPE: &str = "scope";

const SIGILS: &str = "sigils";

const OPENS: &str = "opens";

const TEAM: &str = "team";

// The TOML key in the `[[scope]]` record, to the letter. `state` would read
// better and would be a second spelling of one field: a consumer reading the
// manifest and the object side by side should not have to learn that they are
// the same thing.
const REVIEW_STATE: &str = "review_state";

const LABEL: &str = "label";

const KEY: &str = "key";

const KEY_FOUND: &str = "key_found";

// Claude Code's `PreToolUse` vocabulary, spelled its way and not warlock's: the
// two fields read off the payload are `snake_case` and the four written back are
// `camelCase`, because this is somebody else's schema at both ends and a field
// renamed to match the rest of this file is a hook that silently permits
// everything. Named as constants so the read and the write are the only places
// they appear and a test can be wrong about them out loud.
const TOOL_INPUT: &str = "tool_input";

const FILE_PATH: &str = "file_path";

const HOOK_OUTPUT: &str = "hookSpecificOutput";

const HOOK_EVENT_NAME: &str = "hookEventName";

const PRE_TOOL_USE: &str = "PreToolUse";

const PERMISSION_DECISION: &str = "permissionDecision";

const DENY: &str = "deny";

const PERMISSION_REASON: &str = "permissionDecisionReason";

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
    // The covering scope's `[[scope]]` record, spread flat rather than held as
    // one `Option<ScopeRecord>`: a covered path whose scope nobody recorded is
    // three `null`s here, and the object is the same shape either way. An
    // absent record is not an empty one — `""` would tell a script it was
    // filed to a team whose name is the empty string.
    team: Option<String>,
    review_state: Option<String>,
    label: Option<String>,
    // The *name* a key is stored under and never a key. Nothing in this module
    // reads a value, so there is none to leak into a `Debug`, a panic or an
    // error, and the never-print-a-secret rule costs no care below.
    key: Option<String>,
    // Two fields rather than one, because the absences are fixed in different
    // places: nothing bound is `warlock key use`, a name the store has never
    // heard of is `warlock key add`. `key_found` is false for both, so it is
    // never read on its own.
    key_found: bool,
}

// Nothing on disk has to exist for this to answer: coverage is a walk up the
// manifest's stored paths and never a walk of the filesystem, so a check about a
// file somebody is *about* to write answers exactly as one about a file that is
// there. `path` is joined onto the working directory, which leaves an absolute
// one as it stands, and a `..` that climbs out of the repository is refused by
// the spelling below rather than resolved into something inside it.
pub(crate) fn check(path: PathBuf, json: bool) -> Result<(), Error> {
    // A home that cannot be resolved is `None` and reads as nothing held — see
    // the module docs for why it is not `Unknown`.
    checked_onto(
        &Standing::here(FOR_CHECK)?,
        Standing::home().ok().as_deref(),
        path,
        json,
        &mut io::stdout(),
    )
}

// Split from `check` so the order — manifest, then the answer, then one line —
// is something a test can run against a temporary repository and a temporary
// home. It is where the reading that is easy to get backwards lives: a *missing*
// manifest is an empty one and answers "nothing covers this", while a manifest
// that will not *parse* is a failure.
//
// The home is a parameter for the working directory's reason: resolved in here,
// a test running against a scratch repository would read the sigil store of the
// machine it runs on, and `check` is the one caller no test has.
fn checked_onto<W: Write>(
    standing: &Standing,
    home: Option<&Path>,
    path: PathBuf,
    json: bool,
    out: &mut W,
) -> Result<(), Error> {
    let checked = checked_at(standing, home, path)?;

    if json {
        write_object(out, &object(&checked));
    } else {
        drop(writeln!(out, "{}", prose(&checked)));
    }
    Ok(())
}

// A missing manifest is an empty one and not a failure: a repository that has
// never pacted anything has never scoped anything either, and "nothing covers
// this path" is the answer rather than the absence of one.
fn checked_at(standing: &Standing, home: Option<&Path>, path: PathBuf) -> Result<Checked, Error> {
    let manifest = standing.manifest()?;

    checked(
        standing.repo_root(),
        home,
        &manifest,
        &standing.target(path),
    )
}

// Every input is a parameter — the manifest in hand, the home the caller
// resolved, the path it joined — so the one thing read from disk here is the
// sigil config, under the home handed in. That is what keeps the tests off the
// developer's real home.
//
// The path is spelled *before* the route is asked for, and both refusals are
// the same one: `spelled` and `route_facts` agree by construction, since the
// second is the first followed by a walk. Asking here means a refused path is
// this command's `Unspellable` refusal before anything is printed rather than a
// `Route` error with the same cause.
fn checked(
    repo_root: &Path,
    home: Option<&Path>,
    manifest: &Manifest,
    target: &Path,
) -> Result<Checked, Error> {
    let path = spelled(repo_root, target)?;
    // One engine call for the whole route, and the reporting form of it: every
    // absence `resolve_route` refuses on — an unscoped path, a scope with no
    // record, nothing bound, a name the store has never heard of — arrives here
    // as a value, which is what keeps a check's exit status 0 whatever it finds.
    // The alternative was reading `manifest.scopes()` and the sigil config for
    // the binding here, which is the `[[scope]]` lookup written a second time
    // somewhere it can disagree with the first. Its scope is `scope_covering`'s
    // answer, so there is no second coverage walk here either.
    let facts =
        route_facts(target, repo_root, manifest, home).map_err(|source| Error::Route { source })?;
    let scope = facts.scope().map(str::to_owned);
    let sigils = home.map_or(Sigils::Nothing, |home| sigils_under(home, repo_root));
    // `Nothing` and `Unknown` are both the empty slice on the way in
    // (`Sigils::as_slice`), which is what makes `opens` false for both over a
    // scoped path and true for both over an unscoped one.
    let opens = scope_opens_to(facts.scope(), sigils.as_slice());
    let record = facts.record();

    Ok(Checked {
        path,
        scope,
        sigils,
        config: home.map(|home| sigils_path(home, repo_root)),
        opens,
        team: record.map(|record| record.team().to_owned()),
        review_state: record.map(|record| record.review_state().to_owned()),
        label: record.map(|record| record.label().to_owned()),
        key: facts.key().map(str::to_owned),
        key_found: facts.stored(),
    })
}

// `warlock check --gate <PATH>`: the same answer with the verdict spent on the
// exit status instead of printed. A shell and a `PreToolUse` hook can both stop
// a write before it lands, which neither can do with five lines of prose and a
// 0, and nothing here is printed at all — the closed case is `main`'s one line
// on stderr, and the open case says nothing because there is nothing to say
// about a write that may proceed.
//
// The refusal travels through `Error::ClosedScope`, which is the register the
// headless writes already refuse in: one line, exit **3**, and re-running will
// never work. No new status and no `--force`.
pub(crate) fn gate(path: PathBuf) -> Result<(), Error> {
    gated_onto(
        &Standing::here(FOR_CHECK)?,
        Standing::home().ok().as_deref(),
        path,
    )
}

// Split from `gate` for `checked_onto`'s reason and to the same shape: the home,
// the root, the manifest and the joined path become parameters in exactly one
// place, so a test gates against a temporary repository and a temporary home
// rather than against the machine it runs on.
fn gated_onto(standing: &Standing, home: Option<&Path>, path: PathBuf) -> Result<(), Error> {
    gated(&checked_at(standing, home, path)?)
}

// The gate is `check`'s own verdict read as a decision, and deliberately not a
// second reading of the boundary: `opens` is one `scope_opens_to` call over one
// `scope_covering` walk, both inside [`checked`], so the status here and the
// line `warlock check` prints cannot come to disagree about a path. What a gate
// costs over the narrowest possible question is the key binding and the key
// store `route_facts` also reads — two small files, against a rule written once.
//
// `path` is already the repository-relative spelling `query::spelled` gave
// [`checked`], which is what the refusal wants: an absolute machine path in a
// line a hook shows somebody names their home directory back at them.
//
// Matched on the scope rather than on `opens` alone, for `boundary::permits`'s
// reason: an unscoped path is open (`scope_opens_to(None, ..)` is `true`), so a
// refusal is always in some named scope's name and this arm can never invent
// one to refuse in.
fn gated(checked: &Checked) -> Result<(), Error> {
    match (checked.scope.as_deref(), checked.opens) {
        (Some(scope), false) => Err(Error::ClosedScope {
            path: checked.path.clone(),
            scope: scope.to_owned(),
        }),
        _ => Ok(()),
    }
}

// `warlock check --gate` with no path: the same gate asked by a Claude Code
// `PreToolUse` hook, which sends the write it is about to make as a JSON payload
// on stdin and reads an object back off stdout.
//
// It is the same flag as the path form and not a second one, and the exit status
// is where that asymmetry lives: **2** is the only status Claude Code honours
// from a hook, and it means "block this tool call" for every event rather than
// "the scope is closed", so a gate that refused here with the boundary's **3**
// would be a write Claude Code waved through. The refusal therefore travels in
// the JSON and the status stays 0 either way — which leaves one flag serving a
// shell that reads statuses and a hook that reads objects off one verdict,
// rather than two flags that can come to disagree about a path.
//
// Stdin is read here and nowhere below, so everything under this takes the
// payload as bytes and a test runs the code a hook runs. It is read to the end
// rather than a line at a time, unlike `key`'s: the payload is one object and the
// hook closes the stream behind it.
pub(crate) fn hook() -> Result<(), Error> {
    let mut payload = Vec::new();
    // The first two of the four quiet exits, and all four are the same reading:
    // a hook cannot refuse a write it cannot name, so a stdin that will not read
    // and a working directory in no repository permit rather than deny. Refusing
    // on either would stop every write in a session over something that is not a
    // boundary at all, and there is nowhere for a word about it to go — stdout
    // here is a schema and stderr is a hook's log nobody is reading.
    if io::stdin().read_to_end(&mut payload).is_err() {
        return Ok(());
    }
    let Ok(standing) = Standing::here(FOR_CHECK) else {
        return Ok(());
    };

    hooked_onto(
        &standing,
        Standing::home().ok().as_deref(),
        &payload,
        &mut io::stdout(),
    )
}

// Split from `hook` for `gated_onto`'s reason and to the same shape, with the
// payload and the writer added: a test feeds bytes in and reads the object back
// out, against a temporary repository and a temporary home.
//
// The other two quiet exits are here. A payload that will not parse or carries no
// `tool_input.file_path` is a tool call this gate has nothing to say about — a
// `Bash` or a `WebFetch` reaches a `PreToolUse` hook too — and a path that will
// not spell against this repository, or a manifest that will not parse, is
// likewise a write warlock cannot name a scope for. All four permit.
fn hooked_onto<W: Write>(
    standing: &Standing,
    home: Option<&Path>,
    payload: &[u8],
    out: &mut W,
) -> Result<(), Error> {
    let Some(path) = wanted(payload) else {
        return Ok(());
    };
    // Joined and spelled by exactly the road the path form takes, so an absolute
    // `file_path` from the hook and a relative one typed at a shell reach the
    // same scope.
    let Ok(checked) = checked_at(standing, home, path) else {
        return Ok(());
    };

    match gated(&checked) {
        // An open scope, or a path no scope covers: nothing is written at all.
        // Silence is how a `PreToolUse` hook permits, and an empty stdout is
        // also what a shell wrapping this wants — there is no envelope for it
        // to have to parse before it learns nothing was refused.
        Ok(()) => Ok(()),
        Err(Error::ClosedScope { path, scope }) => {
            write_object(out, &denial(&path, &scope));
            Ok(())
        }
        // Unreachable: `gated` refuses with `ClosedScope` and with nothing else.
        // Written out rather than folded into the quiet arm above so that a
        // second refusal added there is a **1** somebody sees, and not a write
        // this hook silently permits.
        Err(error) => Err(error),
    }
}

// Defensive field access through `Value`, for the reason the stream reader takes
// the same shape: this is a schema warlock does not own and the vendor may extend
// at any time, so every step down it is an `Option` and the absence of any of
// them is `None` rather than a refusal.
//
// An empty `file_path` is one of those absences rather than a path: joined onto
// the working directory it would be the directory itself, which in a repository
// whose root carries a scope is a deny object about a write nobody can find.
fn wanted(payload: &[u8]) -> Option<PathBuf> {
    let payload: Value = serde_json::from_slice(payload).ok()?;
    let path = payload.get(TOOL_INPUT)?.get(FILE_PATH)?.as_str()?;

    (!path.is_empty()).then(|| PathBuf::from(path))
}

// Not `query::envelope`: this object is Claude Code's, nested and with no
// `command` field, so it is built here rather than bent out of warlock's own
// shape. It goes out through `write_object` all the same, which is the
// exactly-one-compact-object-on-one-line promise the `--json` answers make, and
// the one a hook's stdout parser wants even more. Field order is insertion order
// — see `preserve_order` in the workspace manifest.
fn denial(path: &str, scope: &str) -> Value {
    json!({
        HOOK_OUTPUT: {
            HOOK_EVENT_NAME: PRE_TOOL_USE,
            PERMISSION_DECISION: DENY,
            PERMISSION_REASON: reason(path, scope),
        }
    })
}

// The boundary's own sentence, and deliberately not a wording of its own: the
// same fact refused at a keystroke, at a shell prompt and at a hook says the same
// thing, names the same scope and points at the same `warlock config`. A sigil
// and the scope it opens share a name, so naming the scope is naming the sigil
// this write wants — which is why "hold that sigil" needs nothing added to it.
//
// The `warlock: ` prefix is `main`'s, carried into the JSON because the status
// line a hook shows says only that permission was denied: without it the reason
// arrives as a sentence from nowhere, and warlock is the program to go and argue
// with. The path is backticked here rather than in the message, which is the
// caller's job for `Verdict::message`'s reason — a panel spells a directory the
// way the tree does.
fn reason(path: &str, scope: &str) -> String {
    format!(
        "warlock: {}",
        closed_scope_message(&format!("`{path}`"), scope)
    )
}

// One line per fact rather than a paragraph, because a reader looking for one
// of them should find it on a line of its own, and all five are printed every
// time: an answer whose shape changes with what is missing is one a reader has
// to count lines in before they can read it, and each line below has something
// to say about an absence.
//
// The repository's two facts lead — what covers the path and where work under
// it is filed are true for anyone who clones this — then what this machine
// holds, then the verdict where those two meet, then the key this checkout is
// bound to.
//
// Every line is composed from the same `Checked` the object is rendered from,
// with no lookup of its own, which is what keeps `prose` and `object` two
// renderings of one answer rather than two answers that can disagree.
fn prose(checked: &Checked) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        covering_line(&checked.path, checked.scope.as_deref()),
        route_line(
            checked.scope.as_deref(),
            checked.team.as_deref(),
            checked.review_state.as_deref(),
            checked.label.as_deref(),
        ),
        holding_line(&checked.sigils, checked.config.as_deref()),
        verdict_line(checked.scope.as_deref(), checked.opens),
        key_line(
            checked.key.as_deref(),
            checked.key_found,
            checked.team.as_deref(),
        ),
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

// `opens` is deliberately not a parameter: a scope this machine holds no sigil
// for still files where it files, and blanking the route for a closed one would
// leave somebody covering for a colleague with a verdict and nowhere to file.
// The closed line below sits beside this one instead.
//
// The three record fields arrive as three `Option`s and are matched as one,
// because they are one `[[scope]]` record spread flat by `Checked` — a partial
// combination cannot be built, and the arms below would rather fall through to
// "no record" than print a line that names two thirds of a route.
//
// Neither absent case names a command that writes a record, because there is
// none: `warlock scope add` puts a scope *name* on a pacted directory, and the
// `[[scope]]` record it routes with is hand-written in the manifest.
fn route_line(
    scope: Option<&str>,
    team: Option<&str>,
    review_state: Option<&str>,
    label: Option<&str>,
) -> String {
    match (scope, team, review_state, label) {
        (_, Some(team), Some(review_state), Some(label)) => {
            format!(
                "work here is filed to `{team}`, labelled `{label}`, and a finished pull moves \
                 the ticket to `{review_state}`"
            )
        }
        (Some(scope), ..) => format!(
            "`{scope}` has no `[[scope]]` record, so there is nothing to route to: a record \
             in `.warlock/pacts.toml` is what would fix it"
        ),
        (None, ..) => "there is nothing to route to: a scope covering this path is what would \
             fix it, with `warlock scope add`"
            .to_owned(),
    }
}

// The wording is `warlock config`'s and the header's to the letter — one fact
// should not have two wordings. What is added here is the file: `Sigils::line`
// says only "holding unknown", and a reader running a subcommand about a config
// that will not parse is owed the path to go and fix.
fn holding_line(sigils: &Sigils, config: Option<&Path>) -> String {
    match (sigils, config) {
        (Sigils::Unknown, Some(path)) => {
            format!("holding unknown: `{}` could not be read", path.display())
        }
        _ => sigils
            .line()
            .unwrap_or_else(|| "holding nothing".to_owned()),
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

// The name a key is stored under and never a key: nothing in this module holds
// a value, so there is none for a format string to reach.
//
// The two ways of having no usable key are fixed in different files by
// different commands, so they get different lines rather than one line hedging
// between them — `warlock key use` writes the checkout's binding, `warlock key
// add` writes the machine's store, and sending somebody to the wrong one costs
// them a read of both.
//
// A bound name that resolves is said together with the team it would file to,
// because that pairing is the question being asked: "where does work here go,
// and can this checkout file it". Naming the key alone would answer half of it
// on a line that looks like the whole answer. With no record there is no team
// to pair it with, and the line says only what is true.
fn key_line(key: Option<&str>, found: bool, team: Option<&str>) -> String {
    match (key, found) {
        (None, _) => "no key is bound to this checkout: `warlock key use <name>` binds a name \
             this machine stores, and `warlock key add <name>` stores a new one"
            .to_owned(),
        (Some(key), false) => format!(
            "the key `{key}` is bound here and this machine has not stored it: \
             `warlock key add {key}` stores it, `warlock key use <name>` binds another"
        ),
        (Some(key), true) => match team {
            Some(team) => {
                format!("filing to `{team}` would use the key `{key}`, which this machine stores")
            }
            None => format!("the key `{key}` is bound here and stored on this machine"),
        },
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
            (SCOPE, text(checked.scope.as_deref())),
            (SIGILS, sigils_value(&checked.sigils)),
            (OPENS, Value::Bool(checked.opens)),
            (TEAM, text(checked.team.as_deref())),
            (REVIEW_STATE, text(checked.review_state.as_deref())),
            (LABEL, text(checked.label.as_deref())),
            (KEY, text(checked.key.as_deref())),
            (KEY_FOUND, Value::Bool(checked.key_found)),
        ],
    )
}

// Flat beside the four fields that were here first, and not a nested `route`
// object: a consumer asking `.opens and .key_found` should not have to know
// which half of the answer a field was added with, and nesting would make the
// unrecorded case a choice between a `null` object and an object of `null`s.
fn text(value: Option<&str>) -> Value {
    value.map_or(Value::Null, |value| Value::String(value.to_owned()))
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
