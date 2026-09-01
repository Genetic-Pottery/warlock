//! The conversation: the register it is in, the field it is typed at, and the
//! turns it is made of.
//!
//! [`Chat`] is the whole of it, and it is one value because these are one thing.
//! A draft is typed into the field; submitting it is a message, or one of the
//! three commands; `/brief` puts the conversation in a register and settles
//! where a document written out of it would go; `/write` asks for that document;
//! and the window that opens over the answer is the same conversation's. Every
//! one of those five was a separate local of the event loop, wired together by
//! rules written down in prose at four different call sites — and the rules are
//! in here now, where getting one wrong is visible.
//!
//! The register itself is deliberately *not* here: it lives on the [`App`],
//! inside the panel, because the panel's border title is drawn from it and
//! because that is the one part of the app a failed pact's restore puts back
//! untouched, beside the very thread it describes. [`Chat`] sets it and reads it
//! through the app rather than keeping a second copy.
//!
//! ## The turn, and the shape it shares with a run
//!
//! [`pacting`](crate::pacting) over a much smaller job, and deliberately the
//! same four parts in the same order: [`spawn_turn`] starts the worker and hands
//! back the channel it reports on, [`start_turn`] is everything the event loop
//! has to keep about work it is not doing, [`run_turn`] is the worker's whole
//! body — a free function, so a test drives it on its own thread with no thread
//! at all — and [`apply_turn`] is the loop's other half, draining with
//! `try_recv` so a frame is never spent waiting on a model. A turn is one
//! `claude` and a pact is a hundred, but the shape of not blocking the screen on
//! either is the same shape, and one of them is enough.
//!
//! What is *not* the same is everything a turn ends in. A run writes documents,
//! saves a manifest and makes the tree on screen a load out of date; a turn
//! writes nothing of its own and reloads nothing, so nothing here touches a
//! [`Tree`](warlock_engine::Tree), and the [`Manifest`] arrives as an argument on
//! the one method that needs one. What a turn produces is rows: the work as it
//! happens, and then either the answer or the one line saying why there is none.
//!
//! It does now touch a repository, which it did not before, and that is the one
//! property this module gave up: `/brief` reads a template and a config from
//! [`Chat::root`], and `/write` spells a proposed path relative to it. The root
//! is resolved once by the load and held for the session, so it is a field
//! rather than a parameter on four methods — and `/write` reading nothing at all
//! is what keeps a document twenty turns in the making from arriving at a window
//! that will not open.
//!
//! ## The field is muted by the turn and by nothing else
//!
//! One question at a time is what a conversation is, so the field says nothing
//! while an answer is out. That is derived from the turn rather than set and
//! cleared: a turn ends in five ways and all five of them arrive in
//! [`Chat::keep_up`], so no ending has to remember to hand the keyboard back. A
//! pact is deliberately not a reason — the two workers share nothing, and a
//! reader watching a long run is exactly who most wants to ask something about
//! the repository it is walking.
//!
//! That is the second difference, and it is the one worth being explicit about.
//! Nothing here comes back as an error. A `claude` that is not installed, a
//! child that exits non-zero, a turn that runs past its timeout, a model that
//! finishes with nothing to say and a reader who pressed Ctrl-C are five
//! different facts and one kind of consequence: exactly one [`Ending`] on the
//! thread and the same sentence on the footer, with the session as usable for
//! the next question as it was for this one. An event loop that could be ended
//! by a bad answer would be a chat that takes the tree down with it.
//!
//! Stopping a turn is the pact's bargain over again, and literally so — the
//! say-when is [`pacting`](crate::pacting)'s own [`CancelGuard`], reused rather
//! than written a second time. A turn cancelled on purpose ends in
//! [`Ending::Cancelled`]; a turn whose value is simply dropped, because warlock
//! is quitting or because the loop returned through a `?`, takes its `claude`
//! with it and reports to nobody, which is the whole reason the handle is a
//! guard rather than a flag somebody has to remember.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Instant;

use warlock_engine::{BriefsError, DEFAULT_BRIEF_DIRECTORY, Manifest, load_briefs};
use warlock_tui::{
    Activities, Activity, App, BRIEF_EFFORT, CHAT_INSTRUCTION, Cancel, ChatAgent, Composed,
    Composer, Edited, Ending, Focus, Mode, ScopePrompt, Submitted, TemplateError,
    WRITE_INSTRUCTION, brief_instruction, brief_template, ending_for, submitted_for,
};

use crate::error::one_line;
use crate::pacting::CancelGuard;
use crate::writing::{write_edit, write_opened};

/// What the thread and the footer say when the worker stopped without reporting
/// anything — which, since it reports on every path it takes itself, means it
/// panicked.
///
/// [`PACT_LOST`](crate::pacting)'s opposite number, and worded as the tail of
/// [`Ending::Broke`]'s sentence rather than as a sentence of its own, so the
/// line reads as every other failed turn reads: `the turn could not run — …`.
/// Nothing was written and nothing was saved, so unlike a lost pact there is
/// nothing to put back — the conversation keeps every line that arrived before
/// the silence, and the next question runs as if this one had merely failed.
const TURN_LOST: &str = "it stopped without saying how it went";

/// What `/brief` is shown as on the thread card, and `/chat`.
///
/// The word the reader typed, spelled here rather than taken from the draft: a
/// draft is trimmed and matched by [`submitted_for`], so `"  /brief  "` and
/// `"/brief"` are one command and have to read as one row. What is actually sent
/// is [`brief_asking`] and [`CHAT_INSTRUCTION`], which the card never shows
/// (see [`Chat::say`](chatting::Chat::say)).
const BRIEF_COMMAND: &str = "/brief";
/// `/chat` as the card shows it. See [`BRIEF_COMMAND`].
const CHAT_COMMAND: &str = "/chat";
/// `/write` as the card shows it. See [`BRIEF_COMMAND`]: what is actually sent
/// is [`WRITE_INSTRUCTION`], and a screenful of warlock's prose in the place the
/// reader's own words go would be warlock putting words in their mouth.
const WRITE_COMMAND: &str = "/write";

/// The one line the thread gains when the conversation enters brief mode.
///
/// A note and not a turn: warlock saying what it did with the word it was given,
/// unclocked, at the point in the history the command was typed. It is said on a
/// *change* only — `/brief` in brief mode re-sends the instruction and has
/// nothing new to say about the register — and it says what the mode is and how
/// to leave it, because the way out is the one thing a reader in a mode cannot
/// work out from the screen. The border title says which mode it is from then
/// on; this line says when it started.
const BRIEF_NOTE: &str =
    "brief mode — this conversation is now converging on a document. /chat leaves it.";

/// The one line the thread gains when the conversation leaves brief mode.
///
/// [`BRIEF_NOTE`]'s counterpart, on the same rule and for the same reason: one
/// unclocked line where the command was typed, and only on a change.
const CHAT_NOTE: &str = "chat mode — the brief is over and nothing is being converged on.";

/// The one line `/chat` costs when the conversation is in chat mode already.
///
/// The command has nothing to do — there is no mode to leave — so it does
/// nothing, and says so rather than spending a turn telling the model something
/// it was never told the other way. One line, in warlock's own voice, where the
/// refusal of a mistyped command goes.
const ALREADY_CHATTING: &str = "already in chat mode — /brief is what changes that.";

/// The one line `/write` costs when the conversation was never aimed at a
/// document.
///
/// [`ALREADY_CHATTING`]'s sibling and the same bargain: there is nothing to ask
/// for, so nothing is asked. A `/write` in chat mode would be warlock demanding
/// a brief from a conversation about where the loader is, and the model,
/// obliging, would invent one — a turn's wait and a screenful of fiction to
/// discover that the command was typed in the wrong register.
///
/// It names the way in, because the way in is the one thing a reader who has
/// just been refused cannot work out from the screen: the border title says
/// which register the conversation is in, and this says which register `/write`
/// wants and what puts it there. Decided from [`App::mode`] — the very state
/// that title is drawn from — so the refusal and the header cannot disagree.
const NOT_BRIEFING: &str = "/write is only in brief mode — /brief enters it";

/// The one line `/brief` costs when the repository's own template is there and
/// cannot be read.
///
/// [`ALREADY_CHATTING`]'s and [`NOT_BRIEFING`]'s third sibling, and the only one
/// with something of somebody else's to quote: the file that was found and, in
/// the loader's own words, why it could not be had (see [`TemplateError`], whose
/// wording is `error.rs`'s for an unreadable sigil config). One line, unclocked,
/// where a refusal goes.
///
/// The refusal itself is the point. A template file that exists is a shape
/// somebody meant, so warlock will not quietly put its own in its place and open
/// a twenty-turn conversation aimed at the wrong document — the same bargain
/// `ignores.rs` makes about rules it cannot read and `sigils.rs` about a config
/// it cannot read. Nothing else about the session moves: the mode is not entered,
/// no turn is spent, and the card is what it was with one line added.
///
/// It says what the reader can do, because a reader who has just been refused
/// cannot see from the screen that the fix is theirs: the file is in their
/// repository and `e` opens it.
fn unreadable_template(error: &TemplateError) -> String {
    format!("{error} — /brief did nothing, so fix or remove the file and type it again")
}

/// The instruction a `/brief` sends, for the repository at `root`: its own
/// template if it has written one, and warlock's if it has not.
///
/// The two halves of the command's one composition in one place — the load and
/// the wording — so that what the turn carries is a function of a root and the
/// bytes under it, testable without a terminal, a `claude` or a thread.
///
/// Read here, at the keystroke, and held nowhere: no copy on [`App`], on
/// [`Chat`] or in the loop. That is the whole of what makes a template edited
/// with `e` between two briefs a template the second one is held to (see
/// [`brief_template`]).
///
/// # Errors
///
/// [`TemplateError`] when the file is there and cannot be read or decoded, which
/// the caller turns into one line and no turn — never into the built-in default.
fn brief_asking(root: &Path) -> Result<String, TemplateError> {
    brief_template(root).map(|template| brief_instruction(&template))
}

/// The one line `/brief` costs when the repository's own `.warlock/briefs.toml`
/// is there and cannot be had.
///
/// [`unreadable_template`]'s twin, in its wording and its shape, because the two
/// are the same refusal about the two files `/brief` reads: the loader's own
/// sentence — which names the file and quotes the parser (see [`BriefsError`]) —
/// and what it cost, on one unclocked line where a refusal goes.
///
/// Flattened with [`one_line`] where that one is not, and only because the
/// material differs: a `briefs.toml` that will not parse carries the TOML
/// parser's multi-line diagnostic inside it, exactly as an unreadable
/// `pacts.toml` does, and the thread card is one line per note. `error.rs` does
/// the same to a manifest for the same reason.
///
/// The refusal, again, is the point. A `directory` that was written down is a
/// place somebody meant, so warlock will not quietly aim twenty turns at
/// `docs/` instead — a misspelled key is a line to go and fix rather than a
/// silent write somewhere nobody asked for. Nothing else about the session
/// moves: no mode, no turn, and the card is what it was with one line added.
fn unreadable_briefs(error: &BriefsError) -> String {
    format!(
        "{} — /brief did nothing, so fix or remove the file and type it again",
        one_line(&error.to_string())
    )
}

/// Everything `/brief` has to read out of the repository at `root` before it can
/// happen: the instruction to send, and the directory a `/write` in the mode it
/// opens would propose.
///
/// The whole of the command's *load* in one place, and in one order, so that the
/// arm below is a single question with a single refusal. Both files are optional
/// and both are re-read at every `/brief`, which is what makes either of them
/// edited with `e` between two briefs a file the second one is held to.
///
/// The template comes first, and that is the answer to a repository where both
/// files are broken: the first failure stops the command, so what the reader is
/// shown is one line about the template, and the `briefs.toml` line is what the
/// next `/brief` says once that one is fixed. One refusal at a time is the rule
/// the two must not disagree about — two notes for one keystroke would be
/// warlock reporting its own reading order — and neither costs a turn.
///
/// # Errors
///
/// The finished line for whichever file could not be had, already worded for the
/// card ([`unreadable_template`], [`unreadable_briefs`]) — the caller has a note
/// to add and nothing to decide.
fn brief_reading(root: &Path) -> Result<(String, String), String> {
    let instruction = brief_asking(root).map_err(|error| unreadable_template(&error))?;
    let directory = load_briefs(root).map_err(|error| unreadable_briefs(&error))?;

    Ok((instruction, directory))
}

/// The whole of the conversation the event loop keeps: who to ask, and the one
/// question currently out.
///
/// Two things, and the reason they are one value is that neither is any use
/// without the other. The agent is built once for the life of the process —
/// a [`ChatAgent`] carries the session id that makes the second question a reply
/// to the first, so one per turn would be a conversation that forgot itself
/// between two Enters — and the turn is `Some` for the seconds one is in flight.
/// Keeping them together is what lets the loop ask "is a question out?" in one
/// word ([`Chat::answering`]) at the two places that decide it: the field's
/// muting, and what Ctrl-C means.
///
/// It holds no draft and no thread. What has been typed is the loop's
/// [`Composer`](warlock_tui::Composer), which a run must not be able to reach,
/// and the conversation itself is on the app's thread card, where the panel can
/// draw it — this value is the *machinery*, and machinery that also held the
/// text would be a second copy of the conversation for somebody to keep in step.
pub(crate) struct Chat {
    /// Who is asked, and the session every turn belongs to. Never spoken to
    /// directly: each turn works off a copy of it wired to that turn's own
    /// cancel and activity port (see [`wired`]).
    agent: ChatAgent,
    /// The turn being answered, if one is. `None` is the ordinary state and the
    /// state a session starts and ends in.
    turn: Option<Chatting>,
    /// The repository this conversation is rooted in, resolved once by the load
    /// and held for the session.
    ///
    /// A field rather than a parameter on four methods, because [`Scope`] is
    /// resolved before the conversation is built and never moves afterwards.
    /// It is what `/brief` reads its two files from and what `/write` spells a
    /// proposed path relative to — so this module does now touch a repository,
    /// which it did not before, and the doc above says so rather than pretending
    /// otherwise.
    root: PathBuf,
    /// The draft at the foot of the panel, and the only copy of it.
    ///
    /// Here rather than on the [`App`] for the reason it was a loop local
    /// before: a pact that ends with nothing recorded puts a copy of the app
    /// taken before it back over the live one, so a draft stored there would be
    /// a draft a run could swallow half a sentence into. Nothing a run does can
    /// reach this.
    composer: Composer,
    /// Where a brief written from this conversation would go, relative to
    /// [`Chat::root`]: the engine's default until a `/brief` says otherwise.
    ///
    /// Settled at `/brief` and read at `/write`, which is the whole of why
    /// `/write` can never fail for want of a file: by the time it asks for a
    /// path this is a string that was read turns ago. Written on every `/brief`
    /// rather than left wherever the last one put it, so a `briefs.toml` edited
    /// between two briefs takes effect without a restart.
    directory: String,
    /// The window no key opens: the path a brief is about to be written to.
    ///
    /// A `/write` turn landing is what opens it (see [`Chat::keep_up`]), which
    /// is why it lives beside the turn that produces it rather than in the loop
    /// that used to hold it. It is a [`ScopePrompt`] rather than a kind of its
    /// own because it is the same field, the same editor and the same window
    /// with a different question in it.
    prompt: ScopePrompt,
}

