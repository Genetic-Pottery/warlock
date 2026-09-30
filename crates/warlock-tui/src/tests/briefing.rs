use std::fs;
use std::path::{Path, PathBuf};

use warlock_engine::briefs_path;
use warlock_tui::{
    BRIEF_EFFORT, BRIEF_MODEL, ChatAgent, DEFAULT_TEMPLATE, Ending, brief_instruction,
};

use super::{OPENING, OVER, PROMPT, briefing, raised, reading};
use crate::error::Error;
use crate::standing::Standing;
use crate::stubs::{Answering, Scripted, Typing};

// Every test builds its own repository out of one of these, so nothing here
// reads a template, a `briefs.toml` or a manifest belonging to the checkout the
// suite is running in.
fn a_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn template_path(root: &Path) -> PathBuf {
    root.join(".warlock").join("brief-template.md")
}

fn write_under(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("the file has a directory"))
        .expect("a `.warlock` directory");
    fs::write(path, text).expect("writes the file");
}

// The whole conversation, driven from a written-down script: `lines` is what
// somebody typed — the list running out is EOF — and `answers` is what the model
// says, one per turn, the first of them the answer to warlock's own instruction.
struct Ran {
    outcome: Result<(), Error>,
    // What a reader would have seen, the prompts aside: those are the ask's, and
    // come back in `asked`.
    said: String,
    asked: Vec<String>,
    // Every message the model was handed, in order and in the words it was sent
    // in.
    turns: Vec<String>,
}

fn run(root: &Path, lines: &[&str], answers: Vec<Answering>) -> Ran {
    let standing = Standing::at(root.to_path_buf(), root.to_path_buf());
    let agent = Scripted::saying(answers);
    let mut typing = Typing::lines(lines.iter().copied());
    let mut out = Vec::new();

    let outcome = briefing(&standing, &agent, &mut typing, &mut out);

    Ran {
        outcome,
        said: String::from_utf8(out).expect("warlock writes its own text"),
        asked: typing.asked().to_vec(),
        turns: agent.said(),
    }
}

// One turn's worth of script, for the tests that are not about what the model
// said back.
fn replying(answers: &[&str]) -> Vec<Answering> {
    answers.iter().map(|text| Answering::says(*text)).collect()
}

#[test]
fn the_conversation_opens_with_the_instruction_paragraph_the_panel_sends() {
    let root = a_root();

    // Nothing typed at all: the instruction has already been sent and answered
    // by the time the first prompt is asked.
    let ran = run(root.path(), &[], replying(&["What is the change?"]));

    ran.outcome.expect("an EOF is not a failure");
    assert_eq!(
        ran.turns,
        [brief_instruction(DEFAULT_TEMPLATE)],
        "the first turn is not `/brief`'s own paragraph, built from the same template",
    );
    assert!(
        ran.said.contains(OPENING),
        "the register is not said before the first turn: {}",
        ran.said
    );
    assert!(
        ran.said.contains("warlock: What is the change?"),
        "the reply was not printed: {}",
        ran.said
    );
    assert_eq!(
        ran.asked,
        [PROMPT],
        "the one cursor, on a line of its own, asked once"
    );
}

#[test]
fn the_shape_is_the_repositorys_own_template_when_it_has_written_one() {
    let root = a_root();
    // Deliberately nothing like the built-in shape: the instruction is built
    // from whatever the file says, and nothing here trims or validates it.
    let shape = "## Only this\n\nsay the thing.";
    write_under(&template_path(root.path()), shape);

    let ran = run(root.path(), &[], replying(&["asking"]));

    ran.outcome.expect("a template this repository wrote");
    assert_eq!(ran.turns, [brief_instruction(shape)]);
    assert_ne!(ran.turns[0], brief_instruction(DEFAULT_TEMPLATE));
}

