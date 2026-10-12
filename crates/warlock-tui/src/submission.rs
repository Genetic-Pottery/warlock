//! Matching is case-sensitive, so `/BRIEF` is refused rather than folded. The
//! two mistakes do not cost the same: a refusal costs one line on the card,
//! read and corrected in a second, while a send costs a turn — a question to
//! the model, money, and a wait — to discover that warlock guessed. Where the
//! costs are that lopsided the cheap failure is the right one.
//!
//! There is no `/help`: a command whose job was to print the list would be a
//! thing to discover before you could discover anything, so the list is on the
//! `?` sheet and in `Submitted::refusal`, and both are drawn from [`COMMANDS`].
//! A command is added there and nowhere else in this file. Nothing here enters a
//! mode, writes a file or opens a turn.

// Named outside the table for the lines that echo or offer a command back: the
// card's row for a `/brief` is spelled from here rather than from the draft,
// because `submitted_for` trims and `"  /brief  "` has to draw as `/brief`.
pub(crate) const BRIEF: &str = "/brief";
pub(crate) const WRITE: &str = "/write";
pub(crate) const CHAT: &str = "/chat";
pub(crate) const PULL: &str = "/pull";

// One row per command word, and the only place warlock says which commands
// exist: the parse, the refusal sentence and the `?` sheet's rows are all read
// off it, so none of the three can name a command the others do not.
struct Command {
    word: &'static str,
    // Handed everything after the word, trimmed; `None` is a refusal.
    read: fn(&str) -> Option<Submitted>,
    takes: Option<&'static str>,
    // A word can leave this empty when another word's row already names it.
    help: &'static [(&'static str, &'static str)],
}

const COMMANDS: &[Command] = &[
    Command {
        word: BRIEF,
        read: |after| after.is_empty().then_some(Submitted::Brief),
        takes: None,
        help: &[("/brief  /write  /chat", "talk a brief into a document")],
    },
    Command {
        word: WRITE,
        read: |after| after.is_empty().then_some(Submitted::Write),
        takes: None,
        help: &[],
    },
    Command {
        word: CHAT,
        read: |after| after.is_empty().then_some(Submitted::Chat),
        takes: None,
        help: &[],
    },
    Command {
        word: "/push",
        read: read_push,
        takes: Some("a scope and a brief for /push"),
        help: &[("/push SCOPE PATH", "file a brief as a project")],
    },
    Command {
        word: "/draft",
        read: read_draft,
        takes: Some("a scope and optionally a project's slug for /draft"),
        help: &[
            ("/draft SCOPE", "list planned projects"),
            ("/draft SCOPE SLUG", "cut a project into issues"),
        ],
    },
    Command {
        word: PULL,
        read: read_pull,
        takes: Some("a scope and optionally a ticket for /pull"),
        help: &[("/pull SCOPE [TICKET]", "work a ticket to a pull request")],
    },
    Command {
        word: "/resume",
        read: read_resume,
        takes: Some("a ticket for /resume"),
        help: &[("/resume TICKET", "pick up a halted pull")],
    },
    Command {
        word: "/branch",
        read: read_branch,
        takes: Some(
            "`use <BRANCH>` or `clear` for /branch, which on its own names the branch pulls start from",
        ),
        help: &[
            ("/branch", "name the branch pulls go into"),
            ("/branch use BRANCH", "pull into BRANCH instead"),
            ("/branch clear", "pull into the remote's default"),
        ],
    },
    Command {
        word: "/sigil",
        read: read_sigil,
        takes: Some(
            "`add <NAME>...`, `remove <NAME>...` or `clear` for /sigil, which on its own names the sigils held",
        ),
        help: &[
            ("/sigil", "name the sigils held"),
            ("/sigil add NAME...", "hold these sigils too"),
            ("/sigil remove NAME...", "stop holding these"),
            ("/sigil clear", "hold no sigils"),
        ],
    },
];

pub(crate) fn help_rows() -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    COMMANDS.iter().flat_map(|command| command.help)
}

fn refusal() -> String {
    let words: Vec<&str> = COMMANDS.iter().map(|command| command.word).collect();
    let takes: Vec<&str> = COMMANDS
        .iter()
        .filter_map(|command| command.takes)
        .collect();
    format!(
        "warlock's commands are {}, and the ones that take arguments take {}.",
        listed(&words, " and "),
        listed(&takes, ", and "),
    )
}

fn listed(items: &[&str], last: &str) -> String {
    match items {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [init @ .., tail] => format!("{}{last}{tail}", init.join(", ")),
    }
}

// Fields rather than the second half of a tuple, because one of the two is
// optional and the other is not: `/pull <SCOPE>` takes the next ticket in the
// scope and `/pull <SCOPE> <TICKET>` takes the one named, and a caller reading
// `Some(("warlock-team", None))` has nothing but the order to tell it which
// position is the scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Taking {
    pub scope: String,
    pub ticket: Option<String>,
}

// The path is as typed: the manifest's spelling of it is made by whoever holds
// the repository root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ToPush {
    pub scope: String,
    pub path: String,
}

// `use` and `clear` are `warlock branch`'s own words, so the chat bar and the
// shell spell the one setting the same way.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Branching {
    Show,
    Use(String),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Sigiling {
    Show,
    Add(Vec<String>),
    Remove(Vec<String>),
    Clear,
}

// Without a slug, the planned projects are listed instead.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ToDraft {
    pub scope: String,
    pub project: Option<String>,
}

