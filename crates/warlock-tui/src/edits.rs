//! The headless writes: the boundary they are all asked over, and the three of
//! them — `warlock unpact <path>`, `warlock scope add <path> <scope>` and
//! `warlock scope remove <path>`.
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
//! An un-pact then asks a *second* boundary question, about what the act would
//! reach rather than where it stands, and that one is asked inside
//! [`Opened::unpacted`] — after the spelling, and after the first one has been
//! answered. The ordering above is untouched by it. That rule is about the
//! target: a machine outside the scope covering the path still learns nothing
//! about this manifest, because it never gets an `Opened` at all. Past that
//! first yes, what the second refusal discloses is scopes at or below a path the
//! reader may already work in — words committed to `.warlock/pacts.toml` and
//! visible to everyone who clones the repository — and it discloses no path, no
//! entry and no count. Nothing is written on the way to it either: the refusal
//! precedes the rebuild and the save.
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
//! # What the refusal costs, and the number it leaves behind
//!
//! A boundary this machine does not open is one line on stderr and **exit
//! status 3**, with `.warlock/pacts.toml` byte-identical to what was read. 3 is
//! the refusal's own number across every write warlock grows, and it is not 1
//! because the two want opposite things done about them: a 1 is warlock unable
//! to do the thing, and the line is there to be read; a 3 is warlock declining
//! to, nothing was spent, and re-running it will never work — the road out is
//! `warlock config` and a sigil somebody else has to hand over. A script that
//! had to tell those apart by their wording would be parsing prose. The
//! vocabulary in full, and the reasoning for the un-pact's *second* refusal
//! keeping a 1, is on [`status_for`](crate::status_for).
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
//! `warlock unpact .` drops every entry in the manifest, and an entry is the
//! only home a scope has. What keeps that from erasing somebody's boundary from
//! outside it is the second question in [`Opened::unpacted`]: a subtree carrying
//! a scope this machine does not hold refuses the un-pact, and an unscoped
//! repository root buys nothing, because the absence of a statement over a path
//! is not permission over the statements below it. That rule is argued in
//! `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`, and the `p`
//! key is held to it in the same words — neither door refuses where the other
//! permits, which is a fact the tests at the foot of this file press both doors
//! to state.
//!
//! Inside the boundary the radius is still the whole subtree, and a scope this
//! machine *does* hold goes with its entry like any other. What differs from
//! the TUI there is where the reader is standing: in the panel you navigate to a
//! visible row, with the scopes drawn beside it and the subtree under the
//! cursor; from a shell it is one line in a script that scrolls past. So the
//! success line is the mitigation rather than decoration: it says how many
//! entries went, and it names every one of them that carried a scope, with the
//! scope. A run that dropped a boundary this machine was inside of says which.
//!
//! # What a scope write is, and where its rules come from
//!
//! `warlock scope add` and `warlock scope remove` are the `s` key's write with
//! the window taken off the front of it, and every rule they keep is a rule
//! [`scope_submit`] keeps. The scope is lower-cased with `to_ascii_lowercase`
//! and then judged by [`validate_scope`], in that order and never the other way
//! round: `Data-Plane` and `data-plane` are one boundary, so folding is what a
//! caller that took a string from a person does, and judging is the engine's and
//! nobody else's. There is no length constant, no character predicate and no
//! second opinion about scopes anywhere in this file.
//!
//! Folding is also the *only* thing done to what was typed. Nothing is trimmed,
//! split on a comma or repaired into acceptability: `control-plane, data-plane`
//! is one refused string rather than two scopes somebody might have meant, and
//! what comes back is the engine's own sentence about the one rule that was
//! broken, on stderr, with `.warlock/pacts.toml` untouched.
//!
//! The write itself is [`with_scope_on`], borrowed from [`mod@crate::scoping`]
//! rather than written again: the manifest is rebuilt through
//! [`PactEntry::with_scope`] and [`PactEntry::without_scope`], every other entry
//! cloned as it stands and the order kept, so the saved file differs from the
//! one on disk by the scope line and nothing else — the document, the granted
//! hash and the granted timestamp are the run's and are not this edit's to move.
//!
//! Two things the shell has that the window does not, and two it does not have.
//! Clearing is `warlock scope remove` rather than an empty field, so there is no
//! empty-argument case here at all — `warlock scope add <path> ''` is
//! [`validate_scope`]'s `Empty` rule, which is what a person who typed it by
//! accident is owed. And a remove over a directory carrying no scope is success
//! rather than a refusal: it says the directory carried no scope, exits 0, and
//! writes a manifest identical to the one it read, because a command whose job
//! is to make a fact true has nothing to complain about when it already is.
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
    Manifest, PactEntry, closed_scopes_at_or_below, repository_root, scope_covering,
    scope_opens_to, unpact_subtree, validate_scope,
};
use warlock_tui::Sigils;

