//! The panel: three cards, one of them showing, and the window a renderer draws
//! from it.
//!
//! The right-hand pane of warlock's screen. It holds an [`Account`] of the run
//! that is happening or last happened, a [`Thread`] of the conversation, and the
//! lines of a document somebody pressed `v` on — and shows exactly one of them
//! at a time, with the swap key stepping between the ones there is anything to
//! see in.
//!
//! # Why it is a module
//!
//! Everything here was a private struct inside `app.rs` with twenty-three
//! [`App`](crate::App) methods reaching into it. That is what a module looks
//! like before it has been given one: the fields were separated already and the
//! behaviour was not, so a question about card-swapping or panel scrolling could
//! only be asked by building a whole `App` — which means a `Tree`, a row list, a
//! selection and a scroll offset, none of which the panel has ever had an
//! opinion about.
//!
//! `App` still forwards every one of those methods, because the renderer and the
//! event loop reach the panel through the app and nothing is bought by making
//! two hundred call sites say `app.panel()` instead. What changed is where the
//! behaviour lives and what can be tested without the rest of the screen.
//!
//! # The three cards and the one that is showing
//!
//! A [`Card<T>`] is a held value plus how far it is scrolled and whether it is
//! following its own tail. [`Showing`] says which card the keys are driving, and
//! it is a closed three-value enum for the reason [`Focus`](crate::Focus) is
//! one: the panel holds three named things, not an index into a list somebody
//! could grow. There is no `Nothing` variant — an empty card showing draws
//! warlock's mark, which is a fact about the card rather than a fourth thing to
//! be showing.

use std::time::Instant;

use crate::account::{Account, Line};
use crate::app::cut_at_cap_message;
use crate::claude::Activity;
use crate::thread::{Ending, Thread};
use crate::wrap::rows as wrap_rows;

/// One slot, three cards: the account of the run, the conversation somebody is
/// having, and the document somebody asked to read, with a window apiece and
/// one bit saying which of them is on screen.
///
/// The group that survives everything. A reload carries it, because the tree is
/// read again *because* a run finished and dropping it would wipe the record at
/// the moment the reader turned to read it; and a run that ends with nothing
/// recorded carries it too, because an account is not a claim about the tree and
/// has no business being rolled back with one. That second rule used to be
/// `App::take_account_from`, a method whose whole purpose was to reach into a
/// live app and steal back the four fields that must not roll back. It is a
/// field move now.
///
/// `account`, `thread` and `document` are the cards, and there are exactly
/// three: no list, no stack, no history and no fourth card, so "which card is
/// showing" is [`Showing`]'s three variants and nothing anybody can grow. Each
/// card is either empty or filled — filled by [`App::start_account`],
/// [`App::start_turn`] and [`App::show_document`] respectively — and a filled
/// card stays filled: reading a file does not throw the account away, a pact
/// starting does not throw the document away, and neither of them empties the
/// conversation. That is the whole point of the slot being three cards rather
/// than one thing at a time: a reader with a document up keeps it while a run
/// goes on filling the card behind it, and the answer to a question they asked
/// a minute ago is still there when they swap back to it.
///
/// A run is written to exactly one of the cards — its own — however many of the
/// three are filled and whichever one the reader is looking at (see
/// [`App::start_account`] and [`App::write_run`]). A conversation carrying the
/// same run would be one run written twice on one screen, and a card swapped
/// away under the reader would be worse: what is showing is theirs.
///
/// A card being filled is not the same as it having lines. An app that has never
/// run a pact has nothing to draw in the panel at all, not even a heading, while
/// an account that has started and recorded nothing yet is a run under way — and
/// the difference between the two is a blank panel and one that has started. A
/// second pact starts a second account rather than appending to the first: one
/// pact, one account. The thread is the one card that is the other way round —
/// one session, one conversation, so a second question goes under the first
/// rather than in place of it. See `Card::accrue`.
///
/// `showing` is which card the panel draws, and it moves for exactly three
/// reasons: the view key bringing a document to the front ([`App::show_document`]),
/// a message submitted below bringing the thread to it ([`App::start_turn`]), and
/// the swap key. A run never moves it — a run fills its own card wherever the
/// reader is looking — and neither does a document being read again under the
/// reader after `$EDITOR` rewrote it ([`App::refill_document`]), nor anything a
/// turn already under way reports.
///
/// Each card carries its own `offset` and `follows`, which is what lets the
/// account go on following the newest line while the document is up and lets the
/// reader come back to the line they left. `height` and `width` are the two things all
/// three cards share, because there is one panel and it is as tall and as wide as the
/// frame says. The width is only ever *used* by two of them — a document is
/// wrapped to it and so is an answer, where an account is cut to it by the
/// renderer instead (see [`mod@crate::wrap`]) — but it is a fact about the panel
/// rather than about any one card, and it sits beside the height for that reason.
/// Together they are to the panel what `viewport_height` and `scroll_offset` are
/// to the tree — with one difference, which is the flag. The tree's window is
/// dragged about by a selection; the panel has no selection, so a card's window
/// is either pinned to the newest line or parked where the reader put it, and
/// `follows` says which. While it is set, `offset` is not read at all: the
/// offset is the end of the card, worked out from the line count at the moment
/// it is asked for, so appending a line moves the window without anybody having
/// to tell the window that a line was appended. See
/// [`App::panel_scroll_offset`].
///
/// `mode` is which register the conversation is in — see [`Mode`] — and it is
/// in this group rather than beside `focus` for the reason everything else here
/// is: it has to survive a run that ended with nothing recorded. A reader who
/// has spent ten turns converging on a brief and then pacts a clean directory
/// would otherwise find the conversation back in chat mode because a *pact*
/// rolled back, which is a run reaching into a conversation it has nothing to
/// do with. It is a fact about the card and not about the slot: the mode is
/// what the thread is, whichever of the three cards is being looked at.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Panel {
    account: Card<Account>,
    thread: Card<Thread>,
    document: Card<Vec<Line>>,
    showing: Showing,
    mode: Mode,
    height: usize,
    width: usize,
}