impl Chat {
    /// A conversation with nothing asked yet.
    ///
    /// The agent is made here, once, and the session id it names is fixed from
    /// this moment: every turn of this warlock is one conversation, and a second
    /// warlock in another terminal is another.
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_agent(root, ChatAgent::new())
    }

    /// A conversation with nothing asked yet, over `agent`.
    ///
    /// [`Chat::new`]'s body with the one thing that varies handed in, so that a
    /// test can drive the very value the loop keeps — the agent and the turn
    /// together — against a stand-in program. A conversation whose failures
    /// could only be asserted through the pieces underneath it would be a
    /// conversation nothing had ever run twice — including the loop's own
    /// submit path, which is why this is reachable from the binary's tests and
    /// not only from this module's.
    pub(crate) fn with_agent(root: impl Into<PathBuf>, agent: ChatAgent) -> Self {
        Self {
            agent,
            turn: None,
            root: root.into(),
            composer: Composer::default(),
            directory: DEFAULT_BRIEF_DIRECTORY.to_owned(),
            prompt: ScopePrompt::default(),
        }
    }

    /// The draft, for the frame and for the hit test.
    ///
    /// Read-only, and it can be nothing else: `ui.rs` is in the library and this
    /// module is the binary's, so the renderer has no way even to name a
    /// [`Chat`], let alone write through one. The crate boundary enforces the
    /// encapsulation that an accessor would otherwise be quietly handing back.
    pub(crate) const fn composer(&self) -> &Composer {
        &self.composer
    }

    /// The write window, for the same two readers and on the same terms.
    pub(crate) const fn write_prompt(&self) -> &ScopePrompt {
        &self.prompt
    }

    /// Where a brief written from this conversation would go.
    ///
    /// Nothing in the event loop reads this — `/write` proposes its own path
    /// from it without asking, which is the whole point of the directory being
    /// settled at `/brief` — so it exists for the tests that are *about*
    /// `/brief` settling it, and is compiled out otherwise.
    #[cfg(test)]
    pub(crate) fn directory(&self) -> &str {
        &self.directory
    }

    /// Whether a question is out and being answered.
    ///
    /// The one fact the rest of the loop asks for, and it is asked twice a
    /// round: the composer is muted for exactly as long as this is true — one
    /// question at a time — and Ctrl-C stops the turn rather than the session
    /// for exactly as long as it is true. Both readings come from here so that
    /// the key and the field cannot disagree about whether anything is running.
    pub(crate) const fn answering(&self) -> bool {
        self.turn.is_some()
    }

    /// Ask `message`, at `now`: on the thread card, and on a worker thread.
    ///
    /// The two halves of a submitted draft, in the order the reader experiences
    /// them. The question goes on the card first, which is also what brings the
    /// thread to the front, so somebody who has just asked something is looking
    /// at it from the instant they asked rather than from whenever the model
    /// first says anything. Then the worker, which owns its own copy of the
    /// message and of the agent and is never joined (see [`start_turn`]).
    ///
    /// `now` is the caller's clock — the instant the key was pressed — because
    /// that is what the turn's work lines are clocked from: a turn is as old as
    /// the question that asked it, not as old as the first thing the model got
    /// round to saying.
    ///
    /// Nothing stops a caller asking twice, because nothing has to: the field is
    /// muted for the whole of a turn, so the Enter that would ask a second
    /// question never becomes a submit. If one ever did, the second turn would
    /// simply replace the first — and the first's [`CancelGuard`] would drop and
    /// take its `claude` with it, which is the safe way for that accident to go.
    pub(crate) fn ask(&mut self, app: &mut App, message: &str, now: Instant) {
        self.say(app, message, message, Asked::Answer, now);
    }

    /// Ask `sent`, at `now`, showing `shown` on the card instead of it.
    ///
    /// [`Chat::ask`]'s whole body with the one thing a synthesized turn needs
    /// pulled apart: what the model is asked and what the reader is shown are
    /// two strings rather than one. `/brief` sends a paragraph of instructions
    /// warlock wrote and shows the word that was typed — the reader asked for
    /// one thing in one word, and a screen of prose they did not write, in the
    /// place their own questions go, would be warlock putting words in their
    /// mouth — which is the rule `warlock_tui`'s thread module states and this
    /// is the only way of keeping.
    ///
    /// Everything else is identical to a typed message, deliberately and by
    /// construction rather than by resemblance: the same [`App::start_turn`], the
    /// same [`start_turn`] worker, the same channel, the same say-when and the
    /// same drain at the bottom of the loop. So the instruction's reply lands
    /// under it like any other answer, its work lines are clocked from the
    /// keystroke like any other turn's, and Ctrl-C stops it like any other.
    ///
    /// The mode is read off the app rather than held here, and it decides one
    /// word of the argument vector: how hard the turn is asked to think (see
    /// [`asking`]). It is read at the moment the turn starts, so a `/brief` that
    /// has already set the mode sends its own instruction at the raised level,
    /// and the `/chat` that leaves the mode sends its own at `low`.
    ///
    /// `asked` is what this turn was started to get, and it is remembered on the
    /// turn itself rather than anywhere the loop keeps: see [`Asked`], which is
    /// the whole of the difference `/write` makes to the machinery here.
    pub(crate) fn say(
        &mut self,
        app: &mut App,
        shown: &str,
        sent: &str,
        asked: Asked,
        now: Instant,
    ) {
        app.start_turn(shown, now);
        self.turn = Some(start_turn(sent, &asking(&self.agent, app.mode()), asked));
        self.settle_field();
    }

    /// Point the field's muting back at the turn.
    ///
    /// The whole of the muting rule, and it is one thing: a question being
    /// answered somewhere else. One question at a time is what a conversation
    /// is — a second asked while the first is out is a second conversation — so
    /// the field says nothing until the answer lands.
    ///
    /// Called at the two points a turn can change and nowhere else: [`Chat::say`]
    /// starts one, [`Chat::keep_up`] is the only thing that ends one. So this is
    /// still *derived* from the turn rather than a flag five different endings
    /// have to remember to clear — a turn ends in five ways and all five of them
    /// arrive in `keep_up`. The loop used to work this out at the top of every
    /// round and set it from outside; the value it read is now in here with the
    /// field, so there is nowhere left for the two to disagree.
    ///
    /// A pact is deliberately *not* a reason. The two workers share nothing: a
    /// run writes documents on its own thread and reports into its own card, a
    /// turn asks its own `claude` and reports into another, and the loop drains
    /// both every round. Muting the field for a run was a guess about a limit
    /// that does not exist, and it cost the reader the thing they most want
    /// while a long pact runs, which is to ask something about the repository it
    /// is walking. What a run does take is the field's *card*: an account
    /// showing has no composer under it at all, so a question asked during a run
    /// is asked from the conversation, one swap away.
    fn settle_field(&mut self) {
        self.composer.set_muted(self.turn.is_some());
    }

    /// Stop the turn in flight, if there is one.
    ///
    /// What Ctrl-C comes to while a question is out. The turn is deliberately
    /// not taken down here: the worker still has one thing to say — that it was
    /// cancelled — and it says it through [`Chat::keep_up`] like any other
    /// ending, which is what puts the cancelled line under the work that had
    /// already arrived and hands the keyboard back to the field.
    ///
    /// Nothing at all with no turn in flight, which is a state this is never
    /// called in: the key means "leave" then, and the loop reads that off
    /// [`Chat::answering`] before it ever gets here.
    pub(crate) fn stop(&self) {
        if let Some(chatting) = self.turn.as_ref() {
            chatting.cancel.cancel();
        }
    }

    /// Apply everything the turn has said since the last frame.
    ///
    /// [`apply_turn`] against the turn this value holds, so the loop's bottom
    /// end is one call rather than a field reached into. It never blocks and it
    /// is what ends a turn: the moment it takes the turn down, the top of the
    /// next round finds [`Chat::answering`] false and gives the field back —
    /// which is how "the composer is live again the moment the turn ends,
    /// however it ends" is one rule rather than five.
    ///
    /// Do to the draft and to the keyboard whatever the composer just made of a
    /// key.
    ///
    /// The other half of [`warlock_tui::compose_for`], in the event loop's own
    /// mouse handler's shape and for its reason: three short arms that would
    /// otherwise be three more paragraphs in the loop. Nothing here reads the terminal, draws,
    /// starts a thread or writes a file — typing at the foot of the panel is the one
    /// thing in warlock that costs nothing but a redraw.
    ///
    /// The draft is a local of the loop and is handed in rather than read off the
    /// app, which is the whole of why it survives a run: a pact that recorded
    /// nothing puts the copy of the app taken before it back over the live one and
    /// keeps only the panel (see [`App::restore_from`]), and this function is the
    /// only thing in the binary that ever writes to the draft.
    ///
    /// The three arms:
    ///
    /// **Typing** is the draft replaced by the one [`compose_for`] just made —
    /// a character more, a character less, or a new line. The app is not told,
    /// because what somebody is halfway through writing is not a fact about the
    /// tree, and the next frame draws whatever the local now holds.
    ///
    /// **Leave** is Esc, the one key that means something different here to what it
    /// means anywhere else: it hands the keyboard back and leaves every character
    /// where it is. Nothing is thrown away — what somebody typed is worth more than
    /// the keystroke that stopped typing it — and the draft is not this arm's
    /// business at all, since a focus change cannot reach it. The panel rather than
    /// the tree, and the same landing [`App::set_focus`] rescues a hidden composer
    /// onto: the field is drawn under the panel, so the panel is what the reader is
    /// looking at and what the movement keys they press next should be about. Tab
    /// from there is one press back into the field.
    ///
    /// **Submit** is a draft offered up, and it is the one arm that can cost
    /// anything. Two statements happen whatever the draft turns out to be — it is
    /// taken, and the field is left empty — and then [`submitted_for`] says which of
    /// three things was submitted, because a submit is no longer the same as a
    /// question. Only a message is one.
    ///
    /// A **message** is what it always was: it goes on the thread as a new turn —
    /// which is also what brings the thread card to the front, so the reader is
    /// looking at the conversation from the instant they asked rather than from
    /// whenever the model first says something (see [`App::start_turn`]) — and then
    /// the worker: [`chatting::start_turn`] owns the channel, the say-when and this
    /// turn's copy of the agent, and what comes back is the one value the loop keeps
    /// about a turn it is not performing. Nothing is waited for here — everything
    /// the turn produces arrives at the bottom of the loop, exactly as a run's does.
    ///
    /// It is also the one place `brief_directory` is written. Where a brief goes is
    /// a fact about the mode rather than about the write, so it is settled at
    /// `/brief`, on this thread, out of what the repository says at that keystroke,
    /// and then held: by the time `/write` proposes a path it is a string the loop
    /// has been carrying for the whole conversation (see [`keep_up`]). What says so
    /// is `.warlock/briefs.toml` — [`load_briefs`], read here and nowhere else,
    /// answering [`DEFAULT_BRIEF_DIRECTORY`] where the repository has written no
    /// file or no `directory`.
    ///
    /// **`/brief`** is two things in the order the reader experiences them: the mode,
    /// and one ordinary turn. [`App::set_mode`] answers whether that was a *change*,
    /// and a change is worth exactly one unclocked note ([`BRIEF_NOTE`]) at the point
    /// in the history the command was typed — where a `/brief` typed in brief mode is
    /// a re-send with nothing new to say about the register and adds none. Then the
    /// turn, whichever it was: [`brief_asking`] goes into the conversation already in
    /// progress through the very path a typed message takes, shown on the card as
    /// [`BRIEF_COMMAND`] and never as the paragraph. So it costs a turn every time —
    /// which is the point of typing it again when the register has drifted — and the
    /// reply lands under it like any other answer.
    ///
    /// What that instruction states the shape is, and where the document it converges
    /// on would land, are both read out of the repository at this keystroke and at no
    /// other moment: `repo_root` is the root the loop already holds, and
    /// [`brief_reading`] loads `.warlock/brief-template.md` and `.warlock/briefs.toml`
    /// under it fresh every time. So a template edited between two briefs is a
    /// template the second one is held to, and a `directory` edited between them is
    /// where the next `/write` proposes — with `e` and no restart. Neither file is
    /// read at startup, and nothing about the template is kept on the app, on the
    /// conversation or in the loop; the directory is kept in exactly one place, the
    /// loop local this function writes.
    ///
    /// The mode is set *before* the turn is sent, and that ordering is load-bearing:
    /// the effort the turn is asked at is read off the app when the worker starts
    /// (see [`Chat::say`](chatting::Chat::say)), so the instruction that enters the
    /// mode is itself asked at the mode's level. The *load* comes before both, which
    /// is the other half of the ordering: a file that is there and cannot be read is
    /// one line ([`unreadable_template`], [`unreadable_briefs`]) and nothing else —
    /// no mode, no turn, no `claude`, and never warlock's own shape or its own
    /// default quietly used in its place. Where both files are broken it is still one
    /// line, the template's, because [`brief_reading`] stops at the first.
    ///
    /// **`/chat`** is the same shape pointed the other way, with one difference: it
    /// is refused when there is nothing to leave. In brief mode it leaves the mode,
    /// notes it once and sends [`CHAT_INSTRUCTION`] as one ordinary turn shown as
    /// [`CHAT_COMMAND`]; in chat mode it is [`ALREADY_CHATTING`] on the card and no
    /// turn at all, because the model was never told the register changed and telling
    /// it that it has not is a question nobody asked.
    ///
    /// Nothing on the card is cleared, hidden or reordered by either of them. A mode
    /// is a word warlock holds and a message into a session that is not replaced: the
    /// turns already on screen are the material the document is made of, and every
    /// one of them is still there, in order, with its answer and its work lines.
    ///
    /// **`/write`** is the ask for the artifact, and it is one ordinary turn: in
    /// brief mode [`WRITE_INSTRUCTION`] goes into the conversation already in
    /// progress by the path a typed message takes, shown on the card as
    /// [`WRITE_COMMAND`] and never as the paragraph, and what comes back lands as an
    /// answer like any other. It changes no mode and needs no ordering against one —
    /// the register is already what it is, and the effort the turn is asked at is
    /// already the mode's.
    ///
    /// Outside brief mode it is refused, on [`ALREADY_CHATTING`]'s rule: one
    /// unclocked line ([`NOT_BRIEFING`]) and no turn, because there is no document
    /// being converged on and asking for one anyway is a screenful of invention
    /// nobody wanted. The decision is read off [`App::mode`], which is the same
    /// state the panel's border title is drawn from, so the line and the title can
    /// never disagree about which register the conversation is in.
    ///
    /// A **refusal** is one line on the thread card and nothing else: no turn, no
    /// model, no `claude`. That line is the whole discovery mechanism for the three
    /// commands warlock has (see [`Submitted::refusal`]), and it is put on the card
    /// rather than the footer because it answers something the reader typed, in the
    /// place their own words are, and because a footer line is gone by the next
    /// keystroke that says anything.
    ///
    /// Cleared rather than kept in all three cases, which is the one place in here
    /// that can lose somebody's typing and is now honest: a submitted message is on
    /// the thread card a row above the field, so nothing is lost, and a field still
    /// holding the question that is being answered would be a field the next Enter
    /// asked it from again. A refused draft is the one thing that is genuinely
    /// thrown away, and the line it leaves says what to type instead — keeping it
    /// would leave the reader editing a word that has already been rejected in a
    /// field that looks exactly as it did before. Nothing is said on the footer
    /// either — a question and a refusal are both on screen, and warlock announcing
    /// what the reader can read would be warlock talking about itself.
    ///
    /// An empty or whitespace-only draft never arrives here at all — [`compose_for`]
    /// answers that Enter with the draft unchanged — so a submission with nothing in
    /// it is a keystroke rather than a mistake and has nothing to report. Neither
    /// does a submit while a turn is already in flight: the field is muted for the
    /// whole of one, so the Enter that would ask a second question is swallowed
    /// before it ever becomes a [`Composed`] (see [`press_for`]).
    ///
    /// Nothing chat-shaped goes anywhere near the engine. The message is handed to
    /// the chat agent and to the thread card, and to nothing else: the request a
    /// pact builds is what it always was, and a run is never told a word of this.
    ///
    /// [`compose_for`]: warlock_tui::compose_for
    /// [`submitted_for`]: warlock_tui::submitted_for
    /// [`Submitted::refusal`]: warlock_tui::Submitted::refusal
    pub(crate) fn compose(&mut self, app: &mut App, outcome: Composed, now: Instant) {
        match outcome {
            Composed::Typing(next) => self.composer = next,
            Composed::Leave => app.set_focus(Focus::Panel),
            Composed::Submit => {
                // Taken before the field is emptied, and emptied by replacing it
                // outright: the muting is put back at the top of the next round from
                // the turn alone, which is what makes "however the turn ends, the
                // field comes back" one line in the loop rather than a flag to unset
                // on five paths.
                let draft = self.composer.draft().to_owned();
                self.composer = Composer::default();

                match submitted_for(&draft) {
                    // The arm that was here before the other three existed: the
                    // words go to the model as they were typed.
                    Submitted::Message => self.ask(app, &draft, now),
                    // The mode, then the turn — in that order, because the turn is
                    // asked at the level the mode it is entering is worth. The note
                    // is the change and not the command, so typing `/brief` twice
                    // costs two turns and one line.
                    Submitted::Brief => match brief_reading(&self.root) {
                        // Both files are read from the repository at the keystroke
                        // — before the mode is touched, because a file that cannot
                        // be read is a command that does not happen at all and a
                        // mode set first would be a register entered by a refusal.
                        Ok((instruction, directory)) => {
                            // Where a `/write` in this mode will propose to put the
                            // document, settled here and held until the next
                            // `/brief` — which is what keeps `/write` from reading
                            // anything and therefore from failing. It is what
                            // `.warlock/briefs.toml` says, or the engine's default
                            // where it says nothing, and it is written on every
                            // `/brief` rather than left wherever the last one put
                            // it: that is the whole of what makes the file edited
                            // between two briefs take effect without a restart.
                            self.directory = directory;
                            if app.set_mode(Mode::Brief) {
                                app.note(BRIEF_NOTE, now);
                            }
                            self.say(app, BRIEF_COMMAND, &instruction, Asked::Answer, now);
                        }
                        // A file that is there and cannot be had: one line, no
                        // mode, no turn, and neither warlock's own shape nor its
                        // own default quietly put in its place. See
                        // [`brief_reading`], [`unreadable_template`] and
                        // [`unreadable_briefs`].
                        Err(line) => app.note(line, now),
                    },
                    // The same, one way only: there is no register to leave in chat
                    // mode, so the command says so on the card and stops. A turn
                    // spent telling the model it is where it already was would be a
                    // question nobody asked and money nobody meant to spend.
                    Submitted::Chat => {
                        if app.set_mode(Mode::Chat) {
                            app.note(CHAT_NOTE, now);
                            self.say(app, CHAT_COMMAND, CHAT_INSTRUCTION, Asked::Answer, now);
                        } else {
                            app.note(ALREADY_CHATTING, now);
                        }
                    }
                    // The artifact, asked for as one ordinary turn — and only where
                    // there is one to ask for. The mode comes off the app rather
                    // than off anything this function remembers, because that is
                    // the state the border title is drawn from and two readings of
                    // the register would eventually be two answers.
                    Submitted::Write => {
                        if app.mode() == Mode::Brief {
                            self.say(app, WRITE_COMMAND, WRITE_INSTRUCTION, Asked::Document, now);
                        } else {
                            app.note(NOT_BRIEFING, now);
                        }
                    }
                    // The one that stops here, without a question and without a
                    // turn: a refusal has exactly one line to say, asked of the
                    // value rather than restated here, so the list of what exists is
                    // written down in one place.
                    said @ Submitted::Refused => {
                        if let Some(line) = said.refusal() {
                            app.note(line, now);
                        }
                    }
                }
            }
        }
    }

    /// Nothing comes back, and that is the point of where the window now lives.
    /// [`apply_turn`] hands up the document a `/write` turn answered with, on
    /// the one round that turn ended in — and the window that opens over it is
    /// this value's own, so it is opened here rather than by a loop that had to
    /// be handed the output directory and a second [`ScopePrompt`] to do it.
    ///
    /// Every other way that turn could have gone hands back nothing, so a
    /// `claude` that is missing, a non-zero exit, a timeout and a cancel all
    /// leave the window closed: the ending is the one line `apply_turn` already
    /// put on the thread.
    ///
    /// The path is proposed in a directory settled turns ago, so this line reads
    /// no file and has no failure to report. That is the whole of why the
    /// directory is a field: a window that opens over a finished document must
    /// be a window that opens.
    pub(crate) fn keep_up(&mut self, app: &mut App, now: Instant) {
        if let Some(document) = apply_turn(&mut self.turn, app, now) {
            self.prompt = write_opened(&self.root, &self.directory, &document);
        }
        self.settle_field();
    }

    /// What one keystroke *inside* the write window comes to.
    ///
    /// The window is this value's, so what a key does to it is settled here and
    /// the loop keeps no copy to put back. The manifest is still handed in: the
    /// scope key writes it and a run saves it, so it is the loop's and this only
    /// reads it. See [`write_edit`](crate::writing::write_edit), which is the
    /// arithmetic and the write itself.
    pub(crate) fn write(
        &mut self,
        app: &mut App,
        manifest: &Manifest,
        edited: Edited,
        now: Instant,
    ) {
        self.prompt = write_edit(app, manifest, &self.root, &self.prompt, edited, now);
    }
}

