use crate::error::Error;
use crate::settings::Settings;
use crate::standing::FOR_SIGILS;
use crate::submission::Sigiling;

pub fn sigil_list() -> Result<(), Error> {
    sigil(&Sigiling::Show)
}

pub fn sigil_add(names: &[String]) -> Result<(), Error> {
    sigil(&Sigiling::Add(names.to_vec()))
}

pub fn sigil_remove(names: &[String]) -> Result<(), Error> {
    sigil(&Sigiling::Remove(names.to_vec()))
}

pub fn sigil_clear() -> Result<(), Error> {
    sigil(&Sigiling::Clear)
}

fn sigil(sigiling: &Sigiling) -> Result<(), Error> {
    let settings = Settings::here(FOR_SIGILS)?;
    settings.print(&settings.sigil(sigiling)?);
    Ok(())
}