#[test]
fn typed_lines_accumulate_and_an_empty_line_sends_them_as_one_turn() {
    let root = a_root();

    let ran = run(
        root.path(),
        // The trailing newlines a terminal really sends, and a line indented on
        // purpose: what is trimmed is the end of a line and never its start.
        &[
            "the CLI cannot be spoken to\n",
            "  both are one mechanism\n",
            "",
        ],
        replying(&["asking", "Two changes, one mechanism."]),
    );

    ran.outcome.expect("a turn sent and answered");
    assert_eq!(
        ran.turns.len(),
        2,
        "two lines became two turns rather than one: {:?}",
        ran.turns
    );
    assert_eq!(
        ran.turns[1], "the CLI cannot be spoken to\n  both are one mechanism",
        "the lines did not reach the model as they were typed",
    );
    assert!(
        ran.said.contains("warlock: Two changes, one mechanism."),
        "{}",
        ran.said
    );
}

#[test]
fn an_empty_line_with_nothing_above_it_sends_nothing() {
    let root = a_root();

    // Enter pressed on its own, three times over, including once after a turn
    // has been sent and its lines cleared.
    let ran = run(
        root.path(),
        &["", "something", "", ""],
        replying(&["asking", "said"]),
    );

    ran.outcome.expect("an empty line is not a failure");
    assert_eq!(
        ran.turns.len(),
        2,
        "silence was sent to the model as a turn: {:?}",
        ran.turns
    );
    assert_eq!(ran.turns[1], "something");
    assert_eq!(ran.asked.len(), 5, "the cursor came back each time");
}

#[test]
fn every_turn_is_a_turn_of_one_conversation() {
    let root = a_root();

    let ran = run(
        root.path(),
        &["first", "", "second", ""],
        replying(&["asking", "one", "two"]),
    );

    ran.outcome.expect("two turns");
    assert_eq!(ran.turns.len(), 3, "{:?}", ran.turns);
    // The instruction rides on the turn that opens the conversation and on no
    // other: a second copy of it would be warlock telling the model where it
    // already is, and paying for the sentence twice.
    for message in &ran.turns[1..] {
        assert!(
            !message.contains("Nothing is written until I ask"),
            "the instruction was sent again: {message}"
        );
    }
    assert_eq!(&ran.turns[1..], ["first", "second"]);
}

fn words(agent: &ChatAgent) -> Vec<String> {
    agent
        .args()
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn value_of<'a>(vector: &'a [String], flag: &str) -> Option<&'a str> {
    let named = vector.iter().position(|word| word == flag)?;
    vector.get(named + 1).map(String::as_str)
}

#[test]
fn the_register_is_the_one_the_panels_brief_mode_raises_to() {
    // Asserted on the vector a turn would really run with, as `chatting`'s own
    // test asserts the mode change: the two words move and nothing else does.
    let agent = ChatAgent::new();
    let built = words(&agent);
    let brief = words(&raised(&agent));

    assert_eq!(brief.len(), built.len());
    let moved: Vec<(&String, &String)> = brief
        .iter()
        .zip(&built)
        .filter(|(brief, built)| brief != built)
        .collect();
    assert_eq!(
        moved.len(),
        2,
        "the register changed something other than which model thinks and how \
         hard: {moved:?}"
    );
    assert_eq!(value_of(&brief, "--effort"), Some(BRIEF_EFFORT));
    assert_eq!(value_of(&brief, "--model"), Some(BRIEF_MODEL));
    assert_eq!(
        value_of(&brief, "--session-id"),
        value_of(&built, "--session-id"),
        "raising the register started a second conversation",
    );
}

#[test]
fn a_template_that_will_not_read_is_a_refusal_with_nothing_sent() {
    let root = a_root();
    // Bytes that are not UTF-8: a file that exists and cannot be had, which is
    // portable where a permission bit is not.
    write_under(&template_path(root.path()), "");
    fs::write(template_path(root.path()), [0x23, 0x20, 0xff, 0xfe, 0x0a])
        .expect("a template that will not decode");

    // Scripted with nothing, so a turn sent anyway is a panic rather than a
    // quiet pass.
    let ran = run(root.path(), &["typed", ""], Vec::new());

    let error = ran.outcome.expect_err("a template that will not read");
    assert!(matches!(error, Error::Template { .. }), "{error:?}");
    assert!(
        ran.turns.is_empty() && ran.asked.is_empty(),
        "a refused run still opened a conversation or asked for a line",
    );
    assert!(
        !ran.said.contains(OPENING),
        "a refusal said the register it never entered: {}",
        ran.said
    );
}

