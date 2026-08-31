//! Where a brief would go: the reply unwrapped, its title slugged, and the next
//! number in the directory.
//!
//! This is the arithmetic behind the path the `/write` prompt opens holding —
//! `docs/warlock-brief-13-scopes-and-sigils.md` — and nothing else. It proposes
//! a path; it does not write one, does not create a directory, does not open a
//! field and does not touch the reply that will land in the file. The one thing
//! it does to the disk is read the names in one directory, because "one above
//! the highest number already there" cannot be answered without them.
//!
//! ## The proposal is only a proposal
//!
//! Every rule below is a guess at what somebody would have typed, and the field
//! it lands in is editable, which is what makes the guesses cheap. So none of
//! them refuses anything: a reply with no title still gets a path, a title made
//! entirely of punctuation still gets a path, and a directory holding a file
//! with `2026` in its name still gets a path. The reader reads the line and
//! changes it or presses Enter. Nothing here can fail, which is why nothing here
//! returns a [`Result`].
//!
//! ## The directory is a constant here
//!
//! [`BRIEFS_DIRECTORY`] is the built-in `docs/`, joined onto the repository
//! root. The configurable output directory is `.warlock/briefs.toml`'s, a later
//! slice's, and deliberately not this one's: nothing in this file reads a
//! config, and the day the setting arrives it replaces one constant in one
//! place.
//!
//! ## The unwrap comes before the title
//!
//! A model asked for a document sometimes hands back the document inside one
//! fenced block, which is why the written file is unwrapped before it is
//! written. The heading is looked for *after* that same unwrap, so a fenced
//! reply's title still supplies the slug rather than the slug being `untitled`
//! for a document whose first line is a fence. [`unfenced`] is that one
//! transformation, and it is the same function the write itself will use, so the
//! path proposed and the bytes written can never disagree about what the
//! document is.
//!
//! It unwraps only a reply that is *entirely* one block: an opening fence on the
//! first line, a closing fence on the last, and no closing fence in between. A
//! document that merely *contains* code blocks is left exactly as it stands,
//! because that is a document, and stripping its first and last lines would be
//! warlock editing prose it promised to copy.
//!
//! ## The slug is the title, folded, and no cleverer than that
//!
//! Lower-cased, runs of non-alphanumerics collapsed to a single hyphen, leading
//! and trailing hyphens trimmed. Nothing before a colon is stripped:
//! `# Scopes: the boundary problem` becomes `scopes-the-boundary-problem`,
//! because a title's first word is usually the subject and a filename that
//! throws it away is a filename nobody can find. There is no stop-word list, no
//! transliteration and no dictionary — every one of those would be warlock
//! having opinions about English in a string the reader is looking at and can
//! retype.
//!
//! The cap is *about* sixty characters and is broken at a hyphen, so the slug
//! ends on a whole word rather than mid-word. A single word longer than the cap
//! has nowhere to break and is kept whole, which is why the cap is "about": a
//! name cut into nonsense is worse than a name a few characters over.
//!
//! A reply with no `# ` line, and a title that leaves nothing behind once it is
//! folded, both come out as [`UNTITLED`]. A literal word rather than an empty
//! string, because `docs/warlock-brief-13-.md` reads as a bug and
//! `docs/warlock-brief-13-untitled.md` reads as an invitation to type.
//!
//! ## The number is one above the highest one there, wherever it is in the name
//!
//! Every digit run in every name in the directory counts, rather than only those
//! matching warlock's own filename shape. `docs/` today holds
//! `red-brief-12-…`, written before warlock existed and by another hand, and a
//! rule that only counted `warlock-brief-NN-…` would propose `01` into a
//! directory whose next brief is plainly the thirteenth. The cost is that a name
//! carrying a year proposes a number nobody expected — and that is a line in an
//! editable field, on screen, one keystroke from being fixed.
//!
//! Two digits with a leading zero below ten, so the directory sorts the way it
//! reads. Past ninety-nine the number simply gets wider — `100`, not `00` and
//! not a hundred pretending to be `99` — because the padding exists to make
//! `01` sort before `10`, and a wider number keeps that true for as long as the
//! widths agree.

