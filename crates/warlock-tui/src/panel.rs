//! The right-hand pane: an [`Account`] of the run, a [`Thread`] of the
//! conversation, and the lines of a document somebody pressed `v` on — three
//! cards, exactly one showing, and the swap key stepping between the ones there
//! is anything to see in.
//!
//! It is a module because everything here was a private struct inside `app.rs`
//! with twenty-three [`App`](crate::App) methods reaching into it, so a question
//! about card-swapping or panel scrolling could only be asked by building a
//! whole `App` — a `Tree`, a row list, a selection and a scroll offset, none of
//! which the panel has ever had an opinion about. `App` still forwards every one
//! of those methods, because nothing is bought by making two hundred call sites
//! say `app.panel()` instead.

use std::time::Instant;

use crate::account::{Account, Line};
use crate::app::cut_at_cap_message;
use crate::claude::Activity;
use crate::thread::{Ending, Thread};
use crate::wrap::rows as wrap_rows;

/// The group that survives everything. A reload carries it, because the tree is
/// read again *because* a run finished and dropping it would wipe the record at
/// the moment the reader turned to read it; a run that ends with nothing
/// recorded carries it too, because an account is not a claim about the tree.
/// That second rule used to be `App::take_account_from`, a method whose whole
/// purpose was to reach into a live app and steal back the fields that must not
/// roll back. It is a field move now.
///
/// A filled card stays filled: reading a file does not throw the account away, a
/// pact starting does not throw the document away, and neither empties the
/// conversation. A run is written to its own card however many of the three are
/// filled and whichever one the reader is looking at — a card swapped away under
/// the reader would be worse than a run they have to swap to see.
///
/// A card being filled is not the same as it having lines: an app that has never
/// run a pact draws nothing at all, while an account that has recorded nothing
/// yet is a run under way. A second pact starts a second account; the thread is
/// the one card that is the other way round, since one session is one
/// conversation. See `Card::accrue`.
///
/// `showing` moves for exactly three reasons: the view key, a message submitted
/// below, and the swap key. Not for a run, not for
/// [`Panel::refill_document`], and not for anything a turn already under way
/// reports.
///
/// Each card carries its own `offset` and `follows`, which is what lets the
/// account go on following the newest line while the document is up. `height`
/// and `width` are shared, because there is one panel. While `follows` is set,
/// `offset` is not read at all: the offset is worked out from the line count at
/// the moment it is asked for, so appending a line moves the window without
/// anybody having to tell the window that a line was appended.
///
/// `mode` is here rather than beside `focus` because it too has to survive a run
/// that ended with nothing recorded: a reader who has spent ten turns converging
/// on a brief and then pacts a clean directory should not find the conversation
/// back in chat mode because a *pact* rolled back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Panel {
    account: Card<Account>,
    thread: Card<Thread>,
    document: Card<Vec<Line>>,
    showing: Showing,
    mode: Mode,
    height: usize,
    width: usize,
}

/// Three variants and no fourth, for [`Focus`](crate::Focus)'s reason: the panel
/// holds three named things, not an index into a list somebody could grow. There
/// is no `Nothing` — an empty card showing draws warlock's mark, which is a fact
/// about the card rather than a fourth thing to be showing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Showing {
    Account,
    #[default]
    Thread,
    Document,
}

impl Showing {
    /// An arm per card rather than an index step, so a fourth card is a compile
    /// error here rather than a swap that quietly went nowhere. The order is the
    /// order the reader comes to them in: the conversation they start on, the
    /// run they asked for, then the file it wrote.
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
/// A mode is not a second system prompt and not a second session — it is this
/// word plus one ordinary turn sent into the conversation already in progress
/// (see [`brief_instruction`](crate::brief_instruction) and
/// [`CHAT_INSTRUCTION`](crate::CHAT_INSTRUCTION)) — so everything the word
/// changes is said out loud somewhere else: which instruction a command sends,
/// how hard the turn is asked to think, and which model is asked.
///
/// It changes nothing on the card. Nothing on the thread is cleared, hidden or
/// reordered, the session is the same session, and the tool grant is
/// byte-identical.
///
/// Drawn in exactly one place, the panel's border title. Not on the run header's
/// row, which a pact takes over and which a brief started mid-run would collide
/// with, and not on a row of the card, which would spend a row of the
/// conversation saying what the border says for nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Mode {
    #[default]
    Chat,
    Brief,
}

