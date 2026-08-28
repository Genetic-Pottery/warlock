//! `warlock config`: the sigils this machine holds for this repository.
//!
//! The second subcommand, in the shape `warlock init` gave the first: it
//! is dispatched before anything touches the terminal, it never enters the
//! alternate screen, it installs no panic hook, and everything it has to say is
//! printed on the ordinary screen. A failure is an [`Error`] returned to `main`,
//! which prints it in the same place and the same shape as a tree that would not
//! load.
//!
//! # What it is for
//!
//! A **scope** is a fact about a directory, committed in `.warlock/pacts.toml`
//! and read by everybody who clones the repository. A **sigil** is what one
//! person on one machine holds, and it is never written inside a repository.
//! This subcommand is the only way to record the second, and it records it at
//! `<home>/.warlock/<project>/config.toml` — see
//! [`sigils_path`](warlock_engine::sigils_path). Nothing here writes, or offers
//! to write, any file inside the repository, and nothing here matches a sigil
//! against a scope: what ships is the record and the vocabulary.
//!
//! # Print, then read, and say everything before the cursor
//!
//! The interaction is one line in and one file out, and the whole of it is
//! stated above the cursor: which project was resolved and where its file is,
//! what is held right now, the rules a sigil follows, and what each of the three
//! possible answers does. A blank line *clears* the set, and a prompt whose
//! blank answer destroys something has to say so on the screen rather than in a
//! manual — which is also what buys the single entry point. There is no `warlock
//! config clear`, no `warlock config set`, no flag and no second spelling of
//! anything, so there is no argument parser here and no line editor: exactly one
//! line is read, with [`io::Stdin::read_line`], in cooked mode.
//!
//! Cooked mode is also the whole of what Ctrl-C needs. The terminal is never put
//! into raw mode and no panic hook is installed, so Ctrl-C at this prompt is a
//! SIGINT that ends the process where it stands — before the read returns, and
//! so before anything is validated or written. EOF is the same promise by
//! another road and is the one this file has to keep deliberately: a `0` from
//! the read writes nothing at all, leaving a missing file missing and an
//! existing one byte for byte as it was.
//!
//! # Where each judgement is made
//!
//! [`sigils_in`] turns the line into the set and is the only judge of it, and
//! [`preamble`] composes everything printed above the prompt. Both are pure
//! functions of their arguments — no stdin, no repository, no home directory —
//! so the two things worth being sure of, that a line means what it looks like
//! it means and that the screen says what it has to say before the cursor, are
//! tested without a process to spawn. What a sigil may be is not decided here at
//! all: it is [`validate_sigil`]'s answer, asked one string at a time, which is
//! what keeps one vocabulary for a boundary typed at this prompt and a boundary
//! read out of a manifest.

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::{env, fmt};

use warlock_engine::{
    SigilError, load_sigils, repository_root, save_sigils, sigils_path, validate_sigil,
};

use crate::error::{Error, one_line};

/// What this subcommand wants a repository root for, as the tail of
/// [`Error::NoRepository`]'s sentence. `init`'s own tail is spelled beside
/// `init`, in `main.rs`.
const FOR_SIGILS: &str = "hold sigils for";

/// The environment variable a home directory is read from, and the one it falls
/// back to on Windows. Two names and no third: this is a lookup, not a search,
/// and a machine with neither set is told so rather than guessed at.
const HOME: &str = "HOME";

/// The Windows spelling of [`HOME`], consulted only when `HOME` says nothing.
const USERPROFILE: &str = "USERPROFILE";

/// The cursor's own line. Named for what is expected of it and for the one
/// thing a reader has to get right about the syntax — several sigils are
/// separated by spaces — because it is the last thing on the screen before they
/// type.
const PROMPT: &str = "sigils (separated by spaces)> ";

/// How a set holding nothing is worded, at the prompt and afterwards.
const NOTHING: &str = "nothing";

