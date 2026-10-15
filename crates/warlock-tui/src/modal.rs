use crate::confirm::{
    Answer, Carry, CutConfirm, Cutting, Filing, PullConfirm, PushConfirm, QuitConfirm, Review,
    Undertaking,
};
use crate::help::HelpSheet;
use crate::prompt::{KeyChoice, KeyPrompt, RecordForm, RecordPrompt, ScopeField, ScopePrompt};

// Two variants over `ScopeField` rather than one carrying a tag, so the field
// a key is typed into and the heading it is drawn under cannot come from two
// different windows: the session answers a submit from each differently, and
// the frame words each differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Modal<'a> {
    Quit(Answer),
    Help(u16),
    Push(&'a Filing),
    Cut(&'a Cutting),
    Pull(&'a Undertaking),
    Review(&'a Review),
    Carry(&'a Carry),
    WriteBrief(Answer),
    Scope(&'a ScopeField),
    Record(&'a RecordForm),
    Key(&'a KeyChoice),
    Write(&'a ScopeField),
}

// Named fields rather than positional arguments: three of these are prompts of
// one type, and handed in by position they could be swapped with nothing to
// catch it. Each window's state stays with the flow that owns it — the cut
// owns its three, the push its dialog, the chat its write prompt — and more than
// one can be up at once, since a brief-mode turn, a write turn or a board's
// answer opens its window with no keystroke. So this does not make two open impossible; it makes
// [`Modals::current`] the one answer to which of them counts.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Modals<'a> {
    pub quit: QuitConfirm,
    pub help: HelpSheet,
    pub push: &'a PushConfirm,
    pub cut: &'a CutConfirm,
    pub pull: &'a PullConfirm,
    pub review: Option<&'a Review>,
    pub carry: Option<&'a Carry>,
    pub write_brief: Option<Answer>,
    pub scope: &'a ScopePrompt,
    pub record: &'a RecordPrompt,
    pub key: &'a KeyPrompt,
    pub write: &'a ScopePrompt,
}

impl Default for Modals<'_> {
    fn default() -> Self {
        Self {
            quit: QuitConfirm::Closed,
            help: HelpSheet::Closed,
            push: &PushConfirm::Closed,
            cut: &CutConfirm::Closed,
            pull: &PullConfirm::Closed,
            review: None,
            carry: None,
            write_brief: None,
            scope: &ScopePrompt::Closed,
            record: &RecordPrompt::Closed,
            key: &KeyPrompt::Closed,
            write: &ScopePrompt::Closed,
        }
    }
}

impl<'a> Modals<'a> {
    // The window that takes the keys, swallows the pointer and is drawn, all
    // three off this one answer: when keys and paint were ordered separately,
    // the quit dialog took the keys while painted under whatever came up after
    // it. Only this one is drawn, so the order below is the whole precedence.
    //
    // Quit first: it is the gate on the way out, and a window can come up under
    // it with nobody pressing anything — a board answering a `/draft`, a slice's
    // drafts arriving, a brief turn asking whether to write, a write turn
    // answering into its prompt.
    //
    // Help second: under quit, so Ctrl-C asks over it, and over everything
    // else, so a window that comes up on no keystroke does not take the keys
    // from a sheet somebody opened and is reading.
    //
    // The push, cut, pull, review, carry and write questions before the four
    // windows under them, for the same reason one step down: one of those four
    // can come up under a question on no keystroke, and the question is the
    // window somebody is looking at. Among those six the order is a statement
    // rather than a choice: a `/push`, `/draft` or `/pull` is typed into the
    // composer, which takes no keys while any of them is up; a slice is being
    // reviewed, or asking whether to carry on, or neither; and the write
    // question goes last because nothing is waiting on it, where a run is
    // waiting on the review and carry questions.
    //
    // The pull question sits with the other two the composer opens rather than
    // with the two a run puts up, because that is what it is: it is asked once,
    // before anything is checked out, and nothing it starts asks anything else.
    //
    // Then the four windows a scope, a record, a binding and a path are
    // settled in, in that order, and the order among them is which of them
    // somebody has just acted in. The two the `s` key puts up come first,
    // because either can be up with the write prompt — `s` opens one from the
    // tree while a write turn is out, and that turn's answer opens the other —
    // and the `s` window is the one somebody is typing in; the scope and record
    // windows are never both up, since one opens exactly as the other closes.
    // The key-binding window comes after those two because a submit of the
    // record form is one of the things that can put it up, so a binding asked
    // for over a record being filled in cannot take the keys from the fields it
    // came out of; and before the write prompt because it is opened by a
    // keystroke or a typed command, where a path prompt comes up on a turn's
    // answer with nobody pressing anything. It is under all six questions
    // above with nothing said about it that is not said about the other three:
    // a push, cut or pull dialog already up keeps the keys, and a binding asked
    // about underneath one waits there until that question is answered.
    #[must_use]
    pub(crate) fn current(self) -> Option<Modal<'a>> {
        self.quit
            .highlighted()
            .map(Modal::Quit)
            .or_else(|| self.help.scroll().map(Modal::Help))
            .or_else(|| self.push.filing().map(Modal::Push))
            .or_else(|| self.cut.cutting().map(Modal::Cut))
            .or_else(|| self.pull.undertaking().map(Modal::Pull))
            .or_else(|| self.review.map(Modal::Review))
            .or_else(|| self.carry.map(Modal::Carry))
            .or_else(|| self.write_brief.map(Modal::WriteBrief))
            .or_else(|| self.scope.field().map(Modal::Scope))
            .or_else(|| self.record.form().map(Modal::Record))
            .or_else(|| self.key.choice().map(Modal::Key))
            .or_else(|| self.write.field().map(Modal::Write))
    }
}

#[cfg(test)]
#[path = "tests/modal.rs"]
mod tests;