/// Generic over what is on the card because the window rule is not: an offset, a
/// follow flag and a height decide where a list of lines is cut, whether that
/// list is an account being written or a file that was read once. What differs
/// is how the lines are counted and produced, which is [`Shown`].
///
/// `held` is `None` for a card nothing has filled yet, which is not the same as
/// a card holding an empty list: an unfilled card is a panel with nothing to
/// say, and an empty account is something having happened. See
/// [`Panel::has_content`].
#[derive(Debug, Clone, PartialEq)]
struct Card<T> {
    held: Option<T>,
    offset: usize,
    follows: bool,
}

/// Written out rather than derived, because a derived `Default` would demand one
/// of `T` as well and [`Account`] has none: an account is a run that started at
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
    /// Whatever the card was holding goes: one pact is one account, and one press
    /// of the view key is one document.
    fn place(&mut self, held: T, follows: bool) {
        self.held = Some(held);
        self.offset = 0;
        self.follows = follows;
    }

    /// The append path, and deliberately not a second [`Card::place`]: `place`
    /// drops what the card held, which is right for a run and for a read and
    /// wrong for a conversation. One session is one thread, so a question asked
    /// ten minutes in goes under the nine before it.
    ///
    /// It follows, every time. Somebody who has just asked something is asking
    /// to see the answer, so the window goes back to the newest line even if they
    /// had scrolled up — the one thing that moves a card's window without a
    /// movement key, and it is their own keystroke that does it.
    fn accrue(&mut self) -> &mut T
    where
        T: Default,
    {
        self.follows = true;
        self.held.get_or_insert_with(T::default)
    }

    /// A count of rows on screen rather than of lines held: a line too long for
    /// the width is drawn in several rows, and this is what the window is cut out
    /// of.
    fn line_count(&self, width: usize) -> usize {
        self.held.as_ref().map_or(0, |held| held.line_count(width))
    }

    fn scroll_offset(&self, height: usize, width: usize) -> usize {
        panel_offset_for(self.line_count(width), height, self.offset, self.follows)
    }

    fn window(&self, height: usize, width: usize, now: Instant) -> Vec<Line> {
        self.held.as_ref().map_or_else(Vec::new, |held| {
            held.window(self.scroll_offset(height, width), height, width, now)
        })
    }

    fn lines_below(&self, height: usize, width: usize) -> usize {
        self.line_count(width)
            .saturating_sub(self.scroll_offset(height, width) + height)
    }

    fn scroll_to(&mut self, offset: usize, height: usize, width: usize) {
        // Where the end is, asked of the one function that decides it, so that
        // "as far down as this card goes" means the same thing to a keystroke as
        // it does to the frame being drawn.
        let end = panel_offset_for(self.line_count(width), height, 0, true);
        self.offset = offset.min(end);
        self.follows = self.offset == end;
    }
}

/// The three implementations are the three cards: an [`Account`], which words its
/// own lines and has clocks in them, a [`Thread`], which has both clocks and
/// prose, and a document, which is the lines themselves. A document has no
/// clock, so `now` reaches only the other two.
trait Shown {
    fn line_count(&self, width: usize) -> usize;

    fn window(&self, offset: usize, height: usize, width: usize, now: Instant) -> Vec<Line>;
}

/// The count is taken from the account's own start, which is the one instant an
/// account that has rows can always name. `now` decides what a clock *says*
/// rather than whether it is a row — with the one exception that a clock going
/// from `9:59` to `10:00` takes a column off the line beside it, which can push
/// a line that was exactly one column inside the width onto a second row. That
/// costs one row of scrollback being a frame behind, which the next frame
/// settles.
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

