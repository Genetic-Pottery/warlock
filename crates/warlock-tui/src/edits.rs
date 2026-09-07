//! The headless writes — `warlock unpact`, `warlock scope add` and `warlock
//! scope remove` — and the boundary all three are asked over.
//!
//! [`Opened`] cannot be built without that question having been asked, which is
//! how the ordering is kept, and first means first: before the spelling, before
//! the existence check, before any look at what the manifest holds. A closed
//! boundary must not be able to answer "there is no entry for that directory",
//! because that is a fact about the inside of a manifest a reader has just been
//! told they may not work in. [`closed_scope`](crate::session::closed_scope) is
//! the same rule's other door and is not written in terms of this one, since it
//! is about a selected row on an `App` and there is no app here.
//!
//! A boundary this machine does not open is one line on stderr and exit status
//! **3** rather than 1, because the two want opposite things done about them: a
//! 1 is warlock unable to do the thing, a 3 is warlock declining to, and
//! re-running will never work. There is no `--force`.

use std::path::{Path, PathBuf};

use warlock_engine::{Manifest, PactEntry, unpact_subtree, validate_scope};

use crate::boundary::{Reach, Verdict, verdict};
use crate::error::Error;
use crate::query::spelled;
use crate::scoping::with_scope_on;
use crate::session::sigils_under;
use crate::standing::{FOR_SCOPE_ADD, FOR_SCOPE_REMOVE, FOR_UNPACT, Standing};

// The type is the gate: the fields are private to this module and the only
// constructor asks the boundary, so possessing an `Opened` is proof that this
// machine's sigils open the scope covering the path inside it. A later
// subcommand that wants to edit the manifest from a shell asks for one and
// inherits the check rather than remembering to repeat it — `running.rs` is the
// first, and the one it matters most for, because `warlock pact` spends model
// passes and rewrites documents, so a boundary asked after the descent had
// started would already have cost somebody's tokens and somebody else's prose.
#[derive(Debug)]
pub(crate) struct Opened {
    repo_root: PathBuf,
    manifest: Manifest,
    // Joined onto the working directory and never normalised beyond that, so a
    // `..` that climbs out of the repository is refused rather than resolved
    // back inside it.
    target: PathBuf,
    // Kept only because there is a *second* boundary question and exactly one of
    // the three writes asks it: an un-pact reaches below the path it was handed
    // and drops the scopes it finds there. Not a licence to ask the upward
    // question twice — that one is settled in `new`, once.
    held: Vec<String>,
}

impl Opened {
    // Every input is a parameter — the manifest in hand, the home the caller
    // resolved, the path it joined — so the tests run against a temporary home
    // and a temporary repository. The environment becomes those parameters in
    // exactly one place, `opened`.
    //
    // A path the manifest cannot spell is passed through as open, which is
    // `closed_scope`'s own reading of that case: coverage has nothing to say
    // about a path that is not in this repository, and the command's own refusal
    // — one line, naming the root it is not inside — is the better sentence than
    // a boundary refusal on a technicality. It is refused a moment later, in the
    // write, so nothing is written either way.
    pub(crate) fn new(
        repo_root: PathBuf,
        home: Option<&Path>,
        manifest: Manifest,
        target: PathBuf,
    ) -> Result<Self, Error> {
        // Resolved here rather than by the caller, so that the one thing this
        // check reads from disk is read inside the check. A home that cannot be
        // resolved is nothing held rather than a config that would not read:
        // there is no file in that case, so there is nothing broken to report.
        // Flattened here, at the door: what the shell needs is the two-valued
        // fact the gate takes, and the third state is a line only the header
        // and `warlock check` ever print. See `boundary::verdict`.
        let held: Vec<String> = home.map_or_else(Vec::new, |home| {
            sigils_under(home, &repo_root).as_slice().to_vec()
        });
        // The decision is [`verdict`]'s, and this is the shell's half of what to
        // do about it: an `Error`, which carries the exit status and prints the
        // very sentence the panel's footer says. The panel renders the same
        // verdict onto the footer and neither of them works the answer out for
        // itself. See `boundary.rs`.
        if let Verdict::Closed { scope } =
            verdict(&target, &repo_root, &manifest, &held, Reach::Here)
        {
            return Err(Error::ClosedScope {
                // Refused paths are spellable by construction: a path with no
                // manifest form has no coverage, and a path with no coverage did
                // not reach here. So the `?` is a formality that keeps the
                // fallible call honest rather than a second refusal.
                path: spelled(&repo_root, &target)?,
                scope,
            });
        }

        Ok(Self {
            repo_root,
            manifest,
            target,
            held,
        })
    }

    // Getters rather than public fields because the fields being private is the
    // gate: a caller may look at what an open boundary gave it and may not
    // assemble one of these out of parts it found lying around.
    pub(crate) fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub(crate) const fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub(crate) fn target(&self) -> &Path {
        &self.target
    }