/// Which of the panel's three cards is on screen.
///
/// Three variants and no fourth, for [`Focus`]'s reason: the panel holds an
/// account, a conversation and a document, so "which card is showing" is one of
/// three named things, not an index into a list somebody could grow. There is no
/// `Nothing` here — a slot with an empty card showing draws warlock's mark,
/// which is a fact about the card rather than a fourth thing to be showing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) enum Showing {
    /// The account of the pact running now, or of the last one to run. Reached
    /// only once a pact has run: an empty one is a card about nothing, and the
    /// swap key steps over it.
    Account,
    /// The conversation: every question typed below and what came back, and the
    /// card the panel opens on.
    ///
    /// The one card that is always somewhere to be. It is where the field is
    /// drawn, so it is where a session starts and where the swap key can always
    /// go back to — empty, it draws warlock's mark over a field with nothing
    /// typed in it yet, which is the whole of what an app that has just started
    /// has to say.
    #[default]
    Thread,
    /// The document last read. Reached only once [`App::show_document`] has put
    /// one there, and only by the swap key after that.
    Document,
}

impl Showing {
    /// The card after this one: what [`App::swap_card`] shows next.
    ///
    /// [`Focus::next`] for the slot, and written the same way and for the same
    /// reason — an arm per card, so "the panel is showing one of these" stays
    /// the thing the type says and a fourth card is a compile error here rather
    /// than a swap that quietly went nowhere. The order is the order the reader
    /// comes to them in: the conversation they start on, the run they asked for,
    /// then the file it wrote.
    const fn next(self) -> Self {
        match self {
            Self::Thread => Self::Account,
            Self::Account => Self::Document,
            Self::Document => Self::Thread,
        }
    }
}

/// Which register the conversation is in: questions about the repository, or a
/// conversation converging on a document.
///
/// Two variants and no third. A mode here is not a second system prompt and not
/// a second session — it is a state warlock holds plus one ordinary turn sent
/// into the conversation already in progress (see
/// [`brief_instruction`](crate::brief_instruction) and
/// [`CHAT_INSTRUCTION`](crate::CHAT_INSTRUCTION)) — so what the app keeps is
/// only this word, and everything the word changes is said out loud somewhere
/// else: which instruction a command sends, how hard the turn is asked to think
/// (see [`BRIEF_EFFORT`](crate::BRIEF_EFFORT)), and which model is asked (see
/// [`BRIEF_MODEL`](crate::BRIEF_MODEL)).
///
/// What it does *not* change is the card. Nothing on the thread is cleared,
/// hidden or reordered by a mode change, the session is the same session, the
/// system prompt is the same string, and the tool grant is byte-identical.
///
/// It is drawn in exactly one place: the panel's border title, where the thread
/// already says which card it is (see `draw_panel` in [`mod@crate::ui`]). Not on
/// the run header's row, which is the fixed row a pact takes over the panel and
/// would collide with a brief started while a run is in flight, and not on a row
/// of the card, which would be a row of the conversation spent saying what the
/// border was already able to say for nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Questions about the repository, answered one at a time: the register
    /// warlock opens in and the one it goes back to on `/chat`.
    #[default]
    Chat,
    /// The conversation is aimed at an artifact, and the model's job is to argue
    /// toward a decision about it rather than to answer as asked. Entered by
    /// `/brief` from the composer, and by nothing else — no key, no click and no
    /// tree action puts a conversation in this mode.
    Brief,
}

/// One card of the panel: what it holds, if anything, and where its window sits
/// over it.
///
/// Generic over what is on the card because the window rule is not: an offset,
/// a follow flag and a height decide where a list of lines is cut, whether that
/// list is an account being written or a file that was read once. What differs
/// is only how the lines are counted and produced, which is [`Shown`]'s two
/// implementations.
///
/// `held` is `None` for a card nothing has filled yet, and that is not the same
/// as a card holding an empty list: an unfilled card is a panel with nothing to
/// say, and an empty account or an empty file is something having happened. See
/// [`App::has_panel_content`].
#[derive(Debug, Clone, PartialEq)]
struct Card<T> {
    held: Option<T>,
    offset: usize,
    follows: bool,
}

/// An empty card whose window is at the top and following nothing: what both
/// cards of a freshly built [`App`] are.
///
/// Written out rather than derived because a derived `Default` would demand one
/// of `T` as well, and [`Account`] has none: an account is a run that started at
/// some instant, and there is no default instant.
impl<T> Default for Card<T> {
    fn default() -> Self {
        Self {
            held: None,
            offset: 0,
            follows: false,
        }
    }
}

impl<T: Shown> Card<T> {
    /// Put `held` on the card, with its window at the top and `follows` saying
    /// whether it is pinned to the newest line.
    ///
    /// Whatever the card was holding goes: one pact is one account, and one
    /// press of the view key is one document.
    fn place(&mut self, held: T, follows: bool) {
        self.held = Some(held);
        self.offset = 0;
        self.follows = follows;
    }

    /// What is on the card, ready to be added to, filling an empty card first
    /// and pinning its window to the newest line.
    ///
    /// The append path, and deliberately not a second [`Card::place`]: `place`
    /// drops what the card held, which is right for a run and for a read and
    /// wrong for a conversation. One session is one thread, so a question asked
    /// ten minutes in goes under the nine before it rather than in place of
    /// them, and only the first of them fills the card at all.
    ///
    /// It follows, every time. Somebody who has just asked something is asking
    /// to see the answer, so the window goes back to the newest line even if
    /// they had scrolled up to re-read an older turn — the one thing that moves
    /// a card's window without the reader having pressed a movement key, and it
    /// is their own keystroke that does it.
    fn accrue(&mut self) -> &mut T
    where
        T: Default,
    {
        self.follows = true;
        self.held.get_or_insert_with(T::default)
    }