/// A document is its lines, already worded — the file's own, plus the one
/// sentence about a read the cap cut short. [`App`](crate::App) never holds the
/// path it came from and never opens anything: what reaches it is text, from
/// whoever did the reading.
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

/// The wrap module's answer and not this one's, so what the app counts and what
/// the renderer draws are the same rows from the same code. See
/// [`rows`](crate::wrap::rows), which is also where the shape of a continuation
/// row is decided.
fn rows_of(line: &Line, width: usize) -> Vec<Line> {
    wrap_rows(line, width)
}

impl Panel {
    #[must_use]
    pub fn scroll_offset(&self) -> usize {
        match self.showing {
            Showing::Account => self.account.scroll_offset(self.height, self.width),
            Showing::Thread => self.thread.scroll_offset(self.height, self.width),
            Showing::Document => self.document.scroll_offset(self.height, self.width),
        }
    }

    #[must_use]
    pub fn window(&self, now: Instant) -> Vec<Line> {
        match self.showing {
            Showing::Account => self.account.window(self.height, self.width, now),
            Showing::Thread => self.thread.window(self.height, self.width, now),
            Showing::Document => self.document.window(self.height, self.width, now),
        }
    }

    #[must_use]
    pub fn lines_below(&self) -> usize {
        match self.showing {
            Showing::Account => self.account.lines_below(self.height, self.width),
            Showing::Thread => self.thread.lines_below(self.height, self.width),
            Showing::Document => self.document.lines_below(self.height, self.width),
        }
    }

    pub fn scroll_to(&mut self, offset: usize) {
        let (height, width) = (self.height, self.width);
        match self.showing {
            Showing::Account => self.account.scroll_to(offset, height, width),
            Showing::Thread => self.thread.scroll_to(offset, height, width),
            Showing::Document => self.document.scroll_to(offset, height, width),
        }
    }
}

/// Where the panel's window onto `lines` lines should start, given a `viewport`
/// that many lines tall, a window parked at `offset`, and whether it is
/// `following` the newest line.
///
/// The rule the tree's `scroll_offset_for` cannot be: the tree's window is
/// dragged about by a selection and moves as little as it can, while the panel's
/// is either at the end of a list still being written or exactly where the reader
/// left it. So there is no minimum-movement case and no selection. Following, the
/// answer is the last screenful whatever the length is *now*, which is what pins
/// the newest line to the bottom row as lines arrive.
///
/// A viewport of zero rows — a panel nobody has drawn — has no screen to scroll,
/// and the honest offset for it is the top, exactly as the tree's rule says.
#[must_use]
pub fn panel_offset_for(lines: usize, viewport: usize, offset: usize, following: bool) -> usize {
    if viewport == 0 {
        return 0;
    }
    let end = lines.saturating_sub(viewport);
    if following { end } else { offset.min(end) }
}

/// Every one of these used to be a method on [`App`](crate::App) that touched
/// `self.panel` and nothing else. They are here so the panel can be built,
/// driven and asserted about without a tree, a row list, a selection or a scroll
/// offset anywhere near it.
impl Panel {
    #[cfg(test)]
    #[must_use]
    pub const fn showing(&self) -> Showing {
        self.showing
    }

