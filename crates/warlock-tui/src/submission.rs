//! Matching is case-sensitive, so `/BRIEF` is refused rather than folded. The
//! two mistakes do not cost the same: a refusal costs one line on the card,
//! read and corrected in a second, while a send costs a turn — a question to
//! the model, money, and a wait — to discover that warlock guessed. Where the
//! costs are that lopsided the cheap failure is the right one, and it keeps the
//! set of things that are commands small and stateable rather than a family of
//! spellings nobody can enumerate.
//!
//! `Submitted::refusal` is the entire discovery mechanism, which is why there
//! is no `/help`: a fourth command whose job was to print the list would be a
//! thing to discover before you could discover anything. One line, because the
//! card it lands on is where the reader's own words and the model's answers
//! are, and warlock speaking there at length would be warlock talking over
//! them.
//!
//! Nothing here carries the draft's text, exactly as `Composed::Submit` carries
//! none: the draft is already in the caller's hand, so a copy here would be a
//! second version of what was typed, free to disagree with the first. And
//! nothing here enters a mode, writes a file or opens a turn — what a
//! recognised command *does* belongs to the loop that owns the thread.

// Stated once, here, because it is the only place warlock says which commands
// exist: a second copy of this sentence in the loop or in a test fixture would
// be a second list to keep true.
const REFUSAL: &str = "warlock has three commands — /brief, /write and /chat — and none of them takes anything after it.";

// Five variants and no sixth for "empty", because an empty draft never gets
// here — `Composer::is_submittable` declines to offer one up — and a function
// that is total anyway is worth more than a variant every caller must match on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Submitted {
    Brief,
    Write,
    Chat,
    Message,
    // An unknown word, a bare `/`, or a command word with something after it.
    // It never reaches the model: the point of refusing rather than sending is
    // that a typo costs a line and not a turn.
    Refused,
}

impl Submitted {
    // A method rather than a bare constant, so the caller asks the value what
    // it has to say instead of asking itself which variant deserves a line.
    #[must_use]
    pub const fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Refused => Some(REFUSAL),
            _ => None,
        }
    }
}

// The order is the whole design: trim, then the first token, then "does it start
// with a slash", then "is it a path", and only then the match against the three
// words. Everything not caught by one of the first four steps is a message, so a
// draft of ordinary prose is never examined at all.
//
// Trailing trim is not tidiness. `"/brief "` is how the command is actually
// typed, because a space after a word is what a hand does before it notices
// there is no second word; a `/brief` that worked and a `/brief ` that was
// refused would be one invisible character deciding between a mode change and a
// complaint.
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
