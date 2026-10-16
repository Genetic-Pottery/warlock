use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use warlock_engine::{
    config_path, held_base_branch, open_scopes, save_base_branch, save_open_scopes,
    validate_open_scope,
};

use crate::app::{Held, OpenScopes, Target};
use crate::cut::listed;
use crate::error::{Error, say};
use crate::standing::Standing;
use crate::submission::{Branching, Opening};

/// This checkout's settings on this machine: the open scopes and the branch
/// pulls start from, both in the one config file under the home.
///
/// The CLI's `warlock open`, `warlock close` and `warlock branch` and the chat
/// bar's `/open`, `/close` and `/branch` all go through here, so the two doors
/// read, write and word the setting the same way.
pub(crate) struct Settings {
    home: Option<PathBuf>,
    root: PathBuf,
    // Derived once: the derivation canonicalizes and hashes the root, and
    // `changed` stats this path every frame.
    config: Option<PathBuf>,
    stamp: Stamp,
}

// The file's modification time at the last read. `Unread` and not a missing
// time, because a file that is not there has no time either and the first
// `changed` still has to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stamp {
    Unread,
    Read(Option<SystemTime>),
}

impl Settings {
    // No home opens no scope rather than failing: a reader of the tree has no
    // use for one, and the writes refuse with `Error::NoHome` when asked for.
    pub(crate) fn at(home: Option<PathBuf>, root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let config = home.as_deref().map(|home| config_path(home, &root));
        Self {
            home,
            root,
            config,
            stamp: Stamp::Unread,
        }
    }