    /// About the card rather than about the panel, which is the point:
    /// [`Panel::scroll_offset`] answers for the one showing, and what a reader
    /// needs to know is that the other two kept their place while it was not.
    #[cfg(test)]
    #[must_use]
    pub fn window_of(&self, card: Showing) -> (usize, bool) {
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

    #[cfg(test)]
    #[must_use]
    pub fn document_lines(&self) -> &[Line] {
        self.document.held.as_deref().unwrap_or_default()
    }

    pub const fn show(&mut self, card: Showing) {
        self.showing = card;
    }

    pub fn open_account(&mut self, at: Instant) {
        self.account.place(Account::new(at), true);
    }

    /// One operation rather than two, because a document nobody was shown is a
    /// file read for nothing: every caller that refills wants it up.
    pub fn show_document(&mut self, lines: impl IntoIterator<Item = impl Into<String>>, cut: bool) {
        self.refill_document(lines, cut);
        self.showing = Showing::Document;
    }

    #[must_use]
    pub const fn has_content(&self) -> bool {
        match self.showing {
            Showing::Account => self.has_account(),
            Showing::Thread => self.has_thread(),
            Showing::Document => self.has_document(),
        }
    }

    /// The conversation is always somewhere to be — empty, it draws warlock's
    /// mark over a field with nothing typed in it — so a swap can always get back
    /// to it. The other two are reached only once there is a run to read or a
    /// file to look at.
    const fn stops_on(&self, card: Showing) -> bool {
        match card {
            Showing::Account => self.has_account(),
            Showing::Thread => true,
            Showing::Document => self.has_document(),
        }
    }

    /// Two steps at most, because there are three cards and the one showing is
    /// not a candidate. `None` is the key having done nothing, which the caller
    /// says out loud.
    #[must_use]
    pub fn next_card(&self) -> Option<Showing> {
        let next = self.showing.next();
        [next, next.next()]
            .into_iter()
            .find(|&card| self.stops_on(card))
    }
    /// The one way a run's events reach the panel. A closure rather than the
    /// event, because [`Account`] already knows how to word every line a run will
    /// ever have and a method here per kind of event would be a second place a
    /// run's line could come to be spelled.
    ///
    /// Does nothing when there is no account to write to, which is a run nobody
    /// started through
    /// [`App::start_account`](crate::App::start_account) — a test driving events
    /// straight down the channel. Dropping the line is the honest way to fail.
    pub fn write_run(&mut self, write: impl FnOnce(&mut Account)) {
        if let Some(account) = self.account.held.as_mut() {
            write(account);
        }
    }

    /// [`App::show_document`](crate::App::show_document) without the one thing
    /// the view key does. `v` is a reader asking to look at a file, so it brings
    /// the file to the front; this is the file somebody has just edited being
    /// read again underneath them, and a panel that flipped to the document
    /// because a `WARLOCK.md` was saved would take the account of a run out of
    /// the reader's hands without their having pressed anything.
    ///
    /// The window goes back to the top and follows nothing. The reader's line is
    /// deliberately not kept: the file has been rewritten under them, so line
    /// forty of the file they were reading is not line forty of the file that is
    /// there now.
    pub fn refill_document(
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

    /// The only way an app comes to have a turn in its thread at all, and one of
    /// the three things that decide which card is drawn: somebody who has just
    /// asked a question is looking for the answer. The card accumulates rather
    /// than being replaced — one session, one conversation — which is
    /// `Card::accrue`'s whole reason for existing beside `Card::place`.
    ///
    /// The message is the reader's own text, never a path or a prompt this type
    /// built: [`App`](crate::App) runs nothing and asks nobody, so whoever took
    /// the draft starts the worker themselves.
    pub fn start_turn(&mut self, message: impl Into<String>, at: Instant) {
        self.thread.accrue().ask(message, at);
        self.showing = Showing::Thread;
    }

    /// What warlock says for itself, as against a turn, which is something
    /// somebody asked a model. It brings the conversation to the front for
    /// [`Panel::start_turn`]'s reason: an answer on a card the reader is not
    /// looking at is not an answer.
    ///
    /// One unclocked row, and no turn is opened, closed or frozen by it — see
    /// [`Thread::note`].
    pub fn note(&mut self, text: impl Into<String>, at: Instant) {
        self.thread.accrue().note(text, at);
        self.showing = Showing::Thread;
    }

    pub fn record_turn(&mut self, activity: &Activity, at: Instant) {
        if let Some(thread) = self.thread.held.as_mut() {
            thread.record(activity, at);
        }
    }

    pub fn answer_turn(&mut self, answer: impl Into<String>, at: Instant) {
        if let Some(thread) = self.thread.held.as_mut() {
            thread.answer(answer, at);
        }
    }

    pub fn end_turn(&mut self, ending: &Ending, at: Instant) {
        if let Some(thread) = self.thread.held.as_mut() {
            thread.end(ending, at);
        }
    }

    #[must_use]
    pub const fn thread(&self) -> Option<&Thread> {
        self.thread.held.as_ref()
    }

    #[must_use]
    pub const fn has_account(&self) -> bool {
        self.account.held.is_some()
    }

    #[must_use]
    pub const fn has_thread(&self) -> bool {
        self.thread.held.is_some()
    }

    #[must_use]
    pub const fn has_document(&self) -> bool {
        self.document.held.is_some()
    }

    #[must_use]
    pub const fn showing_thread(&self) -> bool {
        matches!(self.showing, Showing::Thread)
    }

    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// The answer is what a caller needs and cannot easily get back afterwards: a
    /// mode *change* is worth one note in the thread, where `/brief` typed in
    /// brief mode is a re-send with nothing new to say about the register. So the
    /// comparison happens here, once, rather than in each caller against a copy
    /// of [`Panel::mode`] it had to remember to take first.
    ///
    /// It sets one word and touches nothing else: the card showing does not move,
    /// no turn is started or ended, and the run header is not consulted.
    pub fn set_mode(&mut self, mode: Mode) -> bool {
        let changed = self.mode != mode;
        self.mode = mode;
        changed
    }

    #[must_use]
    pub const fn account(&self) -> Option<&Account> {
        self.account.held.as_ref()
    }

    pub const fn account_mut(&mut self) -> Option<&mut Account> {
        self.account.held.as_mut()
    }

    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Only the layout knows the height, and a height passed per call would let
    /// two callers disagree about the size of one window. Safe to call every
    /// frame: nothing has to be brought back into line afterwards, because the
    /// offset is clamped when it is read and a window that was following is still
    /// following.
    pub fn set_height(&mut self, height: u16) {
        self.height = usize::from(height);
    }

    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// [`Panel::set_height`]'s counterpart, and safe to call every frame in the
    /// same way.
    ///
    /// Nothing is brought back into line afterwards, and here that is worth
    /// saying out loud: a terminal made narrower gives a document more rows than
    /// it had, so the offset the reader parked at is a row further up the file
    /// than it was. Remembering which line of the file the top row came from and
    /// re-deriving the offset on every resize is a second window to keep in step,
    /// for something a reader sees once per drag of a terminal's corner. What a
    /// resize can cost is a reader's place, never a panel scrolled off the end.
    ///
    /// A width of `0` — a panel nobody has measured — wraps nothing: the lines
    /// are drawn as they are and cut to the width by the renderer.
    pub fn set_width(&mut self, width: u16) {
        self.width = usize::from(width);
    }

    /// `false` for a document from the moment it arrives, wherever its window is:
    /// nothing is ever appended to a file that has been read, so there is no
    /// newest line to be pinned to.
    ///
    /// The card that is not showing keeps its own answer, which is what puts the
    /// newest line of a run on screen when the reader swaps back to it.
    #[must_use]
    pub const fn follows(&self) -> bool {
        match self.showing {
            Showing::Account => self.account.follows,
            Showing::Thread => self.thread.follows,
            Showing::Document => self.document.follows,
        }
    }

    /// Whether the composer is drawn, and so whether focus is allowed to land on
    /// it.
    ///
    /// One question with two readers — the renderer, deciding whether to cut rows
    /// off the bottom of the panel's column, and the event loop, deciding whether
    /// a keystroke can be the composer's — so it is here rather than in either of
    /// them.
    ///
    /// The thread and nothing else: the field writes into the conversation, so a
    /// document showing gives those rows back to the file and an account showing
    /// gives them back to the run. The rule is about the *card*, not about the
    /// draft: a composer holding nothing is still showable, and a draft somebody
    /// typed is not thrown away by the card that hides it.
    #[must_use]
    pub const fn composer_showable(&self) -> bool {
        match self.showing {
            Showing::Thread => true,
            Showing::Account | Showing::Document => false,
        }
    }

    #[must_use]
    pub const fn page(&self) -> usize {
        if self.height == 0 { 1 } else { self.height }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Mode, Panel, Showing};
    use crate::account::Line;

    fn base() -> Instant {
        Instant::now()
    }

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
