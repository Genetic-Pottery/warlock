//! `warlock check <path>`: which boundary a path sits inside, what this machine
//! holds, and whether the two meet — on the ordinary screen, with nothing
//! written anywhere.
//!
//! The fifth subcommand, in the shape the four before it gave: dispatched
//! before anything touches the terminal, no alternate screen, no raw mode, no
//! panic hook, and a failure is an [`Error`] returned to `main`, which prints it
//! in the same place and the same shape as a tree that would not load. Nothing
//! here writes a file, spawns a process or runs a model pass — this is a
//! question, and the whole of what it does is read a manifest, read a config and
//! print the answer.
//!
//! # Nothing here decides what a boundary means
//!
//! The two halves of the answer are the engine's own
//! [`scope_covering`] and [`scope_opens_to`], called once each and neither
//! re-implemented: nearest-scope-wins, an invalid scope read as no scope, an
//! unscoped path open to anyone, and holding nothing opening nothing that is
//! scoped are all decided in `crates/warlock-engine/src/scope.rs` and nowhere
//! else. That is the point of the subcommand existing: the alternative for a
//! script is walking `.warlock/pacts.toml` upwards by hand, which is the
//! boundary rule written a second time somewhere it will drift from the first.
//!
//! It is the same pair the TUI's run keys are refused by
//! ([`closed_scope`](crate::session::closed_scope)), asked here with nothing on
//! screen — and this asks only. A check reports; it refuses nothing, because
//! there is nothing here to refuse.
//!
//! # What this machine holds is resolved once, in the header's own words
//!
//! [`sigils_under`] is the resolution, borrowed from the session rather than
//! copied: the same three states the header states, from the same file, with the
//! same reading of a missing one. `--json` keeps all three apart, and that is
//! the field's whole reason for being three-valued — a config that exists and
//! will not parse printed as `[]` would tell an operator they hold nothing when
//! the truth is that warlock could not read what they hold.
//!
//! A home directory that cannot be resolved is [`Sigils::Nothing`] and not
//! [`Sigils::Unknown`], which is [`sigils_held`](crate::session) reading of it
//! and is the honest one: `Unknown` says *a file is there and would not read*,
//! and the prose for it names that file. With no home there is no file and no
//! path to name one by, so there is nothing broken to report — only a machine
//! nobody has configured, which is what `Nothing` says.
//!
//! # What is refused, and what is merely closed
//!
//! A closed scope is an answer: the prose says so, `opens` is `false`, and the
//! exit status is 0 — which is what makes `warlock check <path> --json | jq -e
//! '.opens'` the CI recipe, with `jq` and not warlock spending the non-zero
//! status on the verdict. `Nothing` and `Unknown` are answers too, and both
//! close every scoped path without failing.
//!
//! A path [`scope_covering`] refuses — outside the repository root, or not
//! spellable as UTF-8 — is the one refusal here: one line on stderr, exit 1, and
//! no object printed at all. It is never reported as `scope: null`, per that
//! function's own doc: such a path is not unscoped, it is a path this manifest
//! has nothing whatever to say about.

use std::env;
use std::path::{Path, PathBuf};

use serde_json::Value;
use warlock_engine::{Manifest, repository_root, scope_covering, scope_opens_to, sigils_path};
use warlock_tui::Sigils;

use crate::config::home_directory;
use crate::error::Error;
use crate::query::{envelope, print_object, spelled};
use crate::session::{load_manifest, sigils_under};

/// What a check wants a repository root for, as the tail of
/// [`Error::NoRepository`]'s sentence. The other subcommands' tails are spelled
/// beside them, each where its subcommand is written.
const FOR_CHECK: &str = "answer about the boundary over";

/// The word this subcommand names itself with in its object, and the word a
/// reader typed to get it.
const CHECK: &str = "check";

/// The path the check was asked about, as the manifest spells it.
const PATH: &str = "path";

/// The scope covering that path, or `null` when nothing covers it.
const SCOPE: &str = "scope";

/// What this machine holds: the list, `[]` for nothing, `null` for a config
/// that would not read.
const SIGILS: &str = "sigils";

/// Whether those sigils open that scope.
const OPENS: &str = "opens";

