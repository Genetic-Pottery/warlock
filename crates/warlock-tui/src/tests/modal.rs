use warlock_engine::Destination;

use super::{Modal, Modals};
use crate::confirm::{Carry, CutConfirm, PushConfirm, QuitConfirm, Review};
use crate::prompt::{RecordPrompt, ScopePrompt};

#[test]
fn nothing_up_is_no_modal() {
    assert_eq!(Modals::default().current(), None);
}

#[test]
fn the_precedence_is_quit_the_four_questions_then_the_four_fields() {
    let push = PushConfirm::open(
        "Push a brief to the board",
        Destination::new("warlock-team", "Warlock", "warlock", "work"),
    );
    let cut = CutConfirm::open("A project", "planned", 9, "Warlock", "work");
    let review = Review::open("slice 1", vec!["A draft".to_owned()], true);
    let carry = Carry::open("2 slices");
    let filing = ScopePrompt::open("Which board", "work");
    let scope = ScopePrompt::open("crates/warlock-engine", "data-plane");
    let record = RecordPrompt::open("crates/warlock-engine", "data-plane");
    let write = ScopePrompt::open("Write the brief to", "docs/brief.md");

    // Every window up at once, which no session reaches, then each taken down
    // in turn: what is left on top at each step is the whole order.
    let mut modals = Modals {
        quit: QuitConfirm::open(),
        push: &push,
        cut: &cut,
        review: Some(&review),
        carry: Some(&carry),
        filing: &filing,
        scope: &scope,
        record: &record,
        write: &write,
    };
    let mut seen = Vec::new();
    while let Some(modal) = modals.current() {
        seen.push(modal);
        match modal {
            Modal::Quit(_) => modals.quit = QuitConfirm::Closed,
            Modal::Push(_) => modals.push = &PushConfirm::Closed,
            Modal::Cut(_) => modals.cut = &CutConfirm::Closed,
            Modal::Review(_) => modals.review = None,
            Modal::Carry(_) => modals.carry = None,
            Modal::Filing(_) => modals.filing = &ScopePrompt::Closed,
            Modal::Scope(_) => modals.scope = &ScopePrompt::Closed,
            Modal::Record(_) => modals.record = &RecordPrompt::Closed,
            Modal::Write(_) => modals.write = &ScopePrompt::Closed,
        }
    }

    assert_eq!(
        seen,
        [
            Modal::Quit(QuitConfirm::open().highlighted().expect("open")),
            Modal::Push(push.filing().expect("open")),
            Modal::Cut(cut.cutting().expect("open")),
            Modal::Review(&review),
            Modal::Carry(&carry),
            Modal::Filing(filing.field().expect("open")),
            Modal::Scope(scope.field().expect("open")),
            Modal::Record(record.form().expect("open")),
            Modal::Write(write.field().expect("open")),
        ]
    );
}
