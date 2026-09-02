//! The headless writes: the boundary they are all asked over, and
//! `warlock unpact <path>`.
//!
//! The sixth subcommand, and the first one that changes anything. It keeps
//! every shape the five before it gave — dispatched before anything touches the
//! terminal, no alternate screen, no raw mode, no panic hook, no worker thread
//! and no subprocess, with a failure returned to `main` as an [`Error`] and
//! printed on the line that prints a tree which would not load — and it adds the
//! one thing a question never had: a `.warlock/pacts.toml` that is different
//! afterwards.
//!
//! That difference is the whole reason this module exists rather than a
//! `fn unpact` beside [`check`](crate::check). A question may be answered by
//! anybody; a write may not, and the rule about who may write where is one rule
//! with two doors onto it. Inside warlock the door is [`closed_scope`], which
//! refuses `p`, `r` and `s` over a boundary this machine does not hold. From a
//! shell the door is [`Opened::new`], below. Both ask the engine the same two
//! questions in the same order and print the same sentence when the answer is
//! no; neither is written in terms of the other, because [`closed_scope`] is
//! about a selected row on an [`App`](warlock_tui::App) and there is no app
//! here.
//!
//! # The boundary is asked first, and that ordering is the security property
//!
//! [`Opened`] cannot be built without the boundary having been asked, which is
//! how the ordering is kept: a subcommand that wants a repository root, a
//! manifest and a path gets all three from [`opened`] or gets none of them, and
//! by the time it has them the scope covering that path has already been held
//! against this machine's sigils. Nothing between the two can be forgotten,
//! because there is nothing between the two.
//!
//! First means first. Not before the write — before the *spelling*, before the
//! existence check, before any look at what the manifest holds. A closed
//! boundary must not be able to answer "there is no entry for that directory",
//! because that sentence is a fact about the inside of a manifest a reader has
//! just been told they may not work in; asked in the other order the refusal
//! would still be printed, and the shape of the repository would have leaked
//! past it anyway. The consequence is visible in [`Opened::unpacted`]: it spells
//! the path it was handed and may refuse it, and it can only run at all because
//! the boundary already said yes.
//!
//! # Nothing here decides what a boundary means
//!
//! [`scope_covering`] and [`scope_opens_to`], called once each and neither
//! re-implemented. Nearest-scope-wins, an invalid scope read as no scope, an
//! unscoped path open to anyone, and a machine holding nothing opening nothing
//! that is scoped are all decided in `crates/warlock-engine/src/scope.rs` and
//! nowhere else. What is held is [`sigils_under`]'s answer, which is the
//! header's own reading of the file `warlock config` writes: both
//! [`Sigils::Nothing`] — nobody has run `warlock config` on this machine — and
//! [`Sigils::Unknown`] — the config is there and will not parse — hold nothing,
//! so a scoped directory refuses them both. There is no `--force`, no
//! environment variable and no flag past this: `warlock config` is the one road.
//!
//! # What an un-pact is, and what it is not
//!
//! It is the manifest edit the TUI's `p` on an already-pacted subtree performs
//! and nothing else: [`unpact_subtree`] drops the entry for the named directory
//! and every entry below it, and [`Manifest::save`] writes the result. No walk,
//! no hash, no model pass, no `WARLOCK.md` removed or moved. Un-pacting is
//! warlock forgetting it ever promised to keep a document current; the documents
//! themselves are the repository's, and deleting somebody's prose because they
//! stopped tracking its freshness is not a thing warlock gets to do.
//!
//! # The blast radius, and why the success line is what it is
//!
//! `at_or_below` in the engine begins `selected == ROOT_MODULE`, so
//! `warlock unpact .` drops every entry in the manifest — and if the repository
//! root itself carries no scope, the boundary above waves it straight through
//! whatever the directories underneath are scoped. That is exactly what the `p`
//! key does today and this subcommand deliberately does not change it.
//!
//! What it changes is where the reader is standing. In the TUI you navigate to a
//! visible row, with the scopes drawn beside it and the subtree under the
//! cursor; from a shell it is one line in a script that scrolls past. So the
//! success line is the mitigation rather than decoration: it says how many
//! entries went, and it names every one of them that carried a scope, with the
//! scope. A run that quietly dropped somebody else's boundary now says whose.
//!
//! # An un-pact in a repository that never pacted anything
//!
//! [`load_manifest`] reads a missing `.warlock/pacts.toml` as an empty manifest,
//! so this succeeds, drops nothing, says `0 entries dropped` — and saves, which
//! creates the file. That is the decision rather than an oversight: the write is
//! unconditional so that there is one road through this function and no
//! second, quieter one for a caller to reason about, and what it writes is a
//! manifest saying exactly what was already true, which is that nothing is
//! pacted. It is idempotent, it costs one small file, and it removes the only
//! case where `warlock unpact` would have exited 0 having provably not written
//! the thing it says it wrote.