/// The whole answer to one check: what was asked about, what covers it, what is
/// held, and whether the two meet.
///
/// A value rather than four things printed as they are worked out, so that the
/// prose and the object are two renderings of one answer and cannot disagree
/// about it — and so the answer itself is testable with nothing attached to
/// stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Checked {
    /// The path asked about, repository-root-relative with forward slashes, and
    /// `.` for the repository root itself. Never the absolute path typed: an
    /// absolute machine path is the one non-reproducible thing in a document
    /// meant to be diffable across machines.
    path: String,
    /// The scope covering it, or `None` when nothing at or above it carries
    /// one. Owned rather than borrowed from the manifest, because what is
    /// printed outlives the read.
    scope: Option<String>,
    /// What this machine holds, in the header's own three states.
    sigils: Sigils,
    /// The config those sigils were read from, or `None` when there is no home
    /// directory to look under. Named in the prose for [`Sigils::Unknown`] and
    /// unused otherwise: a file that would not read is only useful to a reader
    /// who is told which file it is.
    config: Option<PathBuf>,
    /// Whether the sigils open the scope: [`scope_opens_to`]'s answer, and
    /// `false` for both `Nothing` and `Unknown` over a scoped path.
    opens: bool,
}

/// `warlock check <path>`: say which scope covers `path`, what this machine
/// holds, and whether those sigils open it.
///
/// The steps are the module doc's, in this order: the working directory says
/// where the repository is, the repository root is resolved from it, the
/// manifest and the sigil config are read, and the answer is printed as prose or
/// as one object.
///
/// `path` is taken relative to the working directory, as a person typing one at
/// a shell means it, and an absolute one is used as it stands ([`Path::join`]
/// does both). It is not made absolute or normalised beyond that: a `..` that
/// climbs out of the repository is refused by the spelling below rather than
/// being resolved into something inside it.
///
/// Nothing on disk has to exist for this to answer. Coverage is a walk up the
/// manifest's stored paths and never a walk of the filesystem, so a check about
/// a file somebody is *about* to write answers exactly as one about a file that
/// is there.
///
/// # Errors
///
/// [`Error::WorkingDirectory`] and [`Error::NoRepository`] before anything is
/// read; [`Error::Manifest`] for a manifest that will not parse; and
/// [`Error::Unspellable`] for a path with no repository-relative form. Every one
/// of them is one line on stderr and an exit status of 1, and none of them
/// prints a partial answer first. A config that will not read is deliberately
/// not among them: it is a state of the answer, not a failure to reach one.
pub(crate) fn check(path: PathBuf, json: bool) -> Result<(), Error> {
    let working_dir = env::current_dir().map_err(|source| Error::WorkingDirectory { source })?;
    // Asked directly rather than through a load, as `init` and `config` ask:
    // this reads a manifest and a config, and walking the tree would be reading
    // every directory in the repository to answer a question about ancestors.
    let repo_root = repository_root(&working_dir).ok_or(Error::NoRepository {
        start: working_dir.clone(),
        wanted: FOR_CHECK,
    })?;
    // A missing manifest is an empty one and not a failure: a repository that
    // has never pacted anything has never scoped anything either, and "nothing
    // covers this path" is the answer rather than the absence of one.
    let manifest = load_manifest(&repo_root)?;
    // The one place the environment becomes a home path, and the reason it is
    // resolved here: everything below takes it as a parameter, so the tests run
    // against a temporary home rather than the developer's own. A home that
    // cannot be resolved is `None` and reads as nothing held — see the module
    // docs for why it is not `Unknown`.
    let home = home_directory().ok();

    let checked = checked(
        &repo_root,
        home.as_deref(),
        &manifest,
        &working_dir.join(path),
    )?;

    if json {
        print_object(&object(&checked));
    } else {
        println!("{}", prose(&checked));
    }
    Ok(())
}