    pub(crate) fn here(wanted: &'static str) -> Result<Self, Error> {
        let standing = Standing::here(wanted)?;
        Ok(Self::at(Some(Standing::home()?), standing.repo_root()))
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The home this checkout's settings were read under, for the one caller
    /// outside this file that writes something else into the same config: the
    /// key window's binding. Read off here rather than resolved a second time,
    /// for the reason every write below takes it from this field — a home
    /// cannot move under a running warlock, and a second resolution could name
    /// a different machine's config from the one the chrome is drawn off.
    pub(crate) fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    pub(crate) fn config(&self) -> Option<&Path> {
        self.config.as_deref()
    }

    /// Read whole, and never an error: what the chrome draws and what the
    /// boundary checks in the panel go by.
    pub(crate) fn snapshot(&mut self) -> Held {
        self.stamp = Stamp::Read(self.modified());
        Held {
            open: self.open_or_unknown(),
            base: self
                .home()
                .and_then(|home| held_base_branch(home, &self.root).ok().flatten()),
        }
    }

    // A stat each round rather than a read: the file is opened only when its
    // modification time has moved since the last read, which is how a `warlock
    // open` run in another shell reaches a warlock already running.
    pub(crate) fn changed(&mut self) -> Option<Held> {
        if self.stamp == Stamp::Read(self.modified()) {
            return None;
        }
        Some(self.snapshot())
    }

    fn modified(&self) -> Option<SystemTime> {
        let config = self.config()?;
        fs::metadata(config)
            .and_then(|metadata| metadata.modified())
            .ok()
    }

    /// Read for acting on rather than for drawing: a config that will not read
    /// is refused here rather than taken for opening no scope.
    pub(crate) fn open_scopes(&self) -> Result<Vec<String>, Error> {
        let home = self.home().ok_or(Error::NoHome)?;
        open_scopes(home, &self.root).map_err(|source| Error::Config { source })
    }

    pub(crate) fn base(&self) -> Result<Option<String>, Error> {
        let Some(home) = self.home() else {
            return Ok(None);
        };
        held_base_branch(home, &self.root).map_err(|source| Error::Config { source })
    }

    // Every name is judged before anything is written, so a typo in the third
    // name of an `open` leaves the first two unwritten too rather than half done.
    // A config that will not read is refused rather than written over: `open`
    // and `close` are edits of what is there, and there is no telling what is
    // there.
    pub(crate) fn open(&self, opening: &Opening) -> Result<Said, Error> {
        let mut open = self.open_scopes()?;
        match opening {
            Opening::Show => return Ok(Said::Shown(open_line(&OpenScopes::named(open), None))),
            Opening::Open(names) => {
                for name in names {
                    let scope = folded(name)?;
                    if !open.contains(&scope) {
                        open.push(scope);
                    }
                }
            }
            Opening::Close(names) => {
                for name in names {
                    let scope = folded(name)?;
                    let Some(at) = open.iter().position(|kept| *kept == scope) else {
                        return Err(Error::NotOpen { scope });
                    };
                    open.remove(at);
                }
            }
        }
        let home = self.home().ok_or(Error::NoHome)?;
        save_open_scopes(home, &self.root, &open).map_err(|source| Error::Config { source })?;
        Ok(Said::Wrote(open_line(&OpenScopes::named(open), None)))
    }

    /// `remote_head` is only named in the line, never resolved here.
    pub(crate) fn branch(
        &self,
        branching: &Branching,
        remote_head: Option<&str>,
    ) -> Result<Said, Error> {
        let line = |base: Option<&str>| {
            format!(
                "pulls start from and open pull requests against {}",
                Target::of(base, remote_head).sentence()
            )
        };
        Ok(match branching {
            Branching::Show => Said::Shown(line(self.base()?.as_deref())),
            Branching::Use(name) => Said::Wrote(line(self.set_base(Some(name))?.as_deref())),
            Branching::Clear => Said::Wrote(line(self.set_base(None)?.as_deref())),
        })
    }

    fn set_base(&self, base: Option<&str>) -> Result<Option<String>, Error> {
        let home = self.home().ok_or(Error::NoHome)?;
        save_base_branch(home, &self.root, base).map_err(|source| Error::Config { source })?;
        Ok(base.map(ToOwned::to_owned))
    }

    pub(crate) fn print(&self, said: &Said) {
        let line = match said {
            Said::Shown(line) => line.clone(),
            Said::Wrote(line) => self.written(line),
        };
        say(&mut io::stdout(), &line);
    }

    fn written(&self, line: &str) -> String {
        let config = self
            .config()
            .map_or_else(String::new, |path| path.display().to_string());
        format!(
            "{line} for `{}`, written to `{config}`",
            self.root.display()
        )
    }

    pub(crate) fn pull_line(&self) -> Result<String, Error> {
        let held = self.open_scopes()?;
        if held.is_empty() {
            return Ok(
                "`/pull` takes a scope, and AI can work in no scope on this machine — \
                       `/open <scope>` lets it work in one"
                    .to_owned(),
            );
        }
        Ok(format!(
            "`/pull` takes a scope: AI can work in {}",
            listed(&held)
        ))
    }

    // Never an error: the open scopes are a line in a box, and warlock is a way
    // of reading a tree. A config that will not read becomes
    // `OpenScopes::Unknown` — said out loud, so that broken is never drawn as
    // empty — and nothing here can return upwards to end the event loop.
    // [`Settings::open_scopes`], and the engine's filing and routing through
    // `open_scopes`, read the same file and refuse instead; the difference is
    // theirs to keep.
    //
    // No home reads as no scope open rather than as a config that would not read:
    // there is no file in that case and no path to name one by, so
    // `OpenScopes::Unknown` would be claiming something on disk is broken when
    // nothing on disk was ever looked at.
    pub(crate) fn open_or_unknown(&self) -> OpenScopes {
        self.home().map_or(OpenScopes::Nothing, |home| {
            open_scopes(home, &self.root).map_or(OpenScopes::Unknown, OpenScopes::named)
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Said {
    Shown(String),
    Wrote(String),
}

impl Said {
    pub(crate) fn line(&self) -> &str {
        match self {
            Self::Shown(line) | Self::Wrote(line) => line,
        }
    }
}

/// The open scopes as one line: the one wording for `/open`, `warlock open`
/// and `warlock check`. `config` is named only when the file would not read.
pub(crate) fn open_line(open: &OpenScopes, config: Option<&Path>) -> String {
    match (open, config) {
        (OpenScopes::Named(held), _) => format!("AI can work in {}", holding(held)),
        (OpenScopes::Nothing, _) => "AI can work in no scope".to_owned(),
        (OpenScopes::Unknown, Some(path)) => {
            format!(
                "where AI can work is unknown: `{}` could not be read",
                path.display()
            )
        }
        (OpenScopes::Unknown, None) => "where AI can work is unknown".to_owned(),
    }
}

fn holding(open: &[String]) -> String {
    if open.is_empty() {
        return "no scope".to_owned();
    }
    listed(open)
}

// `to_ascii_lowercase` rather than `to_lowercase`, because the character class
// a scope name is drawn from is ASCII: folding a non-ASCII capital would produce a
// character that is refused anyway, and this way the word refused is the word
// typed.
fn folded(name: &str) -> Result<String, Error> {
    let scope = name.to_ascii_lowercase();
    validate_open_scope(&scope).map_err(|rule| Error::ScopeName {
        entered: name.to_owned(),
        rule,
    })?;
    Ok(scope)
}

#[cfg(test)]
#[path = "tests/settings.rs"]
mod tests;