    /// How many rows the card draws as at `width`, or none at all while nothing
    /// has filled it.
    ///
    /// A count of rows on screen rather than of lines held: a document line too
    /// long for the width is drawn in several rows, and this is what the window
    /// is cut out of, so it is the wrapped count or the arithmetic below it
    /// would be about a screen nobody is looking at.
    fn line_count(&self, width: usize) -> usize {
        self.held.as_ref().map_or(0, |held| held.line_count(width))
    }

    /// Which row of the card is drawn at the top row of a window `height` rows
    /// tall and `width` columns wide.
    fn scroll_offset(&self, height: usize, width: usize) -> usize {
        panel_offset_for(self.line_count(width), height, self.offset, self.follows)
    }

    /// The `height` rows the card's window covers, with every clock measured
    /// against `now`.
    fn window(&self, height: usize, width: usize, now: Instant) -> Vec<Line> {
        self.held.as_ref().map_or_else(Vec::new, |held| {
            held.window(self.scroll_offset(height, width), height, width, now)
        })
    }

    /// How many of the card's rows sit below a window `height` rows tall.
    fn lines_below(&self, height: usize, width: usize) -> usize {
        self.line_count(width)
            .saturating_sub(self.scroll_offset(height, width) + height)
    }

    /// Park the card's window at `offset`, or as near to it as the card's own
    /// length allows, and say whether that is still following.
    fn scroll_to(&mut self, offset: usize, height: usize, width: usize) {
        // Where the end is, asked of the one function that decides it, so that
        // "as far down as this card goes" means the same thing to a keystroke as
        // it does to the frame being drawn.
        let end = panel_offset_for(self.line_count(width), height, 0, true);
        self.offset = offset.min(end);
        self.follows = self.offset == end;
    }
}

/// What a card can hold: something with lines the panel can count and cut a
/// window out of.
///
/// The three implementations are the three cards — an [`Account`], which words
/// its own lines and has clocks in them, a [`Thread`], which has both clocks and
/// prose, and a document, which is the lines themselves. A document has no
/// clock, so `now` reaches only the other two; the panel draws one list whichever
/// card is up, and which list it is is this trait's business rather than the
/// renderer's.
trait Shown {
    /// How many rows this draws as in a panel `width` columns wide.
    fn line_count(&self, width: usize) -> usize;

    /// The `height` rows starting at `offset` in a panel `width` columns wide,
    /// with any clocks measured against `now`.
    fn window(&self, offset: usize, height: usize, width: usize, now: Instant) -> Vec<Line>;
}

/// An account is wrapped like everything else the panel draws: a pass that
/// reports a long tool detail, or fails with a whole sentence of somebody else's
/// stderr, has that line broken across the rows it needs rather than cut off at
/// the edge (see [`mod@crate::wrap`]). What separates one thing that happened
/// from the next is then the clock in front of it, since the count of rows is no
/// longer the count of events.
///
/// The count is taken from the account's own start, which is the one instant an
/// account that has rows can always name. `now` decides what a clock *says*
/// rather than whether it is a row — with the one exception that a clock going
/// from `9:59` to `10:00` takes a column off the line beside it, which can move
/// a line that was exactly one column inside the width onto a second row. The
/// cost of that is one row of scrollback being a frame behind, which the next
/// frame settles.
impl Shown for Account {
    fn line_count(&self, width: usize) -> usize {
        self.lines(self.started())
            .iter()
            .map(|line| rows_of(line, width).len())
            .sum()
    }

    fn window(&self, offset: usize, height: usize, width: usize, now: Instant) -> Vec<Line> {
        self.lines(now)
            .iter()
            .flat_map(|line| rows_of(line, width))
            .skip(offset)
            .take(height)
            .collect()
    }
}

/// A thread is every kind of line the panel has in one card — a question, the
/// account's own work lines, prose — and every one of them goes through the same
/// `rows_of` at the panel's width, so a terminal dragged narrower re-flows the
/// turn a reader is looking at rather than asking the model again. What differs
/// between them is only where a continuation row sits: under the marker for a
/// question, under the clock for a work line, flush left for the answer.
impl Shown for Thread {
    fn line_count(&self, width: usize) -> usize {
        // Counted from the rows themselves rather than from a second formula
        // over the turns, so the count and the window cannot come to disagree
        // about what a width does. Any instant answers: `now` decides what a
        // clock *says* and never whether it is a row, and nothing else in a
        // turn moves — so the first entry's own instant, the one a thread that
        // has rows can always name, is as good as the frame's.
        self.started().map_or(0, |started| {
            self.lines(started)
                .iter()
                .map(|line| rows_of(line, width).len())
                .sum()
        })
    }

    fn window(&self, offset: usize, height: usize, width: usize, now: Instant) -> Vec<Line> {
        self.lines(now)
            .iter()
            .flat_map(|line| rows_of(line, width))
            .skip(offset)
            .take(height)
            .collect()
    }
}

/// A document is its lines, already worded — the file's own lines, and the one
/// sentence about a read the cap cut short, which is the only line in it the
/// file did not write. [`App`] never holds the path it came from and never opens
/// anything: what reaches it is text, from whoever did the reading.
///
/// It is the card the width reaches, and a line of it is drawn in as many rows
/// as the width needs — see [`mod@crate::wrap`] for why a file is wrapped where
/// an account is cut. The lines held are the file's own either way: wrapping
/// happens on the way to the screen, at whatever width the frame is, so a
/// terminal made narrower re-flows the document a reader is looking at rather
/// than re-reading it.
impl Shown for Vec<Line> {
    fn line_count(&self, width: usize) -> usize {
        self.iter().map(|line| rows_of(line, width).len()).sum()
    }

    fn window(&self, offset: usize, height: usize, width: usize, _now: Instant) -> Vec<Line> {
        self.iter()
            .flat_map(|line| rows_of(line, width))
            .skip(offset)
            .take(height)
            .collect()
    }
}

