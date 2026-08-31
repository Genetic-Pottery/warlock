//! Warlock's terminal front end, minus the terminal.
//!
//! Almost everything here is data and functions over data: the flattened tree
//! the screen shows, where the selection sits, which colour a state is drawn
//! in, and what one frame looks like. None of it opens a terminal, so all of it
//! is testable with nothing attached to stdout — the draw path against an
//! in-memory buffer, the rest against plain values. Raw mode, the alternate
//! screen and the event loop belong to the binary in `src/main.rs`.
//!
//! A view here outlives the tree it was built on, because a run writes documents
//! and the tree has to be read again to show them. [`reseat_on`] is that move,
//! and it is a function over two values like everything else: rows, states and
//! the tally come from the new tree, while what the reader has done to the view
//! — the collapsed directories, the filters, the window and the focus — comes
//! from the old one, along with what the footer was saying and the account in
//! the panel. The selection travels by path rather than by row number, because
//! an index names whichever node now sits at that position, so a reload leaves
//! the reader where they were rather than at the top. It reads no disk and asks
//! for no reload of its own — *when* a tree is read again is the binary's
//! business.
//!
//! Which of an [`App`]'s facts survive that move is a property of the type
//! rather than a list somebody keeps: the reader's view state, the footer's and
//! the panel's are each one value, so a re-seat moves three things and has
//! nothing inside them to forget. What it does *not* move is the header line —
//! see [`Chrome`], which is not app state at all, because neither half of it can
//! change while warlock runs.
//!
//! What a run is *doing* while it runs is data of the same kind. [`Account`] is
//! one pact's own record of itself — a section per directory, a line per thing a
//! pass was seen doing, an elapsed clock on every line — and it holds no clock of
//! its own: the instant a line is measured against is handed in by whoever is
//! drawing the frame, so a whole minutes-long run can be driven through it in a
//! test with nothing attached to stdout and no time passing at all.
//!
//! The conversation is data of that kind too. [`Thread`] is every message
//! somebody typed at the foot of the panel, in order, each with the work the
//! model was seen doing about it and the answer that came back — the same clock
//! rule as an [`Account`], down to the same code, so a turn's work lines count
//! from the question that started them and the newest one ticks with the `now`
//! the frame hands in. Two things about it are not the account's, and both are
//! wording rather than mechanism: a turn's money says out loud that it belongs
//! to no pact's total, because the panel now draws both kinds of row and a
//! sentence is the only thing that can keep them from being read as summable;
//! and a turn that does not answer ends in exactly one line — cancelled, no
//! `claude` to ask, a non-zero exit, a timeout, or a model with nothing to say —
//! so a failure costs the reader a row rather than a screen. Between the turns
//! sit warlock's own lines — a note is one unclocked row it says for itself, in
//! the same sequence, because when it was said is the whole of what it means.
//! It is the one card that shows prose, and it is still a value: no channel, no
//! child process, and no clock of its own.
//!
//! A run that happens while a conversation is going on puts nothing in it. The
//! panel has a card for a run — the [`Account`], one swap away — and a
//! conversation that also carried it would be the same run written twice on one
//! screen, arriving in the middle of what the reader was reading. So a pact
//! fills its own card while they go on reading the answer they asked for, and
//! neither card takes the other's place: what is on screen is the reader's, and
//! nothing a run does moves it. Money follows the same line — a pass's cost is
//! part of what the pass produced and is said on the account, per pass and
//! totalled in `pact finished — …`, and what a chat turn costs is said
//! nowhere.
//!
//! *When* the tree is read again is decided the same way. The disk keeps moving
//! after a load — a file saved in another window, a branch checked out, a build
//! writing thousands of files nobody wants to hear about — and [`WatchPolicy`]
//! is what that movement is weighed by: a filter that is the last walk itself,
//! so an event counts only when its immediate parent is a directory the load
//! produced, and three timing rules that turn one editor save into one reload
//! and a long burst into a handful. It watches nothing and reloads nothing, and
//! it holds no clock either — the instant it compares against is handed in, so a
//! ten-second burst is driven through it in a test in microseconds. Hearing that
//! the disk moved at all is the one impure thing in that module and is kept
//! apart from every decision made about it: [`Watch`] owns the watcher handle,
//! hands on the paths it is given unfiltered, and comes back as [`Watching`] —
//! a value — when no watcher could be started, because warlock without live
//! updates is still warlock.
//!
//! The other exception is the two agents, which own child processes.
//! [`ClaudeAgent`] implements the engine's [`Agent`](warlock_engine::Agent) port
//! by running the `claude` CLI, because the engine spawns nothing and something
//! has to. It decides nothing about what a prompt says, and it needs no terminal
//! either — its tests point it at stand-ins and pass on a machine with no
//! `claude` installed.
//!
//! [`ChatAgent`] is the second one: the same CLI and the same transport, run for
//! a different kind of work. A pact is a pass per directory that writes
//! documents; a turn here is one message somebody typed at the foot of the panel,
//! answered in prose for the panel. That is why it is not an
//! [`Agent`](warlock_engine::Agent) at all — a request names a directory and
//! carries the files under it, and a typed sentence names nothing and carries
//! nothing, so a turn is a message rather than a request and the port is left to
//! the runs that fit it. What goes to the child is that message and nothing else:
//! no tree dump, no repository contents, no transcript warlock kept, because one
//! session serves the life of the process and the model remembers its own
//! conversation. And a turn is read-only by construction rather than by
//! intention — the vector grants `Read`, `Grep` and `Glob` and nothing that
//! writes, edits, runs a shell or reaches the network — so a question about the
//! repository is answered out of the repository and the repository is left
//! exactly as it was. Both agents live in the one module here that runs
//! anything; everything else in this crate is still data and functions over
//! data.
//!
//! The gate on the way out is data too. [`QuitConfirm`] is whether the quit
//! confirmation is up and which of its two answers is lit, and [`answer_for`]
//! says what one key does to it — a value and a pure function, not a mode the
//! event loop keeps in its head, so both the drawing and the key handling are
//! assertable with nothing attached to stdout. It is deliberately not a field on
//! [`App`]: answering No has to leave the app exactly as it was, and the app
//! never having heard of the question is a cheaper guarantee of that than
//! putting every field back.
//!
//! The scope prompt is the same arrangement with a string in it. [`ScopePrompt`]
//! is whether the field is up, which directory it is over, what has been typed
//! and why the last submit was refused, and [`edit_for`] says what one key does
//! to it — append a character, take one back, submit, or close. It judges
//! nothing: Enter comes back as a submit whatever was typed, and whether that is
//! a scope is the engine's
//! [`validate_scope`](warlock_engine::validate_scope)'s answer, asked by the
//! caller, so this crate holds no idea of how long a scope may be or which
//! characters it may hold. It is not a field on [`App`] either, for the reason
//! the confirmation is not.
//!
//! The composer is that arrangement again with several lines in it.
//! [`Composer`] is what has been typed at the foot of the panel's column, and
//! [`compose_for`] says what one key does to it — append a character, take one
//! back, start a new line on Alt+Enter, offer the draft up on Enter, or hand the
//! keyboard back on Esc. It is the field that lets every single-letter command
//! warlock has go back to being a letter: while it holds the keyboard, `p` is
//! the letter p. It knows its own height as well as its own text, because the
//! panel above it loses exactly the rows it takes and that arithmetic has to
//! happen before the frame is cut — one row when empty, one more per newline or
//! wrap, and never more than [`COMPOSER_MAX_ROWS`], past which it scrolls within
//! itself so the row the cursor is on stays on screen.
//!
//! What a submitted draft *is* is the sentence after that one, and it is a
//! function too. [`submitted_for`] takes the draft and comes back with one
//! [`Submitted`]: one of the three commands warlock has, an ordinary message
//! for the model, or a refusal. The rule is trim, first token, no leading slash
//! means a message, a second slash means a path and so also a message, and only
//! then a case-sensitive match against three words with nothing permitted after
//! them. Case-sensitive because the two mistakes cost different amounts — a
//! refusal costs one line on the card and a send costs a turn — and the refusal
//! is one line naming all three, which is the whole of the discovery mechanism
//! and the reason there is no `/help`. Like everything else here it decides and
//! does nothing: no mode, no file, no turn.
//!
//! The dependency edge runs TUI -> engine: this crate knows the engine's
//! vocabulary, and the engine knows nothing about terminals.