use crate::config::home_directory;
use crate::error::Error;
use crate::query::spelled;
use crate::scoping::with_scope_on;
use crate::session::{load_manifest, sigils_under};

/// What an un-pact wants a repository root for, as the tail of
/// [`Error::NoRepository`]'s sentence. The other subcommands' tails are spelled
/// beside them, each where its subcommand is written.
const FOR_UNPACT: &str = "un-pact anything under";

/// What `warlock scope add` wants one for, in the same shape.
const FOR_SCOPE_ADD: &str = "write a scope in";

/// And `warlock scope remove`, which is the same fact about `.git` with the
/// other consequence on the end of it.
const FOR_SCOPE_REMOVE: &str = "clear a scope in";

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
/// Three of the fields are the whole of what such a write needs: where the
/// manifest lives, what it currently says, and where the reader pointed. The
/// fourth is this machine's sigils, kept because there is a *second* boundary
/// question and exactly one of the three writes asks it: an un-pact reaches
/// below the path it was handed and drops the scopes it finds there, so
/// [`Opened::unpacted`] asks what the act would reach after this constructor has
/// asked whether it may act here at all. Keeping them is not a licence to ask
/// the upward question twice — that one is settled here, once, and no write in
/// this module asks it again.
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
    /// What this machine holds, read once by [`Opened::new`] from the config
    /// `warlock config` writes, for the one question that is left to ask:
    /// whether an un-pact of `target` would drop a boundary it does not hold.
    /// Read by [`Opened::unpacted`] and by nothing else.
    sigils: Sigils,
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
                sigils,
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
                sigils,
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
    /// The whole of `warlock unpact` past the boundary, and it is three engine
    /// calls: [`closed_scopes_at_or_below`] for the boundaries the act would
    /// take with it, [`unpact_subtree`] for the manifest that should be there
    /// now, and [`Manifest::save`] to put it there. Nothing else — no walk, no
    /// hash, no pass, no reload, and not a single `WARLOCK.md` touched. The
    /// manifest this value holds is left as it was found, because there is
    /// nobody left to show it to: the process is about to print one line and
    /// exit.
    ///
    /// # The second boundary question, which only an un-pact raises
    ///
    /// [`Opened::new`] asked whether this machine may act *at* this path, and
    /// coverage walks up, so it has not looked below it. This call drops every
    /// entry underneath as well, and an entry is the only home a scope has — so
    /// without a second question a boundary could be erased by aiming at its
    /// parent, from a machine that holds nothing. It is refused instead, in the
    /// footer's own words, and the reasoning is
    /// `docs/warlock-decision-un-pacting-across-a-descendant-scope.md`.
    ///
    /// The question is asked here and not in [`Opened::new`], because `new` is
    /// also `warlock scope add`'s gate and `warlock scope remove`'s, and those
    /// two write one line onto one entry and erase no boundary at all. It is
    /// asked of [`closed_scopes_at_or_below`] rather than worked out from the
    /// dropped entries below, because the [`p`](crate::pacting) key asks the
    /// same engine function over the same manifest: one answer, so the two doors
    /// cannot drift into refusing where the other permits.
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
    /// [`Error::ClosedScopeBelow`], naming every distinct scope in the way, when
    /// the subtree carries a boundary this machine does not hold; and
    /// [`Error::Manifest`] for a manifest that will not save, which is the
    /// engine's own sentence about the file it could not write. The old
    /// `.warlock/pacts.toml` is exactly as it was in all three cases — the save
    /// is a write beside and a rename over, so there is no half-written state to
    /// leave behind.
    fn unpacted(&self) -> Result<String, Error> {
        // Spelled before the edit, because it is the name the answer is about
        // and because it is this command's refusal of a path from outside the
        // repository. `unpact_subtree` refuses the same path on the same grounds
        // a line later; asking here means the refusal happens before a manifest
        // is rebuilt rather than after.
        let path = spelled(&self.repo_root, &self.target)?;
        // Before the rebuild, and before anything is said about what the
        // manifest holds: a refusal by a boundary names the scopes in the way
        // and nothing else about the inside of this repository.
        let blocking = closed_scopes_at_or_below(
            &self.target,
            &self.repo_root,
            &self.manifest,
            self.sigils.as_slice(),
        )
        // Unreachable past the line above, which is the same spelling on the
        // same two arguments: kept as the same rewrapping the engine's other
        // call gets, so the fallible call stays honest.
        .map_err(|source| Error::Unspellable { source })?;
        if !blocking.is_empty() {
            return Err(Error::ClosedScopeBelow {
                path,
                scopes: blocking.into_iter().map(str::to_owned).collect(),
            });
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

    /// Write `scope` onto this path's entry, save, and say what the directory is
    /// scoped now.
    ///
    /// The whole of `warlock scope add` past the boundary, in the order
    /// [`scope_submit`](crate::scoping::scope_submit) does it: fold, judge,
    /// find the entry, rebuild, save. The fold is `to_ascii_lowercase` and the
    /// judge is [`validate_scope`], which is the only thing in warlock that
    /// decides what a scope may be — see the module docs for why nothing here
    /// trims or repairs.
    ///
    /// The path is spelled first because the module docs' ordering rule stops at
    /// the boundary and not at the write: a path with no manifest form is this
    /// command's own refusal, exactly as it is for an un-pact, and it is asked
    /// before anything is judged so that a run with two things wrong with it
    /// answers about where it was pointed.
    ///
    /// What comes back is the line without its `warlock: ` prefix, and it names
    /// the scope that was there before when there was a different one. A scope
    /// is somebody's boundary and moving one silently is the thing the
    /// informative success line exists to prevent — the same reasoning as
    /// [`unpacted_line`]'s, over one directory instead of a subtree.
    ///
    /// # Errors
    ///
    /// [`Error::Unspellable`] for a path with no repository-relative form,
    /// [`Error::Scope`] for a string that is not a scope, [`Error::NoPact`] for
    /// a directory the manifest has no entry for, and [`Error::Manifest`] for a
    /// manifest that will not save. Nothing at all is written for the first
    /// three, and the save is a write beside and a rename over, so there is no
    /// half-written state to leave behind for the fourth.
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

    /// Clear the scope on this path's entry, save, and say what was cleared.
    ///
    /// `warlock scope remove` past the boundary, and the same rebuild with a
    /// `None` in it: [`PactEntry::without_scope`] takes the one field a person
    /// owns and leaves the document, the granted hash and the granted timestamp
    /// exactly as the run left them.
    ///
    /// A directory carrying no scope is success and not a refusal, and the save
    /// still happens — one road through this function, and what it writes is a
    /// manifest identical to the one it read. That is the un-pact's decision
    /// about an empty repository, for the same reason: a second, quieter road
    /// through a write is a thing a caller then has to reason about.
    ///
    /// # Errors
    ///
    /// [`Error::Unspellable`], [`Error::NoPact`] and [`Error::Manifest`], as
    /// [`Opened::scoped`] refuses them. There is no [`Error::Scope`] here
    /// because there is nothing to judge: clearing is not a scope.
    fn unscoped(&self) -> Result<String, Error> {
        let module = spelled(&self.repo_root, &self.target)?;
        let was = self.scope_on(&module)?.map(str::to_owned);
        with_scope_on(&self.manifest, &module, None)
            .save(&self.repo_root)
            .map_err(|source| Error::Manifest { source })?;

        Ok(unscoped_line(&module, was.as_deref()))
    }

    /// The scope the entry for `module` carries now, or `None` when it carries
    /// none.
    ///
    /// The existence check and the "what was there before" both, because they
    /// are one look at one entry: a scope write has to know whether there is an
    /// entry to write on, and the success line has to know what it replaced.
    ///
    /// # Errors
    ///
    /// [`Error::NoPact`], naming the directory, when the manifest has no entry
    /// for it. Only ever reached past an open boundary — this is a fact about
    /// what the manifest holds, and the gate above is what keeps it from being
    /// asked from outside a scope this machine does not open.
    fn scope_on(&self, module: &str) -> Result<Option<&str>, Error> {
        self.manifest
            .entry(module)
            .map(PactEntry::scope)
            .ok_or_else(|| Error::NoPact {
                module: module.to_owned(),
            })
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
/// `eprintln!`: a 3 when the boundary over the path is closed to this machine,
/// and a 1 for everything else it can refuse.
///
/// # Errors
///
/// Everything [`opened`] and [`Opened::unpacted`] refuse, unchanged and
/// unwrapped: this adds no sentence of its own.
pub(crate) fn unpact(path: &Path) -> Result<(), Error> {
    println!("warlock: {}", opened(FOR_UNPACT, path)?.unpacted()?);
    Ok(())
}

/// `warlock scope add <path> <scope>`: write that scope onto that directory's
/// pact, and say what it is scoped now.
///
/// Two lines for [`unpact`]'s reasons, over the same two halves: [`opened`] is
/// the boundary and the resolution, [`Opened::scoped`] is the judging, the edit
/// and the sentence.
///
/// # Errors
///
/// Everything [`opened`] and [`Opened::scoped`] refuse, unchanged and
/// unwrapped: this adds no sentence of its own.
pub(crate) fn scope_add(path: &Path, scope: &str) -> Result<(), Error> {
    println!("warlock: {}", opened(FOR_SCOPE_ADD, path)?.scoped(scope)?);
    Ok(())
}

/// `warlock scope remove <path>`: clear the scope on that directory's pact, and
/// say what was cleared.
///
/// # Errors
///
/// Everything [`opened`] and [`Opened::unscoped`] refuse, unchanged and
/// unwrapped.
pub(crate) fn scope_remove(path: &Path) -> Result<(), Error> {
    println!("warlock: {}", opened(FOR_SCOPE_REMOVE, path)?.unscoped()?);
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

/// What a scope write on `module` says it did, having replaced `was`.
///
/// The fact first — this directory is scoped that — because it is the state the
/// reader asked for and the one a second run would find. The scope that was
/// there before comes after it and only when there was a different one: moving
/// somebody's boundary is worth saying out loud, for [`unpacted_line`]'s reason,
/// while re-writing the scope a directory already carried is a no-op nobody
/// needs a clause about.
///
/// The scope is backticked the way [`closed_scope_message`](crate::session) and
/// `warlock config` both spell one, so a sigil reads the same wherever warlock
/// prints it.
fn scoped_line(module: &str, scope: &str, was: Option<&str>) -> String {
    match was {
        Some(was) if was != scope => format!("{module} is scoped `{scope}` — was `{was}`"),
        _ => format!("{module} is scoped `{scope}`"),
    }
}

/// What a scope clear on `module` says it did, having cleared `was`.
///
/// Two sentences, because there are two things that can have happened and a
/// reader is owed the difference: a boundary that was there and is not any more,
/// named so that the run is auditable, and a directory that carried no scope to
/// begin with — which is success, exits 0, and says so rather than implying a
/// removal nobody performed.
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
    use crate::session::{closed_scope_message, load_manifest, sigils_under};
    use crate::status_for;

    /// The grant every entry below carries, so that "the scope write left the
    /// run's own fields alone" is an assertion about two values that are really
    /// there.
    const HASH: &str = "d0f5a1";

    /// When that grant happened, in the form the manifest stores.
    const AT: &str = "2026-08-19T07:32:00Z";

    /// A throwaway directory. Every test here builds both its repository and
    /// its home out of one of these, so nothing goes near the developer's real
    /// home or a real repository.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// An entry for `module`, documented and granted the way a pact leaves one.
    ///
    /// Granted rather than bare, because a scope write promises to leave the run's
    /// own fields where it found them and a promise about a hash needs a hash to
    /// be about. An un-pact drops whole entries, so it neither knows nor cares.
    fn entry(module: &str) -> PactEntry {
        PactEntry::new(".", module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root")
            .with_grant(HASH, AT)
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

    /// `warlock scope add <path> <scope>`, on the production road: the boundary
    /// through [`Opened::new`] and then the write, with no way to reach the
    /// second without the first.
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

    /// `warlock scope remove <path>`, the same way.
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

    /// The entry stored for `module` in the manifest on disk under `repo_root`.
    fn stored(repo_root: &Path, module: &str) -> PactEntry {
        load_manifest(repo_root)
            .expect("a manifest that reads")
            .entry(module)
            .expect("the manifest holds this module")
            .clone()
    }

    /// The engine's own sentence about why `text` is not a scope: asked of the
    /// one judge rather than retyped here, so a test cannot agree with a wording
    /// warlock no longer uses.
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

    /// The scope this machine's one sigil opens, on both doors and in every
    /// case below.
    const HELD: &str = "platform";

    /// The scope it does not, which is the one every refusal here is by.
    const CLOSED: &str = "data-plane";

    /// One manifest holding every shape the two doors have to answer alike: a
    /// root entry carrying no scope, a boundary this machine holds on `crates`,
    /// one it does not on `crates/engine` below that, and a second subtree whose
    /// only boundary below is one it does hold.
    ///
    /// One manifest rather than one per case, because a parity test over several
    /// fixtures would be showing that the two doors agree about several
    /// different repositories.
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

    /// A repository holding that manifest, and a home holding the one sigil,
    /// both throwaway.
    ///
    /// No documents on disk: what these tests are about is which un-pacts are
    /// allowed, and neither door reads a `WARLOCK.md` to decide that. The
    /// promise that an un-pact leaves every document where it was is pinned
    /// above, over a repository that has them.
    fn a_repository_of_boundaries() -> (tempfile::TempDir, tempfile::TempDir) {
        let repo = a_dir();
        let home = a_dir();
        a_manifest_of_boundaries_both_ways()
            .save(repo.path())
            .expect("a manifest that saves");
        holding(home.path(), repo.path(), &[HELD]);

        (repo, home)
    }

    /// The panel over that repository: every directory the manifest names, each
    /// one pacted, so that `p` on any of its rows is an un-pact.
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

    /// What one door said about one un-pact: it went ahead, or it was refused
    /// with this sentence.
    ///
    /// The answer, and deliberately not the mechanism. The panel refuses by
    /// painting nothing and putting a line on the footer, the shell by handing
    /// `main` an error to print; those are two shapes of one rule, and this is
    /// what the two of them have to be equal in.
    #[derive(Debug, PartialEq, Eq)]
    enum Answer {
        WentAhead,
        Refused(String),
    }

    /// `sentence` with the panel's name for the repository root written the way
    /// the shell writes it.
    ///
    /// The one difference between the doors that is not about the rule:
    /// [`App::label_for`](warlock_tui::App::label_for) spells a row relative to
    /// the tree's root and falls back to the absolute path for the root row
    /// itself, where the shell spells that row `.`. Every other row is named by
    /// the manifest's own spelling on both sides, so this is a no-op for them.
    fn as_the_shell_says_it(sentence: &str, repo_root: &Path) -> String {
        sentence.replace(&repo_root.display().to_string(), ".")
    }

    /// What the `p` key answers about un-pacting `path`, pressed on that row of
    /// [`a_panel_over`].
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

    /// What `warlock unpact <path>` answers about the same directory, on the
    /// production road: the boundary through [`Opened::new`], the edit through
    /// [`Opened::unpacted`].
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