/// The rows one line of a card draws as at `width`.
///
/// One row wherever the line fits, and the rows it needs where it does not: the
/// wrap module's answer, not this one's, so that what the app counts and what
/// the renderer draws are the same rows from the same code. See
/// [`rows`](crate::wrap::rows), which is also where the shape of a continuation
/// row is decided.
fn rows_of(line: &Line, width: usize) -> Vec<Line> {
    wrap_rows(line, width)
}

impl Panel {
    /// Which row of the showing card is drawn at the panel's top row.
    pub(crate) fn scroll_offset(&self) -> usize {
        match self.showing {
            Showing::Account => self.account.scroll_offset(self.height, self.width),
            Showing::Thread => self.thread.scroll_offset(self.height, self.width),
            Showing::Document => self.document.scroll_offset(self.height, self.width),
        }
    }

    /// The rows the panel draws now, from the showing card.
    pub(crate) fn window(&self, now: Instant) -> Vec<Line> {
        match self.showing {
            Showing::Account => self.account.window(self.height, self.width, now),
            Showing::Thread => self.thread.window(self.height, self.width, now),
            Showing::Document => self.document.window(self.height, self.width, now),
        }
    }

    /// How many of the showing card's rows sit below the panel.
    pub(crate) fn lines_below(&self) -> usize {
        match self.showing {
            Showing::Account => self.account.lines_below(self.height, self.width),
            Showing::Thread => self.thread.lines_below(self.height, self.width),
            Showing::Document => self.document.lines_below(self.height, self.width),
        }
    }

    /// Move the showing card's window, and only that card's: the other two keep
    /// the line the reader left them on, and an account or a thread left
    /// following goes on following.
    pub(crate) fn scroll_to(&mut self, offset: usize) {
        let (height, width) = (self.height, self.width);
        match self.showing {
            Showing::Account => self.account.scroll_to(offset, height, width),
            Showing::Thread => self.thread.scroll_to(offset, height, width),
            Showing::Document => self.document.scroll_to(offset, height, width),
        }
    }
}

/// Where the panel's window onto an account of `lines` lines should start, given
/// a window `viewport` lines tall, a window parked at `offset`, and whether it is
/// `following` the newest line.
///
/// The rule the tree's `scroll_offset_for` cannot be: the tree's window is
/// dragged about by a selection and moves as little as it can, while the panel's
/// is either at the end of a list that is still being written or exactly where
/// the reader left it. So there is no minimum-movement case here, and no
/// selection either — two inputs decide it. Following, the answer is the last
/// screenful, whatever the account's length is *now*, which is what pins the
/// newest line to the bottom row as lines arrive. Parked, the answer is the
/// reader's own offset, clamped to what the account allows so that a window can
/// never hang past the end.
///
/// A window at least as tall as the account has nothing to scroll: everything is
/// on screen, so the top is the end and following it is standing still. A window
/// no lines tall — which is what a panel nobody has drawn has — has no screen to
/// scroll at all, and the honest offset for it is the top, exactly as the tree's
/// rule says of a viewport of zero rows.
///
/// Pure, and deliberately free of [`App`]: it takes three numbers and a flag and
/// returns one number, so every edge of it is testable with no account, no tree
/// and no terminal.
pub(crate) fn panel_offset_for(
    lines: usize,
    viewport: usize,
    offset: usize,
    following: bool,
) -> usize {
    if viewport == 0 {
        return 0;
    }
    let end = lines.saturating_sub(viewport);
    if following { end } else { offset.min(end) }
}

/// The panel's own operations: the three cards, which one is showing,
/// what mode the conversation is in, and the window the renderer draws.
///
/// Every one of these used to be a method on [`App`](crate::App) that
/// touched `self.panel` and nothing else. They are here so that the panel
/// can be built, driven and asserted about without a tree, a row list, a
/// selection or a scroll offset anywhere near it; `App` keeps a forward for
/// each, because the renderer and the event loop reach the panel through the
/// app and there is nothing to be gained by making them say so twice.
impl Panel {
    /// Which card is on screen.
    #[cfg(test)]
    pub(crate) const fn showing(&self) -> Showing {
        self.showing
    }

    /// Where `card` would be if the reader swapped to it: the line at the top of
    /// its window, and whether it is still following its own newest line.
    ///
    /// About the card rather than about the panel, which is the whole point —
    /// [`Panel::scroll_offset`] answers for the one showing, and what a reader
    /// needs to know is that the other two kept their place while it was not.
    #[cfg(test)]
    pub(crate) fn window_of(&self, card: Showing) -> (usize, bool) {
        // An arm each rather than one over a borrowed card, because the three
        // hold different things and so are three different types. The shape is
        // the same in all three, which is what `Shown` is for.
        match card {
            Showing::Account => (
                self.account.scroll_offset(self.height, self.width),
                self.account.follows,
            ),
            Showing::Thread => (
                self.thread.scroll_offset(self.height, self.width),
                self.thread.follows,
            ),
            Showing::Document => (
                self.document.scroll_offset(self.height, self.width),
                self.document.follows,
            ),
        }
    }

    /// Every line the document card holds, whole, whatever the window is over
    /// and whichever card is showing.
    ///
    /// The card underneath the window rather than the window itself, so a test
    /// can say the lines themselves never moved rather than that a cut of them
    /// looked the same.
    #[cfg(test)]
    pub(crate) fn document_lines(&self) -> &[Line] {
        self.document.held.as_deref().unwrap_or_default()
    }

    /// Put `card` on screen.
    ///
    /// Unconditional: the callers that care whether there is anything in it ask
    /// [`Panel::next_card`] first, and the one that does not — a run starting on
    /// an empty panel — wants the account up whether it has a line in it yet or
    /// not.
    pub(crate) const fn show(&mut self, card: Showing) {
        self.showing = card;
    }