mod account;
mod app;
mod claude;
mod colour;
mod composer;
mod confirm;
/// The hand-written tree the tests in this crate draw and walk, so none of
/// them needs a repository on disk. Test-only, and private on purpose: the
/// real tree comes from the engine's loader.
#[cfg(test)]
mod fixture;
mod prompt;
mod submission;
mod thread;
mod ui;
mod watch;
mod wrap;

/// Everything one pact did, in the order it did it: a section per directory, a
/// line per thing a pass was seen doing, and a clock on every line.
pub use account::Account;
/// One drawable row of the panel: a section heading, a clocked line or the run's
/// closing summary of an [`Account`], or one line of a file somebody asked to
/// read.
pub use account::Line;
/// How a directory's pass ended, in the words it ends its section with.
pub use account::Outcome;
/// One directory's pass inside an [`Account`].
pub use account::Section;
/// The front end's state: the flattened tree, which of it is collapsed, the
/// selected row and the slice of rows on screen — moved by the keys, which
/// drive whichever pane has the focus, or by a pointer, which names the row and
/// the pane it landed on and so consults no focus at all. It also carries what
/// the binary has told it about the terminal's mouse capture, which the footer
/// names the key that changes it by. What it does not carry is the header line,
/// which is a [`Chrome`].
pub use app::App;
/// What the header line states — which tree is on screen, and what this machine
/// holds for the repository it came out of. Resolved once by whoever loaded the
/// app and handed to [`draw`] beside the two windows, deliberately not a field
/// on [`App`]: neither half can change under a running warlock, so an app that
/// has never heard of them is an app no keystroke can be suspected of having
/// changed them on.
pub use app::Chrome;
/// Which of the screen's three places the keys are driving — the tree, the
/// panel or the composer: cycled by the focus key, or set outright by a click
/// naming a pane.
pub use app::Focus;
/// Which register the conversation is in — questions about the repository, or a
/// conversation converging on a document — entered and left from the composer,
/// and said on the panel's border title and nowhere else.
pub use app::Mode;
/// What a pact toggle changed, for whoever has to write it to the manifest.
pub use app::PactToggle;
/// One line of the flattened tree.
pub use app::Row;
/// Which kind of run is in flight — a pact or a refresh — which is the one word
/// of the footer's progress line the two differ by.
pub use app::Run;
/// The run in flight in parts rather than in words — which run, the directory it
/// is working spelled for the tree on screen, and its position out of the run's
/// total — for whatever draws a header over the panel.
pub use app::RunHeader;
/// What this machine holds for the repository on screen — sigils, nothing, or a
/// config that could not be read — as the tree pane's header states it.
pub use app::Sigils;
/// Put a view back on top of a tree that has just been read again: same
/// selection, same collapsed directories, same filters, same window, same
/// footer, same panel, new rows. Every field of the old view is named in one
/// pattern with no `..` in it, so a field added to [`App`] stops this
/// compiling rather than quietly ceasing to be carried.
pub use app::reseat_on;
/// A handle for hearing what a model pass is doing, from another thread, while
/// it is still running.
pub use claude::Activities;
/// One thing a model pass was seen doing: a tool call, a stretch of thinking, or
/// what the pass cost.
pub use claude::Activity;
/// How hard a turn is asked to think once the conversation is aimed at a
/// document: a level above the one a question runs at, still overridden by
/// `WARLOCK_EFFORT`.
pub use claude::BRIEF_EFFORT;
/// What warlock says into the conversation already in progress when somebody
/// asks for brief mode: the artifact, the shape it takes, and that the job is to
/// argue toward a decision rather than to agree.
pub use claude::BRIEF_INSTRUCTION;
/// The same instruction the other way: no artifact, no shape, and back to
/// answering questions about the repository as they come.
pub use claude::CHAT_INSTRUCTION;
/// A handle for stopping a model pass from another thread: it kills the child
/// running now and refuses to start another.
pub use claude::Cancel;
/// The reading half of the model seam: a conversation with the `claude` CLI, one
/// session for the life of the agent, where a turn is a message rather than a
/// request and the model may read the repository but never write to it.
pub use claude::ChatAgent;
/// The engine's model-pass port, implemented by running the `claude` CLI as a
/// child process.
pub use claude::ClaudeAgent;
/// How long one model pass is given before it is killed: five minutes, per
/// invocation rather than per pact.
pub use claude::INVOCATION_TIMEOUT;
/// The colour a node state is drawn in.
pub use colour::colour_for;
/// The most rows the composer is ever drawn in, however long the draft gets.
pub use composer::COMPOSER_MAX_ROWS;
/// What a keystroke comes to while the composer holds the keyboard: the draft
/// stays with one character more or less, the keyboard goes back on Esc, or a
/// draft with something in it is submitted on Enter.
pub use composer::Composed;
/// The composer's draft: the several lines typed at the foot of the panel's
/// column, with the cursor always at the end, and how many rows they need at a
/// given width.
pub use composer::Composer;
/// What one key does to the composer, given the draft it is holding.
pub use composer::compose_for;
/// One of the two answers the quit confirmation offers: Yes, drawn on the left,
/// and No, drawn on the right and lit when the question opens.
pub use confirm::Answer;
/// What a keystroke comes to while the quit confirmation is up: the question
/// stays with an answer lit, closes on No, or leaves on Yes.
pub use confirm::Answered;
/// Whether the quit confirmation is up, and which answer is lit while it is —
/// a value of its own rather than a field on [`App`], so answering No leaves the
/// app untouched rather than carefully restored.
pub use confirm::QuitConfirm;
/// What one key does to the quit confirmation, given the answer it has lit.
pub use confirm::answer_for;
/// What a keystroke comes to while the scope prompt is up: the field stays with
/// one character more or less, closes on Esc, or is submitted on Enter.
pub use prompt::Edited;
/// The scope prompt's field: the directory being scoped, the text typed into it
/// with the cursor always at the end, and the one line saying why the last
/// submit was refused.
pub use prompt::ScopeField;
/// Whether the scope prompt is up, and the field being typed into while it is —
/// a value of its own rather than a field on [`App`], so Esc leaves the app
/// untouched rather than carefully restored.
pub use prompt::ScopePrompt;
/// What one key does to the scope prompt, given the field it is open over.
pub use prompt::edit_for;
/// What a submitted draft turns out to be: one of warlock's three commands, an
/// ordinary message for the model, or a refusal — which carries the one line
/// naming the commands that exist.
pub use submission::Submitted;
/// What a draft submitted at the composer means, given nothing but the draft.
pub use submission::submitted_for;
/// One thing that stopped a turn short of an answer — a cancel, no `claude` to
/// ask, a non-zero exit, a timeout, or a model that said nothing — in the one
/// line it ends with.
pub use thread::Ending;
/// The conversation the panel's third card holds, as one ordered sequence:
/// every message somebody typed, the work the model was seen doing about it,
/// what it answered — and, where they were said, warlock's own unclocked notes.
pub use thread::Thread;
/// One turn of a [`Thread`]: a question, everything the model was seen doing
/// about it, and the answer when it lands — or the one line it ended in
/// instead.
pub use thread::Turn;
/// Which [`Ending`] a failed turn is: the model seam's failure vocabulary as
/// the panel's, in one place, so whoever runs a turn hands the error over rather
/// than wording it.
pub use thread::ending_for;
/// What is drawn where a pointer landed: the footer, a border, the tree's
/// header, a row of the tree's window, the run's header, a line of the panel's
/// window, the composer.
pub use ui::Hit;
/// How many rows of the panel's column the composer takes in a terminal of a
/// given size, its border included — the rows [`panel_height`] no longer has.
pub use ui::composer_height;
/// The composer as a frame drawn over an app has it: nothing at all while the
/// document card is showing.
pub use ui::composer_on_screen;
/// Draw one frame of the app.
pub use ui::draw;
/// Which part of the frame a screen point falls on, measured off the layout the
/// frame is cut by.
pub use ui::hit_test;
/// How many lines of the pact's account a terminal of a given size has room for
/// in the panel, once the composer under it and the run's header above it have
/// taken their rows.
pub use ui::panel_height;
/// How many columns wide the panel's contents are in a terminal of a given
/// size, which is the width a document in it is wrapped to.
pub use ui::panel_width;
/// How many rows of the panel a terminal of a given size gives the run's header
/// — the rows [`panel_height`] no longer has — and none at all with no run in
/// flight.
pub use ui::run_header_height;
/// How many rows of tree a terminal of a given size has room for.
pub use ui::tree_height;
/// How many further reloads the events arriving during a reload are worth: one,
/// however many of them there were.
pub use watch::COALESCED_RELOADS;
/// The directories the last successful load produced: the filter every
/// filesystem event is held against.
pub use watch::NodeSet;
/// How long the disk has to be quiet before a reload — the debounce.
pub use watch::QUIET_PERIOD;
/// The longest a reload is put off while events keep arriving — the ceiling.
pub use watch::RELOAD_CEILING;
/// A running filesystem watcher over the tree's root and the manifest: it
/// hands on the paths it hears about and decides nothing.
pub use watch::Watch;
/// Told what moved on disk and when, it answers whether the tree is owed a
/// reload right now — and reloads nothing itself.
pub use watch::WatchPolicy;
/// What came of asking for a watcher: one that is running, or one line saying
/// why there is none.
pub use watch::Watching;