/// A turn being answered by a worker thread, from the point of view of the
/// thread drawing the screen.
///
/// Two things, and no handle to join: what the worker has to say, and how to
/// tell it to stop. [`Running`](crate::pacting) keeps two more — what the run
/// was asked to do, and the app as it stood before the keystroke painted
/// anything — and a turn needs neither. What it was asked is already on the
/// thread card as the reader's own message, put there by the keystroke that
/// started this; and there is nothing to put back, because a turn changes no
/// row, no colour and no file.
///
/// The say-when is a [`CancelGuard`] rather than a bare [`Cancel`] for exactly
/// the reason a run's is: every way out of the event loop drops this value, and
/// each of them would otherwise have to remember to stop the turn — which is to
/// say one of them eventually would not, and would leave a `claude` burning the
/// user's subscription with nobody left to read what it says.
pub(crate) struct Chatting {
    /// The work as it happens and, once, how the turn ended. Closed by the
    /// worker dropping its end, which is how a panicked worker is noticed.
    pub(crate) events: Receiver<TurnEvent>,
    /// Say-when for the worker: the handle its agent answers to, and the kill
    /// switch for the `claude` it is waiting on.
    pub(crate) cancel: CancelGuard,
    /// What this turn was started to get, fixed at the keystroke that started
    /// it and never read by the worker: it is the drain's, and it decides
    /// whether the answer is handed back as well as put on the card.
    pub(crate) asked: Asked,
}

/// What a turn was started to get.
///
/// The one thing a `/write` turn is not like every other turn in, and it is
/// written down *here*, on the turn, rather than as a flag beside the turn in
/// the event loop. A loop-side flag would be a second record of which question
/// is out, and the two would disagree the first time a turn ended in a way
/// nobody remembered to clear it on — a cancelled `/write`, a `claude` that is
/// not installed, a worker that panicked. There is one turn at a time and this
/// rides on it, so it goes down exactly when the turn does.
///
/// It changes nothing about how the turn is run. The same worker, the same
/// channel, the same agent, the same say-when, the same clock and the same
/// endings: what it decides is one thing at the drain, which is whether the
/// answer is handed back to the loop as well as put on the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Asked {
    /// An answer to read, which is what every turn a reader types is for. The
    /// answer lands on the card and the loop is told nothing.
    Answer,
    /// The document `/write` asked for. The answer lands on the card exactly as
    /// an ordinary one does — it is the reply, and the reader can read it — and
    /// it is handed back besides, so the loop can open the path prompt over it
    /// (see [`apply_turn`]).
    Document,
}

/// What a turn's worker thread has to say for itself.
///
/// Two things, and the order of them is fixed: any number of
/// [`TurnEvent::Doing`] while the model works, and then exactly one
/// [`TurnEvent::Finished`]. Nothing else is sent and nothing is sent after the
/// end — the worker drops its end of the channel and stops.
///
/// One channel for both, as a run has one channel for its progress and its
/// activities, and for the same reason: they come from the same worker, they are
/// read by the same thread, and a second receiver would be a second thing the
/// event loop has to poll and a second way for the two streams to arrive out of
/// the order the turn produced them in. It is also what makes the answer land
/// *after* the work lines it was produced by, on screen as in life.
#[derive(Debug)]
pub(crate) enum TurnEvent {
    /// The model was seen doing something: a tool call, a stretch of thinking,
    /// a stretch of writing, or what it cost.
    ///
    /// Carries nothing to say which turn it belongs to, because it needs
    /// nothing: one question at a time is the rule the loop keeps, so the live
    /// turn is the only turn anything can be filed under.
    Doing(Activity),
    /// The turn is over, however it went: the answer, or the one [`Ending`] the
    /// thread and the footer are worded from.
    ///
    /// A `Result` because a turn either answered or did not, and never a
    /// `Result` this module hands *out*: see [`apply_turn`], where both arms
    /// come to a row.
    Finished(Result<String, Ending>),
}

/// An activity port that forwards to `events`, for the agent of one turn.
///
/// [`pacting`](crate::pacting)'s port with a different event around it, and the
/// same three properties, which are the whole of why it is made per turn rather
/// than kept on the loop's long-lived agent. It is attached to a copy of the
/// agent that dies with the worker, so a `claude` still writing to a pipe after
/// its turn was abandoned has nowhere to report into the turn after it. It is
/// called on the worker's thread from inside the run, while the run is still
/// going, so it does the least it can: one send and back. And a send that fails
/// is ignored, because a receiver that has gone away is an application that is
/// quitting, and failing a turn that is otherwise going fine for the sake of a
/// screen nobody is looking at would help nobody.
fn activity_port(events: &Sender<TurnEvent>) -> Activities {
    let events = events.clone();
    Activities::new(move |activity| {
        let _ = events.send(TurnEvent::Doing(activity));
    })
}

/// This turn's own copy of the agent: the loop's, answering to `cancel` and
/// reporting to `events`.
///
/// The pair of handles that make a turn a thing that can be stopped and
/// watched, attached in one place so that the worker the loop starts and the
/// worker a test drives are wired the same way rather than similarly. A
/// [`ChatAgent`] is a command line, a session id and a timeout, so the copy
/// costs nothing and the loop's own agent keeps the handles it was built
/// with — which nobody holds and nothing listens to, so a cancel can never
/// reach out of the turn it was pressed for.
fn wired(agent: &ChatAgent, cancel: &Cancel, events: &Sender<TurnEvent>) -> ChatAgent {
    agent
        .clone()
        .with_cancel(cancel.clone())
        .with_activities(activity_port(events))
}

/// The loop's own agent, asked at the level `mode` is worth.
///
/// The whole of what a mode changes about the model, and it is one word of one
/// argument: a brief-mode turn is asked at [`BRIEF_EFFORT`] because proposing
/// options, costing them and arguing for one is not what `low` is priced for,
/// and a chat turn is asked at the level the agent was built with. Everything
/// else is byte-identical between the two — the same system prompt, the same
/// tools, the same session, down to whether this turn opens the conversation or
/// resumes it — because the returned agent is a clone of the one long-lived
/// [`ChatAgent`] and shares the very latch the session id is claimed in.
///
/// Which is the property a mode change has to have, and the reason this is a
/// clone rather than a rebuild: the turns already said are the material the
/// document is made of, and a second `ChatAgent` would be a second conversation
/// that had never heard any of them. Nothing here constructs one, and nothing
/// anywhere else does either once the process has started.
///
/// Chat mode is the agent untouched rather than the agent asked for `low` again,
/// so the level a fresh agent takes — including a `WARLOCK_EFFORT` a reader set
/// for the session — is carried rather than restated here. Brief mode goes
/// through [`ChatAgent::at_effort`], which lets the same variable win over
/// [`BRIEF_EFFORT`] too.
fn asking(agent: &ChatAgent, mode: Mode) -> ChatAgent {
    match mode {
        Mode::Brief => agent.at_effort(BRIEF_EFFORT),
        Mode::Chat => agent.clone(),
    }
}

/// Put `message` to the model on a thread of its own, and hand back the channel
/// the answer comes back on.
///
/// The worker owns everything it touches — its own copy of the message and its
/// own copy of the agent — so nothing is shared with the event loop but the
/// channel and `cancel`, and that is what makes the thread safe to abandon: the
/// loop can return and the process exit at any moment without waiting for it,
/// because there is no state the two of them are half way through agreeing on.
/// A turn writes no file, so there is not even a half-written one to worry
/// about; the only thing it can leave behind is a child process, and the handle
/// is what takes that with it.
///
/// The [`JoinHandle`](std::thread::JoinHandle) is dropped on purpose, as a
/// run's is: joining is waiting, and this thread exists precisely so that
/// nobody waits for it. The worker reports on every path it takes itself, so
/// the only way the channel closes without a [`TurnEvent::Finished`] is a panic
/// in the worker — which [`apply_turn`] reads as the turn being over rather than
/// as a reason to hang.
pub(crate) fn spawn_turn(message: &str, agent: &ChatAgent, cancel: Cancel) -> Receiver<TurnEvent> {
    let (events, received) = mpsc::channel();
    let message = message.to_owned();
    let agent = wired(agent, &cancel, &events);
    thread::spawn(move || run_turn(&message, &agent, &cancel, &events));
    received
}

/// Everything a submitted message comes to, once the composer has decided there
/// is one.
///
/// [`start_run`](crate::pacting)'s opposite number: the one value the event loop
/// keeps about a turn it is not performing, made in one place so that starting a
/// turn is one call rather than a channel, a handle and an agent assembled
/// correctly at the keystroke.
///
/// The handle is made here and never reused, for the reason a run's is: a cancel
/// is final, so the turn after a cancelled one has to start with a handle nobody
/// has said stop to.
///
/// Nothing is said to the app here. The reader's message goes on the thread card
/// by [`App::start_turn`](warlock_tui::App::start_turn), at the keystroke, so
/// that the question is on screen from the moment it is asked rather than from
/// whenever the model first says something.
///
/// `asked` is carried rather than acted on: it is what the drain reads when this
/// turn ends, and nothing between here and there consults it.
pub(crate) fn start_turn(message: &str, agent: &ChatAgent, asked: Asked) -> Chatting {
    let cancel = CancelGuard::new();
    Chatting {
        events: spawn_turn(message, agent, cancel.handle()),
        cancel,
        asked,
    }
}

/// The worker thread's whole body: ask, and say how it went.
///
/// Written as a function of its channel rather than inside the closure so that
/// it can be driven straight from a test — against a stand-in program, on the
/// test's own thread — and what a real turn sends asserted without a terminal, a
/// thread or a `claude`, exactly as [`run_pact`](crate::pacting) is.
///
/// Exactly one [`TurnEvent::Finished`] is sent, always, and it is the last thing
/// this function does. There is no path out of here that says nothing: a failure
/// is an ending like any other, and the wording of every one of them is
/// [`ending_for`]'s rather than this file's, so the panel's vocabulary for a
/// failed turn is written once, beside the thread it is drawn on.
///
/// The cancel is the one thing the seam cannot word for itself, and this is
/// where that fact is held. A killed child comes back as interrupted I/O and
/// nothing about the errno says who killed it, so a failure with the handle
/// latched is read as [`Ending::Cancelled`] here — which is what turns `the turn
/// could not run — …` into `the turn was cancelled` for the one person who
/// already knows why. An answer that beat the cancel by a hair is kept: it is a
/// real answer, and throwing it away would be a lie in the other direction.
fn run_turn(message: &str, agent: &ChatAgent, cancel: &Cancel, events: &Sender<TurnEvent>) {
    let finished = match agent.turn(message) {
        Ok(answer) => Ok(answer),
        Err(_) if cancel.is_cancelled() => Err(Ending::Cancelled),
        Err(error) => Err(ending_for(&error)),
    };
    // Ignored for the reason the activity port's sends are: a receiver that has
    // gone away is an application that is quitting.
    let _ = events.send(TurnEvent::Finished(finished));
}

