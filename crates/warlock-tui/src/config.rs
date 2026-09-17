//! `warlock config`: the sigils this machine holds for this repository. A scope
//! is committed in `.warlock/pacts.toml` and read by everyone who clones the
//! repository; a sigil is what one person holds on one machine, so nothing here
//! writes, or offers to write, a file inside the checkout.
//!
//! One line in and one file out, with everything the answer turns on printed
//! above the cursor — including that a blank line clears the set. That is what
//! buys the single entry point: no `warlock config clear`, no flag, no second
//! spelling, and so no argument parser and no line editor here. EOF is the one
//! answer that writes nothing, told apart from a blank line in [`read_line`]
//! rather than anywhere below it. Ctrl-C needs no code at all, because this
//! subcommand never enters raw mode and installs no panic hook.

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use warlock_engine::{load_sigils, save_sigils, sigils, sigils_path, validate_sigil};

use crate::error::{Error, one_line};
use crate::standing::{FOR_SIGILS, Standing};

const PROMPT: &str = "sigils (separated by spaces)> ";

const NOTHING: &str = "nothing";

// A second statement of `validate_sigil`'s rules, written out because that
// function judges strings and has nothing to say about itself. This is the
// honest place for the duplicate: it is what a person reads before typing, and
// the alternative is a prompt that says nothing and refuses afterwards. The
// wildcard is last because it is the one thing a sigil may be that a scope may
// not.
const RULES: &str = "a sigil is 1 to 24 characters of lowercase letters, digits, `-` and \
                     `_`, begins with a letter and does not end with `-` or `_`; `*` on \
                     its own means anywhere";

pub(crate) fn configure() -> Result<(), Error> {
    let standing = Standing::here(FOR_SIGILS)?;
    // The one subcommand that takes the error rather than `.ok()`: a home is the
    // thing it was asked to write under, so not having one is a failure and not
    // an answer of "nothing held".
    let home = Standing::home()?;

    prompted(&standing, &home, read_line, &mut io::stdout())
}

// Split from `configure` so the order is something a test can run: `ask` is a
// canned answer under test and `out` collects what a reader would have seen.
//
// That order is the part worth pinning. The preamble is flushed before anything
// is read, because the prompt carries no newline and would otherwise sit in the
// terminal's buffer behind a cursor waiting on a person; EOF is answered before
// anything is parsed; and the confirmation names the file only after `hold`
// has written it.
fn prompted<W: Write>(
    standing: &Standing,
    home: &Path,
    ask: impl FnOnce() -> Result<Option<String>, Error>,
    out: &mut W,
) -> Result<(), Error> {
    let root = standing.repo_root();
    let path = sigils_path(home, root);

    drop(write!(
        out,
        "{}",
        preamble(root, &path, &held_for(home, root))
    ));
    // Best effort, and the only thing that could be done about it: the prompt
    // has no newline of its own, so it sits in the terminal's buffer until this
    // pushes it out. A stdout that will not flush has nothing useful to say
    // about itself, and the read below reports anything that really goes wrong.
    drop(out.flush());

    let Some(line) = ask()? else {
        // EOF, which is Ctrl-D at a terminal and an empty pipe everywhere else.
        // Nothing is parsed and nothing is written: a missing file stays missing
        // and an existing one is not opened. The newline is because the prompt
        // above has none and the cursor is still sitting on it.
        drop(writeln!(out, "\nwarlock: nothing changed"));
        return Ok(());
    };

    let sigils = hold(home, root, &line)?;
    drop(writeln!(
        out,
        "warlock: holding {} for `{}`, written to `{}`",
        holding(&sigils),
        root.display(),
        path.display()
    ));
    Ok(())
}

// Two answers rather than three, and the join is deliberate: a config that is
// not there and a config holding an empty set both read as nothing held,
// because the line about to be typed replaces either of them the same way. A
// file that is there and cannot be read has to stay apart from those — it is
// the one case where what this prints is not what is on disk, and the reader is
// about to be offered a line that would overwrite it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Held {
    Sigils(Vec<String>),
    Unreadable(String),
}