/// The rules a sigil follows, in the one sentence they are printed as.
///
/// Written out rather than derived from
/// [`validate_sigil`](warlock_engine::validate_sigil), which judges strings and
/// has nothing to say about itself. That makes this a second statement of the
/// same rules, and it is the honest place for one: it is what a person reads
/// before typing, and the alternative is a prompt that says nothing and refuses
/// afterwards. The wildcard is on the end because it is the one thing a sigil
/// may be that a scope may not.
const RULES: &str = "a sigil is 1 to 24 characters of lowercase letters, digits, `-` and \
                     `_`, begins with a letter and does not end with `-` or `_`; `*` on \
                     its own means anywhere";

/// `warlock config`: print what is held, read one line, and write what it says.
///
/// The steps are the ones the module doc describes, in that order and with the
/// read in the middle of them: the working directory says where to start,
/// [`repository_root`] walks up to the nearest ancestor with a `.git/` — so
/// running this from any subdirectory configures the one checkout — the home
/// directory says where the file goes, and what is already held is printed
/// before a cursor ever appears.
///
/// Every way out of the read but one writes something: a line of sigils replaces
/// the set, a blank line replaces it with the empty set, and EOF returns here
/// having touched nothing. That last case is a `return` of its own rather than
/// an empty set, because "changed nothing" and "cleared it" are the two answers
/// this prompt most has to keep apart.
///
/// # Errors
///
/// [`Error::WorkingDirectory`], [`Error::NoRepository`] and [`Error::NoHome`]
/// before anything is printed; [`Error::Prompt`] if the line cannot be read;
/// [`Error::Sigil`] if something on it is not a sigil, in which case nothing is
/// written; and [`Error::Sigils`] if the file itself will not write.
pub(crate) fn configure() -> Result<(), Error> {
    let working_dir = env::current_dir().map_err(|source| Error::WorkingDirectory { source })?;
    // Asked directly rather than through a load, exactly as `init` asks: this
    // resolves a checkout in order to name a file under the home directory, and
    // walking the tree would be reading every directory in the repository to
    // answer a question about ancestors.
    let root = repository_root(&working_dir).ok_or(Error::NoRepository {
        start: working_dir,
        wanted: FOR_SIGILS,
    })?;
    let home = home_directory()?;
    let path = sigils_path(&home, &root);

    print!("{}", preamble(&root, &path, &held_for(&home, &root)));
    // Best effort, and the only thing that could be done about it: the prompt
    // has no newline of its own, so it sits in the terminal's buffer until this
    // pushes it out. A stdout that will not flush has nothing useful to say
    // about itself, and the read below reports anything that really goes wrong.
    drop(io::stdout().flush());

    let Some(line) = read_line()? else {
        // EOF, which is Ctrl-D at a terminal and an empty pipe everywhere else.
        // Nothing is parsed and nothing is written: a missing file stays missing
        // and an existing one is not opened. The newline is because the prompt
        // above has none and the cursor is still sitting on it.
        println!("\nwarlock: nothing changed");
        return Ok(());
    };

    let sigils = hold(&home, &root, &line)?;
    println!(
        "warlock: holding {} for `{}`, written to `{}`",
        holding(&sigils),
        root.display(),
        path.display()
    );
    Ok(())
}

/// What this machine holds for a repository, as `warlock config` found it.
///
/// Two answers rather than three, and the join is deliberate: a config that is
/// not there and a config holding an empty set both read as *nothing held* at
/// this prompt, because the line about to be typed replaces either of them in
/// exactly the same way. A file that is there and cannot be read is the answer
/// that must stay apart from those, since it is the one case where what is on
/// disk is not what this says.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Held {
    /// The sigils the config holds, in file order and exactly as stored. Empty
    /// means nothing is held, whether the file said so or was not there at all.
    Sigils(Vec<String>),
    /// The file is there and could not be read or understood, with the one line
    /// saying why. Printed as it stands: the reader is about to be offered a
    /// line that would overwrite it, and they should know that is what it does.
    Unreadable(String),
}