use std::fs;
use std::path::Path;

/// Where a brief is written, relative to the repository root.
///
/// The built-in default, and the whole of the answer in this slice: the
/// directory named in `.warlock/briefs.toml` is a later one's, and until it
/// lands this constant is the only place the answer is written down.
// Nothing outside the tests below reaches for it yet, for the reason
// `proposed_path` gives.
#[allow(dead_code)]
pub(crate) const BRIEFS_DIRECTORY: &str = "docs";

/// What every brief's filename begins with, before its number and its slug.
const BRIEF_PREFIX: &str = "warlock-brief";

/// The slug for a document that gave nothing to make one out of.
const UNTITLED: &str = "untitled";

/// About how many characters of the title survive into the slug, before the
/// last whole word that fits.
const SLUG_MAX: usize = 60;

/// The path the write prompt opens holding, spelled relative to the repository
/// root: `docs/warlock-brief-NN-slug.md`.
///
/// The three rules in one line, in the order they depend on each other: the
/// reply is unwrapped, the slug comes off the unwrapped document's first `# `
/// line, and the number comes off the names in the directory the file would
/// land in. Relative rather than absolute because it is going into a field
/// somebody reads and edits, and because it is the spelling the thread names
/// the written file by afterwards.
///
/// Reads `repo_root/docs` and nothing else, creates nothing, and is as happy
/// with a directory that is not there as with an empty one.
// The event loop does not call this yet: the prompt it fills opens in the next
// slice, and the alternative to one narrow allow here is wiring a window before
// the code that draws it exists. The attribute comes off with that wiring, and
// it is on the two crate-visible names rather than on the module, so everything
// below still has to be reachable from here or be told it is not.
#[allow(dead_code)]
pub(crate) fn proposed_path(repo_root: &Path, reply: &str) -> String {
    let number = spelled(next_number(&repo_root.join(BRIEFS_DIRECTORY)));
    let slug = slug_of(unfenced(reply));
    format!("{BRIEFS_DIRECTORY}/{BRIEF_PREFIX}-{number}-{slug}.md")
}

/// The document inside a reply that is entirely one fenced code block, or the
/// reply exactly as it stands.
///
/// The one transformation warlock makes to a reply besides ensuring a trailing
/// newline, and it is deliberately narrow. All three of these have to hold, or
/// the reply is handed back untouched: the first line of the trimmed reply
/// opens a fence, the last line closes it, and no line between them closes it.
/// The third is what tells a document wrapped in a fence from a document
/// containing fenced examples — the second kind is far more common in the
/// briefs this writes — and getting it wrong would silently eat the first and
/// last lines of somebody's document.
///
/// Both of the fence characters markdown defines are understood, since which one a
/// model reaches for is not warlock's decision to have a preference about, and a
/// closing fence must be at least as long as the one that opened it, so a
/// document whose own examples are three backticks long survives being handed
/// back inside a four-backtick fence.
///
/// A slice of the input rather than a new [`String`]: whoever writes the file
/// writes these bytes, and copying them would be a second place for them to
/// change.
pub(crate) fn unfenced(reply: &str) -> &str {
    let block = reply.trim();
    let Some((first, rest)) = block.split_once('\n') else {
        return reply;
    };
    let Some(fence) = opening_fence(first) else {
        return reply;
    };
    // A block with nothing in it is `rest` being the closing fence alone, with
    // no newline left to split on.
    let (body, last) = rest.rsplit_once('\n').unwrap_or(("", rest));
    if !closes(last, fence) || body.lines().any(|line| closes(line, fence)) {
        return reply;
    }
    body
}

/// The slug for a document: its first `# ` line folded into a filename, or
/// [`UNTITLED`] when there is nothing to fold.
///
/// The first such line and not the longest, the outermost or the one the model
/// meant — a document's title is its first heading, and looking for anything
/// cleverer would mean parsing markdown to propose a filename. The line must
/// begin with `# ` exactly: an indented heading, a `#Title` with no space and a
/// `## Section` are all not it, and a reply that has only those is `untitled`,
/// which is a field the reader can type into rather than a guess they have to
/// undo.
pub(crate) fn slug_of(document: &str) -> String {
    let title = document
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .unwrap_or_default();
    let slug = slugged(title);
    if slug.is_empty() {
        return UNTITLED.to_string();
    }
    slug
}