use std::env;
use std::path::{Path, PathBuf};

use warlock_engine::{
    Manifest, PactEntry, repository_root, scope_covering, scope_opens_to, unpact_subtree,
};
use warlock_tui::Sigils;

use crate::config::home_directory;
use crate::error::Error;
use crate::query::spelled;
use crate::session::{load_manifest, sigils_under};

/// What an un-pact wants a repository root for, as the tail of
/// [`Error::NoRepository`]'s sentence. The other subcommands' tails are spelled
/// beside them, each where its subcommand is written.
const FOR_UNPACT: &str = "un-pact anything under";

/// A repository a headless write may go ahead in, with the boundary already
/// asked.
///
/// The type is the gate. Its fields are private to this module and its only
/// constructor is [`Opened::new`], which asks [`scope_covering`] and
/// [`scope_opens_to`] before it hands one back — so possessing an `Opened` is
/// proof that this machine's sigils open the scope covering the path inside it,
/// and no write in this module can be reached without one. A later subcommand
/// that wants to edit the manifest from a shell asks for one of these and
/// inherits the check rather than remembering to repeat it.
///
/// The three fields are the whole of what such a write needs: where the
/// manifest lives, what it currently says, and where the reader pointed. What is
/// deliberately *not* kept is the sigils — they were a question asked once, and
/// keeping the answer around would invite a second read of it further down.
#[derive(Debug)]
pub(crate) struct Opened {
    /// The repository root: where `.warlock/pacts.toml` is read from and saved
    /// to, and what every stored path is spelled against.
    repo_root: PathBuf,
    /// The manifest as it stands, read once before the boundary was asked. A
    /// missing one is an empty one — see [`load_manifest`].
    manifest: Manifest,
    /// The path the reader named, joined onto the working directory: absolute
    /// when they typed an absolute one, and never normalised beyond that, so a
    /// `..` that climbs out of the repository is refused rather than resolved
    /// back inside it.
    target: PathBuf,
}

impl Opened {
    /// The boundary asked over `target`, and everything a write needs if it
    /// opens.
    ///
    /// Every input is a parameter — the manifest already in hand, the home the
    /// caller resolved, the path the caller joined — for the reason
    /// [`checked`](crate::check) takes them: the tests run against a temporary
    /// home and a temporary repository rather than the developer's own. The
    /// environment becomes those parameters in exactly one place, [`opened`].
    ///
    /// The two engine calls are the whole of the decision. A path the manifest
    /// cannot spell is passed through as open, which is [`closed_scope`]'s own
    /// reading of that case: coverage has nothing to say about a path that is
    /// not in this repository, and the command's own refusal — one line, naming
    /// the root it is not inside — is the better sentence than a boundary
    /// refusal on a technicality. It is refused a moment later, in the write, so
    /// nothing is written either way.
    ///
    /// # Errors
    ///
    /// [`Error::ClosedScope`], naming the path and the scope, when a scope
    /// covers `target` and this machine's sigils do not open it. Nothing is read
    /// or written after that, and the caller has no `Opened` to write with.
    fn new(
        repo_root: PathBuf,
        home: Option<&Path>,
        manifest: Manifest,
        target: PathBuf,
    ) -> Result<Self, Error> {
        // Resolved here rather than by the caller, so that the one thing this
        // check reads from disk is read inside the check. A home that cannot be
        // resolved is nothing held rather than a config that would not read:
        // there is no file in that case, so there is nothing broken to report.
        let sigils = home.map_or(Sigils::Nothing, |home| sigils_under(home, &repo_root));
        // `ok()` and not `?`: see above — a path with no manifest form is not a
        // boundary question, and the caller has a better sentence for it.
        let covering = scope_covering(&target, &repo_root, &manifest)
            .ok()
            .flatten();
        // `Nothing` and `Unknown` are both the empty slice on the way in
        // (`Sigils::as_slice`), which is what closes every scoped path to a
        // machine that has never been configured and to one whose config will
        // not parse.
        if scope_opens_to(covering, sigils.as_slice()) {
            return Ok(Self {
                repo_root,
                manifest,
                target,
            });
        }

        let Some(scope) = covering else {
            // Unreachable: `scope_opens_to` answers `true` for every path
            // nothing covers, so a refusal is always a refusal by a named
            // scope. Written out rather than unwrapped, because the one thing
            // this arm must never do is invent a scope to refuse in the name of.
            return Ok(Self {
                repo_root,
                manifest,
                target,
            });
        };
        Err(Error::ClosedScope {
            // Refused paths are spellable by construction: a path with no
            // manifest form has no coverage, and a path with no coverage did not
            // reach here. So the `?` is a formality that keeps the fallible call
            // honest rather than a second refusal.
            path: spelled(&repo_root, &target)?,
            scope: scope.to_owned(),
        })
    }

