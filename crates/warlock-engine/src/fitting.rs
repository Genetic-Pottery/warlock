//! Fitting a directory into a request a model pass can be handed: what goes
//! in, what is described instead of sent, and what is left as a name and a
//! size.
//!
//! [`pact`](crate::pact) owns the descent — which directories are visited, in
//! what order, and what becomes of the documents they produce. This module owns
//! the one question underneath it: given a directory on disk and two byte caps,
//! what exactly is the [`agent::Request`] for it? Section 11 of the design doc
//! calls context scoping "the actual differentiator: maximal relevant context,
//! minimal waste", and this is that sentence made mechanical.
//!
//! # The one thing a caller has to know
//!
//! [`fit`] is the whole interface. It answers with a request or with the one
//! kind of failure a directory can have — it could not be walked, or a path in
//! it has no relative, forward-slash, UTF-8 spelling — and never with anything
//! in between. **Neither cap may fail a fit.** Going over either always
//! degrades: a file too big to send is described, and a file that cannot be
//! described is named and sized. Every such demotion comes back as a
//! [`Problem`] beside the request, so nothing is dropped silently and nothing
//! is truncated ever.
//!
//! That is the reason the ladder is behind one function rather than beside it.
//! It used to be four calls in a row — gather, reduce what is over the
//! per-file cap, demote until the whole request fits, then announce the weight
//! — and getting the order or the announcement wrong was a caller's mistake to
//! make. Now the order is not a caller's to get wrong, and the numbers
//! [`Observer::requesting`] hears are, by construction, the numbers of the
//! request that is about to be sent.
//!
//! # What goes in, and nothing else
//!
//! * **The directory's own files**, each with its bytes: the whole listing,
//!   except every markdown file in it — its own `WARLOCK.md`, its readme, and
//!   anything else prose ([`PROSE_EXTENSIONS`]). Files below the immediate
//!   children are never read — that is the waste the scoping exists to avoid.
//! * **Each immediate child directory's `WARLOCK.md`**, where one exists. This
//!   is how a directory learns what is under it: the children have already
//!   described themselves, so their parent reads summaries instead of source.
//!   A child with no document contributes no entry and is not an error; it is
//!   the ordinary state of a directory nobody has pacted yet.
//!
//! And one thing that never goes in: **this directory's own previous
//! `WARLOCK.md`**. It is the last pass's claim about the directory rather than
//! evidence of anything, and a pass shown it carried its sentences forward
//! whether or not the files still supported them. [`agent::Request`] carries
//! the whole argument; here it is simply dropped from the listing.
//!
//! The walk is the same walk as [`load`](crate::load) and [`hash`](crate::hash)
//! — the [`ignore`] crate, `follow_links(false)`, `require_git(false)`,
//! `.warlock/` pruned by name, `.warlockignore` honoured — so a file that is
//! gitignored, hidden, excluded by the repository or Warlock's own bookkeeping
//! is as absent from a request as it is from a tree or a digest. Symlinks are
//! neither followed nor listed.
//!
//! # The ladder, in order
//!
//! 1. **Gather.** Walk the directory, read what is under the per-file cap, and
//!    leave anything over it as a name and a size.
//! 2. **Reduce what was too big.** Every file left over the per-file cap comes
//!    back as its own declaration lines, from the language table. No model
//!    pass, no cache, no network.
//! 3. **Fit the whole request.** With skeletons now something a request can
//!    carry, meet the request cap by demoting to skeletons first and to names
//!    only when even the skeletons do not fit.
//! 4. **Announce the weight**, once, immediately before the request is handed
//!    back: the only point where both numbers are true, since steps 2 and 3 are
//!    exactly what changes them.
//!
//! # The two caps, and why neither can fail a pact
//!
//! A request has a budget, because a context window does:
//! [`PER_FILE_BYTE_CAP`] for any one file and [`request_byte_cap`] for the
//! whole thing. What happens at the edge of a budget is a decision, and two
//! were made here.
//!
//! **Omit and list, never truncate.** A file over budget is still in the
//! request — as its name and its size in bytes, with no contents at all. Half a
//! source file invites confident wrong conclusions about the half that never
//! arrived; a name and a size is accurate information a model can document
//! honestly ("a 4.1 MB `Cargo.lock`, not read"). That is the floor an over-cap
//! file can never fall below — and neither cap drops a file onto it while there
//! is a rung in between. A file in a request is **sent whole**, or **reduced**
//! (a name, a size, and the file's own declaration lines with a marker where
//! the bodies were), or **listed** by name and size, and both caps are met in
//! that order: the per-file one in [`lift_over_cap`], the whole-request one in
//! [`demote_to_budget`]. A name and a size is where a file lands when nothing
//! better can be said about it, not the first answer either cap gives. The
//! sections below are that ladder in full: how a skeleton is made, and how the
//! whole-request cap climbs down to it.
//!
//! **Over budget is never fatal.** Section 3 of the design doc says Warlock
//! never makes the wrong thing impossible, and failing here would do exactly
//! that: one committed lockfile or one generated schema would leave a directory
//! permanently unpactable, with no way out but deleting the file. So every
//! omission is a [`Problem`] reported alongside a request that is still
//! perfectly good — the same non-fatal shape [`load::Problem`](crate::load::Problem)
//! established, for the same reason: the thing that went wrong is said out
//! loud, once, rather than silently changing what happened.
//!
//! This does not contradict the rule that an unreadable file is fatal to a
//! *hash* (see [`hash`](crate::hash)): over budget is a disclosed policy this
//! module applies on purpose, while an undetected hole in a digest is a false
//! green nobody earned. A file that genuinely cannot be read is a third case
//! again, and gets its own cause ([`Omission::Unreadable`]) so it is never
//! mistaken for either.
//!
//! # What a file too big to send becomes
//!
//! A name and a size is honest, and it is thin. A directory whose biggest thing
//! is a two-megabyte lockfile got a document that could say the lockfile is
//! there and nothing whatever about what is in it — freshness with a hole in
//! the middle of it. So between the gather and the directory's own pass sits a
//! step of its own, [`lift_over_cap`]: every file the per-file cap listed is
//! read from disk and put back into the request as its own declaration lines,
//! by [`Reducing::skeleton_of`] and the table in
//! [`languages`](crate::languages).
//!
//! **What travels back is the file's own text, and no model pass is spent on
//! it.** A skeleton is lines the file really contains, in the order it contains
//! them, with a marker where the bodies were — so a route naming a symbol found
//! in one is anchored in code rather than in prose about code, and a second
//! pact over unchanged bytes costs exactly what the first did, because there
//! was never anything to cache.
//!
//! This replaced a map-reduce of model passes over each file's chunks, whose
//! answers were kept under `.warlock/summaries/`. Both are gone. Prose about a
//! file is a claim nothing can check and it cost a pass per chunk to obtain;
//! the declarations lifted out of the file are evidence, and they are free. A
//! file the table cannot reduce — an extension with no row, or one already all
//! declarations — stays exactly where it was, a name and a size, which is the
//! floor the whole ladder is built on and the one rung that never lies about
//! having read anything.
//!
//! # The cliff becomes a ladder
//!
//! The whole-request cap has the same choice to make, one file at a time, and it
//! now makes it in the same order. [`gather_request`] runs no passes, so the
//! only move it has is the old one — the biggest files become names and sizes
//! ([`trim_to_budget`]) — and [`demote_to_budget`] is that decision taken again
//! where the skeletons exist. Whole files step down to their declaration lines,
//! largest first, until the request fits; the files gather already cliffed are
//! read once more and step *up* to skeletons of themselves wherever one fits in
//! what is left of the budget, never back to their bytes; and a file loses its
//! skeleton altogether only when the request is still over the cap with every
//! skeleton in it. Three passes over a fixed list, so it always terminates, and a request
//! that will not fit whatever is given up — an enormous child document, a
//! directory of files that cannot be described — is simply sent as it is, over
//! the cap, with the problems that say why. The rung a file lands on is the
//! difference between a document that can say what the biggest thing in a
//! directory holds and one that can only say how many bytes it weighs.
//!

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::Utf8Error;

use ignore::WalkBuilder;

use crate::document::Described;
use crate::hash::length;
use crate::ignores;
use crate::languages;
use crate::pact::{DOCUMENT_FILE, Error, MANIFEST_DIR, Observer};
use crate::{Agent, agent, to_manifest_path};

/// A directory fitted into a request, and everything left out of it on the way.
///
/// The two fields are separate because a caller does different things with
/// them: the request is what a pass runs on, and the problems are what a front
/// end reports. A problem is never a failure — every one of them is a file that
/// was described or named instead of sent, which is what the caps are *for* —
/// so a [`Fitted`] with a long list of problems is an ordinary result and not a
/// degraded one.
#[derive(Debug)]
pub(crate) struct Fitted {
    /// What to hand the [`Agent`]: within both caps, whatever it took.
    pub(crate) request: agent::Request,
    /// Every file that did not go in whole, and why.
    pub(crate) problems: Vec<Problem>,
    /// What the fitting learned that the directory pass is not asked for: the
    /// line and routes each file pass wrote, and the names every text file
    /// declares. Carried to the render, where it is written beside the
    /// directory pass's own answer.
    pub(crate) described: Described,
}

/// The request for `directory`, within both caps, and what it cost to get there.
///
/// The whole of this module's interface. `prompt` is the instruction the pass
/// runs under, `agent` is the port the directory's own pass will run through,
/// and `observer` hears the finished request's weight once. Reducing a file
/// that is over the per-file cap runs no pass of its own, so there is nothing
/// between those two for either of them to hear about.
///
/// # Errors
///
/// [`Error::Walk`] if the directory cannot be listed, and [`Error::Path`] if
/// something under it has no relative, forward-slash, UTF-8 spelling. Those two
/// are the whole of it: **neither byte cap can fail a fit**, because going over
/// one demotes a file rather than refusing the directory. A caller that gets an
/// `Ok` back has a request it can send, however little of the directory fitted
/// into it.
pub(crate) fn fit(
    prompt: &str,
    directory: &Path,
    agent: &dyn Agent,
    observer: &mut dyn Observer,
) -> Result<Fitted, Error> {
    // Asked once, at the top, and carried down every rung: a budget that
    // changed under the ladder would let a file be given up against one number
    // and reported against another.
    let cap = request_byte_cap(agent.context_tokens());

    let Gathered {
        request,
        mut problems,
    } = gather_request(prompt, directory, cap)?;

    // The names every file the gather read declares, off the text itself:
    // free, mechanical, and the one part of a file's account nothing can
    // invent. Files read later — over the per-file cap, or lifted from the
    // cliff — are added as they are read.
    let mut described = Described::default();
    for file in request.files() {
        let text = file.kept().or_else(|| {
            file.bytes()
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
        });
        if let Some(text) = text {
            let names = languages::declared_names(Path::new(file.path()), text);
            if !names.is_empty() {
                described.declared.insert(file.path().to_owned(), names);
            }
        }
    }

    // Before the pass that writes the document, the step that gives it lines to
    // read for the files it would otherwise only be able to name. Costs no pass
    // and keeps nothing between runs. Infallible by construction: it answers
    // with a request either way, and every way it can go wrong is a `Problem`
    // in the list above.
    let mut passes = Reducing {
        directory,
        described: &mut described,
    };
    let request = lift_over_cap(request, &mut problems, &mut passes);

    // Then the whole-request budget, which gather could only meet by turning its
    // biggest files into names: with a skeleton of a file now something a
    // request can carry, the cap is met by demoting to skeletons first and to
    // names only when even the skeletons do not fit. Infallible in the same
    // way, and free in the same way.
    let request = demote_to_budget(request, cap, &mut problems, &mut passes);

    // What is about to be handed back, said out loud before it is: the only
    // point where both numbers are true, since summarising and demoting above
    // are exactly what changes them. Nothing is measured here that the budget
    // did not already measure. It is inside this function rather than left to
    // the caller so that the numbers announced cannot be the numbers of a
    // different request than the one that goes out.
    observer.requesting(
        request.files().len(),
        carried_bytes(request.files(), request.child_documents()),
    );

    Ok(Fitted {
        request,
        problems,
        described,
    })
}
/// How deep the walk goes: the directory itself (0), its own files and its
/// immediate children (1), and the files directly inside those children (2),
/// of which only `WARLOCK.md` is kept.
///
/// The depth limit *is* the scoping rule, enforced by the walker rather than by
/// remembering to check: source below an immediate child cannot reach a request
/// even by accident, because the walk never descends far enough to meet it.
const WALK_DEPTH: usize = 2;

/// The most bytes one file may contribute before it is listed instead of sent:
/// 128 KiB.
///
/// Roughly 300,000 tokens of source at the ~3.5 bytes per token that code
/// tokenises at — comfortably more than any hand-written source file, and
/// comfortably less than the generated artefacts this cap exists for:
/// lockfiles, vendored bundles, checked-in schemas, minified assets. A file
/// that trips this cap is almost never a file a model needed to read line by
/// line; its name, and the fact that it is enormous, is the part worth
/// documenting.
///
/// It was 128 KiB, against an assumed 200,000-token window and a claim that the
/// largest module in this repository was under 50 KiB. Both had stopped being
/// true — the windows the front end's [`Agent`] reaches are measured in
/// millions of tokens now, and this repository's own `app.rs` is over 400
/// KiB — so hand-written source was tripping a cap sized
/// to catch minified bundles, and paying for it in the worst currency there is:
/// one model pass per 96 KiB chunk, serially, while somebody watched. Pacting
/// `crates/warlock-tui/src` cost fifteen passes and 8m55s, of which fourteen
/// passes were summarising three ordinary Rust files. Those passes are gone
/// entirely now — an over-cap file is reduced to its declaration lines by the
/// language table — but the cap stays raised, because the rung below whole is
/// still worse than whole.
///
/// What the raise buys is not only time. A document written from part of a file
/// is a worse document than one written from the file, and at 1 MiB the source
/// goes to the pass that describes it.
pub const PER_FILE_BYTE_CAP: u64 = 1024 * 1024;