/// The whole answer about `target`, for the repository at `repo_root`, with
/// this machine's sigils read from under `home`.
///
/// Every input is a parameter — the manifest already in hand, the home the
/// caller resolved, the path the caller joined — so the one thing this reads
/// from disk is the sigil config, under the home it was handed. That is what
/// keeps the tests off the developer's real home and out of a real repository.
///
/// The path is spelled *before* coverage is asked for, and both refusals are the
/// same one: [`spelled`] and [`scope_covering`] agree by construction, since the
/// second is the first followed by a walk. Asking here means a refused path is a
/// refusal before anything is printed, rather than an answer with an
/// unprintable path in it.
///
/// # Errors
///
/// [`Error::Unspellable`] for a `target` outside `repo_root` or not spellable as
/// UTF-8.
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

/// The answer as prose: the boundary, the holding, and the verdict, one line
/// each.
///
/// Three lines rather than a paragraph, because the three facts are answers to
/// three questions and a reader looking for one of them should find it on a line
/// of its own. Each is a sentence in warlock's own vocabulary: the wording of
/// what is held is `warlock config`'s and the header's, to the letter, and the
/// closed line ends the way the TUI's refusal ends — naming `warlock config`,
/// which is the one place a sigil is recorded and the only road from this line
/// to the work.
fn prose(checked: &Checked) -> String {
    format!(
        "{}\n{}\n{}",
        covering_line(&checked.path, checked.scope.as_deref()),
        holding_line(&checked.sigils, checked.config.as_deref()),
        verdict_line(checked.scope.as_deref(), checked.opens),
    )
}

/// What covers `path`, or that nothing does.
///
/// "Nothing scopes" rather than "is unscoped", because the fact is about the
/// whole line of ancestors and not only about the directory named: an
/// unscoped directory under a scoped one is covered, and this line is the
/// answer after that walk.
fn covering_line(path: &str, scope: Option<&str>) -> String {
    match scope {
        Some(scope) => format!("`{path}` is scoped `{scope}`"),
        None => format!("nothing scopes `{path}`"),
    }
}

/// What this machine holds, in the words `warlock config` and the header use.
///
/// The same "holding", the same backticked sigils in the order the config lists
/// them, and the same "nothing" and "unknown" — one fact should not have two
/// wordings. What is added here is the file: `Sigils::line` says only "holding
/// unknown", and a reader running a subcommand about a config that will not
/// parse is owed the path to go and fix.
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

/// Whether the holding opens the boundary, said as the consequence it is.
///
/// The unscoped case is stated as the permissive default it comes from rather
/// than as a bare "open": a reader who has just been told nothing scopes the
/// path should be told that this is what makes it open, not left to wonder which
/// sigil did it.
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

/// The object `--json` prints for a check: the command it answers, the path it
/// answers about, and the three facts of the answer.
///
/// The same envelope a listing prints, with this command's own body in it, and
/// in this order: `command`, `path`, `scope`, `sigils`, `opens`. There is no
/// `root` field, deliberately — an absolute machine path is the one
/// non-reproducible thing in a document meant to be diffable across machines,
/// and it tells a consumer a constant.
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