/// One above the highest number appearing anywhere in any name in `directory` —
/// or `1` for a directory that is empty, unreadable or not there at all.
///
/// A directory that cannot be read proposes the same number as an empty one on
/// purpose. The alternatives are refusing to propose a path, which costs the
/// reader the prompt, or saying so on a line, which says nothing they can act on
/// — the directory is about to be created by the write, and the field in front
/// of them is where a number they disagree with gets fixed.
///
/// Every entry counts, files and directories alike, because "the highest number
/// already there" is a fact about the names in front of a reader rather than
/// about which of them warlock recognises.
fn next_number(directory: &Path) -> u32 {
    let Ok(entries) = fs::read_dir(directory) else {
        return 1;
    };
    let highest = entries
        .flatten()
        .filter_map(|entry| highest_number_in(&entry.file_name().to_string_lossy()))
        .max()
        .unwrap_or(0);
    // Saturating rather than wrapping, so a directory holding a name with four
    // billion in it proposes an absurd number instead of proposing `01` and
    // colliding with the first brief ever written there.
    highest.saturating_add(1)
}

/// A number as a filename spells it: `01`, `13`, `100`.
///
/// Two digits so the directory sorts the way it reads, and no ceiling, because
/// the padding is there to keep `01` ahead of `10` rather than to promise a
/// width. A hundredth brief is `100`.
fn spelled(number: u32) -> String {
    format!("{number:02}")
}

/// The highest number any run of digits in `name` spells, or `None` for a name
/// with no digits in it.
///
/// A run too long to be a [`u32`] is not a number anybody meant, and is ignored
/// rather than clamped: a checksum in a filename should not decide what the next
/// brief is called.
fn highest_number_in(name: &str) -> Option<u32> {
    name.split(|character: char| !character.is_ascii_digit())
        .filter_map(|run| run.parse::<u32>().ok())
        .max()
}