/// Bytes of source text per token, as a fraction, for turning a context window
/// into a budget this module can count against.
///
/// **Measured, not guessed.** Two files of different sizes were sent through
/// the same model and the difference in reported input tokens divided by the
/// difference in bytes, so the model's own fixed overhead cancels out:
///
/// | content | bytes per token |
/// |---|---|
/// | Rust (`scope.rs`) | 2.62 |
/// | TypeScript (a real service) | 2.60 |
///
/// Code lands near 2.6 whatever the language — punctuation, short identifiers
/// and indentation all tokenise poorly — so one number serves and 2.5 is that
/// number with a little margin under it.
///
/// **The margin points one way on purpose.** Estimating too *few* bytes per
/// token budgets a smaller request than the window could hold, which wastes a
/// little context. Estimating too many hands the model a request it cannot
/// read, and something downstream drops the difference with none of the care
/// taken here — the exact failure this whole budget exists to prevent. This
/// constant was 3, which is 15% the wrong side of the measurement, and the
/// measurement is why it is not any more.
const BYTES_PER_TOKEN_NUMERATOR: u64 = 5;

/// The denominator of [`BYTES_PER_TOKEN_NUMERATOR`]: together, 2.5 bytes to the
/// token. A fraction rather than a rounded integer because the nearest whole
/// numbers are 2 (a third of the window thrown away) and 3 (over the measured
/// figure, in the direction that breaks things).
const BYTES_PER_TOKEN_DENOMINATOR: u64 = 2;

/// Tokens of the window kept back from the file budget.
///
/// A request is not only the bytes this module counts. Measured on the same
/// runs that produced [`BYTES_PER_TOKEN_NUMERATOR`], what else is in the
/// window:
///
/// * **~16,700 tokens** of the agent's own standing overhead — its system
///   prompt and tool definitions — present before a byte of this request is.
///   That is the item most easily forgotten, because nothing in this crate
///   produces it and nothing here can see it.
/// * **~1,400 tokens** of [`PROMPT`](crate::pact) and the labels [`fit`] wraps each
///   file in.
/// * **The answer**, which for a document covering thirty-odd files runs to
///   6,000 tokens and should be allowed twice that.
///
/// Forty thousand covers those with roughly ten thousand to spare. The
/// children's documents and the previous document are *not* in this number:
/// they are inside the budget, counted by [`carried_bytes`] like any other
/// bytes the request carries.
const RESERVED_TOKENS: u64 = 40_000;

/// What an account of one file is reckoned to cost, when the cost has to be
/// guessed before any pass has written one.
///
/// Used by [`trim_to_budget`] alone, and only to keep back room for the
/// accounts [`lift_from_the_cliff`] is about to buy. Six kibibytes, measured
/// against the twenty accounts written under the `v2` prompts:
///
/// | min | median | mean | p75 | p90 | max |
/// |---|---|---|---|---|---|
/// | 3.6KB | 5.3KB | 5.5KB | 5.9KB | 7.4KB | 8.1KB |
///
/// It was eight, taken from the 52 accounts the `v1` prompts had produced —
/// a wider and heavier spread (mean 7.6KB, max 21.3KB). Asking for exact names
/// and visibility instead of free prose turned out to make accounts *shorter*
/// and far more uniform, so eight sat above the whole `v2` distribution and
/// reserved about half again as much as any account actually wanted. Six is a
/// little above the mean, which is where this number belongs.
///
/// **Which way to be wrong.** Reserving too much is not free: every extra byte
/// held back is a file cliffed that need not have been, and a file cliffed
/// loses its own text to prose about it. Reserving too little strands a file as
/// a bare name, which is worse. So this errs high — but by a margin taken from
/// a measurement rather than by a whole multiple.
///
/// Nothing is decided by it in any case: [`lift_from_the_cliff`] measures each
/// account against the real budget before committing to it, so this only sets
/// how much room is kept back, never whether an account fits.
const SAMPLE_BYTE_CAP: u64 = 6 * 1024;

/// The most lines of a file a sample carries, whatever they weigh.
///
/// A byte cap alone would send one line of a minified bundle and call it a
/// sample; a line cap alone would send six megabytes of a wide CSV. The two
/// together mean a sample is always both short and shaped like the file.
const SAMPLE_LINE_CAP: usize = 60;

/// The smallest budget any window may produce, however little is left after
/// [`RESERVED_TOKENS`].
///
/// A floor rather than an error, following this module's rule that no budget
/// may fail a fit: an agent reporting an implausibly small window gets a
/// request that is mostly names and sizes, which is a poor document and still a
/// document.
const MINIMUM_REQUEST_BYTES: u64 = 64 * 1024;

/// The most bytes one whole request may carry, given the window a pass will
/// actually be read in.
///
/// # Why this is a function now
///
/// It was a constant of 2 MiB, and the reasoning written beside it was that the
/// engine names no model, so the number had to be a budget the caller sets
/// rather than a limit anything here could measure. That was right about the
/// engine and wrong about the consequence. 2 MiB is around 700,000 tokens: it
/// fits a 1,000,000-token window and nothing else, and the model a pass really
/// runs on has 200,000. So the cap sat above the window it existed to respect,
/// every ladder below it was unreachable for any ordinary directory, and a
/// request too large for the model was handed over as though it fitted —
/// leaving the overflow to whatever the transport does about it, which is
/// exactly the silent, unordered, unreported truncation this module spends its
/// length refusing to do itself.
///
/// The engine still names no model. It asks: [`Agent::context_tokens`] is the
/// front end's answer about what it is talking to, and this turns that answer
/// into bytes the ladder can count. A budget that tracks the window is the only
/// kind that can keep the promise the rest of this module makes.
///
/// # What the budget counts
///
/// Everything the request carries: the bytes of the files sent whole, the
/// surviving lines of the ones elided or reduced to declarations, and the text
/// of the children's documents. Only files ever give anything up to get under
/// it, and they give it up in rungs — bytes before skeleton, skeleton before
/// name. [`gather_request`] makes the first answer without reading the
/// over-cap files at all, and [`demote_to_budget`] is that answer reconsidered
/// once their skeletons exist.
pub(crate) const fn request_byte_cap(context_tokens: u64) -> u64 {
    let budget = context_tokens
        .saturating_sub(RESERVED_TOKENS)
        .saturating_mul(BYTES_PER_TOKEN_NUMERATOR)
        / BYTES_PER_TOKEN_DENOMINATOR;
    if budget < MINIMUM_REQUEST_BYTES {
        MINIMUM_REQUEST_BYTES
    } else {
        budget
    }
}

/// The budget an [`Agent`] that names no window produces, so the tests have one
/// number to reason against.
///
/// Not a second policy: it is [`request_byte_cap`] of
/// [`DEFAULT_CONTEXT_TOKENS`](crate::agent::DEFAULT_CONTEXT_TOKENS), which is exactly what every stub agent in
/// this crate gets by taking the trait's default. Naming it keeps the tests
/// written against the cap the code computes rather than against a constant
/// that could drift away from it.
#[cfg(test)]
pub(crate) const REQUEST_BYTE_CAP: u64 = request_byte_cap(crate::agent::DEFAULT_CONTEXT_TOKENS);

/// The file as the pass should see it: its own lines with test bodies dropped
/// where [`languages`](crate::languages) knows how, and otherwise the file whole.
///
/// # Why this is policy and not a rung
///
/// Every other step in this module happens because a budget was exceeded, and
/// each one is reported as a [`Problem`] because something the pass wanted was
/// taken away from it. This is neither. A pass is asked what a directory is and
/// what a reader has to know before changing it, and the *body* of a test
/// answers neither question while its *name* answers both — so the bytes given
/// up here were never wanted, at any budget. Applying it only under pressure
/// would make a directory's request change shape when some unrelated file grew,
/// which is the opposite of the reproducibility the demotion order is sorted to
/// protect.
///
/// It is also the difference between a pact that fits and one that does not: in
/// this workspace it takes `warlock-engine/src` from 934KB to 508KB and
/// `warlock-tui/src` from 2.9MB to 1.4MB, which is the room the rungs below
/// would otherwise have taken out of source a reader does need.
///
/// # What it never does
///
/// Guess. A file whose extension no row claims, a language whose tests live in
/// files of their own, a block that opens and never closes — all of them come
/// back whole, because [`languages::elide`](crate::languages::elide) answers `None` rather than
/// improvising, and an elision that cut in the wrong place would delete working
/// code from a request without saying so. Bytes that are not UTF-8 have no
/// lines and are likewise untouched.
fn elided_or_whole(path: &Path, relative: String, size: u64, bytes: Vec<u8>) -> agent::File {
    let Ok(text) = str::from_utf8(&bytes) else {
        return agent::File::present(relative, bytes);
    };
    match languages::elide(path, text) {
        Some(elided) => agent::File::elided(relative, size, elided.text),
        None => agent::File::present(relative, bytes),
    }
}

/// Turn the biggest files into names and sizes until `carried` is inside
/// [`request_byte_cap`], reporting each one.
///
/// `on_disk` is the path each entry of `files` came from, index for index, so a
/// problem can name a file on the filesystem rather than a relative spelling.
/// Stops when the budget is met or when there is nothing left to give up,
/// whichever comes first — the second case is over budget with every file
/// already listed, which is still a request and still not an error.
///
/// Every file it can be handed is either sent whole or already listed, because
/// this runs inside [`gather_request`], which never opens an over-cap file: the
/// third state does not exist yet when this runs, so the demotion order it
/// calls for — whole, then reduced, then a bare name — is not a case this has
/// to answer. It is answered one level up, by [`demote_to_budget`], which runs
/// the same budget over the same order once the skeletons exist and steps every
/// file this function cliffed back up to a skeleton of itself wherever one
/// fits. So a listing made here is a first answer rather than a
/// final one, and this function stays what it is: the budget as it can be met
/// with no model pass at all.
fn trim_to_budget(
    files: &mut [agent::File],
    on_disk: &[PathBuf],
    carried: u64,
    cap: u64,
    problems: &mut Vec<Problem>,
) {
    if carried <= cap {
        return;
    }

    // Biggest first, and by path where two are the same size: the order files
    // are given up in has to be a property of the directory, not of how the
    // filesystem happened to enumerate it.
    let mut order: Vec<usize> = (0..files.len())
        .filter(|&index| !files[index].is_omitted())
        .collect();
    order.sort_by_key(|&index| (Reverse(files[index].size()), files[index].path().to_owned()));

    let mut carried = carried;
    for index in order {
        if carried <= cap {
            break;
        }
        // What the file costs the request, which is not what it costs the
        // disk: an elided file is carried as its surviving lines and reported
        // as its full size, and subtracting the second would credit the budget
        // with bytes that were never in it — leaving the loop convinced it had
        // met a cap it was still over.
        let spent = file_bytes(&files[index]);

        // A file given up here is not gone for good: it is on the cliff, and
        // [`lift_from_the_cliff`] buys an account of it back wherever one fits.
        // So the room that account will want is charged for now, at the moment
        // the file is given up, rather than discovered to be missing later.
        //
        // Without this the two steps work against each other. This one has no
        // agent and so has only one move, and it stops the instant the request
        // is inside the cap — leaving every byte of the budget claimed and the
        // lift nothing to spend. Measured on this workspace's own
        // `warlock-tui/src`: eleven files cliffed, 17KB of headroom left, and
        // seven of them — including the four largest and most important files
        // in the crate — reaching the pass as nothing but names, which is what
        // a document written by guessing is made of.
        //
        // Charging per file rather than reserving a flat share of the budget is
        // what keeps a directory that cliffs one file from paying for twenty.
        let Some(freed) = spent.checked_sub(SAMPLE_BYTE_CAP).filter(|&f| f > 0) else {
            // Its account would cost about what the file costs. Naming it would
            // buy nothing and lose its text, so it stays as it is and the next
            // file down is asked instead.
            continue;
        };

        let size = files[index].size();
        let path = files[index].path().to_owned();
        // A sample of its own text rather than a bare name. The room for one
        // was reserved a few lines up — `freed` is what is given up *after*
        // `SAMPLE_BYTE_CAP` is kept back — so the arithmetic below is the same
        // arithmetic it always was, and what the reserve buys is now sitting in
        // the request instead of being owed to it.
        files[index] = sampled(&files[index]).unwrap_or_else(|| agent::File::omitted(path, size));
        carried = carried.saturating_sub(freed);
        problems.push(Problem {
            path: on_disk[index].clone(),
            cause: Omission::OverBudget { size },
        });
    }
}