#[test]
fn a_briefs_file_that_will_not_read_is_a_refusal_with_nothing_sent() {
    let root = a_root();
    write_under(&briefs_path(root.path()), "this is not toml\n");

    let ran = run(root.path(), &["typed", ""], Vec::new());

    let error = ran
        .outcome
        .expect_err("a `briefs.toml` that will not parse");
    assert!(matches!(error, Error::Briefs { .. }), "{error:?}");
    assert!(
        ran.turns.is_empty() && ran.asked.is_empty(),
        "a refused run still opened a conversation or asked for a line",
    );
}

#[test]
fn both_files_are_read_before_the_first_turn_and_the_template_is_read_first() {
    let root = a_root();
    // Both broken at once: one refusal, and it is the template's, so a reader
    // fixing that file is told about the other one by the next run rather than
    // reading warlock's reading order off the screen.
    fs::create_dir_all(template_path(root.path())).expect("a directory in the template's place");
    write_under(&briefs_path(root.path()), "directory = 42\n");

    let error = reading(root.path()).expect_err("two broken files");

    assert!(matches!(error, Error::Template { .. }), "{error:?}");
    // And the `briefs.toml` refusal is what is left once the template reads.
    fs::remove_dir(template_path(root.path())).expect("removes the directory");
    let error = reading(root.path()).expect_err("one broken file");
    assert!(matches!(error, Error::Briefs { .. }), "{error:?}");

    // Neither file is needed: a repository that has written neither is the
    // built-in shape and the default directory.
    let bare = a_root();
    assert_eq!(
        reading(bare.path()).expect("nothing written is not a refusal"),
        brief_instruction(DEFAULT_TEMPLATE),
    );
}

#[test]
fn a_reply_is_printed_in_full_rather_than_flattened() {
    let root = a_root();
    let reply = "Two ways.\n\n1. The cheap one.\n2. The one that lasts.";

    let ran = run(root.path(), &[], vec![Answering::says(reply)]);

    ran.outcome.expect("an EOF is not a failure");
    // Whole and as it was said, the blank line in the middle of it included:
    // nothing here folds a reply onto one line or cuts it to a width, which is
    // what `one_line` is for and a refusal is.
    assert!(
        ran.said.contains(reply),
        "the reply was not printed as it was said: {}",
        ran.said
    );
    assert!(
        ran.said.contains("warlock: Two ways."),
        "the reply is warlock's line and says so: {}",
        ran.said
    );
}

#[test]
fn a_turn_that_failed_is_a_line_and_the_cursor_comes_back() {
    let root = a_root();

    // The instruction's own turn lost to a machine with no `claude` on it, and
    // then a turn that answers: a failure is not the end of the conversation.
    let ran = run(
        root.path(),
        &["carry on", ""],
        vec![Answering::missing(), Answering::says("still here")],
    );

    ran.outcome
        .expect("a turn that could not run is a line rather than a failure");
    let ending = Ending::NoModel {
        program: "claude".to_owned(),
    };
    assert!(
        ran.said.contains(&ending.line()),
        "the ending was not said in the panel's own words: {}",
        ran.said
    );
    assert_eq!(ran.turns.len(), 2, "{:?}", ran.turns);
    assert!(ran.said.contains("warlock: still here"), "{}", ran.said);
}

#[test]
fn an_end_of_file_ends_the_conversation_and_says_so() {
    let root = a_root();

    let ran = run(root.path(), &["typed", ""], replying(&["asking", "said"]));

    ran.outcome.expect("an EOF is not a failure");
    assert!(
        ran.said.ends_with(&format!("\nwarlock: {OVER}\n")),
        "the cursor's own line was not closed before the last word: {:?}",
        ran.said
    );
}

#[test]
fn nothing_is_written_inside_the_repository() {
    let root = a_root();

    let ran = run(root.path(), &["typed", ""], replying(&["asking", "said"]));

    ran.outcome.expect("a conversation");
    assert_eq!(
        fs::read_dir(root.path())
            .expect("reads the repository")
            .count(),
        0,
        "a conversation wrote a file; nothing here writes one until `/write` does",
    );
}