    // Three engine calls and nothing else: no walk, no hash, no pass, and not a
    // single `WARLOCK.md` touched. Un-pacting is warlock forgetting it ever
    // promised to keep a document current; the documents are the repository's,
    // and deleting somebody's prose because they stopped tracking its freshness
    // is not a thing warlock gets to do.
    //
    // The second boundary question is here, and only an un-pact raises it.
    // `Opened::new` asked whether this machine may act *at* this path and
    // coverage walks up, so it has not looked below. This call drops every entry
    // underneath as well, and an entry is the only home a scope has — so without
    // a second question a boundary could be erased by aiming at its parent from a
    // machine that holds nothing. `warlock unpact .` drops the whole manifest,
    // and an unscoped root buys nothing, because the absence of a statement over
    // a path is not permission over the statements below it. The reasoning is
    // `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`. It is not
    // asked in `new`, which is also the two scope writes' gate and those erase no
    // boundary; and it is asked of the same engine function the `p` key asks, so
    // the two doors cannot drift into refusing where the other permits.
    //
    // The dropped entries are worked out by difference rather than by asking the
    // engine twice: whatever `unpact_subtree` kept is what remains. That keeps
    // the count and the names honest against the engine's rule about what "below"
    // means — including that `crates/engine` does not swallow
    // `crates/engine-tools` — without this file holding an opinion about it.
    //
    // The line comes back rather than being printed here, so the sentence a
    // reader sees is a value a test can assert about.
    fn unpacted(&self) -> Result<String, Error> {
        // Spelled before the edit, because it is the name the answer is about
        // and because it is this command's refusal of a path from outside the
        // repository. `unpact_subtree` refuses the same path on the same grounds
        // a line later; asking here means the refusal happens before a manifest
        // is rebuilt rather than after.
        let path = spelled(&self.repo_root, &self.target)?;
        // Before the rebuild, and before anything is said about what the
        // manifest holds: a refusal by a boundary names the scopes in the way
        // and nothing else about the inside of this repository. The narrower
        // question was already asked and answered by `Opened::new`, so what
        // this reach can still turn up is only ever what is underneath.
        if let Verdict::ClosedBelow { scopes } = verdict(
            &self.target,
            &self.repo_root,
            &self.manifest,
            &self.held,
            Reach::HereAndBelow,
        ) {
            return Err(Error::ClosedScopeBelow { path, scopes });
        }

        let remaining = unpact_subtree(&self.target, &self.repo_root, &self.manifest)
            // The engine's own case, rewrapped as the spelling refusal it is:
            // this agrees with the line above by construction, since both are
            // `to_manifest_path`.
            .map_err(|source| Error::Unspellable { source })?;

        let dropped: Vec<&PactEntry> = self
            .manifest
            .entries()
            .iter()
            .filter(|entry| remaining.entry(entry.module()).is_none())
            .collect();

        remaining
            .save(&self.repo_root)
            .map_err(|source| Error::Manifest { source })?;

        Ok(unpacted_line(&path, &dropped))
    }

    // Fold, then judge, in that order and never the other way round:
    // `Data-Plane` and `data-plane` are one boundary, so folding is what a caller
    // that took a string from a person does, and judging is `validate_scope`'s
    // and nobody else's. Folding is also the *only* thing done to what was typed
    // — nothing is trimmed, split on a comma or repaired into acceptability, so
    // `control-plane, data-plane` is one refused string rather than two scopes
    // somebody might have meant.
    //
    // The path is spelled first because the ordering rule stops at the boundary
    // and not at the write: a path with no manifest form is this command's own
    // refusal, and asking first means a run with two things wrong with it answers
    // about where it was pointed.
    fn scoped(&self, scope: &str) -> Result<String, Error> {
        let module = spelled(&self.repo_root, &self.target)?;
        // `to_ascii_lowercase` rather than `to_lowercase`, for `scope_submit`'s
        // reason: a scope is drawn from ASCII, so folding a non-ASCII capital
        // would produce a character the judge refuses anyway, and this way what
        // is refused is closer to what was typed.
        let folded = scope.to_ascii_lowercase();
        validate_scope(&folded).map_err(|rule| Error::Scope { rule })?;

        let was = self.scope_on(&module)?.map(str::to_owned);
        with_scope_on(&self.manifest, &module, Some(&folded))
            .save(&self.repo_root)
            .map_err(|source| Error::Manifest { source })?;

        Ok(scoped_line(&module, &folded, was.as_deref()))
    }

    // A directory carrying no scope is success and not a refusal, and the save
    // still happens: one road through this function, and what it writes is a
    // manifest identical to the one it read. A second, quieter road through a
    // write is a thing a caller then has to reason about.
    fn unscoped(&self) -> Result<String, Error> {
        let module = spelled(&self.repo_root, &self.target)?;
        let was = self.scope_on(&module)?.map(str::to_owned);
        with_scope_on(&self.manifest, &module, None)
            .save(&self.repo_root)
            .map_err(|source| Error::Manifest { source })?;

        Ok(unscoped_line(&module, was.as_deref()))
    }

    // The existence check and the "what was there before" both, because they are
    // one look at one entry. Only ever reached past an open boundary — whether
    // there is an entry is a fact about what the manifest holds, and the gate
    // above is what keeps it from being asked from outside a scope this machine
    // does not open.
    fn scope_on(&self, module: &str) -> Result<Option<&str>, Error> {
        self.manifest
            .entry(module)
            .map(PactEntry::scope)
            .ok_or_else(|| Error::NoPact {
                module: module.to_owned(),
            })
    }
}

// The one place in the headless writes where the environment becomes a home path
// and a repository root; everything past it takes both as parameters, which is
// what keeps the tests off the developer's own home. `path` is joined onto the
// working directory, which leaves an absolute one as it stands, and nothing on
// disk has to exist for the boundary to be asked: coverage is a walk up the
// manifest's stored paths and never a walk of the filesystem.
//
// `wanted` is the tail of the sentence a missing repository is refused with, so
// each subcommand says what *it* could not do. Shared with `running.rs` rather
// than copied there: a second function doing this would be a second order for
// these steps to be in.
//
// A sigil config that will not read is deliberately not a failure here: it is a
// state of the answer — nothing held, so nothing scoped is open.
pub(crate) fn opened(wanted: &'static str, path: &Path) -> Result<Opened, Error> {
    let standing = Standing::here(wanted)?;
    let manifest = standing.manifest()?;
    let home = Standing::home().ok();
    let target = standing.target(path);

    Opened::new(
        standing.repo_root().to_path_buf(),
        home.as_deref(),
        manifest,
        target,
    )
}

// Two lines, because the halves are elsewhere on purpose: `opened` is the
// boundary and the resolution, `Opened::unpacted` is the edit and the sentence,
// and this is the subcommand. `main` prints a refusal on stderr and takes the
// status from the error — a 3 for a closed boundary, a 1 for everything else.
pub(crate) fn unpact(path: &Path) -> Result<(), Error> {
    println!("warlock: {}", opened(FOR_UNPACT, path)?.unpacted()?);
    Ok(())
}

