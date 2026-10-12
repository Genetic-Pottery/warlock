use std::io::{self, Write};
use std::path::Path;

use warlock_engine::{held_sigils, save_sigils, sigils_path, validate_sigil};

use crate::error::Error;
use crate::standing::{FOR_SIGILS, Standing};

const NOTHING: &str = "nothing";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change<'a> {
    Add(&'a [String]),
    Remove(&'a [String]),
    Clear,
}

pub fn sigil_list() -> Result<(), Error> {
    let standing = Standing::here(FOR_SIGILS)?;
    let home = Standing::home()?;
    let held =
        held_sigils(&home, standing.repo_root()).map_err(|source| Error::Sigils { source })?;
    drop(writeln!(
        io::stdout(),
        "warlock: holding {}",
        holding(&held)
    ));
    Ok(())
}

pub fn sigil_add(names: &[String]) -> Result<(), Error> {
    changed_here(Change::Add(names))
}

pub fn sigil_remove(names: &[String]) -> Result<(), Error> {
    changed_here(Change::Remove(names))
}

pub fn sigil_clear() -> Result<(), Error> {
    changed_here(Change::Clear)
}

fn changed_here(change: Change<'_>) -> Result<(), Error> {
    let standing = Standing::here(FOR_SIGILS)?;
    let home = Standing::home()?;
    let root = standing.repo_root();
    let held = change_sigils(&home, root, change)?;
    drop(writeln!(
        io::stdout(),
        "warlock: holding {} for `{}`, written to `{}`",
        holding(&held),
        root.display(),
        sigils_path(&home, root).display()
    ));
    Ok(())
}

// Every name is judged before anything is written, so a typo in the third
// name of an `add` leaves the first two unwritten too rather than half done.
// A config that will not read is refused rather than written over: `add` and
// `remove` are edits of what is there, and there is no telling what is there.
pub(crate) fn change_sigils(
    home: &Path,
    root: &Path,
    change: Change<'_>,
) -> Result<Vec<String>, Error> {
    let mut held = held_sigils(home, root).map_err(|source| Error::Sigils { source })?;
    match change {
        Change::Add(names) => {
            for name in names {
                let sigil = folded(name)?;
                if !held.contains(&sigil) {
                    held.push(sigil);
                }
            }
        }
        Change::Remove(names) => {
            for name in names {
                let sigil = folded(name)?;
                let Some(at) = held.iter().position(|kept| *kept == sigil) else {
                    return Err(Error::UnheldSigil { sigil });
                };
                held.remove(at);
            }
        }
        Change::Clear => held.clear(),
    }
    save_sigils(home, root, &held).map_err(|source| Error::Sigils { source })?;
    Ok(held)
}

// `to_ascii_lowercase` rather than `to_lowercase`, because the character class
// a sigil is drawn from is ASCII: folding a non-ASCII capital would produce a
// character that is refused anyway, and this way the word refused is the word
// typed.
fn folded(name: &str) -> Result<String, Error> {
    let sigil = name.to_ascii_lowercase();
    validate_sigil(&sigil).map_err(|rule| Error::Sigil {
        entered: name.to_owned(),
        rule,
    })?;
    Ok(sigil)
}

pub(crate) fn holding(sigils: &[String]) -> String {
    if sigils.is_empty() {
        return NOTHING.to_owned();
    }
    sigils
        .iter()
        .map(|sigil| format!("`{sigil}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "tests/sigil.rs"]
mod tests;
