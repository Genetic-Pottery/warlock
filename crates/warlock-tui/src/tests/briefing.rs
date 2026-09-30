use std::fs;
use std::path::{Path, PathBuf};

use warlock_engine::{DEFAULT_BRIEF_DIRECTORY, briefs_path, from_manifest_path};
use warlock_tui::{
    BRIEF_EFFORT, BRIEF_MODEL, ChatAgent, DEFAULT_TEMPLATE, Ending, WRITE_INSTRUCTION,
    brief_instruction,
};

use super::{ONLY_WRITE, OPENING, OVER, PROMPT, briefing, raised, reading};
use crate::error::Error;
use crate::standing::Standing;
use crate::stubs::{Answering, Scripted, Typing};
use crate::writing::proposed_path;

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
    let opening = reading(bare.path()).expect("nothing written is not a refusal");
    assert_eq!(opening.shape, DEFAULT_TEMPLATE);
    assert_eq!(opening.directory, DEFAULT_BRIEF_DIRECTORY);
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
fn a_conversation_that_writes_nothing_leaves_the_repository_alone() {
    let root = a_root();

    let ran = run(root.path(), &["typed", ""], replying(&["asking", "said"]));

    ran.outcome.expect("a conversation");
    assert_eq!(
        fs::read_dir(root.path())
            .expect("reads the repository")
            .count(),
        0,
        "a conversation nobody asked for a document from wrote a file",
    );
}

// A reply in the shape the built-in template asks for, which is what a `/write`
// turn is meant to come back with: a title line, the problem in prose, and the
// five sections `missing_sections` goes looking for.
fn a_brief() -> String {
    "# Give the headless CLI a voice\n\nThe shell cannot be spoken to.\n\n\
     ## Outcome\n\nA reader runs `warlock brief` and argues a change.\n\n\
     ## Success criteria\n\n- a document lands where `briefs.toml` says\n\n\
     ## Constraints\n\n- nothing about a brief is worded twice\n\n\
     ## Out of scope\n\n- resuming an interrupted conversation\n\n\
     ## Scope\n\n### 1. The subcommand\ndepends_on: []\n\nIt holds one session.\n"
        .to_owned()
}

// The path `/write` offers for a reply, asked of the very rule that proposes it:
// the test states the spelling once below and takes it from `writing.rs`
// everywhere else, so a change to the numbering or the slug moves both.
fn proposal_for(root: &Path, reply: &str) -> String {
    proposed_path(root, DEFAULT_BRIEF_DIRECTORY, reply)
}

fn wrote(root: &Path, stored: &str) -> String {
    fs::read_to_string(from_manifest_path(root, stored)).expect("the document that was written")
}

#[test]
fn write_asks_for_the_document_and_offers_the_path_writing_proposes_for_it() {
    let root = a_root();
    let brief = a_brief();
    // Asked before the run, because the proposal counts the names already in the
    // directory: after the write there is a `01` in there and the same rule
    // proposes `02`.
    let proposed = proposal_for(root.path(), &brief);
    assert_eq!(
        proposed, "docs/warlock-brief-01-give-the-headless-cli-a-voice.md",
        "the proposal is the numbering and the slug `writing.rs` makes",
    );

    let ran = run(
        root.path(),
        // The command, and then Enter on the offer.
        &["/write", ""],
        vec![Answering::says("asking"), Answering::says(brief.as_str())],
    );

    ran.outcome.expect("a document written");
    assert_eq!(
        ran.turns[1], WRITE_INSTRUCTION,
        "the document was asked for in words of this module's own",
    );
    assert!(
        ran.said.contains(&proposed),
        "the path was never offered: {}",
        ran.said
    );
    assert_eq!(
        wrote(root.path(), &proposed),
        brief,
        "what landed is not the reply the write turn gave",
    );
    assert!(
        ran.said.contains(&format!("warlock: wrote {proposed}")),
        "the path written was not printed: {}",
        ran.said
    );
    // The run is over on the write: the cursor is asked for the turn and for the
    // path, and never again.
    assert_eq!(ran.asked.len(), 2, "{:?}", ran.asked);
    assert!(
        !ran.said.contains(OVER),
        "a run that wrote its document ended as one nobody wrote: {}",
        ran.said
    );
}

#[test]
fn a_typed_path_replaces_the_one_that_was_offered() {
    let root = a_root();
    let brief = a_brief();

    let ran = run(
        root.path(),
        &["/write", "docs/somewhere/mine.md"],
        vec![Answering::says("asking"), Answering::says(brief.as_str())],
    );

    ran.outcome.expect("a document written");
    assert_eq!(wrote(root.path(), "docs/somewhere/mine.md"), brief);
    assert!(
        !from_manifest_path(root.path(), &proposal_for(root.path(), &brief)).exists(),
        "the offer was written as well as the path that replaced it",
    );
}

#[test]
fn a_path_that_already_has_a_file_is_refused_and_the_cursor_comes_back() {
    let root = a_root();
    let brief = a_brief();
    let taken = from_manifest_path(root.path(), "docs/taken.md");
    write_under(&taken, "somebody else's document\n");

    let ran = run(
        root.path(),
        &["/write", "docs/taken.md", "docs/mine.md"],
        vec![Answering::says("asking"), Answering::says(brief.as_str())],
    );

    ran.outcome.expect("the second path is written");
    assert!(
        ran.said.contains("docs/taken.md already exists"),
        "the refusal did not name the file: {}",
        ran.said
    );
    assert_eq!(
        fs::read_to_string(&taken).expect("the file that was there"),
        "somebody else's document\n",
        "a refused path was written over anyway",
    );
    // One turn, and three lines read: the offer came back after the refusal.
    assert_eq!(ran.turns.len(), 2, "{:?}", ran.turns);
    assert_eq!(ran.asked.len(), 3, "{:?}", ran.asked);
    assert_eq!(wrote(root.path(), "docs/mine.md"), brief);
}