pub(crate) fn scope_add(path: &Path, scope: &str) -> Result<(), Error> {
    println!("warlock: {}", opened(FOR_SCOPE_ADD, path)?.scoped(scope)?);
    Ok(())
}

pub(crate) fn scope_remove(path: &Path) -> Result<(), Error> {
    println!("warlock: {}", opened(FOR_SCOPE_REMOVE, path)?.unscoped()?);
    Ok(())
}

// The success line is the mitigation for the blast radius, not decoration. In
// the panel you navigate to a visible row with the subtree under the cursor;
// from a shell this is one line in a script that scrolls past. So the count
// comes first — `warlock unpact .` in a repository of forty pacted directories
// says `40 entries dropped`, which is a number a reader notices — and then the
// scopes, and only the scopes. Naming all forty paths would be a paragraph on a
// line that has to stay one line; naming the scoped ones is the half that
// matters, because a scope is somebody else's boundary. Nothing scoped drops the
// clause rather than reporting `0 scoped`: a clause that is almost always "and
// none" trains a reader to stop reading the line.
//
// A dropped entry's scope is named exactly as it is written down, including one
// `validate_scope` would refuse. Coverage ignores such a scope, so it never
// closed this boundary — but it is a word somebody wrote in the file, and
// omitting it would be warlock deciding on a reader's behalf that what they
// wrote did not count.
fn unpacted_line(path: &str, dropped: &[&PactEntry]) -> String {
    let scoped: Vec<String> = dropped
        .iter()
        .filter_map(|entry| {
            entry
                .scope()
                .map(|scope| format!("{}: {scope}", entry.module()))
        })
        .collect();
    let taken = if scoped.is_empty() {
        String::new()
    } else {
        format!(", {} scoped ({})", scoped.len(), scoped.join(", "))
    };
    // Singular for one, because a line a person reads should not say "1
    // entries"; the count itself is what a script would parse either way.
    let entries = if dropped.len() == 1 {
        "entry"
    } else {
        "entries"
    };

    format!(
        "unpacted {path} — {} {entries} dropped{taken}",
        dropped.len()
    )
}

// The scope that was there before is named only when there was a different one:
// moving somebody's boundary is worth saying out loud, for `unpacted_line`'s
// reason, while re-writing the scope a directory already carried is a no-op
// nobody needs a clause about.
fn scoped_line(module: &str, scope: &str, was: Option<&str>) -> String {
    match was {
        Some(was) if was != scope => format!("{module} is scoped `{scope}` — was `{was}`"),
        _ => format!("{module} is scoped `{scope}`"),
    }
}

