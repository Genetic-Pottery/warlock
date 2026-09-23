use crate::confirm::{
    Answer, Carry, CutConfirm, Cutting, Filing, PushConfirm, QuitConfirm, Review,
};
use crate::prompt::{RecordForm, RecordPrompt, ScopeField, ScopePrompt};

// Three variants over `ScopeField` rather than one carrying a tag, so the field
// a key is typed into and the heading it is drawn under cannot come from two
// different windows: the binary answers a submit from each differently, and the
// frame words each differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal<'a> {
    Quit(Answer),
    Push(&'a Filing),
    Cut(&'a Cutting),
    Review(&'a Review),
    Carry(&'a Carry),
    Filing(&'a ScopeField),
    Scope(&'a ScopeField),
    Record(&'a RecordForm),
    Write(&'a ScopeField),
}

// Named fields rather than positional arguments: four of these are prompts of
// one type, and handed in by position they could be swapped with nothing to
// catch it. Each window's state stays with the flow that owns it — the cut
// owns its three, the push its two, the chat its write prompt — and more than
// one can be up at once, since a `/write` turn or a board's answer opens its
// window with no keystroke. So this does not make two open impossible; it makes
// [`Modals::current`] the one answer to which of them counts.
#[derive(Debug, Clone, Copy)]
pub struct Modals<'a> {
    pub quit: QuitConfirm,
    pub push: &'a PushConfirm,
    pub cut: &'a CutConfirm,
    pub review: Option<&'a Review>,
    pub carry: Option<&'a Carry>,
    pub filing: &'a ScopePrompt,
    pub scope: &'a ScopePrompt,
    pub record: &'a RecordPrompt,
    pub write: &'a ScopePrompt,
}

impl Default for Modals<'_> {
    fn default() -> Self {
        Self {
            quit: QuitConfirm::Closed,
            push: &PushConfirm::Closed,
            cut: &CutConfirm::Closed,
            review: None,
            carry: None,
            filing: &ScopePrompt::Closed,
            scope: &ScopePrompt::Closed,
            record: &RecordPrompt::Closed,
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
    // drafts arriving, a `/write` turn answering into its prompt.
    //
    // The push, cut, review and carry questions before the three fields, for
    // the same reason one step down: a field can come up under one of them on no
    // keystroke, and the question is the window somebody is looking at. Among
    // those four and the filing field the order is a statement rather than a
    // choice: a `/push` or `/draft` is typed into the composer, which takes no
    // keys while any of them is up; the filing field's submit is what puts the
    // push dialog up; and a slice is being reviewed, or asking whether to carry
    // on, or neither.
    //
    // The two windows the `s` key puts up before the write prompt, because
    // either can be up with it — `s` opens one from the tree while a `/write`
    // turn is out, and that turn's answer opens the other — and the `s` window
    // is the one somebody is typing in. The scope and record windows are never
    // both up: one opens exactly as the other closes.
    #[must_use]
    pub fn current(self) -> Option<Modal<'a>> {
        self.quit
            .highlighted()
            .map(Modal::Quit)
            .or_else(|| self.push.filing().map(Modal::Push))
            .or_else(|| self.cut.cutting().map(Modal::Cut))
            .or_else(|| self.review.map(Modal::Review))
            .or_else(|| self.carry.map(Modal::Carry))
            .or_else(|| self.filing.field().map(Modal::Filing))
            .or_else(|| self.scope.field().map(Modal::Scope))
            .or_else(|| self.record.form().map(Modal::Record))
            .or_else(|| self.write.field().map(Modal::Write))
    }
}

#[cfg(test)]
#[path = "tests/modal.rs"]
mod tests;
