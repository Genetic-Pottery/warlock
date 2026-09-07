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
mod tests {
    use std::path::{Path, PathBuf};

    use warlock_engine::{load_sigils, scope, sigils_path};

    use super::{Held, NOTHING, PROMPT, held_for, hold, holding, preamble, prompted, sigils_in};
    use crate::error::Error;
    use crate::standing::Standing;

    // Every test that writes anything builds both its home *and* its repository
    // root out of these, so nothing here touches the developer's real home.
    fn a_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    // The production composition with the two things a person supplies handed
    // in instead: the answer — `None` is EOF — and somewhere to print.
    fn prompt_with(repo: &Path, home: &Path, line: Option<&str>) -> (Result<(), Error>, String) {
        let standing = Standing::at(repo.to_path_buf(), repo.to_path_buf());
        let answer = line.map(str::to_owned);
        let mut out = Vec::new();
        let outcome = prompted(&standing, home, || Ok(answer), &mut out);
        (
            outcome,
            String::from_utf8(out).expect("warlock writes its own text"),
        )
    }

    #[test]
    fn the_composition_says_what_is_held_before_it_reads_anything() {
        let (home, repo) = (a_dir(), a_dir());

        let (outcome, said) = prompt_with(repo.path(), home.path(), None);

        outcome.expect("an EOF is not a failure");
        assert!(
            said.contains(PROMPT),
            "the cursor's own line never got out: {said}"
        );
        assert!(
            said.contains(NOTHING),
            "a machine that has recorded nothing should be told so: {said}"
        );
    }

    #[test]
    fn an_end_of_file_writes_nothing_and_says_so() {
        let (home, repo) = (a_dir(), a_dir());

        let (outcome, said) = prompt_with(repo.path(), home.path(), None);

        outcome.expect("an EOF is not a failure");
        assert!(said.ends_with("warlock: nothing changed\n"), "{said:?}");
        assert!(
            !sigils_path(home.path(), repo.path()).exists(),
            "a missing config was created by a prompt nobody answered"
        );
    }

    #[test]
    fn a_line_of_sigils_is_written_and_the_file_is_named_back() {
        let (home, repo) = (a_dir(), a_dir());

        let (outcome, said) = prompt_with(repo.path(), home.path(), Some("data-plane web\n"));

        outcome.expect("two sigils are a line this accepts");
        let path = sigils_path(home.path(), repo.path());
        assert_eq!(
            load_sigils(home.path(), repo.path()).expect("the config just written loads"),
            ["data-plane", "web"],
            "what went to disk is not what was typed"
        );
        assert!(
            said.contains(&path.display().to_string()),
            "the confirmation does not name the file it wrote: {said}"
        );
    }

    #[test]
    fn a_line_that_is_not_sigils_writes_nothing_at_all() {
        let (home, repo) = (a_dir(), a_dir());

        // Begins with `-`, which no sigil may. Not an uppercase word: those are
        // folded to ASCII lowercase before they are judged, so `WEB` is `web`.
        let (outcome, _) = prompt_with(repo.path(), home.path(), Some("-nope\n"));

        let error = outcome.expect_err("a line of nonsense is refused");
        assert!(matches!(error, Error::Sigil { .. }), "{error:?}");
        assert!(
            !sigils_path(home.path(), repo.path()).exists(),
            "a refused line still wrote a file"
        );
    }

    fn parsed(line: &str) -> Vec<String> {
        sigils_in(line).expect("every word on this line is a sigil")
    }

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
                assert_eq!(rule, scope::Rule::Character { character: '!' });
            }
            other => panic!("expected a refusal, got {other:?}"),
        }

        // The first failing word ends the line, whatever follows it.
        match sigils_in("1data billing\n") {
            Err(Error::Sigil { entered, rule }) => {
                assert_eq!(entered, "1data");
                assert_eq!(rule, scope::Rule::Beginning { character: '1' });
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
