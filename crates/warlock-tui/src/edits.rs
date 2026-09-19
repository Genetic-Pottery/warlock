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
use crate::scoping::{records_scope, with_scope_on, with_scope_recorded};
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
    // about where it was pointed. `scope_on` — the existence check — comes next
    // and stays above everything about the flags: three values typed for a
    // directory nobody has pacted would never have been written whatever they
    // said, so the pact is the refusal a reader is owed.
    //
    // Which of the two roads the flags then take is [`records_scope`]'s answer
    // and nothing else's: the same comparison
    // [`route_facts`](warlock_engine::route_facts) routes by and the `s` key
    // consults. A lookup of its own here that folded or trimmed could send a
    // name that already routes down the recording road, to write a second
    // record the router never reads.
    fn scoped(&self, scope: &str, flags: Flags<'_>) -> Result<String, Error> {
        let module = spelled(&self.repo_root, &self.target)?;
        // `to_ascii_lowercase` rather than `to_lowercase`, for `scope_submit`'s
        // reason: a scope is drawn from ASCII, so folding a non-ASCII capital
        // would produce a character the judge refuses anyway, and this way what
        // is refused is closer to what was typed.
        let folded = scope.to_ascii_lowercase();
        validate_scope(&folded).map_err(|rule| Error::Scope { rule })?;
        let was = self.scope_on(&module)?.map(str::to_owned);

        // Both roads end in one manifest and one save, which is the whole point
        // of building the thing before writing it: a scope on disk whose record
        // failed to write is the half-state [`with_scope_recorded`] exists to
        // make impossible, and a second save here would put it back.
        let next = if records_scope(&self.manifest, &folded) {
            flags.unwanted(&folded)?;
            with_scope_on(&self.manifest, &module, Some(&folded))
        } else {
            let record = flags.record(&folded)?;
            with_scope_recorded(
                &self.manifest,
                &module,
                &folded,
                record.team,
                record.review_state,
                record.label,
            )
            // Unreachable: this road is taken because nothing records the
            // name. Asked rather than unwrapped, because a record that
            // appeared in between deserves the refusal a flag at a recorded
            // name gets, and somebody else's edit does not deserve a panic.
            .ok_or_else(|| Error::RecordedScope {
                scope: folded.clone(),
            })?
        };
        self.saved(&next)?;

        Ok(scoped_line(&module, &folded, was.as_deref()))
    }

    // A directory carrying no scope is success and not a refusal, and the save
    // still happens: one road through this function, and what it writes is a
    // manifest identical to the one it read. A second, quieter road through a
    // write is a thing a caller then has to reason about.
    fn unscoped(&self) -> Result<String, Error> {
        let module = spelled(&self.repo_root, &self.target)?;
        let was = self.scope_on(&module)?.map(str::to_owned);
        self.saved(&with_scope_on(&self.manifest, &module, None))?;

        Ok(unscoped_line(&module, was.as_deref()))
    }

    // Every question is behind us by the time this is called, which is the
    // ordering the two writes above share: whatever they refuse, they refuse
    // with nothing written.
    fn saved(&self, next: &Manifest) -> Result<(), Error> {
        next.save(&self.repo_root)
            .map_err(|source| Error::Manifest { source })
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

// The three record flags exactly as clap left them: absent, or a value with
// nothing done to it. Nothing is judged in here — whether they are required,
// forbidden or blank is a fact about what the manifest already records, so the
// questions are asked from inside [`Opened`] and never before it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Flags<'a> {
    pub(crate) team: Option<&'a str>,
    pub(crate) review_state: Option<&'a str>,
    pub(crate) label: Option<&'a str>,
}

impl<'a> Flags<'a> {
    // In the record window's field order — team, review state, label — so the
    // shell and the panel complain about the same three things in the same
    // order, and the flag names live in exactly one place.
    fn given(self) -> [(&'static str, Option<&'a str>); 3] {
        [
            ("--team", self.team),
            ("--review-state", self.review_state),
            ("--label", self.label),
        ]
    }

    fn named(self, wrong: impl Fn(Option<&str>) -> bool) -> Vec<&'static str> {
        self.given()
            .iter()
            .filter(|(_, value)| wrong(*value))
            .map(|(flag, _)| *flag)
            .collect()
    }

    // The road for a name nothing records: all three or none of it. The tuple
    // match is the check and the unwrapping both, so there is no second reading
    // of `None` further down that could disagree with the refusal above it.
    //
    // Blank is judged on a trimmed copy while the untrimmed string is what gets
    // stored, exactly as [`record_submit`](crate::scoping::record_submit) does
    // it: a team, a review state and a label belong to somebody's tracker, and
    // warlock is in no position to correct their spelling.
    fn record(self, scope: &str) -> Result<Record<'a>, Error> {
        let (Some(team), Some(review_state), Some(label)) =
            (self.team, self.review_state, self.label)
        else {
            return Err(Error::UnrecordedScope {
                scope: scope.to_owned(),
                missing: self.named(|value| value.is_none()),
            });
        };

        let blank = self.named(|value| value.is_some_and(|value| value.trim().is_empty()));
        if !blank.is_empty() {
            return Err(Error::BlankRecord { flags: blank });
        }

        Ok(Record {
            team,
            review_state,
            label,
        })
    }

    // The road for a name something already records, where the only acceptable
    // answer is no flags at all: warlock does not rewrite, merge or delete a
    // record, so a value handed to one would be a value dropped on the floor.
    fn unwanted(self, scope: &str) -> Result<(), Error> {
        if self.given().iter().any(|(_, value)| value.is_some()) {
            return Err(Error::RecordedScope {
                scope: scope.to_owned(),
            });
        }

        Ok(())
    }
}

// The three values past the point where `None` is possible. A struct rather
// than a tuple of three strings, because three fields of one type in a row is
// somewhere to swap two of them without the compiler minding.
#[derive(Debug, Clone, Copy)]
struct Record<'a> {
    team: &'a str,
    review_state: &'a str,
    label: &'a str,
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

pub(crate) fn scope_add(path: &Path, scope: &str, flags: Flags<'_>) -> Result<(), Error> {
    println!(
        "warlock: {}",
        opened(FOR_SCOPE_ADD, path)?.scoped(scope, flags)?
    );
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
#[path = "tests/edits.rs"]
mod tests;