/// `title` as a filename spells it: lower-cased, runs of non-alphanumerics
/// collapsed to one hyphen, hyphens trimmed off both ends, and capped.
///
/// [`char::is_alphanumeric`] rather than an ASCII test, so a title in a script
/// warlock has never heard of keeps its own letters instead of folding away to
/// nothing. What is left is empty only when the title held no letter or digit at
/// all, and [`slug_of`] answers that with [`UNTITLED`].
fn slugged(title: &str) -> String {
    let mut slug = String::new();
    for character in title.chars() {
        if character.is_alphanumeric() {
            slug.extend(character.to_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    capped(slug.trim_matches('-')).to_string()
}

/// `slug` cut to about [`SLUG_MAX`] characters at a hyphen.
///
/// Three cases, and the third is the reason the cap is "about". A slug that fits
/// is returned whole; a slug that does not is cut back to the last hyphen inside
/// the cap; and a slug whose first word is itself longer than the cap is cut
/// after that word, however long it is, because there is nowhere to break it and
/// half a word is not a name.
fn capped(slug: &str) -> &str {
    let Some((cut, _)) = slug.char_indices().nth(SLUG_MAX) else {
        return slug;
    };
    // The character just past the cap is the separator itself: the last word
    // ends exactly on the cap, so everything up to it fits.
    if slug[cut..].starts_with('-') {
        return &slug[..cut];
    }
    if let Some(hyphen) = slug[..cut].rfind('-') {
        return &slug[..hyphen];
    }
    match slug[cut..].find('-') {
        Some(end) => &slug[..cut + end],
        None => slug,
    }
}

/// A fence as the line that opened it defines it: which character it is made of,
/// and how many of them the fence that closes it needs.
#[derive(Clone, Copy)]
struct Fence {
    marker: char,
    width: usize,
}

/// The fence `line` opens, or `None` for a line that opens none.
///
/// The markdown rule, less the indentation it allows: three or more backticks or
/// tildes, then an info string, which for a backtick fence may not itself
/// contain a backtick — that last is what keeps a first line that merely quotes
/// some backticks at each other from being read as a fence somebody opened.
fn opening_fence(line: &str) -> Option<Fence> {
    let line = line.trim_end();
    let marker = line
        .chars()
        .next()
        .filter(|character| matches!(character, '`' | '~'))?;
    let width = line
        .chars()
        .take_while(|&character| character == marker)
        .count();
    if width < 3 {
        return None;
    }
    let info = line.trim_start_matches(marker);
    if marker == '`' && info.contains('`') {
        return None;
    }
    Some(Fence { marker, width })
}

/// Whether `line` closes `fence`: that character and nothing else, at least as
/// many of them as opened it.
fn closes(line: &str, fence: Fence) -> bool {
    let line = line.trim();
    line.chars().count() >= fence.width && line.chars().all(|character| character == fence.marker)
}

/// What a `/write` proposes to call the file: the fence taken off, the title
/// folded, the number counted, and the whole path assembled out of the three.
///
/// Every rule is a function of a string and at most one directory of the test's
/// own, so the whole suite runs with no terminal, no network, no `claude` and
/// nothing written anywhere but a temporary directory that goes away with the
/// test that made it.
#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::{SLUG_MAX, next_number, proposed_path, slug_of, unfenced};

    /// A repository of this test's own, removed when the test that made it
    /// ends.
    fn a_repo() -> TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// `docs/` under `root` holding an empty file of each of `names`, which is
    /// all the numbering ever looks at.
    fn docs_holding(root: &Path, names: &[&str]) {
        let docs = root.join("docs");
        fs::create_dir_all(&docs).expect("makes the output directory");
        for name in names {
            fs::write(docs.join(name), "").expect("writes a file into it");
        }
    }

    /// The whole of a reply that is one document with one title line.
    fn titled(title: &str) -> String {
        format!("{title}\n\nSome prose about it.\n")
    }

    #[test]
    fn a_title_is_lowercased_and_hyphenated_at_every_run_of_anything_else() {
        for (title, slug) in [
            ("# Freshness", "freshness"),
            ("# The Brief Workflow", "the-brief-workflow"),
            // Runs collapse to one hyphen however long they are, and whatever
            // they are made of.
            ("# Read   what  it wrote", "read-what-it-wrote"),
            ("# Scopes & sigils", "scopes-sigils"),
            ("# back  --  to  green", "back-to-green"),
            // Both ends trimmed, so no filename starts or ends in a separator.
            ("# --Big files!--", "big-files"),
            ("#    At work   ", "at-work"),
            // Digits are alphanumeric and stay where they are.
            ("# 12 factor apps", "12-factor-apps"),
            // A script warlock has never heard of keeps its own letters rather
            // than folding away to nothing.
            ("# Дом and home", "дом-and-home"),
        ] {
            assert_eq!(slug_of(&titled(title)), slug, "the slug for {title:?}");
        }
    }

    #[test]
    fn nothing_before_a_colon_is_stripped() {
        // The rule that is easiest to get wrong by being helpful: a title's
        // first word is usually its subject, and a filename that throws it away
        // is a filename nobody can find again.
        assert_eq!(
            slug_of(&titled("# Scopes: the boundary problem")),
            "scopes-the-boundary-problem"
        );
    }

    #[test]
    fn the_first_hash_space_line_is_the_title_and_nothing_else_is() {
        // A heading is a line beginning with `# ` exactly. A deeper heading, an
        // indented one and a hash with no space after it are all not one, so
        // the first *real* heading wins however far down it is.
        let reply = "## Not this\n   # Nor this\n#NorThis\n\n# The real title\n\n# A later one\n";

        assert_eq!(slug_of(reply), "the-real-title");
    }

    #[test]
    fn a_title_capped_at_about_sixty_characters_breaks_at_a_hyphen() {
        let slug = slug_of(&titled(
            "# The quick brown fox jumps over the lazy dog and keeps on running for a while",
        ));

        // Cut back to the last whole word inside the cap: never `…-run`.
        assert_eq!(
            slug,
            "the-quick-brown-fox-jumps-over-the-lazy-dog-and-keeps-on"
        );
        assert!(slug.chars().count() <= SLUG_MAX, "{slug} is over the cap");
        assert!(!slug.ends_with('-'), "{slug} ends on a separator");
    }

    #[test]
    fn a_slug_that_fits_is_left_exactly_as_it_is() {
        // Sixty characters on the nose, and the word after it: the first is
        // never touched, and the second is cut at the separator that is already
        // there rather than one character earlier.
        let sixty = "aaaaaaaaaa bbbbbbbbbb cccccccccc dddddddddd eeeeeeeeee fffff";
        let folded = "aaaaaaaaaa-bbbbbbbbbb-cccccccccc-dddddddddd-eeeeeeeeee-fffff";
        assert_eq!(folded.chars().count(), SLUG_MAX);

        assert_eq!(slug_of(&titled(&format!("# {sixty}"))), folded);
        assert_eq!(slug_of(&titled(&format!("# {sixty} ggggg"))), folded);
    }

    #[test]
    fn one_word_longer_than_the_cap_is_kept_whole_rather_than_cut_in_half() {
        // "About sixty" is the whole of the promise: there is nowhere to break
        // this, and half a word is not a name.
        let long = "a".repeat(SLUG_MAX + 20);

        assert_eq!(slug_of(&titled(&format!("# {long}"))), long);
        assert_eq!(slug_of(&titled(&format!("# {long} and more"))), long);
    }

    #[test]
    fn a_reply_that_is_entirely_one_fenced_block_is_unwrapped() {
        for reply in [
            "```\n# Freshness\n\nProse.\n```",
            // An info string, which is what a model actually writes.
            "```markdown\n# Freshness\n\nProse.\n```\n",
            // Whitespace either side of the whole thing, and a closing fence
            // longer than the one that opened it.
            "\n\n~~~\n# Freshness\n\nProse.\n~~~~\n\n",
        ] {
            assert_eq!(unfenced(reply), "# Freshness\n\nProse.", "{reply:?}");
        }
    }

    #[test]
    fn a_document_that_merely_contains_fenced_blocks_is_left_exactly_as_it_is() {
        for reply in [
            // The ordinary case: prose with an example in it.
            "# Freshness\n\n```rust\nlet x = 1;\n```\n\nMore prose.\n",
            // Two blocks and nothing else, which opens and closes twice.
            "```\none\n```\n\n```\ntwo\n```\n",
            // A block that is never closed.
            "```\n# Freshness\n\nProse.\n",
            // A fence made of a different character than the one that opened
            // it, and a closing fence shorter than the opening one.
            "````\n# Freshness\n```\n",
            "~~~\n# Freshness\n```\n",
            // Not a fence at all.
            "# Freshness\n\nProse.\n",
            "`code` and ```more```\n# Freshness\n```\n",
            // One line, so there is no last line for a fence to close on.
            "# Freshness",
        ] {
            assert_eq!(unfenced(reply), reply, "{reply:?} was interfered with");
        }
    }

    #[test]
    fn the_title_is_looked_for_after_the_unwrap() {
        // The whole reason the two are ordered: a fenced reply's first line is
        // a fence, and a slug taken before the unwrap would be `untitled` for a
        // document that plainly has a title.
        let repo = a_repo();

        assert_eq!(
            proposed_path(
                repo.path(),
                "```markdown\n# Scopes and sigils\n\nProse.\n```\n"
            ),
            "docs/warlock-brief-01-scopes-and-sigils.md"
        );
    }

    #[test]
    fn a_reply_with_no_heading_is_untitled() {
        let repo = a_repo();

        // The literal word, in the whole path, so the field opens on something
        // that reads as an invitation to type rather than as a bug.
        assert_eq!(
            proposed_path(repo.path(), "Prose with no title anywhere in it.\n"),
            "docs/warlock-brief-01-untitled.md"
        );
    }

    #[test]
    fn a_title_that_folds_away_to_nothing_is_untitled_too() {
        let repo = a_repo();

        assert_eq!(
            proposed_path(repo.path(), &titled("# ---")),
            "docs/warlock-brief-01-untitled.md"
        );
    }

    #[test]
    fn the_number_is_one_above_the_highest_in_any_name_in_the_directory() {
        let repo = a_repo();
        // What `docs/` actually holds: documents written by another hand, in
        // another shape, which a rule that only counted warlock's own filenames
        // would ignore.
        docs_holding(
            repo.path(),
            &[
                "red-brief-01-foundations.md",
                "red-brief-12-the-brief-workflow.md",
                "red-brief-09-scopes-and-sigils.md",
                "warlock-design-doc.md",
            ],
        );

        assert_eq!(next_number(&repo.path().join("docs")), 13);
        assert_eq!(
            proposed_path(repo.path(), &titled("# Scopes and sigils")),
            "docs/warlock-brief-13-scopes-and-sigils.md"
        );
    }

    #[test]
    fn an_empty_or_absent_directory_proposes_01() {
        // Absent: nothing has ever been written here, which is every repository
        // the first time.
        let repo = a_repo();
        assert_eq!(next_number(&repo.path().join("docs")), 1);
        assert_eq!(
            proposed_path(repo.path(), &titled("# Foundations")),
            "docs/warlock-brief-01-foundations.md"
        );

        // Empty: the directory is there and holds nothing, which is the same
        // answer.
        docs_holding(repo.path(), &[]);
        assert_eq!(next_number(&repo.path().join("docs")), 1);
        assert_eq!(
            proposed_path(repo.path(), &titled("# Foundations")),
            "docs/warlock-brief-01-foundations.md"
        );
    }

    #[test]
    fn a_number_counts_wherever_in_the_name_it_sits() {
        let repo = a_repo();
        // Named for what this costs as much as for what it buys: a year in a
        // filename is a number in a filename, and the answer lands in an
        // editable field rather than in a file.
        docs_holding(repo.path(), &["notes-2026-08-31.md", "red-brief-03-x.md"]);

        assert_eq!(next_number(&repo.path().join("docs")), 2027);
    }

    #[test]
    fn a_directory_is_a_name_in_the_directory_too() {
        let repo = a_repo();
        docs_holding(repo.path(), &["red-brief-02-x.md"]);
        fs::create_dir(repo.path().join("docs/07-archive")).expect("makes a subdirectory");

        assert_eq!(next_number(&repo.path().join("docs")), 8);
    }

    #[test]
    fn nothing_outside_the_output_directory_is_counted() {
        let repo = a_repo();
        docs_holding(repo.path(), &["red-brief-04-x.md"]);
        // Beside `docs/`, not in it.
        fs::write(repo.path().join("release-99.md"), "").expect("writes a file at the root");

        assert_eq!(
            proposed_path(repo.path(), &titled("# At work")),
            "docs/warlock-brief-05-at-work.md"
        );
    }

    #[test]
    fn past_ninety_nine_the_number_gets_wider_rather_than_wrapping() {
        // The padding exists to keep `01` ahead of `10`; it is not a promise
        // about width, and a hundredth brief is `100` rather than `00`.
        let repo = a_repo();
        docs_holding(repo.path(), &["warlock-brief-99-x.md"]);

        assert_eq!(
            proposed_path(repo.path(), &titled("# Keeping up")),
            "docs/warlock-brief-100-keeping-up.md"
        );
    }

    #[test]
    fn a_run_of_digits_too_long_to_be_a_number_is_ignored() {
        // A checksum in a filename is not somebody's numbering, and clamping to
        // the largest number there is would propose the same path forever
        // after.
        let repo = a_repo();
        docs_holding(
            repo.path(),
            &["sha-99999999999999999999999999.md", "red-brief-06-x.md"],
        );

        assert_eq!(next_number(&repo.path().join("docs")), 7);
    }

    #[test]
    fn proposing_a_path_writes_nothing_and_creates_nothing() {
        // The whole module in one assertion: it reads one directory and answers
        // with a string. Nothing here makes the directory it names, and the
        // repository is exactly as empty afterwards as it was before.
        let repo = a_repo();

        let path = proposed_path(repo.path(), &titled("# Foundations"));

        assert_eq!(path, "docs/warlock-brief-01-foundations.md");
        assert!(!repo.path().join("docs").exists(), "the directory was made");
        assert_eq!(
            fs::read_dir(repo.path())
                .expect("the repository reads back")
                .count(),
            0,
            "something was written into the repository"
        );
    }
}
