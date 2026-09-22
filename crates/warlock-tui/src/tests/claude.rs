use std::fmt::Write as _;
use std::io;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::stream;
use super::{
    Activities, Activity, BRIEF_EFFORT, BRIEF_MODEL, CHAT_INSTRUCTION, CHAT_SYSTEM_PROMPT, Cancel,
    ChatAgent, ClaudeAgent, Converses, DRAFT_NOW_INSTRUCTION, DRAFTING_CONTRACT,
    DRAFTING_ONE_SHOT_CONTRACT, DRAFTING_ROUNDS, Drafted, Drafting, EFFORT, EFFORT_VAR,
    INVOCATION_TIMEOUT, MODEL, MODEL_VAR, NOTHING_SETTLES_IT, OsString, PROPOSING_SYSTEM_PROMPT,
    Replied, SYSTEM_PROMPT, WRITE_INSTRUCTION, Wired, brief_instruction, drafting_opening,
    or_default, overridden, proposing_instruction, render, session_id,
};
use crate::brief::scope_block_in;
use crate::panel::Mode;
use crate::template::DEFAULT_TEMPLATE;
use warlock_engine::{Agent, agent, drafting};

// A name no directory on `PATH` can hold, so the lookup fails the way it does on
// a machine with no `claude` installed.
const NOT_A_PROGRAM: &str = "warlock-test-no-such-program-8f3a1c";

fn words(vector: &[OsString]) -> Vec<String> {
    vector
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn args(agent: &ClaudeAgent) -> Vec<String> {
    words(agent.args())
}

fn turn_args(agent: &ChatAgent) -> Vec<String> {
    words(&agent.args())
}

// The vector is flags and values in pairs, so a value is the word after its flag.
// Asked this way rather than by index, a test says what it is about instead of
// where the argument happens to sit.
fn value_of<'a>(vector: &'a [String], flag: &str) -> Option<&'a str> {
    let named = vector.iter().position(|word| word == flag)?;
    vector.get(named + 1).map(String::as_str)
}

#[test]
fn the_defaults_are_the_real_thing() {
    let agent = ClaudeAgent::new();

    assert_eq!(agent.program(), "claude");
    assert_eq!(agent.timeout(), INVOCATION_TIMEOUT);
    assert_eq!(
        INVOCATION_TIMEOUT.as_secs(),
        300,
        "five minutes, per invocation"
    );
    // Exactly this, in this order: print mode, the streaming output format,
    // the `--verbose` the CLI insists on before it will stream at all, and
    // then the three answers a pact refuses to inherit — which model, how
    // hard it thinks, and what it may reach for.
    assert_eq!(
        args(&agent),
        [
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--model",
            "claude-sonnet-5",
            "--effort",
            "low",
            "--tools",
            "",
            "--system-prompt",
            SYSTEM_PROMPT,
            "--setting-sources",
            "",
        ],
        "a pass names its own terms rather than taking the reader's",
    );
    assert_eq!(args(&ClaudeAgent::default()), args(&agent));
    assert_eq!(ClaudeAgent::default().timeout(), agent.timeout());
}

#[test]
fn a_rendered_request_carries_the_files_it_was_given() {
    // The regression this file exists to never repeat. A request holds its
    // prompt, its files and its children's documents as three separate
    // things; stdin is one stream; and for a long time only the first of
    // the three reached it. The pass then read a prompt telling it that it
    // had been given this directory's files, found none, and wrote a
    // WARLOCK.md saying so — which passed the length floor and was granted.
    let request =
        agent::Request::new("describe this directory", "/repo/crates/engine").with_files(vec![
            agent::File::present("src/lib.rs", &b"//! Core engine.\n"[..]),
            agent::File::present("Cargo.toml", &b"[package]\n"[..]),
        ]);

    let rendered = render(&request);

    assert!(
        rendered.starts_with("describe this directory"),
        "{rendered}"
    );
    for said in ["src/lib.rs", "//! Core engine.", "Cargo.toml", "[package]"] {
        assert!(rendered.contains(said), "{said:?} is missing:\n{rendered}");
    }
    // And the guard between the instructions and the repository's own text
    // is in front of all of it.
    let guard = rendered.find("---").expect("a guard line");
    assert!(guard < rendered.find("//! Core engine.").expect("the file"));
}

#[test]
fn each_of_a_files_states_renders_as_the_prompt_says_it_will() {
    let request = agent::Request::new("describe this directory", "/repo").with_files(vec![
        agent::File::present("small.rs", &b"fn small() {}\n"[..]),
        agent::File::omitted("huge.bin", 4_200_000),
    ]);

    let rendered = render(&request);

    // Sent whole: its text is there.
    assert!(rendered.contains("fn small() {}"), "{rendered}");
    // Left out: its name and its size, and nothing pretending to be its
    // contents.
    assert!(
        rendered.contains("huge.bin (4200000 bytes, contents not sent)"),
        "{rendered}"
    );
}

#[test]
fn bytes_that_are_not_text_are_named_rather_than_mangled_onto_stdin() {
    // A directory holds whatever is in it, and `agent::File` carries bytes on
    // purpose. There is no way to put a PNG on stdin as text, and a lossy
    // conversion would send a screenful of replacement characters that a
    // pass could only describe as the file's contents — so a file that is
    // not text renders as one that was not sent.
    let request = agent::Request::new("describe this directory", "/repo").with_files(vec![
        agent::File::present("logo.png", &[0x89, b'P', b'N', b'G', 0xFF, 0xFE][..]),
    ]);

    let rendered = render(&request);

    assert!(
        rendered.contains("logo.png (6 bytes, not text)"),
        "{rendered}"
    );
    assert!(
        !rendered.contains('\u{FFFD}'),
        "no replacement characters reached the prompt:\n{rendered}"
    );
}

#[test]
fn a_childs_document_is_carried_under_the_directory_it_belongs_to() {
    let request = agent::Request::new("describe this directory", "/repo/crates/engine")
        .with_child_documents(vec![agent::ChildDocument::new(
            "src",
            "# src\n\nThe engine's modules.\n",
        )]);

    let rendered = render(&request);

    assert!(rendered.contains("the WARLOCK.md of src"), "{rendered}");
    assert!(rendered.contains("The engine's modules."), "{rendered}");
}

#[test]
fn a_rendered_request_names_the_directory_it_is_about() {
    // The pass cannot see its own working directory — asked outright, it
    // answers with the repository root — and the prompt tells it to head
    // the document with the directory's name. So the request says it.
    let request = agent::Request::new("describe this directory", "/repo/crates/engine/src")
        .with_files(vec![agent::File::present("lib.rs", &b"//! Engine.\n"[..])]);

    let rendered = render(&request);

    assert!(rendered.contains("named `src`"), "{rendered}");
    // The name, not the path: an absolute path is the reader's home
    // directory, and it would be committed inside the document.
    assert!(!rendered.contains("/repo/crates"), "{rendered}");
}

#[test]
fn a_request_with_nothing_attached_is_its_prompt_and_no_more() {
    // What every map and reduce pass is: the engine has already written the
    // chunk into the prompt, so there is nothing here to lay out and
    // nothing to say about laying it out.
    let request = agent::Request::new("summarise this part of a file: fn main() {}", "/repo");

    assert_eq!(
        render(&request),
        "summarise this part of a file: fn main() {}"
    );
}

#[test]
fn an_override_takes_over_only_when_it_says_something() {
    // Unset and exported-but-blank are the same answer — the reader has
    // not chosen — and the constant stands.
    assert_eq!(or_default(None, MODEL), MODEL);
    assert_eq!(or_default(Some(OsString::new()), MODEL), MODEL);

    // Anything else is theirs, carried through exactly as written: this is
    // the one thing here that is not warlock's to decide, so it is not
    // warlock's to correct either. An unknown level or a misspelt model is
    // the CLI's to reject, with its own message, rather than something
    // this file second-guesses against a list it would have to keep.
    assert_eq!(or_default(Some(OsString::from("opus")), MODEL), "opus");
    assert_eq!(or_default(Some(OsString::from("max")), EFFORT), "max");
    assert_eq!(
        or_default(Some(OsString::from("no-such-model")), MODEL),
        "no-such-model",
    );
}

// What `--session-id` accepts, checked by hand because there is no `uuid` crate
// here: thirty-six characters, dashes in the four places, lowercase hex
// elsewhere, version nibble `4` and variant nibble one of `8`, `9`, `a`, `b`.
fn is_uuid_shaped(id: &str) -> bool {
    let characters: Vec<char> = id.chars().collect();
    if characters.len() != 36 {
        return false;
    }
    let dashes = [8, 13, 18, 23];
    for (index, character) in characters.iter().enumerate() {
        let ok = if dashes.contains(&index) {
            *character == '-'
        } else {
            character.is_ascii_hexdigit() && !character.is_ascii_uppercase()
        };
        if !ok {
            return false;
        }
    }
    // Segment lengths, said as themselves rather than inferred from where
    // the dashes were found above.
    let segments: Vec<usize> = id.split('-').map(str::len).collect();
    segments == [8, 4, 4, 4, 12]
        && characters[14] == '4'
        && matches!(characters[19], '8' | '9' | 'a' | 'b')
}

#[test]
fn a_session_id_is_shaped_like_the_uuid_the_cli_demands() {
    let id = session_id();

    assert!(is_uuid_shaped(&id), "not UUID-shaped: {id}");
    // The hand-rolled check is only worth trusting if it rejects things, so
    // say what it rejects.
    assert!(!is_uuid_shaped(""));
    assert!(!is_uuid_shaped(&id[..35]));
    assert!(!is_uuid_shaped(&id.to_uppercase()));
    assert!(!is_uuid_shaped(&id.replace('-', "0")));
    assert!(!is_uuid_shaped(&format!("{}5{}", &id[..14], &id[15..])));
    assert!(!is_uuid_shaped(&format!("{}c{}", &id[..19], &id[20..])));
}

#[test]
fn no_two_session_ids_are_the_same() {
    // A clock alone would not carry this: several of these are generated
    // inside one tick of a coarse timer, and it is the counter that keeps
    // them apart. Every id is checked for shape too, so a generator that
    // stayed unique by degenerating into a counter would still fail.
    let ids: std::collections::HashSet<String> = (0..500).map(|_| session_id()).collect();

    assert_eq!(ids.len(), 500, "session ids repeated within one process");
    assert!(ids.iter().all(|id| is_uuid_shaped(id)));
}

#[test]
fn a_pass_is_given_no_tools_to_reach_for() {
    // The empty string is the argument, not a missing one: `--tools ""` is
    // how the CLI is told none, and dropping the pair would hand a pass
    // the whole default set instead.
    let args = args(&ClaudeAgent::new());
    let tools = args
        .iter()
        .position(|arg| arg == "--tools")
        .expect("a pass says what it may reach for");

    assert_eq!(args.get(tools + 1).map(String::as_str), Some(""));
}

#[test]
fn a_pass_reads_nobodys_standing_instructions_and_a_turn_reads_the_projects() {
    // A pass runs inside the repository being pacted, and `claude --print`
    // would otherwise load that repository's `CLAUDE.md` into it: a pass
    // given warlock's own block wrote a stamp and a scope section of its
    // own invention. A turn keeps them, because a turn answers questions
    // about that repository and its standing instructions are context.
    let pass = args(&ClaudeAgent::new());
    assert_eq!(
        value_of(&pass, "--setting-sources"),
        Some(""),
        "no setting source at all: {pass:?}"
    );
    let turn = turn_args(&ChatAgent::new());
    assert!(
        !turn.iter().any(|arg| arg == "--setting-sources"),
        "a turn is not cut off from the project: {turn:?}"
    );
}

#[test]
fn a_turns_defaults_are_the_real_thing_too() {
    let agent = ChatAgent::new();
    let vector = turn_args(&agent);
    let session = value_of(&vector, "--session-id").expect("a turn belongs to a conversation");

    assert_eq!(agent.program(), "claude");
    assert_eq!(agent.timeout(), INVOCATION_TIMEOUT);
    assert!(is_uuid_shaped(session), "not UUID-shaped: {session}");
    // The same five leading arguments a pass has, because the transport
    // reads the same stream; then the three answers a turn refuses to
    // inherit, warlock's own system prompt, and the conversation this
    // agent's turns all belong to.
    assert_eq!(
        vector,
        [
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--model",
            "claude-sonnet-5",
            "--effort",
            "low",
            "--tools",
            "Read,Grep,Glob",
            "--system-prompt",
            CHAT_SYSTEM_PROMPT,
            "--session-id",
            session,
        ],
    );

    // A second agent is the same vector with a different conversation in
    // it, and nothing else different.
    let another = turn_args(&ChatAgent::default());
    assert_eq!(another[..another.len() - 1], vector[..vector.len() - 1]);
    assert_eq!(ChatAgent::default().timeout(), agent.timeout());
}

#[test]
fn a_turn_may_look_at_the_repository_and_do_nothing_whatever_else() {
    let agent = ChatAgent::new();

    // Both registers, one assertion: a mode is a message and an effort
    // level, and a conversation aimed at a document is granted not one tool
    // more than a question about a file is. Warlock holds the pen in both.
    for vector in [
        turn_args(&agent),
        turn_args(&agent.at_effort(BRIEF_EFFORT).at_model(BRIEF_MODEL)),
    ] {
        let granted = value_of(&vector, "--tools").expect("a turn says what it may reach for");

        // Exactly three, named rather than left to a default that could grow.
        assert_eq!(
            granted.split(',').collect::<Vec<&str>>(),
            ["Read", "Grep", "Glob"],
        );
        // Asked of the whole vector rather than of the grant alone, because
        // the grant is not the only way in: a permission flag is how a writer
        // would arrive without ever being named as a tool, and a turn carries
        // none of those either.
        for smuggled in [
            "Write",
            "Edit",
            "Bash",
            "WebFetch",
            "--permission-mode",
            "--dangerously-skip-permissions",
            "--allowed-tools",
            "--allowedTools",
            "acceptEdits",
            "bypassPermissions",
        ] {
            assert!(
                !vector.iter().any(|word| word.contains(smuggled)),
                "{smuggled:?} is somewhere in the vector a turn is run with: {vector:?}",
            );
        }
    }
}

