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
        preamble(&root, &path, &held_for(home.path(), &root)).contains(&path.display().to_string())
    );
}