    /// Start a fresh account, following its own tail.
    ///
    /// The card is replaced rather than appended to: an account is the record of
    /// one run, and the run starting is the moment the last one stops being the
    /// thing on screen.
    pub(crate) fn open_account(&mut self, at: Instant) {
        self.account.place(Account::new(at), true);
    }

    /// Fill the document card and show it.
    ///
    /// One operation rather than two, because a document nobody was shown is a
    /// file read for nothing: every caller that refills wants it up.
    pub(crate) fn show_document(
        &mut self,
        lines: impl IntoIterator<Item = impl Into<String>>,
        cut: bool,
    ) {
        self.refill_document(lines, cut);
        self.showing = Showing::Document;
    }

    /// Whether the card on screen has anything in it.
    pub(crate) const fn has_content(&self) -> bool {
        match self.showing {
            Showing::Account => self.has_account(),
            Showing::Thread => self.has_thread(),
            Showing::Document => self.has_document(),
        }
    }

    /// Whether the swap key stops on `card`, which it does when there is
    /// something in it.
    ///
    /// The conversation is always somewhere to be — empty, it draws warlock's
    /// mark over a field with nothing typed in it — so a swap can always get
    /// back to it. The other two are reached only once there is a run to read or
    /// a file to look at.
    const fn stops_on(&self, card: Showing) -> bool {
        match card {
            Showing::Account => self.has_account(),
            Showing::Thread => true,
            Showing::Document => self.has_document(),
        }
    }

    /// The card the swap key shows next, or `None` when there is nowhere to go.
    ///
    /// Two steps at most, because there are three cards and the one showing is
    /// not a candidate: if neither of the others has anything in it, the answer
    /// is that the key did nothing and the caller says so out loud.
    pub(crate) fn next_card(&self) -> Option<Showing> {
        let next = self.showing.next();
        [next, next.next()]
            .into_iter()
            .find(|&card| self.stops_on(card))
    }
    /// Do `write` to the account of the run in flight.
    ///
    /// The one way a run's events reach the panel. It takes a closure rather
    /// than the event because [`Account`] already knows how to word every line a
    /// run will ever have — opening a directory's section, recording an activity
    /// or a summarising pass, closing a section with its outcome, finishing the
    /// run — and a method here per kind of event would be a second place a run's
    /// line could come to be spelled.
    ///
    /// Does nothing when there is no account to write to, which is a run nobody
    /// started through [`App::start_account`]: a test driving events straight
    /// down the channel. Dropping the line is the honest way to fail.
    pub(crate) fn write_run(&mut self, write: impl FnOnce(&mut Account)) {
        if let Some(account) = self.account.held.as_mut() {
            write(account);
        }
    }

    /// Put the lines of a file on the document card again, leaving which card is
    /// showing exactly as it was.
    ///
    /// [`App::show_document`] without the one thing the view key does, and the
    /// two are one method plus a line for that reason: `v` is a reader asking to
    /// look at a file, so it brings the file to the front; this is the file
    /// somebody has just edited being read again underneath them, and a panel
    /// that flipped to the document because a `WARLOCK.md` was saved would take
    /// the account of a run out of the reader's hands without their having
    /// pressed anything. So the bit that says which card is drawn is not touched
    /// here at all: a document showing stays showing, an account showing stays
    /// showing, and the card behind is filled either way.
    ///
    /// *Which* file the card holds is not known here and is deliberately not kept
    /// here: what arrives is text and a `cut`, never a path, exactly as
    /// [`App::show_document`] documents — [`App`] opens nothing, so a path on it
    /// would be a path something later had to open. Whoever pressed the key knows
    /// which file it read, and it is that caller who decides this is the same
    /// file and calls this rather than leaving the card alone.
    ///
    /// The window goes back to the top and follows nothing, which is
    /// [`App::show_document`]'s rule and not a second one. The reader's line is
    /// deliberately not kept: the file has been rewritten under them, so line
    /// forty of the file they were reading is not line forty of the file that is
    /// there now, and parking the window at a number would point it at a line
    /// nobody chose. The top of the file is somewhere they can see they are.
    ///
    /// Says nothing and takes down nothing the last keystroke said, for
    /// [`App::show_document`]'s reason: it is not a keystroke's whole answer.
    pub(crate) fn refill_document(
        &mut self,
        lines: impl IntoIterator<Item = impl Into<String>>,
        cut: bool,
    ) {
        let mut lines: Vec<Line> = lines
            .into_iter()
            .map(|text| Line::Text { text: text.into() })
            .collect();
        if cut {
            lines.push(Line::Text {
                text: cut_at_cap_message(),
            });
        }
        self.document.place(lines, false);
    }

    /// Put `message` on the thread as a new turn asked at `at`, and show it.
    ///
    /// What a submitted composer comes to. The message is the reader's own text,
    /// exactly as they typed it and never a path or a prompt this type built:
    /// [`App`] runs nothing and asks nobody, so whoever took the draft starts the
    /// worker themselves and reports back through the three methods below.
    ///
    /// This is the only way an app comes to have a turn in its thread at all, and
    /// one of the three things that decide which card is drawn: somebody who has
    /// just asked a question is looking for the answer, so the conversation comes
    /// to the front and its window goes to the newest line. The card accumulates
    /// rather than being replaced — one session, one conversation, so the turn
    /// goes under every turn before it — which is `Card::accrue`'s whole reason
    /// for existing beside `Card::place`.
    ///
    /// The other two cards are left exactly as they are, lines and windows and
    /// all: a run goes on filling the account behind the conversation, and a
    /// document read an hour ago is still on the line the reader left it on.
    /// The field is drawn under the conversation, so a question asked from a
    /// field that was already on screen leaves the focus exactly where it was.
    ///
    /// Not a keystroke's whole answer: it says nothing and takes down nothing
    /// the last keystroke said, exactly as [`App::start_account`] does not.
    pub(crate) fn start_turn(&mut self, message: impl Into<String>, at: Instant) {
        self.thread.accrue().ask(message, at);
        self.showing = Showing::Thread;
    }