// Two sentences, because there are two things that can have happened and a
// reader is owed the difference: a boundary that was there and is not any more,
// named so the run is auditable, and a directory that carried no scope to begin
// with — which is success, exits 0, and says so rather than implying a removal
// nobody performed.
fn unscoped_line(module: &str, was: Option<&str>) -> String {
    match was {
        Some(was) => format!("{module} is no longer scoped — was `{was}`"),
        None => format!("{module} carried no scope"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use warlock_engine::{
        Manifest, Node, NodeState, PactEntry, Tree, manifest_path, save_sigils, validate_scope,
    };
    use warlock_tui::App;

    use super::{Opened, scoped_line, unpacted_line, unscoped_line};
    use crate::error::Error;
    // The other door onto the un-pact rule, pressed here so that the two are
    // held to one answer in one place. See `pressed_p`.
    use crate::pacting::pressed_p;
    // The sentence itself, asked of the one function that writes it rather than
    // retyped: the footer and the shell refuse the same boundary in the same
    // words, and a test holding a copy of those words is a test that would go on
    // passing while the two doors drifted apart.
    use crate::boundary::closed_scope_message;
    use crate::session::{load_manifest, sigils_under};
    use crate::status_for;

    // A grant on every entry, so that "the scope write left the run's own fields
    // alone" is an assertion about two values that are really there.
    const HASH: &str = "d0f5a1";

    const AT: &str = "2026-08-19T07:32:00Z";

    // Every test here builds both its repository and its home out of one of
    // these, so nothing goes near the developer's real home.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    // Granted rather than bare, because a scope write promises to leave the run's
    // own fields where it found them and a promise about a hash needs a hash to be
    // about. An un-pact drops whole entries, so it neither knows nor cares.
    fn entry(module: &str) -> PactEntry {
        PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root")
            .with_grant(HASH, AT)
    }

    fn a_manifest() -> Manifest {
        Manifest::with_entries([
            entry("crates").with_scope("platform"),
            entry("crates/engine").with_scope("data-plane"),
            entry("crates/engine/src"),
            entry("docs"),
        ])
    }

    // The documents are on disk rather than assumed, because "every `WARLOCK.md`
    // stays where it was" is one of the things an un-pact promises and a promise
    // about files needs files to be about.
    fn a_repository() -> tempfile::TempDir {
        let repo = a_dir();
        a_manifest()
            .save(repo.path())
            .expect("a manifest that saves");
        for entry in a_manifest().entries() {
            let document = entry.document_path(repo.path());
            fs::create_dir_all(document.parent().expect("a document has a directory"))
                .expect("a module directory");
            fs::write(&document, "a document\n").expect("a document");
        }

        repo
    }

    fn holding(home: &Path, repo_root: &Path, sigils: &[&str]) {
        let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
        save_sigils(home, repo_root, &sigils).expect("a config that writes");
    }

    // Bytes rather than a parsed `Manifest`, because what a refusal promises is
    // that the file did not change — not that it still parses to something equal.
    fn manifest_bytes(repo_root: &Path) -> Option<Vec<u8>> {
        fs::read(manifest_path(repo_root)).ok()
    }

    // The production road exactly, with what the environment would have settled
    // handed in instead: the boundary through `Opened::new` and then the edit,
    // with no way to reach the second without the first. The two below are the
    // same road.
    fn unpact(repo_root: &Path, home: &Path, path: &str) -> Result<String, Error> {
        let manifest = load_manifest(repo_root).expect("a manifest that reads");
        Opened::new(
            repo_root.to_path_buf(),
            Some(home),
            manifest,
            repo_root.join(path),
        )?
        .unpacted()
    }

    fn scope_add(repo_root: &Path, home: &Path, path: &str, scope: &str) -> Result<String, Error> {
        let manifest = load_manifest(repo_root).expect("a manifest that reads");
        Opened::new(
            repo_root.to_path_buf(),
            Some(home),
            manifest,
            repo_root.join(path),
        )?
        .scoped(scope)
    }

    fn scope_remove(repo_root: &Path, home: &Path, path: &str) -> Result<String, Error> {
        let manifest = load_manifest(repo_root).expect("a manifest that reads");
        Opened::new(
            repo_root.to_path_buf(),
            Some(home),
            manifest,
            repo_root.join(path),
        )?
        .unscoped()
    }

    fn stored(repo_root: &Path, module: &str) -> PactEntry {
        load_manifest(repo_root)
            .expect("a manifest that reads")
            .entry(module)
            .expect("the manifest holds this module")
            .clone()
    }

    // Asked of the one judge rather than retyped here, so a test cannot go on
    // agreeing with a wording warlock no longer uses.
    fn refusal(text: &str) -> String {
        validate_scope(text)
            .expect_err("this text is not a scope")
            .to_string()
    }

    #[test]
    fn an_open_boundary_drops_the_subtree_and_leaves_every_document_on_disk() {
        let repo = a_repository();
        let home = a_dir();
        holding(home.path(), repo.path(), &["platform", "data-plane"]);

        let said = unpact(repo.path(), home.path(), "crates").expect("an open boundary writes");

        // The subtree went, the sibling that merely shares a prefix of the name
        // did not, and the engine decided which was which.
        let modules: Vec<String> = load_manifest(repo.path())
            .expect("a manifest that reads")
            .entries()
            .iter()
            .map(|entry| entry.module().to_owned())
            .collect();
        assert_eq!(modules, ["docs"]);
        assert!(
            said.starts_with("unpacted crates — 3 entries dropped"),
            "{said}"
        );
        assert_eq!(status_for(&Ok(())), 0);

        // The promise the whole command is shaped around: warlock forgot the
        // pact, and the prose is still the repository's.
        for module in ["crates", "crates/engine", "crates/engine/src", "docs"] {
            let document = repo.path().join(module).join("WARLOCK.md");
            assert!(document.is_file(), "{} was removed", document.display());
        }
    }

    #[test]
    fn a_closed_boundary_refuses_and_leaves_the_manifest_byte_identical() {
        let repo = a_repository();
        let home = a_dir();
        // The nearest scope wins, so a machine holding the outer boundary is
        // still outside the inner one.
        holding(home.path(), repo.path(), &["platform"]);
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        let refused = unpact(repo.path(), home.path(), "crates/engine");

        let error = refused.expect_err("a scope this machine does not hold refuses");
        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "the boundary was refused as something else: {error:?}"
        );
        // The footer's own sentence, named rather than copied, about this path
        // and this scope: the shell says what the keystroke says.
        assert_eq!(
            error.to_string(),
            closed_scope_message("crates/engine", "data-plane")
        );
        assert!(!error.to_string().contains('\n'), "`main` prints one line");
        // The refusal's own status: not the 1 warlock spends on something it
        // could not do, because nothing was spent and nothing here can be
        // retried into working.
        assert_eq!(status_for(&Err(error)), 3);
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    }

    #[test]
    fn a_machine_holding_nothing_is_refused_by_every_scope_it_meets() {
        // No config at all: the ordinary state of a machine nobody has run
        // `warlock config` on, and the one an agent in a fresh checkout is in.
        let repo = a_repository();
        let home = a_dir();
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        for path in ["crates", "crates/engine", "crates/engine/src"] {
            let error = unpact(repo.path(), home.path(), path)
                .expect_err("holding nothing opens nothing that is scoped");
            assert!(
                matches!(error, Error::ClosedScope { .. }),
                "{path}: {error:?}"
            );
        }
        // And the unscoped directory beside them is open to that same machine:
        // the permissive default is on the directory and only there.
        unpact(repo.path(), home.path(), "docs").expect("nothing scopes `docs`");
        assert_ne!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    }

    #[test]
    fn the_success_line_names_every_dropped_entry_that_carried_a_scope() {
        // `.` is the root and the root carries no scope, so the boundary over it
        // waves this through — and the boundaries *under* it wave it through
        // because this machine holds both of them. It then says out loud whose
        // they were.
        let repo = a_repository();
        let home = a_dir();
        holding(home.path(), repo.path(), &["platform", "data-plane"]);

        let said = unpact(repo.path(), home.path(), ".").expect("every scope below is held");

        assert_eq!(
            said,
            "unpacted . — 4 entries dropped, 2 scoped (crates: platform, \
             crates/engine: data-plane)"
        );
        assert!(
            load_manifest(repo.path())
                .expect("a manifest that reads")
                .entries()
                .is_empty()
        );
    }

    #[test]
    fn a_boundary_below_the_path_refuses_the_unpact_and_names_every_scope_in_the_way() {
        // The blast radius, closed. `crates` opens to this machine and the root
        // is scoped by nobody, but both un-pacts reach a boundary this machine
        // is outside of — and an entry is the only home a scope has.
        let repo = a_repository();
        let home = a_dir();
        holding(home.path(), repo.path(), &["platform"]);
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        let error = unpact(repo.path(), home.path(), "crates")
            .expect_err("`crates/engine` is scoped `data-plane` and this machine is not");
        assert!(
            matches!(error, Error::ClosedScopeBelow { .. }),
            "the descendant boundary was refused as something else: {error:?}"
        );
        assert_eq!(
            error.to_string(),
            "un-pacting crates would drop pacts scoped `data-plane` — hold that sigil with \
             `warlock config`, or un-pact the parts you hold"
        );
        assert!(!error.to_string().contains('\n'), "`main` prints one line");
        // A 1 and deliberately not the boundary's 3: this machine may work at
        // `crates`, and the second road out of the sentence — un-pact the parts
        // you hold — needs no sigil at all, so it is not the "you are outside,
        // go and ask" verdict 3 exists for. Argued on `status_for`.
        assert_eq!(status_for(&Err(error)), 1);

        // The root, whose own unscoped-ness bought the whole repository today:
        // every distinct scope in the way is named, deduplicated and in the
        // manifest's order, so obtaining one sigil does not reveal the next.
        let error = unpact(repo.path(), home.path(), ".")
            .expect_err("an unscoped root is not permission over the scopes below it");
        assert_eq!(
            error.to_string(),
            "un-pacting . would drop pacts scoped `data-plane` — hold that sigil with \
             `warlock config`, or un-pact the parts you hold"
        );

        // And nothing was written on the way to either refusal.
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));

        // What is left is the road out the sentence offers: the parts this
        // machine does hold still un-pact, one subtree at a time.
        assert_eq!(
            unpact(repo.path(), home.path(), "docs").expect("nothing at or below `docs` is scoped"),
            "unpacted docs — 1 entry dropped"
        );
    }

    #[test]
    fn a_machine_holding_nothing_is_told_about_every_boundary_at_once() {
        // Holding nothing — the ordinary state of a fresh checkout — the root
        // un-pact meets both scopes, and both are named in the manifest's own
        // order rather than one at a time.
        let repo = a_repository();
        let home = a_dir();

        let error = unpact(repo.path(), home.path(), ".")
            .expect_err("holding nothing opens nothing that is scoped");
        assert_eq!(
            error.to_string(),
            "un-pacting . would drop pacts scoped `platform`, `data-plane` — hold those sigils \
             with `warlock config`, or un-pact the parts you hold"
        );
        // The descendant refusal's 1 again, and holding nothing does not change
        // it: what decides the status is which question was refused, not how
        // much this machine holds. See `status_for`.
        assert_eq!(status_for(&Err(error)), 1);
    }

    #[test]
    fn one_entry_is_counted_in_the_singular_and_an_unscoped_drop_says_no_more() {
        let repo = a_repository();
        let home = a_dir();

        assert_eq!(
            unpact(repo.path(), home.path(), "docs").expect("nothing scopes `docs`"),
            "unpacted docs — 1 entry dropped"
        );
    }

    #[test]
    fn a_path_with_no_manifest_form_is_refused_with_nothing_written() {
        // Not a boundary question — coverage has nothing to say about a path
        // that is not in this repository — so it is the command's own refusal,
        // in the shape every other subcommand refuses one.
        let repo = a_repository();
        let home = a_dir();
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        for outside in [PathBuf::from("/elsewhere"), repo.path().join("..")] {
            let manifest = load_manifest(repo.path()).expect("a manifest that reads");
            let refused = Opened::new(
                repo.path().to_path_buf(),
                Some(home.path()),
                manifest,
                outside.clone(),
            )
            .and_then(|opened| opened.unpacted());

            let error = refused.expect_err("a path outside the repository has no manifest form");
            assert!(
                matches!(error, Error::Unspellable { .. }),
                "{}: {error:?}",
                outside.display()
            );
            assert!(!error.to_string().contains('\n'), "`main` prints one line");
            assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
        }
    }

    #[test]
    fn an_unpact_in_a_repository_that_never_pacted_anything_writes_the_empty_manifest() {
        // The decision recorded in the module docs, pinned here: it succeeds, it
        // drops nothing, and it saves — so the file that appears says exactly
        // what was already true.
        let repo = a_dir();
        let home = a_dir();
        assert_eq!(manifest_bytes(repo.path()), None);

        assert_eq!(
            unpact(repo.path(), home.path(), ".").expect("an empty manifest has no boundary"),
            "unpacted . — 0 entries dropped"
        );
        assert!(
            load_manifest(repo.path())
                .expect("a manifest that reads")
                .entries()
                .is_empty()
        );
        assert!(manifest_bytes(repo.path()).is_some(), "nothing was saved");
    }

    #[test]
    fn a_scope_no_boundary_would_honour_is_still_named_when_it_is_dropped() {
        // `Data Plane!` is not a scope, so coverage ignores it and it closed
        // nothing — but somebody wrote it in the file, and a line that left it
        // out would be warlock deciding it did not count.
        let dropped = entry("crates/engine").with_scope("Data Plane!");

        assert_eq!(
            unpacted_line("crates", &[&dropped]),
            "unpacted crates — 1 entry dropped, 1 scoped (crates/engine: Data Plane!)"
        );
    }

    #[test]
    fn an_open_boundary_writes_the_scope_and_moves_nothing_else_in_the_file() {
        let repo = a_repository();
        let home = a_dir();

        // `docs` carries no scope and nothing above it does, so it is open to a
        // machine that has never run `warlock config`.
        let said =
            scope_add(repo.path(), home.path(), "docs", "billing").expect("nothing scopes `docs`");

        assert_eq!(said, "docs is scoped `billing`");
        assert_eq!(status_for(&Ok(())), 0);
        let docs = stored(repo.path(), "docs");
        assert_eq!(docs.scope(), Some("billing"));
        // The one field a person owns, and nothing else on the entry.
        assert_eq!(docs.document(), "docs/WARLOCK.md");
        assert_eq!(docs.granted_hash(), Some(HASH));
        assert_eq!(docs.granted_at(), Some(AT));

        // Every other entry cloned untouched, in the order they were in, so the
        // diff against what was there is the one scope line.
        let after = load_manifest(repo.path()).expect("a manifest that reads");
        let before = a_manifest();
        assert_eq!(
            after
                .entries()
                .iter()
                .map(PactEntry::module)
                .collect::<Vec<_>>(),
            before
                .entries()
                .iter()
                .map(PactEntry::module)
                .collect::<Vec<_>>(),
        );
        for module in ["crates", "crates/engine", "crates/engine/src"] {
            assert_eq!(after.entry(module), before.entry(module), "{module}");
        }
    }

    #[test]
    fn a_scope_that_replaces_another_says_whose_boundary_it_moved() {
        let repo = a_repository();
        let home = a_dir();
        holding(home.path(), repo.path(), &["data-plane"]);

        let said = scope_add(repo.path(), home.path(), "crates/engine", "billing")
            .expect("the machine holds the scope covering this directory");

        // The mitigation the un-pact line is: a boundary that moved is named,
        // because a script that quietly redrew somebody else's says whose.
        assert_eq!(said, "crates/engine is scoped `billing` — was `data-plane`");
        assert_eq!(
            stored(repo.path(), "crates/engine").scope(),
            Some("billing")
        );
        // And re-writing the scope a directory already carries has nothing to
        // report about a boundary nobody moved.
        assert_eq!(
            scoped_line("docs", "billing", Some("billing")),
            "docs is scoped `billing`"
        );
    }

    #[test]
    fn what_was_given_is_folded_before_it_is_judged_and_stored() {
        // `Data-Plane` and `data-plane` are one boundary, and folding belongs to
        // the caller that took the string from a person — the judge refuses a
        // capital outright, as the assertion below shows.
        let repo = a_repository();
        let home = a_dir();

        let said = scope_add(repo.path(), home.path(), "docs", "Data-Plane")
            .expect("the fold happened before the judge");

        assert!(validate_scope("Data-Plane").is_err());
        assert_eq!(said, "docs is scoped `data-plane`");
        assert_eq!(stored(repo.path(), "docs").scope(), Some("data-plane"));
    }

    #[test]
    fn removing_a_scope_clears_it_and_leaves_the_document_and_the_grant() {
        let repo = a_repository();
        let home = a_dir();
        holding(home.path(), repo.path(), &["data-plane"]);

        let said = scope_remove(repo.path(), home.path(), "crates/engine")
            .expect("the machine holds the scope covering this directory");

        assert_eq!(said, "crates/engine is no longer scoped — was `data-plane`");
        assert_eq!(status_for(&Ok(())), 0);
        let engine = stored(repo.path(), "crates/engine");
        assert_eq!(engine.scope(), None);
        assert_eq!(engine.document(), "crates/engine/WARLOCK.md");
        assert_eq!(engine.granted_hash(), Some(HASH));
        assert_eq!(engine.granted_at(), Some(AT));
        // The entry above it kept its own boundary: this is one entry's field.
        assert_eq!(stored(repo.path(), "crates").scope(), Some("platform"));
    }

    #[test]
    fn removing_a_scope_from_a_directory_that_carries_none_is_success_and_writes_the_same_file() {
        // Idempotence, said as a fact rather than as a refusal: the command's
        // job is to make "this directory carries no scope" true, and it already
        // was.
        let repo = a_repository();
        let home = a_dir();
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        let said = scope_remove(repo.path(), home.path(), "docs").expect("nothing scopes `docs`");

        assert_eq!(said, "docs carried no scope");
        assert_eq!(status_for(&Ok(())), 0);
        assert_eq!(
            manifest_bytes(repo.path()).as_deref(),
            Some(&before[..]),
            "an idempotent clear rewrote the file differently"
        );
    }

    #[test]
    fn a_closed_boundary_refuses_both_scope_writes_and_leaves_the_manifest_byte_identical() {
        let repo = a_repository();
        let home = a_dir();
        // The nearest scope wins, so the machine holding the outer boundary is
        // still outside the inner one.
        holding(home.path(), repo.path(), &["platform"]);
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        let refusals = [
            scope_add(repo.path(), home.path(), "crates/engine", "billing")
                .expect_err("a scope this machine does not hold refuses an add"),
            scope_remove(repo.path(), home.path(), "crates/engine")
                .expect_err("and refuses a remove"),
        ];

        for error in refusals {
            assert!(
                matches!(error, Error::ClosedScope { .. }),
                "the boundary was refused as something else: {error:?}"
            );
            // The same sentence the un-pact is refused with and the same one
            // the footer puts up, asked of the one function that writes it.
            assert_eq!(
                error.to_string(),
                closed_scope_message("crates/engine", "data-plane")
            );
            assert!(!error.to_string().contains('\n'), "`main` prints one line");
            // The same boundary, so the same status the un-pact gets: one
            // refusal, one number, whichever write met it.
            assert_eq!(status_for(&Err(error)), 3);
        }
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    }

    #[test]
    fn the_boundary_is_asked_before_the_path_is_checked_for_an_entry() {
        // The ordering that is the security property: `crates/tui` has no entry
        // in the manifest and sits inside the boundary `crates` draws, so from
        // outside that boundary the answer is the scope refusal — never "is not
        // in the manifest", which is a fact about the inside of a file the
        // reader has just been told they may not work in.
        let repo = a_repository();
        let home = a_dir();
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        let error = scope_add(repo.path(), home.path(), "crates/tui", "billing")
            .expect_err("holding nothing opens nothing that is scoped");

        assert!(
            matches!(error, Error::ClosedScope { .. }),
            "the manifest's shape leaked past a closed boundary: {error:?}"
        );
        assert!(
            !error.to_string().contains("not in the manifest"),
            "{error}"
        );
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));

        // And past the same boundary held, the same path answers with what the
        // manifest holds — so the sentence exists and is only ever reached from
        // inside.
        holding(home.path(), repo.path(), &["platform"]);
        let error = scope_add(repo.path(), home.path(), "crates/tui", "billing")
            .expect_err("there is no entry to write a scope on");
        assert!(matches!(error, Error::NoPact { .. }), "{error:?}");
    }

    #[test]
    fn a_closed_boundary_answers_a_clear_and_an_unpact_before_either_reads_the_manifest() {
        // The ordering the test above pins for `scope add`, held over the other
        // two writes, because it is one rule and the gate is one place: from
        // outside the boundary `crates` draws, neither may say what the manifest
        // holds about `crates/tui` — not "is not in the manifest" for the clear,
        // and not "0 entries dropped" for the un-pact, which is the same fact
        // about an empty subtree worded as a success.
        let repo = a_repository();
        let home = a_dir();
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        for refused in [
            scope_remove(repo.path(), home.path(), "crates/tui"),
            unpact(repo.path(), home.path(), "crates/tui"),
        ] {
            let error = refused.expect_err("holding nothing opens nothing that is scoped");
            assert!(
                matches!(error, Error::ClosedScope { .. }),
                "the manifest's shape leaked past a closed boundary: {error:?}"
            );
            assert!(
                !error.to_string().contains("not in the manifest"),
                "{error}"
            );
            // And the status leaks nothing either: a closed boundary over a
            // path with no entry is the boundary's 3, the same number it would
            // be over a path with one.
            assert_eq!(status_for(&Err(error)), 3);
        }
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));

        // And past the same boundary held, each answers about the manifest
        // after all: the clear with the refusal naming the pact that is not
        // there, the un-pact with a subtree that had nothing in it — so both
        // sentences exist and are only ever reached from inside.
        holding(home.path(), repo.path(), &["platform"]);
        let error = scope_remove(repo.path(), home.path(), "crates/tui")
            .expect_err("there is no entry to clear a scope on");
        assert!(matches!(error, Error::NoPact { .. }), "{error:?}");
        assert_eq!(
            unpact(repo.path(), home.path(), "crates/tui")
                .expect("nothing is pacted at or below `crates/tui`"),
            "unpacted crates/tui — 0 entries dropped"
        );
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    }

    #[test]
    fn a_scope_the_engine_refuses_prints_its_rule_and_writes_nothing() {
        let repo = a_repository();
        let home = a_dir();
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        // The list, the capital-with-a-space, and the empty argument — which is
        // the `Empty` rule rather than a clear, because clearing is `scope
        // remove`. Each is judged after the fold, so the text held against the
        // judge here is the lower-cased one.
        for (given, folded) in [
            ("control-plane, data-plane", "control-plane, data-plane"),
            ("Control Plane", "control plane"),
            ("", ""),
            ("data-plane-", "data-plane-"),
        ] {
            let error = scope_add(repo.path(), home.path(), "docs", given)
                .expect_err("this is not a scope");

            assert!(matches!(error, Error::Scope { .. }), "{given:?}: {error:?}");
            // The engine's own sentence about the one rule that was broken,
            // asked of the judge rather than retyped — and asked about the
            // folded text, because folding is the one thing done to what was
            // given.
            assert_eq!(error.to_string(), refusal(folded), "{given:?}");
            assert!(!error.to_string().contains('\n'), "{given:?}");
            assert_eq!(status_for(&Err(error)), 1, "{given:?}");
        }

        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    }

    #[test]
    fn a_directory_with_no_entry_is_refused_past_an_open_boundary_and_writes_nothing() {
        // Nothing scopes `docs/adr` and nothing above it does, so the boundary
        // waves it through and the manifest gets the next word: there is no pact
        // here to carry a scope.
        let repo = a_repository();
        let home = a_dir();
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        let refusals = [
            scope_add(repo.path(), home.path(), "docs/adr", "billing")
                .expect_err("`docs/adr` has no entry"),
            scope_remove(repo.path(), home.path(), "docs/adr")
                .expect_err("and has none to clear either"),
        ];

        for error in refusals {
            assert!(matches!(error, Error::NoPact { .. }), "{error:?}");
            let said = error.to_string();
            // `no_pact_message`'s shape: it names the directory and points at
            // pacting it.
            assert!(said.contains("docs/adr"), "{said}");
            assert!(said.contains("`p`"), "{said}");
            assert!(!said.contains('\n'), "`main` prints one line");
            assert_eq!(status_for(&Err(error)), 1);
        }
        assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
    }

    #[test]
    fn a_path_with_no_manifest_form_is_refused_by_both_scope_writes() {
        let repo = a_repository();
        let home = a_dir();
        let before = manifest_bytes(repo.path()).expect("a manifest on disk");

        for outside in [PathBuf::from("/elsewhere"), repo.path().join("..")] {
            let manifest = load_manifest(repo.path()).expect("a manifest that reads");
            let opened = || {
                Opened::new(
                    repo.path().to_path_buf(),
                    Some(home.path()),
                    manifest.clone(),
                    outside.clone(),
                )
            };

            for refused in [
                opened().and_then(|opened| opened.scoped("billing")),
                opened().and_then(|opened| opened.unscoped()),
            ] {
                let error =
                    refused.expect_err("a path outside the repository has no manifest form");
                assert!(
                    matches!(error, Error::Unspellable { .. }),
                    "{}: {error:?}",
                    outside.display()
                );
                assert!(!error.to_string().contains('\n'), "`main` prints one line");
            }
            assert_eq!(manifest_bytes(repo.path()).as_deref(), Some(&before[..]));
        }
    }

    #[test]
    fn a_clear_that_took_a_boundary_away_names_it_and_one_that_took_nothing_says_so() {
        assert_eq!(
            unscoped_line("crates/engine", Some("data-plane")),
            "crates/engine is no longer scoped — was `data-plane`"
        );
        assert_eq!(unscoped_line("docs", None), "docs carried no scope");
    }

    // The one sigil this machine holds, on both doors and in every case below.
    const HELD: &str = "platform";

    // The scope it does not, which is the one every refusal here is by.
    const CLOSED: &str = "data-plane";

    // Every shape the two doors have to answer alike, in one manifest: a root
    // entry carrying no scope, a boundary this machine holds on `crates`, one it
    // does not on `crates/engine` below that, and a second subtree whose only
    // boundary below is one it does hold. One manifest rather than one per case,
    // because a parity test over several fixtures would be showing that the two
    // doors agree about several different repositories.
    fn a_manifest_of_boundaries_both_ways() -> Manifest {
        Manifest::with_entries([
            // Spelled out rather than through `entry`, which would document the
            // root as `./WARLOCK.md`.
            PactEntry::new(".", ".", "WARLOCK.md")
                .expect("the repository root is inside itself")
                .with_grant(HASH, AT),
            entry("crates").with_scope(HELD),
            entry("crates/engine").with_scope(CLOSED),
            entry("crates/engine/src"),
            entry("docs"),
            entry("docs/api").with_scope(HELD),
        ])
    }

    // No documents on disk: these tests are about which un-pacts are allowed, and
    // neither door reads a `WARLOCK.md` to decide that. That an un-pact leaves
    // every document where it was is pinned above, over a repository that has
    // them.
    fn a_repository_of_boundaries() -> (tempfile::TempDir, tempfile::TempDir) {
        let repo = a_dir();
        let home = a_dir();
        a_manifest_of_boundaries_both_ways()
            .save(repo.path())
            .expect("a manifest that saves");
        holding(home.path(), repo.path(), &[HELD]);

        (repo, home)
    }

    // Every directory the manifest names, each one pacted, so that `p` on any row
    // is an un-pact.
    fn a_panel_over(repo_root: &Path) -> App {
        let node = |name: &str, children: Vec<Node>| {
            Node::new(
                repo_root.join(name),
                None::<PathBuf>,
                NodeState::PactedFresh,
            )
            .with_children(children)
        };

        App::from_tree(&Tree::new(
            Node::new(repo_root, None::<PathBuf>, NodeState::PactedFresh).with_children([
                node(
                    "crates",
                    vec![node(
                        "crates/engine",
                        vec![node("crates/engine/src", Vec::new())],
                    )],
                ),
                node("docs", vec![node("docs/api", Vec::new())]),
            ]),
        ))
    }

    // The answer, and deliberately not the mechanism: the panel refuses by
    // painting nothing and putting a line on the footer, the shell by handing
    // `main` an error to print. Those are two shapes of one rule, and this is
    // what the two of them have to be equal in.
    #[derive(Debug, PartialEq, Eq)]
    enum Answer {
        WentAhead,
        Refused(String),
    }

    // The one difference between the doors that is not about the rule:
    // `App::label_for` spells a row relative to the tree's root and falls back to
    // the absolute path for the root row itself, where the shell spells that row
    // `.`. Every other row is named by the manifest's own spelling on both sides,
    // so this is a no-op for them.
    fn as_the_shell_says_it(sentence: &str, repo_root: &Path) -> String {
        sentence.replace(&repo_root.display().to_string(), ".")
    }

    fn panel_answer(repo_root: &Path, home: &Path, path: &str) -> Answer {
        let manifest = load_manifest(repo_root).expect("a manifest that reads");
        // The header's own reading of the config `warlock config` wrote, which
        // is what the running app holds and what the shell reads for itself.
        let sigils = sigils_under(home, repo_root);
        let mut app = a_panel_over(repo_root);
        let target = if path == "." {
            repo_root.to_path_buf()
        } else {
            repo_root.join(path)
        };
        let row = app
            .rows()
            .iter()
            .position(|row| row.path == target)
            .expect("the panel draws a row for this directory");
        app.select_row(row);

        match pressed_p(&mut app, &manifest, repo_root, &sigils) {
            Some(toggle) => {
                assert!(!toggle.pacted, "{path}: the press was not an un-pact");
                Answer::WentAhead
            }
            None => Answer::Refused(as_the_shell_says_it(
                app.message().expect("a refused press says why"),
                repo_root,
            )),
        }
    }

    fn shell_answer(repo_root: &Path, home: &Path, path: &str) -> Answer {
        match unpact(repo_root, home, path) {
            Ok(_) => Answer::WentAhead,
            Err(error) => Answer::Refused(error.to_string()),
        }
    }

    #[test]
    fn a_key_press_and_a_shell_prompt_answer_the_same_un_pact_alike() {
        // The rule's own last clause: there is no path by which one door refuses
        // and the other permits. Both are pressed over one manifest, by one
        // machine holding one sigil, and the answers are held against each other
        // *and* against what the answer is supposed to be — so a change to
        // either door alone fails here, and so does a change to both that moves
        // the rule.
        //
        // The rule is `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`.
        let refused_here = format!(
            "crates/engine is scoped `{CLOSED}` — hold that sigil to work here, \
             with `warlock config`"
        );
        let refused_below = |label: &str| {
            format!(
                "un-pacting {label} would drop pacts scoped `{CLOSED}` — hold that sigil with \
                 `warlock config`, or un-pact the parts you hold"
            )
        };

        for (path, expected) in [
            // A scope on the target itself, which coverage has always seen.
            ("crates/engine", Answer::Refused(refused_here)),
            // A target this machine's own sigil opens, over an entry below it
            // that it does not: passing the first question is not permission for
            // the second.
            ("crates", Answer::Refused(refused_below("crates"))),
            // A boundary below that this machine holds is no obstacle, so the
            // subtree goes — the rule refuses over scopes, not over having any.
            ("docs", Answer::WentAhead),
            // The root, which carries no scope of its own. That is the absence
            // of a statement rather than permission over the statements below.
            (".", Answer::Refused(refused_below("."))),
        ] {
            // A repository each, because the un-pact that goes ahead saves.
            let (repo, home) = a_repository_of_boundaries();

            let panel = panel_answer(repo.path(), home.path(), path);
            let shell = shell_answer(repo.path(), home.path(), path);

            assert_eq!(
                panel, shell,
                "`p` and `warlock unpact` disagree over {path}"
            );
            assert_eq!(panel, expected, "the answer over {path} has changed");
        }
    }
}