#[test]
fn the_first_turn_opens_the_conversation_and_every_turn_after_it_resumes() {
    // `claude` opens a conversation with `--session-id` and refuses to open
    // the same one twice — `Session ID … is already in use` — so the second
    // question somebody types must arrive as `--resume` or it fails before
    // the model ever hears it. Run against a program that exists and will
    // not understand a word of what it is handed: the spawn is what claims
    // the id, and what came back is not what this is about.
    let agent = ChatAgent::new().with_program("/bin/sh");
    let opening = turn_args(&agent);
    let session = value_of(&opening, "--session-id")
        .expect("the first turn opens the conversation")
        .to_owned();

    let _ = agent.turn("what is in crates?");
    let resuming = turn_args(&agent);

    assert_eq!(
        value_of(&resuming, "--resume"),
        Some(session.as_str()),
        "the second turn opened a conversation the first one already had",
    );
    assert!(
        !resuming.iter().any(|word| word == "--session-id"),
        "a turn cannot open a session that is already in use: {resuming:?}",
    );
    // One flag apart and not one word else: the same model, the same tools
    // and the same conversation, said the way a turn after the first says
    // it.
    assert_eq!(
        resuming[..resuming.len() - 2],
        opening[..opening.len() - 2],
        "resuming a conversation changed something other than how it is named",
    );

    // And it stays that way however many turns are taken.
    let _ = agent.turn("and which of those is biggest?");
    assert_eq!(turn_args(&agent), resuming);
}

#[test]
fn a_conversation_no_child_ever_took_is_still_waiting_to_be_opened() {
    // The id is claimed by a spawn, not by a turn: on a machine with no
    // `claude` nothing ever took it, so the turn that runs once one is
    // installed is still the turn that opens the conversation.
    let agent = ChatAgent::new().with_program(NOT_A_PROGRAM);
    let before = turn_args(&agent);

    let error = agent
        .turn("what is in crates?")
        .expect_err("nothing to run");
    assert!(matches!(error, agent::Error::NotFound { .. }), "{error:?}");
    assert_eq!(turn_args(&agent), before);
    assert!(value_of(&before, "--session-id").is_some());
}

#[test]
fn every_turn_of_one_agent_is_one_conversation_and_two_agents_are_two() {
    // The session is settled when the agent is made and every turn names
    // it, so what has to be true is that taking turns does not change which
    // conversation it is. Taken against a program that cannot exist,
    // because the question is what the turn was run with rather than what
    // came back.
    let agent = ChatAgent::new().with_program(NOT_A_PROGRAM);
    let before = turn_args(&agent);

    for message in ["what is in crates?", "and which of those is biggest?"] {
        let error = agent
            .turn(message)
            .expect_err("nothing by that name can be on PATH");
        assert!(matches!(error, agent::Error::NotFound { .. }), "{error:?}");
    }

    assert_eq!(
        turn_args(&agent),
        before,
        "a turn changed the conversation it belongs to",
    );

    // And an agent made afterwards is a conversation of its own: nothing
    // of the first is carried into it, so a second warlock — or a second
    // thread of talk — never resumes somebody else's session.
    let session = value_of(&before, "--session-id").expect("a turn names its session");
    let others: std::collections::HashSet<String> = (0..64)
        .map(|_| {
            let vector = turn_args(&ChatAgent::new());
            value_of(&vector, "--session-id")
                .expect("a turn names its session")
                .to_owned()
        })
        .collect();

    assert_eq!(others.len(), 64, "two agents shared a conversation");
    assert!(!others.contains(session));
}

#[test]
fn a_turn_runs_under_warlocks_own_prompt_rather_than_a_passs() {
    let vector = turn_args(&ChatAgent::new());
    let prompt = value_of(&vector, "--system-prompt").expect("a turn brings its own");

    assert!(!prompt.trim().is_empty());
    // Not the pass's. A pass is told the least that will stop the CLI
    // supplying a persona, because its request carries everything else; a
    // message arrives with none of that, so this one says what a message
    // cannot.
    assert_ne!(prompt, SYSTEM_PROMPT, "a turn is not a documentation pass");
    for said in ["warlock", "tree", "WARLOCK.md", "green", "yellow"] {
        assert!(
            prompt.contains(said),
            "{said:?} is missing from the prompt a turn runs under: {prompt}",
        );
    }
    // And the pass's prompt is exactly where it was.
    assert_eq!(
        value_of(&args(&ClaudeAgent::new()), "--system-prompt"),
        Some(SYSTEM_PROMPT),
    );
}

#[test]
fn the_one_prompt_is_true_in_both_registers() {
    // The sentence that had to go. It was true while a turn was only ever an
    // answer on a panel and false the moment a document can be asked for,
    // and a model told nothing it says can reach a file is a model that
    // hedges when it is asked for the file.
    assert!(
        !CHAT_SYSTEM_PROMPT.contains("nothing you say is put in a file"),
        "the prompt still promises a turn can never reach a file",
    );
    // What replaces it: one document, copied as it stands, into a path the
    // model does not choose, and nothing else it says written anywhere.
    for said in [
        "verbatim",
        "file",
        "path warlock decides",
        "never choose where anything goes",
        "the one thing you say that becomes bytes on disk",
    ] {
        assert!(
            CHAT_SYSTEM_PROMPT.contains(said),
            "{said:?} is missing from the prompt both registers run under: {CHAT_SYSTEM_PROMPT}",
        );
    }
    // Still one prompt, and still not a pass's: the mode is a message and a
    // level, not a second configuration of the agent.
    let agent = ChatAgent::new();
    for vector in [
        turn_args(&agent),
        turn_args(&agent.at_effort(BRIEF_EFFORT).at_model(BRIEF_MODEL)),
    ] {
        assert_eq!(
            value_of(&vector, "--system-prompt"),
            Some(CHAT_SYSTEM_PROMPT)
        );
    }
}

#[test]
fn a_brief_turn_thinks_harder_on_a_better_model_and_is_otherwise_the_same_turn() {
    let agent = ChatAgent::new();
    let question = turn_args(&agent);
    let brief = turn_args(&agent.at_effort(BRIEF_EFFORT).at_model(BRIEF_MODEL));

    // Above `low`, which is the whole of what the raised level has to be:
    // one of the levels the CLI takes, and not the one a question runs at.
    assert_eq!(value_of(&question, "--effort"), Some(EFFORT));
    assert_eq!(value_of(&brief, "--effort"), Some(BRIEF_EFFORT));
    assert_ne!(BRIEF_EFFORT, EFFORT);
    assert!(
        ["medium", "high", "xhigh", "max"].contains(&BRIEF_EFFORT),
        "{BRIEF_EFFORT:?} is not a level above low",
    );

    // The other half of a mode: a frontier model where a question runs on
    // the mid-tier one, and a full name in both so neither register's price
    // can move without a diff.
    assert_eq!(value_of(&question, "--model"), Some(MODEL));
    assert_eq!(value_of(&brief, "--model"), Some(BRIEF_MODEL));
    assert_ne!(BRIEF_MODEL, MODEL);
    assert!(
        !BRIEF_MODEL.is_empty() && BRIEF_MODEL.contains('-'),
        "{BRIEF_MODEL:?} is an alias rather than a pinned name",
    );

    // And those are the only two words of the vector that moved. Said as a
    // count rather than by index so that an argument added to either
    // register fails here rather than sliding past.
    assert_eq!(brief.len(), question.len());
    let moved: Vec<(&String, &String)> = brief
        .iter()
        .zip(&question)
        .filter(|(brief, question)| brief != question)
        .collect();
    assert_eq!(
        moved.len(),
        2,
        "a mode changed something other than how hard the turn thinks and \
             which model thinks it: {moved:?}",
    );
    assert_eq!(value_of(&brief, "--tools"), Some("Read,Grep,Glob"));
    assert_eq!(
        value_of(&brief, "--system-prompt"),
        value_of(&question, "--system-prompt"),
    );
}

#[test]
fn a_mode_is_the_same_conversation_said_at_a_different_level() {
    // The property the whole design rests on: entering brief mode cannot
    // cost the twenty turns already said, which means the id and the flag
    // that names it are shared rather than copied.
    let agent = ChatAgent::new();
    let brief = agent.at_effort(BRIEF_EFFORT);

    let opening = turn_args(&agent);
    let session = value_of(&opening, "--session-id").expect("a turn opens a conversation");
    assert_eq!(value_of(&turn_args(&brief), "--session-id"), Some(session));

    // And once a child has taken the id — claimed here rather than by
    // spawning one, since this test runs on machines with no `claude` and
    // starts no process of any kind — both registers resume it, which is
    // only true because they share the latch and not merely its value.
    agent.session.as_ref().expect("a conversation").claim();
    let resuming = turn_args(&agent);
    let resuming_brief = turn_args(&brief);

    assert_eq!(value_of(&resuming, "--resume"), Some(session));
    assert_eq!(value_of(&resuming_brief, "--resume"), Some(session));
    for vector in [&resuming, &resuming_brief] {
        assert!(
            !vector.iter().any(|word| word == "--session-id"),
            "a mode reopened a session that is already in use: {vector:?}",
        );
    }
}

#[test]
fn the_effort_and_model_variables_win_in_both_registers() {
    // Driven as the pure function it is rather than by setting a real
    // variable: `set_var` is unsafe in this edition, process-wide, and racy
    // against every other test on the runner.
    let asked = OsString::from("xhigh");
    for level in [EFFORT, BRIEF_EFFORT] {
        assert_eq!(or_default(Some(asked.clone()), level), asked);
        assert_eq!(or_default(None, level), OsString::from(level));
        // An exported-but-blank variable is a shell saying nothing.
        assert_eq!(
            or_default(Some(OsString::new()), level),
            OsString::from(level)
        );
    }

    // The model half of the same variable question, and the same three
    // answers: a named model wins, an unset one falls back, a blank one is
    // a shell saying nothing rather than an empty `--model`.
    let named = OsString::from("claude-haiku-4-5-20251001");
    for model in [MODEL, BRIEF_MODEL] {
        assert_eq!(or_default(Some(named.clone()), model), named);
        assert_eq!(or_default(None, model), OsString::from(model));
        assert_eq!(
            or_default(Some(OsString::new()), model),
            OsString::from(model)
        );
    }

    // And what a resolved value comes to on the vector, through the same
    // seam `at_effort` and `at_model` use once they have read the
    // environment. One flag moves and the other stays where it was, which
    // is the property that keeps the two halves of a mode independent.
    let agent = ChatAgent::new();
    let overridden = agent
        .replacing("--effort", or_default(Some(asked.clone()), BRIEF_EFFORT))
        .replacing("--model", or_default(Some(named.clone()), BRIEF_MODEL));
    let vector = turn_args(&overridden);

    assert_eq!(value_of(&vector, "--effort"), Some("xhigh"));
    assert_eq!(value_of(&vector, "--model"), Some(named.to_str().unwrap()));

    // A vector a caller handed in whole is left exactly as it was handed
    // in, by either half of a mode: there is no flag in it to speak for.
    let given = ChatAgent::new().with_args(["-c", "echo hello"]);
    assert_eq!(
        turn_args(&given.at_effort(BRIEF_EFFORT)),
        ["-c", "echo hello"]
    );
    assert_eq!(
        turn_args(&given.at_model(BRIEF_MODEL)),
        ["-c", "echo hello"]
    );
}

#[test]
fn the_two_instructions_are_the_mode_said_each_way() {
    // The brief instruction has three jobs, and a test per job: name the
    // artifact, place the shape it was handed, and say that arguing is the
    // work. Composed here with the built-in shape, which is what a
    // repository that has written no template of its own is given.
    let briefing = brief_instruction(DEFAULT_TEMPLATE);

    assert!(briefing.contains("brief"));
    assert!(briefing.contains("markdown document"));
    for section in [
        "## Outcome",
        "## Success criteria",
        "## Constraints",
        "## Out of scope",
    ] {
        assert!(
            briefing.contains(section),
            "{section:?} is missing from the shape the brief must take",
        );
    }
    for said in [
        "the two or three ways",
        "costs",
        "recommend one",
        "push back",
        "Agreement is not the product",
    ] {
        assert!(
            briefing.contains(said),
            "{said:?} is missing: the instruction asks for agreement rather than argument",
        );
    }
    // It ends by asking, so the reply to the command is the model's opening
    // question rather than a paragraph agreeing to help.
    assert!(briefing.contains("Start now by asking"));
    // The shape is a shape the model is given, never a file it is sent to
    // read: warlock loads the template and the model never hears where
    // from.
    assert!(!briefing.contains(".warlock"));

    // And the matching instruction back, which undoes each of the three.
    for said in [
        "not converging on a document any more",
        "no artifact",
        "Drop the shape",
        "answering questions about this repository",
    ] {
        assert!(
            CHAT_INSTRUCTION.contains(said),
            "{said:?} is missing from the instruction that leaves brief mode",
        );
    }
    assert_ne!(briefing, CHAT_INSTRUCTION);
    // Neither is a system prompt, and neither is ever passed as one: they go
    // in on stdin as an ordinary turn.
    for instruction in [briefing.as_str(), CHAT_INSTRUCTION] {
        assert_ne!(instruction, CHAT_SYSTEM_PROMPT);
        assert!(
            !turn_args(&ChatAgent::new())
                .iter()
                .any(|word| word == instruction),
            "an instruction reached the argument vector",
        );
    }
}

#[test]
fn the_shape_the_brief_instruction_states_is_the_template_it_was_handed() {
    // A stand-in nothing else in the crate says, so what comes back can
    // only have come from the argument: this composes a string and reads
    // no file, no repository and no default.
    const SHAPE: &str = "# say it in haiku\n\n## Syllables\n\nFive, seven, five.";

    let asking = brief_instruction(SHAPE);

    assert!(
        asking.contains(SHAPE),
        "the template was not placed verbatim: {asking}",
    );
    assert!(
        !asking.contains("## Success criteria"),
        "warlock's own shape arrived beside the repository's: {asking}",
    );

    // In the order the three parts stop being guesses: the artifact, then
    // the shape it takes, then the work until it is asked for.
    let artifact = asking.find("one artifact").expect("the artifact named");
    let shape = asking.find(SHAPE).expect("the shape stated");
    let argument = asking
        .find("argue toward a decision")
        .expect("the argument asked for");

    assert!(
        artifact < shape && shape < argument,
        "out of order: {asking}"
    );
    assert!(asking.ends_with("what the change is and what it is for."));
}