/// Apply everything the turn has said since the last frame, and take it down
/// once it has said how it ended.
///
/// [`Pact::keep_up`](crate::pacting::Pact::keep_up)'s opposite number and the loop's whole
/// half of a turn. Drained rather than received: the worker is not waiting for
/// this thread, so a burst of tool calls can arrive between two frames, and the
/// loop ends the moment there is nothing left to read — which is the ordinary
/// case, since a turn is seconds and a frame is a tenth of one. Nothing here can
/// block, and that is the point of it: the tree still scrolls and the clocks
/// still tick while the model thinks.
///
/// Every ending is two lines and no error. One goes on the thread, under
/// whatever work had already arrived, so a turn cancelled after two tool calls
/// still shows those two tool calls; the other goes on the footer, in the same
/// words, because the reader may well be looking at the account or a document
/// rather than at the conversation. They are the same sentence by construction
/// rather than by two spellings happening to agree — [`Ending::line`] is asked
/// twice.
///
/// An answer says nothing on the footer. It is already on screen, on the card
/// the question brought to the front, and a footer line announcing it would be
/// warlock telling the reader about something they are looking at.
///
/// A channel that closes without an ending is a worker that panicked. It is
/// treated as the turn having ended, because it has: the panic hook has already
/// printed what happened, this loop is still drawing, and waiting for a message
/// from a thread that no longer exists would hang warlock on the one path where
/// it can least afford to.
///
/// `now` is the caller's clock, and this function reads none of its own: every
/// event drained here is filed under it, so a whole turn is drivable from a base
/// instant in a test exactly as the thread below it is.
///
/// One thing comes back, and only on the one round a `/write` turn answers in:
/// the document, so the loop can open the path prompt over it. It is handed back
/// rather than left for the loop to read off the card because *this* is where a
/// turn is known to have been the write request — the turn carries what it was
/// started for ([`Asked`]) — and a loop that went looking for the newest answer
/// afterwards would be a second opinion about which turn just ended. It costs
/// one clone of one answer, on the one round in a session that asks for a
/// document; the card gets the original, so there is still exactly one copy of
/// the reply anybody draws or writes.
///
/// Every other round hands back nothing, and that includes every way a `/write`
/// turn can fail: an ending is an ending, so no prompt opens, no file is
/// proposed and the conversation is as usable as it was.
pub(crate) fn apply_turn(
    chat: &mut Option<Chatting>,
    app: &mut App,
    now: Instant,
) -> Option<String> {
    // No turn, nothing drained — which is what almost every frame of warlock's
    // life does here.
    let chatting = chat.as_ref()?;
    let asked = chatting.asked;

    let finished = loop {
        match chatting.events.try_recv() {
            // Filed under the live turn, which is the one this worker is
            // answering: what each activity comes to is the thread's business
            // and not this file's — a tool is its name and its one detail,
            // thinking and writing are the words for them, and a cost is summed
            // rather than drawn. See `Thread::record`.
            Ok(TurnEvent::Doing(activity)) => app.record_turn(&activity, now),
            Ok(TurnEvent::Finished(finished)) => break Some(finished),
            // Still going, and nothing new to say.
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => break None,
        }
    };

    // The turn is over on every path below, so the loop stops holding it before
    // anything is worded: what the reader does next — another question, or the
    // key that cancels — is answered by an empty slot rather than by a receiver
    // nobody will ever hear from again.
    chat.take();
    match finished {
        Some(Ok(answer)) => {
            // Cloned only for the turn that asked for a document, and cloned
            // rather than moved because the answer belongs on the card first:
            // the reply is a turn of the conversation whatever is done with it,
            // and a `/write` whose answer went to the loop instead of the thread
            // would be a document nobody could read.
            let document = (asked == Asked::Document).then(|| answer.clone());
            app.answer_turn(answer, now);
            document
        }
        Some(Err(ending)) => {
            end(app, &ending, now);
            None
        }
        None => {
            end(
                app,
                &Ending::Broke {
                    reason: TURN_LOST.to_owned(),
                },
                now,
            );
            None
        }
    }
}

/// End the live turn with `ending`, and say the same thing on the footer.
///
/// The two halves of a failure, in one place so that they cannot drift: a reader
/// on the thread card sees the line where the turn was, and a reader anywhere
/// else in the panel still hears that the question they asked is not coming
/// back. Neither of them is an error out of the event loop, and neither of them
/// touches anything else the app is holding — the tree, the account and the
/// document card are exactly as they were, which is what makes the next question
/// as ordinary as the last.
fn end(app: &mut App, ending: &Ending, now: Instant) {
    app.set_message(ending.line());
    app.end_turn(ending, now);
}

