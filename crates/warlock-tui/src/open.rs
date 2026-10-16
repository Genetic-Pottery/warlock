use crate::error::Error;
use crate::settings::Settings;
use crate::standing::FOR_OPEN;
use crate::submission::Opening;

pub fn open_list() -> Result<(), Error> {
    open(&Opening::Show)
}

pub fn open_scopes(names: &[String]) -> Result<(), Error> {
    open(&Opening::Open(names.to_vec()))
}

pub fn close_scopes(names: &[String]) -> Result<(), Error> {
    open(&Opening::Close(names.to_vec()))
}

fn open(opening: &Opening) -> Result<(), Error> {
    let settings = Settings::here(FOR_OPEN)?;
    settings.print(&settings.open(opening)?);
    Ok(())
}