// A read that goes wrong is a state to print rather than a way out: the job
// here is to *set* the sigils, and a config that cannot be read is the
// situation a reader most needs to be able to type over.
fn held_for(home: &Path, root: &Path) -> Held {
    match load_sigils(home, root) {
        Ok(sigils) => Held::Sigils(sigils),
        Err(sigils::Error::NotFound { .. }) => Held::Sigils(Vec::new()),
        Err(error) => Held::Unreadable(one_line(&error.to_string())),
    }
}

// Pure, and it ends *without* a newline, because the last thing it composes is
// the line the reader types on. The order is fixed by what the reader needs
// before they can answer: what this is about, what it is now, what a legal
// answer looks like, and only then what their answer will do — the destructive
// one named as plainly as the other two.
fn preamble(root: &Path, path: &Path, held: &Held) -> String {
    // One `format!` rather than a line at a time, so what is on the screen is
    // read here in the order it is printed in.
    format!(
        "sigils for `{root}`\n\
         stored at `{path}`\n\
         holding {held}\n\
         {RULES}\n\
         a line of sigils replaces everything held for this repository\n\
         a blank line clears it\n\
         Ctrl-C or EOF changes nothing\n\
         {PROMPT}",
        root = root.display(),
        path = path.display(),
    )
}

impl fmt::Display for Held {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sigils(sigils) => write!(f, "{}", holding(sigils)),
            // Said as what it is, so a broken file is never printed as an empty
            // one: the two look identical afterwards and mean opposite things
            // about what was on disk beforehand.
            Self::Unreadable(reason) => write!(f, "unknown: {reason}"),
        }
    }
}

// One wording, used before the line is typed and after it is written, so that
// running `warlock config` twice and typing the same line the second time
// prints back exactly what the first run printed.
fn holding(sigils: &[String]) -> String {
    if sigils.is_empty() {
        return NOTHING.to_owned();
    }

    sigils
        .iter()
        .map(|sigil| format!("`{sigil}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

// Whitespace separates the words and nothing else does, which is what makes the
// trailing newline vanish rather than become a token and a line of only spaces
// the same answer as an empty one. The empty set that comes back from a blank
// line is written like any other: clearing is a set replaced, not a file
// deleted.
//
// The first word that fails ends the whole line, because a set is replaced
// entirely and writing the words before a typo would leave the reader holding
// half of what they typed.
fn sigils_in(line: &str) -> Result<Vec<String>, Error> {
    let mut sigils: Vec<String> = Vec::new();
    for word in line.split_whitespace() {
        // `to_ascii_lowercase` rather than `to_lowercase`, because the character
        // class a sigil is drawn from is ASCII: folding a non-ASCII capital
        // would produce a character that is refused on the next line anyway, and
        // this way the word that is refused is the word that was typed.
        let sigil = word.to_ascii_lowercase();
        // The engine's sibling entry point, which accepts `*` and otherwise
        // defers to `validate_scope`. The asymmetry — `*` is a sigil and never a
        // directory scope, because blank already means "open to anyone" there
        // while `*` is the only way to say "anywhere" here — lives in
        // `validate_sigil`'s own doc, so this call site needs no exception of
        // its own and there is still exactly one character class in the
        // workspace.
        validate_sigil(&sigil).map_err(|rule| Error::Sigil {
            entered: word.to_owned(),
            rule,
        })?;
        if !sigils.contains(&sigil) {
            sigils.push(sigil);
        }
    }
    Ok(sigils)
}

// `home` is a parameter all the way down to the engine, which is what lets this
// be tested against a temporary directory and keeps any test off the
// developer's real home. Nothing is written unless the whole line is sigils:
// `sigils_in` runs first and returns before the save is reached.
fn hold(home: &Path, root: &Path, line: &str) -> Result<Vec<String>, Error> {
    let sigils = sigils_in(line)?;
    save_sigils(home, root, &sigils).map_err(|source| Error::Sigils { source })?;
    Ok(sigils)
}

// `Ok(0)` is EOF and nothing else. It is told apart from an empty line here
// rather than further down, because everything below treats a line as text and
// only this function can tell "they pressed Enter" from "there is no line and
// never will be".
fn read_line() -> Result<Option<String>, Error> {
    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(0) => Ok(None),
        Ok(_) => Ok(Some(line)),
        Err(source) => Err(Error::Prompt { source }),
    }
}

#[cfg(test)]
#[path = "tests/config.rs"]
mod tests;