    /// Drop the entry for this path and every entry below it, save, and say what
    /// went.
    ///
    /// The whole of `warlock unpact` past the boundary, and it is two engine
    /// calls: [`unpact_subtree`] for the manifest that should be there now, and
    /// [`Manifest::save`] to put it there. Nothing else — no walk, no hash, no
    /// pass, no reload, and not a single `WARLOCK.md` touched. The manifest this
    /// value holds is left as it was found, because there is nobody left to show
    /// it to: the process is about to print one line and exit.
    ///
    /// What comes back is the line, without its `warlock: ` prefix, rather than
    /// anything printed here — so the sentence a reader sees is a value a test
    /// can assert about, exactly as [`prose`](crate::check) is.
    ///
    /// The dropped entries are worked out by difference rather than by asking
    /// the engine twice: whatever [`unpact_subtree`] kept is what remains, so an
    /// entry of the old manifest with no module of that name in the new one is
    /// an entry this call dropped. That keeps the count and the names honest
    /// against the engine's rule about what "below" means, including the part
    /// that says `crates/engine` does not swallow `crates/engine-tools`, without
    /// this file holding an opinion about it.
    ///
    /// A dropped entry's scope is named exactly as it is written down, including
    /// one [`validate_scope`](warlock_engine::validate_scope) would refuse.
    /// Coverage ignores such a scope, so it never closed this boundary — but it
    /// is a word somebody wrote in the file, and a line that silently omitted it
    /// would be warlock deciding on a reader's behalf that what they wrote did
    /// not count.
    ///
    /// # Errors
    ///
    /// [`Error::Unspellable`] for a path with no repository-relative form, in
    /// the shape [`spelled`] gives every subcommand, before anything is written;
    /// and [`Error::Manifest`] for a manifest that will not save, which is the
    /// engine's own sentence about the file it could not write. The old
    /// `.warlock/pacts.toml` is exactly as it was in both cases — the save is
    /// a write beside and a rename over, so there is no half-written state to
    /// leave behind.
    fn unpacted(&self) -> Result<String, Error> {
        // Spelled before the edit, because it is the name the answer is about
        // and because it is this command's refusal of a path from outside the
        // repository. `unpact_subtree` refuses the same path on the same grounds
        // a line later; asking here means the refusal happens before a manifest
        // is rebuilt rather than after.
        let path = spelled(&self.repo_root, &self.target)?;
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
}

/// Everything a headless write needs, resolved from the environment, with the
/// boundary already asked: the shared front half of every subcommand that edits
/// the manifest.
///
/// The steps are [`check`](crate::check)'s, in the same order and for the same
/// reasons: the working directory says where the repository is, the repository
/// root is resolved from it, the manifest and this machine's sigil config are
/// read, and `path` is taken relative to the working directory as a person
/// typing one at a shell means it ([`Path::join`] uses an absolute one as it
/// stands). This is the one place in the headless writes where the environment
/// becomes a home path and a repository root; everything past it takes both as
/// parameters, which is what keeps the tests off the developer's own home.
///
/// `wanted` is the tail of the sentence a missing repository is refused with, so
/// each subcommand says what *it* could not do rather than sharing one vague
/// one.
///
/// Nothing on disk has to exist for the boundary to be asked: coverage is a walk
/// up the manifest's stored paths and never a walk of the filesystem.
///
/// # Errors
///
/// [`Error::WorkingDirectory`] and [`Error::NoRepository`] before anything is
/// read, [`Error::Manifest`] for a manifest that will not parse, and
/// [`Error::ClosedScope`] for a boundary this machine does not hold. A sigil
/// config that will not read is deliberately not among them: it is a state of
/// the answer — nothing held, so nothing scoped is open — rather than a failure
/// to reach one.
fn opened(wanted: &'static str, path: &Path) -> Result<Opened, Error> {
    let working_dir = env::current_dir().map_err(|source| Error::WorkingDirectory { source })?;
    // Asked directly rather than through a load, as `init`, `config` and the
    // three questions ask: this edits one file, and walking the tree to find its
    // root would be reading every directory in the repository to answer a
    // question about ancestors.
    let repo_root = repository_root(&working_dir).ok_or(Error::NoRepository {
        start: working_dir.clone(),
        wanted,
    })?;
    let manifest = load_manifest(&repo_root)?;
    let home = home_directory().ok();
    let target = working_dir.join(path);

    Opened::new(repo_root, home.as_deref(), manifest, target)
}

/// `warlock unpact <path>`: drop that directory's pact and every pact below it,
/// and say what went.
///
/// Two lines, because the two halves are elsewhere on purpose: [`opened`] is the
/// boundary and the resolution, [`Opened::unpacted`] is the edit and the
/// sentence, and this is the subcommand — one println and an exit status.
///
/// The line is `init`'s shape, `warlock: ` and then the fact, on stdout, and the
/// status is 0. A refusal is one line on stderr through `main`'s own
/// `eprintln!`, and a 1.
///
/// # Errors
///
/// Everything [`opened`] and [`Opened::unpacted`] refuse, unchanged and
/// unwrapped: this adds no sentence of its own.
pub(crate) fn unpact(path: &Path) -> Result<(), Error> {
    println!("warlock: {}", opened(FOR_UNPACT, path)?.unpacted()?);
    Ok(())
}

/// What an un-pact of `path` that dropped `dropped` says it did.
///
/// The count first, because it is the fact that says whether the blast radius
/// was the one that was meant — `warlock unpact .` in a repository of forty
/// pacted directories says `40 entries dropped`, which is a number a reader
/// notices in a way that `unpacted .` is not.
///
/// Then the scopes, and only the scopes. Naming all forty paths would be a
/// paragraph on a line that has to stay one line ([`Error`]'s rule for stderr,
/// kept here for stdout because a script reads this with `read`); naming the
/// scoped ones is the half that matters, because a scope is somebody else's
/// boundary and dropping it silently is the thing this line exists to prevent.
/// Each is `path: scope`, in the manifest's own order, so the sentence reads as
/// a list of what was taken from whom.
///
/// Nothing scoped is dropped from the sentence rather than reported as `0
/// scoped`: the common case is a directory nobody has drawn a boundary near, and
/// a clause that is almost always "and none" trains a reader to stop reading the
/// line.
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use warlock_engine::{Manifest, PactEntry, manifest_path, save_sigils};

