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
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use warlock_engine::{Manifest, PactEntry, save_sigils, sigils_path};
    use warlock_tui::Sigils;

    use super::{Checked, checked, checked_onto, object, prose};
    use crate::error::Error;
    use crate::standing::Standing;
    use crate::status_for;

    fn standing_in(repo: &Path) -> Standing {
        Standing::at(repo.to_path_buf(), repo.to_path_buf())
    }

    fn said(repo: &Path, path: &str, json: bool) -> String {
        let mut out = Vec::new();
        checked_onto(&standing_in(repo), PathBuf::from(path), json, &mut out)
            .expect("a check inside a repository answers");
        String::from_utf8(out).expect("warlock writes its own text")
    }

    #[test]
    fn the_composition_answers_about_a_path_the_manifest_has_never_heard_of() {
        let repo = tempfile::tempdir().expect("a temporary directory");

        let line = said(repo.path(), "src", false);

        assert!(
            line.contains("nothing scopes `src`"),
            "a repository that never pacted anything covers nothing: {line}"
        );
        assert!(
            line.contains("open to anyone"),
            "an unscoped path is open, and the answer should say so: {line}"
        );
        assert!(line.ends_with('\n'), "the answer is terminated: {line:?}");
    }

    #[test]
    fn the_composition_reads_the_scope_the_manifest_holds() {
        let repo = tempfile::tempdir().expect("a temporary directory");
        Manifest::with_entries([PactEntry::new(".", "src", "src/WARLOCK.md")
            .expect("a relative module path is inside the root")
            .with_scope("data-plane")])
        .save(repo.path())
        .expect("a manifest that saves");

        let line = said(repo.path(), "src", false);

        assert!(
            line.contains("data-plane"),
            "the scope on disk is not in the answer: {line}"
        );
    }

    #[test]
    fn a_missing_manifest_answers_and_an_unparsable_one_refuses() {
        let repo = tempfile::tempdir().expect("a temporary directory");
        // Missing: an answer, not a failure.
        let mut out = Vec::new();
        checked_onto(
            &standing_in(repo.path()),
            PathBuf::from("src"),
            false,
            &mut out,
        )
        .expect("a repository with no manifest still answers");

        // There and broken: a failure, and not one word printed first.
        let warlock = repo.path().join(".warlock");
        fs::create_dir_all(&warlock).expect("the bookkeeping directory");
        fs::write(warlock.join("pacts.toml"), "not toml {{{").expect("a broken manifest");

        let mut out = Vec::new();
        let error = checked_onto(
            &standing_in(repo.path()),
            PathBuf::from("src"),
            false,
            &mut out,
        )
        .expect_err("a manifest that will not parse is a failure");

        assert!(matches!(error, Error::Manifest { .. }), "{error:?}");
        assert!(
            out.is_empty(),
            "a partial answer was printed before the refusal: {out:?}"
        );
        assert_eq!(status_for(&Err(error)), 1);
    }

    #[test]
    fn the_json_answer_is_one_line_and_names_the_command() {
        let repo = tempfile::tempdir().expect("a temporary directory");

        let line = said(repo.path(), "src", true);

        assert_eq!(line.lines().count(), 1, "not one line: {line:?}");
        assert!(
            line.contains("\"command\":\"check\""),
            "the envelope does not name the command: {line}"
        );
    }

    // A path rather than a directory on disk, deliberately: coverage is a walk
    // up the manifest's stored paths and never a walk of the filesystem, so
    // nothing here has to exist for the answer to be the answer.
    const REPO: &str = "/repo";

    // Every test that reads or writes a config builds its home out of one of
    // these, so nothing here goes near the developer's real home.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    fn entry(module: &str) -> PactEntry {
        PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root")
    }

    fn a_manifest() -> Manifest {
        Manifest::with_entries([
            entry("crates").with_scope("platform"),
            entry("crates/engine").with_scope("data-plane"),
            entry("docs"),
        ])
    }

    fn answer(home: &Path, path: &str) -> Checked {
        checked(
            Path::new(REPO),
            Some(home),
            &a_manifest(),
            &Path::new(REPO).join(path),
        )
        .expect("a path inside the repository has a manifest form")
    }

    fn holding(home: &Path, sigils: &[&str]) {
        let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
        save_sigils(home, Path::new(REPO), &sigils).expect("a config that writes");
    }

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
        // A closed scope is the answer and not a failure to reach one, so what
        // `main` makes of it is a 0: the verdict is in the output, and the
        // non-zero status is left for `jq -e '.opens'` to spend. Asked a second
        // time for its `Result` alone, which the helper above unwraps — a check
        // reads one config file and walks a manifest already in hand, so asking
        // twice costs nothing.
        assert_eq!(
            status_for(
                &checked(
                    Path::new(REPO),
                    Some(home.path()),
                    &a_manifest(),
                    &Path::new(REPO).join("crates/engine"),
                )
                .map(|_| ())
            ),
            0
        );
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
            let refused =
                checked(Path::new(REPO), Some(home.path()), &a_manifest(), &outside).map(|_| ());
            // A question warlock could not answer, so 1 — the status a closed
            // scope above deliberately does not take.
            assert_eq!(status_for(&refused), 1, "{}", outside.display());

            let error = refused.expect_err("a path outside the repository has no manifest form");
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