/// Replace every file the per-file cap listed with an account of what is in it,
/// wherever `agent` can produce one.
///
/// This is the step between [`gather_request`] and the directory pass. It is
/// handed the request that gather built and the problems it reported, and it
/// answers with the request the pass is actually run on: the same prompt, the
/// same directory, the same children's documents, and files in the same order,
/// with each successfully reduced one turned from [`agent::File::omitted`](crate::agent::File::omitted) into
/// [`agent::File::elided`](crate::agent::File::elided), carrying the file's own
/// declaration lines.
///
/// # Which files
///
/// Only the ones [`Omission::TooLarge`] put on the problem list — a file that is
/// over [`PER_FILE_BYTE_CAP`] by itself and would otherwise reach the pass as a
/// name and a size. Deliberately not the others that share that fate: a file the
/// filesystem refused ([`Omission::Unreadable`]) has no bytes to read, and a
/// file the request cap gave up ([`Omission::OverBudget`]) was given up to make
/// the request smaller, so putting some of it back is the opposite of what was
/// asked. The problem is found by matching its path against
/// `directory.join(file.path())`, which is exact here because only the
/// directory's own files are gathered.
///
/// # There is nothing kept between runs
///
/// The bytes are read once, here, reduced by the language table, and dropped.
/// This step used to hash them, look the digest up under
/// `<root>/.warlock/summaries/`, and pay a map-reduce of model passes on a
/// miss — a cache that existed because those passes were the expensive part of
/// a pact. Lifting declarations costs nothing, so there is nothing left worth
/// keeping, and the same bytes give the same lines on every machine without a
/// stored artefact to go stale, be committed, or be explained.
///
/// # What it does to the problem list
///
/// One file, one entry, always. A file that comes back reduced has its
/// `TooLarge` entry **removed** — lines of it reached the pass, so there is
/// nothing left out to report. A file that does not has that same entry's cause
/// **replaced** by the one that says why there is no skeleton: an extension the
/// table has no row for, a file that is already all declarations, or — for the
/// read this step does and gather did not — the filesystem refusing. Replaced rather than added, so a reader is
/// never told twice about one file, and the entry that survives is the one with
/// something to say.
///
/// Nothing here returns an [`Error`], and nothing here can stop a pact: the
/// worst case is the request gather already built, with better-explained
/// problems beside it.
///
/// # What the observer hears
///
/// Nothing here is announced to an observer and nothing here is paid for: the
/// reduction is a table lookup over bytes already in hand, so there is no pass
/// to report the start of and no spend to attribute. This used to run a
/// map-reduce of model passes per file and announce each one to the observer;
/// the passes, the announcement and the hook that carried it are all gone.
fn lift_over_cap(
    request: agent::Request,
    problems: &mut Vec<Problem>,
    passes: &mut Reducing<'_>,
) -> agent::Request {
    let directory = passes.directory;
    let mut files = request.files().to_vec();
    // The problems whose files ended up described, so their entries can go. Held
    // rather than removed as they are found, because removing from under the
    // loop would move every index still to be matched.
    let mut settled = Vec::new();
    let mut replaced = false;

    for file in &mut files {
        if !file.is_omitted() {
            continue;
        }
        let on_disk = directory.join(file.path());
        let Some(index) = problems.iter().position(|problem| {
            matches!(problem.cause, Omission::TooLarge { .. }) && problem.path == on_disk
        }) else {
            continue;
        };

        // Read here rather than in `gather_request`, which measured this file
        // and deliberately never opened it: the bytes are only worth holding for
        // as long as the passes over them take.
        let bytes = match fs::read(&on_disk) {
            Ok(bytes) => bytes,
            // It was over the cap a moment ago and is unreadable now. Whatever
            // happened to it, the honest cause is the filesystem's, and it is
            // the same one gather reports for a file it could not read.
            Err(source) => {
                problems[index].cause = Omission::Unreadable { source };
                continue;
            }
        };

        // The file's own declaration lines. Free — a table lookup over bytes
        // already read — so there is nothing to cache and nothing to bill.
        match passes.skeleton_of(file.path(), &bytes) {
            Ok(skeleton) => {
                let (path, size) = (file.path().to_owned(), file.size());
                // The size on disk, not the length of the skeleton: a file is
                // as big as it is however little of it is worth sending.
                *file = agent::File::elided(path, size, skeleton);
                settled.push(index);
                replaced = true;
            }
            Err(cause) => {
                // The table cannot reduce it — no row for the extension, or a
                // file that is already all declarations. Its own first lines
                // are still better than its name, so it falls to a sample
                // rather than the rest of the way to nothing.
                //
                // The problem stays on the list either way: most of the file
                // really is missing, and the entry is what says so. What
                // changes is that it now sits beside a sample of the file
                // instead of instead of one.
                problems[index].cause = cause;
                if let Ok(text) = std::str::from_utf8(&bytes)
                    && let Some(head) = sample_of(text)
                {
                    let (path, size) = (file.path().to_owned(), file.size());
                    *file = agent::File::elided(path, size, head);
                    replaced = true;
                }
            }
        }
    }

    // Sorted so the removals are back to front whatever order the files were
    // matched in, and so no earlier removal shifts a later index.
    settled.sort_unstable();
    for index in settled.into_iter().rev() {
        problems.remove(index);
    }

    if !replaced {
        // Nothing to say that the request does not already say. The common case,
        // and the one where rebuilding would be pure copying.
        return request;
    }

    // Rebuilt rather than mutated: an `agent::Request` is a value whose builders
    // append, and a request with one file exchanged is a different value, not a
    // request in a different state.
    agent::Request::new(request.prompt().to_owned(), directory)
        .with_files(files)
        .with_child_documents(request.child_documents().to_vec())
}

/// Fit the request to [`request_byte_cap`] by summaries first and names only
/// after, so the whole-request cap costs a file its text rather than every
/// account of it.
///
/// This is the step between [`lift_over_cap`] and the directory pass, and
/// it is where the third state of a file finally reaches the whole-request
/// budget. [`trim_to_budget`], inside [`gather_request`], knows two states and
/// so has one move: a file too big for the budget becomes a name and a size, and
/// everything the pass could have known about it is gone. Here there is a rung
/// between the two — [`agent::File::summarised`](crate::agent::File::summarised), a couple of hundred bytes of
/// prose about the whole file — and the budget is met by stepping down onto it
/// before anything falls off the ladder altogether.
///
/// # The three rungs, in order
///
/// The order within each rung is [`trim_to_budget`]'s and for the same reason:
/// biggest file first, ties broken by relative path, so which file gives way is
/// a property of the directory rather than of the order a walk happened to
/// return it in. Size is always the file's size **on disk** ([`agent::File::size`](crate::agent::File::size))
/// — what it spends is [`file_bytes`], and the two differ for exactly the files
/// this step is about.
///
/// 1. **Whole files become summaries**, largest first, while the request is over
///    the cap. This is the headline: a directory of hundreds of ordinary files
///    used to send the small ones and name the big ones, and now the big ones
///    arrive described. A file whose account comes back no shorter than the file
///    itself is left whole — a summary that costs more than the text it stands
///    for buys nothing, and the text is the better of the two.
/// 2. **Cliffed files become summaries**, largest first, while there is room
///    inside the cap for the account. By the time this runs, gather's cliff has
///    already turned the largest files into [`agent::File::omitted`](crate::agent::File::omitted) with
///    [`Omission::OverBudget`], so this rung is the ladder reconstituted: those
///    files are read from disk again — they are under [`PER_FILE_BYTE_CAP`], so
///    the read is cheap, and a filesystem that now refuses is
///    [`Omission::Unreadable`] — and put back as summaries. **Never back as
///    themselves.** A cliffed file was given up precisely to make the request
///    smaller, so restoring its bytes would undo the trim that made room and
///    invite this step to trim them again; the only way up from the cliff is the
///    rung above it. The first account that would not fit is put back on the
///    cliff it came from, with the cause it already had, and the rung stops
///    there rather than paying passes down a list of files that have nowhere to
///    go.
/// 3. **Anything still carrying bytes becomes a name and a size**, largest
///    first, while the request is *still* over the cap — summaries from rung one,
///    summaries from [`lift_over_cap`], and the whole files rung one left
///    alone. This is the old cliff, and it is now the last thing tried rather
///    than the first: a file only loses its account when the request does not fit
///    with the accounts in it.
///
/// Children's documents are counted at every rung and demoted at none, exactly
/// as in [`gather_request`]: a file left out still says its name and its size,
/// while a document left out replaces the only account of a whole subtree with
/// nothing.
///
/// # It always terminates, and it can end over the cap
///
/// Three passes over a fixed list of files, none of which loops back: an
/// oversized child document, or a directory whose every file is already a name,
/// comes out of here over the cap and is sent to the pass anyway. That is a fact
/// about the directory reported as [`Problem`]s, not a failure — no [`Error`]
/// variant is reachable from this function, exactly as none is from the caps
/// that led to it.
///
/// # What it does to the problem list
///
/// One file, one entry, the rule [`lift_over_cap`] established. A file that
/// ends up described has no entry — its contents reached the pass, so there is
/// nothing left out to report — and a file that ends up a name has exactly one,
/// whose cause is the honest reason there is no account of it: `OverBudget` when
/// the request simply had no room for one, and the summarising's own cause
/// ([`Omission::NotText`], [`Omission::TooManyChunks`],
/// [`Omission::Unsummarised`], [`Omission::Unreadable`]) when there was room and
/// no account could be made.
///
/// # What it costs, and what the observer hears
///
/// Every account goes through [`Summarising::summary_of`], so a file this
/// repository has read before costs no passes at all and announces nothing;
/// only the passes really run reach `observer`. The common directory — inside
/// the cap, with nothing given up to get there — costs one comparison and
/// returns the request it was handed.
fn demote_to_budget(
    request: agent::Request,
    cap: u64,
    problems: &mut Vec<Problem>,
    passes: &mut Reducing<'_>,
) -> agent::Request {
    let directory = passes.directory;
    let carried = carried_bytes(request.files(), request.child_documents());
    let cliffed = problems
        .iter()
        .any(|problem| matches!(problem.cause, Omission::OverBudget { .. }));
    if carried <= cap && !cliffed {
        // Inside the cap with nothing given up to get there: the ordinary
        // directory, and the one where every rung below is a no-op.
        return request;
    }

    // What no rung can give up: the children's documents, which are never
    // demoted and never dropped. If they are already over the cap by
    // themselves then the request is over the cap
    // whatever happens to the files, and every rung below would be spending
    // model passes to buy room that cannot exist — turning source into prose,
    // and prose into names, for a request that ends over the cap regardless.
    //
    // So nothing is spent and nothing is given up. The request goes out over
    // the cap with its files intact, which the module docs already name as a
    // legitimate ending. This used to happen by accident: `gather_request` had
    // cliffed those files to bare names on its way past, and a bare name is
    // what every rung below declines to work on. Once the trim stopped giving
    // up files whose accounts would cost more than they free, the accident
    // stopped happening and the passes started being paid for.
    let immovable = carried_bytes(&[], request.child_documents());
    if immovable > cap {
        return request;
    }

    let mut files = request.files().to_vec();
    // Computed once, from the sizes on disk, and walked three times: the rung a
    // file stands on changes under these loops, but which file is the biggest
    // may not, or two runs of the same pact could demote in different orders.
    let mut order: Vec<usize> = (0..files.len()).collect();
    order.sort_by_key(|&index| (Reverse(files[index].size()), files[index].path().to_owned()));

    let carried = demote_whole_files(passes, &mut files, &order, carried, cap, problems);
    let carried = lift_from_the_cliff(passes, &mut files, &order, carried, cap, problems);
    list_over_budget(directory, &mut files, &order, carried, cap, problems);

    if files == request.files() {
        // Nothing moved, so the request is the request: a directory whose files
        // were all already names, or one where every account declined.
        return request;
    }

    // Rebuilt rather than mutated, for the reason `summarise_over_cap` gives: a
    // request with one file exchanged is a different value, not a request in a
    // different state.
    agent::Request::new(request.prompt().to_owned(), directory)
        .with_files(files)
        .with_child_documents(request.child_documents().to_vec())
}

/// The bytes an account of `file` should be made from: the file as it is on
/// disk, whatever of it the request is currently carrying.
///
/// A whole file already holds them. An **elided** one does not, and this is why
/// the function exists: its surviving lines are a fair thing to send and a
/// false thing to describe, because a summary written from them would say a
/// file has no tests when what happened is that warlock dropped their bodies.
/// So the file is re-read, and the account is of the whole of it — the same
/// account [`lift_over_cap`] would have made, and the same cache entry,
/// since both are keyed by the file's real contents.
///
/// `None` for a file with no text in the request at all: a name and a size, or
/// an account already made, neither of which this rung has anything to do with.
/// `None` too if the re-read fails, which leaves the file on the rung it is on
/// rather than turning a full request into a failed one.
fn whole_bytes(directory: &Path, file: &agent::File) -> Option<Vec<u8>> {
    if let Some(bytes) = file.bytes() {
        return Some(bytes.to_vec());
    }
    file.kept()?;
    fs::read(directory.join(file.path())).ok()
}

/// Rung one of [`demote_to_budget`]: whole files become accounts of themselves,
/// biggest first, until `carried` is inside [`request_byte_cap`]. Answers with
/// what the request carries afterwards.
///
/// `order` is every index of `files`, biggest file first and ties by path;
/// nothing here reorders it, so the file the budget takes is the same one on
/// every machine.
///
/// Two outcomes are not a demotion at all. A file whose account comes back **no
/// shorter than the file** stays whole: prose that costs what the text costs is
/// a worse thing to send at the same price, and rung three is where it gives way
/// if the request still does not fit. A file whose account **cannot be made** —
/// not text, past the chunk ceiling, no usable answer — has no rung to step onto
/// and so falls to a name and a size now, with that cause reported in place of
/// any budget one.
fn demote_whole_files(
    passes: &mut Reducing<'_>,
    files: &mut [agent::File],
    order: &[usize],
    carried: u64,
    cap: u64,
    problems: &mut Vec<Problem>,
) -> u64 {
    let mut carried = carried;
    for &index in order {
        if carried <= cap {
            break;
        }
        // Only a file whose text is in the request has anything to trade here,
        // and only one that is really spending bytes is worth a pass: the
        // listed and the already described are rungs two and three's.
        let spent = file_bytes(&files[index]);
        if spent == 0 {
            continue;
        }
        let Some(bytes) = whole_bytes(passes.directory, &files[index]) else {
            continue;
        };
        let (path, size) = (files[index].path().to_owned(), files[index].size());

        match passes.skeleton_of(&path, &bytes) {
            Ok(skeleton) if byte_count(skeleton.len()) < spent => {
                carried = carried
                    .saturating_sub(spent)
                    .saturating_add(byte_count(skeleton.len()));
                files[index] = agent::File::elided(path, size, skeleton);
            }
            Ok(_) => {}
            Err(cause) => {
                // No row in the table, or nothing in the file the table would
                // drop. Its own first lines are still better than its name.
                if let Some(sample) = sampled(&files[index]) {
                    carried = carried
                        .saturating_sub(spent)
                        .saturating_add(file_bytes(&sample));
                    files[index] = sample;
                } else {
                    carried = carried.saturating_sub(spent);
                    files[index] = agent::File::omitted(path.clone(), size);
                }
                // Reported either way. A sample is better than a name and it is
                // still most of a file missing, and the entry is what says so.
                report(problems, passes.directory.join(path.clone()), cause);
            }
        }
    }
    carried
}

