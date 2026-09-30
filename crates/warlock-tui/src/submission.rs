//! Matching is case-sensitive, so `/BRIEF` is refused rather than folded. The
//! two mistakes do not cost the same: a refusal costs one line on the card,
//! read and corrected in a second, while a send costs a turn — a question to
//! the model, money, and a wait — to discover that warlock guessed. Where the
//! costs are that lopsided the cheap failure is the right one.
//!
//! `Submitted::refusal` is the entire discovery mechanism, which is why there
//! is no `/help`: a command whose job was to print the list would be a thing to
//! discover before you could discover anything. Nothing here *copies* the
//! draft's text: the four argument-taking variants borrow the argument out of
//! it, so there is still one owner and no second string free to disagree with
//! the first. And nothing here enters a mode, writes a file or opens a turn.

// Stated once, here, because it is the only place warlock says which commands
// exist: a second copy of this sentence in the loop or in a test fixture would
// be a second list to keep true.
const REFUSAL: &str = "warlock has seven commands — /brief, /write, /chat, /push, /draft, /pull and /resume — and four of them take an argument: a brief for /push and /draft, a scope and optionally a ticket for /pull, and a ticket for /resume.";

// Fields rather than the second half of a tuple, because one of the two is
// optional and the other is not: `/pull <SCOPE>` takes the next ticket in the
// scope and `/pull <SCOPE> <TICKET>` takes the one named, and a caller reading
// `Some(("warlock-team", None))` has nothing but the order to tell it which
// position is the scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Taking<'a> {
    pub scope: &'a str,
    pub ticket: Option<&'a str>,
}

// Nine variants and no tenth for "empty", because an empty draft never gets
// here — `Composer::is_submittable` declines to offer one up — and a function
// that is total anyway is worth more than a variant every caller must match on.
//
// The borrows in `Push`, `Cut`, `Pull` and `Resume` are what keep the module's
// promise that nothing here copies the draft: each argument is a slice of the
// draft the caller still owns, and `&str` is `Copy`, so the value stays as cheap
// to pass around as it was when no variant carried anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Submitted<'a> {
    Brief,
    Write,
    Chat,
    // `None` is `/push` on its own, which files what this session wrote.
    Push(Option<&'a str>),
    // `None` is `/draft` on its own, which cuts what this session wrote.
    Cut(Option<&'a str>),
    // `None` is `/pull` on its own, which is the one command word that is worth
    // more than a refusal without its argument: the scopes this machine holds
    // are the answer to it, this module cannot ask for them, so a bare `/pull`
    // goes up to be answered with them named rather than with the list below.
    Pull(Option<Taking<'a>>),
    // Not optional, because a `/resume` with no ticket names nothing warlock
    // could offer back — the refusal already says a ticket goes after the word.
    Resume(&'a str),
    Message,
    // An unknown word, a bare `/`, one of the three command words that take
    // nothing with something after it, a `/resume` with no ticket, and an
    // argument with more words in it than the command has places to put them. It
    // never reaches the model: the point of refusing rather than sending is that
    // a typo costs a line and not a turn.
    Refused,
}

impl Submitted<'_> {
    // A method rather than a bare constant, so the caller asks the value what
    // it has to say instead of asking itself which variant deserves a line.
    #[must_use]
    pub(crate) const fn refusal(self) -> Option<&'static str> {
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
pub(crate) fn submitted_for(draft: &str) -> Submitted<'_> {
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

    // The two commands whose argument is a path, because they are the commands
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

    // The two commands about a ticket, whose arguments are names rather than
    // paths: a scope and a ticket identifier are single words, so they are read
    // as words and a third word is a refusal rather than the tail of the second.
    // Reading them the way a path is read would make `/pull warlock-team WAR-1`
    // one scope called `warlock-team WAR-1`, which exists nowhere and would be
    // refused a screen later by whatever went looking for it.
    //
    // The line break is disqualifying here for `PUSH | CUT`'s reason and not for
    // a reason of its own: a command word with a paragraph under it is somebody
    // expecting the paragraph to be read.
    if matches!(word, PULL | RESUME) && !after.contains('\n') {
        let mut words = after.split_whitespace();
        let (first, second) = (words.next(), words.next());
        if words.next().is_some() {
            return Submitted::Refused;
        }

        if word == PULL {
            return Submitted::Pull(first.map(|scope| Taking {
                scope,
                ticket: second,
            }));
        }
        return match (first, second) {
            (Some(ticket), None) => Submitted::Resume(ticket),
            _ => Submitted::Refused,
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
const PULL: &str = "/pull";
const RESUME: &str = "/resume";

#[cfg(test)]
#[path = "tests/submission.rs"]
mod tests;