#[test]
fn a_document_missing_a_section_of_the_shape_is_refused_and_nothing_is_written() {
    let root = a_root();
    // Everything but the sections: the shape is the built-in template, which
    // asks for five and is handed one.
    let unshaped = "# A change\n\nThe problem.\n\n## Outcome\n\nSomething happens.\n";

    let ran = run(
        root.path(),
        // The command, Enter on the offer, and then the conversation carrying on
        // — which is the whole of what a document warlock will not write leaves
        // a reader with.
        &["/write", "", "you dropped the scope", ""],
        replying(&["asking", unshaped, "sorry"]),
    );

    ran.outcome.expect("an EOF is not a failure");
    assert!(
        ran.said.contains("the document is missing"),
        "the shape refusal was not said: {}",
        ran.said
    );
    for section in ["## Success criteria", "## Constraints", "## Scope"] {
        assert!(ran.said.contains(section), "{section}: {}", ran.said);
    }
    assert!(
        !from_manifest_path(root.path(), DEFAULT_BRIEF_DIRECTORY).exists(),
        "a document the shape turned down was written anyway",
    );
    // The conversation went on: the line after the refusal was sent as a turn of
    // the same session.
    assert_eq!(ran.turns.len(), 3, "{:?}", ran.turns);
    assert_eq!(ran.turns[2], "you dropped the scope");
}

#[test]
fn a_document_the_model_fenced_is_written_as_the_document_inside_the_fence() {
    let root = a_root();
    let brief = a_brief();
    let fenced = format!("```markdown\n{brief}```");
    // The path is proposed from the unwrapped document and the bytes are the
    // unwrapped document, which is `writing.rs`'s rule and not a second one
    // here: the slug comes off the `# ` line inside the fence. Asked before the
    // run, for the reason the first of these tests asks before it.
    let proposed = proposal_for(root.path(), &fenced);
    assert_eq!(
        proposed,
        "docs/warlock-brief-01-give-the-headless-cli-a-voice.md"
    );

    let ran = run(
        root.path(),
        &["/write", ""],
        vec![Answering::says("asking"), Answering::says(fenced.as_str())],
    );

    ran.outcome.expect("a document written");
    assert_eq!(wrote(root.path(), &proposed), brief);
}

#[test]
fn nothing_but_prose_and_write_is_understood_at_the_cursor() {
    let root = a_root();

    let ran = run(
        root.path(),
        &[
            // Every other command the panel has, the case-folded spelling of one
            // of them, and a bare slash.
            "/push docs/brief.md",
            "/chat",
            "/BRIEF",
            "/",
            // And a line that opens with a path, which is prose: `submitted_for`
            // reads a second slash as somebody talking about a file.
            "/tmp/notes is where I keep them",
            "",
        ],
        replying(&["asking", "noted"]),
    );

    ran.outcome.expect("an EOF is not a failure");
    assert_eq!(
        ran.said.matches(ONLY_WRITE).count(),
        4,
        "a command word was sent to the model or refused twice: {}",
        ran.said
    );
    // Two turns: the instruction and the one line of prose. Nothing a refusal
    // touched reached the model, and nothing a refusal touched was kept.
    assert_eq!(ran.turns.len(), 2, "{:?}", ran.turns);
    assert_eq!(ran.turns[1], "/tmp/notes is where I keep them");
}

#[test]
fn a_write_turn_that_failed_asks_for_no_path_and_the_conversation_goes_on() {
    let root = a_root();

    let ran = run(
        root.path(),
        &["/write", "carry on", ""],
        vec![
            Answering::says("asking"),
            Answering::missing(),
            Answering::says("still here"),
        ],
    );

    ran.outcome.expect("a turn that could not run is a line");
    let ending = Ending::NoModel {
        program: "claude".to_owned(),
    };
    assert!(ran.said.contains(&ending.line()), "{}", ran.said);
    assert!(
        !ran.said.contains("the document goes to"),
        "a path was offered for a document that never arrived: {}",
        ran.said
    );
    // The `/write`, the line under it, the empty line that sent it and the EOF
    // that ended the run: no path was ever asked for.
    assert_eq!(ran.asked.len(), 4, "{:?}", ran.asked);
    assert_eq!(ran.turns.len(), 3, "{:?}", ran.turns);
}

#[test]
fn an_end_of_file_at_the_path_writes_nothing_and_ends_the_run() {
    let root = a_root();
    let brief = a_brief();

    let ran = run(
        root.path(),
        &["/write"],
        vec![Answering::says("asking"), Answering::says(brief.as_str())],
    );

    ran.outcome.expect("an EOF is not a failure");
    assert!(
        !from_manifest_path(root.path(), DEFAULT_BRIEF_DIRECTORY).exists(),
        "a path nobody answered was written to anyway",
    );
    assert!(
        ran.said.ends_with(&format!("\nwarlock: {OVER}\n")),
        "the run ended some other way: {:?}",
        ran.said
    );
}
