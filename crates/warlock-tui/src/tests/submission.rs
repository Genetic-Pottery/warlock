use super::{Submitted, submitted_for};

#[test]
fn each_command_word_is_its_own_command() {
    assert_eq!(submitted_for("/brief"), Submitted::Brief);
    assert_eq!(submitted_for("/write"), Submitted::Write);
    assert_eq!(submitted_for("/chat"), Submitted::Chat);
    assert_eq!(submitted_for("/push"), Submitted::Push(None));
}

#[test]
fn push_takes_the_brief_to_file_after_it() {
    // The command a colleague types about a document committed days ago, in a
    // session that has written nothing of its own.
    assert_eq!(
        submitted_for("/push docs/warlock-brief-22-push.md"),
        Submitted::Push(Some("docs/warlock-brief-22-push.md"))
    );
    // Trimmed at both ends and around the word, so the spacing a hand leaves
    // is not part of the path.
    assert_eq!(
        submitted_for("  /push   docs/a.md  "),
        Submitted::Push(Some("docs/a.md"))
    );
    // Whole rather than the next token: a path with a space in it is a path.
    assert_eq!(
        submitted_for("/push docs/a brief.md"),
        Submitted::Push(Some("docs/a brief.md"))
    );
    // And nothing after it is still the bare command, which files what this
    // session wrote.
    assert_eq!(submitted_for("/push "), Submitted::Push(None));
}

#[test]
fn a_push_with_a_second_line_is_refused() {
    // No path has a newline in it, so this is somebody typing a message under
    // a command word and expecting it to be read.
    for draft in ["/push\nsome text", "/push docs/a.md\nand a thought"] {
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
    // `home/cole/notes` is not a command word, and somebody naming a file
    // is the common case rather than the odd one.
    for draft in [
        "/home/cole/notes",
        "/home/cole/notes is stale",
        "/brief/notes",
        "/push/x",
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
    for draft in ["/breif", "/plan", "/BRIEF", "/Brief", "/PUSH", "/Push", "/"] {
        assert_eq!(
            submitted_for(draft),
            Submitted::Refused,
            "{draft:?} is not a command warlock has"
        );
    }
}

#[test]
fn a_command_word_with_anything_after_it_is_refused() {
    // `/push` is the exception and has its own tests. For the other three a
    // second line is an argument by another route: a `/brief` with a paragraph
    // under it is somebody expecting the paragraph to be read.
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
        "/push docs/a.md\nand a thought",
    ];

    for draft in refusals {
        let line = submitted_for(draft)
            .refusal()
            .expect("a refused draft has a line");

        assert!(!line.contains('\n'), "{draft:?} gave more than one line");
        assert!(line.contains("/brief"), "{draft:?} did not name /brief");
        assert!(line.contains("/write"), "{draft:?} did not name /write");
        assert!(line.contains("/chat"), "{draft:?} did not name /chat");
        assert!(line.contains("/push"), "{draft:?} did not name /push");
        assert!(
            line.contains("after it"),
            "{draft:?} did not say which command takes something after it"
        );
    }
}

#[test]
fn nothing_but_a_refusal_has_a_line_to_say() {
    // A command and a message announce nothing: warlock speaks on the card
    // only when it has refused to do what was asked.
    for draft in ["/brief", "/write", "/chat", "/push", "why nine passes?", ""] {
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