    use super::{Opened, unpacted_line};
    use crate::error::Error;
    use crate::session::load_manifest;
    use crate::status_for;

    /// A throwaway directory. Every test here builds both its repository and
    /// its home out of one of these, so nothing goes near the developer's real
    /// home or a real repository.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// An entry for `module`, documented the way a pact would document it.
    fn entry(module: &str) -> PactEntry {
        PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root")
    }

    /// A manifest with a scope on `crates`, a nearer one on `crates/engine`, an
    /// unscoped directory below that one, and a pacted-but-unscoped `docs`
    /// beside the lot.
    fn a_manifest() -> Manifest {
        Manifest::with_entries([
            entry("crates").with_scope("platform"),
            entry("crates/engine").with_scope("data-plane"),
            entry("crates/engine/src"),
            entry("docs"),
        ])
    }

    /// A repository holding [`a_manifest`] and a `WARLOCK.md` beside every
    /// directory in it.
    ///
    /// The documents are on disk rather than assumed, because "every
    /// `WARLOCK.md` stays where it was" is one of the things an un-pact
    /// promises and a promise about files needs files to be about.
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

    /// Write `sigils` as what the machine holds for the repository at
    /// `repo_root`, under `home`.
    fn holding(home: &Path, repo_root: &Path, sigils: &[&str]) {
        let sigils: Vec<String> = sigils.iter().map(|sigil| (*sigil).to_owned()).collect();
        save_sigils(home, repo_root, &sigils).expect("a config that writes");
    }

    /// The bytes of the manifest on disk, or `None` when there is no manifest.
    ///
    /// Bytes rather than a parsed [`Manifest`], because what a refusal promises
    /// is that the file did not change — not that it still parses to something
    /// equal.
    fn manifest_bytes(repo_root: &Path) -> Option<Vec<u8>> {
        fs::read(manifest_path(repo_root)).ok()
    }

    /// `warlock unpact <path>` in the repository at `repo_root`, run by a
    /// machine whose sigils are under `home`, with everything the environment
    /// would have settled handed in instead.
    ///
    /// The production road exactly: the boundary through [`Opened::new`], the
    /// edit through [`Opened::unpacted`], in that order and with no way to
    /// reach the second without the first.
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
        assert_eq!(
            error.to_string(),
            "crates/engine is scoped `data-plane` — hold that sigil to work here, \
             with `warlock config`"
        );
        assert!(!error.to_string().contains('\n'), "`main` prints one line");
        assert_eq!(status_for(&Err(error)), 1);
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
        // The mitigation for the blast radius: `.` is the root, the root carries
        // no scope, so this is waved through — and it says out loud whose
        // boundaries went with it.
        let repo = a_repository();
        let home = a_dir();

        let said = unpact(repo.path(), home.path(), ".").expect("an unscoped root is open");

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
}
