//! Matching is case-sensitive, so `/BRIEF` is refused rather than folded. The
//! two mistakes do not cost the same: a refusal costs one line on the card,
//! read and corrected in a second, while a send costs a turn — a question to
//! the model, money, and a wait — to discover that warlock guessed. Where the
//! costs are that lopsided the cheap failure is the right one.
//!
//! `Submitted::refusal` is the entire discovery mechanism, which is why there
//! is no `/help`: a command whose job was to print the list would be a thing to
//! discover before you could discover anything. Nothing here *copies* the
//! draft's text: the two path-taking variants borrow the path out of it, so
//! there is still one owner and no second string free to disagree with the
//! first. And nothing here enters a mode, writes a file or opens a turn.

// Stated once, here, because it is the only place warlock says which commands
// exist: a second copy of this sentence in the loop or in a test fixture would
// be a second list to keep true.
const REFUSAL: &str = "warlock has five commands — /brief, /write, /chat, /push and /draft — and only /push and /draft take a path after them, the brief to file and the brief to cut.";

// Seven variants and no eighth for "empty", because an empty draft never gets
// here — `Composer::is_submittable` declines to offer one up — and a function
// that is total anyway is worth more than a variant every caller must match on.
//
// The borrows in `Push` and `Cut` are what keep the module's promise that
// nothing here copies the draft: the path is a slice of the draft the caller
// still owns, and `&str` is `Copy`, so the value stays as cheap to pass around
// as it was when no variant carried anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Submitted<'a> {
    Brief,
    Write,
    Chat,
    // `None` is `/push` on its own, which files what this session wrote.
    Push(Option<&'a str>),
    // `None` is `/draft` on its own, which cuts what this session wrote.
    Cut(Option<&'a str>),
    Message,
    // An unknown word, a bare `/`, or one of the other three command words with
    // something after it. It never reaches the model: the point of refusing
    // rather than sending is that a typo costs a line and not a turn.
    Refused,
}

impl Submitted<'_> {
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
// with a slash", then "is it a path", and only then the match against the
// command words. Everything not caught by one of the first four steps is a
// message, so a draft of ordinary prose is never examined at all.
//
// Trailing trim is not tidiness. `"/brief "` is how the command is actually
// typed, because a space after a word is what a hand does before it notices
// there is no second word; a `/brief` that worked and a `/brief ` that was
// refused would be one invisible character deciding between a mode change and a
// complaint.
#[must_use]
pub fn submitted_for(draft: &str) -> Submitted<'_> {
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

    // Untrimmed, because the line break that disqualifies a `/push` is as often
    // the one *before* the argument as one inside it, and trimming first would
    // eat it.
    let after = &draft[word.len()..];

    // The two commands that take an argument, because they are the commands
    // about a file rather than about the conversation: a brief is committed,
    // read for a day, and then filed and cut by whoever gets to it, quite
    // possibly in a session that wrote nothing and by somebody who did not
    // write it.
    //
    // One branch for the two of them, with only the variant chosen from the
    // word, rather than a branch each: a second copy of the lines below would be
    // a second place for "a path with a space in it" and "never across a
    // newline" to stop agreeing, and the two commands take the same argument.
    //
    // Taken whole rather than as a second token, so a path with a space in it
    // arrives as the path it is — but never across a line break, which is
    // somebody typing a message under a command word rather than naming a file
    // no path has a newline in.
    if matches!(word, PUSH | CUT) && !after.contains('\n') {
        let named = after.trim();
        let named = (!named.is_empty()).then_some(named);
        return if word == CUT {
            Submitted::Cut(named)
        } else {
            Submitted::Push(named)
        };
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

// Named because they are matched on before the match below rather than inside
// it, and a literal in two places is two places to change.
const PUSH: &str = "/push";
const CUT: &str = "/draft";

#[cfg(test)]
#[path = "tests/submission.rs"]
mod tests;
