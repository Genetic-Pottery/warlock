//! `warlock key add` and `warlock key list`: the Linear keys this machine
//! holds, by name.
//!
//! The secret is taken on stdin and never in argv, because an argument is
//! readable by every process on the box for as long as the command runs and is
//! written into a shell history file afterwards. Echo suppression was rejected
//! rather than overlooked: turning the terminal's echo off means raw mode, a
//! restore on every way out and a panic hook, in the one family of subcommands
//! that deliberately touches no terminal at all — or a new dependency carrying
//! all of that inside it. What keeps a key off a screen here is the pipe, which
//! the preamble names, and `warlock key add acme < key.txt` is the documented
//! way to do it.
//!
//! No key value reaches a line printed here, an object printed here or an error
//! raised here, and none can: nothing below reads a value except the line being
//! stored, which goes straight to the engine. `keys::Error` keeps the same rule
//! from the other end — it carries paths, names and a line number, never a
//! value.

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use serde_json::Value;
use warlock_engine::{keys, keys_path, load_key_names, save_key, validate_scope};

use crate::error::{Error, one_line};
use crate::query::{envelope, write_object};
use crate::standing::Standing;

const COMMAND: &str = "key list";

const NAMES: &str = "names";

const PROMPT: &str = "key> ";

// Neither verb stands in a repository, and that is the decision rather than an
// omission: a key store is a fact about the machine and is the same store from
// inside any checkout or from none, so asking `Standing::here` for a root would
// refuse `warlock key list` in a home directory over a question that never
// needed one. The verbs that bind a name to a checkout do need it, and resolve
// it themselves.
pub(crate) fn key_add(name: &str) -> Result<(), Error> {
    let home = Standing::home()?;

    added(&home, name, read_line, &mut io::stdout())
}

pub(crate) fn key_list(json: bool) -> Result<(), Error> {
    let home = Standing::home()?;

    listed(&home, json, &mut io::stdout())
}

// Split from `key_add` so the order is something a test can run against a
// temporary home: `ask` is a canned answer under test and `out` collects what a
// reader would have seen.
//
// That order is the part worth pinning, and it is `config::prompted`'s. The
// preamble is flushed before anything is read, because the prompt carries no
// newline and would otherwise sit in the terminal's buffer behind a cursor
// waiting on a person; EOF is answered before anything is looked at; and the
// confirmation names the file only after the engine has written it.
fn added<W: Write>(
    home: &Path,
    name: &str,
    ask: impl FnOnce() -> Result<Option<String>, Error>,
    out: &mut W,
) -> Result<(), Error> {
    // Judged before the preamble is printed rather than left to the save, which
    // asks `validate_scope` again on the way in: the alternative is somebody
    // pasting a live credential at a prompt that was always going to refuse the
    // name afterwards, and a pasted key cannot be un-pasted.
    validate_scope(name).map_err(|rule| Error::KeyName {
        name: name.to_owned(),
        rule,
    })?;

    let path = keys_path(home);
    drop(write!(
        out,
        "{}",
        preamble(name, &path, &stored_under(home, name))
    ));
    // Best effort, and the only thing that could be done about it: the prompt
    // has no newline of its own, so it sits in the terminal's buffer until this
    // pushes it out. A stdout that will not flush has nothing useful to say
    // about itself, and the read below reports anything that really goes wrong.
    drop(out.flush());

    let Some(line) = ask()? else {
        // EOF, which is Ctrl-D at a terminal and an empty pipe everywhere else.
        // Nothing is stored: a missing store stays missing and an existing one
        // is not opened. The newline is because the prompt above has none and
        // the cursor is still sitting on it.
        drop(writeln!(out, "\nwarlock: nothing changed"));
        return Ok(());
    };

    // Trimmed rather than stored as it arrived: a pipe's trailing newline is
    // never part of a key, and a space at either end is a paste accident that
    // would come back as an authentication failure with nothing on screen to
    // see. It is the only thing done to the line — the engine judges a key by
    // nothing, because a key warlock refused to hold because it did not look
    // like one is a key a person cannot use warlock with at all.
    let key = line.trim();
    if key.is_empty() {
        // Deliberately not the EOF arm above. That is a prompt nobody answered;
        // this is an answer, and storing an empty string under the name would
        // leave a checkout that binds it looking bound and failing at the API.
        return Err(Error::NoKey {
            name: name.to_owned(),
        });
    }

    save_key(home, name, key).map_err(|source| Error::Keys { source })?;
    drop(writeln!(
        out,
        "warlock: stored a key for `{name}` in `{}`",
        path.display()
    ));
    Ok(())
}

