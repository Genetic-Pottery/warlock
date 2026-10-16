use std::path::Path;

use warlock_engine::Destination;

use super::{Modal, Modals};
use crate::confirm::{Answer, Carry, CutConfirm, PullConfirm, PushConfirm, QuitConfirm, Review};
use crate::help::HelpSheet;
use crate::prompt::{KeyPrompt, RecordPrompt, RecordTracker, ScopePrompt};

// A home nothing has ever written, which is a key window with no rows: what the
// precedence is about is which window takes the keys, and a window with no rows
// takes them exactly as one with three does. No store is read off the machine
// running this and no key value is anywhere near it.
const NO_HOME: &str = "/warlock/no/such/home";

#[test]
fn nothing_up_is_no_modal() {
    assert_eq!(Modals::default().current(), None);
}

#[test]
fn the_precedence_is_quit_help_the_six_questions_then_the_four_windows_under_them() {
    let push = PushConfirm::open(
        "Push a brief to the board",
        Destination::new("warlock-team", "Warlock", "warlock", "work"),
    );
    let cut = CutConfirm::open("A project", "planned", 9, "Warlock", "work");
    let pull = PullConfirm::open(
        "WAR-143",
        "Add /pull and /resume to the panel",
        "warlock-team",
        "Warlock",
        "war-143/add-pull-and-resume",
    );
    let review = Review::open("slice 1", vec!["A draft".to_owned()]);
    let carry = Carry::open("2 slices");
    let scope = ScopePrompt::open("crates/warlock-engine", "data-plane");
    let record = RecordPrompt::open(RecordTracker::Linear, "crates/warlock-engine", "data-plane");
    let key = KeyPrompt::open(Path::new(NO_HOME), "data-plane")
        .expect("a home with no store opens a window with no rows");
    let write = ScopePrompt::open("Write the brief to", "docs/brief.md");

    // Every window up at once, which no session reaches, then each taken down
    // in turn: what is left on top at each step is the whole order.
    let mut modals = Modals {
        quit: QuitConfirm::open(),
        help: HelpSheet::Open(3),
        push: &push,
        cut: &cut,
        pull: &pull,
        review: Some(&review),
        carry: Some(&carry),
        write_brief: Some(Answer::No),
        scope: &scope,
        record: &record,
        key: &key,
        write: &write,
    };
    let mut seen = Vec::new();
    while let Some(modal) = modals.current() {
        seen.push(modal);
        match modal {
            Modal::Quit(_) => modals.quit = QuitConfirm::Closed,
            Modal::Help(_) => modals.help = HelpSheet::Closed,
            Modal::Push(_) => modals.push = &PushConfirm::Closed,
            Modal::Cut(_) => modals.cut = &CutConfirm::Closed,
            Modal::Pull(_) => modals.pull = &PullConfirm::Closed,
            Modal::Review(_) => modals.review = None,
            Modal::Carry(_) => modals.carry = None,
            Modal::WriteBrief(_) => modals.write_brief = None,
            Modal::Scope(_) => modals.scope = &ScopePrompt::Closed,
            Modal::Record(_) => modals.record = &RecordPrompt::Closed,
            Modal::Key(_) => modals.key = &KeyPrompt::Closed,
            Modal::Write(_) => modals.write = &ScopePrompt::Closed,
        }
    }

    assert_eq!(
        seen,
        [
            Modal::Quit(QuitConfirm::open().highlighted().expect("open")),
            Modal::Help(3),
            Modal::Push(push.filing().expect("open")),
            Modal::Cut(cut.cutting().expect("open")),
            Modal::Pull(pull.undertaking().expect("open")),
            Modal::Review(&review),
            Modal::Carry(&carry),
            Modal::WriteBrief(Answer::No),
            Modal::Scope(scope.field().expect("open")),
            Modal::Record(record.form().expect("open")),
            Modal::Key(key.choice().expect("open")),
            Modal::Write(write.field().expect("open")),
        ]
    );
}