    /// Put one line of warlock's own on the thread at `at`, and show it.
    ///
    /// What warlock says for itself — a draft refused, and later a file written
    /// or a document gone stale — as against a turn, which is something
    /// somebody asked a model. It brings the conversation to the front exactly
    /// as [`Panel::start_turn`] does, and for the same reason: the line is an
    /// answer to what the reader just did, and an answer on a card they are not
    /// looking at is not an answer. The card accumulates, so the note goes under
    /// everything already said rather than replacing it.
    ///
    /// One unclocked row, and no turn is opened, closed or frozen by it — see
    /// [`Thread::note`]. Not a keystroke's whole answer either: it says nothing
    /// on the footer and takes down nothing that is there.
    pub(crate) fn note(&mut self, text: impl Into<String>, at: Instant) {
        self.thread.accrue().note(text, at);
        self.showing = Showing::Thread;
    }

    /// Record what the live turn was seen doing at `at`.
    ///
    /// The tool calls, the thinking and the writing, as they arrive off whatever
    /// channel the turn is reporting over — one clocked line each, in the
    /// account's own words, with no tool result and no fragment of the answer
    /// among them. See [`Thread::record`], which decides all of that; this only
    /// finds the card.
    ///
    /// A no-op when nothing has been asked yet, and when the live turn is over
    /// already — the same rule, one level down, that keeps a late event from
    /// contradicting a line the reader can already see.
    pub(crate) fn record_turn(&mut self, activity: &Activity, at: Instant) {
        if let Some(thread) = self.thread.held.as_mut() {
            thread.record(activity, at);
        }
    }

    /// Land `answer` on the live turn at `at` and close it.
    ///
    /// The one piece of prose warlock draws, kept whole and unwrapped here and
    /// wrapped to the panel's width on the way to the screen. A model that
    /// finished with nothing to say ends the turn instead of answering it — see
    /// [`Thread::answer`] — so a caller with an empty answer in hand needs no
    /// arm of its own.
    ///
    /// A no-op with no live turn, for [`Panel::record_turn`]'s reason.
    pub(crate) fn answer_turn(&mut self, answer: impl Into<String>, at: Instant) {
        if let Some(thread) = self.thread.held.as_mut() {
            thread.answer(answer, at);
        }
    }

    /// Close the live turn at `at` with the one line `ending` makes.
    ///
    /// The cancel and every failure come through here, and every one of them is
    /// one line under whatever had already arrived: a turn stopped after two
    /// tool calls still shows those two tool calls. Nothing is returned and
    /// nothing is thrown: a turn that could not be run is a row in the panel,
    /// which is why [`Ending`] is a value and not an error.
    ///
    /// A no-op with no live turn, for [`Panel::record_turn`]'s reason, and on a
    /// turn that has ended already — the first ending wins, because it is the
    /// one on screen.
    pub(crate) fn end_turn(&mut self, ending: &Ending, at: Instant) {
        if let Some(thread) = self.thread.held.as_mut() {
            thread.end(ending, at);
        }
    }

    /// The conversation this session has had, or `None` before the first
    /// question.
    ///
    /// Reaches the thread card whichever card is showing, exactly as
    /// [`Panel::account`] reaches the account: a turn goes on being answered while
    /// the reader looks at a run, and whether one is in flight is a fact about
    /// the conversation rather than about what is on screen. Read-only — the
    /// four methods above are the whole of how a turn is written — and it is
    /// what a test asks when it wants the turns themselves rather than the rows
    /// the panel would draw them in.
    #[must_use]
    pub(crate) const fn thread(&self) -> Option<&Thread> {
        self.thread.held.as_ref()
    }

    /// Whether a pact has run this session, and so whether the panel has an
    /// account card to draw.
    ///
    /// `false` until [`App::start_account`] and `true` from then on, whichever
    /// card is showing: an account that has finished is still an account, and an
    /// account behind a document is still an account. It is a question about
    /// what the panel *holds*, not about what is on screen.
    #[must_use]
    pub(crate) const fn has_account(&self) -> bool {
        self.account.held.is_some()
    }

    /// Whether anybody has asked anything this session, and so whether the panel
    /// has a thread card to draw.
    ///
    /// `false` until the first [`Panel::start_turn`] and `true` from then on. A
    /// conversation is not started again, so this goes from `false` to `true`
    /// once and never back: a second question goes under the first.
    ///
    /// A run does not make it true. [`App::start_account`] appends a turn to a
    /// conversation that is already there and fills no card that is not, so a
    /// session that has pacted all morning and typed nothing still answers
    /// `false` here — and the swap key goes on stepping over the thread rather
    /// than spending a press on a history nobody has.
    #[must_use]
    pub(crate) const fn has_thread(&self) -> bool {
        self.thread.held.is_some()
    }

    /// Whether a file somebody asked to read is on the panel's document card.
    ///
    /// The same question of the third card, and all three are freely `true` at
    /// once: the slot holds three cards, so a run started under a document fills
    /// one without emptying the others. `false` until the first
    /// [`App::show_document`] of the session.
    #[must_use]
    pub(crate) const fn has_document(&self) -> bool {
        self.document.held.is_some()
    }

    /// Whether the card on screen is the conversation.
    ///
    /// The one question the renderer asks about *which* card is showing, and it
    /// is asked for one reason: a reader must be able to tell the thread from
    /// the account without reading the rows, so the panel says which card it is
    /// on its own edge (see [`draw_panel`](crate::ui)). A run's card and a
    /// file's card each say what they are in every row they draw — a directory
    /// heading and a clock, or the file's own text — where a conversation could
    /// be either at a glance, and the answer to that is a word on the border
    /// rather than a fourth thing colour is made to mean.
    ///
    /// About the slot and not about the thread: a session with ten turns in it
    /// answers `false` here while the reader is looking at the account.
    #[must_use]
    pub(crate) const fn showing_thread(&self) -> bool {
        matches!(self.showing, Showing::Thread)
    }

