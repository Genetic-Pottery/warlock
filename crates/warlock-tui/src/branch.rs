use std::io::{self, Write};

use crate::error::Error;
use crate::git::Git;
use crate::settings::Settings;
use crate::standing::{FOR_BRANCH, Standing};
use crate::submission::Branching;

pub fn branch_show() -> Result<(), Error> {
    let (settings, remote_head) = here()?;
    let line = settings.branch(&Branching::Show, remote_head.as_deref())?;
    drop(writeln!(io::stdout(), "warlock: {line}"));
    Ok(())
}

pub fn branch_use(name: &str) -> Result<(), Error> {
    changed_here(&Branching::Use(name.to_owned()))
}

pub fn branch_clear() -> Result<(), Error> {
    changed_here(&Branching::Clear)
}

fn changed_here(change: &Branching) -> Result<(), Error> {
    let (settings, remote_head) = here()?;
    let line = settings.branch(change, remote_head.as_deref())?;
    drop(writeln!(
        io::stdout(),
        "warlock: {}",
        settings.written(&line)
    ));
    Ok(())
}

// `remote_head` and not `default_branch`, for the panel's reason: this names the
// remote's default the way the panel's title does, and asking the remote itself
// is a round trip a one-line answer should not wait on.
fn here() -> Result<(Settings, Option<String>), Error> {
    let standing = Standing::here(FOR_BRANCH)?;
    let home = Standing::home()?;
    let root = standing.repo_root();
    Ok((Settings::at(Some(home), root), Git::at(root).remote_head()))
}