/// What one turn on a worker thread does: what it says on the way, what it ends
/// with, and what all of that comes to on the app the loop is drawing.
///
/// Two halves, and the seam between them is the channel. The tests below the
/// [`unix`](tests::unix) module drive [`apply_turn`] over a channel this file
/// writes to by hand, so what the loop does with an event is asserted without
/// running anything at all; the ones inside it drive [`run_turn`] and
/// [`spawn_turn`] against `/bin/sh` stand-ins, one per way a turn can fail, so
/// what a real `claude` would come back as is asserted without a `claude`. No
/// terminal, no network and no model in either half.
#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::{Duration, Instant};

    use warlock_tui::{Activity, App, ChatAgent, Ending, INVOCATION_TIMEOUT, Line, Mode};

    use super::{Asked, Chat, Chatting, TurnEvent, apply_turn};
    use crate::pacting::CancelGuard;

    /// The question every test asks, so a row that quotes it is recognisable.
    const ASKED: &str = "what is in crates/warlock-engine?";

    /// `base` plus `seconds`, so a turn's whole timeline is one instant and
    /// some arithmetic rather than a sleep.
    fn at(base: Instant, seconds: u64) -> Instant {
        base + Duration::from_secs(seconds)
    }

    /// An app with [`ASKED`] on its thread card and a turn in flight under it,
    /// and the loop's end of the channel that turn reports on.
    ///
    /// Everything a submitted message leaves behind at the keystroke: the
    /// question is on the card already — [`App::start_turn`] put it there — and
    /// the worker is represented by a receiver this test sends down itself.
    fn asking(base: Instant) -> (App, Sender<TurnEvent>, Option<Chatting>) {
        let (events, received) = mpsc::channel();
        let mut app = App::default();
        app.start_turn(ASKED, base);
        (app, events, Some(chatting(received)))
    }

    /// The value the loop keeps for a turn reporting on `received`: an ordinary
    /// question, which is what every turn but `/write`'s is.
    fn chatting(received: Receiver<TurnEvent>) -> Chatting {
        turn_for(received, Asked::Answer)
    }

    /// The same value for the turn `/write` starts: the one turn whose answer
    /// the drain hands back.
    fn writing(received: Receiver<TurnEvent>) -> Chatting {
        turn_for(received, Asked::Document)
    }

    /// The value the loop keeps for a turn reporting on `received` that was
    /// started to get `asked`.
    fn turn_for(received: Receiver<TurnEvent>, asked: Asked) -> Chatting {
        Chatting {
            events: received,
            cancel: CancelGuard::new(),
            asked,
        }
    }

    /// The loop's drain over `chat`, for a turn that is not the write request.
    ///
    /// [`apply_turn`] with the one thing it hands back asserted away: an
    /// ordinary turn's answer goes on the card and nowhere else, so every test
    /// here that is not about `/write` gets that promise for free rather than
    /// restating it.
    fn drain(chat: &mut Option<Chatting>, app: &mut App, now: Instant) {
        assert_eq!(
            apply_turn(chat, app, now),
            None,
            "an ordinary turn handed something back to the loop"
        );
    }

    /// Every row of the app's thread card, clocked against `now`.
    fn rows(app: &App, now: Instant) -> Vec<Line> {
        app.thread().expect("a question has been asked").lines(now)
    }

    /// The reader's own message as the card draws it.
    fn said() -> Line {
        Line::Said {
            text: ASKED.to_owned(),
        }
    }

    /// A work line, clocked at `seconds` past the question.
    fn clocked(seconds: u64, text: &str) -> Line {
        Line::Clocked {
            clock: format!("0:{seconds:02}"),
            text: text.to_owned(),
        }
    }

    #[test]
    fn work_lines_land_as_they_arrive_and_the_turn_stays_in_flight() {
        let base = Instant::now();
        let (mut app, events, mut chat) = asking(base);

        events
            .send(TurnEvent::Doing(Activity::Tool {
                name: "Read".to_owned(),
                detail: Some("src/lib.rs".to_owned()),
            }))
            .expect("the loop is still listening");
        events
            .send(TurnEvent::Doing(Activity::Thinking))
            .expect("the loop is still listening");
        drain(&mut chat, &mut app, at(base, 3));

        // Drained and returned: the worker has said nothing since, and this
        // call did not sit and wait for it to.
        assert!(chat.is_some(), "an unfinished turn was taken down");
        assert_eq!(
            rows(&app, at(base, 3)),
            vec![
                said(),
                clocked(3, "Read src/lib.rs"),
                clocked(3, "thinking")
            ]
        );
        // Nothing on the footer while a turn is going: the panel is where the
        // work is being shown, and the footer has its own things to say.
        assert_eq!(app.message(), None);
    }

    #[test]
    fn an_answer_lands_on_the_thread_and_takes_the_turn_down() {
        let base = Instant::now();
        let (mut app, events, mut chat) = asking(base);

        events
            .send(TurnEvent::Doing(Activity::Writing { bytes: 0 }))
            .expect("the loop is still listening");
        events
            .send(TurnEvent::Finished(Ok(
                "The tree and the manifest.".to_owned()
            )))
            .expect("the loop is still listening");
        drain(&mut chat, &mut app, at(base, 4));

        assert!(chat.is_none(), "an answered turn is still in flight");
        assert_eq!(
            rows(&app, at(base, 9)),
            vec![
                said(),
                // Frozen where the answer landed rather than counting up to
                // the `now` above: the turn is over.
                clocked(4, "writing"),
                Line::Text {
                    text: "The tree and the manifest.".to_owned()
                },
            ]
        );
        // An answer is on screen where it was asked for, so the footer says
        // nothing about it.
        assert_eq!(app.message(), None);
    }

    #[test]
    fn a_write_turn_hands_its_answer_back_and_leaves_it_on_the_card_as_well() {
        // The one difference `/write` makes to the drain, and both halves of it:
        // the loop is handed the document — which is what it opens the path
        // prompt over — and the card keeps the reply, which is what the reader
        // reads and what the write itself takes its bytes from.
        const DOCUMENT: &str = "# Scopes\n\nA boundary somebody drew.";

        let base = Instant::now();
        let (events, received) = mpsc::channel();
        let mut app = App::default();
        app.start_turn("/write", base);
        let mut chat = Some(writing(received));

        events
            .send(TurnEvent::Finished(Ok(DOCUMENT.to_owned())))
            .expect("the loop is still listening");
        let document = apply_turn(&mut chat, &mut app, at(base, 4));

        assert_eq!(document.as_deref(), Some(DOCUMENT));
        assert!(chat.is_none(), "an answered turn is still in flight");
        assert_eq!(
            rows(&app, at(base, 4)),
            vec![
                Line::Said {
                    text: "/write".to_owned()
                },
                clocked(4, "waiting"),
                // The card holds the answer as the lines it is made of, which
                // is how every answer lands on it: what matters here is that
                // the document is still there as well as in the loop's hand.
                Line::Text {
                    text: "# Scopes".to_owned()
                },
                Line::Text {
                    text: String::new()
                },
                Line::Text {
                    text: "A boundary somebody drew.".to_owned()
                },
            ],
            "the reply left the card it was answered on"
        );
        assert_eq!(app.message(), None);
    }

    #[test]
    fn a_write_turn_that_fails_hands_nothing_back_however_it_failed() {
        // Every ending, over the turn that asked for a document: a failure is a
        // failure, so there is nothing for the loop to open a prompt over and
        // nothing anywhere for it to write.
        let endings = [
            Ending::Cancelled,
            Ending::NoModel {
                program: "claude".to_owned(),
            },
            Ending::Failed {
                code: Some(3),
                stderr: "boom".to_owned(),
            },
            Ending::TimedOut {
                after: INVOCATION_TIMEOUT,
            },
            Ending::NothingSaid,
        ];

        for ending in endings {
            let base = Instant::now();
            let (events, received) = mpsc::channel();
            let mut app = App::default();
            app.start_turn("/write", base);
            let mut chat = Some(writing(received));

            events
                .send(TurnEvent::Finished(Err(ending.clone())))
                .expect("the loop is still listening");

            assert_eq!(
                apply_turn(&mut chat, &mut app, at(base, 2)),
                None,
                "{ending:?} handed the loop a document to write"
            );
            assert!(chat.is_none(), "{ending:?} left the turn in flight");
            assert_eq!(app.message(), Some(ending.line().as_str()), "{ending:?}");
        }
    }

    #[test]
    fn a_write_turn_whose_worker_died_hands_nothing_back_either() {
        // The sixth ending, which arrives as a closed channel rather than as a
        // message: the same nothing, by the road that has no `Ending` on it.
        let base = Instant::now();
        let (events, received) = mpsc::channel();
        let mut app = App::default();
        app.start_turn("/write", base);
        let mut chat = Some(writing(received));

        drop(events);

        assert_eq!(apply_turn(&mut chat, &mut app, at(base, 2)), None);
        assert!(chat.is_none(), "a dead worker is still in flight");
        assert!(app.message().is_some(), "the footer says the turn is over");
    }

    #[test]
    fn a_write_turn_still_in_flight_hands_nothing_back() {
        // The round in the middle: work has arrived, the answer has not, and
        // the prompt has nothing to open over yet.
        let base = Instant::now();
        let (events, received) = mpsc::channel();
        let mut app = App::default();
        app.start_turn("/write", base);
        let mut chat = Some(writing(received));

        events
            .send(TurnEvent::Doing(Activity::Thinking))
            .expect("the loop is still listening");

        assert_eq!(apply_turn(&mut chat, &mut app, at(base, 1)), None);
        assert!(chat.is_some(), "an unfinished turn was taken down");
    }

    #[test]
    fn every_ending_is_one_line_on_the_thread_and_the_same_line_on_the_footer() {
        let endings = [
            Ending::Cancelled,
            Ending::NoModel {
                program: "claude".to_owned(),
            },
            Ending::Failed {
                code: Some(3),
                stderr: "boom\nand more boom".to_owned(),
            },
            Ending::TimedOut {
                after: INVOCATION_TIMEOUT,
            },
            Ending::NothingSaid,
            Ending::Broke {
                reason: "a pipe broke".to_owned(),
            },
        ];

        for ending in endings {
            let base = Instant::now();
            let (mut app, events, mut chat) = asking(base);

            events
                .send(TurnEvent::Finished(Err(ending.clone())))
                .expect("the loop is still listening");
            drain(&mut chat, &mut app, at(base, 2));

            assert!(chat.is_none(), "{ending:?} left the turn in flight");
            // One row for the question and one for the ending, whichever
            // ending it is: a failure costs the reader a line, not a screen.
            assert_eq!(
                rows(&app, at(base, 30)),
                vec![said(), clocked(2, &ending.line())],
                "{ending:?}"
            );
            // And the same sentence for a reader who is looking at another
            // card. Same string, from the same place: two spellings of one
            // failure is two things to keep in step.
            assert_eq!(app.message(), Some(ending.line().as_str()), "{ending:?}");
            let turn = app
                .thread()
                .and_then(|thread| thread.turns().last().map(|turn| (**turn).clone()))
                .expect("the turn is on the card");
            assert_eq!(turn.ending(), Some(&ending));
            assert_eq!(turn.answer(), None);
            // Nothing was returned from any of this: the arm above is the
            // whole of what a failed turn does to the loop.
        }
    }

    #[test]
    fn a_cancel_keeps_every_line_that_arrived_before_it_and_adds_one() {
        // What Ctrl-C during a turn leaves behind. The work the model was seen
        // doing really happened, so it stays where it is; the cancel is one more
        // line under it, in the ordinary shape of an ending. A cancel that
        // cleared the turn would throw away the two tool calls the reader was
        // watching, which is the reader's evidence for what they just stopped.
        let base = Instant::now();
        let (mut app, events, mut chat) = asking(base);

        for name in ["Read", "Grep"] {
            events
                .send(TurnEvent::Doing(Activity::Tool {
                    name: name.to_owned(),
                    detail: Some("src/lib.rs".to_owned()),
                }))
                .expect("the loop is still listening");
        }
        drain(&mut chat, &mut app, at(base, 2));
        assert!(chat.is_some(), "the turn is still in flight");

        // And then the cancel, which arrives as the worker's one ending like
        // any other — the loop does not take the turn down at the keystroke.
        events
            .send(TurnEvent::Finished(Err(Ending::Cancelled)))
            .expect("the loop is still listening");
        drain(&mut chat, &mut app, at(base, 4));

        assert!(chat.is_none(), "a cancelled turn is still in flight");
        assert_eq!(
            rows(&app, at(base, 30)),
            vec![
                said(),
                clocked(2, "Read src/lib.rs"),
                // The line that was newest when the cancel landed, frozen at
                // the moment it landed: it had been ticking since the drain
                // above, which is the clock rule the account already follows.
                clocked(4, "Grep src/lib.rs"),
                clocked(4, &Ending::Cancelled.line()),
            ]
        );
    }

    #[test]
    fn a_worker_that_dies_without_saying_how_it_went_still_ends_the_turn() {
        let base = Instant::now();
        let (mut app, events, mut chat) = asking(base);

        // A panicked worker, as this thread sees it: its end of the channel is
        // gone and no ending ever arrived.
        drop(events);
        drain(&mut chat, &mut app, at(base, 5));

        assert!(chat.is_none(), "a dead worker is still in flight");
        let line = app.message().expect("the footer says the turn is over");
        assert!(line.contains("stopped without saying"), "{line}");
        assert_eq!(rows(&app, at(base, 40)), vec![said(), clocked(5, line)]);
    }

    #[test]
    fn a_second_question_runs_as_ordinarily_as_the_first_did() {
        let base = Instant::now();
        let (mut app, events, mut chat) = asking(base);
        events
            .send(TurnEvent::Finished(Err(Ending::NothingSaid)))
            .expect("the loop is still listening");
        drain(&mut chat, &mut app, at(base, 1));

        // The session is as usable as it was: a new turn, a new channel, and
        // the failed one still on the card above it.
        let (again, received) = mpsc::channel();
        let mut chat = Some(chatting(received));
        app.start_turn("and which of those is the biggest?", at(base, 10));
        again
            .send(TurnEvent::Finished(Ok("The engine.".to_owned())))
            .expect("the loop is still listening");
        drain(&mut chat, &mut app, at(base, 12));

        assert!(chat.is_none());
        assert_eq!(
            rows(&app, at(base, 12)),
            vec![
                said(),
                clocked(1, &Ending::NothingSaid.line()),
                Line::Said {
                    text: "and which of those is the biggest?".to_owned()
                },
                // A turn that answered without ever being seen doing anything
                // keeps the placeholder it was drawn with, frozen at the
                // moment the answer landed: two seconds of waiting is what
                // happened, and the thread says so.
                clocked(2, "waiting"),
                Line::Text {
                    text: "The engine.".to_owned()
                },
            ]
        );
    }

    /// An agent's whole argument vector as strings, for a test that is about
    /// what a mode does to it.
    fn words(agent: &warlock_tui::ChatAgent) -> Vec<String> {
        agent
            .args()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    /// What `flag` was given, if it was given anything: `claude.rs`'s helper,
    /// so a test says what it is about rather than where the word happens to
    /// sit.
    fn value_of<'a>(vector: &'a [String], flag: &str) -> Option<&'a str> {
        let named = vector.iter().position(|word| word == flag)?;
        vector.get(named + 1).map(String::as_str)
    }

    #[test]
    fn a_mode_asks_the_one_conversation_at_a_different_level_and_nothing_else() {
        // What `asking` is allowed to change, asserted on the vector a turn
        // would really run with. Chat mode is the long-lived agent untouched;
        // brief mode is the same agent with one word moved. Nothing here builds
        // a second `ChatAgent`, and neither does the function under test — both
        // vectors name the same session, which is the property the whole design
        // rests on.
        let agent = warlock_tui::ChatAgent::new();
        let question = words(&super::asking(&agent, warlock_tui::Mode::Chat));
        let brief = words(&super::asking(&agent, warlock_tui::Mode::Brief));

        assert_eq!(
            question,
            words(&agent),
            "chat mode is not the agent as built"
        );
        assert_eq!(brief.len(), question.len());
        let moved: Vec<(&String, &String)> = brief
            .iter()
            .zip(&question)
            .filter(|(brief, question)| brief != question)
            .collect();
        assert_eq!(
            moved.len(),
            1,
            "a mode changed something other than how hard the turn thinks: {moved:?}",
        );
        assert_eq!(
            value_of(&brief, "--effort"),
            Some(warlock_tui::BRIEF_EFFORT)
        );
        assert_eq!(
            value_of(&brief, "--session-id"),
            value_of(&question, "--session-id"),
            "a mode change started a second conversation",
        );
        assert_eq!(
            value_of(&brief, "--system-prompt"),
            value_of(&question, "--system-prompt"),
        );
        assert_eq!(value_of(&brief, "--tools"), value_of(&question, "--tools"));
    }

    #[test]
    fn a_mode_change_is_a_state_of_the_conversation_and_never_a_second_one() {
        // The same property as the test above, said where a mode change really
        // happens: the composer's own `apply_compose`, over the value the loop
        // keeps, with the commands submitted as they are typed. What is asserted
        // is the agent the conversation still holds four turns later — the one
        // long-lived `ChatAgent` — because *that* is what a second session would
        // show up in. A `/brief` that rebuilt the conversation would name a new
        // session id here, and the turns already said would be turns nothing had
        // ever heard.
        //
        // The `claude` is one that does not exist, so the four turns these
        // commands really start spawn nothing at all: no terminal, no network
        // and no model. The repository is an empty temporary directory, which
        // is a repository that has written no brief template: `/brief` reads
        // for one, finds none, and states warlock's own shape.
        let base = Instant::now();
        let repo = tempfile::tempdir().expect("a temporary directory");
        let mut app = App::default();
        let mut chat = Chat::with_agent(
            repo.path(),
            ChatAgent::new().with_program("/warlock/no/such/program"),
        );
        let opening = words(&chat.agent);
        let session = value_of(&opening, "--session-id").expect("a turn opens a conversation");
        let prompt = value_of(&opening, "--system-prompt").expect("a turn carries the prompt");

        // Where a brief would be written is the conversation's own now, and
        // this test is about the conversation rather than about that setting:
        // `/brief` settling it has its own assertions in `mod submitting`.
        for draft in ["why nine passes?", "/brief", "/brief", "/chat"] {
            chat.compose(
                &mut app,
                warlock_tui::Composed::Typing(warlock_tui::Composer::new(draft)),
                base,
            );
            chat.compose(&mut app, warlock_tui::Composed::Submit, base);
        }

        // The register was really entered and really left — otherwise the
        // vectors below would be equal for the dullest of reasons.
        assert_eq!(app.mode(), Mode::Chat);
        assert_eq!(
            app.thread().map_or(0, |thread| thread.turns().len()),
            4,
            "the commands did not cost the four turns they are supposed to",
        );
        assert_eq!(
            words(&chat.agent),
            opening,
            "a mode change rebuilt the conversation it is a state of",
        );

        // And said one fact at a time, so a failure names which of them went.
        let brief = words(&super::asking(&chat.agent, Mode::Brief));
        let question = words(&super::asking(&chat.agent, Mode::Chat));
        for (mode, vector) in [("brief", &brief), ("chat", &question)] {
            assert_eq!(
                value_of(vector, "--session-id"),
                Some(session),
                "a {mode}-mode turn is asked in another session",
            );
            assert_eq!(
                value_of(vector, "--system-prompt"),
                Some(prompt),
                "a {mode}-mode turn is asked under another prompt",
            );
        }
        // That the one prompt is `CHAT_SYSTEM_PROMPT` and says what it has to
        // say in both registers is `claude.rs`'s own test, on the constant
        // itself; what is asserted here is that a mode change never swaps it.
    }

    #[test]
    fn a_frame_with_no_turn_in_flight_does_nothing_at_all() {
        let base = Instant::now();
        let mut app = App::default();
        let before = app.clone();

        drain(&mut None, &mut app, at(base, 1));

        assert_eq!(app, before, "a frame with nothing running moved something");
    }

    /// What a turn comes back as when a real child really runs it: one test per
    /// way a turn can fail, and one for a turn that works.
    ///
    /// Unix-only for `claude.rs`'s reason — `/bin/sh` is the stand-in, and a
    /// shell script is the cheapest possible model — and driven through
    /// [`run_turn`] on the test's own thread, so what is asserted is the
    /// sequence of events a real turn sends rather than the timing of one. The
    /// two that go through [`spawn_turn`] and [`start_turn`] are the two that
    /// are *about* the thread: stopping a turn from another thread, and
    /// dropping one.
    #[cfg(unix)]
    mod unix {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc::{self, RecvTimeoutError};
        use std::time::{Duration, Instant};
        use std::{env, fs, process, thread};

        use warlock_tui::{Activity, App, Cancel, ChatAgent, Ending, Mode, ScopePrompt};

        use super::super::{Asked, Chat, TurnEvent, run_turn, spawn_turn, start_turn, wired};
        use super::{ASKED, at, chatting, clocked, drain, rows, said};

        /// How long a test waits for a child to say it is running, or for a
        /// turn to come back, before giving up. Generous, because it is only
        /// reached when something is already wrong; every wait ends as soon as
        /// what it is waiting for happens.
        const AT_MOST: Duration = Duration::from_secs(5);

        /// A repository root the transport tests never read: what a turn does
        /// with a child process is the same wherever the conversation is rooted,
        /// and only `/brief` and `/write` ever look at the root at all.
        const NO_REPOSITORY: &str = "/warlock/no/such/repository";

        /// How often a test looks again while waiting for a worker thread to
        /// finish letting go of its channel.
        ///
        /// Short enough that the ordinary case — the thread returning a moment
        /// after its last send — costs one tick and not a visible pause, and it
        /// is a polling interval rather than a deadline: [`AT_MOST`] is what
        /// gives up. Nothing is timed against this, so it may be made longer or
        /// shorter without any test's meaning changing.
        const TEARDOWN_TICK: Duration = Duration::from_millis(10);

        /// A program that is not there, so a spawn fails the one way that has
        /// its own ending.
        const NOT_A_PROGRAM: &str = "/warlock/no/such/program";

        /// The lines of a turn, in miniature: the session's opening line, a
        /// tool call, a thought, the model starting to write, and the result
        /// line carrying the answer and what the turn cost.
        ///
        /// `claude.rs`'s `PASS` for the other kind of run, and deliberately the
        /// same shape — the transport is the same transport — with a chat's
        /// tools in the opening line and an answer where a document would be.
        const TURN: [&str; 5] = [
            r#"{"type":"system","subtype":"init","tools":["Read","Grep","Glob"]}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"a thought nobody is entitled to"}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"text"}}}"#,
            r#"{"type":"result","subtype":"success","result":"The tree, the manifest and the pact.","total_cost_usd":0.0123}"#,
        ];

        /// The answer [`TURN`]'s result line carries.
        const ANSWER: &str = "The tree, the manifest and the pact.";

        /// What [`TURN`] reports, in order: the tool with its one whitelisted
        /// argument, the bare fact of the thought, the model starting to write,
        /// and the cost. Not the thought's text and not a word of the answer —
        /// the answer arrives once, as the turn's ending, and never in pieces
        /// among the work.
        fn reported() -> Vec<Activity> {
            vec![
                Activity::Tool {
                    name: "Read".to_owned(),
                    detail: Some("src/lib.rs".to_owned()),
                },
                Activity::Thinking,
                Activity::Writing { bytes: 0 },
                Activity::Cost { usd: 0.0123 },
            ]
        }

        /// An agent whose `claude` is `sh -c script`, as `claude.rs`'s tests
        /// build one.
        fn stand_in(script: &str) -> ChatAgent {
            ChatAgent::new()
                .with_program("/bin/sh")
                .with_args(["-c", script])
        }

        /// A shell script that prints `lines`, one per line, and exits.
        fn printing(lines: &[&str]) -> String {
            let arguments: Vec<String> = lines.iter().map(|line| format!("'{line}'")).collect();
            format!("printf '%s\\n' {}", arguments.join(" "))
        }

        /// Everything one turn of `agent` sends, in order, driven on this
        /// thread.
        ///
        /// The worker's body over the worker's own wiring: the agent is given
        /// this turn's cancel and this turn's activity port by the very
        /// function [`spawn_turn`] uses, so what a test drives and what the
        /// loop starts are the same run without a thread between them.
        fn turned(agent: &ChatAgent, cancel: &Cancel) -> Vec<TurnEvent> {
            let (events, received) = mpsc::channel();
            run_turn(ASKED, &wired(agent, cancel, &events), cancel, &events);
            received.try_iter().collect()
        }

        /// The work `events` reported, in order.
        fn doings(events: &[TurnEvent]) -> Vec<Activity> {
            events
                .iter()
                .filter_map(|event| match event {
                    TurnEvent::Doing(activity) => Some(activity.clone()),
                    TurnEvent::Finished(_) => None,
                })
                .collect()
        }

        /// How the turn ended, having asserted that it said so exactly once and
        /// last.
        ///
        /// The promise every test in here is about, checked in one place: a
        /// turn ends once, whatever it was doing when it ended, and nothing is
        /// sent after the end.
        fn ended(events: &[TurnEvent]) -> Result<String, Ending> {
            let endings: Vec<_> = events
                .iter()
                .filter_map(|event| match event {
                    TurnEvent::Finished(finished) => Some(finished.clone()),
                    TurnEvent::Doing(_) => None,
                })
                .collect();
            assert_eq!(endings.len(), 1, "not exactly one ending: {events:?}");
            assert!(
                matches!(events.last(), Some(TurnEvent::Finished(_))),
                "something was said after the turn ended: {events:?}"
            );
            endings.into_iter().next().expect("just counted one")
        }

        /// How `events` ended, having asserted it did not answer.
        fn ending(events: &[TurnEvent]) -> Ending {
            ended(events).expect_err("this stand-in never answers")
        }

        /// A directory of this test's own, removed at the end of the test that
        /// made it — `claude.rs`'s helper, for the one test here that needs a
        /// child to write a file.
        fn scratch(name: &str) -> PathBuf {
            static NEXT: AtomicUsize = AtomicUsize::new(0);

            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let directory =
                env::temp_dir().join(format!("warlock-chat-{}-{name}-{unique}", process::id()));
            fs::create_dir_all(&directory).expect("a scratch directory under the temp directory");
            directory
        }

        /// Best effort: a leftover under `/tmp` is untidy, not a test failure.
        fn clean_up(directory: &Path) {
            let _ = fs::remove_dir_all(directory);
        }

        /// How big `path` is, or nothing if it is not there yet.
        fn size(path: &Path) -> Option<u64> {
            fs::metadata(path).ok().map(|file| file.len())
        }

        /// Go round the loop's bottom end until the turn has ended, or give up
        /// after [`AT_MOST`].
        ///
        /// [`Chat::keep_up`] and nothing else, which is exactly what `keep_up`
        /// in the event loop calls: nothing here waits on the worker, joins a
        /// thread or receives from a channel — the round polls and comes back,
        /// and the turn ending is the drain taking it down.
        ///
        /// What comes back is the window the drain opened, which is the write
        /// window for the one turn `/write` starts and a closed one for every
        /// other turn warlock ever runs. Nothing is handed back to the loop any
        /// more — the conversation opens its own window — so this reads the
        /// window off the conversation afterwards. [`settle`] is this with the
        /// closed window asserted, and is what the tests about an ordinary turn
        /// call.
        fn settled(chat: &mut Chat, app: &mut App, now: Instant) -> ScopePrompt {
            let waited = Instant::now();
            while chat.answering() && waited.elapsed() < AT_MOST {
                chat.keep_up(app, now);
                thread::sleep(Duration::from_millis(10));
            }
            assert!(!chat.answering(), "the turn never ended");
            chat.write_prompt().clone()
        }

        /// [`settled`] for a turn that is not the write request: the rounds go
        /// by and no window opens over anything.
        fn settle(chat: &mut Chat, app: &mut App, now: Instant) {
            assert_eq!(
                settled(chat, app, now),
                ScopePrompt::Closed,
                "an ordinary turn opened the write window"
            );
        }

        /// Wait until `path` exists, or give up after [`AT_MOST`].
        ///
        /// How a test waits for a child to be genuinely running rather than
        /// guessing at a sleep.
        fn wait_for(path: &Path) {
            let waited = Instant::now();
            while size(path).is_none_or(|bytes| bytes == 0) && waited.elapsed() < AT_MOST {
                thread::sleep(Duration::from_millis(10));
            }
        }

        #[test]
        fn a_turn_that_answers_says_what_it_did_and_then_answers_once() {
            let events = turned(&stand_in(&printing(&TURN)), &Cancel::new());

            // The work as it happened, then the answer: one channel, in the
            // order the turn produced them.
            assert_eq!(doings(&events), reported());
            assert_eq!(ended(&events), Ok(ANSWER.to_owned()));
            // Said once more, because it is the promise the port is for: the
            // thought is not in there, and neither is the answer.
            let seen = format!("{:?}", doings(&events));
            assert!(!seen.contains("entitled"), "{seen}");
            assert!(!seen.contains("manifest"), "{seen}");
        }

        #[test]
        fn no_claude_to_ask_is_one_line_and_not_an_error() {
            let events = turned(
                &stand_in("true").with_program(NOT_A_PROGRAM),
                &Cancel::new(),
            );

            assert_eq!(
                ending(&events),
                Ending::NoModel {
                    program: NOT_A_PROGRAM.to_owned()
                }
            );
        }

        #[test]
        fn a_non_zero_exit_is_one_line_carrying_its_status_and_its_stderr() {
            let events = turned(&stand_in("echo boom >&2; exit 3"), &Cancel::new());

            let ended = ending(&events);
            match &ended {
                Ending::Failed { code, stderr } => {
                    assert_eq!(*code, Some(3));
                    assert_eq!(stderr.trim(), "boom", "stderr is carried, not dropped");
                }
                other => panic!("expected a non-zero exit, got {other:?}"),
            }
            // One row, however many lines the child wrote.
            assert!(ended.line().contains("exit status 3"), "{}", ended.line());
        }

        #[test]
        fn a_turn_that_says_nothing_ends_with_nothing_said() {
            for script in ["exit 0", "printf '\\n  \\n'"] {
                let events = turned(&stand_in(script), &Cancel::new());

                assert_eq!(ending(&events), Ending::NothingSaid, "`{script}`");
            }
        }

        #[test]
        fn a_turn_that_runs_too_long_is_stopped_and_says_so() {
            let agent = stand_in("sleep 30").with_timeout(Duration::from_millis(250));

            let started = Instant::now();
            let events = turned(&agent, &Cancel::new());
            let elapsed = started.elapsed();

            assert_eq!(
                ending(&events),
                Ending::TimedOut {
                    after: Duration::from_millis(250)
                }
            );
            assert!(
                elapsed < Duration::from_secs(10),
                "the turn waited {elapsed:?}, far past its timeout"
            );
        }

        #[test]
        fn a_turn_cancelled_before_it_starts_ends_as_cancelled() {
            let cancel = Cancel::new();
            cancel.cancel();

            let events = turned(&stand_in(&printing(&TURN)), &cancel);

            // The seam has no word for a cancel — a killed child is interrupted
            // I/O and the errno says nothing about who killed it — so this is
            // the fact the worker holds, said in the thread's own words rather
            // than as `the turn could not run — …`.
            assert_eq!(ending(&events), Ending::Cancelled);
        }

        #[test]
        fn a_turn_cancelled_from_another_thread_ends_once_and_promptly() {
            let cancel = super::super::CancelGuard::new();
            // The real five-minute timeout: the only thing that can end this
            // turn in time is the cancel.
            let received = spawn_turn(ASKED, &stand_in("sleep 300"), cancel.handle());

            let started = Instant::now();
            cancel.cancel();
            let first = received
                .recv_timeout(AT_MOST)
                .expect("a cancelled turn still says how it ended");
            let elapsed = started.elapsed();

            assert!(
                matches!(&first, TurnEvent::Finished(Err(Ending::Cancelled))),
                "{first:?}"
            );
            assert!(elapsed < AT_MOST, "the cancel took {elapsed:?}");

            // And nothing after it: the worker sent its one ending and stopped.
            // Two facts, and they fail for different reasons, so they are waited
            // on rather than sampled once. `run_turn` sends the ending as its
            // last statement and the channel goes quiet only when the closure
            // returns and drops the two senders — the worker's and the one
            // `wired` gave the agent — so the gap between the ending arriving
            // here and the disconnect arriving is a thread being scheduled. On a
            // loaded machine that is not bounded by any number worth writing
            // down, and a fixed window here read a slow teardown as a talkative
            // worker: `Timeout` says precisely that nothing was said.
            //
            // So an event is the failure, immediately and in its own words, and
            // silence is waited out to [`AT_MOST`] — the budget every other wait
            // in this module keeps.
            let quiet = Instant::now();
            loop {
                match received.recv_timeout(TEARDOWN_TICK) {
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => assert!(
                        quiet.elapsed() < AT_MOST,
                        "the worker held the channel open for {:?} after it ended",
                        quiet.elapsed()
                    ),
                    Ok(event) => panic!("the worker went on talking after it ended: {event:?}"),
                }
            }
        }

        #[test]
        fn dropping_a_turn_kills_the_claude_it_was_waiting_on() {
            let directory = scratch("dropped");
            let ticks = directory.join("ticks");
            // Never exits on its own, and says so in a file: whether it is
            // still running after the drop is a question this test can ask.
            let script = format!(
                "while :; do echo tick >> '{}'; sleep 0.05; done",
                ticks.display()
            );
            let chatting = start_turn(ASKED, &stand_in(&script), Asked::Answer);

            wait_for(&ticks);
            drop(chatting);

            // The guard is the only thing that can have stopped it: nothing
            // above joined a thread, cancelled a handle or waited for a child.
            thread::sleep(Duration::from_millis(300));
            let before = size(&ticks).expect("the child ticked at least once");
            thread::sleep(Duration::from_millis(300));
            assert_eq!(
                Some(before),
                size(&ticks),
                "the child outlived the turn nobody is listening to any more"
            );
            clean_up(&directory);
        }

        #[test]
        fn a_whole_turn_reaches_the_card_the_way_the_loop_drives_it() {
            let base = Instant::now();
            let mut app = App::default();
            app.start_turn(ASKED, base);
            let mut chat = Some(chatting(spawn_turn(
                ASKED,
                &stand_in(&printing(&TURN)),
                Cancel::new(),
            )));

            // The loop's own round, without the terminal: drain, and go round
            // again. Nothing here waits on the worker — `apply_turn` returns
            // whether or not anything has arrived — which is why this is a
            // poll rather than a receive.
            let waited = Instant::now();
            while chat.is_some() && waited.elapsed() < AT_MOST {
                drain(&mut chat, &mut app, at(base, 2));
                thread::sleep(Duration::from_millis(10));
            }

            assert!(chat.is_none(), "the turn never ended");
            assert_eq!(
                rows(&app, at(base, 2)),
                vec![
                    said(),
                    super::clocked(2, "Read src/lib.rs"),
                    super::clocked(2, "thinking"),
                    super::clocked(2, "writing"),
                    warlock_tui::Line::Text {
                        text: ANSWER.to_owned()
                    },
                ]
            );
            // A turn that worked says nothing on the footer.
            assert_eq!(app.message(), None);
        }

        #[test]
        fn stopping_a_turn_ends_it_as_cancelled_and_hands_the_field_back() {
            // Ctrl-C with a question out, end to end and through the value the
            // loop holds: `Chat::stop` is what that key comes to, and it kills
            // the `claude` the turn is waiting on rather than leaving warlock.
            // The stand-in never returns on its own and the agent's timeout is
            // the real five minutes, so the cancel is the only thing that can
            // end this within the test's patience.
            let base = Instant::now();
            let mut app = App::default();
            let mut chat = Chat::with_agent(NO_REPOSITORY, stand_in("sleep 300"));

            chat.ask(&mut app, ASKED, base);
            chat.stop();
            // Still in flight at the keystroke: the worker has one thing left to
            // say, and the drain is where it says it.
            assert!(chat.answering(), "the turn was taken down at the keystroke");
            settle(&mut chat, &mut app, at(base, 3));

            assert_eq!(
                rows(&app, at(base, 30)),
                vec![said(), clocked(3, &Ending::Cancelled.line())]
            );
            assert_eq!(app.message(), Some(Ending::Cancelled.line().as_str()));
            // And the field is live again, on the strength of the drain alone.
            assert!(!chat.answering());
        }

        #[test]
        fn a_synthesized_turn_shows_the_command_and_sends_the_instruction() {
            // The display/send split, end to end and through the value the loop
            // holds: what the reader sees is the word they typed and what the
            // child reads on its stdin is the paragraph warlock wrote. The
            // stand-in copies its stdin to a file before answering, so what was
            // really sent is a thing this test can read rather than infer.
            let directory = scratch("synthesized");
            let sent = directory.join("sent");
            let script = format!("cat > '{}'; {}", sent.display(), printing(&TURN));
            let base = Instant::now();
            let mut app = App::default();
            let mut chat = Chat::with_agent(NO_REPOSITORY, stand_in(&script));
            // Composed here, as the loop composes it: what is sent is a string
            // built around a shape, and this test only cares that whatever was
            // built is what the child read.
            let instruction = warlock_tui::brief_instruction(warlock_tui::DEFAULT_TEMPLATE);

            chat.say(&mut app, "/brief", &instruction, Asked::Answer, base);
            settle(&mut chat, &mut app, at(base, 1));

            // The command, its work lines and its answer: an ordinary turn in
            // every respect but the one word it is shown as.
            assert_eq!(
                rows(&app, at(base, 1)),
                vec![
                    warlock_tui::Line::Said {
                        text: "/brief".to_owned()
                    },
                    clocked(1, "Read src/lib.rs"),
                    clocked(1, "thinking"),
                    clocked(1, "writing"),
                    warlock_tui::Line::Text {
                        text: ANSWER.to_owned()
                    },
                ]
            );
            assert_eq!(
                fs::read_to_string(&sent).expect("the child read something"),
                instruction,
                "the model was not given the instruction",
            );
            clean_up(&directory);
        }

        #[test]
        fn a_failed_turn_leaves_the_conversation_usable_for_the_next_question() {
            // The promise a failure has to keep, driven through the very value
            // the event loop holds: a turn that went wrong is one line on the
            // thread and one on the footer, the turn is taken down — which is
            // what unmutes the field — and the question after it runs as if
            // nothing had happened. One `Chat`, as the loop has one, so the
            // second question is genuinely the next turn of the conversation the
            // first one failed in.
            const AGAIN: &str = "and which of those is the biggest?";

            let directory = scratch("usable");
            let asked_once = directory.join("asked-once");
            // Fails the first time it is asked and answers the second.
            let script = format!(
                "if [ -f '{marker}' ]; then {answer}; else : > '{marker}'; echo boom >&2; exit 3; fi",
                marker = asked_once.display(),
                answer = printing(&TURN),
            );
            let base = Instant::now();
            let mut app = App::default();
            let mut chat = Chat::with_agent(NO_REPOSITORY, stand_in(&script));

            chat.ask(&mut app, ASKED, base);
            assert!(
                chat.answering(),
                "the field is muted for as long as a question is out"
            );
            settle(&mut chat, &mut app, at(base, 1));

            // One line either side, in the same words, and no error anywhere:
            // `keep_up` returns nothing at all, so there is nothing for the loop
            // to have propagated.
            let line = app
                .message()
                .expect("a failed turn says so on the footer")
                .to_owned();
            assert!(line.contains("exit status 3"), "{line}");
            assert_eq!(rows(&app, at(base, 30)), vec![said(), clocked(1, &line)]);

            chat.ask(&mut app, AGAIN, at(base, 10));
            assert!(chat.answering(), "the next question started");
            settle(&mut chat, &mut app, at(base, 12));

            // The failed turn is still on the card above the answered one, and
            // the answered one is whole: the work as it arrived, and the
            // answer.
            assert_eq!(
                rows(&app, at(base, 12)),
                vec![
                    said(),
                    clocked(1, &line),
                    warlock_tui::Line::Said {
                        text: AGAIN.to_owned()
                    },
                    clocked(2, "Read src/lib.rs"),
                    clocked(2, "thinking"),
                    clocked(2, "writing"),
                    warlock_tui::Line::Text {
                        text: ANSWER.to_owned()
                    },
                ]
            );
            clean_up(&directory);
        }

        #[test]
        fn a_write_turn_that_answers_hands_the_document_to_the_loop() {
            // The whole of what `/write` adds to a turn, through the value the
            // loop holds and a `/bin/sh` standing in for the model: the answer
            // comes back to the caller — which is what the path prompt opens
            // over — and the same answer is on the card, where the reader reads
            // it and where the write takes its bytes from.
            let directory = scratch("write-answers");
            let base = Instant::now();
            let mut app = App::default();
            app.set_mode(Mode::Brief);
            let mut chat = Chat::with_agent(NO_REPOSITORY, stand_in(&printing(&TURN)));

            chat.say(
                &mut app,
                "/write",
                warlock_tui::WRITE_INSTRUCTION,
                Asked::Document,
                base,
            );
            let opened = settled(&mut chat, &mut app, at(base, 1));

            // The answer is handed to the window rather than to the loop: the
            // path on it is the one proposed from this very reply, so asserting
            // the field is asserting the document that came back.
            let field = opened.field().expect("the write window opened over it");
            assert_eq!(
                field.text(),
                crate::writing::proposed_path(
                    Path::new(NO_REPOSITORY),
                    warlock_engine::DEFAULT_BRIEF_DIRECTORY,
                    ANSWER
                )
            );
            assert_eq!(
                rows(&app, at(base, 1)),
                vec![
                    warlock_tui::Line::Said {
                        text: "/write".to_owned()
                    },
                    clocked(1, "Read src/lib.rs"),
                    clocked(1, "thinking"),
                    clocked(1, "writing"),
                    warlock_tui::Line::Text {
                        text: ANSWER.to_owned()
                    },
                ],
                "the document left the card it was answered on"
            );
            assert_eq!(app.mode(), Mode::Brief, "the register moved for a write");
            clean_up(&directory);
        }

        #[test]
        fn a_write_turn_that_fails_hands_nothing_back_and_leaves_the_mode_alone() {
            // The failure half of the same thing, and the promise it has to
            // keep: nothing comes back, so the loop has nothing to open a
            // prompt over and nothing to write; the ending is one line in the
            // existing wording; the conversation is still in brief mode and
            // still answers the next question.
            const AGAIN: &str = "what did that leave out?";

            let directory = scratch("write-fails");
            let asked_once = directory.join("asked-once");
            // Fails the first time it is asked and answers the second, exactly
            // as the ordinary failed turn above does.
            let script = format!(
                "if [ -f '{marker}' ]; then {answer}; else : > '{marker}'; echo boom >&2; exit 3; fi",
                marker = asked_once.display(),
                answer = printing(&TURN),
            );
            let base = Instant::now();
            let mut app = App::default();
            app.set_mode(Mode::Brief);
            let mut chat = Chat::with_agent(NO_REPOSITORY, stand_in(&script));

            chat.say(
                &mut app,
                "/write",
                warlock_tui::WRITE_INSTRUCTION,
                Asked::Document,
                base,
            );
            assert_eq!(
                settled(&mut chat, &mut app, at(base, 1)),
                ScopePrompt::Closed,
                "a failed write opened the window anyway"
            );

            let line = app
                .message()
                .expect("a failed turn says so on the footer")
                .to_owned();
            assert!(line.contains("exit status 3"), "{line}");
            assert_eq!(
                rows(&app, at(base, 30)),
                vec![
                    warlock_tui::Line::Said {
                        text: "/write".to_owned()
                    },
                    clocked(1, &line),
                ]
            );
            assert_eq!(app.mode(), Mode::Brief, "a failed write left the register");

            // And the conversation goes on: the next question is asked into the
            // very `Chat` the write failed in.
            chat.ask(&mut app, AGAIN, at(base, 10));
            settle(&mut chat, &mut app, at(base, 12));

            assert_eq!(
                rows(&app, at(base, 12)).last(),
                Some(&warlock_tui::Line::Text {
                    text: ANSWER.to_owned()
                })
            );
            clean_up(&directory);
        }
    }

    /// What a submitted draft comes to, through the function the loop calls.
    ///
    /// [`submitted_for`]'s own tests say what each draft *is*; these say what
    /// the loop then does with it, which is the thing that costs a turn when it
    /// is wrong. Nothing here has a terminal, a network or a `claude`: the
    /// command and refusal drafts never reach [`Chat::ask`] at all, and the one
    /// test that does submit a message hands the conversation an agent whose
    /// program does not exist, so the worker it starts finds nothing to run.
    ///
    /// The repository these submit into is a temporary directory of the test's
    /// own — `/brief` reads `.warlock/brief-template.md` and
    /// `.warlock/briefs.toml` under it at the keystroke — and one that has
    /// nothing in it is a repository that has written neither, which is the
    /// ordinary case.
    mod submitting {
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::thread;
        use std::time::{Duration, Instant};

        use warlock_engine::{DEFAULT_BRIEF_DIRECTORY, briefs_path, load_briefs};
        use warlock_tui::{
            Activity, App, ChatAgent, Composed, Composer, DEFAULT_TEMPLATE, Ending, Line, Mode,
            Submitted, brief_instruction,
        };

        use super::super::{
            ALREADY_CHATTING, BRIEF_COMMAND, BRIEF_NOTE, CHAT_COMMAND, CHAT_NOTE, Chat,
            NOT_BRIEFING, WRITE_COMMAND, brief_asking,
        };
        use crate::error::one_line;
        use crate::writing::write_opened;

        /// A `claude` that is not there, so a turn that does start spawns
        /// nothing. `pacting.rs` and `chatting.rs` build their failures the
        /// same way.
        const NOT_A_PROGRAM: &str = "/warlock/no/such/program";

        /// A repository that does not exist, and therefore has no template
        /// under it.
        ///
        /// The root for every test here that is not *about* the template: an
        /// absent file is an absent file whether the directory over it is there
        /// or not, so these submit against warlock's own shape without a
        /// temporary directory each. Nothing reads or writes it.
        const NO_REPOSITORY: &str = "/warlock/no/such/repository";

        /// A conversation with nothing asked and nothing runnable to ask,
        /// rooted where nothing is.
        fn conversation() -> Chat {
            conversation_in(Path::new(NO_REPOSITORY))
        }

        /// The same, over a stated repository root, for the tests that care what
        /// is under it.
        ///
        /// The root is settled when the conversation is built and never moves
        /// afterwards, which is why it is here and no longer an argument to
        /// every submit below.
        fn conversation_in(root: &Path) -> Chat {
            Chat::with_agent(root, ChatAgent::new().with_program(NOT_A_PROGRAM))
        }

        /// A throwaway repository root, for the tests that put a template in
        /// one.
        fn a_root() -> tempfile::TempDir {
            tempfile::tempdir().expect("a temporary directory")
        }

        /// Put `text` at `<root>/.warlock/brief-template.md`, and hand back
        /// where it went.
        fn write_template(root: &Path, text: &str) -> PathBuf {
            let directory = root.join(".warlock");
            fs::create_dir_all(&directory).expect("a `.warlock` directory");
            let path = directory.join("brief-template.md");
            fs::write(&path, text).expect("a template file");

            path
        }

        /// Put `text` at `<root>/.warlock/briefs.toml` the way a person with an
        /// editor would — the only way that file is ever written — and hand back
        /// where it went.
        fn write_briefs(root: &Path, text: &str) -> PathBuf {
            let path = briefs_path(root);
            fs::create_dir_all(path.parent().expect("the file has a directory"))
                .expect("a `.warlock` directory");
            fs::write(&path, text).expect("a briefs config");

            path
        }

        /// Submit `draft` from a fresh field, and hand back what the app and
        /// the conversation look like afterwards.
        ///
        /// The field comes back inside the conversation now rather than beside
        /// it: `chat.composer()` is the same field this used to return.
        fn submit(draft: &str, now: Instant) -> (App, Chat) {
            let mut app = App::default();
            let mut chat = conversation();
            submit_into(&mut app, &mut chat, draft, now);

            (app, chat)
        }

        /// Submit `draft` into a conversation that is already going.
        ///
        /// What [`submit`] does for one draft, over an app and a chat the caller
        /// keeps: a mode is a state of *this* conversation, so the tests about
        /// it are two and three commands long and every one of them has to be
        /// the same session and the same card.
        ///
        /// Two keystrokes through the real door and no test-only way in: the
        /// draft is typed by handing the conversation the field a typed key
        /// would have produced, and then submitted. Where a brief goes and what
        /// repository it is read from are the conversation's own now, so neither
        /// is a parameter here — which is the whole of why the three rungs this
        /// helper used to sit on top of are gone.
        fn submit_into(app: &mut App, chat: &mut Chat, draft: &str, now: Instant) {
            chat.compose(app, Composed::Typing(Composer::new(draft)), now);
            chat.compose(app, Composed::Submit, now);
        }

        /// The card as one unclocked note of warlock's own.
        fn note(text: &str) -> Line {
            Line::Note {
                text: text.to_owned(),
            }
        }

        /// A turn on the card as it is drawn the instant it is asked: the
        /// message it is shown as, and the clocked row a turn with nothing back
        /// yet draws under it.
        fn asked(shown: &str) -> [Line; 2] {
            [
                Line::Said {
                    text: shown.to_owned(),
                },
                Line::Clocked {
                    clock: "0:00".to_owned(),
                    text: "waiting".to_owned(),
                },
            ]
        }

        /// Every row of the app's thread card, or none at all when nothing has
        /// put a card there.
        fn rows(app: &App, now: Instant) -> Vec<Line> {
            app.thread()
                .map(|thread| thread.lines(now))
                .unwrap_or_default()
        }

        /// How many turns the thread holds, card or no card.
        fn turns(app: &App) -> usize {
            app.thread().map_or(0, |thread| thread.turns().len())
        }

        /// The path a `/write` would pre-fill for a mode pointed at
        /// `directory`, through the very function the loop opens that window
        /// with.
        ///
        /// What the setting actually comes to, rather than the string it was
        /// read as: `writing.rs` owns the numbering and the slug, and this asks
        /// it where the document would land. Spelled relative to the repository
        /// root, as that window spells it.
        fn proposal(root: &Path, directory: &str) -> String {
            let prompt = write_opened(root, directory, "# A brief\n\nsomething was said.");

            prompt
                .field()
                .expect("the write window opened")
                .text()
                .to_owned()
        }

        /// The whole of the muting rule, against the conversation that keeps
        /// it: one question at a time, and the field types whenever there is
        /// not one out.
        ///
        /// This used to be two assertions about a free function taking a
        /// `bool` — which proved that `field_muted(true)` is `true` and left
        /// the interesting half, whether anything ever *calls* it with the
        /// right value, to the event loop nothing could test. The field and the
        /// turn are one value now, so the rule is assertable where it lives.
        ///
        /// A turn ends in five ways — it answers, it is cancelled, there is no
        /// `claude`, it exits non-zero, it times out — and the field comes back
        /// on all five without any of them saying so, because what mutes it is
        /// the turn being out and nothing else. The one below is the third of
        /// the five, which is the one a test can have in a millisecond.
        ///
        /// A pact is deliberately not a reason, and `pacting.rs` asserts that
        /// end of it against a run that is really going.
        #[test]
        fn the_field_is_muted_by_a_turn_and_comes_back_however_it_ends() {
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            assert!(
                !chat.composer().is_muted(),
                "an idle conversation cannot type"
            );

            submit_into(&mut app, &mut chat, "what is a pact?", now);
            assert!(chat.answering(), "the message started no turn");
            assert!(
                chat.composer().is_muted(),
                "a question is out and the field still types"
            );

            // The rounds until the drain takes the turn down. There is no
            // `claude` at that path, so the worker comes back with the ending
            // for it — one of the five, and nothing about that ending says a
            // word about the keyboard.
            let waited = Instant::now();
            while chat.answering() && waited.elapsed() < Duration::from_secs(5) {
                chat.keep_up(&mut app, now);
                thread::sleep(Duration::from_millis(10));
            }
            assert!(!chat.answering(), "the turn never ended");

            assert!(
                !chat.composer().is_muted(),
                "the turn ended and the field is still deaf"
            );
        }

        #[test]
        fn write_outside_brief_mode_is_one_note_and_costs_no_turn() {
            // Nothing is being converged on, so there is nothing to ask for: the
            // command says which register it wants and how to get there, and
            // spends neither a turn nor a `claude`. The line is decided from
            // `App::mode`, which is the state the border title is drawn from, so
            // it cannot say one register while the header says the other.
            let now = Instant::now();

            for draft in ["/write", "  /write  "] {
                let (app, chat) = submit(draft, now);

                assert_eq!(app.mode(), Mode::Chat, "{draft:?} moved the register");
                assert_eq!(
                    rows(&app, now),
                    vec![note(NOT_BRIEFING)],
                    "{draft:?} did not leave exactly one note"
                );
                assert_eq!(turns(&app), 0, "{draft:?} opened a turn");
                assert!(!chat.answering(), "{draft:?} started something");
                assert!(
                    chat.composer().draft().is_empty(),
                    "{draft:?} was left in the field"
                );
            }

            // And after a mode that was entered and left again: the refusal is
            // about the register the conversation is in now, not about whether
            // it was ever in the other one.
            let mut app = App::default();
            let mut chat = conversation();
            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "/chat", now);
            let before = turns(&app);
            submit_into(&mut app, &mut chat, "/write", now);

            assert_eq!(turns(&app), before, "/write out of the mode cost a turn");
            assert_eq!(rows(&app, now).last(), Some(&note(NOT_BRIEFING)));
        }

        #[test]
        fn write_in_brief_mode_sends_one_turn_shown_as_the_command() {
            // The ask for the artifact, and it is an ordinary turn in every
            // respect but the one word it is shown as: the card carries `/write`
            // and never the paragraph that went to the model, and no note is
            // added because no register changed. `chatting.rs` asserts the
            // instruction really is what reaches the child's stdin.
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "  /write  ", now);

            assert_eq!(app.mode(), Mode::Brief, "/write moved the register");
            assert_eq!(
                rows(&app, now),
                [
                    vec![note(BRIEF_NOTE)],
                    asked(BRIEF_COMMAND).to_vec(),
                    asked(WRITE_COMMAND).to_vec(),
                ]
                .concat(),
                "/write is not one turn shown as the command",
            );
            assert_eq!(turns(&app), 2, "/write did not open one turn");
            assert!(chat.answering(), "/write asked the model nothing");
            assert!(
                chat.composer().draft().is_empty(),
                "/write was left in the field"
            );

            // And again, because asking twice is asking twice: a second document
            // costs a second turn and still says nothing about the mode.
            submit_into(&mut app, &mut chat, "/write", now);

            assert_eq!(turns(&app), 3, "the second /write cost no turn");
            assert_eq!(
                rows(&app, now).len(),
                1 + 2 + 2 + 2,
                "the second /write said something about the register",
            );
        }

        #[test]
        fn brief_notes_the_mode_once_and_sends_one_turn_shown_as_the_command() {
            // What `/brief` costs: one unclocked note where it was typed, and
            // one ordinary turn under it. The card shows the word that was
            // typed and never the paragraph that went to the model — a screen of
            // prose the reader did not write, in the place their own questions
            // go, would be warlock putting words in their mouth.
            let now = Instant::now();

            for draft in ["/brief", "  /brief  "] {
                let (app, chat) = submit(draft, now);

                assert_eq!(app.mode(), Mode::Brief, "{draft:?} did not enter the mode");
                assert_eq!(
                    rows(&app, now),
                    [vec![note(BRIEF_NOTE)], asked(BRIEF_COMMAND).to_vec()].concat(),
                    "{draft:?} is not one note and one turn"
                );
                assert_eq!(turns(&app), 1, "{draft:?} did not open one turn");
                assert!(chat.answering(), "{draft:?} asked the model nothing");
                assert!(
                    chat.composer().draft().is_empty(),
                    "{draft:?} was left in the field"
                );
            }
        }

        #[test]
        fn a_brief_in_a_repository_with_no_template_states_the_built_in_shape() {
            // The ordinary case, and the one nobody configures anything for: a
            // repository that has written no `.warlock/brief-template.md` is a
            // repository that has said nothing about the shape, so the command
            // behaves exactly as it did before there was a file to write —
            // mode, note, one turn — and what it sends is warlock's own
            // skeleton.
            let now = Instant::now();
            let repo = a_root();
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.mode(), Mode::Brief, "the mode was not entered");
            assert_eq!(
                rows(&app, now),
                [vec![note(BRIEF_NOTE)], asked(BRIEF_COMMAND).to_vec()].concat(),
            );
            assert_eq!(turns(&app), 1, "the brief did not open one turn");
            assert!(chat.answering(), "the brief asked the model nothing");
            assert!(chat.composer().draft().is_empty());
            // And the instruction it sent, composed through the very function
            // the arm above composes it with: the built-in shape, because there
            // was nothing else to state.
            assert_eq!(
                brief_asking(repo.path()).expect("a repository with no template"),
                brief_instruction(DEFAULT_TEMPLATE),
            );
        }

        #[test]
        fn a_brief_carries_the_shape_the_repository_wrote_rather_than_the_built_in_one() {
            // A repository that has stated its own shape gets its own: the file
            // is read at this keystroke, and the built-in skeleton is nowhere in
            // what goes out. The card and the register are unchanged by any of
            // that — a template is what the instruction says, not what the
            // command does.
            const SHAPE: &str = "## The only heading we want\n\nOne section, and no others.";

            let now = Instant::now();
            let repo = a_root();
            write_template(repo.path(), SHAPE);
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.mode(), Mode::Brief, "the mode was not entered");
            assert_eq!(turns(&app), 1, "the brief did not open one turn");
            assert!(chat.answering());

            let instruction = brief_asking(repo.path()).expect("a template that reads");
            assert!(
                instruction.contains(SHAPE),
                "the template is not in the instruction: {instruction}"
            );
            assert!(
                !instruction.contains("## Success criteria"),
                "the built-in shape was sent as well: {instruction}"
            );
        }

        #[test]
        fn a_template_that_cannot_be_read_refuses_the_brief_and_changes_nothing_else() {
            // Refused rather than degraded: a file somebody wrote is a shape
            // somebody meant, and quietly aiming twenty turns at warlock's own
            // instead would be the wrong document arrived at slowly. So the
            // command is one line naming the file and what the filesystem said
            // about it, and the session is otherwise exactly as it was — no
            // mode, no turn, no `claude`, nothing on the footer.
            //
            // Bytes that are not UTF-8 are the portable way to have a file that
            // exists and cannot be had; `template.rs` fails the same way for the
            // same reason.
            let now = Instant::now();
            let repo = a_root();
            let path = write_template(repo.path(), "");
            fs::write(&path, [0x23, 0x20, 0xff, 0xfe, 0x0a]).expect("a template file");
            let reason = warlock_tui::brief_template(repo.path())
                .expect_err("a template that cannot be read")
                .to_string();
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.mode(), Mode::Chat, "a refusal entered the mode");
            assert_eq!(turns(&app), 0, "a refusal spent a turn");
            assert!(!chat.answering(), "a refusal asked the model something");
            assert!(chat.composer().draft().is_empty());
            assert_eq!(
                app.message(),
                None,
                "a refusal said something on the footer"
            );

            let said = rows(&app, now);
            assert_eq!(said.len(), 1, "a refusal is one line: {said:?}");
            let Some(Line::Note { text }) = said.first() else {
                panic!("a refusal is a note of warlock's own: {said:?}");
            };
            assert!(
                text.contains(&reason),
                "the loader's own words are not in it: {text}"
            );
            assert!(
                text.contains(&path.display().to_string()),
                "the file is not named: {text}"
            );
            assert!(!text.contains('\n'), "the refusal wrapped: {text}");
            assert!(
                !text.contains("## Success criteria"),
                "the built-in shape leaked into the refusal: {text}"
            );

            // And the next `/brief`, once the file reads again, is an ordinary
            // one: the refusal left nothing behind to recover from.
            fs::write(&path, "## Ours\n\nsay the thing.").expect("a template file");
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.mode(), Mode::Brief, "the mode was still not entered");
            assert_eq!(turns(&app), 1, "the second brief opened no turn");
        }

        #[test]
        fn a_repository_that_says_nothing_briefs_into_the_default_directory() {
            // Where a `/write` in this mode would put the document is a value
            // this command settles and the loop then holds: `/write` reads
            // nothing, so a brief that has taken twenty turns cannot arrive at a
            // window that will not open. A repository with no
            // `.warlock/briefs.toml` has stated no preference, which is the
            // engine's default and nothing said on the card — and it is written
            // here rather than left wherever a previous mode pointed, which is
            // `a_second_brief_re_reads_the_file_and_takes_the_new_directory`'s
            // half of it.
            let now = Instant::now();
            let repo = a_root();
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.mode(), Mode::Brief, "the mode was not entered");
            assert_eq!(turns(&app), 1, "the brief did not open one turn");
            assert_eq!(rows(&app, now).len(), 1 + 2, "something was said about it");
            assert_eq!(chat.directory(), DEFAULT_BRIEF_DIRECTORY);
            // And that is a proposal under `docs/`, through the very function
            // the loop opens the write window with.
            let proposed = proposal(repo.path(), chat.directory());
            assert!(
                proposed.starts_with("docs/"),
                "the default is not where a write would go: {proposed}",
            );
        }

        #[test]
        fn the_repositorys_own_directory_is_what_brief_settles_on() {
            // One key in one hand-written file, read at this keystroke: a
            // repository that keeps its briefs in `plans/` gets `plans/`, and
            // nothing about the command changes — the mode is entered, the note
            // is the note, and the turn is the turn.
            let now = Instant::now();
            let repo = a_root();
            write_briefs(repo.path(), "directory = \"plans\"\n");
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.mode(), Mode::Brief, "the mode was not entered");
            assert_eq!(
                rows(&app, now),
                [vec![note(BRIEF_NOTE)], asked(BRIEF_COMMAND).to_vec()].concat(),
                "a setting that reads said something on the card",
            );
            assert_eq!(chat.directory(), "plans");
            let proposed = proposal(repo.path(), chat.directory());
            assert!(
                proposed.starts_with("plans/"),
                "the setting is not where a write would go: {proposed}",
            );
        }

        #[test]
        fn a_briefs_config_that_cannot_be_read_refuses_the_brief_and_changes_nothing_else() {
            // The template's refusal, over the other file and for the same
            // reason: a `directory` somebody wrote down is a place somebody
            // meant, so warlock will not quietly aim twenty turns at `docs/`
            // instead. One line naming the file and quoting the parser, and the
            // session otherwise exactly as it was — no mode, no turn, no
            // `claude`, nothing on the footer, and the directory the loop was
            // carrying untouched.
            let now = Instant::now();
            let repo = a_root();
            let path = write_briefs(repo.path(), "directory = [\n");
            // The loader's own sentence, flattened the way the note flattens it:
            // a `briefs.toml` that will not parse carries the TOML parser's
            // multi-line diagnostic, and the card is one line a note.
            let reason = one_line(
                &load_briefs(repo.path())
                    .expect_err("a config that is not TOML")
                    .to_string(),
            );
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.mode(), Mode::Chat, "a refusal entered the mode");
            assert_eq!(turns(&app), 0, "a refusal spent a turn");
            assert!(!chat.answering(), "a refusal asked the model something");
            assert!(chat.composer().draft().is_empty());
            assert_eq!(
                app.message(),
                None,
                "a refusal said something on the footer"
            );

            let said = rows(&app, now);
            assert_eq!(said.len(), 1, "a refusal is one line: {said:?}");
            let Some(Line::Note { text }) = said.first() else {
                panic!("a refusal is a note of warlock's own: {said:?}");
            };
            assert!(
                text.contains(&path.display().to_string()),
                "the file is not named: {text}"
            );
            assert!(
                text.contains(&reason),
                "the parser's own words are not in it: {text}"
            );
            assert!(!text.contains('\n'), "the refusal wrapped: {text}");

            // And the next `/brief`, once the file reads again, is an ordinary
            // one: the refusal left nothing behind to recover from, and the
            // setting that now parses is the setting the mode is entered with.
            fs::write(&path, "directory = \"plans\"\n").expect("a briefs config");
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.mode(), Mode::Brief, "the mode was still not entered");
            assert_eq!(turns(&app), 1, "the second brief opened no turn");
            assert_eq!(chat.directory(), "plans");
        }

        #[test]
        fn two_broken_files_are_still_one_line_and_no_turn() {
            // The decision when both files under `.warlock/` are broken: the
            // load stops at the first, so the reader gets one refusal rather
            // than two, and the second file's line is what the next `/brief`
            // says once this one is fixed. Two notes for one keystroke would be
            // warlock reporting its own reading order.
            let now = Instant::now();
            let repo = a_root();
            let template = write_template(repo.path(), "");
            fs::write(&template, [0x23, 0x20, 0xff, 0xfe, 0x0a]).expect("a template file");
            write_briefs(repo.path(), "directroy = \"plans\"\n");
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.mode(), Mode::Chat, "a refusal entered the mode");
            assert_eq!(turns(&app), 0, "a refusal spent a turn");
            let said = rows(&app, now);
            assert_eq!(said.len(), 1, "two files, two lines: {said:?}");
            let Some(Line::Note { text }) = said.first() else {
                panic!("a refusal is a note of warlock's own: {said:?}");
            };
            assert!(
                text.contains(&template.display().to_string()),
                "the first file read is not the one named: {text}"
            );

            // The template fixed, the misspelled key is what the next one says
            // — and it is still one line and still no turn.
            fs::write(&template, "## Ours\n\nsay the thing.").expect("a template file");
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(
                app.mode(),
                Mode::Chat,
                "the second refusal entered the mode"
            );
            assert_eq!(turns(&app), 0, "the second refusal spent a turn");
            let said = rows(&app, now);
            assert_eq!(
                said.len(),
                2,
                "the second refusal is one more line: {said:?}"
            );
            let Some(Line::Note { text }) = said.last() else {
                panic!("a refusal is a note of warlock's own: {said:?}");
            };
            assert!(
                text.contains(&briefs_path(repo.path()).display().to_string()),
                "the second file is not named: {text}"
            );
            assert!(
                text.contains("directroy"),
                "the offending key is not named: {text}"
            );
            assert_eq!(
                chat.directory(),
                DEFAULT_BRIEF_DIRECTORY,
                "a refusal moved where a brief would go",
            );
        }

        #[test]
        fn a_refusal_leaves_where_a_brief_would_go_where_the_last_good_one_put_it() {
            // The other half of "the directory is written at every `/brief`":
            // it is written at every `/brief` that *happens*, and a refusal is a
            // command that does not happen at all. So a session that has briefed
            // into `plans/` and then meets a `briefs.toml` somebody has broken
            // goes on pointing at `plans/` — it does not fall back to the
            // default, and it does not point at nothing.
            //
            // This wants a conversation that has already briefed, which is why
            // it is its own test rather than a line in the refusal's: that one
            // is about a first `/brief` refusing, and asserts a mode never
            // entered and a turn never spent.
            let now = Instant::now();
            let repo = a_root();
            write_briefs(repo.path(), "directory = \"plans\"\n");
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);
            assert_eq!(chat.directory(), "plans", "the good brief did not settle");

            // And then the file somebody broke between the two commands.
            write_briefs(repo.path(), "directory = [\n");
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(
                chat.directory(),
                "plans",
                "a refusal moved where a brief would go"
            );
            assert_eq!(turns(&app), 1, "a refusal spent a turn");
        }

        #[test]
        fn a_second_brief_re_reads_the_file_and_takes_the_new_directory() {
            // The file is read at every `/brief` and never held, so editing it
            // with `e` and typing the command again is the whole of changing
            // where a brief lands — no restart, and nothing on the app or the
            // conversation remembering the old answer.
            let now = Instant::now();
            let repo = a_root();
            write_briefs(repo.path(), "directory = \"plans\"\n");
            let mut app = App::default();
            let mut chat = conversation_in(repo.path());

            submit_into(&mut app, &mut chat, "/brief", now);
            assert_eq!(chat.directory(), "plans");

            write_briefs(repo.path(), "directory = \"notes/adr\"\n");
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(
                chat.directory(),
                "notes/adr",
                "the second /brief re-read nothing"
            );
            assert_eq!(app.mode(), Mode::Brief);
            assert_eq!(turns(&app), 2, "the second /brief cost no turn");
        }

        #[test]
        fn brief_in_brief_mode_re_sends_the_instruction_and_notes_nothing() {
            // Typing it again is the remedy for a register that has drifted, so
            // it costs a turn every time — and says nothing new about the mode,
            // because the mode did not change.
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "/brief", now);

            assert_eq!(app.mode(), Mode::Brief);
            assert_eq!(
                rows(&app, now),
                [
                    vec![note(BRIEF_NOTE)],
                    asked(BRIEF_COMMAND).to_vec(),
                    asked(BRIEF_COMMAND).to_vec(),
                ]
                .concat(),
            );
            assert_eq!(turns(&app), 2, "the second /brief cost no turn");
        }

        #[test]
        fn chat_leaves_the_mode_with_one_note_and_one_turn() {
            // The way out, and the same shape as the way in: the register is
            // left, warlock says so once, and the model is told the other
            // instruction as one ordinary turn shown as `/chat`.
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "/chat", now);

            assert_eq!(app.mode(), Mode::Chat, "/chat did not leave the mode");
            assert_eq!(
                rows(&app, now),
                [
                    vec![note(BRIEF_NOTE)],
                    asked(BRIEF_COMMAND).to_vec(),
                    vec![note(CHAT_NOTE)],
                    asked(CHAT_COMMAND).to_vec(),
                ]
                .concat(),
            );
            assert_eq!(turns(&app), 2);
            assert!(chat.answering(), "the instruction was never sent");
        }

        #[test]
        fn chat_in_chat_mode_is_one_line_and_costs_no_turn() {
            // There is nothing to leave, so there is nothing to tell the model:
            // a turn spent saying the conversation is where it already was is a
            // question nobody asked and money nobody meant to spend.
            let now = Instant::now();
            let (app, chat) = submit("/chat", now);

            assert_eq!(app.mode(), Mode::Chat);
            assert_eq!(rows(&app, now), vec![note(ALREADY_CHATTING)]);
            assert_eq!(turns(&app), 0, "/chat in chat mode opened a turn");
            assert!(!chat.answering(), "/chat in chat mode asked the model");
            assert!(chat.composer().draft().is_empty());

            // And the same after a mode that really was left: the refusal is
            // about the state, not about how the conversation got into it.
            let mut app = App::default();
            let mut chat = conversation();
            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "/chat", now);
            let before = turns(&app);
            submit_into(&mut app, &mut chat, "/chat", now);

            assert_eq!(turns(&app), before, "the second /chat cost a turn");
            assert_eq!(
                rows(&app, now).last(),
                Some(&note(ALREADY_CHATTING)),
                "the second /chat said something else",
            );
        }

        #[test]
        fn a_mode_clears_hides_and_reorders_nothing_that_was_already_said() {
            // The property the whole design rests on: the turns already on the
            // card are the material a document is made of. Entering the mode and
            // leaving it again puts rows *under* them and moves none of them.
            let now = Instant::now();
            let mut app = App::default();
            let mut chat = conversation();

            submit_into(&mut app, &mut chat, "why nine passes?", now);
            let before = rows(&app, now);
            submit_into(&mut app, &mut chat, "/brief", now);
            submit_into(&mut app, &mut chat, "/chat", now);

            let after = rows(&app, now);
            assert_eq!(after[..before.len()], before[..], "the card was rewritten");
            assert_eq!(
                after,
                [
                    before,
                    vec![note(BRIEF_NOTE)],
                    asked(BRIEF_COMMAND).to_vec(),
                    vec![note(CHAT_NOTE)],
                    asked(CHAT_COMMAND).to_vec(),
                ]
                .concat(),
            );
            assert_eq!(turns(&app), 3);
        }

        #[test]
        fn a_mode_leaves_every_answer_and_every_work_line_exactly_where_it_was() {
            // The same property with the card full rather than empty, which is
            // the state a `/brief` is actually typed in: the conversation worth
            // converging on is one that has been going for a while, and by then
            // the turns on the card carry the answers and the work lines that
            // are the material a document is made of. Losing those to a mode
            // change would be losing the brief before it started — and it is
            // the failure a second session would show up as, because a session
            // that starts again starts with nothing on the card.
            //
            // The rows come first, because that is what the reader has, and
            // then the turns themselves, because a row that merely *drew* the
            // same is not the same answer.
            let now = Instant::now();
            let later = now + Duration::from_secs(30);
            let mut app = App::default();
            let mut chat = conversation();

            // One turn that was worked at and answered, and one that ended
            // without an answer: both are things a mode change could drop.
            submit_into(&mut app, &mut chat, "why nine passes?", now);
            app.record_turn(
                &Activity::Tool {
                    name: "Read".to_owned(),
                    detail: Some("crates/warlock-engine/src/lib.rs".to_owned()),
                },
                now,
            );
            app.record_turn(&Activity::Thinking, now);
            app.answer_turn("One pass per directory, bottom up.", now);
            submit_into(&mut app, &mut chat, "and the manifest?", now);
            app.end_turn(&Ending::NothingSaid, now);

            let before = rows(&app, later);
            let asked_already: Vec<_> = app
                .thread()
                .expect("two questions were asked")
                .turns()
                .into_iter()
                .cloned()
                .collect();
            // The history is really a history: an answer, work lines and an
            // ending are all on the card before the mode is touched, so the
            // equalities below are about something rather than about nothing.
            assert!(
                before.iter().any(|line| matches!(line, Line::Text { .. })),
                "there is no answer on the card to survive anything: {before:?}"
            );
            assert!(
                before
                    .iter()
                    .filter(|line| matches!(line, Line::Clocked { .. }))
                    .count()
                    >= 3,
                "there is no work on the card to survive anything: {before:?}"
            );

            submit_into(&mut app, &mut chat, "/brief", later);
            submit_into(&mut app, &mut chat, "/chat", later);

            // Every row that was there is still there, at the index it was at:
            // nothing cleared, nothing hidden, nothing reordered, and the two
            // answers and every work line word for word.
            let after = rows(&app, later);
            assert_eq!(
                after[..before.len()],
                before[..],
                "entering and leaving the register rewrote the conversation"
            );
            // And the turns under those rows: the message, the answer and the
            // ending of each, unchanged and in the order they were asked in.
            let asked_now: Vec<_> = app
                .thread()
                .expect("the conversation is still there")
                .turns()
                .into_iter()
                .cloned()
                .collect();
            assert_eq!(
                asked_now[..asked_already.len()],
                asked_already[..],
                "a mode change took a turn of the conversation"
            );
            assert_eq!(
                asked_now.len(),
                asked_already.len() + 2,
                "the two commands did not cost the two turns they are supposed to"
            );
            assert_eq!(app.mode(), Mode::Chat, "the register was never left");
        }

        #[test]
        fn a_refusal_is_exactly_one_note_and_never_a_turn() {
            // The whole of what a missed command costs: one line on the card,
            // in warlock's own voice, and not a question anybody paid for.
            let now = Instant::now();
            let refusal = Submitted::Refused
                .refusal()
                .expect("a refused draft has a line");

            for draft in ["/breif", "/plan", "/BRIEF", "/", "/brief now", "/brief\nx"] {
                let (app, chat) = submit(draft, now);

                assert_eq!(
                    rows(&app, now),
                    vec![Line::Note {
                        text: refusal.to_owned()
                    }],
                    "{draft:?} did not leave exactly one note"
                );
                assert_eq!(turns(&app), 0, "{draft:?} opened a turn");
                assert!(!chat.answering(), "{draft:?} was asked of the model");
                assert!(
                    chat.composer().draft().is_empty(),
                    "{draft:?} was left in the field"
                );
            }
        }

        #[test]
        fn a_message_submits_as_it_always_did() {
            // The behaviour the classifier must not have changed: the words go
            // on the card as the reader's own, one turn is opened, the question
            // is out, and the field is empty behind it. A path is here too,
            // because `/home/cole/notes` is a message and the reader who typed
            // it is talking about a file.
            let now = Instant::now();

            for draft in [
                "why nine passes?",
                "/home/cole/notes",
                "tell me about /brief",
            ] {
                let (app, chat) = submit(draft, now);

                assert_eq!(turns(&app), 1, "{draft:?} did not open one turn");
                // The question as it was typed, and under it the clocked row a
                // turn with nothing back yet draws — a live turn, which is
                // exactly what a command and a refusal never leave.
                assert_eq!(
                    rows(&app, now),
                    vec![
                        Line::Said {
                            text: draft.to_owned()
                        },
                        Line::Clocked {
                            clock: "0:00".to_owned(),
                            text: "waiting".to_owned()
                        }
                    ],
                    "{draft:?} is not on the card as it was typed"
                );
                assert!(chat.answering(), "{draft:?} was never asked");
                assert!(
                    chat.composer().draft().is_empty(),
                    "{draft:?} was left in the field"
                );
            }
        }
    }
}
