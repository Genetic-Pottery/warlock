use super::{
    Branching, COMMANDS, Sigiling, Submitted, Taking, ToDraft, ToPush, Wanted, help_rows,
    submitted_for,
};

fn push(scope: &str, path: &str) -> Submitted {
    Submitted::Wanted(Wanted::Push(ToPush {
        scope: scope.to_owned(),
        path: path.to_owned(),
    }))
}

fn draft(scope: &str, project: Option<&str>) -> Submitted {
    Submitted::Wanted(Wanted::Draft(ToDraft {
        scope: scope.to_owned(),
        project: project.map(str::to_owned),
    }))
}

fn pull(taking: Option<(&str, Option<&str>)>) -> Submitted {
    Submitted::Wanted(Wanted::Pull(taking.map(|(scope, ticket)| Taking {
        scope: scope.to_owned(),
        ticket: ticket.map(str::to_owned),
    })))
}

fn branch(branching: Branching) -> Submitted {
    Submitted::Wanted(Wanted::Branch(branching))
}

fn sigil(sigiling: Sigiling) -> Submitted {
    Submitted::Wanted(Wanted::Sigil(sigiling))
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|&name| name.to_owned()).collect()
}

#[test]
fn each_command_word_is_its_own_command() {
    assert_eq!(submitted_for("/brief"), Submitted::Brief);
    assert_eq!(submitted_for("/write"), Submitted::Write);
    assert_eq!(submitted_for("/chat"), Submitted::Chat);
    // The one command word that is worth passing up without its argument: only
    // the panel knows which scopes this machine holds, so a bare `/pull` is
    // answered with them rather than with the list of commands.
    assert_eq!(submitted_for("/pull"), pull(None));
    assert_eq!(submitted_for("  /pull  "), pull(None));
}

#[test]
fn push_takes_a_scope_and_then_the_brief_to_file() {
    assert_eq!(
        submitted_for("/push warlock-team docs/warlock-brief-22-push.md"),
        push("warlock-team", "docs/warlock-brief-22-push.md")
    );
    assert_eq!(
        submitted_for("  /push   warlock-team   docs/a.md  "),
        push("warlock-team", "docs/a.md")
    );
    // The scope is a word and the path is the rest: a path with a space in it
    // is a path.
    assert_eq!(
        submitted_for("/push warlock-team docs/a brief.md"),
        push("warlock-team", "docs/a brief.md")
    );
    // Short of either is no push: nothing is inferred, least of all the last
    // document this session wrote.
    for draft in [
        "/push",
        "/push ",
        "/push warlock-team",
        "/push warlock-team  ",
    ] {
        assert_eq!(
            submitted_for(draft),
            Submitted::Refused,
            "{draft:?} is a push short of its scope or its brief"
        );
    }
}

#[test]
fn draft_takes_a_scope_and_then_an_optional_slug() {
    assert_eq!(
        submitted_for("/draft warlock-team"),
        draft("warlock-team", None)
    );
    assert_eq!(
        submitted_for("  /draft   warlock-team   9e41c07a2b13  "),
        draft("warlock-team", Some("9e41c07a2b13"))
    );
    for draft in ["/draft", "/draft ", "/draft warlock-team 9e41c07a2b13 now"] {
        assert_eq!(
            submitted_for(draft),
            Submitted::Refused,
            "{draft:?} is not a scope and at most one slug"
        );
    }
}

#[test]
fn pull_takes_a_scope_and_then_an_optional_ticket() {
    assert_eq!(
        submitted_for("/pull warlock-team"),
        pull(Some(("warlock-team", None)))
    );
    // The second word names the ticket to take instead of the next one in the
    // scope, which is the spelling `/resume` leaves in the composer.
    assert_eq!(
        submitted_for("/pull warlock-team WAR-143"),
        pull(Some(("warlock-team", Some("WAR-143"))))
    );
    assert_eq!(
        submitted_for("  /pull   warlock-team   WAR-143  "),
        pull(Some(("warlock-team", Some("WAR-143"))))
    );
    // A scope and a ticket are words, not paths, so a third word is a refusal
    // rather than the tail of the second: `warlock-team WAR-143` is not a scope
    // anybody has, and the refusal costs a line where going looking for it
    // costs a screen.
    for draft in ["/pull warlock-team WAR-143 now", "/pull one two three four"] {
        assert_eq!(
            submitted_for(draft),
            Submitted::Refused,
            "{draft:?} has more words than /pull has places"
        );
    }
}