/// What is held, as JSON's three answers: the list, `[]`, and `null`.
///
/// The three-valuedness is the whole point of this function. `[]` for
/// [`Sigils::Unknown`] would tell an operator they hold nothing when the truth
/// is that warlock could not read what they hold, and those two mean opposite
/// things about what is on disk — so the broken case is `null`, which a consumer
/// has to handle deliberately rather than iterate over by accident.
///
/// A total match, so a fourth state on the header breaks this at compile time
/// rather than being printed as whatever a fallback arm picked.
fn sigils_value(sigils: &Sigils) -> Value {
    match sigils {
        Sigils::Held(held) => Value::Array(held.iter().cloned().map(Value::String).collect()),
        Sigils::Nothing => Value::Array(Vec::new()),
        Sigils::Unknown => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use warlock_engine::{Manifest, PactEntry, save_sigils, sigils_path};
    use warlock_tui::Sigils;

    use super::{Checked, checked, object, prose};
    use crate::error::Error;

    /// The repository every check here is asked about. A path rather than a
    /// directory on disk, deliberately: coverage is a walk up the manifest's
    /// stored paths and never a walk of the filesystem, so nothing here has to
    /// exist for the answer to be the answer.
    const REPO: &str = "/repo";

    /// A throwaway directory. Every test that reads or writes a config builds
    /// its home out of one of these, so nothing here goes near the developer's
    /// real home.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// An entry for `module`, documented the way a pact would document it.
    fn entry(module: &str) -> PactEntry {
        PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root")
    }

    /// A repository with a scope on `crates`, a nearer one on `crates/engine`,
    /// and a pacted-but-unscoped `docs`.
    fn a_manifest() -> Manifest {
        Manifest::with_entries([
            entry("crates").with_scope("platform"),
            entry("crates/engine").with_scope("data-plane"),
            entry("docs"),
        ])
    }

    /// The answer about `path` in [`a_manifest`], with this machine's sigils
    /// read from under `home`.
    fn answer(home: &Path, path: &str) -> Checked {
        checked(
            Path::new(REPO),
            Some(home),
            &a_manifest(),
            &Path::new(REPO).join(path),
        )
        .expect("a path inside the repository has a manifest form")
    }

    /// Write `sigils` as what the machine holds for [`REPO`] under `home`.
    fn holding(home: &Path, sigils: &[&str]) {
        let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
        save_sigils(home, Path::new(REPO), &sigils).expect("a config that writes");
    }

    /// Put a file that is not a config where this machine's sigils would be.
    fn a_broken_config(home: &Path) {
        let path = sigils_path(home, Path::new(REPO));
        fs::create_dir_all(
            path.parent()
                .expect("the config lives in a project directory"),
        )
        .expect("the project directory");
        fs::write(&path, "not a config\n").expect("a file that is not TOML");
    }

    #[test]
    fn a_covered_path_is_named_by_the_nearest_scope_above_it() {
        // The engine's nearest-wins walk, read back out here: a file deep under
        // `crates/engine` answers with the inner scope, and a sibling that has
        // said nothing answers with the outer one.
        let home = a_dir();

        assert_eq!(
            answer(home.path(), "crates/engine/src/lib.rs")
                .scope
                .as_deref(),
            Some("data-plane")
        );
        assert_eq!(
            answer(home.path(), "crates/engine").scope.as_deref(),
            Some("data-plane")
        );
        assert_eq!(
            answer(home.path(), "crates/tui/src").scope.as_deref(),
            Some("platform")
        );
    }

    #[test]
    fn a_path_nothing_covers_says_so_rather_than_naming_a_scope() {
        let home = a_dir();

        // Pacted and unscoped is not covered, and neither is the root itself.
        assert_eq!(answer(home.path(), "docs/adr").scope, None);
        assert_eq!(answer(home.path(), ".").scope, None);
        // And an unscoped path is open to a machine holding nothing at all:
        // the permissive default is on the directory and only there.
        assert!(answer(home.path(), "docs/adr").opens);
        assert_eq!(
            prose(&answer(home.path(), "docs/adr")),
            "nothing scopes `docs/adr`\n\
             holding nothing\n\
             an unscoped path is open to anyone, so this machine may work here"
        );
    }

    #[test]
    fn a_held_sigil_opens_the_scope_it_matches_and_nothing_else() {
        let home = a_dir();
        holding(home.path(), &["platform"]);

        let open = answer(home.path(), "crates/tui/src");
        assert_eq!(open.sigils, Sigils::Held(vec!["platform".to_owned()]));
        assert!(open.opens);
        assert_eq!(
            prose(&open),
            "`crates/tui/src` is scoped `platform`\n\
             holding `platform`\n\
             `platform` is open to this machine"
        );

        // The nearer scope replaces the outer one outright, so holding
        // `platform` does not open what `data-plane` covers.
        let closed = answer(home.path(), "crates/engine");
        assert!(!closed.opens);
        assert_eq!(
            prose(&closed),
            "`crates/engine` is scoped `data-plane`\n\
             holding `platform`\n\
             `data-plane` is closed to this machine — hold that sigil to work \
             here, with `warlock config`"
        );
    }

    #[test]
    fn a_machine_that_holds_nothing_is_closed_by_every_scope_and_still_answers() {
        // No config at all: the ordinary state of a machine nobody has run
        // `warlock config` on. It is an answer, not a failure.
        let home = a_dir();
        let closed = answer(home.path(), "crates/engine");

        assert_eq!(closed.sigils, Sigils::Nothing);
        assert!(!closed.opens);
        assert_eq!(
            object(&closed).to_string(),
            r#"{"command":"check","path":"crates/engine","scope":"data-plane","sigils":[],"opens":false}"#
        );
    }

    #[test]
    fn a_config_that_will_not_parse_is_unknown_and_never_an_empty_holding() {
        // The case `[]` would lie about: an operator told they hold nothing when
        // the truth is that warlock could not read what they hold.
        let home = a_dir();
        a_broken_config(home.path());
        let unknown = answer(home.path(), "crates/engine");

        assert_eq!(unknown.sigils, Sigils::Unknown);
        assert!(
            !unknown.opens,
            "a holding warlock cannot read opens nothing"
        );
        assert_eq!(
            object(&unknown).to_string(),
            r#"{"command":"check","path":"crates/engine","scope":"data-plane","sigils":null,"opens":false}"#
        );
        // And the prose names the file, which is the one thing a reader needs
        // in order to fix it.
        let path = sigils_path(home.path(), Path::new(REPO));
        assert!(
            prose(&unknown).contains(&format!("`{}` could not be read", path.display())),
            "the unreadable file is not named: {}",
            prose(&unknown)
        );
    }

    #[test]
    fn a_missing_home_is_nothing_held_rather_than_a_config_that_would_not_read() {
        // `Unknown` claims a file on disk is broken. With no home there is no
        // file and no path to name one by, so there is nothing broken to report.
        let nowhere = checked(
            Path::new(REPO),
            None,
            &a_manifest(),
            &Path::new(REPO).join("crates/engine"),
        )
        .expect("a path inside the repository has a manifest form");

        assert_eq!(nowhere.sigils, Sigils::Nothing);
        assert_eq!(nowhere.config, None);
        assert!(!nowhere.opens);
        assert!(prose(&nowhere).contains("holding nothing"));
    }

    #[test]
    fn a_path_outside_the_repository_is_refused_rather_than_reported_as_unscoped() {
        // `scope_covering`'s own doc: such a path is not unscoped, it is one
        // this manifest has nothing whatever to say about. Answering
        // `scope: null` would tell a script it is open to anyone.
        let home = a_dir();

        for outside in [
            Path::new("/elsewhere").to_path_buf(),
            Path::new(REPO).join("..").join("elsewhere"),
        ] {
            let error = checked(Path::new(REPO), Some(home.path()), &a_manifest(), &outside)
                .expect_err("a path outside the repository has no manifest form");

            assert!(
                matches!(error, Error::Unspellable { .. }),
                "the engine's own case was rewrapped: {error:?}"
            );
            assert!(!error.to_string().contains('\n'), "`main` prints one line");
        }
    }

    #[test]
    fn the_object_is_the_shape_a_consumer_was_promised() {
        let home = a_dir();
        holding(home.path(), &["billing"]);

        assert_eq!(
            object(&answer(home.path(), "crates/engine")).to_string(),
            r#"{"command":"check","path":"crates/engine","scope":"data-plane","sigils":["billing"],"opens":false}"#
        );
    }

    #[test]
    fn nothing_covering_the_path_is_a_null_scope_rather_than_a_missing_field() {
        let home = a_dir();

        assert_eq!(
            object(&answer(home.path(), "docs/adr")).to_string(),
            r#"{"command":"check","path":"docs/adr","scope":null,"sigils":[],"opens":true}"#
        );
    }

    #[test]
    fn no_object_carries_a_root_or_a_home() {
        // Deliberate: an absolute machine path is the one non-reproducible thing
        // in a document meant to be diffable across machines. The home is worse
        // — it names a person.
        let home = a_dir();
        holding(home.path(), &["billing"]);
        let printed = object(&answer(home.path(), "crates/engine")).to_string();

        assert!(!printed.contains("root"), "{printed}");
        assert!(!printed.contains(REPO), "{printed}");
        assert!(
            !printed.contains(&home.path().display().to_string()),
            "{printed}"
        );

        // Including the one answer that has a file to name: the prose says
        // which file, and the object still does not.
        let broken = a_dir();
        a_broken_config(broken.path());
        let printed = object(&answer(broken.path(), "crates/engine")).to_string();
        assert!(
            !printed.contains(&broken.path().display().to_string()),
            "{printed}"
        );
    }
}