// Three answers rather than two, for `config::Held`'s reason: a store that is
// there and cannot be read has to stay apart from one that simply holds no key
// under this name, because that is the case where what this prints is not what
// is on disk. The save refuses it a moment later — the engine replaces a store
// it could not read for nobody — so the preamble saying "unknown" is the whole
// warning a reader gets before that refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Stored {
    Key,
    Nothing,
    Unknown(String),
}

// Asked through the *names* rather than `load_key`, which would answer the same
// question by reading every value in the store into this process — including
// the one about to be replaced. Nothing here needs a value, so nothing here
// reads one.
fn stored_under(home: &Path, name: &str) -> Stored {
    match load_key_names(home) {
        Ok(names) if names.iter().any(|stored| stored == name) => Stored::Key,
        Ok(_) | Err(keys::Error::NotFound { .. }) => Stored::Nothing,
        Err(error) => Stored::Unknown(one_line(&error.to_string())),
    }
}

impl fmt::Display for Stored {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key => write!(
                f,
                "a key is already stored under this name, and the line typed replaces it"
            ),
            Self::Nothing => write!(f, "no key is stored under this name yet"),
            // Said as what it is, so a broken store is never printed as one
            // holding nothing: the two look identical here and mean opposite
            // things about what is about to be refused.
            Self::Unknown(reason) => {
                write!(f, "what is stored under this name is unknown: {reason}")
            }
        }
    }
}

// Pure, and it ends *without* a newline, because the last thing it composes is
// the line the reader types on. The order is fixed by what they need before they
// can answer: what this is about, where it lands, what is there now, how to
// answer without the key appearing on the screen, and what changes nothing.
fn preamble(name: &str, path: &Path, stored: &Stored) -> String {
    // One `format!` rather than a line at a time, so what is on the screen is
    // read here in the order it is printed in.
    format!(
        "key `{name}`\n\
         stored at `{path}`\n\
         {stored}\n\
         the line is echoed, so `warlock key add {name} < key.txt` is how to keep it off \
         the screen\n\
         Ctrl-C or EOF changes nothing\n\
         {PROMPT}",
        path = path.display(),
    )
}

// Split from `key_list` for `added`'s reason: `out` is what a test reads back,
// and both renderings are built from the one list rather than printed as they
// are found.
fn listed<W: Write>(home: &Path, json: bool, out: &mut W) -> Result<(), Error> {
    let names = names(home)?;

    if json {
        write_object(out, &object(&names));
    } else {
        for name in &names {
            drop(writeln!(out, "{name}"));
        }
    }
    Ok(())
}

// A machine with no store holds no keys, which is an empty answer and exit 0
// rather than a failure: `warlock key list` on a machine nobody has run `key
// add` on is the question "what do I hold", and "nothing" answers it. A store
// that exists and will not parse stays a failure — that is a list warlock could
// not read, and printing it as an empty one would tell an operator they hold
// nothing while their keys sit in the file.
fn names(home: &Path) -> Result<Vec<String>, Error> {
    match load_key_names(home) {
        Err(keys::Error::NotFound { .. }) => Ok(Vec::new()),
        other => other.map_err(|source| Error::Keys { source }),
    }
}

// The same envelope a listing and a check print, with names and nothing else in
// it. There is no field for a key and there is not going to be one: the whole
// point of the subcommand is that no shell history, screen share or CI log ever
// holds a value. `names` is an array and empty when nothing is stored, never
// absent and never null.
fn object(names: &[String]) -> Value {
    envelope(
        COMMAND,
        [(
            NAMES,
            Value::Array(names.iter().cloned().map(Value::String).collect()),
        )],
    )
}

// `Ok(0)` is EOF and nothing else. It is told apart from an empty line here
// rather than further down, because everything above treats a line as text and
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