#[test]
fn resume_takes_the_ticket_to_release_after_it() {
    assert_eq!(
        submitted_for("/resume WAR-143"),
        Submitted::Wanted(Wanted::Resume("WAR-143".to_owned()))
    );
    assert_eq!(
        submitted_for("  /resume   WAR-143  "),
        Submitted::Wanted(Wanted::Resume("WAR-143".to_owned()))
    );
    // Unlike `/pull`, a bare `/resume` is refused here: the ticket it is missing
    // is not something warlock could offer back, and the refusal says a ticket
    // goes after the word.
    for draft in ["/resume", "/resume ", "/resume WAR-143 WAR-144"] {
        assert_eq!(
            submitted_for(draft),
            Submitted::Refused,
            "{draft:?} is not one ticket after the word"
        );
    }
}

#[test]
fn a_push_or_a_cut_with_a_second_line_is_refused() {
    // No path has a newline in it, so this is somebody typing a message under
    // a command word and expecting it to be read.
    for draft in [
        "/push\nsome text",
        "/push warlock-team docs/a.md\nand a thought",
        "/push warlock-team\ndocs/a.md",
        "/draft\nsome text",
        "/draft warlock-team\nand a thought",
    ] {
        assert_eq!(
            submitted_for(draft),
            Submitted::Refused,
            "{draft:?} is a command with a message under it"
        );
    }
}