/// Read what is held for `root` under `home`, without failing.
///
/// A read that goes wrong is a state to print rather than a way out, because
/// this subcommand's job is to *set* the sigils and a config that cannot be read
/// is the situation a reader most needs to be able to type over. The engine's
/// "not found" is the one error that is not a problem — see
/// [`load_sigils`] — and it reads as the empty set here.
fn held_for(home: &Path, root: &Path) -> Held {
    match load_sigils(home, root) {
        Ok(sigils) => Held::Sigils(sigils),
        Err(SigilError::NotFound { .. }) => Held::Sigils(Vec::new()),
        Err(error) => Held::Unreadable(one_line(&error.to_string())),
    }
}

/// Everything printed above the cursor: what was resolved, what is held, what a
/// sigil may be, what each answer does, and the prompt itself.
///
/// Pure, and it ends *without* a newline, because the last thing it composes is
/// the line the reader types on. `root` is the repository this is about and
/// `path` the file it would write, both named because a subcommand that edits a
/// file the reader has never seen should say which file, and because the project
/// directory in that path is what tells two checkouts of one repository apart.
///
/// The order is fixed by what the reader needs before they can answer: what this
/// is about, what it is now, what a legal answer looks like, and only then what
/// their answer will do. The three answers are stated together, in one line
/// each, with the destructive one named as plainly as the others.
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

/// The set `sigils`, as the middle of a sentence: each one quoted and separated
/// by commas, or [`NOTHING`] when there are none.
///
/// One wording, used before the line is typed and after it is written, so that
/// running `warlock config` twice and typing the same line the second time
/// prints back exactly what the first run printed.
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

/// The set of sigils `line` asks for, or the first thing on it that is not one.
///
/// Whitespace separates them and nothing else does: a line is a shell-like list
/// of words, which is what makes the trailing `\n` — or `\r\n` — vanish rather
/// than becoming a token, and what makes a line of only spaces the same answer
/// as an empty one. So a blank line yields the empty set, which the caller
/// writes: clearing is a set that is replaced, not a file that is deleted.
///
/// Each word is lower-cased and then judged, in that order, because folding
/// belongs where a person supplies a string — `Data-Plane` and `data-plane` are
/// one holding, not two — and the engine's validator deliberately never repairs
/// what it is handed. The first word that fails ends the whole line: a set is
/// replaced entirely, so writing the words that came before a typo would leave
/// the reader holding half of what they typed.
///
/// Repeats are dropped, keeping the first of them. What is stored is a set — the
/// question a sigil answers is "do you hold this one" — and typing `Billing
/// billing` is one holding written twice rather than two.
///
/// # Errors
///
/// [`Error::Sigil`], naming the word as it was typed and the one rule it broke.
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

/// Judge `line` and, if every word on it is a sigil, make it the set held for
/// `root` under `home`. Hands back what is now held.
///
/// The whole of what an answered prompt does, with both ends passed in rather
/// than looked up: `home` is a parameter all the way down to the engine, which
/// is what lets this be tested against a temporary directory and what keeps any
/// test from writing to the developer's real home.
///
/// Nothing is written unless the whole line is sigils, because [`sigils_in`]
/// runs first and returns before the save is reached. The save itself is
/// write-and-rename inside the home directory (see
/// [`save_sigils`](warlock_engine::save_sigils)): the set is replaced rather
/// than added to, and no file inside the repository is touched.
///
/// # Errors
///
/// [`Error::Sigil`] for a word that is not a sigil, or [`Error::Sigils`] if the
/// file will not write.
fn hold(home: &Path, root: &Path, line: &str) -> Result<Vec<String>, Error> {
    let sigils = sigils_in(line)?;
    save_sigils(home, root, &sigils).map_err(|source| Error::Sigils { source })?;
    Ok(sigils)
}

/// The one line the prompt reads, or `None` at end of input.
///
/// `Ok(0)` from the read is EOF and nothing else, and it is kept apart from an
/// empty line here rather than anywhere further down: everything below this
/// treats a line as text, and only this function can tell "they pressed Enter on
/// an empty line" from "there is no line and never will be".
///
/// # Errors
///
/// [`Error::Prompt`] if stdin cannot be read at all.
fn read_line() -> Result<Option<String>, Error> {
    let mut line = String::new();
    match io::stdin().read_line(&mut line) {
        Ok(0) => Ok(None),
        Ok(_) => Ok(Some(line)),
        Err(source) => Err(Error::Prompt { source }),
    }
}