    /// Which register the conversation is in: [`Mode::Chat`] until somebody
    /// types `/brief`.
    ///
    /// A fact about the conversation and not about what is on screen: an app
    /// showing the account is still in brief mode, exactly as one showing the
    /// account still has a thread. The renderer asks it to word the panel's
    /// title, and whoever is about to send a turn asks it to decide how hard the
    /// turn thinks; nothing else reads it.
    #[must_use]
    pub(crate) const fn mode(&self) -> Mode {
        self.mode
    }

    /// Put the conversation in `mode`, and say whether that changed anything.
    ///
    /// The answer is what a caller needs and cannot easily get back afterwards:
    /// a mode *change* is worth one note in the thread, where `/brief` typed in
    /// brief mode is a re-send that has nothing new to say about the register.
    /// So the comparison happens here, once, rather than in each caller against
    /// a copy of [`Panel::mode`] it had to remember to take first.
    ///
    /// It sets one word and touches nothing else. The card showing does not
    /// move, no turn is started, ended or reordered, nothing is written into the
    /// thread and the run header is not consulted — the note and the turn are
    /// the caller's, and this is only the state they are about.
    pub(crate) fn set_mode(&mut self, mode: Mode) -> bool {
        let changed = self.mode != mode;
        self.mode = mode;
        changed
    }

    /// The account of the pact running now, or of the last one to run, or `None`
    /// before the first pact of the session.
    ///
    /// Reaches the account card whichever card is showing: a run appends to its
    /// own card while the reader looks at a document, and the pulse in the tree
    /// is a fact about the run rather than about what is on screen.
    #[must_use]
    pub(crate) const fn account(&self) -> Option<&Account> {
        self.account.held.as_ref()
    }

    /// The same account, to record what the run has just been seen doing.
    ///
    /// The account words its own lines — see [`Account`] — so this hands it over
    /// rather than wrapping each of its five mutators in a method here that
    /// would only forward. Nothing needs doing afterwards: while the account's
    /// card is following, its window is the end of the account computed on
    /// demand, so a line recorded through here is on screen at the bottom of the
    /// next frame without this type being told about it.
    ///
    /// `None` before the first pact, where there is nothing to record against,
    /// and only then: a run reports into its own card while a document is up,
    /// and finds the account it left there.
    ///
    /// This card and no other, which is also the whole of where a run goes: what
    /// a run reports reaches the panel through [`Panel::write_run`], and this is
    /// the card it writes.
    pub(crate) const fn account_mut(&mut self) -> Option<&mut Account> {
        self.account.held.as_mut()
    }

    /// How many lines of whatever the panel holds fit in it, as last set by
    /// [`Panel::set_height`].
    #[must_use]
    pub(crate) const fn height(&self) -> usize {
        self.height
    }

    /// Tell the app how many lines fit in the panel.
    ///
    /// The panel's [`App::set_viewport_height`], and a field for the same reason:
    /// only the layout knows the height, only the frame knows when it changed,
    /// and a height passed per call would let two callers disagree about the size
    /// of one window. Safe to call every frame.
    ///
    /// Nothing has to be brought back into line afterwards. The offset is
    /// clamped when it is read, and a window that was following is still
    /// following, so a terminal that has just been made shorter or taller still
    /// shows the newest line at the bottom.
    pub(crate) fn set_height(&mut self, height: u16) {
        self.height = usize::from(height);
    }

    /// How many columns wide the panel is, as last set by
    /// [`Panel::set_width`].
    ///
    /// `0` for a panel nothing has measured, which is not a width to wrap a
    /// document at: see [`Panel::set_width`].
    #[must_use]
    pub(crate) const fn width(&self) -> usize {
        self.width
    }

    /// Tell the app how many columns wide the panel is.
    ///
    /// [`Panel::set_height`]'s counterpart, for the same reason and safe to
    /// call every frame in the same way: only the layout knows the width, and a
    /// line of a document is drawn in as many rows as that width needs, so how
    /// many rows the window is cut out of depends on it exactly as the size of
    /// the window depends on the height.
    ///
    /// Nothing is brought back into line afterwards, and here that is worth
    /// saying out loud: a terminal made narrower gives a document more rows than
    /// it had, so the offset the reader parked at is a row further up the file
    /// than it was. The alternative is remembering which line of the file the
    /// top row came from and re-deriving the offset from it on every resize,
    /// which is a second window to keep in step for something a reader sees once
    /// per drag of a terminal's corner. The offset is clamped when it is read,
    /// so what a resize can cost is a reader's place, never a panel scrolled off
    /// the end of what it holds.
    ///
    /// A width of `0` — a panel nobody has measured — wraps nothing: the
    /// document's lines are drawn as they are and cut to the width by the
    /// renderer, which is what the panel did before it was ever wrapped. Every
    /// frame measures, so that is the state of an app between being built and
    /// being drawn, and of a test that only cares about the height.
    pub(crate) fn set_width(&mut self, width: u16) {
        self.width = usize::from(width);
    }

    /// Whether the showing card's window is following the newest line of what is
    /// on it.
    ///
    /// `true` while that window sits at the end, which is where a fresh account
    /// starts and where the end-of-list movement key puts it back. `false` from
    /// the moment the reader scrolls up, until they scroll back down to the end
    /// or ask for it outright. See [`App::select_last`].
    ///
    /// `false` for a document from the moment it arrives, wherever its window
    /// is: nothing is ever appended to a file that has been read, so there is no
    /// newest line to be pinned to.
    ///
    /// The card that is not showing keeps its own answer, and it is not this
    /// one: an account left following goes on following behind a document, which
    /// is what puts the newest line of a run on screen when the reader swaps
    /// back to it.
    #[must_use]
    pub(crate) const fn follows(&self) -> bool {
        match self.showing {
            Showing::Account => self.account.follows,
            Showing::Thread => self.thread.follows,
            Showing::Document => self.document.follows,
        }
    }