#[test]
fn a_template_that_says_nothing_leaves_the_instruction_saying_nothing_about_shape() {
    // The emptied file is a repository asking for no skeleton, and the one
    // thing this must not do is hand back warlock's own. Whitespace is the
    // same statement typed less carefully.
    for template in ["", "   \n\n\t", "\n"] {
        let asking = brief_instruction(template);

        assert!(
            !asking.contains("## Outcome"),
            "a shape was invented for an empty template: {asking}",
        );
        assert!(
            !asking.contains("shape"),
            "an empty template still left a shape paragraph: {asking}",
        );
        assert!(!asking.contains("---"), "empty rules with nothing between");
        // What is left is still the two things that do not come from a
        // template: the artifact, and that arguing is the job.
        assert!(asking.starts_with("This conversation is now aimed at"));
        assert!(asking.contains("argue toward a decision"));
        assert!(asking.ends_with("what the change is and what it is for."));
    }
}

#[test]
fn the_write_instruction_asks_for_the_whole_document_in_the_shape() {
    // Three jobs, and the same test per job as the other two get. First:
    // the entire reply is the document, because warlock copies it verbatim
    // and a courteous sentence at either end becomes a line of the file.
    for said in [
        "entire reply",
        "no preamble",
        "code fence",
        "copied verbatim",
    ] {
        assert!(
            WRITE_INSTRUCTION.contains(said),
            "{said:?} is missing: the reply is allowed to be more than the document",
        );
    }
    // Second: the shape, restated inline, twenty turns after it was first
    // given — and read out of this constant rather than out of a file.
    for section in [
        "# ",
        "## Outcome",
        "## Success criteria",
        "## Constraints",
        "## Out of scope",
        "## Scope",
    ] {
        assert!(
            WRITE_INSTRUCTION.contains(section),
            "{section:?} is missing from the shape the document must take",
        );
    }
    assert!(WRITE_INSTRUCTION.contains("No other sections"));
    assert!(!WRITE_INSTRUCTION.contains(".warlock"));
    assert!(
        !WRITE_INSTRUCTION.contains("docs/"),
        "the instruction named a path, which is warlock's to choose",
    );
    // Third: what was decided rather than how it was arrived at.
    assert!(WRITE_INSTRUCTION.contains("rather than a summary"));

    // And it is an instruction like the other two: its own words, never a
    // system prompt, and never a word of the argument vector.
    assert_ne!(WRITE_INSTRUCTION, brief_instruction(DEFAULT_TEMPLATE));
    assert_ne!(WRITE_INSTRUCTION, CHAT_INSTRUCTION);
    assert_ne!(WRITE_INSTRUCTION, CHAT_SYSTEM_PROMPT);
    assert!(
        !turn_args(&ChatAgent::new())
            .iter()
            .any(|word| word == WRITE_INSTRUCTION),
        "the write instruction reached the argument vector",
    );
}

// A brief with two slices in it, so that anything of the second one showing up
// in the first one's turn is a composition fault rather than a coincidence.
const TWO_SLICES: &str = "What is wrong now: the knife is blunt.\n\n## Scope\n\n\
     ### 1. Sharpen the knife\n\ndepends_on: []\n\nThe first slice decides on a \
     whetstone.\n\n### 2. Sweep the floor\n\ndepends_on: [1]\n\nThe second slice \
     decides on a broom.\n";

#[test]
fn a_drafting_session_is_its_own_conversation_at_the_brief_register() {
    let agent = ChatAgent::drafting();
    let vector = turn_args(&agent);

    assert_eq!(agent.program(), "claude");
    assert_eq!(agent.timeout(), INVOCATION_TIMEOUT);
    // Read through the same seam the constructor did, so a machine with
    // `WARLOCK_MODEL` or `WARLOCK_EFFORT` set is asserting that the
    // reader's choice still wins rather than failing on it.
    let model = overridden(MODEL_VAR, BRIEF_MODEL);
    let effort = overridden(EFFORT_VAR, BRIEF_EFFORT);
    assert_eq!(value_of(&vector, "--model"), model.to_str());
    assert_eq!(value_of(&vector, "--effort"), effort.to_str());
    // Which, with nothing set, is the register the brief was written in
    // and not the one a question runs at.
    assert_ne!(BRIEF_MODEL, MODEL);
    assert_ne!(BRIEF_EFFORT, EFFORT);

    // Not the chat session under another name: its own id, and a prompt
    // that is neither the panel's nor a pass's.
    let session = value_of(&vector, "--session-id").expect("a drafting turn opens a conversation");
    assert!(is_uuid_shaped(session), "not UUID-shaped: {session}");
    let chat = turn_args(&ChatAgent::new());
    assert_ne!(value_of(&chat, "--session-id"), Some(session));
    let prompt = value_of(&vector, "--system-prompt").expect("a drafting turn brings its own");
    assert_ne!(prompt, CHAT_SYSTEM_PROMPT);
    assert_ne!(prompt, SYSTEM_PROMPT);
    assert!(prompt.contains("tickets") && prompt.contains("WARLOCK.md"));

    // And the panel's own session is exactly where it was.
    assert_eq!(value_of(&chat, "--system-prompt"), Some(CHAT_SYSTEM_PROMPT));
    assert_eq!(value_of(&chat, "--model"), Some(MODEL));
    assert_eq!(value_of(&chat, "--effort"), Some(EFFORT));
}

#[test]
fn a_drafting_session_may_read_the_repository_and_do_nothing_whatever_else() {
    // Named rather than left to a default that could grow: a session that
    // reads a slice and a repository has no business holding a tool that
    // writes one, and this is the whole grant it gets.
    let vector = turn_args(&ChatAgent::drafting());
    let granted = value_of(&vector, "--tools").expect("a drafting turn says what it may reach for");

    assert_eq!(granted, "Read,Grep,Glob");
    assert_eq!(granted.split(',').count(), 3);

    names_no_writing_tool(&vector, "a drafting turn");
}

// Read over every word of the vector and not only over the grant, because the
// system prompt is a word of it too: a prompt that names a writing tool is a
// prompt that invites the model to ask for one.
fn names_no_writing_tool(vector: &[String], what: &str) {
    for named in [
        "Write",
        "Edit",
        "MultiEdit",
        "NotebookEdit",
        "Bash",
        "BashOutput",
        "KillShell",
        "Task",
        "WebFetch",
    ] {
        for word in vector {
            assert!(
                !word
                    .split(|letter: char| !letter.is_ascii_alphanumeric())
                    .any(|token| token == named),
                "{named:?} is named in the vector {what} runs under: {word}",
            );
        }
    }
}

#[test]
fn a_proposing_session_may_read_the_repository_and_do_nothing_whatever_else() {
    // The same grant a drafting turn gets and for the same reason: this one
    // reads a brief, a slice and a repository to propose one sentence, and
    // nothing it is given can change any of the three.
    let vector = turn_args(&ChatAgent::proposing());
    let granted =
        value_of(&vector, "--tools").expect("a proposing turn says what it may reach for");

    assert_eq!(granted, "Read,Grep,Glob");
    assert_eq!(granted.split(',').count(), 3);

    names_no_writing_tool(&vector, "a proposing turn");
    assert_eq!(ChatAgent::proposing().timeout(), INVOCATION_TIMEOUT);
}

#[test]
fn proposing_opens_a_conversation_of_its_own_at_the_briefs_register() {
    let vector = turn_args(&ChatAgent::proposing());

    // Raised the way a drafting turn is, through the same two constants, so
    // the attempt at an answer is made at the register the question was
    // asked at.
    assert_eq!(
        value_of(&vector, "--model"),
        overridden(MODEL_VAR, BRIEF_MODEL).to_str(),
    );
    assert_eq!(
        value_of(&vector, "--effort"),
        overridden(EFFORT_VAR, BRIEF_EFFORT).to_str(),
    );

    // Its own id: neither the panel's conversation nor the one that asked
    // the question, which is mid-cut and whose next turn is the answer.
    let session = value_of(&vector, "--session-id").expect("a proposing turn opens a conversation");
    assert!(is_uuid_shaped(session), "not UUID-shaped: {session}");
    for other in [
        turn_args(&ChatAgent::new()),
        turn_args(&ChatAgent::drafting()),
        turn_args(&ChatAgent::proposing()),
    ] {
        assert_ne!(value_of(&other, "--session-id"), Some(session));
    }

    // And a prompt of its own, which is none of the other three.
    let prompt = value_of(&vector, "--system-prompt").expect("a proposing turn brings its own");
    assert_eq!(prompt, PROPOSING_SYSTEM_PROMPT);
    assert_ne!(prompt, CHAT_SYSTEM_PROMPT);
    assert_ne!(prompt, SYSTEM_PROMPT);
    assert_ne!(
        Some(prompt),
        value_of(&turn_args(&ChatAgent::drafting()), "--system-prompt"),
    );
    assert!(prompt.contains("WARLOCK.md"));
    for said in ["one question", "plain prose"] {
        assert!(
            prompt.contains(said),
            "{said:?} is missing from the prompt a proposing turn runs under",
        );
    }
}

#[test]
fn a_proposing_turn_carries_the_brief_one_slice_and_the_question_on_stdin() {
    let block = scope_block_in(TWO_SLICES).expect("two slices");
    let first = &block.slices()[0];
    let question = "Is the whetstone the one in `tools/` or a new one?";
    let asking = proposing_instruction(block.brief(), first.heading(), first.prose(), question);

    assert!(asking.contains(question));
    assert!(asking.contains(first.heading()));
    assert!(asking.contains("the knife is blunt"));
    assert!(asking.contains("whetstone"));

    // One slice, like the drafting opening: the rest of the scope is not in
    // the turn to be answered from by accident.
    for said in ["Sweep the floor", "broom", "## Scope", "depends_on"] {
        assert!(
            !asking.contains(said),
            "{said:?} reached a turn about the first slice: {asking}",
        );
    }

    // The fixed sentence is written down once and handed to the session
    // rather than described to it: warlock recognises the proposal by the
    // same bytes it asked for.
    assert!(asking.ends_with(NOTHING_SETTLES_IT));
    assert_eq!(asking.matches(NOTHING_SETTLES_IT).count(), 1);
    assert!(NOTHING_SETTLES_IT.ends_with('.') && !NOTHING_SETTLES_IT.contains('\n'));
    assert!(!PROPOSING_SYSTEM_PROMPT.contains(NOTHING_SETTLES_IT));

    // And none of it is an argument: the words go in on stdin as an
    // ordinary turn, the way every other instruction does.
    let vector = turn_args(&ChatAgent::proposing());
    for word in &vector {
        assert!(
            !word.contains(question) && !word.contains(NOTHING_SETTLES_IT),
            "a proposing instruction reached the argument vector: {word}",
        );
    }
    assert!(!vector.iter().any(|word| word == &asking));
}

#[test]
fn a_drafting_opening_carries_the_brief_and_one_slice_of_it() {
    let block = scope_block_in(TWO_SLICES).expect("two slices");
    let first = &block.slices()[0];
    let opening = drafting_opening(
        block.brief(),
        first.heading(),
        first.prose(),
        DRAFTING_CONTRACT,
    );

    assert!(opening.starts_with(DRAFTING_CONTRACT));
    assert!(opening.contains("the knife is blunt"));
    assert!(opening.contains("Sharpen the knife"));
    assert!(opening.contains("whetstone"));

    // The point of the whole builder: a session drafts one slice, so the
    // rest of the scope is not in the turn to be drafted by accident.
    for said in ["Sweep the floor", "broom", "## Scope", "depends_on"] {
        assert!(
            !opening.contains(said),
            "{said:?} reached a turn about the first slice: {opening}",
        );
    }

    // The caps, the shape and what a ticket may be are the engine's words,
    // appended rather than restated here.
    assert!(opening.ends_with(&drafting::drafting_instructions(
        block.brief(),
        first.heading(),
        first.prose(),
        &[],
    )));
    assert!(opening.contains("at most 12 entries"));
    assert!(opening.contains("\"blocked_by\""));

    // And the headless turn is the same turn on different terms.
    let one_shot = drafting_opening(
        block.brief(),
        first.heading(),
        first.prose(),
        DRAFTING_ONE_SHOT_CONTRACT,
    );
    assert_ne!(one_shot, opening);
    assert_eq!(
        one_shot.strip_prefix(DRAFTING_ONE_SHOT_CONTRACT),
        opening.strip_prefix(DRAFTING_CONTRACT),
    );
}

#[test]
fn the_three_drafting_instructions_are_each_said_in_their_own_words() {
    // Three different situations, three different texts. The one-shot
    // contract is not the interactive one with the asking struck out: a
    // model told it may ask and then told the limit is zero spends its one
    // turn asking anyway.
    assert_ne!(DRAFTING_CONTRACT, DRAFTING_ONE_SHOT_CONTRACT);
    assert_ne!(DRAFT_NOW_INSTRUCTION, DRAFTING_CONTRACT);
    assert_ne!(DRAFT_NOW_INSTRUCTION, DRAFTING_ONE_SHOT_CONTRACT);

    // What the interactive contract has to say: how warlock tells a
    // question from an answer, and how many rounds of it there are.
    for said in [
        "JSON object",
        "is the drafts",
        "is a question",
        "at most three questions",
    ] {
        assert!(
            DRAFTING_CONTRACT.contains(said),
            "{said:?} is missing from the contract a drafting session is held to",
        );
    }

    // And what the one-shot one has to: nobody there, one turn, and no
    // count of rounds to argue about.
    for said in ["Nobody is reading", "only turn"] {
        assert!(
            DRAFTING_ONE_SHOT_CONTRACT.contains(said),
            "{said:?} is missing from the contract the headless path uses",
        );
    }
    assert!(
        !DRAFTING_ONE_SHOT_CONTRACT.contains("three"),
        "the headless contract counts rounds that cannot happen",
    );
    assert!(DRAFT_NOW_INSTRUCTION.contains("third question"));

    // None of the three is a system prompt, and none reaches the argument
    // vector: they go in on stdin as ordinary turns.
    let vector = turn_args(&ChatAgent::drafting());
    for instruction in [
        DRAFTING_CONTRACT,
        DRAFTING_ONE_SHOT_CONTRACT,
        DRAFT_NOW_INSTRUCTION,
    ] {
        assert_ne!(instruction, CHAT_SYSTEM_PROMPT);
        assert!(
            !vector.iter().any(|word| word == instruction),
            "a drafting instruction reached the argument vector",
        );
    }
}