#[test]
fn a_pull_or_a_resume_with_a_second_line_is_refused() {
    // Including the bare `/pull`, which is otherwise the one command word that
    // passes up without an argument: with a paragraph under it, it is somebody
    // expecting the paragraph to be read.
    for draft in [
        "/pull\nsome text",
        "/pull warlock-team\nand a thought",
        "/pull warlock-team\nWAR-143",
        "/resume\nsome text",
        "/resume WAR-143\nand a thought",
    ] {
        assert_eq!(
            submitted_for(draft),
            Submitted::Refused,
            "{draft:?} is a command with a message under it"
        );
    }
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
    // `src/lib.rs` is not a command word, and somebody naming a file
    // is the common case rather than the odd one.
    for draft in [
        "/src/lib.rs",
        "/src/lib.rs is stale",
        "/brief/notes",
        "/push/x",
        "/draft/x",
        "/pull/x",
        "/resume/x",
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
fn a_word_that_is_not_a_command_is_refused() {
    // A typo, a command another program has, the right word in the wrong
    // case, and the bare slash somebody types to find out what exists.
    // The case-folded spellings refuse with an argument too, since the word
    // that would have taken one was never a command word.
    for draft in [
        "/breif",
        "/plan",
        "/BRIEF",
        "/Brief",
        "/PUSH",
        "/Push",
        "/CUT",
        "/Draft",
        "/CUT docs/a.md",
        "/Draft docs/a.md",
        "/PULL",
        "/Pull warlock-team",
        "/RESUME WAR-143",
        "/Resume WAR-143",
        "/",
    ] {
        assert_eq!(
            submitted_for(draft),
            Submitted::Refused,
            "{draft:?} is not a command warlock has"
        );
    }
}

#[test]
fn a_command_word_with_anything_after_it_is_refused() {
    // `/push` and `/draft` are the exceptions and have their own tests. For the
    // other three a second line is an argument by another route: a `/brief`
    // with a paragraph under it is somebody expecting the paragraph to be read.
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
    let refusals = [
        "/breif",
        "/plan",
        "/BRIEF",
        "/",
        "/brief now",
        "/brief\nx",
        "/PUSH",
        "/push warlock-team docs/a.md\nand a thought",
        "/push",
        "/push warlock-team",
        "/CUT",
        "/draft",
        "/draft warlock-team 9e41c07a2b13 now",
        "/resume",
        "/pull warlock-team WAR-143 now",
        "/resume WAR-143 WAR-144",
        "/pull warlock-team\nand a thought",
        "/branch dev",
        "/sigil docs",
        "/sigil add",
    ];

    for draft in refusals {
        let line = submitted_for(draft)
            .refusal()
            .expect("a refused draft has a line");

        assert!(!line.contains('\n'), "{draft:?} gave more than one line");
        for command in [
            "/brief", "/write", "/chat", "/push", "/draft", "/pull", "/resume", "/branch", "/sigil",
        ] {
            assert!(
                line.contains(command),
                "{draft:?} did not name {command}, one of warlock's nine commands"
            );
        }
        // The list on its own would leave six of the nine looking like `/brief`
        // and send somebody to a shell to find out what `/pull` wants.
        for said in [
            "a scope and a brief for /push",
            "a scope and optionally a project's slug for /draft",
            "a scope and optionally a ticket for /pull",
            "a ticket for /resume",
            "`use <BRANCH>` or `clear` for /branch",
            "`add <NAME>...`, `remove <NAME>...` or `clear` for /sigil",
        ] {
            assert!(
                line.contains(said),
                "{draft:?} did not say {said}, one of the arguments the commands take"
            );
        }
    }
}

#[test]
fn nothing_but_a_refusal_has_a_line_to_say() {
    // A command and a message announce nothing: warlock speaks on the card
    // only when it has refused to do what was asked.
    for draft in [
        "/brief",
        "/write",
        "/chat",
        "/push warlock-team docs/a.md",
        "/draft warlock-team",
        "/draft warlock-team 9e41c07a2b13",
        "/pull",
        "/pull warlock-team",
        "/pull warlock-team WAR-143",
        "/resume WAR-143",
        "why nine passes?",
        "",
    ] {
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

#[test]
fn branch_reads_its_three_spellings_and_refuses_the_rest() {
    assert_eq!(submitted_for("/branch"), branch(Branching::Show));
    assert_eq!(
        submitted_for("/branch use release/next "),
        branch(Branching::Use("release/next".to_owned()))
    );
    assert_eq!(submitted_for("/branch clear"), branch(Branching::Clear));
    for draft in [
        "/branch dev",
        "/branch use",
        "/branch use a b",
        "/branch clear now",
        "/branch use dev\nand a thought",
    ] {
        assert_eq!(submitted_for(draft), Submitted::Refused, "{draft:?}");
    }
}

#[test]
fn sigil_reads_its_four_spellings_and_refuses_the_rest() {
    assert_eq!(submitted_for("/sigil"), sigil(Sigiling::Show));
    assert_eq!(
        submitted_for("/sigil add a  b "),
        sigil(Sigiling::Add(names(&["a", "b"])))
    );
    assert_eq!(
        submitted_for("/sigil remove a"),
        sigil(Sigiling::Remove(names(&["a"])))
    );
    assert_eq!(submitted_for("/sigil clear"), sigil(Sigiling::Clear));
    for draft in [
        "/sigil add",
        "/sigil remove ",
        "/sigil clear x",
        "/sigil hold a",
        "/sigil a",
        "/sigil add a\nand a thought",
    ] {
        assert_eq!(submitted_for(draft), Submitted::Refused, "{draft:?}");
    }
}

#[test]
fn every_command_is_on_the_help_sheet() {
    for command in COMMANDS {
        assert!(
            help_rows().any(|(keys, _)| keys.split_whitespace().any(|key| key == command.word)),
            "{} has no row on the help sheet",
            command.word
        );
    }
}

#[test]
fn every_command_has_a_row_in_controls() {
    let controls = include_str!("../../../../CONTROLS.md");
    for command in COMMANDS {
        let row = format!("| `{}", command.word);
        assert!(
            controls
                .lines()
                .any(|line| line.starts_with(&row) && line[row.len()..].starts_with(['`', ' '])),
            "CONTROLS.md has no row for {}",
            command.word
        );
    }
}
