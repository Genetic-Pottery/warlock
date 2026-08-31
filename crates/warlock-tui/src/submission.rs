//! What a submitted draft turns out to be: a command, a message, or neither.
//!
//! [`compose_for`](crate::compose_for) says what a key does to the draft and
//! stops there — [`Composed::Submit`](crate::Composed::Submit) means "somebody
//! pressed Enter on something", and deliberately not "somebody asked the model
//! a question". This module is the sentence after that one. A draft goes in and
//! one intention comes out: one of the three commands warlock has, an ordinary
//! message for the model, or a refusal, which is what a draft that was *trying*
//! to be a command and missed comes to.
//!
//! It is the shape the binary's `action_for` has, and
//! [`edit_for`](crate::edit_for), and [`compose_for`](crate::compose_for): a
//! value in, one intention out, no terminal, no [`App`](crate::App) and no
//! [`ChatAgent`](crate::ChatAgent) to ask anything of. Which is the whole
//! reason the rule below is twenty lines of assertions rather than a session
//! somebody has to sit through.
//!
//! ## The rule, in the order it is applied
//!
//! 1. Trim the draft at both ends.
//! 2. Take the first whitespace-separated token.
//! 3. A token that does not begin with `/` means the draft is a message.
//! 4. A token that begins with `/` and holds a second `/` means the draft is a
//!    message too.
//! 5. Otherwise the token is a command word: it is matched, case-sensitively,
//!    against exactly `/brief`, `/write` and `/chat`, and refused if it matches
//!    none of them or if anything at all follows it — another token, or another
//!    line.
//!
//! Trailing trim is not tidiness, it is the rule that makes the feature usable:
//! `"/brief "` is how the command is actually typed, because a space after a
//! word is what a hand does before it notices there is no second word to type.
//! A `/brief` that worked and a `/brief ` that was refused would be one
//! invisible character deciding between a mode change and a complaint.
//!
//! The second-slash test is step 4 rather than a special case bolted on at the
//! end because a path is the common accident: `/home/cole/notes` is not a
//! mistyped command, it is somebody talking about a file, and `home/cole/notes`
//! is not a command word anyway. Sending it is what they meant.
//!
//! ## Why matching is case-sensitive
//!
//! `/BRIEF` is refused rather than folded to `/brief`. The two mistakes here do
//! not cost the same: a refusal costs one line on the card, which is read and
//! corrected in a second, and a send costs a turn — a question to the model,
//! money, and a wait — to discover that warlock guessed at what was meant.
//! Where the costs are that lopsided, the cheap failure is the right one. It is
//! also the rule the rest of the program already keeps, where `g` and `G` are
//! two different keys, and it keeps the set of things that are commands small
//! and stateable rather than a family of spellings nobody can enumerate.
//!
//! ## Why the refusal is one line, and why there is no `/help`
//!
//! [`Submitted::refusal`] is the entire discovery mechanism. A reader who types
//! `/` and nothing else, or `/plan` because another program has one, gets back
//! one line naming the three commands that exist and saying that none of them
//! takes anything after it — so the list is reachable without knowing anything,
//! and a typo is corrected without costing a turn to find out about. A fourth
//! command whose job was to print that list would be a thing to discover before
//! you could discover anything, which is the problem it was meant to solve.
//!
//! One line, because it is a note on a conversation and not a manual page: the
//! card it lands on is where the reader's own words and the model's answers are,
//! and warlock speaking there at length would be warlock talking over them.
//!
//! ## What this deliberately does not carry
//!
//! The message. [`Submitted::Message`] holds no text, exactly as
//! [`Composed::Submit`](crate::Composed::Submit) holds none: the draft is
//! already in the caller's hand, so a copy of it here would only be a second
//! version of what was typed, free to disagree with the first.
//!
//! And it does nothing about any of them. Nothing here enters a mode, writes a
//! file, opens a turn or puts a line on the thread — a command word is
//! recognised and handed back, and what a recognised command *does* belongs to
//! the loop that owns the thread.

/// The one line a refused draft comes to.
///
/// Stated once, here, because it is the only place warlock says which commands
/// exist: a second copy of this sentence in the loop or in a test fixture would
/// be a second list to keep true. It names all three and says what none of them
/// takes, and it is one line — see the module docs for why it is not a page.
const REFUSAL: &str = "warlock has three commands — /brief, /write and /chat — and none of them takes anything after it.";

/// What a submitted draft is: one of the three commands, a message for the
/// model, or a refusal.
///
/// Five variants and no sixth for "empty", because an empty draft never gets
/// here — [`Composer::is_submittable`](crate::Composer::is_submittable) is what
/// declines to offer one up — and a function that is total anyway is worth more
/// than a variant every caller would have to match on. Nothing but
/// [`Submitted::Message`] ever reaches the model, and none of the five carries
/// any text of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Submitted {
    /// `/brief`: enter brief mode.
    Brief,
    /// `/write`: converge and write the artifact.
    Write,
    /// `/chat`: leave brief mode, writing nothing.
    Chat,
    /// An ordinary message: the draft goes to the model as it stands. It
    /// carries no text, because the caller is already holding the draft.
    Message,
    /// A draft that meant to be a command and is not one: an unknown word, a
    /// bare `/`, or a command word with something after it. It never reaches
    /// the model — the point of refusing rather than sending is that a typo
    /// costs a line and not a turn — and it comes with [`Submitted::refusal`]
    /// to say so.
    Refused,
}

impl Submitted {
    /// The one line to put on the thread, for a refusal, and nothing for
    /// anything else.
    ///
    /// A method rather than a bare constant so that the caller asks the value
    /// what it has to say instead of asking itself which variant deserves a
    /// line: a command and a message have nothing to announce, and this is the
    /// shape that says so.
    #[must_use]
    pub const fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Refused => Some(REFUSAL),
            _ => None,
        }
    }
}