    /// Whether the composer is a thing on screen: whether it is drawn, and so
    /// whether focus is allowed to land on it.
    ///
    /// One question with two readers. The renderer asks it to decide whether to
    /// cut rows off the bottom of the panel's column at all, and the event loop
    /// asks it to decide whether a keystroke can be the composer's. It is the
    /// same answer for both, and it is here rather than in either of them
    /// because it is a fact about which card the panel is showing.
    ///
    /// The thread and nothing else. The field writes into the conversation, so
    /// it belongs under the conversation: a document showing gives those rows
    /// back to the file, and an account showing gives them back to the run. A
    /// field under a run is a field about the wrong card — there is nothing on
    /// the account a sentence could be added to — and a reader who wants to ask
    /// something is one swap from the card that takes it.
    ///
    /// The rule is about the *card*, not about the draft. A composer holding
    /// nothing is still showable — that is the one empty row somebody types the
    /// first character into — and a draft somebody typed is not thrown away by
    /// the card that hides it.
    #[must_use]
    pub(crate) const fn composer_showable(&self) -> bool {
        match self.showing {
            Showing::Thread => true,
            Showing::Account | Showing::Document => false,
        }
    }

    /// How many lines one page key moves the panel by, on the same rule.
    pub(crate) const fn page(&self) -> usize {
        if self.height == 0 { 1 } else { self.height }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Mode, Panel, Showing};
    use crate::account::Line;

    /// Somewhere to hang a clocked line off. Any instant will do — nothing here
    /// asserts a duration, only that the cards keep what they were given.
    fn base() -> Instant {
        Instant::now()
    }

    /// A panel of a size a window can be cut out of.
    fn sized() -> Panel {
        let mut panel = Panel::default();
        panel.set_height(10);
        panel.set_width(40);
        panel
    }

    #[test]
    fn a_fresh_panel_opens_on_the_conversation_with_nothing_in_it() {
        let panel = Panel::default();

        assert_eq!(panel.showing(), Showing::Thread);
        assert_eq!(panel.mode(), Mode::Chat);
        assert!(!panel.has_account());
        assert!(!panel.has_thread());
        assert!(!panel.has_document());
        assert!(
            !panel.has_content(),
            "an app that has just started has nothing to show"
        );
    }

    #[test]
    fn the_swap_key_has_nowhere_to_go_until_something_else_has_anything_in_it() {
        let mut panel = sized();

        assert_eq!(
            panel.next_card(),
            None,
            "a panel holding only an empty conversation has nowhere to swap to"
        );

        panel.open_account(base());

        assert_eq!(
            panel.next_card(),
            Some(Showing::Account),
            "a run has happened and the swap key should reach it"
        );
    }

    #[test]
    fn the_swap_key_steps_over_the_card_with_nothing_in_it() {
        let mut panel = sized();
        panel.show_document(["a line of a file"], false);

        // Showing the document, with no account ever started: the only other
        // card worth stopping on is the conversation.
        assert_eq!(panel.showing(), Showing::Document);
        assert_eq!(panel.next_card(), Some(Showing::Thread));
    }

    #[test]
    fn each_card_keeps_its_own_place_while_another_one_is_showing() {
        let mut panel = sized();
        panel.open_account(base());
        panel.show_document((0..40).map(|n| format!("line {n}")), false);

        // The document is showing and scrolled off its own tail; the account
        // behind it is untouched and still following.
        panel.scroll_to(3);

        assert_eq!(panel.window_of(Showing::Document).0, 3);
        assert!(
            !panel.window_of(Showing::Document).1,
            "a card scrolled by hand stops following its own newest line"
        );
        assert!(
            panel.window_of(Showing::Account).1,
            "the card nobody touched stopped following"
        );
    }

    #[test]
    fn a_document_is_shown_the_moment_it_is_filled() {
        let mut panel = sized();

        panel.show_document(["one line"], false);

        assert_eq!(panel.showing(), Showing::Document);
        assert!(panel.has_document());
        assert_eq!(
            panel.document_lines(),
            [Line::Text {
                text: "one line".to_owned()
            }]
        );
    }

    #[test]
    fn a_second_read_replaces_the_document_rather_than_adding_to_it() {
        let mut panel = sized();
        panel.show_document(["before"], false);

        panel.show_document(["after"], false);

        assert_eq!(
            panel.document_lines().len(),
            1,
            "the card grew instead of being replaced: {:?}",
            panel.document_lines()
        );
    }

    #[test]
    fn a_mode_change_is_reported_only_when_it_changes_something() {
        let mut panel = Panel::default();

        assert!(panel.set_mode(Mode::Brief), "chat to brief is a change");
        assert_eq!(panel.mode(), Mode::Brief);
        assert!(
            !panel.set_mode(Mode::Brief),
            "brief to brief is not a change and must not be announced as one"
        );
    }

    #[test]
    fn a_run_opens_a_fresh_account_over_whatever_the_last_one_left() {
        let mut panel = sized();
        // Showing it as well as opening it: `open_account` places the card, and
        // putting it on screen is the app's own step (see `App::start_account`).
        panel.show(Showing::Account);
        panel.open_account(base());
        panel.write_run(|account| account.open_section("crates", base()));
        let first = panel.window(base()).len();

        panel.open_account(base() + Duration::from_secs(1));

        assert!(first > 0, "the first run wrote nothing to begin with");
        assert!(
            panel.window(base() + Duration::from_secs(1)).len() < first,
            "the new run inherited the old one's lines"
        );
    }

    #[test]
    fn the_window_is_the_height_it_was_given_and_no_more() {
        let mut panel = sized();
        panel.show_document((0..100).map(|n| format!("line {n}")), false);

        assert_eq!(
            panel.window(base()).len(),
            10,
            "the window is not the panel's height"
        );
        assert!(
            panel.lines_below() > 0,
            "a hundred lines in a ten-line window has nothing below it"
        );
    }
}