/// Rung two of [`demote_to_budget`]: the files gather's cliff already took,
/// back up to an account of themselves wherever one fits in what is left of
/// [`request_byte_cap`]. Answers with what the request carries afterwards.
///
/// Only the files [`Omission::OverBudget`] put on the problem list, and never
/// back to their own bytes — see [`demote_to_budget`] for why the ladder only
/// goes one way. A file the summarising has already declined once
/// ([`lift_over_cap`]) is not asked again, because it would be the same
/// passes for the same no.
///
/// A file that comes back described stops being a [`Problem`]; one the
/// filesystem now refuses, or the summarising declines, keeps its single entry
/// with the new cause in place of the budget's. The first account that does not
/// fit ends the rung: the file stays on its cliff, and no further passes are
/// spent finding out that the budget is still full.
fn lift_from_the_cliff(
    passes: &mut Reducing<'_>,
    files: &mut [agent::File],
    order: &[usize],
    carried: u64,
    cap: u64,
    problems: &mut Vec<Problem>,
) -> u64 {
    let mut carried = carried;
    for &index in order {
        if carried > cap {
            // Nothing would fit, so nothing is read and no pass is paid for: a
            // request already too big is no place to be adding prose.
            break;
        }
        if !files[index].is_omitted() {
            continue;
        }
        let on_disk = passes.directory.join(files[index].path());
        let Some(reported) = problems.iter().position(|problem| {
            matches!(problem.cause, Omission::OverBudget { .. }) && problem.path == on_disk
        }) else {
            continue;
        };

        // Read here, having been measured and given up by gather without ever
        // being opened. Cheap by construction: everything the cliff took was
        // under `PER_FILE_BYTE_CAP` to begin with.
        let bytes = match fs::read(&on_disk) {
            Ok(bytes) => bytes,
            // It was readable a moment ago and is not now. Whatever happened to
            // it, the honest cause is the filesystem's.
            Err(source) => {
                problems[reported].cause = Omission::Unreadable { source };
                continue;
            }
        };

        match passes.skeleton_of(files[index].path(), &bytes) {
            Ok(skeleton) => {
                let length = byte_count(skeleton.len());
                if carried.saturating_add(length) > cap {
                    break;
                }
                let (path, size) = (files[index].path().to_owned(), files[index].size());
                files[index] = agent::File::elided(path, size, skeleton);
                // Reduced, so no longer left out of anything.
                problems.remove(reported);
                carried = carried.saturating_add(length);
            }
            Err(cause) => problems[reported].cause = cause,
        }
    }
    carried
}

/// Rung three of [`demote_to_budget`]: whatever still carries bytes becomes a
/// name and a size, biggest first, while `carried` is over
/// [`request_byte_cap`].
///
/// The last rung, so it answers with nothing: what the request comes to after
/// this is the pass's business rather than any caller's.
///
/// [`trim_to_budget`]'s move, made last instead of first and over all three
/// states: the accounts rung one made, the accounts [`lift_over_cap`] made,
/// and the whole files rung one had no shorter account for. Reaching it at all
/// means the request does not fit with every account in it, which is why the
/// cause is [`Omission::OverBudget`] whatever the file was a moment ago.
///
/// No pass, no read, no agent: everything this needs is already in the request.
/// It can run out of files before it runs out of bytes — a child's document
/// never gives way — and a request still over the cap is the answer then.
fn list_over_budget(
    directory: &Path,
    files: &mut [agent::File],
    order: &[usize],
    carried: u64,
    cap: u64,
    problems: &mut Vec<Problem>,
) {
    let mut carried = carried;
    for &index in order {
        if carried <= cap {
            break;
        }
        let spent = file_bytes(&files[index]);
        if spent == 0 {
            // Already a name and a size: nothing left to give up, and the entry
            // saying why is already on the list.
            continue;
        }
        let (path, size) = (files[index].path().to_owned(), files[index].size());
        // The last rung, and it is a sample rather than a name: a file the
        // whole-request cap could not afford whole is still a file whose first
        // lines cost almost nothing. Only text that cannot be sampled at all
        // falls the rest of the way, and only that is a `Problem`.
        if let Some(sample) = sampled(&files[index]).filter(|s| file_bytes(s) < spent) {
            carried = carried
                .saturating_sub(spent)
                .saturating_add(file_bytes(&sample));
            files[index] = sample;
            report(
                problems,
                directory.join(path),
                Omission::OverBudget { size },
            );
        } else {
            files[index] = agent::File::omitted(path.clone(), size);
            carried = carried.saturating_sub(spent);
            report(
                problems,
                directory.join(path),
                Omission::OverBudget { size },
            );
        }
    }
}

/// Say `cause` about the file at `path`, as the one thing said about it.
///
/// The reporting rule of this module in one function: a file has at most one
/// [`Problem`], so a new cause for a file already on the list **replaces** the
/// one there rather than joining it. A reader is never told twice about one
/// file, and the entry that survives is the last and most specific reason its
/// contents did not reach the pass.
fn report(problems: &mut Vec<Problem>, path: PathBuf, cause: Omission) {
    match problems.iter().position(|problem| problem.path == path) {
        Some(index) => problems[index].cause = cause,
        None => problems.push(Problem { path, cause }),
    }
}

/// What it takes to reduce a file to its declarations: where the files are,
/// and where the names each one declares are recorded for the render.
///
/// It used to hold an [`Agent`] and an [`Observer`] too, because a file too big
/// to send was described by model passes over its chunks. It is not any more:
/// [`languages::skeleton`] lifts the file's own declaration lines out of it for
/// nothing, so there is no pass to run, nothing to announce and nothing to
/// cache.
struct Reducing<'a> {
    /// The directory being pacted, which every file's path is relative to.
    directory: &'a Path,
    /// Where the names each file declares are recorded for the render.
    described: &'a mut Described,
}

impl Reducing<'_> {
    /// `bytes` as the file's own declaration lines, and the names it declares
    /// recorded on the way past.
    ///
    /// Free, in every sense that matters: no model pass, no network, no cache,
    /// and the same answer on every machine. What comes back is text the file
    /// really contains, so a route naming a symbol found in it is anchored in
    /// code rather than in prose about code.
    ///
    /// # Errors
    ///
    /// [`Omission::NotText`] when the bytes are not UTF-8, and
    /// [`Omission::Unreducible`] when there is nothing to lift: an extension
    /// the language table has no row for, or a file whose declarations come to
    /// no less than the file. Either way the caller leaves it as a name and a
    /// size.
    fn skeleton_of(&mut self, path: &str, bytes: &[u8]) -> Result<String, Omission> {
        let size = byte_count(bytes.len());
        let text =
            std::str::from_utf8(bytes).map_err(|source| Omission::NotText { size, source })?;
        let on_disk = Path::new(path);
        let skeleton = languages::skeleton(on_disk, text).ok_or(Omission::Unreducible { size })?;

        let declared = languages::declared_names(on_disk, text);
        if !declared.is_empty() {
            self.described.declared.insert(path.to_owned(), declared);
        }
        Ok(skeleton.text)
    }
}

/// What one request is built from: the directory's own files, and its immediate
/// children's documents, each keyed by the relative path it will be named by.
///
/// [`BTreeMap`]s because the key order is the request's order, and the request's
/// order has to be the same on two machines that enumerate a directory
/// differently.
#[derive(Debug)]
struct Found {
    /// The files sitting directly in the directory, keyed by name. The
    /// directory's own `WARLOCK.md` is not among them: it reaches no pass.
    files: BTreeMap<String, PathBuf>,
    /// The `WARLOCK.md` of each immediate child that has one, keyed by the
    /// child directory's name.
    child_documents: BTreeMap<String, PathBuf>,
}

pub(crate) fn gather_request(
    prompt: impl Into<String>,
    directory: impl AsRef<Path>,
    cap: u64,
) -> Result<Gathered, Error> {
    let directory = directory.as_ref();
    let found = walk(directory)?;

    let mut problems = Vec::new();

    // Children first: their documents are part of the budget the files are
    // then fitted into, and they are the part that never gives way.
    let mut child_documents = Vec::new();
    for (child, path) in found.child_documents {
        match fs::read_to_string(&path) {
            Ok(text) => {
                child_documents.push(agent::ChildDocument::new(child, text));
            }
            // Including a document that could not be read is not an option —
            // there is no text — so it contributes nothing and says so.
            Err(source) => problems.push(Problem {
                path,
                cause: Omission::Unreadable { source },
            }),
        }
    }

    // Files in sorted order, each sent whole unless it alone is too big. The
    // size comes from the filesystem before anything is opened, so an enormous
    // file is never read into memory just to be dropped again.
    let mut files = Vec::new();
    let mut on_disk = Vec::new();
    for (relative, path) in found.files {
        let size = match fs::metadata(&path) {
            Ok(metadata) => metadata.len(),
            // No size means nothing true to list, so the file is left out
            // entirely rather than listed at a made-up length.
            Err(source) => {
                problems.push(Problem {
                    path,
                    cause: Omission::Unreadable { source },
                });
                continue;
            }
        };

        // The per-file cap, but never above the budget for the whole request.
        // The two used to disagree: 1 MiB against a 400 KB request meant a
        // 900 KB file passed this gate, was read whole, and then single-handedly
        // blew the budget by more than twice over — guaranteeing a demotion the
        // read had already been paid for. A gate that admits what the next gate
        // must reject is not a gate, so this one is clamped to it.
        let file = if size > PER_FILE_BYTE_CAP.min(cap) {
            problems.push(Problem {
                path: path.clone(),
                cause: Omission::TooLarge { size },
            });
            agent::File::omitted(relative, size)
        } else {
            match fs::read(&path) {
                Ok(bytes) => elided_or_whole(&path, relative, size, bytes),
                Err(source) => {
                    problems.push(Problem {
                        path: path.clone(),
                        cause: Omission::Unreadable { source },
                    });
                    agent::File::omitted(relative, size)
                }
            }
        };
        files.push(file);
        on_disk.push(path);
    }

    // Counted once, from what was actually gathered, rather than added up as
    // the loops went: what a file spends is a property of the file that ended
    // up in the request, not of the branch it came out of.
    let carried = carried_bytes(&files, &child_documents);
    trim_to_budget(&mut files, &on_disk, carried, cap, &mut problems);

    let request = agent::Request::new(prompt, directory)
        .with_files(files)
        .with_child_documents(child_documents);

    Ok(Gathered { request, problems })
}

/// The file extensions warlock reads as prose, and never hands to a pass.
///
/// Markdown is the one kind of file in a repository whose claims warlock has
/// nothing to check against. Everything else in a request is evidence — the
/// bytes are what they are — but a `README.md` is somebody's writing *about*
/// the code, which is exactly the class of input the document schema exists to
/// keep out of a `WARLOCK.md`.
///
/// # What made this necessary
///
/// A route's symbol is verified by looking for it in the target's text
/// ([`document::accept`](crate::document::accept)), and that check cannot tell a declaration from
/// a mention. Measured on this repository: `crates/warlock-tui`'s document
/// routed "what keys the TUI binds" to `README.md` `action_for`, which
/// verified against one English sentence — *"Every binding is in
/// `action_for`"* — while the function itself sits in `src/input.rs`, a
/// directory that pass never saw. Five of the engine crate's eight routes went
/// the same way, naming real types against prose that merely mentions them. A
/// chatty readme can manufacture a verified route to the wrong place for any
/// symbol it happens to name, and the parent that should have routed *down* to
/// its `src/` child routes sideways into prose instead.
///
/// So prose is not evidence and does not travel. What is left in a parent
/// directory of a workspace is its manifests and its children's documents,
/// which is what a router should have been reading in the first place.
///
/// # What this deliberately does not touch
///
/// * **Hashing.** [`subtree_hash`](crate::subtree_hash) still digests every markdown file, so
///   editing a `README.md` still turns its directory yellow. The digest is
///   warlock's record that *something moved*, and a child's `WARLOCK.md` being
///   part of its parent's digest is what makes the two-phase pact work at all.
///   Narrowing it is a separate decision with its own consequences.
/// * **A child's `WARLOCK.md`.** Those are gathered by the other branch of
///   [`walk`], as [`agent::ChildDocument`]s, and are the whole of how a
///   directory learns what is below it. They are prose too, and they are the
///   one prose warlock wrote itself, to a shape it checked.
/// * **[`view_file`](crate::view_file).** The `v` key opens any file a person points it at. A
///   person asking to read a readme is not a pass being handed one.
const PROSE_EXTENSIONS: &[&str] = &["md", "markdown", "mdx"];

/// Whether `path` is prose by [`PROSE_EXTENSIONS`], matched case-insensitively
/// so a `README.MD` is as excluded as a `readme.md`.
fn is_prose(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            PROSE_EXTENSIONS
                .iter()
                .any(|prose| extension.eq_ignore_ascii_case(prose))
        })
}

