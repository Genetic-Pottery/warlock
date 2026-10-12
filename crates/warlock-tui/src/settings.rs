use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use warlock_engine::{
    held_base_branch, held_sigils, save_base_branch, save_sigils, sigils_path, validate_sigil,
};

use crate::app::{Held, Sigils, Target};
use crate::cut::listed;
use crate::error::{Error, say};
use crate::standing::Standing;
use crate::submission::{Branching, Sigiling};

/// This checkout's settings on this machine: the sigils held and the branch
/// pulls start from, both in the one config file under the home.
///
/// The CLI's `warlock sigil` and `warlock branch` and the chat bar's `/sigil`
/// and `/branch` all go through here, so the two doors read, write and word
/// the setting the same way.
pub(crate) struct Settings {
    home: Option<PathBuf>,
    root: PathBuf,
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
    // No home is nothing held rather than a failure: a reader of the tree has no
    // use for one, and the writes refuse with `Error::NoHome` when asked for.
    pub(crate) fn at(home: Option<PathBuf>, root: impl Into<PathBuf>) -> Self {
        Self {
            home,
            root: root.into(),
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

    fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    pub(crate) fn config(&self) -> Option<PathBuf> {
        self.home().map(|home| sigils_path(home, &self.root))
    }

    /// Read whole, and never an error: what the chrome draws and what the
    /// boundary checks in the panel go by.
    pub(crate) fn snapshot(&mut self) -> Held {
        self.stamp = Stamp::Read(self.modified());
        Held {
            sigils: self.sigils_or_unknown(),
            base: self
                .home()
                .and_then(|home| held_base_branch(home, &self.root).ok().flatten()),
        }
    }

    // A stat each round rather than a read: the file is opened only when its
    // modification time has moved since the last read, which is how a `warlock
    // sigil` run in another shell reaches a warlock already open.
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
    /// is refused here rather than taken for holding nothing.
    pub(crate) fn sigils(&self) -> Result<Vec<String>, Error> {
        let home = self.home().ok_or(Error::NoHome)?;
        held_sigils(home, &self.root).map_err(|source| Error::Sigils { source })
    }

    pub(crate) fn base(&self) -> Result<Option<String>, Error> {
        let Some(home) = self.home() else {
            return Ok(None);
        };
        held_base_branch(home, &self.root).map_err(|source| Error::Sigils { source })
    }

    // Every name is judged before anything is written, so a typo in the third
    // name of an `add` leaves the first two unwritten too rather than half done.
    // A config that will not read is refused rather than written over: `add` and
    // `remove` are edits of what is there, and there is no telling what is there.
    pub(crate) fn sigil(&self, sigiling: &Sigiling) -> Result<Said, Error> {
        let mut held = self.sigils()?;
        match sigiling {
            Sigiling::Show => return Ok(Said::Shown(sigils_line(&Sigils::held(held), None))),
            Sigiling::Add(names) => {
                for name in names {
                    let sigil = folded(name)?;
                    if !held.contains(&sigil) {
                        held.push(sigil);
                    }
                }
            }
            Sigiling::Remove(names) => {
                for name in names {
                    let sigil = folded(name)?;
                    let Some(at) = held.iter().position(|kept| *kept == sigil) else {
                        return Err(Error::UnheldSigil { sigil });
                    };
                    held.remove(at);
                }
            }
            Sigiling::Clear => held.clear(),
        }
        let home = self.home().ok_or(Error::NoHome)?;
        save_sigils(home, &self.root, &held).map_err(|source| Error::Sigils { source })?;
        Ok(Said::Wrote(sigils_line(&Sigils::held(held), None)))
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
        save_base_branch(home, &self.root, base).map_err(|source| Error::Sigils { source })?;
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
        let held = self.sigils()?;
        if held.is_empty() {
            return Ok(
                "`/pull` takes a scope, and this machine holds no sigil at all — \
                       `/sigil add` holds one"
                    .to_owned(),
            );
        }
        Ok(format!(
            "`/pull` takes a scope: this machine holds {}",
            listed(&held)
        ))
    }

    // Never an error: what a machine holds is a line on a header, and warlock is
    // a way of reading a tree. A config that will not parse becomes
    // `Sigils::Unknown` — said out loud, so that broken is never drawn as absent
    // — and nothing here can return upwards to end the event loop.
    // [`Settings::sigils`], and the engine's filing and routing through
    // `held_sigils`, read the same file and refuse instead; the difference is
    // theirs to keep.
    //
    // No home reads as nothing held rather than as a config that would not read:
    // there is no file in that case and no path to name one by, so
    // `Sigils::Unknown` would be claiming something on disk is broken when
    // nothing on disk was ever looked at.
    pub(crate) fn sigils_or_unknown(&self) -> Sigils {
        self.home().map_or(Sigils::Nothing, |home| {
            held_sigils(home, &self.root).map_or(Sigils::Unknown, Sigils::held)
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

/// What is held, as one line: the one wording for the panel, `warlock sigil`
/// and `warlock check`. `config` is named only when the file would not read.
pub(crate) fn sigils_line(sigils: &Sigils, config: Option<&Path>) -> String {
    match (sigils, config) {
        (Sigils::Held(held), _) => format!("holding {}", holding(held)),
        (Sigils::Nothing, _) => "holding nothing".to_owned(),
        (Sigils::Unknown, Some(path)) => {
            format!("holding unknown: `{}` could not be read", path.display())
        }
        (Sigils::Unknown, None) => "holding unknown".to_owned(),
    }
}

fn holding(sigils: &[String]) -> String {
    if sigils.is_empty() {
        return "nothing".to_owned();
    }
    listed(sigils)
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

#[cfg(test)]
#[path = "tests/settings.rs"]
mod tests;