/// What the submitted `draft` is: a command, a message, or a refusal.
///
/// The rule is in the module docs, in the order it is applied, and the order is
/// the whole design: trim, then the first token, then "does it start with a
/// slash", then "is it a path", and only then the match against the three words.
/// Everything that is not caught by one of the first four steps is a message,
/// which is why a draft of ordinary prose never has to be examined at all.
///
/// Pure, and total: a draft of nothing at all is a message, though nothing
/// submits one.
#[must_use]
pub fn submitted_for(draft: &str) -> Submitted {
    let draft = draft.trim();
    let Some(word) = draft.split_whitespace().next() else {
        return Submitted::Message;
    };
    let Some(rest) = word.strip_prefix('/') else {
        return Submitted::Message;
    };
    // A second slash makes it a path somebody is talking about, not a command
    // word they misspelled.
    if rest.contains('/') {
        return Submitted::Message;
    }
    // The trimmed draft *is* the word when nothing follows it — which covers a
    // second token and a second line in the one comparison, since both leave
    // characters the first token does not have.
    if draft != word {
        return Submitted::Refused;
    }

    match word {
        "/brief" => Submitted::Brief,
        "/write" => Submitted::Write,
        "/chat" => Submitted::Chat,
        // Case-sensitively, so `/BRIEF` lands here: see the module docs.
        _ => Submitted::Refused,
    }
}

#[cfg(test)]
mod tests {
    use super::{Submitted, submitted_for};

    #[test]
    fn each_command_word_is_its_own_command() {
        assert_eq!(submitted_for("/brief"), Submitted::Brief);
        assert_eq!(submitted_for("/write"), Submitted::Write);
        assert_eq!(submitted_for("/chat"), Submitted::Chat);
    }

    #[test]
    fn whitespace_at_either_end_is_trimmed_before_matching() {
        // The trailing case is the one that matters: a space after the word is
        // what a hand does before it notices there is no second word to type,
        // so `"/brief "` has to be the command and not a complaint.
        for draft in [
            "  /brief",
            "/brief ",
            "  /brief  ",
            "\n/brief\n",
            "\t/brief\t",
        ] {
            assert_eq!(
                submitted_for(draft),
                Submitted::Brief,
                "{draft:?} is the /brief command"
            );
        }
    }

    #[test]
    fn a_draft_that_does_not_begin_with_a_slash_is_a_message() {
        for draft in [
            "why nine passes?",
            "  what does the engine do  ",
            "one\ntwo",
            "brief",
            "tell me about /brief",
        ] {
            assert_eq!(
                submitted_for(draft),
                Submitted::Message,
                "{draft:?} is a message"
            );
        }
    }

    #[test]
    fn a_second_slash_makes_it_a_path_and_so_a_message() {
        // `home/cole/notes` is not a command word, and somebody naming a file
        // is the common case rather than the odd one.
        for draft in [
            "/home/cole/notes",
            "/home/cole/notes is stale",
            "/brief/notes",
            "//",
        ] {
            assert_eq!(
                submitted_for(draft),
                Submitted::Message,
                "{draft:?} is a path, not a command"
            );
        }
    }

    #[test]
    fn a_word_that_is_not_one_of_the_three_is_refused() {
        // A typo, a command another program has, the right word in the wrong
        // case, and the bare slash somebody types to find out what exists.
        for draft in ["/breif", "/plan", "/BRIEF", "/Brief", "/"] {
            assert_eq!(
                submitted_for(draft),
                Submitted::Refused,
                "{draft:?} is not a command warlock has"
            );
        }
    }

    #[test]
    fn a_command_word_with_anything_after_it_is_refused() {
        // No command takes an argument, and a second line is an argument by
        // another route: a `/brief` with a paragraph under it is somebody
        // expecting the paragraph to be read.
        for draft in [
            "/brief now",
            "/brief  now",
            "/write docs/plan.md",
            "/chat please",
            "/brief\nsome text",
            "/brief \n some text ",
        ] {
            assert_eq!(
                submitted_for(draft),
                Submitted::Refused,
                "{draft:?} takes something after the command word"
            );
        }
    }

    #[test]
    fn every_refusal_is_the_same_one_line() {
        // One line and one wording, whichever way the draft missed: the reader
        // gets the list of what exists rather than a diagnosis of what they
        // typed, because the list is the thing that helps.
        let refusals = ["/breif", "/plan", "/BRIEF", "/", "/brief now", "/brief\nx"];

        for draft in refusals {
            let line = submitted_for(draft)
                .refusal()
                .expect("a refused draft has a line");

            assert!(!line.contains('\n'), "{draft:?} gave more than one line");
            assert!(line.contains("/brief"), "{draft:?} did not name /brief");
            assert!(line.contains("/write"), "{draft:?} did not name /write");
            assert!(line.contains("/chat"), "{draft:?} did not name /chat");
            assert!(
                line.contains("after it"),
                "{draft:?} did not say commands take nothing after them"
            );
        }
    }

    #[test]
    fn nothing_but_a_refusal_has_a_line_to_say() {
        // A command and a message announce nothing: warlock speaks on the card
        // only when it has refused to do what was asked.
        for draft in ["/brief", "/write", "/chat", "why nine passes?", ""] {
            assert_eq!(
                submitted_for(draft).refusal(),
                None,
                "{draft:?} should have nothing to say"
            );
        }
    }

    #[test]
    fn a_draft_of_nothing_is_a_message() {
        // Total for the sake of being total: the composer never offers one up.
        for draft in ["", "   ", "\n", " \t \n "] {
            assert_eq!(submitted_for(draft), Submitted::Message);
        }
    }
}