/// The commands that are not about the conversation, handed up whole for the
/// loop to answer: every rule any of them has is about a manifest, a queue, a
/// working tree or a file under the home, and none of those is the chat's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Wanted {
    Push(ToPush),
    Draft(ToDraft),
    // `None` is `/pull` on its own, which is the one command word that is worth
    // more than a refusal without its argument: the scopes this machine holds
    // are the answer to it, this module cannot ask for them, so a bare `/pull`
    // goes up to be answered with them named rather than with the list of
    // commands.
    Pull(Option<Taking>),
    // Not optional, because a `/resume` with no ticket names nothing warlock
    // could offer back — the refusal already says a ticket goes after the word.
    Resume(String),
    Branch(Branching),
    Sigil(Sigiling),
}

// No variant for "empty", because an empty draft never gets here —
// `Composer::is_submittable` declines to offer one up — and a function that is
// total anyway is worth more than a variant every caller must match on.
//
// Owned rather than borrowed from the draft: the submit that reads a draft
// empties the field, and what goes up to the loop outlives that.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Submitted {
    Brief,
    Write,
    Chat,
    Wanted(Wanted),
    Message,
    // An unknown word, a bare `/`, a command word with a second line under it,
    // and any argument [`COMMANDS`] does not read. It never reaches the model:
    // the point of refusing rather than sending is that a typo costs a line and
    // not a turn.
    Refused,
}

impl Submitted {
    // A method rather than a bare function, so the caller asks the value what
    // it has to say instead of asking itself which variant deserves a line.
    #[must_use]
    pub(crate) fn refusal(&self) -> Option<String> {
        match self {
            Self::Refused => Some(refusal()),
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
pub(crate) fn submitted_for(draft: &str) -> Submitted {
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

    // Case-sensitively, so `/BRIEF` is refused: see the module docs.
    let Some(command) = COMMANDS.iter().find(|command| command.word == word) else {
        return Submitted::Refused;
    };

    // Asked of the whole of what follows the word, before any reader trims it:
    // the line break is as often the one *before* the argument as one inside
    // it. A command word with a paragraph under it is somebody expecting the
    // paragraph to be read, and no path, scope or ticket has a newline in it.
    let after = &draft[word.len()..];
    if after.contains('\n') {
        return Submitted::Refused;
    }

    (command.read)(after.trim()).unwrap_or(Submitted::Refused)
}

// The scope first and then the path, taken whole rather than as a second token,
// so a path with a space in it arrives as the path it is. A `/push` short of
// either is refused rather than guessed at: a push files a project nothing here
// can take back.
fn read_push(after: &str) -> Option<Submitted> {
    let (scope, path) = after.split_once(char::is_whitespace)?;
    Some(Submitted::Wanted(Wanted::Push(ToPush {
        scope: scope.to_owned(),
        path: path.trim().to_owned(),
    })))
}

// The commands whose arguments are names rather than paths: a scope, a slug, a
// ticket identifier, a branch and a sigil are single words, so they are read as
// words and a word too many is a refusal rather than the tail of the one before.
// Reading them the way a path is read would make `/pull warlock-team WAR-1` one
// scope called `warlock-team WAR-1`, which exists nowhere and would be refused a
// screen later by whatever went looking for it.
fn words(after: &str) -> Vec<&str> {
    after.split_whitespace().collect()
}

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

fn read_draft(after: &str) -> Option<Submitted> {
    let drafting = match words(after).as_slice() {
        [scope] => ToDraft {
            scope: (*scope).to_owned(),
            project: None,
        },
        [scope, project] => ToDraft {
            scope: (*scope).to_owned(),
            project: Some((*project).to_owned()),
        },
        _ => return None,
    };
    Some(Submitted::Wanted(Wanted::Draft(drafting)))
}

fn read_pull(after: &str) -> Option<Submitted> {
    let taking = match words(after).as_slice() {
        [] => None,
        [scope] => Some(Taking {
            scope: (*scope).to_owned(),
            ticket: None,
        }),
        [scope, ticket] => Some(Taking {
            scope: (*scope).to_owned(),
            ticket: Some((*ticket).to_owned()),
        }),
        _ => return None,
    };
    Some(Submitted::Wanted(Wanted::Pull(taking)))
}

fn read_resume(after: &str) -> Option<Submitted> {
    match words(after).as_slice() {
        [ticket] => Some(Submitted::Wanted(Wanted::Resume((*ticket).to_owned()))),
        _ => None,
    }
}

fn read_branch(after: &str) -> Option<Submitted> {
    let branching = match words(after).as_slice() {
        [] => Branching::Show,
        ["use", name] => Branching::Use((*name).to_owned()),
        ["clear"] => Branching::Clear,
        _ => return None,
    };
    Some(Submitted::Wanted(Wanted::Branch(branching)))
}

fn read_sigil(after: &str) -> Option<Submitted> {
    let sigiling = match words(after).as_slice() {
        [] => Sigiling::Show,
        ["add", names @ ..] if !names.is_empty() => Sigiling::Add(owned(names)),
        ["remove", names @ ..] if !names.is_empty() => Sigiling::Remove(owned(names)),
        ["clear"] => Sigiling::Clear,
        _ => return None,
    };
    Some(Submitted::Wanted(Wanted::Sigil(sigiling)))
}

#[cfg(test)]
#[path = "tests/submission.rs"]
mod tests;
