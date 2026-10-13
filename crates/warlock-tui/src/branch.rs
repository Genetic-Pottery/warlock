use crate::error::Error;
use crate::git::Git;
use crate::settings::Settings;
use crate::standing::FOR_BRANCH;
use crate::submission::Branching;

pub fn branch_show() -> Result<(), Error> {
    branch(&Branching::Show)
}

pub fn branch_use(name: &str) -> Result<(), Error> {
    branch(&Branching::Use(name.to_owned()))
}

pub fn branch_clear() -> Result<(), Error> {
    branch(&Branching::Clear)
}

// `remote_head` and not `default_branch`, for the panel's reason: this names the
// remote's default the way the panel's title does, and asking the remote itself
// is a round trip a one-line answer should not wait on. A `use` names its own
// branch, so the remote's default never reaches its line and is not looked up.
fn branch(branching: &Branching) -> Result<(), Error> {
    let settings = Settings::here(FOR_BRANCH)?;
    let remote_head = match branching {
        Branching::Use(_) => None,
        Branching::Show | Branching::Clear => Git::at(settings.root()).remote_head(),
    };
    settings.print(&settings.branch(branching, remote_head.as_deref())?);
    Ok(())
}