/// Everything at or just below `dir` that a request can be built from.
///
/// One pass, [`WALK_DEPTH`] deep, under the ignore rules the rest of the crate
/// walks by. Directories are not collected: a child directory matters here only
/// as the place a `WARLOCK.md` was found, and a child with none simply never
/// appears.
/// Domain separation for the carry digest, on the same terms as
/// [`HASH_CONTEXT`](crate::hash): this digest answers a different question from
/// the subtree hash, and the two must not be capable of colliding.
const CARRY_HASH_CONTEXT: &str = "warlock carry hash v1 2026-09-06";

/// A digest over everything that has to be unchanged for `directory`'s recorded
/// grant to stand without a new pass: what a pass would be shown — its own
/// non-prose files and each immediate child's `WARLOCK.md` — **and the document
/// that grant was earned for**.
///
/// # Why this is not [`subtree_hash`](crate::subtree_hash)
///
/// The subtree hash asks whether anything at or below a directory moved. That
/// is the right trigger for *this document is owed a look*, and it stays the
/// trigger. It is the wrong question for *would a pass write anything
/// different*, because a request holds neither the directory's readme nor a
/// single byte from below its immediate children: both can move, restale the
/// directory, and leave the pass with the identical input it had last time.
///
/// This digest is taken over the request's own inputs instead, so two runs
/// that would be shown the same thing agree — and [`pact`](crate::pact) can
/// decline to pay for the second. It is the early cutoff a build system does
/// with the hash of a target's output: the trigger stays mechanical and
/// nobody's opinion enters, and what is skipped is the work that provably had
/// nothing new to read.
///
/// # Why the directory's own document is in here, when no pass ever sees it
///
/// Because a cutoff carries that document forward and grants it, and a grant
/// says a model pass produced what is on disk. A `WARLOCK.md` is prose, so it
/// reaches no request and moves no other digest here — and a person who edits
/// one by hand would otherwise slip past the cutoff and have their own
/// sentences stamped granted, with no pass having read anything. That is the
/// false green the ledger exists to prevent, and it is the reason the document
/// is a third section below rather than an omission worth explaining.
///
/// So an edited document does not match, and the pass runs: the only road back
/// to fresh is still a pass, exactly as the repository's own `CLAUDE.md` says.
///
/// # It cannot drift from the request
///
/// The walk is [`walk`], the same call [`gather_request`] makes, with the same
/// depth, the same ignore rules and the same prose exclusion. A file the rules
/// removed is absent from both; a readme is in neither. There is no second
/// list to keep in step.
///
/// # Raw bytes, not the fitted form
///
/// A file is digested as it sits on disk, before elision. So editing a test
/// body that [`languages`](crate::languages) would have dropped anyway still
/// counts as a change, and still costs a pass. That is the conservative
/// direction on purpose: this digest can charge for a pass that was not
/// strictly owed, and can never skip one that was. A false green is the one
/// outcome that has to be earned, and no saving is worth reaching it.
///
/// # `None` is safe, which is why it is `None` and not an error
///
/// A directory that cannot be walked, or a file that cannot be read, answers
/// `None`, and the only thing a caller may conclude from `None` is *run the
/// pass*. That is the opposite of [`subtree_hash`](crate::subtree_hash), where
/// an unreadable file has to be a hard error because a digest that quietly
/// skipped it would match the digest of a subtree that never had it — a false
/// green. Nothing of the sort is reachable from here: every way this function
/// fails costs a pass that might not have been owed, and none of them can skip
/// one that was. So the failure is worth no error type of its own.
pub(crate) fn carry_hash(directory: &Path) -> Option<String> {
    let found = walk(directory).ok()?;
    let mut hasher = blake3::Hasher::new_derive_key(CARRY_HASH_CONTEXT);

    // Two sections, each length-prefixed and each announced by its count, so no
    // arrangement of one can be read as the other: a directory holding a file
    // named `x` and one holding a child `x` with a document are different
    // inputs and must be different digests.
    hasher.update(&length(found.files.len()).to_le_bytes());
    for (relative, path) in &found.files {
        hasher.update(&length(relative.len()).to_le_bytes());
        hasher.update(relative.as_bytes());
        let bytes = fs::read(path).ok()?;
        hasher.update(&length(bytes.len()).to_le_bytes());
        hasher.update(&bytes);
    }

    hasher.update(&length(found.child_documents.len()).to_le_bytes());
    for (child, path) in &found.child_documents {
        hasher.update(&length(child.len()).to_le_bytes());
        hasher.update(child.as_bytes());
        let bytes = fs::read(path).ok()?;
        hasher.update(&length(bytes.len()).to_le_bytes());
        hasher.update(&bytes);
    }

    // The third section: the document itself, absent and empty told apart by
    // the marker byte, so a directory with no document cannot digest as one
    // holding a document of nothing.
    match fs::read(directory.join(DOCUMENT_FILE)) {
        Ok(bytes) => {
            hasher.update(&[1]);
            hasher.update(&length(bytes.len()).to_le_bytes());
            hasher.update(&bytes);
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            hasher.update(&[0]);
        }
        Err(_) => return None,
    }

    Some(hasher.finalize().to_hex().to_string())
}

fn walk(dir: &Path) -> Result<Found, Error> {
    let walker = WalkBuilder::new(dir)
        // The same three rules as `load` and `hash`, for the same reasons: a
        // symlinked cycle has to terminate, a fixture with a `.gitignore` and
        // no `.git` still has to be ignored properly, and `.warlock/` is
        // Warlock's own bookkeeping rather than content of the module.
        .follow_links(false)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != OsStr::new(MANIFEST_DIR))
        // The repository's own exclusions, on the same terms as everywhere
        // else: a file the rules removed reaches no request in any of its three
        // states — not whole, not as a name and a size, not as declarations —
        // because it is never found here to be put in one.
        .add_custom_ignore_filename(ignores::FILENAME)
        .max_depth(Some(WALK_DEPTH))
        .build();

    let mut found = Found {
        files: BTreeMap::new(),
        child_documents: BTreeMap::new(),
    };
    for entry in walker {
        let entry = entry.map_err(|source| Error::Walk {
            directory: dir.to_path_buf(),
            source,
        })?;
        // Rules that could not be read are the failure they are rather than a
        // verdict of "nothing is excluded"; see `pactable_directories`.
        if let Some(source) = entry.error() {
            return Err(Error::Walk {
                directory: dir.to_path_buf(),
                source: source.clone(),
            });
        }
        let depth = entry.depth();
        // Regular files only. With `follow_links(false)` a symlink reports as a
        // symlink, so it is neither descended into nor listed as whatever it
        // points at.
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.into_path();

        if depth == 1 {
            // The directory's own files, minus everything that is prose — see
            // `PROSE_EXTENSIONS` for why a readme is not evidence about the
            // code beside it. That covers the directory's own `WARLOCK.md`
            // along with the rest: it is the previous pass's claim about this
            // directory rather than part of the directory's evidence about
            // itself, and `agent::Request` carries the argument for why no
            // pass is shown its predecessor.
            if !is_prose(&path) {
                found.files.insert(relative(dir, &path)?, path);
            }
        } else if depth == WALK_DEPTH && path.file_name() == Some(OsStr::new(DOCUMENT_FILE)) {
            // A child's document, filed under the child directory rather than
            // under the document: the name is the same for every child, and the
            // directory is what a reader needs to place it.
            let Some(child) = path.parent().map(Path::to_path_buf) else {
                continue;
            };
            found.child_documents.insert(relative(dir, &child)?, path);
        }
    }
    Ok(found)
}

/// `path` named relative to `dir`, in the manifest's forward-slash spelling.
fn relative(dir: &Path, path: &Path) -> Result<String, Error> {
    to_manifest_path(dir, path).map_err(|source| Error::Path {
        directory: dir.to_path_buf(),
        path: path.to_path_buf(),
        source: Box::new(source),
    })
}

/// A byte count as the budget counts it.
///
/// Saturating rather than fallible, exactly as in [`hash`](crate::hash):
/// `usize` is at most 64 bits on every target this builds for, so the clamp is
/// unreachable, and a budget is no place to introduce a panic over a case that
/// cannot happen.
pub(crate) fn byte_count(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// The first [`SAMPLE_LINE_CAP`] lines of `text`, or the first
/// [`SAMPLE_BYTE_CAP`] bytes of it, whichever comes first, with a line saying
/// what was left off. `None` where the whole of it already fits.
///
/// # Why a file is never reduced below its own bytes
///
/// The floor under every rung of the ladder used to be a name and a size. That
/// is honest and it is nearly useless: a document could say a directory holds a
/// 1.5 MB `inventory.json` and nothing whatever about what is in it, and the
/// pass was not even asked to try — which is right, because a line written from
/// a filename is a guess. But the choice was never between a guess and a name.
/// The first forty lines of that file say it holds `{id, sku, qty}` records,
/// and those lines are the file's own text: evidence, free, and exactly the
/// kind of thing a map exists to carry.
///
/// So no readable file is reduced below a sample of itself. The marker is what
/// makes that honest rather than a truncation — the same device
/// [`languages`](crate::languages) uses for an elided body, and the reason a
/// reader can tell a part of a file from the whole of one.
///
/// Bytes that are not UTF-8 have no lines to cut on and no honest sample, and
/// they are the one thing still reduced to a name and a size.
fn sample_of(text: &str) -> Option<String> {
    let mut head = String::new();
    let mut taken = 0usize;
    for line in text.lines().take(SAMPLE_LINE_CAP) {
        // One over-long line — a minified bundle is one line — is cut at a
        // character boundary rather than dropped, so a sample of it is still a
        // sample rather than nothing.
        let cap = usize::try_from(SAMPLE_BYTE_CAP).unwrap_or(usize::MAX);
        let room = cap.saturating_sub(head.len());
        if room == 0 {
            break;
        }
        let cut = if line.len() <= room {
            line
        } else {
            let mut end = room;
            while end > 0 && !line.is_char_boundary(end) {
                end -= 1;
            }
            &line[..end]
        };
        head.push_str(cut);
        head.push('\n');
        taken += 1;
        if head.len() >= cap {
            break;
        }
    }

    let total = text.lines().count();
    if taken >= total && head.len() >= text.len() {
        // The whole file fits, so there is no sample to take: the caller keeps
        // what it has rather than swapping it for a copy of itself.
        return None;
    }
    if head.is_empty() {
        return None;
    }
    // What was left off, said in whichever unit is true. Lines, where lines
    // were dropped; bytes, where the file is one long line and what happened to
    // it was a cut rather than a drop. Saying "0 further lines" about a
    // minified bundle would be the truncation this marker exists to rule out.
    let left = total.saturating_sub(taken);
    if left > 0 {
        let _ = write!(head, "… {left} further lines not shown …");
    } else {
        let bytes = text.len().saturating_sub(head.len());
        let _ = write!(head, "… {bytes} further bytes not shown …");
    }
    Some(head)
}

/// `file` reduced to a sample of its own text, or `None` where it has no text
/// to sample — bytes that are not UTF-8, or a file already down to a name.
fn sampled(file: &agent::File) -> Option<agent::File> {
    let owned;
    let text = if let Some(bytes) = file.bytes() {
        owned = std::str::from_utf8(bytes).ok()?;
        owned
    } else {
        file.kept()?
    };
    let head = sample_of(text)?;
    Some(agent::File::elided(
        file.path().to_owned(),
        file.size(),
        head,
    ))
}

/// How much of [`request_byte_cap`] one file spends: what the request carries
/// for it, never what it weighs on disk.
///
/// Three states, three answers, and only one of them is the file's size:
///
/// * A file **sent whole** spends its bytes, which is also its size.
/// * A file **listed** by name and size spends nothing. Its size is still in
///   the request as a fact about the directory, but no contents travel with it,
///   and charging the budget for bytes nobody sent is how a directory holding
///   one lockfile ends up sending nothing else.
/// * A file **reduced** to its declaration lines spends the length of those
///   lines — exactly the way a child's document is counted, and for the same
///   reason: that text is what travels. Its on-disk size is never what is
///   counted here; a four-megabyte file whose declarations come to three
///   hundred bytes costs three hundred bytes.
fn file_bytes(file: &agent::File) -> u64 {
    if let Some(bytes) = file.bytes() {
        return byte_count(bytes.len());
    }
    // The file's own surviving lines: real text, and what it costs is what is
    // there rather than what is on disk.
    if let Some(kept) = file.kept() {
        return byte_count(kept.len());
    }
    // Prose about the file, counted like the child document it resembles.
    if let Some(summary) = file.summary() {
        return byte_count(summary.len());
    }
    // A name and a size: nothing of it is in the request to pay for.
    0
}

/// Everything a request would carry, counted the way the budget counts it: the
/// files by [`file_bytes`], plus every child document's text.
///
/// The child documents are counted and never trimmed. They are what a
/// directory knows that its files do not say, they are small next to source,
/// and [`trim_to_budget`] only ever gives up files — so counting them here is
/// what keeps the files' share honest rather than what puts them at risk.
///
/// Saturating throughout, like [`byte_count`]: a budget is no place to panic
/// over a total that cannot happen.
pub(crate) fn carried_bytes(
    files: &[agent::File],
    child_documents: &[agent::ChildDocument],
) -> u64 {
    let mut carried: u64 = 0;
    for file in files {
        carried = carried.saturating_add(file_bytes(file));
    }
    for child in child_documents {
        carried = carried.saturating_add(byte_count(child.text().len()));
    }
    carried
}

/// What gathering produced: the request, and everything left out of it.
///
/// A plain pair for the same reason as [`Loaded`](crate::Loaded): the request is
/// the thing to send, the problems are the thing to report once, and they have
/// different lifetimes. Nothing on [`agent::Request`](crate::agent::Request) records that a file was
/// omitted beyond the file's own missing bytes — a request is what a model sees,
/// not a log of how it was built.
#[derive(Debug)]
pub(crate) struct Gathered {
    /// The request, ready to hand to an [`Agent`](crate::Agent).
    pub(crate) request: agent::Request,
    /// Every file the caps left out, in the order they were given up: the
    /// per-file cases in path order, then the whole-request ones largest first.
    /// Empty is the normal case.
    pub(crate) problems: Vec<Problem>,
}

/// One file left out of a request, and why.
///
/// The shape [`load::Problem`](crate::load::Problem) established — a path, a cause,
/// one line of [`Display`](fmt::Display) — because it is the same kind of thing:
/// something that went wrong without being worth failing over, said once and in
/// full. A caller that ignores these gets a pact built on slightly less than the
/// whole directory, which is safe, just unexplained.
///
/// One thing this is deliberately not: a file that reached the request as its
/// declaration lines is **not** a `Problem`. It was read in full and real lines
/// of it are in the request, which is the opposite of being left out. What
/// stays a `Problem` is every fallback from that — the file could not be read,
/// or the language table had no way to reduce it — since each of those leaves
/// the pass with a name and a size and nothing more.
#[derive(Debug)]
pub struct Problem {
    /// The file that was left out, as it sits on disk.
    pub path: PathBuf,
    /// Why it was left out.
    pub cause: Omission,
}

/// Why one file's contents are not in a request.
///
/// Separate answers rather than one "skipped", because they call for different
/// reactions, and they fall into three groups:
///
/// * **The two byte caps.** [`Omission::TooLarge`] and
///   [`Omission::OverBudget`] call for nothing at all — a huge generated file
///   is working as intended — though a directory that keeps tripping the
///   whole-request cap is one worth splitting up. `OverBudget` in particular is
///   a file the request had no room for even as its declaration lines, since
///   skeletons are what the cap takes first and names only after.
/// * **The filesystem.** [`Omission::Unreadable`] calls for a look at the disk,
///   because a file Warlock cannot read is a file nobody's tooling can read.
///
/// There used to be a third group — the ways summarising an over-cap file could
/// fail, when reducing one meant a map-reduce of model passes. Reducing is a
/// table lookup now, so the only ways it does not happen are an extension with
/// no row and a file that is already all declarations, and both of those leave
/// the file exactly where it was rather than anywhere new to report.
///
/// Every variant is a file the pass did not see the whole of. What it left in
/// the request is no longer the same for all of them, and that is the point of
/// [`sample_of`]: a readable file gives up its middle, not its text, so what
/// stands in for it is its own first lines under a marker saying what was left
/// off. Only bytes that are not UTF-8 fall the whole way to a name and a size,
/// because there is no honest sample of them to take.
///
/// They stay reported all the same. Most of a sampled file really is missing,
/// and a reader deciding whether to go and look wants to know that — the entry
/// now sits beside a sample rather than instead of one. A file that reached the
/// pass as its declaration lines has no variant here and never will, because
/// nothing structural was left out of it — see [`Problem`].
#[derive(Debug)]
#[non_exhaustive]
pub enum Omission {
    /// The file is larger by itself than [`PER_FILE_BYTE_CAP`], so it was
    /// listed rather than sent. Nothing else in the directory is affected.
    TooLarge {
        /// Its size in bytes, which is what the request carries in place of it.
        size: u64,
    },
    /// The file fitted [`PER_FILE_BYTE_CAP`], but the request had no room even
    /// for an account of it: the directory as a whole was over
    /// [`request_byte_cap`] and this was one of the largest files in it.
    ///
    /// The last rung rather than the first. [`gather_request`] never opens an
    /// over-cap file, so this is the only move it has and it makes it there and
    /// then; but by the time a *pact* hands this back, the budget has been met
    /// the cheap way as far as it will go — the largest files demoted to their
    /// declaration lines — and this file still had nowhere to stand: either the
    /// request was over the cap with every skeleton in it, or the budget was
    /// already full when its turn came. See [`demote_to_budget`]. A file the
    /// whole-request cap cost its bytes but not its skeleton is in the request
    /// and is no [`Problem`] at all.
    OverBudget {
        /// Its size in bytes, which is what the request carries in place of it.
        size: u64,
    },
    /// The file could not be read at all. Not a budget decision and never
    /// counted as one: this is the filesystem saying no.
    Unreadable {
        /// What the filesystem said.
        source: std::io::Error,
    },
    /// The file is over a byte cap and its bytes are not valid UTF-8, so there
    /// is nothing to read declarations out of.
    ///
    /// Nothing is wrong with the file: a checked-in PNG, a fixture of random
    /// bytes or a compiled artefact is doing exactly what it is for. Separate
    /// from [`Omission::TooLarge`] because it is a different fact about the
    /// file, and the document says which.
    NotText {
        /// Its size in bytes.
        size: u64,
        /// Where the bytes stopped being text.
        source: Utf8Error,
    },
    /// The file is over a byte cap and there is nothing of it worth lifting:
    /// its extension has no row in the language table, or its declaration
    /// lines come to no less than the whole file.
    ///
    /// A lockfile, a minified bundle, a CSV. Warlock reduces source it
    /// recognises and declines to guess at anything else, so the file keeps the
    /// floor every over-cap file has: a name and a size, with the reason said
    /// out loud.
    Unreducible {
        /// Its size in bytes.
        size: u64,
    },
}

impl fmt::Display for Omission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size } => write!(
                f,
                "{size} bytes is over the {PER_FILE_BYTE_CAP}-byte per-file cap, so it is listed \
                 by name and size"
            ),
            // No number named, unlike the per-file cap above: the request
            // budget is derived from the window the pass will be read in
            // ([`request_byte_cap`]), so there is no one figure that is true of
            // every run and quoting a stale one would be worse than quoting
            // none.
            Self::OverBudget { size } => write!(
                f,
                "the directory is over the request budget, so this file of {size} bytes is \
                 listed by name and size"
            ),
            Self::Unreadable { source } => write!(f, "it could not be read: {source}"),
            Self::NotText { size, source } => write!(
                f,
                "its {size} bytes are not text ({source}), so there is nothing to reduce and it \
                 is listed by name and size"
            ),
            Self::Unreducible { size } => write!(
                f,
                "there is nothing of its {size} bytes to lift — warlock has no reader for this \
                 kind of file — so it is listed by name and size"
            ),
        }
    }
}

