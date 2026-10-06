use std::io::{self, Write};
use std::path::Path;

use warlock_engine::{save_base_branch, sigils_path};

use crate::error::Error;
use crate::standing::{FOR_BRANCH, Standing};

pub fn branch_use(name: &str) -> Result<(), Error> {
    let standing = Standing::here(FOR_BRANCH)?;
    let home = Standing::home()?;

    set(&home, standing.repo_root(), Some(name), &mut io::stdout())
}

pub fn branch_clear() -> Result<(), Error> {
    let standing = Standing::here(FOR_BRANCH)?;
    let home = Standing::home()?;

    set(&home, standing.repo_root(), None, &mut io::stdout())
}

fn set<W: Write>(home: &Path, root: &Path, branch: Option<&str>, out: &mut W) -> Result<(), Error> {
    save_base_branch(home, root, branch).map_err(|source| Error::Sigils { source })?;

    let target = branch.map_or_else(
        || "the remote's default branch".to_owned(),
        |name| format!("`{name}`"),
    );
    drop(writeln!(
        out,
        "warlock: pulls in `{}` start from and open pull requests against {target}, written to `{}`",
        root.display(),
        sigils_path(home, root).display()
    ));
    Ok(())
}

#[cfg(test)]
#[path = "tests/branch.rs"]
mod tests;