// A conversation written down in advance: what it was sent is kept, and what it
// says back was decided by the test. Here rather than in `stubs.rs` because that
// module belongs to the binary crate and this one is `claude.rs`'s own.
//
// Shared through `Arc` so that a copy handed out by `wired` is the same
// stand-in: a session wires the agent it was given before it sends anything,
// and a double that recorded into its own clone would record nothing the test
// could read.
#[derive(Debug, Clone, Default)]
struct Scripted {
    sent: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<Vec<String>>>,
}

impl Scripted {
    fn answering<S: Into<String>>(answers: impl IntoIterator<Item = S>) -> Self {
        let answers = answers.into_iter().map(Into::into).collect();
        Self {
            sent: Arc::new(Mutex::new(Vec::new())),
            answers: Arc::new(Mutex::new(answers)),
        }
    }

    fn sent(&self) -> Vec<String> {
        self.sent
            .lock()
            .expect("the script is not poisoned")
            .clone()
    }
}

impl Wired for Scripted {
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        self.clone()
    }
}

impl Converses for Scripted {
    fn turn(&self, message: &str) -> Result<String, agent::Error> {
        self.sent
            .lock()
            .expect("the script is not poisoned")
            .push(message.to_owned());
        let mut answers = self.answers.lock().expect("the script is not poisoned");
        assert!(
            !answers.is_empty(),
            "a turn was taken the script has no answer for: {message}",
        );
        Ok(answers.remove(0))
    }

    fn raised(&self, _model: &str, _effort: &str) -> Self {
        self.clone()
    }
}

fn asking(replied: Replied) -> String {
    match replied {
        Replied::Question(question) => question,
        Replied::Answer(accepted) => panic!("a reply was taken as the answer: {accepted:?}"),
    }
}

fn answered(replied: Replied) -> Drafted {
    match replied {
        Replied::Answer(drafted) => drafted,
        Replied::Question(question) => panic!("a reply was relayed as a question: {question}"),
    }
}

// The drafts and what warlock had to repair to get them, or a panic saying which
// other ending arrived.
fn drafts(replied: Replied) -> (drafting::Fill, Vec<String>) {
    match answered(replied) {
        Drafted::Drafts { fill, repairs } => (fill, repairs),
        Drafted::Unusable(defect) => panic!("nothing was drafted: {defect}"),
    }
}

fn drafting_with(agent: &Scripted) -> Drafting<Scripted> {
    let block = scope_block_in(TWO_SLICES).expect("two slices");
    let first = &block.slices()[0];
    Drafting::for_slice(agent, block.brief(), first.heading(), first.prose())
}

fn one_shot_with(agent: &Scripted) -> Drafting<Scripted> {
    let block = scope_block_in(TWO_SLICES).expect("two slices");
    let first = &block.slices()[0];
    Drafting::one_shot(agent, block.brief(), first.heading(), first.prose())
}

// One draft, every field inside its caps and no reference to another: an answer
// the contract accepts as it stands, so a test that uses it is asserting about
// the road it took and not about the repair.
const ONE_DRAFT: &str = "{\"drafts\":[{\"title\":\"Sharpen the knife on the \
                         whetstone\",\"body\":\"The knife is blunt and the \
                         whetstone is in the drawer.\"}]}";

// Four replies with not a brace between them, so every one of them is prose to
// `drafting::accept` and the count is the only thing deciding what happens to
// it.
const FOUR_QUESTIONS: [&str; 4] = [
    "Which of the two knives is the slice about?",
    "And is the whetstone the one in the drawer?",
    "Should sharpening come before sweeping?",
    "Thank you - one last thing before I draft?",
];

#[test]
fn three_questions_are_relayed_and_the_fourth_turn_says_to_draft_with_what_it_has() {
    // Four questions, and then the object: the fourth is past the rounds, so
    // it is a failed attempt rather than a question, and the session asks
    // again rather than stopping there.
    let agent = Scripted::answering(FOUR_QUESTIONS.into_iter().chain([ONE_DRAFT]));
    let mut session = drafting_with(&agent);

    assert_eq!(session.questions_left(), DRAFTING_ROUNDS);
    assert_eq!(asking(session.open().expect("a turn")), FOUR_QUESTIONS[0]);
    assert_eq!(session.questions_left(), 2);
    assert_eq!(
        asking(session.answer("the first one").expect("a turn")),
        FOUR_QUESTIONS[1],
    );
    assert_eq!(
        asking(session.answer("yes, the drawer").expect("a turn")),
        FOUR_QUESTIONS[2],
    );
    // Three rounds, and the session is out of them.
    assert_eq!(session.questions_left(), 0);

    // The fourth reply is prose again, and prose is a question only while
    // there is somebody being asked. This one is a failed attempt instead:
    // asked again with what was wrong with it, the session ends in drafts and
    // nothing is put to anybody.
    let (fill, repairs) = drafts(session.answer("sharpening first").expect("a turn"));
    assert_eq!(fill.drafts.len(), 1);
    assert!(
        repairs.is_empty(),
        "a clean object was repaired: {repairs:?}"
    );

    let sent = agent.sent();
    assert_eq!(
        sent.len(),
        5,
        "one turn per round, then the one it took to get an object: {sent:?}",
    );
    assert!(sent[0].starts_with(DRAFTING_CONTRACT));
    assert!(sent[0].contains("the knife is blunt"));
    assert_eq!(sent[1], "the first one");
    assert_eq!(sent[2], "yes, the drawer");
    // The answer to the third question is carried with the instruction and
    // not dropped: a round spent asking that throws the reply away is a
    // round wasted.
    assert!(sent[3].starts_with("sharpening first"));
    assert!(sent[3].ends_with(DRAFT_NOW_INSTRUCTION));
    for earlier in &sent[..3] {
        assert!(
            !earlier.contains(DRAFT_NOW_INSTRUCTION),
            "the session gave up before its rounds were spent: {earlier}",
        );
    }
    // And the fifth turn is the attempt loop's, not a fourth question's: the
    // request again, with what the last reply was wrong about.
    assert!(sent[4].contains("turned down"));
    assert!(sent[4].contains("not a JSON object"));
}

#[test]
fn a_reply_shaped_like_the_drafts_object_is_the_answer_and_ends_the_asking() {
    let agent = Scripted::answering([ONE_DRAFT, "And now, a question?"]);
    let mut session = drafting_with(&agent);

    let (fill, repairs) = drafts(session.open().expect("a turn"));

    assert!(
        repairs.is_empty(),
        "a clean object was repaired: {repairs:?}"
    );
    assert_eq!(fill.drafts.len(), 1);
    assert_eq!(fill.drafts[0].title, "Sharpen the knife on the whetstone");
    // One turn: the object ends the conversation on the spot, with all
    // three rounds unspent, so the second answer in the script is never
    // reached.
    assert_eq!(agent.sent().len(), 1);
    assert_eq!(session.questions_left(), DRAFTING_ROUNDS);
}

#[test]
fn an_object_that_filled_itself_badly_is_still_the_answer_rather_than_a_question() {
    // It parsed, so it is the drafts however badly it filled them: a slice
    // nobody drafted is something to repair, and a repair is not a question
    // for the person who asked for this cut.
    let agent = Scripted::answering(["Here you go:\n\n{\"drafts\": []}"]);
    let mut session = drafting_with(&agent);

    let (fill, repairs) = drafts(session.open().expect("a turn"));

    assert_eq!(fill.drafts.len(), 1, "the mend left the slice uncut");
    assert!(!repairs.is_empty(), "a repair went unreported");
    // One turn: it was repaired, not asked again, and not put to anybody.
    assert_eq!(agent.sent().len(), 1);
    assert_eq!(session.questions_left(), DRAFTING_ROUNDS);
}

// Parses, and is wrong in four ways at once: a title over two lines, a body
// nobody wrote, a title too short to name a ticket, and an order pointing at a
// draft this slice does not hold.
const A_BAD_OBJECT: &str = "{\"drafts\":[\
    {\"title\":\"Sharpen the knife on the whetstone\\nand then sweep\",\
     \"body\":\"\",\"blocked_by\":[],\"blocks\":[]},\
    {\"title\":\"Sweep\",\"body\":\"The floor, afterwards.\",\
     \"blocked_by\":[],\"blocks\":[7]}]}";

#[test]
fn a_defective_answer_comes_back_repaired_with_a_line_for_every_repair() {
    let agent = Scripted::answering([A_BAD_OBJECT]);
    let mut session = drafting_with(&agent);

    let (fill, repairs) = drafts(session.open().expect("a turn"));

    // Repaired, not refused and not asked again: one turn, and what comes
    // back is clean by the contract's own check.
    assert_eq!(agent.sent().len(), 1);
    assert!(
        drafting::check(&fill).is_empty(),
        "a defective fill was handed back unmended: {fill:?}",
    );
    assert_eq!(fill.drafts[0].title, "Sharpen the knife on the whetstone");
    assert!(!fill.drafts[0].body.trim().is_empty());
    assert!(fill.drafts[1].blocks.is_empty());

    // Every mend the engine made, in its own words and in its own order.
    // Derived from the engine rather than written out here, because the
    // assertion is that nothing was dropped on the way to the caller — what
    // a repair is called is `Mend`'s to say.
    let drafting::Accepted::Defective { fill: raw, .. } = drafting::accept(A_BAD_OBJECT) else {
        panic!("the fixture is no longer a defective object");
    };
    let block = scope_block_in(TWO_SLICES).expect("two slices");
    let slice = &block.slices()[0];
    let (_, mends) = drafting::mend(&raw, slice.heading(), slice.prose());
    let said: Vec<String> = mends.iter().map(ToString::to_string).collect();

    assert_eq!(repairs, said);
    assert_eq!(repairs.len(), 4, "a repair went unreported: {repairs:?}");
    assert!(
        repairs
            .iter()
            .any(|line| line.contains("drafts[0].title") && line.contains("keeps its first")),
        "the repairs do not say what was done to the title: {repairs:?}",
    );
}

#[test]
fn an_answer_that_is_not_the_object_is_asked_again_with_what_was_wrong_with_it() {
    const PROSE: &str = "I would start with the whetstone, I think.";
    // The one-shot road, where there are no rounds at all, so the first
    // reply that is not the object is a failed attempt and nothing else.
    let agent = Scripted::answering([PROSE, ONE_DRAFT]);
    let mut session = one_shot_with(&agent);

    let (fill, _) = drafts(session.open().expect("a turn"));
    assert_eq!(fill.drafts.len(), 1);

    let sent = agent.sent();
    assert_eq!(sent.len(), 2, "the attempt was not made again: {sent:?}");

    // The second message is the request again, carrying the first attempt's
    // own defect as something not to repeat.
    let drafting::Accepted::Unparsed(defect) = drafting::accept(PROSE) else {
        panic!("prose is no longer unparseable");
    };
    assert!(sent[1].contains("turned down"));
    assert!(
        sent[1].contains(&defect.to_string()),
        "the re-ask does not carry the defect: {}",
        sent[1],
    );
    // A request and not a scolding: the brief and the slice are in it, so
    // the attempt has what it needs to be made again.
    assert!(sent[1].contains("the knife is blunt"));
    assert!(sent[1].contains("Sharpen the knife"));
    // And the terms were said once, at the opening, where they still are.
    assert!(sent[0].starts_with(DRAFTING_ONE_SHOT_CONTRACT));
    assert!(!sent[1].contains(DRAFTING_ONE_SHOT_CONTRACT));
}

#[test]
fn the_one_shot_road_puts_nothing_to_anybody_and_stops_at_the_attempt_count() {
    // Every reply prose, which on the interactive road is three questions
    // and then the instruction. Here there is nobody to ask, so all of it is
    // the attempt loop.
    let agent = Scripted::answering(
        (1..=drafting::ATTEMPTS).map(|round| format!("Question {round}, since nobody said?")),
    );
    let mut session = one_shot_with(&agent);

    assert_eq!(session.questions_left(), 0);
    // `answered` is the assertion: it panics on anything relayed as a
    // question, and every turn of this session went through it.
    let ending = answered(session.open().expect("a turn"));
    assert!(
        matches!(ending, Drafted::Unusable(_)),
        "four unparseable answers ended somewhere else: {ending:?}",
    );
    assert_eq!(session.questions_left(), 0);

    let sent = agent.sent();
    assert_eq!(
        sent.len(),
        drafting::ATTEMPTS,
        "the loop is bounded by the engine's count and nothing else: {sent:?}",
    );
    for message in &sent {
        // Neither the interactive terms nor the instruction that ends them:
        // both talk about questions this road cannot have.
        assert!(!message.contains(DRAFTING_CONTRACT));
        assert!(!message.contains(DRAFT_NOW_INSTRUCTION));
    }
}

#[test]
fn a_session_is_a_value_its_caller_holds_and_can_stop() {
    let agent = Scripted::answering(["a question?"]);
    let session = drafting_with(&agent);
    let cancel = session.cancel();

    assert!(!cancel.is_cancelled());
    // From somewhere that is not the thread waiting on the turn, which is
    // the whole point of handing the handle out.
    thread::spawn(move || cancel.cancel())
        .join()
        .expect("the thread ran");

    assert!(session.cancel().is_cancelled());
    assert!(
        agent.sent().is_empty(),
        "a session spawns nothing until it is opened"
    );
}

#[test]
fn drafting_is_a_session_of_its_own_rather_than_a_register_of_the_panels() {
    // Exhaustive on purpose and not a wildcard: a `Mode` variant added for
    // drafting stops this compiling, which is the assertion. A mode is the
    // one chat session said at a different level, and a conversation with
    // its own prompt, its own id and a JSON answer is not that.
    for mode in [Mode::Chat, Mode::Brief] {
        let named = match mode {
            Mode::Chat => "chat",
            Mode::Brief => "brief",
        };
        assert!(!named.contains("draft"));
    }
    assert_eq!(Mode::default(), Mode::Chat);

    // And the panel's conversation runs under the prompt it always did:
    // nothing about tickets, slices or an object to fill reached it.
    assert_eq!(
        value_of(&turn_args(&ChatAgent::new()), "--system-prompt"),
        Some(CHAT_SYSTEM_PROMPT),
    );
    for said in ["ticket", "slice", "draft", "JSON", "drafts"] {
        assert!(
            !CHAT_SYSTEM_PROMPT.contains(said),
            "{said:?} reached the prompt the panel's chat runs under",
        );
    }
}