impl std::error::Error for Omission {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unreadable { source } => Some(source),
            Self::NotText { source, .. } => Some(source),
            Self::TooLarge { .. } | Self::OverBudget { .. } | Self::Unreducible { .. } => None,
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` was left out of the pact request: {}",
            self.path.display(),
            self.cause
        )
    }
}

impl std::error::Error for Problem {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        BYTES_PER_TOKEN_DENOMINATOR, BYTES_PER_TOKEN_NUMERATOR, Gathered, MINIMUM_REQUEST_BYTES,
        Omission, PER_FILE_BYTE_CAP, Problem, REQUEST_BYTE_CAP, RESERVED_TOKENS, SAMPLE_BYTE_CAP,
        carried_bytes, gather_request, request_byte_cap,
    };

    use crate::agent;
    use crate::pact::DOCUMENT_FILE;
    /// Write `contents` at `dir/name`, creating whatever directories it needs.
    fn write(dir: &Path, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().expect("a file has a parent")).expect("creates parents");
        fs::write(&path, contents).expect("writes a file");
        path
    }

    /// `size` bytes of something, cheap to make and impossible to confuse with
    /// a fixture's real text.
    fn filler(size: u64) -> Vec<u8> {
        vec![b'x'; usize::try_from(size).expect("a test file fits in memory")]
    }

    /// `percent` of [`request_byte_cap`], in bytes.
    ///
    /// Every fixture below that is about the budget biting is written in these
    /// rather than in kibibytes, because what those tests are about is a size
    /// *relative to the cap* — three files at 39% apiece are over budget and two
    /// are not, whatever the cap happens to be this year. They used to be
    /// absolute, against a 256 KiB cap, and raising that cap turned a directory
    /// that was deliberately over budget into one that fit, which is a fixture
    /// silently ceasing to test what it was written for rather than a test
    /// failing honestly.
    fn share(percent: u64) -> u64 {
        REQUEST_BYTE_CAP * percent / 100
    }

    /// A request budget nothing in a fixture can reach, so that the per-file
    /// cap is the only thing able to leave anything out.
    ///
    /// Four times [`PER_FILE_BYTE_CAP`] rather than a number typed in: what
    /// these fixtures probe is the per-file boundary, and a request budget that
    /// could also demote a file would make a failure here ambiguous about which
    /// cap caused it. Derived from the other cap so it cannot drift under it.
    const AMPLE_CAP: u64 = PER_FILE_BYTE_CAP * 4;

    /// The request for `dir`, insisting nothing was left out of it.
    ///
    /// Most of these fixtures are small enough to send whole, so an empty
    /// problem list is part of what they assert: a gather that quietly started
    /// dropping files would fail here rather than pass unnoticed. It gathers
    /// against [`AMPLE_CAP`] for that reason — the only cap allowed to drop
    /// anything here is the per-file one.
    fn request_for(dir: &Path) -> agent::Request {
        let Gathered { request, problems } =
            gather_request("summarise", dir, AMPLE_CAP).expect("gathers");
        assert!(problems.is_empty(), "{problems:?}");
        request
    }

    #[test]
    fn a_source_file_reaches_the_pass_with_its_test_bodies_elided() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(
            dir.path(),
            "scope.rs",
            "\
pub fn covering() -> u32 {
    7
}

#[cfg(test)]
mod tests {
    #[test]
    fn refuses_when_scope_closed() {
        let a = super::covering();
        let b = super::covering();
        assert_eq!(a, 7);
        assert_eq!(b, 7);
        assert_eq!(a, b);
    }
}
",
        );
        let request = request_for(dir.path());
        let file = file(&request, "scope.rs");

        let kept = file.kept().expect("a Rust test module is elided");
        assert!(
            kept.contains("pub fn covering()"),
            "the code a reader needs stays: {kept}"
        );
        assert!(
            kept.contains("fn refuses_when_scope_closed()"),
            "and so does the test's name, which is a sentence about behaviour: {kept}"
        );
        assert!(
            !kept.contains("assert_eq!(a, b)"),
            "the body is what is given up: {kept}"
        );
        assert_eq!(
            file.bytes(),
            None,
            "what survives elision is not the file's bytes and never answers as them"
        );
        assert_eq!(
            file.summary(),
            None,
            "nor is it prose about the file: it is the file's own lines"
        );
    }

    #[test]
    fn an_elided_file_still_reports_its_size_on_disk() {
        // The distinction the whole ladder rests on: how big a file is and how
        // much of it was sent are two facts, and a document that says the
        // second is a document that understates the directory.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let source = "\
pub fn work() {}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let one = 1;
        let two = 2;
        assert_eq!(one + two, 3);
    }
}
";
        write(dir.path(), "work.rs", source);
        let request = request_for(dir.path());
        let file = file(&request, "work.rs");

        assert_eq!(
            file.size(),
            source.len() as u64,
            "the size is the file's, not the elision's"
        );
        assert!(
            file.kept().expect("elided").len() < source.len(),
            "and the elision really is smaller"
        );
    }

    #[test]
    fn a_file_in_a_language_with_no_row_is_sent_exactly_as_it_is() {
        // The property that lets the table grow one language at a time: an
        // extension nobody has described is never guessed at.
        let dir = tempfile::tempdir().expect("a temporary directory");
        let source = "section .text\nglobal _start\n_start:\n    mov eax, 1\n";
        write(dir.path(), "boot.asm", source);
        let request = request_for(dir.path());
        let file = file(&request, "boot.asm");

        assert_eq!(
            file.bytes(),
            Some(source.as_bytes()),
            "an unknown language is sent whole and untouched"
        );
        assert_eq!(file.kept(), None, "and nothing claims to have elided it");
    }

    #[test]
    fn the_request_budget_follows_the_window_it_will_be_read_in() {
        // The defect this replaced: a fixed 2 MiB cap sat above every window it
        // was meant to respect, so nothing was ever demoted and the overflow
        // was left to whatever the transport does about it.
        let small = request_byte_cap(200_000);
        let large = request_byte_cap(1_000_000);
        assert!(
            small < large,
            "a bigger window buys a bigger request: {small} vs {large}"
        );
        assert!(
            small < 200_000 * BYTES_PER_TOKEN_NUMERATOR / BYTES_PER_TOKEN_DENOMINATOR,
            "and the prompt, the children's documents and the answer are kept back"
        );
    }

    #[test]
    fn trimming_an_elided_file_credits_the_budget_only_what_it_carried() {
        // The bug this pins: `trim_to_budget` demoted a file and then credited
        // the budget with the file's size *on disk*. That is right for a file
        // sent whole and wrong for an elided one, which is carried as its
        // surviving lines and reported as its full size — so the loop
        // over-counted what it had recovered, stopped early, and handed back a
        // request still over the cap while believing it had met it. Silently
        // over the cap is the one outcome the whole ladder exists to prevent.
        let dir = tempfile::tempdir().expect("a temporary directory");
        // Each file is real code that survives elision plus a test module that
        // does not, so after eliding they are still collectively over the cap
        // and trimming has to run — which is the only situation in which the
        // miscounting could bite.
        for name in ["a.rs", "b.rs", "c.rs", "d.rs"] {
            let code = (0..400).fold(String::new(), |mut code, n| {
                use std::fmt::Write as _;
                let _ = write!(code, "pub fn work_{n}() -> u32 {{\n    {n}\n}}\n\n");
                code
            });
            let body = "        let filler = 1;\n".repeat(2_000);
            write(
                dir.path(),
                name,
                format!(
                    "{code}#[cfg(test)]\nmod tests {{\n    #[test]\n    \
                     fn it_works() {{\n{body}    }}\n}}\n"
                ),
            );
        }

        let cap = 40_000;
        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), cap).expect("gathers");

        let carried = carried_bytes(request.files(), request.child_documents());
        assert!(
            carried <= cap,
            "the request really is inside the cap it reports meeting: {carried} > {cap}"
        );
        assert!(
            !problems.is_empty(),
            "and it said out loud what it gave up to get there"
        );
    }

    #[test]
    fn a_file_whose_account_would_cost_more_than_it_frees_is_not_given_up() {
        // Naming a file frees what it was carrying and commits the lift to
        // buying an account back, so for a small file the trade loses before it
        // starts. It also used to be how the biggest files in a directory ended
        // as bare names: the trim stopped the instant it was inside the cap,
        // leaving the lift nothing to spend, and `warlock-tui/src` reached its
        // pass with `app.rs`, `ui.rs`, `pacting.rs` and `claude.rs` as names
        // alone. Charging each cliff for the account it will want is what fixed
        // that, and this is the other end of the same rule.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "small.txt", filler(512));
        write(dir.path(), "big.txt", filler(200_000));

        let cap = 100_000;
        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), cap).expect("gathers");

        assert!(
            file(&request, "big.txt").is_omitted(),
            "the file whose account is worth buying is given up",
        );
        assert!(
            !file(&request, "small.txt").is_omitted(),
            "the file whose account would cost more than it frees is kept whole",
        );
        assert_eq!(
            problems.len(),
            1,
            "and only the real trade is reported: {problems:?}"
        );
    }

    #[test]
    fn the_trim_leaves_room_for_the_accounts_the_lift_will_buy() {
        // The defect this pins, measured on `warlock-tui/src`: the trim met the
        // cap exactly, so 17KB of headroom stood against eleven cliffed files
        // and seven of them stayed bare names all the way to the pass. A
        // document written about a file nobody could read is a document written
        // by guessing.
        let dir = tempfile::tempdir().expect("a temporary directory");
        for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
            write(dir.path(), name, filler(60_000));
        }

        let cap = 100_000;
        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), cap).expect("gathers");

        let carried = carried_bytes(request.files(), request.child_documents());
        let cliffed = problems.len() as u64;
        assert!(cliffed > 0, "the fixture is over the cap");
        assert!(
            cap - carried >= cliffed * SAMPLE_BYTE_CAP,
            "every cliffed file has room left for an account of it: {} spare for \
             {cliffed} files",
            cap - carried,
        );
    }

    #[test]
    fn the_budget_fits_the_window_at_the_measured_density_of_real_code() {
        // The arithmetic nobody did the first time, written down so a future
        // tightening of these constants has to argue with a measurement.
        //
        // Measured by sending two files of different sizes through the model
        // and dividing the difference in reported input tokens by the
        // difference in bytes: Rust 2.62 bytes per token, TypeScript 2.60. Also
        // measured, and the item most easily forgotten: about 16,700 tokens of
        // the agent's own system prompt and tool definitions sit in the window
        // before this request contributes anything.
        const MEASURED_BYTES_PER_TOKEN: u64 = 26; // tenths, the denser of the two
        const AGENT_OVERHEAD_TOKENS: u64 = 16_700;
        const WINDOW: u64 = 200_000;

        let cap = request_byte_cap(WINDOW);
        let content_tokens = cap * 10 / MEASURED_BYTES_PER_TOKEN;
        let used = content_tokens + AGENT_OVERHEAD_TOKENS;

        assert!(
            used < WINDOW,
            "a full request has to fit the window it is read in: {cap} bytes is \
             {content_tokens} tokens of code, plus {AGENT_OVERHEAD_TOKENS} of \
             agent overhead, against a {WINDOW}-token window",
        );
        // And enough left for the answer, which is the other thing in there.
        assert!(
            WINDOW - used >= 12_000,
            "only {} tokens left for the document itself",
            WINDOW - used,
        );
    }

    #[test]
    fn an_implausible_window_still_yields_a_usable_budget() {
        // No budget may fail a fit, this module's oldest rule: an agent that
        // reports a window smaller than the room reserved gets a poor request
        // rather than an impossible one.
        assert_eq!(request_byte_cap(0), MINIMUM_REQUEST_BYTES);
        assert_eq!(request_byte_cap(RESERVED_TOKENS), MINIMUM_REQUEST_BYTES);
    }

    /// The paths of a request's files, in the order it carries them.
    fn file_paths(request: &agent::Request) -> Vec<&str> {
        request.files().iter().map(agent::File::path).collect()
    }

    /// The file a request carries at `path`.
    fn file<'a>(request: &'a agent::Request, path: &str) -> &'a agent::File {
        request
            .files()
            .iter()
            .find(|file| file.path() == path)
            .unwrap_or_else(|| panic!("`{path}` is in the request: {:?}", file_paths(request)))
    }

    /// How many bytes a request actually carries: the files sent whole, the
    /// summaries of the files described, and the children's documents.
    ///
    /// Written from the public accessors rather than by calling
    /// [`super::carried_bytes`], so the module's own accounting has something
    /// independent to agree with. A listed file contributes nothing — its size
    /// is a fact about the directory, not bytes in the request.
    fn carried(request: &agent::Request) -> u64 {
        let files: u64 = request
            .files()
            .iter()
            .map(|file| {
                let bytes = file.bytes().map_or(0, <[u8]>::len);
                let summary = file.summary().map_or(0, str::len);
                (bytes + summary) as u64
            })
            .sum();
        let children: u64 = request
            .child_documents()
            .iter()
            .map(|child| child.text().len() as u64)
            .sum();
        files + children
    }

    /// A fat directory: five files that come to nearly twice the request cap
    /// between them, named so that alphabetical order is the reverse of size
    /// order — an operation that gave files up in path order would fail on it.
    ///
    /// Written in [`share`]s of the cap, for the reason given there. The
    /// thirty-one-to-forty-seven spread leaves the two smallest fitting and the
    /// third not.
    const FAT_SHARES: [(&str, u64); 5] = [
        ("a.bin", 31),
        ("b.bin", 35),
        ("c.bin", 39),
        ("d.bin", 43),
        ("e.bin", 47),
    ];

    /// The files of [`FAT_SHARES`], with their sizes, in the same order.
    fn fat() -> [(&'static str, u64); 5] {
        FAT_SHARES.map(|(name, percent)| (name, share(percent)))
    }

    #[test]
    fn a_directory_sends_its_own_files_and_its_children_summarise_themselves() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "Cargo.toml", "[package]\n");
        write(dir.path(), "build.rs", "fn main() {}\n");
        write(dir.path(), "src/WARLOCK.md", "# src\n\nThe code.\n");
        write(dir.path(), "src/lib.rs", "//! Core engine.\n");
        write(dir.path(), "src/inner/lib.rs", "//! Deeper still.\n");
        write(dir.path(), "src/inner/WARLOCK.md", "# inner\n");
        write(dir.path(), "tests/it.rs", "#[test] fn works() {}\n");

        let request = request_for(dir.path());

        assert_eq!(
            file_paths(&request),
            ["Cargo.toml", "build.rs"],
            "only the directory's own files, sorted; nothing from below it"
        );
        assert_eq!(
            file(&request, "build.rs").bytes(),
            Some(&b"fn main() {}\n"[..]),
            "and they carry their bytes",
        );
        assert_eq!(
            request
                .child_documents()
                .iter()
                .map(|child| (child.directory(), child.text()))
                .collect::<Vec<_>>(),
            [("src", "# src\n\nThe code.\n")],
            "a child with a document contributes it; `tests/` has none and \
             contributes no entry, which is not an error",
        );
        assert!(
            !format!("{request:?}").contains("Deeper still"),
            "a grandchild's document is already covered by its parent's, and \
             its source is never read at all",
        );
        assert_eq!(request.directory(), dir.path());
        assert_eq!(request.prompt(), "summarise");
    }

    #[test]
    fn two_gathers_of_an_unchanged_directory_are_the_same_value() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // Written in an order that is not the sorted one, so a request that
        // simply kept what the filesystem offered would have to be lucky.
        write(dir.path(), "zeta.rs", "//! z\n");
        write(dir.path(), "alpha.rs", "//! a\n");
        write(dir.path(), "zeta/WARLOCK.md", "# zeta\n");
        write(dir.path(), "alpha/WARLOCK.md", "# alpha\n");

        let request = request_for(dir.path());

        assert_eq!(file_paths(&request), ["alpha.rs", "zeta.rs"]);
        assert_eq!(
            request
                .child_documents()
                .iter()
                .map(agent::ChildDocument::directory)
                .collect::<Vec<_>>(),
            ["alpha", "zeta"],
        );
        assert_eq!(request, request_for(dir.path()), "two gathers, one value");
    }

    #[test]
    fn the_request_obeys_the_same_ignore_rules_as_the_rest_of_the_crate() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), ".gitignore", "secret.txt\n/generated\n");
        write(dir.path(), "secret.txt", "shh\n");
        write(dir.path(), ".hidden", "shh\n");
        write(dir.path(), ".warlock/notes.md", "# ours\n");
        write(dir.path(), ".warlock/WARLOCK.md", "# not a module\n");
        write(
            dir.path(),
            "generated/WARLOCK.md",
            "# not a module either\n",
        );
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        // And the repository's own exclusions, which is the same walk again.
        // `sketches.md` is small enough to be sent whole and `logo.png` is far
        // too big for the per-file cap, so a rule the walk failed to read would
        // show up twice over: as a file in the request, and as the `Problem`
        // that says the big one was listed rather than sent.
        write(dir.path(), ".warlockignore", "sketches.md\nassets/\n");
        write(dir.path(), "sketches.md", "# the author's notebook\n");
        write(dir.path(), "assets/logo.png", filler(PER_FILE_BYTE_CAP + 1));
        write(dir.path(), "assets/WARLOCK.md", "# not a module at all\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP).expect("gathers");

        assert_eq!(
            file_paths(&request),
            ["lib.rs"],
            "gitignored, hidden, `.warlock/` and `.warlockignore`d files come \
             through the same walk as everything else, so they never arrive at \
             all"
        );
        assert!(
            request.child_documents().is_empty(),
            "and a document inside an ignored, excluded or pruned directory is \
             not a child document: {:?}",
            request.child_documents(),
        );
        assert!(
            problems.is_empty(),
            "excluded content is not in the request and is not reported \
             missing from it either — it is no part of what Warlock covers: \
             {problems:?}",
        );
    }

    #[test]
    fn rules_the_request_walk_cannot_parse_fail_the_gather_and_name_the_file() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        // A range that runs backwards: a glob the matcher will not compile.
        write(dir.path(), ".warlockignore", "a[z-a]\n");

        let error = gather_request("summarise", dir.path(), REQUEST_BYTE_CAP)
            .expect_err("rules that cannot be read are not no rules");

        assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
        assert!(
            error.to_string().contains(".warlockignore"),
            "the failure names the file to go and fix: {error}"
        );
    }

    /// Only on unix, because the fixture needs `std::os::unix::fs::symlink` to
    /// build the cycle at all. The behaviour under test — that the walk does
    /// not follow links — is not platform-specific.
    #[cfg(unix)]
    #[test]
    fn a_symlink_is_neither_followed_nor_listed() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        std::os::unix::fs::symlink(dir.path(), dir.path().join("up")).expect("links to itself");
        std::os::unix::fs::symlink(dir.path().join("lib.rs"), dir.path().join("alias.rs"))
            .expect("links to a file");

        let request = request_for(dir.path());

        assert_eq!(file_paths(&request), ["lib.rs"]);
        assert!(request.child_documents().is_empty());
    }

    #[test]
    fn prose_reaches_no_pass_and_is_in_no_document() {
        // The defect this closes, measured on this repository: a route's
        // symbol is checked by looking for it in the target's text, so a
        // readme sentence naming `action_for` verified a route to the readme
        // while the function itself sat in another directory. Prose cannot be
        // told from a declaration by a substring test, so prose does not
        // travel.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        write(
            dir.path(),
            "README.md",
            "Every binding is in `action_for`.\n",
        );
        write(
            dir.path(),
            "CLAUDE.md",
            "Read the WARLOCK.md files first.\n",
        );
        write(dir.path(), "NOTES.MARKDOWN", "# notes\n");
        write(dir.path(), "guide.mdx", "# guide\n");
        write(dir.path(), DOCUMENT_FILE, "# engine\n\nWhat it was.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), AMPLE_CAP).expect("gathers");

        assert_eq!(
            file_paths(&request),
            ["lib.rs"],
            "every markdown file is out, whatever its case or spelling, and the \
             directory's own document with them",
        );
        assert!(
            problems.is_empty(),
            "and dropping prose is the ordinary case, not something to report: {problems:?}",
        );
    }

    #[test]
    fn a_directory_of_nothing_but_prose_gathers_no_files_and_still_fits() {
        // A workspace's parent directory, near enough: a readme, a licence in
        // markdown, and one child that has described itself. What is left to
        // route with is the child's document, which is the point.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "README.md", "The pitch.\n");
        write(dir.path(), "CONTRIBUTING.md", "How to help.\n");
        write(dir.path(), "src/WARLOCK.md", "# src\n\nThe code.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), AMPLE_CAP).expect("gathers");

        assert!(
            file_paths(&request).is_empty(),
            "{:?}",
            file_paths(&request)
        );
        assert_eq!(
            request.child_documents().len(),
            1,
            "the child's document is the one prose warlock wrote itself, and it stays",
        );
        assert_eq!(request.child_documents()[0].directory(), "src");
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn an_existing_document_reaches_no_pass_at_all() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "WARLOCK.md", "# engine\n\nWhat it was.\n");
        write(dir.path(), "lib.rs", "//! Core engine.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), AMPLE_CAP).expect("gathers");

        assert_eq!(
            file_paths(&request),
            ["lib.rs"],
            "the directory's own document is not one of its files: it is the \
             previous pass's claim about the directory, and no pass is shown \
             its predecessor",
        );
        assert!(
            request
                .child_documents()
                .iter()
                .all(|child| child.directory() != "." && child.directory() != "WARLOCK.md"),
            "and it is nobody's child document: {:?}",
            request.child_documents(),
        );
        assert!(
            problems.is_empty(),
            "and dropping it is the ordinary case, not something to report: {problems:?}"
        );
        assert!(
            !request.prompt().contains("What it was"),
            "nor does it reach the pass by any other road"
        );
    }

    #[test]
    fn an_enormous_existing_document_costs_the_budget_nothing() {
        // A document is dropped before it is measured, so a hand-edited or
        // generated accident of any size leaves the files' whole budget to the
        // files.
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), DOCUMENT_FILE, filler(PER_FILE_BYTE_CAP * 3));
        write(dir.path(), "lib.rs", "//! Core engine.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP).expect("gathers");

        assert_eq!(file_paths(&request), ["lib.rs"]);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            carried_bytes(request.files(), request.child_documents()),
            "//! Core engine.\n".len() as u64
        );
    }

    #[test]
    fn a_file_over_the_per_file_cap_is_listed_by_name_and_size_and_reported() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let size = PER_FILE_BYTE_CAP + 1;
        let lock = write(dir.path(), "Cargo.lock", filler(size));
        write(dir.path(), "lib.rs", "//! Core engine.\n");

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP)
                .expect("a huge file is not fatal");

        let listed = file(&request, "Cargo.lock");
        assert!(listed.is_omitted());
        assert_eq!(
            listed.size(),
            size,
            "the size is the fact that goes instead"
        );
        assert_eq!(
            listed.bytes(),
            None,
            "never truncated: no part of it is presented as if it were the whole"
        );
        assert_eq!(
            file(&request, "lib.rs").bytes(),
            Some(&b"//! Core engine.\n"[..]),
            "and the rest of the directory is untouched",
        );

        assert_eq!(problems.len(), 1, "{problems:?}");
        assert_eq!(problems[0].path, lock, "the problem names the file on disk");
        assert!(
            matches!(problems[0].cause, Omission::TooLarge { size: reported } if reported == size),
            "{:?}",
            problems[0],
        );
    }

    #[test]
    fn a_file_exactly_at_the_per_file_cap_is_still_sent_whole() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "big.bin", filler(PER_FILE_BYTE_CAP));

        let request = request_for(dir.path());

        assert_eq!(
            file(&request, "big.bin").bytes().map(<[u8]>::len),
            Some(usize::try_from(PER_FILE_BYTE_CAP).expect("fits")),
            "the cap is what a file may not exceed, not what it may not reach",
        );
    }

    #[test]
    fn a_sample_is_the_files_own_lines_and_says_what_it_left_off() {
        let lines: String = (0..500)
            .map(|n: u32| n.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let sample = super::sample_of(&lines).expect("a long file is sampled");

        for line in sample.lines().filter(|line| !line.starts_with('…')) {
            assert!(
                lines.lines().any(|original| original == line),
                "every line is one the file really has: {line}",
            );
        }
        assert!(
            sample.starts_with("0\n1\n2\n"),
            "and they are its first lines, in order: {sample}",
        );
        assert!(
            sample.contains("further lines not shown"),
            "with a marker saying what was left off, which is what makes it a \
             sample rather than a truncation: {sample}",
        );
        assert!(
            super::byte_count(sample.len()) <= SAMPLE_BYTE_CAP + 64,
            "inside the cap the trim reserves for it",
        );
    }

    #[test]
    fn one_enormous_line_is_cut_and_reports_bytes_rather_than_lines() {
        // A minified bundle is one line. Lines are the wrong unit to report
        // what was left off it, and "0 further lines" would be a truncation
        // dressed as a sample.
        let minified = "x".repeat(200_000);
        let sample = super::sample_of(&minified).expect("one long line is still sampled");

        assert!(
            sample.contains("further bytes not shown"),
            "the unit has to be the one that is true: {}",
            &sample[sample.len().saturating_sub(60)..],
        );
        assert!(
            !sample.contains("0 further"),
            "and it must not claim nothing was left off",
        );
        assert!(super::byte_count(sample.len()) <= SAMPLE_BYTE_CAP + 64);
    }

    #[test]
    fn a_file_that_already_fits_is_not_swapped_for_a_copy_of_itself() {
        assert!(
            super::sample_of("fn small() {}\n").is_none(),
            "there is no sample to take of a file that is already whole",
        );
    }

    #[test]
    fn bytes_that_are_not_text_are_the_one_thing_still_reduced_to_a_name() {
        let not_text = agent::File::present("logo.png", vec![0xff, 0xfe, 0x00, 0x01]);
        assert!(
            super::sampled(&not_text).is_none(),
            "there is no honest sample of bytes with no lines to cut on",
        );
    }

    #[test]
    fn a_directory_over_the_request_cap_gives_up_its_largest_files_first() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // Named so that alphabetical order is the reverse of size order: a
        // gather that dropped files in path order would fail here.
        let sizes = fat();
        for (name, size) in sizes {
            write(dir.path(), name, filler(size));
        }

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP)
                .expect("a fat directory is not fatal");

        assert_eq!(
            file_paths(&request),
            ["a.bin", "b.bin", "c.bin", "d.bin", "e.bin"],
            "every file is still in the request, in path order",
        );
        for (name, size) in sizes {
            assert_eq!(
                file(&request, name).size(),
                size,
                "and every one of them still says how big it is",
            );
        }
        assert_eq!(
            request
                .files()
                .iter()
                .filter(|file| !file.is_omitted())
                .map(agent::File::path)
                .collect::<Vec<_>>(),
            ["a.bin", "b.bin", "c.bin", "d.bin", "e.bin"],
            "not one of them is reduced to a bare name: the biggest are still \
             given up first, but what they are given up to is a sample of \
             their own text",
        );
        assert_eq!(
            request
                .files()
                .iter()
                .filter(|file| file.kept().is_some())
                .map(agent::File::path)
                .collect::<Vec<_>>(),
            ["c.bin", "d.bin", "e.bin"],
            "and the three given up are exactly the three carrying a sample",
        );
        for name in ["c.bin", "d.bin", "e.bin"] {
            let kept = file(&request, name).kept().expect("sampled");
            assert!(
                kept.contains("not shown"),
                "a sample says what it left off, or it is a truncation: {kept}",
            );
            assert!(
                super::byte_count(kept.len()) <= SAMPLE_BYTE_CAP + 64,
                "and it stays inside the room the trim reserved for it",
            );
        }
        assert!(
            carried(&request) <= REQUEST_BYTE_CAP,
            "{} bytes is still over the {REQUEST_BYTE_CAP}-byte cap",
            carried(&request),
        );

        assert_eq!(
            problems
                .iter()
                .map(|problem| problem.path.clone())
                .collect::<Vec<_>>(),
            ["e.bin", "d.bin", "c.bin"].map(|name| dir.path().join(name)),
            "reported in the order they were given up, largest first",
        );
        assert!(
            problems
                .iter()
                .all(|problem| matches!(problem.cause, Omission::OverBudget { .. })),
            "over budget is its own cause, not the per-file one: {problems:?}",
        );
        assert!(
            problems.iter().all(|problem| {
                let name = problem.path.file_name().expect("a file name");
                file(&request, &name.to_string_lossy()).kept().is_some()
            }),
            "a file the budget gave up is still reported — most of it really \
             is missing — but it is reported alongside a sample of itself \
             rather than instead of one",
        );
    }

    #[test]
    fn a_directory_inside_the_request_cap_gives_up_nothing() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(dir.path(), "a.bin", filler(REQUEST_BYTE_CAP / 2));
        write(dir.path(), "b.bin", filler(REQUEST_BYTE_CAP / 2));

        let request = request_for(dir.path());

        assert!(
            request.files().iter().all(|file| !file.is_omitted()),
            "exactly at the cap is inside it",
        );
        assert_eq!(carried(&request), REQUEST_BYTE_CAP);
    }

    #[test]
    fn a_childs_document_counts_towards_the_budget_and_is_never_the_thing_dropped() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        write(
            dir.path(),
            "src/WARLOCK.md",
            "x".repeat(usize::try_from(REQUEST_BYTE_CAP).expect("fits")),
        );
        write(dir.path(), "lib.rs", filler(1024));

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP)
                .expect("an enormous child document is not fatal either");

        assert_eq!(
            request.child_documents().len(),
            1,
            "the account of a whole subtree is the one thing that never gives \
             way: dropping it would leave nothing in its place",
        );
        // And the file does not give way either, which is the newer half of
        // this. Naming a 1KB file frees 1KB and commits the lift to buying an
        // account that costs several times that, so the trade is a loss before
        // it starts — and here it cannot even help, since the child document
        // alone is already the whole budget. Giving it up would cost its text
        // and buy nothing at all.
        assert!(
            !file(&request, "lib.rs").is_omitted(),
            "a file too small for the trade to pay keeps its contents",
        );
        assert!(
            problems.is_empty(),
            "and nothing is reported as given up, because nothing was: {problems:?}",
        );
        assert!(
            carried_bytes(request.files(), request.child_documents()) > REQUEST_BYTE_CAP,
            "the request stays honestly over the cap, which this module allows",
        );
    }

    #[test]
    fn the_budget_counts_a_summarised_file_as_its_summary_and_never_its_size() {
        // Nothing gathered from a directory can be summarised yet, so the
        // accounting is exercised where it is decided: on a request built by
        // hand carrying one of each of the three states.
        let summary = "The locked dependency graph: 412 crates, no path dependencies.";
        let request = agent::Request::new("summarise", "/repo")
            .with_files([
                agent::File::present("lib.rs", *b"//! Core engine.\n"),
                agent::File::omitted("vendor.js", 900_000),
                agent::File::summarised("Cargo.lock", 4_200_000, summary),
            ])
            .with_child_documents([agent::ChildDocument::new("src", "# src\n")]);

        assert_eq!(
            super::carried_bytes(request.files(), request.child_documents()),
            (b"//! Core engine.\n".len() + summary.len() + "# src\n".len()) as u64,
            "sent whole costs its bytes, listed costs nothing, and summarised \
             costs its summary — the way the child document beside it does",
        );
        assert_eq!(
            super::carried_bytes(
                &[agent::File::summarised("Cargo.lock", 4_200_000, summary)],
                &[]
            ),
            summary.len() as u64,
            "the account travels; the 4.2 MB it stands for never does",
        );
        assert_eq!(
            carried(&request),
            super::carried_bytes(request.files(), request.child_documents()),
            "and the tests' own count of what a request carries agrees",
        );
    }

    /// Only on unix, because there is no portable way to make a file
    /// unreadable. What is under test — that a file the filesystem refuses is
    /// its own case, and still not fatal — is not platform-specific.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_its_own_cause_and_still_not_fatal() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("a temporary directory");
        let unreadable = write(dir.path(), "secret.rs", "fn hidden() {}\n");
        write(dir.path(), "lib.rs", "//! Core engine.\n");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).expect("chmods");
        if fs::read(&unreadable).is_ok() {
            // Running as root: no file is unreadable, so there is nothing here
            // to assert against.
            return;
        }

        let Gathered { request, problems } =
            gather_request("summarise", dir.path(), REQUEST_BYTE_CAP)
                .expect("an unreadable file is not fatal");

        assert!(file(&request, "secret.rs").is_omitted());
        assert_eq!(
            file(&request, "lib.rs").bytes(),
            Some(&b"//! Core engine.\n"[..]),
            "one file nobody can read is one file's problem",
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            matches!(problems[0].cause, Omission::Unreadable { .. }),
            "a refused file is never reported as a budget decision: {:?}",
            problems[0],
        );

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).expect("chmods back");
    }

    #[test]
    fn an_empty_directory_is_a_request_with_nothing_in_it() {
        let dir = tempfile::tempdir().expect("a temporary directory");

        let request = request_for(dir.path());

        assert!(request.files().is_empty());
        assert!(request.child_documents().is_empty());
    }

    #[test]
    fn a_directory_that_is_not_there_is_a_walk_error() {
        let dir = tempfile::tempdir().expect("a temporary directory");

        let error = gather_request("summarise", dir.path().join("nowhere"), REQUEST_BYTE_CAP)
            .expect_err("there is nothing to walk");

        assert!(matches!(error, super::Error::Walk { .. }), "{error:?}");
        assert!(error.source().is_some(), "{error:?}");
    }

    #[test]
    fn every_problem_says_what_was_left_out_and_why_on_one_line() {
        let problems = [
            Problem {
                path: PathBuf::from("/repo/Cargo.lock"),
                cause: Omission::TooLarge { size: 4_200_000 },
            },
            Problem {
                path: PathBuf::from("/repo/data.json"),
                cause: Omission::OverBudget { size: 90_000 },
            },
            Problem {
                path: PathBuf::from("/repo/secret.rs"),
                cause: Omission::Unreadable {
                    source: std::io::Error::other("permission denied"),
                },
            },
        ];

        for problem in &problems {
            let rendered = problem.to_string();
            assert!(!rendered.contains('\n'), "{rendered}");
            assert!(
                rendered.contains(&problem.path.display().to_string()),
                "a problem names its file: {rendered}",
            );
        }
        assert!(
            problems[0].to_string().contains("4200000"),
            "{}",
            problems[0],
        );
        assert!(
            problems[2].to_string().contains("permission denied"),
            "{}",
            problems[2],
        );
        assert_eq!(
            problems
                .iter()
                .filter(|problem| problem.source().is_some())
                .count(),
            3,
            "every problem's cause is reachable as a source",
        );
        assert!(
            problems[2]
                .source()
                .and_then(std::error::Error::source)
                .is_some(),
            "and an unreadable file's cause names the io error under it",
        );
    }
}
