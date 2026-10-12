use std::io::{self, Write};

use crate::error::Error;
use crate::settings::Settings;
use crate::standing::{FOR_SIGILS, Standing};
use crate::submission::Sigiling;

pub fn sigil_list() -> Result<(), Error> {
    let settings = here()?;
    let line = settings.sigil(&Sigiling::Show)?;
    drop(writeln!(io::stdout(), "warlock: {line}"));
    Ok(())
}

pub fn sigil_add(names: &[String]) -> Result<(), Error> {
    changed_here(&Sigiling::Add(names.to_vec()))
}

pub fn sigil_remove(names: &[String]) -> Result<(), Error> {
    changed_here(&Sigiling::Remove(names.to_vec()))
}

pub fn sigil_clear() -> Result<(), Error> {
    changed_here(&Sigiling::Clear)
}

fn changed_here(change: &Sigiling) -> Result<(), Error> {
    let settings = here()?;
    let line = settings.sigil(change)?;
    drop(writeln!(
        io::stdout(),
        "warlock: {}",
        settings.written(&line)
    ));
    Ok(())
}

fn here() -> Result<Settings, Error> {
    let standing = Standing::here(FOR_SIGILS)?;
    let home = Standing::home()?;
    Ok(Settings::at(Some(home), standing.repo_root()))
}