/// This machine's home directory, from [`HOME`] or, failing that,
/// [`USERPROFILE`].
///
/// The only place in warlock that reads either variable: the engine takes a home
/// as a parameter all the way through, which is what keeps its tests off the
/// developer's real one, so this is the single point where a process's
/// environment becomes a path. It is shared with the TUI's own read of the
/// config — [`sigils_held`](crate::session::sigils_held), which states what is
/// held on the header — rather than copied there, so the two answer "where is
/// home" the same way or not at all.
///
/// Read as an `OsString`, since a home directory is a path rather than text and
/// need not be UTF-8, and an empty value is treated as unset — an exported but
/// empty `HOME` would otherwise resolve the config to `/.warlock/...`.
///
/// # Errors
///
/// [`Error::NoHome`] if neither is set to anything.
pub(crate) fn home_directory() -> Result<PathBuf, Error> {
    [HOME, USERPROFILE]
        .into_iter()
        .find_map(|variable| env::var_os(variable).filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .ok_or(Error::NoHome)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use warlock_engine::{ScopeRule, load_sigils, sigils_path};

    use super::{Held, NOTHING, PROMPT, held_for, hold, holding, preamble, sigils_in};
    use crate::error::Error;

    /// A throwaway directory. Every test that writes anything builds both its
    /// home *and* its repository root out of these, so nothing here reads or
    /// writes the developer's real home.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// The sigils `line` asks for, for the lines that are all sigils.
    fn parsed(line: &str) -> Vec<String> {
        sigils_in(line).expect("every word on this line is a sigil")
    }

    /// The sigils held for `root` under `home`, as the engine has them on disk.
    fn on_disk(home: &Path, root: &Path) -> Vec<String> {
        load_sigils(home, root).expect("a config that was just written")
    }

    #[test]
    fn a_blank_line_is_the_empty_set_rather_than_a_refusal() {
        // The clearing answer, in every spelling of blank a terminal can send.
        for line in ["", "\n", "\r\n", "   \n", "\t \t\n"] {
            assert_eq!(parsed(line), Vec::<String>::new(), "for `{line:?}`");
        }
    }

    #[test]
    fn one_sigil_is_one_sigil_and_the_newline_is_not_a_word() {
        for line in [
            "data-plane",
            "data-plane\n",
            "data-plane\r\n",
            "  billing \n",
        ] {
            assert_eq!(parsed(line).len(), 1, "for `{line:?}`");
        }
        assert_eq!(parsed("data-plane\n"), ["data-plane"]);
        assert_eq!(parsed("  billing \n"), ["billing"]);
    }

    #[test]
    fn several_sigils_are_kept_in_the_order_they_were_typed() {
        assert_eq!(
            parsed("billing data-plane web\n"),
            ["billing", "data-plane", "web"],
            "the line reads back as it was written"
        );
        assert_eq!(parsed("billing\t\tweb\n"), ["billing", "web"]);
    }

    #[test]
    fn the_wildcard_is_a_sigil_here_and_needs_no_exception_of_its_own() {
        // `*` is refused as a directory scope and accepted as a sigil: the
        // asymmetry is `validate_sigil`'s, and this call site simply asks it.
        assert_eq!(parsed("*\n"), ["*"]);
        assert_eq!(parsed("* billing\n"), ["*", "billing"]);
        assert!(warlock_engine::validate_scope("*").is_err());
    }

    #[test]
    fn a_typed_capital_is_folded_and_then_accepted() {
        // Folding belongs where a person supplies the string, so `Data-Plane`
        // and `data-plane` cannot become two holdings.
        assert_eq!(parsed("Data-Plane\n"), ["data-plane"]);
        assert_eq!(parsed("BILLING WEB\n"), ["billing", "web"]);
    }

    #[test]
    fn a_word_that_is_not_a_sigil_names_itself_and_its_rule_and_writes_nothing() {
        match sigils_in("billing data plane!\n") {
            Err(Error::Sigil { entered, rule }) => {
                assert_eq!(entered, "plane!", "the word as it was typed");
                assert_eq!(rule, ScopeRule::Character { character: '!' });
            }
            other => panic!("expected a refusal, got {other:?}"),
        }

        // The first failing word ends the line, whatever follows it.
        match sigils_in("1data billing\n") {
            Err(Error::Sigil { entered, rule }) => {
                assert_eq!(entered, "1data");
                assert_eq!(rule, ScopeRule::Beginning { character: '1' });
            }
            other => panic!("expected a refusal, got {other:?}"),
        }

        // A comma is not a separator here — the line is words — so it is part of
        // the word it is stuck to and refused as the character it is.
        assert!(matches!(
            sigils_in("data-plane, billing\n"),
            Err(Error::Sigil { .. })
        ));
    }

    #[test]
    fn a_repeat_is_one_holding_rather_than_two() {
        assert_eq!(parsed("billing billing\n"), ["billing"]);
        // Including a repeat that only the folding makes one.
        assert_eq!(parsed("Billing billing\n"), ["billing"]);
        assert_eq!(
            parsed("web billing web\n"),
            ["web", "billing"],
            "the first of each is the one that is kept"
        );
    }

    #[test]
    fn the_preamble_says_everything_before_the_cursor() {
        let text = preamble(
            Path::new("/repo"),
            Path::new("/home/someone/.warlock/repo-abc/config.toml"),
            &Held::Sigils(vec!["billing".to_owned(), "web".to_owned()]),
        );

        assert!(text.contains("/repo"), "the repository it resolved: {text}");
        assert!(
            text.contains("/home/someone/.warlock/repo-abc/config.toml"),
            "the file it would write: {text}"
        );
        assert!(text.contains("holding `billing`, `web`"), "{text}");
        assert!(text.contains("lowercase letters"), "the rules: {text}");
        assert!(text.contains('*'), "including the wildcard: {text}");
        assert!(text.contains("replaces everything held"), "{text}");
        assert!(text.contains("a blank line clears it"), "{text}");
        assert!(text.contains("Ctrl-C or EOF changes nothing"), "{text}");
        assert!(
            text.ends_with(PROMPT),
            "the prompt is the last thing, with no newline after it: {text}"
        );

        // The order the reader needs them in: what this is, what it is now, what
        // a legal answer is, and only then what their answer does.
        let at = |needle: &str| text.find(needle).expect("said somewhere");
        assert!(at("sigils for") < at("holding"));
        assert!(at("holding") < at("lowercase letters"));
        assert!(at("lowercase letters") < at("replaces everything held"));
        assert!(at("replaces everything held") < at("a blank line clears it"));
        assert!(at("a blank line clears it") < at("Ctrl-C or EOF changes nothing"));
        assert!(at("Ctrl-C or EOF changes nothing") < at(PROMPT));
    }

    #[test]
    fn holding_nothing_is_said_rather_than_left_blank() {
        assert_eq!(holding(&[]), NOTHING);
        let text = preamble(
            Path::new("/repo"),
            Path::new("/home/someone/.warlock/repo-abc/config.toml"),
            &Held::Sigils(Vec::new()),
        );
        assert!(text.contains("holding nothing"), "{text}");
    }

    #[test]
    fn a_config_that_cannot_be_read_is_never_printed_as_an_empty_one() {
        let text = preamble(
            Path::new("/repo"),
            Path::new("/home/someone/.warlock/repo-abc/config.toml"),
            &Held::Unreadable("malformed sigil config at `x`: boom".to_owned()),
        );

        assert!(
            text.contains("holding unknown: malformed sigil config"),
            "{text}"
        );
        assert!(
            !text.contains("holding nothing"),
            "broken and empty mean opposite things about what is on disk: {text}"
        );
    }

    #[test]
    fn the_same_line_twice_holds_the_same_set_twice() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = elsewhere.path();

        let first = hold(home.path(), root, "billing Data-Plane\n").expect("writes");
        assert_eq!(first, ["billing", "data-plane"]);
        assert_eq!(held_for(home.path(), root), Held::Sigils(first.clone()));

        // Run again, same line: the set replaces itself rather than doubling,
        // and the second run reports what the first one did.
        let second = hold(home.path(), root, "billing Data-Plane\n").expect("writes again");
        assert_eq!(second, first);
        assert_eq!(held_for(home.path(), root), Held::Sigils(second));
        assert_eq!(on_disk(home.path(), root), ["billing", "data-plane"]);
    }

    #[test]
    fn a_blank_line_clears_the_set_and_a_line_replaces_it_entirely() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = elsewhere.path();

        hold(home.path(), root, "billing web\n").expect("writes");
        assert_eq!(
            hold(home.path(), root, "data-plane\n").expect("writes"),
            ["data-plane"],
            "a line replaces the set rather than adding to it"
        );

        assert_eq!(
            hold(home.path(), root, "   \n").expect("writes"),
            Vec::<String>::new()
        );
        assert_eq!(held_for(home.path(), root), Held::Sigils(Vec::new()));
        assert!(
            sigils_path(home.path(), root).exists(),
            "clearing writes an empty set rather than deleting the file"
        );
    }

    #[test]
    fn nothing_held_reads_the_same_whether_the_file_is_absent_or_empty() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = elsewhere.path();

        assert_eq!(
            held_for(home.path(), root),
            Held::Sigils(Vec::new()),
            "a machine that has never been configured holds nothing"
        );

        hold(home.path(), root, "\n").expect("writes");
        assert_eq!(held_for(home.path(), root), Held::Sigils(Vec::new()));
    }

    #[test]
    fn a_config_that_will_not_parse_is_a_state_rather_than_a_way_out() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = elsewhere.path();
        let path = sigils_path(home.path(), root);
        std::fs::create_dir_all(path.parent().expect("the config has a directory"))
            .expect("creates the project directory");
        std::fs::write(&path, "this is not toml\n").expect("writes a broken config");

        match held_for(home.path(), root) {
            Held::Unreadable(reason) => {
                assert!(reason.contains("config.toml"), "{reason}");
                assert!(!reason.contains('\n'), "one line: {reason}");
            }
            held @ Held::Sigils(_) => panic!("expected an unreadable config, got {held:?}"),
        }

        // And it can be typed over, which is the reason it is not a way out.
        assert_eq!(
            hold(home.path(), root, "billing\n").expect("writes"),
            ["billing"]
        );
    }

    #[test]
    fn a_refused_line_writes_nothing_at_all() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = elsewhere.path();

        assert!(hold(home.path(), root, "not a sigil!\n").is_err());
        assert!(
            !sigils_path(home.path(), root).exists(),
            "a missing config stays missing"
        );

        hold(home.path(), root, "billing\n").expect("writes");
        let before = std::fs::read(sigils_path(home.path(), root)).expect("reads");
        assert!(hold(home.path(), root, "billing plane!\n").is_err());
        assert_eq!(
            std::fs::read(sigils_path(home.path(), root)).expect("reads"),
            before,
            "and an existing one is byte for byte what it was"
        );
    }

    #[test]
    fn nothing_is_written_inside_the_repository() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root = elsewhere.path();

        hold(home.path(), root, "billing web\n").expect("writes");
        held_for(home.path(), root);

        assert_eq!(
            std::fs::read_dir(root)
                .expect("reads the repository")
                .count(),
            0,
            "a sigil is a fact about a machine, and never a file in a checkout"
        );
        assert!(sigils_path(home.path(), root).starts_with(home.path()));
    }

    #[test]
    fn the_file_the_preamble_names_is_the_file_that_is_written() {
        let (home, elsewhere) = (a_dir(), a_dir());
        let root: PathBuf = elsewhere.path().to_path_buf();
        let path = sigils_path(home.path(), &root);

        hold(home.path(), &root, "billing\n").expect("writes");
        assert!(path.exists(), "`{}` should be the file", path.display());
        assert!(
            preamble(&root, &path, &held_for(home.path(), &root))
                .contains(&path.display().to_string())
        );
    }
}
