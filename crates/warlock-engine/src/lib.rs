//! Core engine for warlock.
//!
//! This crate owns the domain vocabulary. It never depends on the TUI or on
//! any terminal crate: the dependency edge runs TUI -> engine and never back.
//! It does touch the filesystem — it reads and writes the pact manifest at
//! `.warlock/pacts.toml`, it walks a directory to build a [`Tree`] from it,
//! and it reads the bytes of the files under a directory to hash them
//! ([`subtree_hash`]) — but it opens no sockets and spawns no subprocesses,
//! and it never follows a symlink out of the directory a caller hands it.
//!
//! That last promise holds even though a pact is written by a model. Reaching
//! one is a port: this crate defines the trait ([`Agent`]), the request and the
//! response, and owns what to ask and what an answer means, while the binary
//! implements the trait by running the `claude` CLI and owns the child process,
//! its output and its timeout. So the process lives on the far side of the
//! seam, and these tests need no `claude`, no network and no terminal.

pub mod agent;
pub mod briefs;
pub mod claude_md;
pub mod clock;
pub mod decide;
pub mod fitting;
pub mod hash;
mod ignores;
pub mod load;
pub mod manifest;
pub mod pact;
pub mod scope;
pub mod sigils;
pub mod state;
pub mod tree;

/// One pass of a model over one directory: the port the engine asks through
/// and the binary implements with a subprocess.
pub use agent::Agent;
/// One immediate child directory's `WARLOCK.md`, carried in its parent's
/// request so a pass learns what is below it without reading down there.
/// Everything that can stop a model pass producing a document, in the engine's
/// vocabulary rather than the transport's.
/// One file of the directory a model pass is about: its relative path, and its
/// bytes — or, for a file left out, its size in place of them.
/// What one model pass needs in order to run: the prompt, the directory to run
/// it in, that directory's files, and its children's documents.
/// What one model pass produced: the text the model wrote, unparsed.
/// The directory briefs go in when a repository has not said otherwise,
/// relative to its root: spelled once here, so nothing has to hard-code it.
pub use briefs::DEFAULT_BRIEF_DIRECTORY;
/// Everything that can stop the brief directory being read, each one naming the
/// file. A missing file is deliberately not in here: an optional setting that
/// was never set is the default, not a fault.
/// Where a repository's brief settings live under its root:
/// `.warlock/briefs.toml`, beside the manifest. A join; nothing here searches.
pub use briefs::briefs_path;
/// The directory a repository asks its briefs to be written in, relative to its
/// root. Absent file or absent key is [`DEFAULT_BRIEF_DIRECTORY`]; anything
/// else — not TOML, an unknown key, an absolute path, a `..` component — is a
/// named error rather than a quiet fall back to it. Reads one file and writes
/// nothing.
pub use briefs::load_briefs;
/// Everything that can stop a `CLAUDE.md` being written, each one naming the
/// file. A missing file is not in here: that is the case where one is created.
/// What writing a `CLAUDE.md` did — created a file, or brought warlock's
/// section in one that was already there up to date — and which file it was.
pub use claude_md::Written;
/// Write `<root>/CLAUDE.md`: what warlock is, what a `WARLOCK.md` is, what the
/// colours mean, and to read the documents first, in a delimited section that a
/// second run replaces in place. Every byte outside the delimiters survives, and
/// this writes that one file and nothing else — no manifest, no document, no
/// model pass.
pub use claude_md::write_claude_md;
/// Now, as the RFC 3339 UTC timestamp a grant records: `2026-08-21T14:03:11Z`.
/// Computed from the system clock with this crate's own calendar arithmetic —
/// no date/time dependency — and infallible, because a clock set before 1970 is
/// not something a caller can do anything about either.
pub use clock::now_rfc3339;
/// The colour of a node, from its manifest entry and the hash of its content:
/// no entry is unpacted, a granted hash equal to the computed one is fresh, and
/// everything else — including never judged — is stale.
///
/// Freshness is granted only where a pass has just run: [`pact_subtree`] and
/// [`refresh_subtree`] both hash each directory they described once every
/// document is written, and nothing else in this workspace writes a
/// `granted_hash`. This is also the judgement [`refresh_subtree`] runs on, one
/// directory at a time, to decide stale-or-skip: fresh directories are passed
/// over with their entries untouched, and everything else is described again.
pub use decide::decide_state;
/// Why one file's contents are not in a request: too large by itself, left by
/// the whole-request cap with no room for even an account of it, unreadable, or
/// — for a file too large to send that summarising did not rescue — not text at
/// all, past the ceiling on how many model passes one file is worth, or left
/// without a usable account by the passes that ran. Every one of them leaves
/// the same thing in the request, a name and a size, and none of them fails a
/// pact.
pub use fitting::Omission;
/// The most bytes one file may carry before it is listed by name and size
/// instead.
pub use fitting::PER_FILE_BYTE_CAP;
/// One file the byte caps left out of a request, and why. Non-fatal by
/// definition: the request that produced it is still a whole request.
/// Everything that can stop a subtree being hashed.
/// The hash of everything at and below a directory.
pub use hash::subtree_hash;
/// Everything that can stop a directory becoming a tree.
/// What a load produced: the coloured tree, plus its non-fatal problems.
pub use load::Loaded;
/// One node a load could not colour properly, and why. Non-fatal by
/// definition: the load that produced it finished.
/// Why a node is in a load's problems: a subtree that could not be hashed, or a
/// manifest entry carrying a string that is not a scope.
/// Build a tree from a directory on disk, coloured by the manifest above it.
pub use load::load_tree;
/// The nearest ancestor of a directory that holds a `.git/` directory: the
/// repository root, under which the manifest lives at `.warlock/pacts.toml`.
pub use load::repository_root;
/// Everything that can go wrong reading, writing or building a manifest.
/// The record of which modules are pacted: one `.warlock/pacts.toml` per
/// repository.
pub use manifest::Manifest;
/// One pacted module: its directory, its document, whatever was granted to it,
/// and the boundary a person put on it — the only place a scope can be stored,
/// so a module with no entry has nowhere to hold one.
pub use manifest::PactEntry;
/// The manifest schema version this build reads and writes.
pub use manifest::SCHEMA_VERSION;
/// A manifest-relative path, back as a path under a root directory.
pub use manifest::from_manifest_path;
/// Where the manifest lives under a repository root.
pub use manifest::manifest_path;
/// A caller's path in the form the manifest stores: relative, forward slashes.
pub use manifest::to_manifest_path;
/// Everything that can stop a directory getting a document, each one naming the
/// directory. Neither byte cap is in here: an over-budget file is a
/// [`fitting::Problem`], never a failure.
/// One directory a subtree pact did not finish with: no document, no manifest
/// entry, or no hash to grant it against. Never fatal to the pact around it.
/// The fewest bytes an answer may come to, trimmed, and still be written as a
/// document. A length is the only measure taken: nothing anywhere reads what
/// the text says.
pub use pact::MINIMUM_DOCUMENT_BYTES;
/// Where a subtree pact has got to and whether it should carry on: the port a
/// front end draws progress from and cancels through, asked once per directory
/// and never bound to a thread. It is also told, without being asked anything,
/// about each model pass spent describing a file too big to send.
/// What a pact produced: the `WARLOCK.md` it wrote, plus the files its byte
/// caps left out of the request behind it.
pub use pact::Pacted;
/// What a subtree pact produced: the manifest to save, the directories that
/// failed, and the files its byte caps left out.
pub use pact::PactedSubtree;
/// What a [`pact::Observer`] answers about the directory it was just offered:
/// pact it, or stop the pact before it.
pub use pact::Pacting;
/// Why a model pass produced no document: the agent failed, or the answer was
/// too short to be one. The whole rejection policy, in two variants.
pub use pact::Refusal;
/// Why a file has no text to show: it could not be read, or it is not text.
/// Two answers because they are worth different words in front of a person, and
/// neither is fatal. The per-file cap is not in here: a file cut at the cap is
/// a [`Viewed`], not a failure.
pub use pact::Unviewable;
/// The [`pact::Observer`] for a caller with nothing to show and nothing to
/// cancel: every directory is pacted and nothing is reported.
pub use pact::Unwatched;
/// What reading a file to look at produced: its text, cut at the same per-file
/// cap a request is built under, and whether the cap cut it. No marker line and
/// no lines at all — the caller words the cut and splits the text.
pub use pact::Viewed;
/// The distinct scopes at or below a directory that a set of held sigils does
/// not open: the downward half of the boundary question, asked where an act
/// would reach past the target rather than stop at it. [`scope_covering`] walks
/// *up* and says whose a path is; this walks over exactly the entries
/// [`unpact_subtree`] would drop — segment-wise, on the manifest's stored paths,
/// the repository root above everything — and says which of their boundaries
/// this machine is outside of. Deduplicated, in manifest file order, so two
/// callers name the same scope in the same words; a scope [`validate_scope`]
/// refuses reads as no scope here as everywhere. It reports and refuses
/// nothing.
pub use pact::closed_scopes_at_or_below;
/// Pact one directory: gather it, describe the files too big to send, run one
/// model pass over it, and write what came back verbatim to its `WARLOCK.md`.
/// Describing a file is passes of its own, and no way it can fail is fatal.
/// Writes no manifest entry and grants nothing.
pub use pact::pact_directory;
/// Pact a directory and everything below it: write every document first,
/// children before parents, then hash each directory and grant it what it
/// earned. Returns the manifest to save and saves nothing itself. Each directory
/// is announced to a [`pact::Observer`] first, which may stop the pact there.
pub use pact::pact_subtree;
/// Refresh a directory and everything below it: describe every directory that
/// has gone stale, exactly as a pact would, and pass over every directory that
/// is still fresh. Skipped directories keep the entry and the grant they had —
/// a refresh removes no entry and drops no grant. Returns the manifest to save
/// and saves nothing itself.
pub use pact::refresh_subtree;
/// Un-pact a directory and everything below it: drop their manifest entries and
/// leave every `WARLOCK.md` on disk, byte for byte. Returns the manifest to save
/// and saves nothing itself.
pub use pact::unpact_subtree;
/// Read one file for a person to look at: its text from the first byte, at most
/// [`PER_FILE_BYTE_CAP`] bytes of it, and the fact that there is more when
/// there is. Writes nothing, runs nothing, and never cuts a character in half.
pub use pact::view_file;
/// The rules a scope keeps, as one line for a prompt to show before anything is
/// typed: the length and the character class, worded here so that nothing that
/// asks for a scope has to describe one for itself.
/// The one rule a string broke on its way to not being a scope, renderable as
/// a single line for a prompt to refuse with or a load to report.
/// The scope covering a path: the one on the nearest pacted directory at or
/// above it that carries a valid scope, and at most one — nearest wins, so an
/// inner scope replaces an outer one outright and the outer one is a *default*
/// for what has said nothing below it, never a second gate to also satisfy. A
/// scope [`validate_scope`] refuses is stepped over as if it were not written,
/// so the answer is never a string that is not a scope. It is the one home
/// callers ask coverage from, rather than three of them each walking up the
/// tree their own way; [`scope_opens_to`] is the other half, and answers
/// whether the boundary it names is open to the person at the keyboard.
pub use scope::scope_covering;
/// Whether the scope covering a directory is open to somebody holding a given
/// set of sigils: a membership test, never an expression, since a directory
/// carries at most one scope and any one matching sigil opens it. Both defaults
/// are permissive and deliberately so — an unscoped directory is open to
/// anyone, and a machine holding nothing is unrestricted rather than shut out —
/// and the wildcard `*` is a sigil that opens everything, never a scope.
pub use scope::scope_opens_to;
/// Whether a string is a scope a pacted directory may carry: 1 to 24
/// characters of ASCII lowercase letters, digits, `-` and `_`, beginning with a
/// letter and not ending with a separator. Judges only — it never lower-cases,
/// trims or otherwise repairs what it was handed, so folding case stays with
/// the callers that take the string from a person.
pub use scope::validate_scope;
/// Whether a string is a sigil a person may hold: everything
/// [`validate_scope`] accepts, plus the wildcard `*`, which means "may work
/// anywhere" and is refused on the directory side because blank already says
/// "open to anyone" there.
pub use scope::validate_sigil;
/// Everything that can stop the machine-local sigils being read or written,
/// each one naming the file. A missing file is in here, as
/// [`Manifest::load`]'s is: absent and empty are different facts and neither is
/// invented for the caller.
/// The sigils held for a repository on this machine, read from under a home
/// directory the caller supplies. Stored strings, unvalidated: a missing file
/// is [`sigils::Error::NotFound`] rather than an empty set, and a file that cannot
/// be read or parsed is a named error rather than either.
pub use sigils::load_sigils;
/// The directory name one repository's machine-local config sits in: the
/// repository root's own name plus a short digest of its canonical absolute
/// path, so two checkouts of one repository hold sigils separately.
pub use sigils::project_directory;
/// Replace the sigils held for a repository on this machine, atomically, under
/// a home directory the caller supplies. Writes that one file and nothing else
/// — never anything inside the repository.
pub use sigils::save_sigils;
/// Where one repository's machine-local sigils live under a home directory:
/// `<home>/.warlock/<project>/config.toml`. A join and a derivation; nothing
/// here reads `HOME`.
pub use sigils::sigils_path;
/// The three-state vocabulary every node is coloured by.
pub use state::NodeState;
/// A depth-first walk over a tree, yielding each node with its depth.
pub use tree::DepthFirst;
/// What `Node::new` accepts for a node's `WARLOCK.md`: anything path-like, or
/// `None` for a node that has none.
pub use tree::IntoDocument;
/// One node of the project tree: its path, its document, its state, its
/// children.
pub use tree::Node;
/// How many nodes of a tree sit in each state.
pub use tree::StateCounts;
/// A whole project tree, owning its root node.
pub use tree::Tree;