// The shape is written down twice — as the template, and restated inside
// `WRITE_INSTRUCTION` — and only the template is enforced: `write_submit` refuses
// a document missing one of its sections. So a section the instruction never asks
// for is not a gap in a document, it is a `/write` that can never succeed. That
// happened once, with `## Scope` named in the template and omitted from the
// instruction; this is the assertion that catches it.
#[test]
fn the_write_instruction_names_every_section_the_shape_is_checked_for() {
    // The template's `## ` lines, read the same way `missing_sections` reads
    // them, so this test and the check cannot disagree about what a section
    // is.
    let checked_for: Vec<&str> = DEFAULT_TEMPLATE
        .lines()
        .filter_map(|line| line.trim().strip_prefix("## "))
        .map(str::trim)
        .filter(|section| !section.is_empty())
        .collect();
    assert!(
        !checked_for.is_empty(),
        "the built-in shape asks for no sections, so this guards nothing",
    );

    for section in &checked_for {
        assert!(
            WRITE_INSTRUCTION.contains(&format!("## {section}")),
            "the shape is checked for `## {section}` and the write \
                 instruction never asks for it: every brief would be refused",
        );
    }

    // And said the other way round, against the check itself: a document
    // carrying exactly what the instruction asks for is a document warlock
    // will write.
    let mut obedient = String::from("# A change\n\nWhat is wrong now.\n");
    for section in &checked_for {
        let _ = write!(obedient, "\n## {section}\n\nSomething under it.\n");
    }
    assert!(
        crate::template::missing_sections(DEFAULT_TEMPLATE, &obedient).is_empty(),
        "a document in the instructed shape was refused by the check",
    );
}

#[test]
fn a_chat_agents_program_arguments_and_clock_are_a_callers_to_replace() {
    // The same three fields a pass has, for the same reason: every failure
    // path is exercised with a stand-in on a machine with no `claude`.
    let agent = ChatAgent::new()
        .with_program("/bin/sh")
        .with_args(["-c", "echo hello"])
        .with_timeout(Duration::from_millis(250));

    assert_eq!(agent.program(), "/bin/sh");
    assert_eq!(turn_args(&agent), ["-c", "echo hello"]);
    assert_eq!(agent.timeout(), Duration::from_millis(250));
    assert!(turn_args(&ChatAgent::new().with_args(Vec::<&str>::new())).is_empty());
}

#[test]
fn a_chat_agent_answers_to_the_handles_a_caller_attaches() {
    let cancel = Cancel::new();
    let (sender, received) = mpsc::channel();
    let agent = ChatAgent::new()
        .with_cancel(cancel.clone())
        .with_activities(Activities::new(move |activity| {
            let _ = sender.send(activity);
        }));

    agent.activities().report(Activity::Thinking);
    assert_eq!(received.recv(), Ok(Activity::Thinking));

    // The half of a cancel that needs no child: a turn asked for after one
    // is refused before anything is spawned, which is why this test can
    // hold an agent pointed at the real `claude` and still run nothing.
    cancel.cancel();
    let error = agent
        .turn("anything")
        .expect_err("a cancelled agent takes no turns");
    assert!(
        matches!(&error, agent::Error::Io { source } if source.kind() == std::io::ErrorKind::Interrupted),
        "{error:?}",
    );

    // And an agent nobody wired a port to reports into nothing.
    ChatAgent::new().activities().report(Activity::Thinking);
}

#[test]
fn the_arguments_are_a_field_a_caller_can_replace_outright() {
    // Not appended to and not merged with: what a caller asks for is what
    // is run, which is how every stand-in below works and how a later
    // slice changes the invocation without touching this file.
    let agent = ClaudeAgent::new().with_args(["-c", "echo hello"]);

    assert_eq!(args(&agent), ["-c", "echo hello"]);
    assert!(args(&ClaudeAgent::new().with_args(Vec::<&str>::new())).is_empty());
}

#[test]
fn a_missing_binary_is_reported_by_name_not_as_an_errno() {
    // No `claude` needed to test the no-`claude` case, which is the point:
    // this is the state of every machine that has never installed it.
    let agent = ClaudeAgent::new().with_program(NOT_A_PROGRAM);

    let error = agent
        .run(&agent::Request::new("anything", "."))
        .expect_err("nothing by that name can be on PATH");

    match error {
        agent::Error::NotFound { program } => assert_eq!(program, NOT_A_PROGRAM),
        other => panic!("expected a missing binary, got {other:?}"),
    }
}

#[test]
fn a_cancel_handle_is_one_flag_shared_by_every_clone() {
    // The property the whole mechanism rests on: the thread that cancels
    // is never the thread that is running the pass.
    fn held_across_threads<T: Send + Sync + 'static>(_: &T) {}

    let cancel = Cancel::new();
    held_across_threads(&cancel);
    let watcher = cancel.clone();
    assert!(!cancel.is_cancelled());
    assert!(!Cancel::default().is_cancelled(), "and a fresh one is live");

    thread::spawn(move || {
        watcher.cancel();
        // Latched, not toggled, and saying it twice is not an error.
        watcher.cancel();
    })
    .join()
    .expect("the cancelling thread ran");

    assert!(cancel.is_cancelled());
}

#[test]
fn a_port_nobody_listens_to_swallows_everything_reported_to_it() {
    // What an agent has until a caller attaches one: reporting is a no-op,
    // not a panic and not a failure, so the parsing side can report
    // unconditionally.
    for activities in [Activities::none(), Activities::default()] {
        activities.report(Activity::Thinking);
        activities.report(Activity::Tool {
            name: "Read".to_owned(),
            detail: Some("src/lib.rs".to_owned()),
        });
        activities.report(Activity::Cost { usd: 0.03 });
    }

    // And that is what an agent nobody wired up has.
    ClaudeAgent::new().activities().report(Activity::Thinking);
}

#[test]
fn every_clone_of_a_port_reports_to_the_same_place() {
    // The property the whole port rests on, and [`Cancel`]'s in reverse:
    // the thread running the pass is never the thread listening to it.
    fn held_across_threads<T: Send + Sync + 'static>(_: &T) {}

    let (sender, received) = mpsc::channel();
    let activities = Activities::new(move |activity| {
        sender.send(activity).expect("the test is still listening");
    });
    held_across_threads(&activities);
    let passing = activities.clone();

    thread::spawn(move || {
        passing.report(Activity::Thinking);
        passing.report(Activity::Tool {
            name: "Bash".to_owned(),
            detail: Some("cargo test".to_owned()),
        });
        passing.report(Activity::Cost { usd: 0.25 });
    })
    .join()
    .expect("the reporting thread ran");

    // In the order they were reported, through the clone, from the other
    // thread.
    assert_eq!(received.recv(), Ok(Activity::Thinking));
    assert_eq!(
        received.recv(),
        Ok(Activity::Tool {
            name: "Bash".to_owned(),
            detail: Some("cargo test".to_owned()),
        })
    );
    assert_eq!(received.recv(), Ok(Activity::Cost { usd: 0.25 }));
    // The original still reports to the same place after the clone is gone.
    activities.report(Activity::Thinking);
    assert_eq!(received.recv(), Ok(Activity::Thinking));
}

#[test]
fn an_attached_port_is_the_one_the_agent_reports_to() {
    let (sender, received) = mpsc::channel();
    let agent = ClaudeAgent::new().with_activities(Activities::new(move |activity| {
        let _ = sender.send(activity);
    }));

    agent.activities().report(Activity::Thinking);

    assert_eq!(received.recv(), Ok(Activity::Thinking));
    // Attaching one changes nothing else about the agent.
    assert_eq!(agent.program(), "claude");
    assert_eq!(agent.timeout(), INVOCATION_TIMEOUT);
}

// One assistant line carrying `blocks` as its content, the shape a real stream
// uses.
fn assistant(blocks: &str) -> String {
    format!(r#"{{"type":"assistant","message":{{"role":"assistant","content":[{blocks}]}}}}"#)
}

#[test]
fn each_whitelisted_tool_carries_its_one_argument_and_the_rest_carry_none() {
    // The table verbatim, and the point of the last row: a tool nobody
    // wrote down is shown by name, not by dumping whatever its call
    // carried.
    let expected = [
        ("Read", r#"{"file_path":"src/lib.rs"}"#, Some("src/lib.rs")),
        (
            "Edit",
            r#"{"file_path":"src/main.rs"}"#,
            Some("src/main.rs"),
        ),
        (
            "Write",
            r#"{"file_path":"docs/plan.md"}"#,
            Some("docs/plan.md"),
        ),
        ("Glob", r#"{"pattern":"**/*.rs"}"#, Some("**/*.rs")),
        ("Grep", r#"{"pattern":"fn main"}"#, Some("fn main")),
        ("Bash", r#"{"command":"cargo test"}"#, Some("cargo test")),
        ("WebFetch", r#"{"url":"https://example.invalid"}"#, None),
    ];

    for (name, input, detail) in expected {
        let line = assistant(&format!(
            r#"{{"type":"tool_use","id":"toolu_1","name":"{name}","input":{input}}}"#
        ));

        let reading = stream::read_line(&line);

        assert_eq!(
            reading.activities,
            vec![Activity::Tool {
                name: name.to_owned(),
                detail: detail.map(str::to_owned),
            }],
            "one activity for {name}, with exactly the whitelisted detail"
        );
        assert_eq!(reading.text, None, "a tool call is not the document");
    }
}

#[test]
fn a_whitelisted_tool_missing_its_argument_is_still_the_bare_name() {
    // Three ways the key is not there, none of them a reason to lose the
    // activity or to reach for some other key.
    for input in [r"{}", r#"{"offset":12}"#, r#"{"file_path":7}"#] {
        let line = assistant(&format!(
            r#"{{"type":"tool_use","name":"Read","input":{input}}}"#
        ));

        assert_eq!(
            stream::read_line(&line).activities,
            vec![Activity::Tool {
                name: "Read".to_owned(),
                detail: None,
            }],
            "Read with input {input}"
        );
    }

    // And a block with no `input` at all.
    assert_eq!(
        stream::read_line(&assistant(r#"{"type":"tool_use","name":"Bash"}"#)).activities,
        vec![Activity::Tool {
            name: "Bash".to_owned(),
            detail: None,
        }]
    );
}

#[test]
fn a_thought_reaches_the_panel_as_the_fact_that_it_happened_and_nothing_else() {
    let secret = "the user's code is beyond saving and I shall say so gently";
    let line = assistant(&format!(
        r#"{{"type":"thinking","thinking":"{secret}","signature":"abc"}}"#
    ));

    let reading = stream::read_line(&line);

    assert_eq!(reading.activities, vec![Activity::Thinking]);
    // The whole point of the bare variant: there is nowhere for the text to
    // be, so it cannot be printed by accident later.
    assert!(
        !format!("{reading:?}").contains("beyond saving"),
        "no part of a thought survives the parse"
    );
}

#[test]
fn tool_results_and_the_models_own_prose_are_not_activities() {
    let enormous = "x".repeat(200_000);
    let lines = [
        // A tool result comes back on a `user` line, which is not a line
        // type this reads at all...
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_1","content":"{enormous}"}}]}}}}"#
        ),
        // ...and would still be nothing if it arrived on one that is.
        assistant(&format!(
            r#"{{"type":"tool_result","tool_use_id":"toolu_1","content":"{enormous}"}}"#
        )),
        // The model's prose is the document, not a sign of life.
        assistant(r#"{"type":"text","text":"Here is the summary you asked for."}"#),
    ];

    for line in lines {
        let reading = stream::read_line(&line);

        assert_eq!(
            reading,
            stream::Reading::default(),
            "nothing from {line:.60}"
        );
    }
}

#[test]
fn a_line_this_code_does_not_understand_is_skipped_rather_than_fatal() {
    let lines = [
        "",
        "   ",
        "not json at all",
        "{",
        "[1, 2, 3]",
        "null",
        r#""a bare string""#,
        // JSON, well formed, and about something else entirely.
        r#"{"type":"system","subtype":"init","tools":["Read","Bash"]}"#,
        r#"{"type":"kraken","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
        r#"{"message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
        // The right type, with the levels below it missing or the wrong
        // shape.
        r#"{"type":"assistant"}"#,
        r#"{"type":"assistant","message":{"content":"not a list"}}"#,
        &assistant(r#"{"type":"tool_use"}"#),
        &assistant(r#"{"no":"type"}"#),
    ];

    for line in lines {
        assert_eq!(
            stream::read_line(line),
            stream::Reading::default(),
            "nothing, and no panic, from {line:?}"
        );
    }
}

#[test]
fn a_thinking_tokens_line_is_the_sign_of_life_a_thinking_pass_gives() {
    // The line a real pass emits every few seconds while it thinks. What is
    // taken from it is the fact, not the estimate: the panel's clock
    // already measures how long thinking has been going.
    let line = r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":113,"estimated_tokens_delta":63}"#;

    assert_eq!(stream::read_line(line).activities, vec![Activity::Thinking]);
    assert_eq!(stream::read_line(line).text, None);
}

#[test]
fn a_text_block_opening_is_the_pass_starting_to_write() {
    // The event that separates the two halves of a toolless pass. It
    // arrives when the writing begins; the finished block arrives when the
    // document is done, which is what the outcome line is for.
    let line = r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}}"#;

    // Zero bytes, because none of the answer has arrived yet — and the one
    // line that says so, which is how the reader knows to start counting
    // again rather than to go on adding to whatever came before.
    assert_eq!(
        stream::read_line(line).activities,
        vec![Activity::Writing { bytes: 0 }]
    );
    assert!(stream::read_line(line).opens_text);
}

#[test]
fn a_text_delta_is_that_much_more_of_the_answer_arrived() {
    // The words are measured and thrown away: what comes back is a size,
    // and the document still comes whole from the result line.
    let line = r##"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"# engine"}}}"##;

    let reading = stream::read_line(line);

    assert_eq!(reading.activities, vec![Activity::Writing { bytes: 8 }]);
    assert_eq!(reading.text, None);
    // A delta is not an opening. Telling the two apart is the reader's
    // whole means of knowing when a count starts over.
    assert!(!reading.opens_text);
}

#[test]
fn a_delta_is_counted_in_bytes_and_not_in_characters() {
    // Eleven characters and fifteen bytes: an em dash and a curly
    // apostrophe are three bytes each, and what went down the pipe — and
    // what the finished document will weigh — is the bytes.
    let line = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"it’s — done"}}}"#;

    assert_eq!("it’s — done".chars().count(), 11);
    assert_eq!(
        stream::read_line(line).activities,
        vec![Activity::Writing { bytes: 15 }]
    );
}

#[test]
fn a_delta_reports_its_own_length_and_never_a_running_total() {
    // The proof that reading a line is a pure function of that line: the
    // same delta twice in a row reads the same both times, and three
    // different ones read as three separate lengths rather than 2, 5, 9.
    // Adding them up is the reading thread's job, and its alone.
    let lines = ["ab", "ab", "cde", "fghi"].map(|text| {
            format!(
                r#"{{"type":"stream_event","event":{{"type":"content_block_delta","index":1,"delta":{{"type":"text_delta","text":"{text}"}}}}}}"#
            )
        });

    let counted: Vec<_> = lines
        .iter()
        .map(|line| stream::read_line(line).activities)
        .collect();

    assert_eq!(
        counted,
        vec![
            vec![Activity::Writing { bytes: 2 }],
            vec![Activity::Writing { bytes: 2 }],
            vec![Activity::Writing { bytes: 3 }],
            vec![Activity::Writing { bytes: 4 }],
        ]
    );
}

#[test]
fn the_rest_of_a_partial_message_stream_is_read_as_nothing() {
    for line in [
        // A thinking block opening says what the `thinking_tokens` lines
        // already said, and one fact wants one source.
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}}"#,
        // The thought arriving in pieces. Not the words, and not their
        // size either: a thought is measured by the clock on its own line.
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCkYIBRgCKk"}}}"#,
        // The shape of a message being assembled.
        r#"{"type":"stream_event","event":{"type":"content_block_stop","index":1}}"#,
        r#"{"type":"stream_event","event":{"type":"message_start","message":{"role":"assistant"}}}"#,
        r#"{"type":"stream_event","event":{"type":"message_stop"}}"#,
        r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"}}}"#,
        // And the shapes that are not there at all. Half of these are
        // deltas now that deltas are looked at: no `delta`, a `text_delta`
        // with no `text`, and a `text` that is not a string. A count of
        // *something else* would be worse than no count.
        r#"{"type":"stream_event"}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_start"}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta"}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":128}}}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":{"was":"a string once"}}}}"#,
        r##"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"something_added_later","text":"# engine"}}}"##,
    ] {
        assert_eq!(
            stream::read_line(line),
            stream::Reading::default(),
            "nothing, and no panic, from {line:?}"
        );
    }
}

#[test]
fn every_other_system_line_is_still_somebody_elses_business() {
    // `init` names a working directory, `rate_limit_event` an allowance —
    // neither is a thing the pass is doing, and the panel says only what a
    // pass does.
    for line in [
        r#"{"type":"system","subtype":"init","cwd":"/repo/crates/engine"}"#,
        r#"{"type":"system","subtype":"something_added_later"}"#,
        r#"{"type":"system"}"#,
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#,
    ] {
        assert_eq!(
            stream::read_line(line),
            stream::Reading::default(),
            "nothing, and no panic, from {line:?}"
        );
    }
}

#[test]
fn a_stretch_of_writing_is_counted_up_and_each_block_counts_its_own() {
    // The reading thread over a canned stream and no child process: bytes
    // in on a `&[u8]`, activities out on a channel. Two text blocks with
    // an empty delta in the middle of the first, which is the shape that
    // would break a reader that took a zero for a fresh block.
    let stream = [
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
            r##"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"# En"}}}"##,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"gine"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":""}}}"#,
            // Two bytes, one character: the total climbs by what crossed the
            // pipe.
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"é"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0}}"#,
            // A second stretch of writing — a summarising pass, or a model
            // that resumed after a tool. Its own line, its own count.
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"ok"}}}"#,
            r##"{"type":"result","subtype":"success","result":"# Engineé","total_cost_usd":0.0042}"##,
        ]
        .join("\n");

    let (sender, received) = mpsc::channel();
    let activities = Activities::new(move |activity| {
        let _ = sender.send(activity);
    });
    let document = super::read(io::Cursor::new(stream), activities)
        .join()
        .expect("the reading thread does not panic")
        .expect("reading a slice of bytes cannot fail");
    let reported: Vec<Activity> = received.iter().collect();

    assert_eq!(
        reported,
        vec![
            // Nothing yet, the moment writing began.
            Activity::Writing { bytes: 0 },
            // Running totals, not deltas: 4, then 4 more.
            Activity::Writing { bytes: 4 },
            Activity::Writing { bytes: 8 },
            // The empty delta holds, and does not send the count home.
            Activity::Writing { bytes: 8 },
            Activity::Writing { bytes: 10 },
            // The second block starts again from nothing rather than
            // carrying the first one's ten.
            Activity::Writing { bytes: 0 },
            Activity::Writing { bytes: 2 },
            Activity::Cost { usd: 0.0042 },
        ]
    );

    // And, since the counts are the only thing that changed: the totals
    // never go backwards inside a stretch, and the document is still the
    // result line's field, with not one delta accumulated into it.
    for stretch in reported.split(|activity| *activity == Activity::Writing { bytes: 0 }) {
        assert!(
            stretch.windows(2).all(|pair| match pair {
                [
                    Activity::Writing { bytes: before },
                    Activity::Writing { bytes: after },
                ] => after >= before,
                _ => true,
            }),
            "a count went backwards in {stretch:?}"
        );
    }
    assert_eq!(document, "# Engineé");
}

#[test]
fn the_final_line_carries_the_document_and_what_the_pass_cost() {
    let line = r##"{"type":"result","subtype":"success","is_error":false,"duration_ms":8123,"result":"# Warlock\n\nThe freshness ledger.\n","total_cost_usd":0.0342,"usage":{"input_tokens":11}}"##;

    let reading = stream::read_line(line);

    assert_eq!(reading.activities, vec![Activity::Cost { usd: 0.0342 }]);
    // Verbatim, including the trailing newline: this field is what
    // `--print` prints.
    assert_eq!(
        reading.text.as_deref(),
        Some("# Warlock\n\nThe freshness ledger.\n")
    );
}

#[test]
fn a_result_line_missing_a_half_still_gives_up_the_other_one() {
    let costless = stream::read_line(r#"{"type":"result","result":"a document"}"#);
    assert_eq!(costless.activities, vec![]);
    assert_eq!(costless.text.as_deref(), Some("a document"));

    let textless = stream::read_line(r#"{"type":"result","total_cost_usd":1.5}"#);
    assert_eq!(textless.activities, vec![Activity::Cost { usd: 1.5 }]);
    assert_eq!(textless.text, None);

    // A cost that is not a number is no cost, not a failed pass.
    let nonsense = stream::read_line(r#"{"type":"result","total_cost_usd":"lots"}"#);
    assert_eq!(nonsense, stream::Reading::default());
}

#[test]
fn one_line_of_several_blocks_is_several_activities_in_order() {
    // What a real assistant message looks like when the model thinks, says
    // something, then calls two tools.
    let line = assistant(concat!(
        r#"{"type":"thinking","thinking":"which file"},"#,
        r#"{"type":"text","text":"Let me look."},"#,
        r#"{"type":"tool_use","name":"Grep","input":{"pattern":"TODO","path":"src"}},"#,
        r#"{"type":"tool_use","name":"Read","input":{"file_path":"src/app.rs"}}"#
    ));

    let reading = stream::read_line(&line);

    assert_eq!(
        reading.activities,
        vec![
            Activity::Thinking,
            Activity::Tool {
                name: "Grep".to_owned(),
                // The whitelisted key, not the first key, and not both.
                detail: Some("TODO".to_owned()),
            },
            Activity::Tool {
                name: "Read".to_owned(),
                detail: Some("src/app.rs".to_owned()),
            },
        ]
    );
    assert_eq!(reading.text, None);
}

#[test]
fn cancelling_with_no_pass_running_is_a_no_op_that_still_latches() {
    // No child registered, so there is nothing to kill; the flag is the
    // whole effect, and it is the half that stops the *next* pass.
    let cancel = Cancel::new();

    cancel.cancel();

    assert!(cancel.is_cancelled());
}

// The stand-ins below are shell scripts, so the whole module is Unix-only. What
// is under test — the pipes, the timeout, the kill — is not, but a portable
// stand-in would have to be a second binary to build.
#[cfg(unix)]
mod unix {
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use std::{env, fs, process, thread};

    use warlock_engine::{Agent, agent};

    use super::super::{Activities, Activity, Cancel, ClaudeAgent};

    // Only reached when something is already wrong; the waits themselves end as soon
    // as the pid file appears.
    const AT_MOST: Duration = Duration::from_secs(5);

    // A plausible pass in miniature, so that what a real stream looks like is written
    // down once: the opening line, a tool call, a thought beside the model's prose,
    // and the result line carrying the document and the cost.
    const PASS: [&str; 4] = [
        r#"{"type":"system","subtype":"init","tools":["Read","Bash"]}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Read","input":{"file_path":"src/lib.rs"}}]}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"a thought nobody is entitled to"},{"type":"text","text":"Here is the summary you asked for."}]}}"#,
        r##"{"type":"result","subtype":"success","result":"# module\n\nWhat it does.\n","total_cost_usd":0.0342}"##,
    ];

    const DOCUMENT: &str = "# module\n\nWhat it does.\n";

    fn reported() -> Vec<Activity> {
        vec![
            Activity::Tool {
                name: "Read".to_owned(),
                detail: Some("src/lib.rs".to_owned()),
            },
            Activity::Thinking,
            Activity::Cost { usd: 0.0342 },
        ]
    }

    fn stand_in(script: &str) -> ClaudeAgent {
        ClaudeAgent::new()
            .with_program("/bin/sh")
            .with_args(["-c", script])
    }

    // `printf '%s\n' a b c` repeats its format once per argument, so this is one
    // process, no loop, and every line arrives whole. The single quotes hold because
    // no canned line in this module contains one.
    fn printing(lines: &[&str]) -> String {
        let arguments: Vec<String> = lines.iter().map(|line| format!("'{line}'")).collect();
        format!("printf '%s\\n' {}", arguments.join(" "))
    }

    fn listening(agent: ClaudeAgent) -> (ClaudeAgent, mpsc::Receiver<Activity>) {
        let (sender, received) = mpsc::channel();
        let agent = agent.with_activities(Activities::new(move |activity| {
            let _ = sender.send(activity);
        }));
        (agent, received)
    }

    fn drained(received: &mpsc::Receiver<Activity>) -> Vec<Activity> {
        received.try_iter().collect()
    }

    fn is_cancelled(error: &agent::Error) -> bool {
        matches!(error, agent::Error::Io { source } if source.kind() == ErrorKind::Interrupted)
    }

    fn pid(path: &Path) -> Option<String> {
        let text = fs::read_to_string(path).ok()?;
        let pid = text.trim().to_owned();
        (!pid.is_empty()).then_some(pid)
    }

    // Stopped once the child is genuinely running, which it says by writing its pid
    // — a sleep here would be a race dressed up as a delay.
    fn cancel_once_running(cancel: Cancel, pid_file: &Path) -> thread::JoinHandle<()> {
        let pid_file = pid_file.to_owned();
        thread::spawn(move || {
            let waited = Instant::now();
            while pid(&pid_file).is_none() && waited.elapsed() < AT_MOST {
                thread::sleep(Duration::from_millis(10));
            }
            cancel.cancel();
        })
    }

    // Hand-rolled rather than a dependency: this crate's manifest gains nothing for
    // a temp directory.
    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory =
            env::temp_dir().join(format!("warlock-claude-{}-{name}-{unique}", process::id()));
        fs::create_dir_all(&directory).expect("a scratch directory under the temp directory");
        directory
    }

    fn clean_up(directory: &Path) {
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn a_clean_run_comes_back_as_the_document_the_stream_carried() {
        // `cat` is still the smallest possible model: it answers with the
        // prompt it was given, which proves the prompt reached stdin *and*
        // that stdin was closed — without EOF, `cat` would never return.
        // What it is given is now the result line of a stream, so the same
        // test also shows the parse working on bytes out of a real pipe
        // rather than on a string literal.
        let (agent, received) = listening(
            ClaudeAgent::new()
                .with_program("/bin/cat")
                .with_args(Vec::<&str>::new()),
        );

        let response = agent
            .run(&agent::Request::new(format!("{}\n", PASS[3]), "."))
            .expect("cat exits cleanly and prints what it was given");

        assert_eq!(response.text(), DOCUMENT);
        assert_eq!(drained(&received), vec![Activity::Cost { usd: 0.0342 }]);
    }

    #[test]
    fn a_whole_pass_reports_what_it_did_and_returns_its_document() {
        let (agent, received) = listening(stand_in(&printing(&PASS)));

        let response = agent
            .run(&agent::Request::new("anything", "."))
            .expect("the canned pass exits cleanly and prints a document");

        // Byte for byte the result line's own field, newlines and all.
        assert_eq!(response.text(), DOCUMENT);
        let activities = drained(&received);
        assert_eq!(activities, reported());
        // Said once more, because it is the promise the port is for: none
        // of the thought and none of the prose came with it.
        let seen = format!("{activities:?}");
        assert!(!seen.contains("entitled"), "{seen}");
        assert!(!seen.contains("summary"), "{seen}");
    }

    #[test]
    fn an_activity_reaches_the_port_while_the_pass_is_still_running() {
        // The whole reason for reading a line at a time: the tool call is
        // reported, and only then does the child get around to finishing.
        // A drain-to-EOF reader passes every other test in this module and
        // fails this one.
        let script = format!(
            "{}; sleep 1; {}",
            printing(&PASS[..2]),
            printing(&PASS[3..])
        );
        let (agent, received) = listening(stand_in(&script));

        let started = Instant::now();
        let pass = thread::spawn(move || agent.run(&agent::Request::new("anything", ".")));
        let first = received
            .recv_timeout(AT_MOST)
            .expect("the tool call is reported as it happens");
        let reported_after = started.elapsed();
        // Asked of the run itself rather than of the clock: the child is
        // a second into its sleep at this point, so a thread that has
        // already returned would mean the activity only turned up once
        // the pass was over.
        let still_running = !pass.is_finished();
        let response = pass
            .join()
            .expect("the pass ran")
            .expect("the canned pass exits cleanly");
        let finished_after = started.elapsed();

        assert_eq!(first, reported()[0]);
        assert!(
            still_running,
            "the pass had already returned by the time its first activity \
                 arrived: that is a drain, not a stream"
        );
        assert!(
            reported_after + Duration::from_millis(300) < finished_after,
            "the activity arrived at {reported_after:?} and the pass ended at \
                 {finished_after:?}: that is not streaming"
        );
        assert_eq!(response.text(), DOCUMENT);
    }

    #[test]
    fn garbage_in_the_stream_costs_neither_the_document_nor_an_activity() {
        // A warning on stdout, a half-written line, and an event from a
        // future version of the CLI. None of it is a reason to throw away
        // minutes of work and a written document.
        let lines = [
            "Warning: something the CLI felt like mentioning",
            PASS[1],
            "{not json",
            r#"{"type":"kraken","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
            PASS[2],
            "",
            PASS[3],
        ];
        let (agent, received) = listening(stand_in(&printing(&lines)));

        let response = agent
            .run(&agent::Request::new("anything", "."))
            .expect("a stream with junk in it still produced a document");

        assert_eq!(response.text(), DOCUMENT);
        assert_eq!(drained(&received), reported());
    }

    #[test]
    fn nothing_from_a_tool_result_reaches_the_port_however_big_it_is() {
        // What a tool *returned* is the one thing in the stream with no
        // upper bound: a file, a build log, a screenful of grep. It is
        // also no sign of life — the tool call above it already said what
        // was happening — so it is worth proving that a quarter of a
        // megabyte of it goes past the port without a byte getting out,
        // in both places a block of that type can turn up.
        //
        // The payload is built by the shell rather than written here: a
        // quarter of a megabyte inside `sh -c` would be a single argument
        // past what the kernel will take, and this test would fail for a
        // reason that has nothing to do with what it is about.
        let payload = r"payload=$(yes gribbleflix | head -n 20000 | tr '\n' ' ')";
        let returned = r#"printf '{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"%s"}]}}\n' "$payload""#;
        let misfiled = r#"printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"%s"}]}}\n' "$payload""#;
        let script = [
            payload,
            &printing(&PASS[..2]),
            returned,
            misfiled,
            &printing(&PASS[3..]),
        ]
        .join("; ");
        let (agent, received) = listening(stand_in(&script));

        let response = agent
            .run(&agent::Request::new("anything", "."))
            .expect("a pass that read something still writes its document");

        assert_eq!(response.text(), DOCUMENT);
        let activities = drained(&received);
        // Asked first, and asked by length rather than by printing what
        // leaked: a failure here is half a megabyte wide, and a test that
        // fails by filling somebody's terminal is a bad way to find out.
        let seen = format!("{activities:?}");
        assert!(
            !seen.contains("gribbleflix"),
            "a tool's output reached the port: {} bytes of activity",
            seen.len()
        );
        // The call and the cost, and nothing in between: the result is
        // not an activity, so it is not a line of a panel either.
        assert_eq!(
            activities,
            vec![reported()[0].clone(), Activity::Cost { usd: 0.0342 }]
        );
    }

    #[test]
    fn a_thought_reaches_the_port_as_the_bare_fact_that_there_was_one() {
        // Thinking is shown as *that it happened*, never as what it said.
        // The text is the model reasoning with itself, and this front end
        // does not put prose on the screen; the promise is only worth
        // anything if the words never leave the transport at all.
        let secret = "the person asking has misunderstood their own schema";
        let thought = format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"thinking","thinking":"{secret}","signature":"abc123"}}]}}}}"#
        );
        let (agent, received) = listening(stand_in(&printing(&[thought.as_str(), PASS[3]])));

        let response = agent
            .run(&agent::Request::new("anything", "."))
            .expect("a pass that thought about it still writes its document");

        assert_eq!(response.text(), DOCUMENT);
        let activities = drained(&received);
        assert_eq!(
            activities,
            vec![Activity::Thinking, Activity::Cost { usd: 0.0342 }]
        );
        let seen = format!("{activities:?}");
        assert!(!seen.contains("misunderstood"), "{seen}");
        assert!(!seen.contains("abc123"), "{seen}");
    }

    #[test]
    fn what_the_pass_cost_comes_back_from_its_result_line() {
        // The number the CLI totalled up, not one this crate works out:
        // reported as it stands, after everything the pass did, because
        // the result line is the last line there is.
        let expensive = r##"{"type":"result","subtype":"success","result":"# module\n\nWhat it does.\n","total_cost_usd":1.25}"##;
        let (agent, received) = listening(stand_in(&printing(&[PASS[1], expensive])));

        let response = agent
            .run(&agent::Request::new("anything", "."))
            .expect("the canned pass exits cleanly");

        assert_eq!(response.text(), DOCUMENT);
        assert_eq!(
            drained(&received),
            vec![reported()[0].clone(), Activity::Cost { usd: 1.25 }]
        );
    }

    #[test]
    fn a_pass_nobody_listens_to_runs_exactly_as_it_did_before() {
        // The port is a side channel and must stay one. Every kind of
        // ending this transport has — a document, a refusal, a silence,
        // a stream that never got to a result, a directory that is not
        // there — run twice, once through an agent with a port attached
        // and once through one without, and the two are compared. A
        // difference either way would mean listening had changed the run.
        let elsewhere = "/warlock/no/such/directory";
        let passes = [
            (printing(&PASS), "."),
            ("echo boom >&2; exit 3".to_owned(), "."),
            ("exit 0".to_owned(), "."),
            (printing(&PASS[..3]), "."),
            ("true".to_owned(), elsewhere),
        ];

        for (script, directory) in passes {
            let deaf = stand_in(&script).run(&agent::Request::new("anything", directory));
            let (agent, received) = listening(stand_in(&script));
            let heard = agent.run(&agent::Request::new("anything", directory));

            // `agent::Error` is not comparable — it carries an
            // `io::Error` — so the two endings are compared as they are
            // written down, which is what a caller would see of them.
            assert_eq!(
                format!("{deaf:?}"),
                format!("{heard:?}"),
                "`{script}` in {directory} ended differently with somebody listening"
            );
            // And the reporting itself still happened, so this is not
            // passing because the port was quietly disconnected.
            if script == printing(&PASS) {
                assert_eq!(drained(&received), reported());
            }
        }
    }

    #[test]
    fn the_pass_runs_in_the_directory_the_request_names() {
        let directory = scratch("cwd");
        fs::write(directory.join("marker.txt"), "here").expect("a file to look for");

        // `ls`, wrapped in the result line a pass would have wrapped it in.
        let response =
            stand_in(r#"printf '{"type":"result","result":"%s"}\n' "$(ls | tr '\n' ' ')""#)
                .run(&agent::Request::new("ignored", &directory))
                .expect("ls exits cleanly and prints a name");

        assert!(
            response.text().contains("marker.txt"),
            "the child ran somewhere else: {}",
            response.text()
        );
        clean_up(&directory);
    }

    #[test]
    fn a_non_zero_exit_carries_its_status_and_its_stderr() {
        let error = stand_in("echo boom >&2; exit 3")
            .run(&agent::Request::new("anything", "."))
            .expect_err("this stand-in refuses");

        match error {
            agent::Error::Failed { code, stderr } => {
                assert_eq!(code, Some(3));
                assert_eq!(stderr.trim(), "boom", "stderr is captured, not dropped");
            }
            other => panic!("expected a non-zero exit, got {other:?}"),
        }
    }

    #[test]
    fn a_clean_run_that_says_nothing_is_empty_output() {
        // Four ways to say nothing: no output at all, blank lines, a whole
        // stream that never carried a result line, and a result line whose
        // document is whitespace. The last two are new shapes of the same
        // old answer — a document of blank lines is no document, and so is
        // a pass that produced none.
        let scripts = [
            "exit 0".to_owned(),
            "printf '\\n  \\n'".to_owned(),
            printing(&PASS[..3]),
            printing(&[r#"{"type":"result","result":"  \n\t"}"#]),
        ];

        for script in scripts {
            let error = stand_in(&script)
                .run(&agent::Request::new("anything", "."))
                .expect_err("there is no document in silence");

            assert!(
                matches!(error, agent::Error::EmptyOutput),
                "`{script}` gave {error:?}"
            );
        }
    }

    #[test]
    fn a_missing_directory_is_io_rather_than_a_missing_binary() {
        // The syscall says `NotFound` for both; only one of them deserves
        // the message telling the user to install `claude`.
        let error = stand_in("true")
            .run(&agent::Request::new(
                "anything",
                "/warlock/no/such/directory",
            ))
            .expect_err("nothing can run in a directory that is not there");

        assert!(matches!(error, agent::Error::Io { .. }), "{error:?}");
    }

    #[test]
    fn a_big_prompt_and_a_chatty_child_do_not_deadlock() {
        // Both directions past a pipe buffer at once: the prompt is bigger
        // than one, and so is the stream. Reading on a thread is what makes
        // this return at all, and reading by line rather than to EOF must
        // not have quietly reintroduced the block — a reader that stopped
        // consuming would leave this child wedged on a full pipe forever.
        let prompt = "x".repeat(200_000);
        let chatter = PASS[2];
        let script = format!(
            "cat > /dev/null; yes '{chatter}' | head -n 20000; {}",
            printing(&PASS[3..])
        );

        let (agent, received) = listening(stand_in(&script));
        let response = agent
            .run(&agent::Request::new(prompt, "."))
            .expect("a chatty stand-in still exits cleanly");

        // Roughly two megabytes of stream, every line of it read and every
        // thought in it reported, then the document at the end.
        assert_eq!(response.text(), DOCUMENT);
        let activities = drained(&received);
        assert_eq!(activities.len(), 20_001);
        assert!(
            activities[..20_000]
                .iter()
                .all(|activity| *activity == Activity::Thinking)
        );
        assert_eq!(activities[20_000], Activity::Cost { usd: 0.0342 });
    }

    #[test]
    fn a_hanging_pass_times_out_and_its_child_stops() {
        let directory = scratch("hang");
        let ticks = directory.join("ticks");
        // Never exits on its own, and says so in a file: whether it is
        // still running after the call is a question the test can ask.
        let agent = stand_in("while :; do echo tick >> ticks; sleep 0.05; done")
            .with_timeout(Duration::from_millis(250));

        let started = Instant::now();
        let error = agent
            .run(&agent::Request::new("anything", &directory))
            .expect_err("this stand-in never finishes");
        let elapsed = started.elapsed();

        match error {
            agent::Error::TimedOut { after } => assert_eq!(after, Duration::from_millis(250)),
            other => panic!("expected a timeout, got {other:?}"),
        }
        assert!(
            elapsed < Duration::from_secs(10),
            "the call waited {elapsed:?}, far past its timeout"
        );

        let before = fs::metadata(&ticks).map_or(0, |file| file.len());
        thread::sleep(Duration::from_millis(300));
        let after = fs::metadata(&ticks).map_or(0, |file| file.len());
        assert_eq!(
            before, after,
            "the child outlived the call that gave up on it"
        );
        clean_up(&directory);
    }

    // The kill is only half of it — a child nobody waits on stays in the process
    // table. `/proc` is where that is visible, so this test alone is Linux-only; the
    // kill itself is covered on every Unix above.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_timed_out_child_is_reaped_not_left_a_zombie() {
        let directory = scratch("reap");
        let agent = stand_in("echo $$ > pid; sleep 30").with_timeout(Duration::from_millis(250));

        let started = Instant::now();
        let error = agent
            .run(&agent::Request::new("anything", &directory))
            .expect_err("this stand-in sleeps far past its timeout");
        let elapsed = started.elapsed();

        assert!(matches!(error, agent::Error::TimedOut { .. }), "{error:?}");
        assert!(
            elapsed < Duration::from_secs(20),
            "the call outlasted the sleep it was supposed to cut short: {elapsed:?}"
        );

        let pid = fs::read_to_string(directory.join("pid")).expect("the child wrote its pid");
        let pid = pid.trim();
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "process {pid} is still in the table: killed but never reaped"
        );
        clean_up(&directory);
    }

    #[test]
    fn a_cancel_from_another_thread_ends_the_pass_promptly() {
        let directory = scratch("cancel");
        let pid_file = directory.join("pid");
        let cancel = Cancel::new();
        // The real five-minute timeout: the only thing that can end this
        // call in time is the cancel.
        let agent = stand_in("echo $$ > pid; sleep 30").with_cancel(cancel.clone());

        let stopper = cancel_once_running(cancel, &pid_file);

        let started = Instant::now();
        let error = agent
            .run(&agent::Request::new("anything", &directory))
            .expect_err("a cancelled pass has no document");
        let elapsed = started.elapsed();
        stopper.join().expect("the cancelling thread ran");

        assert!(is_cancelled(&error), "{error:?}");
        assert!(
            elapsed < Duration::from_secs(20),
            "the call sat out the sleep it was told to cut short: {elapsed:?}"
        );
        clean_up(&directory);
    }

    // `sh -c "echo $$ > pid; sleep 30"` is one process under a shell that execs its
    // last command and two under one that forks, so only sometimes does anything
    // outlive the kill. This script forks on purpose — `wait` is a builtin, so no
    // shell can exec away — and pins the behaviour on both. It is also the shape a
    // real `claude` has: a tool subprocess inheriting the pipes it was given.
    #[test]
    fn a_cancel_does_not_wait_on_output_a_survivor_still_holds() {
        let directory = scratch("cancel-survivor");
        let pid_file = directory.join("pid");
        let survivor_file = directory.join("survivor");
        let cancel = Cancel::new();
        let agent = stand_in("sleep 30 & echo $! > survivor; echo $$ > pid; wait")
            .with_cancel(cancel.clone());

        let stopper = cancel_once_running(cancel, &pid_file);

        let started = Instant::now();
        let error = agent
            .run(&agent::Request::new("anything", &directory))
            .expect_err("a cancelled pass has no document");
        let elapsed = started.elapsed();
        stopper.join().expect("the cancelling thread ran");

        assert!(is_cancelled(&error), "{error:?}");
        assert!(
            elapsed < Duration::from_secs(20),
            "the call waited on output the survivor was still holding: {elapsed:?}"
        );
        // The survivor is the point of the test, so it is this test's to
        // clear up. Nothing else can: the kill reaches the child, and this
        // one was never the child.
        if let Some(survivor) = pid(&survivor_file) {
            let _ = process::Command::new("/bin/kill").arg(survivor).status();
        }
        clean_up(&directory);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_cancelled_childs_process_is_gone_afterwards() {
        let directory = scratch("cancel-reap");
        let pid_file = directory.join("pid");
        let cancel = Cancel::new();
        let agent = stand_in("echo $$ > pid; sleep 30").with_cancel(cancel.clone());

        let stopper = cancel_once_running(cancel, &pid_file);

        let error = agent
            .run(&agent::Request::new("anything", &directory))
            .expect_err("a cancelled pass has no document");
        stopper.join().expect("the cancelling thread ran");

        assert!(is_cancelled(&error), "{error:?}");
        let pid = pid(&pid_file).expect("the child wrote its pid before it was stopped");
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "process {pid} survived the cancel, or was killed and never reaped"
        );
        clean_up(&directory);
    }

    #[test]
    fn a_pass_started_after_a_cancel_spawns_nothing_at_all() {
        let directory = scratch("never-started");
        let marker = directory.join("marker");
        let cancel = Cancel::new();
        cancel.cancel();
        // Anything that ran would leave a file behind, and the call only
        // returns once its child has exited — so a missing marker is a
        // child that never existed, not one that has not got there yet.
        let agent = stand_in("touch marker").with_cancel(cancel);

        let error = agent
            .run(&agent::Request::new("anything", &directory))
            .expect_err("a cancelled agent runs nothing");

        assert!(is_cancelled(&error), "{error:?}");
        assert!(!marker.exists(), "a cancelled agent spawned a child anyway");
        clean_up(&directory);
    }

    #[test]
    fn a_handle_nobody_cancels_leaves_the_run_exactly_as_it_was() {
        let cancel = Cancel::new();
        let agent = stand_in(&printing(&PASS)).with_cancel(cancel.clone());

        let response = agent
            .run(&agent::Request::new("anything", "."))
            .expect("attaching a handle does not change a clean run");

        assert_eq!(response.text(), DOCUMENT);
        // The pass is over and the handle knows it: this reaches for a
        // child that is no longer registered, and returns rather than
        // killing whatever came next.
        cancel.cancel();
        assert!(cancel.is_cancelled());
    }

    // A child module rather than a sibling so every stand-in above is reusable: the
    // same machinery reached through a different door, and a second set of helpers
    // would let the two drift. What differs is where a stand-in is pointed — a pass
    // runs in the directory its request names, a turn wherever warlock does, which
    // here is this source tree, so every script below names its files absolutely.
    mod turns {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};
        use std::{fs, thread};

        use warlock_engine::agent;

        use super::super::NOT_A_PROGRAM;
        use super::{cancel_once_running, clean_up, drained, is_cancelled, pid, printing, scratch};
        use crate::{Activities, Activity, Cancel, ChatAgent};

        // [`PASS`](super::PASS)'s counterpart, deliberately not the same canned stream: a
        // turn can call a tool, and it says one thing a toolless pass never does — the
        // moment it stops thinking and starts writing.
        const TURN: [&str; 6] = [
            r#"{"type":"system","subtype":"init","tools":["Read","Grep","Glob"]}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Grep","input":{"pattern":"fn load"}}]}}"#,
            r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":113,"estimated_tokens_delta":63}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"The loader is in src/load.rs."}]}}"#,
            r#"{"type":"result","subtype":"success","result":"The loader is in src/load.rs.","total_cost_usd":0.0042}"#,
        ];

        const ANSWER: &str = "The loader is in src/load.rs.";

        fn reported() -> Vec<Activity> {
            vec![
                Activity::Tool {
                    name: "Grep".to_owned(),
                    detail: Some("fn load".to_owned()),
                },
                Activity::Thinking,
                // Zero, and only zero: [`TURN`] opens a text block and
                // never sends a delta, so the count begins and the answer
                // arrives whole on the result line.
                Activity::Writing { bytes: 0 },
                Activity::Cost { usd: 0.0042 },
            ]
        }

        fn stand_in(script: &str) -> ChatAgent {
            ChatAgent::new()
                .with_program("/bin/sh")
                .with_args(["-c", script])
        }

        fn listening(agent: ChatAgent) -> (ChatAgent, mpsc::Receiver<Activity>) {
            let (sender, received) = mpsc::channel();
            let agent = agent.with_activities(Activities::new(move |activity| {
                let _ = sender.send(activity);
            }));
            (agent, received)
        }

        #[test]
        fn a_turns_stdin_is_the_message_and_not_one_byte_more() {
            // The promise the whole thread card rests on: what reaches the
            // model is the sentence somebody typed, with no tree dump, no
            // repository contents and no transcript warlock appended. So
            // the stand-in keeps its stdin instead of describing it, and
            // the bytes are compared with the message as written —
            // including the absence of a trailing newline, which is the
            // easiest thing for a transport to add without meaning to.
            let directory = scratch("turn-stdin");
            let captured = directory.join("stdin");
            let message = "where is the loader?\n\nand a second paragraph, with a 'quote' in it";
            let script = format!("cat > '{}'; {}", captured.display(), printing(&TURN[5..]));

            let answer = stand_in(&script)
                .turn(message)
                .expect("the stand-in exits cleanly and prints a result line");

            assert_eq!(answer, ANSWER);
            assert_eq!(
                fs::read(&captured).expect("the stand-in kept its stdin"),
                message.as_bytes(),
                "something other than the message reached the child",
            );

            // And `cat` says the other half of it. It answers with what it
            // was given and returns only at EOF, so a turn that comes back
            // at all is a turn whose stdin was written *and closed* —
            // without the close, this call would hang until the timeout.
            let echoed = ChatAgent::new()
                .with_program("/bin/cat")
                .with_args(Vec::<&str>::new())
                .turn(&format!("{}\n", TURN[5]))
                .expect("cat exits cleanly once its stdin is closed");

            assert_eq!(echoed, ANSWER);
            clean_up(&directory);
        }

        #[test]
        fn a_whole_turn_reports_what_it_did_and_returns_the_answer() {
            // The four kinds of sign of life a turn gives, out of one
            // canned stream and through a real pipe: the tool it reached
            // for, that it thought, that it began writing, and what it
            // cost.
            let (agent, received) = listening(stand_in(&printing(&TURN)));

            let answer = agent
                .turn("where is the loader?")
                .expect("the canned turn exits cleanly and prints an answer");

            assert_eq!(answer, ANSWER);
            let activities = drained(&received);
            assert_eq!(activities, reported());
            // The other half of the promise, and the one worth saying
            // twice: the answer is the turn's return value, and no part of
            // it went out over the port. A panel that showed the prose
            // would be a viewer rather than a ledger.
            let seen = format!("{activities:?}");
            assert!(!seen.contains("The loader"), "{seen}");
            assert!(!seen.contains("src/load.rs"), "{seen}");
        }

        #[test]
        fn a_missing_binary_is_reported_by_name_not_as_an_errno() {
            // No `claude` needed to test the no-`claude` case, which is the
            // point: this is the state of every machine that has never
            // installed it. And a turn names no directory of its own, so
            // there is no second thing a `NotFound` could have been.
            let error = ChatAgent::new()
                .with_program(NOT_A_PROGRAM)
                .turn("anything")
                .expect_err("nothing by that name can be on PATH");

            match error {
                agent::Error::NotFound { program } => assert_eq!(program, NOT_A_PROGRAM),
                other => panic!("expected a missing binary, got {other:?}"),
            }
        }

        #[test]
        fn a_turn_that_refuses_carries_its_status_and_its_stderr() {
            let error = stand_in("echo boom >&2; exit 3")
                .turn("anything")
                .expect_err("this stand-in refuses");

            match error {
                agent::Error::Failed { code, stderr } => {
                    assert_eq!(code, Some(3));
                    assert_eq!(stderr.trim(), "boom", "stderr is captured, not dropped");
                }
                other => panic!("expected a non-zero exit, got {other:?}"),
            }
        }

        #[test]
        fn a_turn_that_says_nothing_is_empty_output() {
            // The same four silences a pass has: nothing at all, blank
            // lines, a stream that never reached a result line, and a
            // result line whose answer is whitespace.
            let scripts = [
                "exit 0".to_owned(),
                "printf '\\n  \\n'".to_owned(),
                printing(&TURN[..5]),
                printing(&[r#"{"type":"result","result":"  \n\t"}"#]),
            ];

            for script in scripts {
                let error = stand_in(&script)
                    .turn("anything")
                    .expect_err("there is no answer in silence");

                assert!(
                    matches!(error, agent::Error::EmptyOutput),
                    "`{script}` gave {error:?}"
                );
            }
        }

        #[test]
        fn a_hanging_turn_times_out_and_its_child_stops() {
            let directory = scratch("turn-hang");
            let ticks = directory.join("ticks");
            // Never exits on its own, and says so in a file: whether it is
            // still running after the call is a question the test can ask.
            let agent = stand_in(&format!(
                "while :; do echo tick >> '{}'; sleep 0.05; done",
                ticks.display()
            ))
            .with_timeout(Duration::from_millis(250));

            let started = Instant::now();
            let error = agent
                .turn("anything")
                .expect_err("this stand-in never finishes");
            let elapsed = started.elapsed();

            match error {
                agent::Error::TimedOut { after } => {
                    assert_eq!(after, Duration::from_millis(250));
                }
                other => panic!("expected a timeout, got {other:?}"),
            }
            assert!(
                elapsed < Duration::from_secs(10),
                "the call waited {elapsed:?}, far past its timeout"
            );

            let before = fs::metadata(&ticks).map_or(0, |file| file.len());
            thread::sleep(Duration::from_millis(300));
            let after = fs::metadata(&ticks).map_or(0, |file| file.len());
            assert_eq!(
                before, after,
                "the child outlived the turn that gave up on it"
            );
            clean_up(&directory);
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn a_timed_out_turns_child_is_reaped_not_left_a_zombie() {
            let directory = scratch("turn-reap");
            let pid_file = directory.join("pid");
            let agent = stand_in(&format!("echo $$ > '{}'; sleep 30", pid_file.display()))
                .with_timeout(Duration::from_millis(250));

            let error = agent
                .turn("anything")
                .expect_err("this stand-in sleeps far past its timeout");

            assert!(matches!(error, agent::Error::TimedOut { .. }), "{error:?}");
            let pid = pid(&pid_file).expect("the child wrote its pid");
            assert!(
                !std::path::Path::new(&format!("/proc/{pid}")).exists(),
                "process {pid} is still in the table: killed but never reaped"
            );
            clean_up(&directory);
        }

        #[test]
        fn a_cancel_from_another_thread_ends_a_turn_promptly() {
            let directory = scratch("turn-cancel");
            let pid_file = directory.join("pid");
            let cancel = Cancel::new();
            // The real five-minute timeout: the only thing that can end
            // this call in time is the cancel.
            let agent = stand_in(&format!("echo $$ > '{}'; sleep 30", pid_file.display()))
                .with_cancel(cancel.clone());

            let stopper = cancel_once_running(cancel, &pid_file);

            let started = Instant::now();
            let error = agent
                .turn("anything")
                .expect_err("a cancelled turn has no answer");
            let elapsed = started.elapsed();
            stopper.join().expect("the cancelling thread ran");

            assert!(is_cancelled(&error), "{error:?}");
            assert!(
                elapsed < Duration::from_secs(20),
                "the call sat out the sleep it was told to cut short: {elapsed:?}"
            );
            clean_up(&directory);
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn a_cancelled_turns_process_is_gone_afterwards() {
            let directory = scratch("turn-cancel-reap");
            let pid_file = directory.join("pid");
            let cancel = Cancel::new();
            let agent = stand_in(&format!("echo $$ > '{}'; sleep 30", pid_file.display()))
                .with_cancel(cancel.clone());

            let stopper = cancel_once_running(cancel, &pid_file);

            let error = agent
                .turn("anything")
                .expect_err("a cancelled turn has no answer");
            stopper.join().expect("the cancelling thread ran");

            assert!(is_cancelled(&error), "{error:?}");
            let pid = pid(&pid_file).expect("the child wrote its pid before it was stopped");
            assert!(
                !std::path::Path::new(&format!("/proc/{pid}")).exists(),
                "process {pid} survived the cancel, or was killed and never reaped"
            );
            clean_up(&directory);
        }

        #[test]
        fn a_turn_started_after_a_cancel_spawns_nothing_at_all() {
            let directory = scratch("turn-never-started");
            let marker = directory.join("marker");
            let cancel = Cancel::new();
            cancel.cancel();
            // Anything that ran would leave a file behind, and a turn only
            // returns once its child has exited — so a missing marker is a
            // child that never existed, not one that has not got there yet.
            let agent = stand_in(&format!("touch '{}'", marker.display())).with_cancel(cancel);

            let error = agent
                .turn("anything")
                .expect_err("a cancelled agent takes no turns");

            assert!(is_cancelled(&error), "{error:?}");
            assert!(!marker.exists(), "a cancelled agent spawned a child anyway");
            clean_up(&directory);
        }

        #[test]
        fn a_turn_nobody_listens_to_runs_exactly_as_it_did_before() {
            // The port is a side channel here too. Every kind of ending a
            // turn has — an answer, a refusal, a silence, a stream that
            // never reached its result line — run twice, once with a
            // listener and once without, and compared as a caller would
            // see them.
            let endings = [
                printing(&TURN),
                "echo boom >&2; exit 3".to_owned(),
                "exit 0".to_owned(),
                printing(&TURN[..5]),
            ];

            for script in endings {
                let deaf = stand_in(&script).turn("anything");
                let (agent, received) = listening(stand_in(&script));
                let heard = agent.turn("anything");

                // `agent::Error` is not comparable — it carries an
                // `io::Error` — so the two endings are compared as they are
                // written down.
                assert_eq!(
                    format!("{deaf:?}"),
                    format!("{heard:?}"),
                    "`{script}` ended differently with somebody listening"
                );
                if script == printing(&TURN) {
                    assert_eq!(drained(&received), reported());
                }
            }
        }
    }
}

#[test]
fn a_failure_arrives_quickly_rather_than_after_the_timeout() {
    // The timeout is a backstop, not a delay every failure pays.
    let agent = ClaudeAgent::new()
        .with_program(NOT_A_PROGRAM)
        .with_timeout(INVOCATION_TIMEOUT);

    let started = Instant::now();
    let _ = agent.run(&agent::Request::new("anything", "."));

    assert!(started.elapsed() < Duration::from_secs(5));
}
