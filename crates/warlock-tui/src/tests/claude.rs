use std::fmt::Write as _;
use std::io;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::stream;
use super::{
    Activities, Activity, BRIEF_EFFORT, BRIEF_MODEL, Bounded, CHAT_INSTRUCTION, CHAT_SYSTEM_PROMPT,
    Cancel, ChatAgent, ClaudeAgent, Converses, DRAFT_NOW_INSTRUCTION, DRAFTING_CONTRACT,
    DRAFTING_ONE_SHOT_CONTRACT, DRAFTING_ROUNDS, Drafted, Drafting, EFFORT, EFFORT_VAR,
    INVOCATION_TIMEOUT, MODEL, MODEL_VAR, NOTHING_SETTLES_IT, OsString, PROPOSING_SYSTEM_PROMPT,
    Replied, SYSTEM_PROMPT, Stopped, WORKING_ATTEMPTS, WORKING_TIMEOUT, WORKING_TURNS,
    WRITE_INSTRUCTION, Wired, Worked, Working, brief_instruction, drafting_opening, or_default,
    overridden, propose_answer, proposing_instruction, render, session_id, working_opening,
    working_retry, working_system_prompt,
};
use crate::brief::scope_block_in;
use crate::panel::Mode;
use crate::template::DEFAULT_TEMPLATE;
use warlock_engine::{Agent, agent, drafting, working};

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

// Stands in for the prompt sub-task 4 writes: this file is about the fence, and
// none of the assertions below read a word of what the session is told. Plain on
// purpose — no capitalised tool name in it — so that a vector found to name
// `Edit` is the grant naming it and not this string.
const WORKING_PROMPT: &str = "You are working one sub-task of one ticket.";

#[test]
fn the_sub_task_session_is_granted_exactly_six_tools_in_both_flags() {
    let vector = turn_args(&ChatAgent::working(WORKING_PROMPT));
    let granted = value_of(&vector, "--tools").expect("the sub-task session says what it holds");

    // Exactly these, in this order, and the same list in both flags:
    // `--tools` is what the session has at all and `--allowedTools` is what it
    // may use without stopping to ask a person who is not there.
    assert_eq!(granted, "Read,Grep,Glob,Edit,Write,Bash");
    assert_eq!(
        granted.split(',').collect::<Vec<_>>(),
        ["Read", "Grep", "Glob", "Edit", "Write", "Bash"],
    );
    assert_eq!(value_of(&vector, "--allowedTools"), Some(granted));

    // One of each flag, so neither grant is a first word some later pair
    // quietly widens.
    for flag in ["--tools", "--allowedTools"] {
        assert_eq!(
            vector.iter().filter(|word| *word == flag).count(),
            1,
            "{flag} is named twice in the sub-task session's vector",
        );
    }

    // And nothing else that hands a tool over or waves a permission through:
    // the grant above is the whole of what this session may do.
    for refused in [
        "--permission-mode",
        "--dangerously-skip-permissions",
        "--allow-dangerously-skip-permissions",
        "--add-dir",
        "--agents",
        "--mcp-config",
    ] {
        assert!(
            !vector.iter().any(|word| word == refused),
            "{refused} reached the sub-task session's vector",
        );
    }
}

#[test]
fn the_sub_task_session_reaches_no_machine_settings_and_no_mcp_server() {
    let vector = turn_args(&ChatAgent::working(WORKING_PROMPT));

    // This machine's user, project and local settings are refused, so the
    // session is told what warlock told it and not what somebody's global
    // hooks or a repository's own instructions would say.
    assert_eq!(value_of(&vector, "--setting-sources"), Some(""));
    // And no MCP server at all: `--strict-mcp-config` with nothing beside it
    // is the only way the session cannot reach Linear — settings sources are
    // not where MCP servers come from. So warlock is the only route out.
    assert!(vector.iter().any(|word| word == "--strict-mcp-config"));
    assert!(!vector.iter().any(|word| word == "--mcp-config"));

    // Both flags together, which is sub-task 2's finding and not an
    // assumption: a `PreToolUse` hook given with `--settings` still loads
    // under `--setting-sources ""`, established against the real CLI by
    // `a_pre_tool_use_hook_given_with_settings_loads_under_no_setting_sources`
    // below. Were that ever to change, the hook stays and this flag goes.
    assert!(vector.iter().any(|word| word == "--settings"));
}

#[test]
fn the_sub_task_session_carries_the_gate_hook_inline_with_no_file_on_disk() {
    let vector = turn_args(&ChatAgent::working(WORKING_PROMPT));
    let settings = value_of(&vector, "--settings").expect("the sub-task session brings its fence");

    // Inline JSON and not a path: `--settings` takes either, and a file would
    // be settings on disk outliving a session that may have been killed, plus
    // a path for somebody to clean up. Asserted both ways round — it parses as
    // an object, and there is no such file to have been written.
    assert!(settings.starts_with('{'), "not inline JSON: {settings}");
    assert!(
        !std::path::Path::new(settings).exists(),
        "the fence was written to a file: {settings}",
    );
    let settings: serde_json::Value =
        serde_json::from_str(settings).expect("the fence is a JSON object");

    let hooks = settings["hooks"]["PreToolUse"]
        .as_array()
        .expect("a PreToolUse hook");
    assert_eq!(hooks.len(), 1);
    let matcher = hooks[0]["matcher"].as_str().expect("a matcher");
    for gated in ["Edit", "Write", "MultiEdit", "NotebookEdit"] {
        assert!(
            matcher.split('|').any(|named| named == gated),
            "{gated} is not gated: {matcher}",
        );
    }
    // `Bash` is not in the matcher and cannot be: a hook is handed a command
    // string rather than a path, and a shell line writes wherever the operator
    // can. A fact about the fence, asserted so it stays a stated one.
    assert!(!matcher.split('|').any(|named| named == "Bash"));

    let ran = hooks[0]["hooks"].as_array().expect("one command to run");
    assert_eq!(ran.len(), 1);
    assert_eq!(ran[0]["type"], "command");
    let command = ran[0]["command"].as_str().expect("a command line");
    assert!(command.ends_with("check --gate"), "not the gate: {command}");
    // Warlock's own binary, named absolutely, because the hook runs in a child
    // of `claude` whose working directory is the repository being worked.
    let program = std::env::current_exe().expect("the test binary knows its own path");
    assert!(
        command.contains(&program.display().to_string()),
        "the hook names some other warlock: {command}",
    );
}

#[test]
fn the_sub_task_session_runs_under_its_own_clock_and_its_own_turn_limit() {
    let agent = ChatAgent::working(WORKING_PROMPT);

    // Not the five minutes a pass gets: that clock is sized for one document,
    // and this session edits files and runs a test suite that is minutes on
    // its own.
    assert_eq!(agent.timeout(), WORKING_TIMEOUT);
    assert_ne!(agent.timeout(), INVOCATION_TIMEOUT);
    assert_ne!(
        WORKING_TIMEOUT.as_secs(),
        300,
        "the sub-task session is running on a pass's five minutes",
    );
    assert!(WORKING_TIMEOUT > INVOCATION_TIMEOUT);

    // The second bound, and a different kind: a session can spin cheaply
    // inside one tool loop for half an hour, and it can also spend its turns
    // in a minute.
    let vector = turn_args(&agent);
    assert_eq!(
        value_of(&vector, "--max-turns"),
        Some(WORKING_TURNS.to_string().as_str()),
    );
    // A bound rather than a formality, and one a retry can still double.
    assert_ne!(WORKING_TURNS, 0);
    assert_ne!(WORKING_TURNS.checked_mul(2), None);
}

#[test]
fn the_sub_task_session_is_its_own_conversation_at_the_briefs_register() {
    let vector = turn_args(&ChatAgent::working(WORKING_PROMPT));

    // Raised the way a drafting turn is and through the same two constants: a
    // session that writes code has less business being cheap than one
    // answering a question.
    assert_eq!(
        value_of(&vector, "--model"),
        overridden(MODEL_VAR, BRIEF_MODEL).to_str(),
    );
    assert_eq!(
        value_of(&vector, "--effort"),
        overridden(EFFORT_VAR, BRIEF_EFFORT).to_str(),
    );

    // Its own id, and the prompt it was handed rather than one of warlock's:
    // what this session is told is the caller's to say, because it names the
    // scope the ticket was pulled under and the sigils this machine holds.
    let session = value_of(&vector, "--session-id").expect("a sub-task opens a conversation");
    assert!(is_uuid_shaped(session), "not UUID-shaped: {session}");
    for other in [
        turn_args(&ChatAgent::new()),
        turn_args(&ChatAgent::drafting()),
        turn_args(&ChatAgent::working(WORKING_PROMPT)),
    ] {
        assert_ne!(value_of(&other, "--session-id"), Some(session));
    }
    assert_eq!(value_of(&vector, "--system-prompt"), Some(WORKING_PROMPT));
}

#[test]
fn the_sub_task_session_is_the_only_one_given_a_writing_tool() {
    // Every session kind that existed before the sub-task one, each read word
    // by word: the grant, the system prompt and every other argument. The
    // sub-task session is the single exception, and it is exempt rather than
    // fixed for the obvious reason — writing is what it is for, and its fence
    // is the hook and the six-tool grant asserted above, not the absence of a
    // tool name from its vector.
    names_no_writing_tool(&args(&ClaudeAgent::new()), "a pass");
    names_no_writing_tool(&turn_args(&ChatAgent::new()), "a panel turn");
    names_no_writing_tool(
        &turn_args(
            &ChatAgent::new()
                .at_model(BRIEF_MODEL)
                .at_effort(BRIEF_EFFORT),
        ),
        "a brief turn",
    );
    names_no_writing_tool(&turn_args(&ChatAgent::drafting()), "a drafting turn");
    names_no_writing_tool(&turn_args(&ChatAgent::proposing()), "a proposing turn");

    // And the exception is a real one rather than a spare sentence: the helper
    // above would refuse the sub-task session's own vector.
    let vector = turn_args(&ChatAgent::working(WORKING_PROMPT));
    for named in ["Edit", "Write", "Bash"] {
        assert!(
            vector.iter().any(|word| word
                .split(|letter: char| !letter.is_ascii_alphanumeric())
                .any(|token| token == named)),
            "{named} is missing from the one session that is meant to have it",
        );
    }
}

// The three things a sub-task session is handed, each spelt so a test can find
// it again in what warlock writes around it. A real sub-task's shape — front
// matter, a goal, a definition of done — because the builders trim and frame
// markdown rather than prose.
const A_SUB_TASK_BRIEF: &str = "---\nsubtask_id: WAR-140.02\nparent: WAR-140\n---\n\n\
                                ## Goal\nRead the queue's own ordering in the panel.\n\n\
                                ## Definition of done\n- [ ] A queue with nothing behind it \
                                draws no header.";

const A_TICKET_TITLE: &str = "Read a scope's ticket queue from Linear";

const A_TICKET_DESCRIPTION: &str = "## Problem\nThe panel offers a scope with no queue behind \
                                    it.\n\n## Out of scope\nThe filing, which is another \
                                    ticket's.";

#[test]
fn the_sub_task_system_prompt_names_the_scope_and_the_sigils_this_machine_holds() {
    let held = ["data-plane".to_owned(), "web".to_owned()];
    let prompt = working_system_prompt("data-plane", &held);

    // The scope the ticket was pulled under, and every sigil this machine
    // holds: the two halves of where this session's boundary is.
    assert!(prompt.contains("scope `data-plane`"), "{prompt}");
    assert!(
        prompt.contains("the sigils `data-plane`, `web`"),
        "{prompt}"
    );

    // One sigil is not a list, and none is a sentence rather than an empty one:
    // a session shown "holds the sigils " with nothing after it reads it as a
    // prompt that was built wrong, and guesses which way.
    assert!(working_system_prompt("web", &held[1..]).contains("the sigil `web`"));
    assert!(working_system_prompt("web", &[]).contains("no sigils at all"));

    // And the sigils are listed rather than judged: which scopes they open is
    // the gate's answer at every write, so this prompt states what is held and
    // that the gate decides — never a second copy of the rule.
    assert!(prompt.contains("warlock gates every edit and every new file"));
}

#[test]
fn the_sub_task_system_prompt_makes_a_refused_write_the_end_of_the_sub_task() {
    let prompt = working_system_prompt("data-plane", &["web".to_owned()]);

    for said in [
        // A write the hook refuses is reported, with the refusal's own words:
        // that sentence is warlock's, names the path and the scope, and is the
        // one thing the operator needs to read.
        "report `blocked`",
        "refusal's own words as the reason",
        // And going around it is refused in advance, by each route there is.
        "Routing around it is not an option",
        "not with a shell command",
        "not by writing somewhere else",
        "not by editing a scope, a sigil or warlock's own configuration",
    ] {
        assert!(prompt.contains(said), "{said:?} is not said: {prompt}");
    }
}

#[test]
fn the_sub_task_system_prompt_leaves_the_history_alone() {
    let prompt = working_system_prompt("data-plane", &["data-plane".to_owned()]);

    // The four in the ticket's own words, so a rewording that drops one fails
    // here rather than in a run that force-pushed a branch.
    assert!(prompt.contains("Do not commit, do not push"), "{prompt}");
    assert!(
        prompt.contains("do not switch, create or delete a branch"),
        "{prompt}",
    );
    assert!(prompt.contains("do not rewrite history"), "{prompt}");

    // Named as the commands a session would actually reach for, because
    // "history" is an abstraction and `git reset --hard` is not.
    for refused in [
        "git commit",
        "git push",
        "git switch",
        "git checkout",
        "git rebase",
        "git reset",
        "git stash",
    ] {
        assert!(
            prompt.contains(refused),
            "`{refused}` is not refused by name: {prompt}",
        );
    }

    // And the shell is not taken away with them: the tests are what say the
    // change holds, and a session told to leave `git` alone has to be told the
    // difference.
    assert!(prompt.contains("run the tests the sub-task asks for"));
}

#[test]
fn the_sub_task_session_runs_under_the_prompt_that_names_its_boundary() {
    let prompt = working_system_prompt("data-plane", &["data-plane".to_owned()]);
    let vector = turn_args(&ChatAgent::working(&prompt));

    // Built at the moment the session is raised and passed through untouched,
    // which is why the constant beside the others is a builder here.
    assert_eq!(value_of(&vector, "--system-prompt"), Some(prompt.as_str()));

    // The exemption above shown to be a real one: this is the single prompt in
    // the file that names a writing tool, because a session holding six tools
    // and told about none of them spends turns asking for what it has.
    for named in ["Edit", "Write", "Bash"] {
        assert!(prompt.contains(named), "{named} is not named: {prompt}");
    }
}

#[test]
fn the_sub_task_opening_carries_the_brief_and_the_ticket_as_context() {
    let opening = working_opening(A_SUB_TASK_BRIEF, A_TICKET_TITLE, A_TICKET_DESCRIPTION, &[]);

    // The sub-task's own brief, whole, and said to be the whole of the work.
    assert!(opening.contains("Read the queue's own ordering in the panel."));
    assert!(opening.contains("draws no header."));
    assert!(opening.contains("the whole of what you are to do"));

    // The ticket, framed rather than merely included: a title and a
    // description that arrive unframed are read as more work to do, and the
    // half of them another sub-task owns gets done twice.
    assert!(opening.contains(A_TICKET_TITLE));
    assert!(opening.contains("The panel offers a scope with no queue behind it."));
    assert!(opening.contains("they are not a to-do list"), "{opening}");
    assert!(opening.contains("belongs to another sub-task or to nobody"));

    // The brief comes first and the ticket after it, so what to do is read
    // before the context it was cut out of.
    assert!(opening.find("## Goal") < opening.find(A_TICKET_TITLE));
}

#[test]
fn the_sub_task_opening_appends_the_engines_contract_rather_than_restating_it() {
    let opening = working_opening(A_SUB_TASK_BRIEF, A_TICKET_TITLE, A_TICKET_DESCRIPTION, &[]);

    // Last words said, and exactly once.
    assert!(opening.ends_with(working::RESULT_PROMPT));
    assert_eq!(opening.matches(working::RESULT_PROMPT).count(), 1);

    // And written down once: with the engine's own text taken away, nothing
    // left in the prompt says what the object looks like. Two copies of a
    // shape is one copy that will disagree with the reader enforcing it.
    let ours = opening.replace(working::RESULT_PROMPT, "");
    for key in ["\"status\"", "\"summary\"", "blocked_reason"] {
        assert!(
            !ours.contains(key),
            "{key} is restated outside the engine's contract: {ours}",
        );
    }
}

#[test]
fn the_sub_task_opening_carries_what_the_finished_siblings_said() {
    let finished = [
        ("WAR-140.01", "Added the issues query and its two fakes."),
        (
            "WAR-140.03",
            "Took a named ticket over the queue's own rules.",
        ),
    ];
    let opening = working_opening(
        A_SUB_TASK_BRIEF,
        A_TICKET_TITLE,
        A_TICKET_DESCRIPTION,
        &finished,
    );

    for (id, summary) in finished {
        assert!(opening.contains(id), "{id} is missing: {opening}");
        assert!(opening.contains(summary), "{summary:?} is missing");
    }

    // In the order they were handed over, each id above its own summary, so a
    // reader can tell which finished sibling said what.
    assert!(opening.find("WAR-140.01") < opening.find("WAR-140.03"));
    assert!(opening.find("WAR-140.01") < opening.find("Added the issues query"));

    // Said to be finished, because a summary of work already in the tree reads
    // as work to do otherwise.
    assert!(opening.contains("already finished"));
    assert!(opening.contains("neither redo nor revise them"));

    // The orchestrator's history is not in it and cannot be: the whole of what
    // this builder is handed is the brief, the ticket and these summaries, so
    // the prompt is those three plus warlock's own framing and nothing else.
    let ours = [A_SUB_TASK_BRIEF, A_TICKET_TITLE, A_TICKET_DESCRIPTION]
        .into_iter()
        .chain(finished.into_iter().flat_map(|(id, summary)| [id, summary]))
        .fold(opening.clone(), |text, part| text.replace(part, ""));
    assert!(!ours.contains("WAR-140"));
}

#[test]
fn a_sub_task_opening_with_no_finished_siblings_has_no_section_for_them() {
    let alone = working_opening(A_SUB_TASK_BRIEF, A_TICKET_TITLE, A_TICKET_DESCRIPTION, &[]);
    let with = working_opening(
        A_SUB_TASK_BRIEF,
        A_TICKET_TITLE,
        A_TICKET_DESCRIPTION,
        &[("WAR-140.01", "Added the issues query.")],
    );

    // Not a heading over nothing: a session shown "sub-tasks that have already
    // finished" followed by silence reads it as siblings that finished and said
    // nothing about what they did.
    assert!(!alone.contains("already finished"));
    assert!(!alone.contains("neither redo nor revise"));

    // And no dangling rule either, which is the part a `contains` would miss:
    // between the ticket's closing rule and the contract there is nothing but
    // the blank line separating them.
    let tail = alone.rsplit_once("---").expect("the ticket block closes").1;
    assert_eq!(tail, format!("\n\n{}", working::RESULT_PROMPT));

    // The two rules the section brings are the only rules it adds.
    assert_eq!(
        alone.matches("---").count() + 2,
        with.matches("---").count()
    );
}

#[test]
fn a_sub_task_retry_runs_in_the_tree_the_failed_attempt_left() {
    let opening = working_opening(A_SUB_TASK_BRIEF, A_TICKET_TITLE, A_TICKET_DESCRIPTION, &[]);
    let again = working_retry(&opening, "  the tests would not build  ");

    // What went wrong last time, trimmed and in the failed attempt's own words.
    assert!(again.contains("the tests would not build"));
    assert!(!again.contains("  the tests"));

    // The one fact a second attempt cannot see for itself: warlock undoes
    // nothing, so this session starts in a tree that may be half changed.
    for said in [
        "working tree",
        "that attempt left",
        "Nothing it wrote has been undone",
        "nothing it wrote has been committed",
        "rather than from the beginning",
    ] {
        assert!(again.contains(said), "{said:?} is not said: {again}");
    }

    // Composed in that order and no other: the notice first, then the opening
    // carried verbatim, so the sub-task and the shape of the object are still
    // the last words said and there is no second paraphrase of either to keep
    // in step.
    assert!(again.starts_with("This sub-task was attempted before"));
    assert!(again.ends_with(&opening));
    assert!(again.ends_with(working::RESULT_PROMPT));
    assert_eq!(again.matches(working::RESULT_PROMPT).count(), 1);
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

// A stand-in that comes back with nothing at all: it keeps what it was asked and
// then fails the way a child that ran out of clock does.
#[derive(Debug, Clone, Default)]
struct Failing {
    sent: Arc<Mutex<Vec<String>>>,
}

impl Failing {
    fn turns(&self) -> usize {
        self.sent.lock().expect("the count is not poisoned").len()
    }
}

impl Wired for Failing {
    fn wired(&self, _cancel: Cancel, _activities: Activities) -> Self {
        self.clone()
    }
}

impl Converses for Failing {
    fn turn(&self, message: &str) -> Result<String, agent::Error> {
        self.sent
            .lock()
            .expect("the count is not poisoned")
            .push(message.to_owned());
        Err(agent::Error::TimedOut {
            after: INVOCATION_TIMEOUT,
        })
    }

    fn raised(&self, _model: &str, _effort: &str) -> Self {
        self.clone()
    }
}

const A_QUESTION: &str = "Is the whetstone the one in the drawer or a new one?";

// The proposing call put to a stand-in, over the same brief and first slice the
// drafting tests use, so what is asserted is the road and not the fixture.
fn proposal_from<C: Converses>(agent: &C) -> Result<String, agent::Error> {
    let block = scope_block_in(TWO_SLICES).expect("two slices");
    let first = &block.slices()[0];
    propose_answer(
        agent,
        block.brief(),
        first.heading(),
        first.prose(),
        A_QUESTION,
    )
}

#[test]
fn a_proposal_is_one_turn_carrying_the_question_and_the_slice_and_nothing_kept_after_it() {
    const PROPOSED: &str = "The one in the drawer: the brief names it and the slice \
                            adds no other.";
    let agent = Scripted::answering([PROPOSED, "A second answer, to a second call."]);

    assert_eq!(proposal_from(&agent).expect("a turn"), PROPOSED);

    let sent = agent.sent();
    assert_eq!(
        sent.len(),
        1,
        "a proposal took more than its turn: {sent:?}"
    );
    // The question, this one slice and the brief above it, which is the whole
    // of what the session is allowed to answer from.
    assert!(sent[0].contains(A_QUESTION));
    assert!(sent[0].contains("Sharpen the knife"));
    assert!(sent[0].contains("the knife is blunt"));
    // And no other slice of the same scope.
    assert!(!sent[0].contains("Sweep the floor"));
    // Not the drafting conversation wearing another hat: none of its terms and
    // no object to fill.
    assert!(!sent[0].contains(DRAFTING_CONTRACT));
    assert!(!sent[0].contains(DRAFTING_ONE_SHOT_CONTRACT));

    // A second call is a second session: it says the same thing over again
    // rather than carrying on from the first, because nothing was kept.
    assert_eq!(
        proposal_from(&agent).expect("a turn"),
        "A second answer, to a second call.",
    );
    let sent = agent.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1], sent[0], "the second call remembered the first");
}

#[test]
fn a_reply_that_settles_nothing_comes_back_as_the_fixed_sentence_and_not_the_models_words() {
    // Every one of these is the session saying it has nothing: the sentence it
    // was asked for, the sentence with a guess bolted onto it, the sentence
    // with whitespace around it, and a reply with nothing in it at all.
    let invented = format!("{NOTHING_SETTLES_IT} I would use the one in `tools/` myself.");
    let replies = [
        NOTHING_SETTLES_IT.to_owned(),
        invented.clone(),
        format!("\n\n{NOTHING_SETTLES_IT}\n"),
        "   \n".to_owned(),
    ];

    for reply in replies {
        let agent = Scripted::answering([reply.clone()]);

        let proposal = proposal_from(&agent).expect("a turn");

        assert_eq!(
            proposal, NOTHING_SETTLES_IT,
            "a reply settling nothing came back as something else: {reply:?}",
        );
        assert_eq!(agent.sent().len(), 1);
    }

    // The guess that rode in with the sentence is gone, rather than handed on
    // for somebody to send to the board as a decision.
    let agent = Scripted::answering([invented]);
    assert!(
        !proposal_from(&agent)
            .expect("a turn")
            .contains("I would use"),
        "the model's own words came back with the fixed sentence",
    );

    // And an answer that does not say it is what it says: the sentence is
    // recognised, not every reply that mentions the brief.
    let agent = Scripted::answering(["The brief settles it: the one in the drawer."]);
    assert_eq!(
        proposal_from(&agent).expect("a turn"),
        "The brief settles it: the one in the drawer.",
    );
}

#[test]
fn a_turn_that_failed_is_the_callers_to_report_and_is_not_taken_again() {
    let agent = Failing::default();

    let error = proposal_from(&agent).expect_err("the stand-in fails every turn");

    match error {
        agent::Error::TimedOut { after } => assert_eq!(after, INVOCATION_TIMEOUT),
        other => panic!("a failed turn came back as something else: {other:?}"),
    }
    assert_eq!(
        agent.turns(),
        1,
        "a failed proposal was tried again rather than reported",
    );
}

#[test]
fn a_proposal_and_a_live_drafting_session_leave_each_other_alone() {
    let drafter = Scripted::answering(["Which whetstone is meant?", ONE_DRAFT]);
    let proposer = Scripted::answering(["The one in the drawer."]);
    let mut session = drafting_with(&drafter);

    let question = asking(session.open().expect("a turn"));
    assert_eq!(question, "Which whetstone is meant?");

    // A whole session of its own, over its own stand-in, while the slice's own
    // conversation sits mid-question.
    assert_eq!(
        proposal_from(&proposer).expect("a turn"),
        "The one in the drawer.",
    );
    assert_eq!(proposer.sent().len(), 1);

    // Nothing the proposal did reached the drafting session: not a turn, not
    // the cancel it runs under, and not the count of rounds it has left.
    assert_eq!(drafter.sent().len(), 1);
    assert!(!session.cancel().is_cancelled());
    assert_eq!(session.questions_left(), DRAFTING_ROUNDS - 1);

    // And the session still answers its next turn, which is the drafts.
    let (fill, _) = drafts(session.answer("the one in the drawer").expect("a turn"));
    assert_eq!(fill.drafts.len(), 1);
    let sent = drafter.sent();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1], "the one in the drawer");
    for message in &sent {
        assert!(
            !message.contains(A_QUESTION) && !message.contains(NOTHING_SETTLES_IT),
            "a proposing turn reached the drafting session: {message}",
        );
    }
    assert_eq!(proposer.sent().len(), 1);
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
fn a_result_line_that_carries_no_answer_says_why_the_run_failed_instead() {
    // The line a run stopped at `--max-turns` ends on: no `result` at all, and
    // the only account of the stopping anywhere — stderr is empty. Without this
    // the turn limit worth retrying and a crashed CLI are the same silence.
    let stopped = stream::read_line(
        r#"{"type":"result","subtype":"error_max_turns","is_error":true,"errors":["Reached maximum number of turns (60)"]}"#,
    );
    assert_eq!(
        stopped.text.as_deref(),
        Some("error_max_turns Reached maximum number of turns (60)")
    );

    // Only when the run says it failed, and only when it left no answer: a line
    // that carries a document carries the document.
    let answered = stream::read_line(
        r#"{"type":"result","subtype":"success","result":"a document","is_error":false}"#,
    );
    assert_eq!(answered.text.as_deref(), Some("a document"));

    let silent = stream::read_line(r#"{"type":"result","subtype":"error_during_execution"}"#);
    assert_eq!(silent.text, None);

    // An error with nothing to say about itself is still nothing to say: an
    // empty string here would be an answer the run never gave.
    let wordless = stream::read_line(r#"{"type":"result","is_error":true}"#);
    assert_eq!(wordless.text, None);
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
// A sub-task session written down before it runs: one entry per attempt, taken
// in the order the attempts come. Its own double rather than [`Scripted`],
// because what this session does with an attempt that *failed* is the whole of
// what is under test and a stand-in that can only succeed says nothing about
// it. Shared through `Arc` for `Scripted`'s reason: the session wires the agent
// it was given before it sends anything, and a copy that recorded into itself
// would record nothing a test could read.
#[derive(Debug)]
enum Attempt {
    /// The session's last message.
    Says(String),
    /// The run exited non-zero with this on its stderr — the text the
    /// classification reads, and in these tests the CLI's own words.
    Failed(String),
    /// The clock ran out and the child was stopped.
    RanLong,
    /// Somebody pressed stop while the attempt was in flight: the handle the
    /// session wired is latched, and the turn ends as `cancelled` makes it end.
    Interrupted,
}

#[derive(Debug, Clone, Default)]
struct Attempting {
    attempts: Arc<Mutex<Vec<Attempt>>>,
    sent: Arc<Mutex<Vec<String>>>,
    bounds: Arc<Mutex<Vec<u32>>>,
    cancels: Arc<Mutex<Vec<Cancel>>>,
}

impl Attempting {
    fn taking(attempts: impl IntoIterator<Item = Attempt>) -> Self {
        Self {
            attempts: Arc::new(Mutex::new(attempts.into_iter().collect())),
            ..Self::default()
        }
    }

    fn sent(&self) -> Vec<String> {
        self.sent
            .lock()
            .expect("the script is not poisoned")
            .clone()
    }

    fn turns(&self) -> usize {
        self.sent.lock().expect("the script is not poisoned").len()
    }

    /// Every turn bound this stand-in was re-let with, in order: what a
    /// turn-limit retry is asked to have done.
    fn bounds(&self) -> Vec<u32> {
        self.bounds
            .lock()
            .expect("the script is not poisoned")
            .clone()
    }

    fn cancelled(&self) -> bool {
        self.cancels
            .lock()
            .expect("the script is not poisoned")
            .iter()
            .any(Cancel::is_cancelled)
    }
}

impl Wired for Attempting {
    fn wired(&self, cancel: Cancel, _activities: Activities) -> Self {
        self.cancels
            .lock()
            .expect("the script is not poisoned")
            .push(cancel);
        self.clone()
    }
}

impl Converses for Attempting {
    fn turn(&self, message: &str) -> Result<String, agent::Error> {
        self.sent
            .lock()
            .expect("the script is not poisoned")
            .push(message.to_owned());
        let mut attempts = self.attempts.lock().expect("the script is not poisoned");
        assert!(
            !attempts.is_empty(),
            "an attempt was made the script has no answer for",
        );
        match attempts.remove(0) {
            Attempt::Says(text) => Ok(text),
            Attempt::Failed(said) => Err(agent::Error::Failed {
                code: Some(1),
                stderr: said,
            }),
            Attempt::RanLong => Err(agent::Error::TimedOut {
                after: WORKING_TIMEOUT,
            }),
            Attempt::Interrupted => {
                // Where a real cancel comes from: a handle somebody else
                // pressed, which is the one this session minted and wired.
                for cancel in self
                    .cancels
                    .lock()
                    .expect("the script is not poisoned")
                    .iter()
                {
                    cancel.cancel();
                }
                Err(agent::Error::Io {
                    source: io::Error::new(io::ErrorKind::Interrupted, "the run was cancelled"),
                })
            }
        }
    }

    fn raised(&self, _model: &str, _effort: &str) -> Self {
        self.clone()
    }
}

impl Bounded for Attempting {
    fn at_turns(&self, turns: u32) -> Self {
        self.bounds
            .lock()
            .expect("the script is not poisoned")
            .push(turns);
        self.clone()
    }
}

const A_SUB_TASK: &str = "## Goal\nSharpen the knife.";
const ITS_TICKET: &str = "The knife is blunt";
const ITS_DESCRIPTION: &str = "Two people have cut themselves sawing with it.";

fn an_opening() -> String {
    working_opening(A_SUB_TASK, ITS_TICKET, ITS_DESCRIPTION, &[])
}

fn a_session(agent: &Attempting) -> Working<Attempting> {
    Working::on(agent, &an_opening())
}

// Named apart from `answered` above, which reads a drafting session's reply:
// two sessions, two vocabularies, and one helper serving both would be a name
// that means something different depending on where it is read.
fn told(worked: &Worked) -> &working::Accepted {
    match worked {
        Worked::Answered(accepted) => accepted,
        Worked::Halted(stopped) => panic!("the session halted rather than answering: {stopped}"),
    }
}

fn stopping(worked: &Worked) -> &Stopped {
    match worked {
        Worked::Halted(stopped) => stopped,
        Worked::Answered(accepted) => panic!("the session answered: {accepted:?}"),
    }
}

// What `claude` actually writes when a run stops for each of the reasons worth
// telling apart — the result line's own subtype for a turn limit, which reaches
// the failure through `judge` because stderr is empty, and the API's error text
// for the rest. Written down here so a test says what the CLI says rather than
// what the classification happens to look for.
const AT_THE_TURN_LIMIT: &str = "error_max_turns Reached maximum number of turns (60)";
const AT_THE_USAGE_LIMIT: &str = "Claude AI usage limit reached|1750000000";
const AT_THE_RATE_LIMIT: &str = r#"API Error: 429 {"type":"error","error":{"type":"rate_limit_error","message":"Number of \
       requests has exceeded your rate limit"}}"#;
const WITH_A_BAD_CREDENTIAL: &str = r#"API Error: 401 {"type":"error","error":{"type":"authentication_error","message":"invalid \
       x-api-key"}}"#;

#[test]
fn a_sub_task_that_finishes_is_one_attempt_and_the_summary_the_session_gave() {
    let agent = Attempting::taking([Attempt::Says(working::stub_answer(
        "Sharpened it on the whetstone and left the drawer as it was.",
    ))]);
    let mut session = a_session(&agent);

    let worked = session.run();

    let accepted = told(&worked);
    assert_eq!(accepted.reported(), &working::Reported::Done);
    assert_eq!(
        accepted.summary(),
        "Sharpened it on the whetstone and left the drawer as it was."
    );
    assert_eq!(session.attempts(), 1);
    assert_eq!(
        agent.sent(),
        vec![an_opening()],
        "the first attempt is the opening, verbatim and alone",
    );
}

#[test]
fn a_sub_task_reported_blocked_is_the_end_of_it_rather_than_something_to_retry() {
    let refusal = "`warlock check --gate` refused the write to `docs/brief.md`";
    let agent = Attempting::taking([
        Attempt::Says(working::stub_reply(
            "blocked",
            "Everything but the document; the gate refused that file.",
            Some(refusal),
        )),
        // Never reached: a block is an answer, and warlock arguing with the
        // one party that was in the tree is not a retry.
        Attempt::Says(working::stub_answer("did it anyway")),
    ]);
    let mut session = a_session(&agent);

    let worked = session.run();

    assert_eq!(
        told(&worked).reported(),
        &working::Reported::Blocked(refusal.to_owned())
    );
    assert_eq!(session.attempts(), 1);
    assert_eq!(agent.turns(), 1);
}

#[test]
fn a_failed_attempt_is_taken_again_with_the_retry_prompt_and_can_finish() {
    let reason = "the test suite would not build";
    let agent = Attempting::taking([
        Attempt::Says(working::stub_reply(
            "failed",
            "Got half of it in.",
            Some(reason),
        )),
        Attempt::Says(working::stub_answer("Fixed the build and finished it.")),
    ]);
    let mut session = a_session(&agent);

    let worked = session.run();

    assert_eq!(told(&worked).reported(), &working::Reported::Done);
    assert_eq!(session.attempts(), 2);

    let sent = agent.sent();
    assert_eq!(sent[0], an_opening());
    assert_eq!(
        sent[1],
        working_retry(&an_opening(), reason),
        "the second attempt is the retry prompt over the same opening",
    );
    assert!(sent[1].contains(reason), "{}", sent[1]);
}

#[test]
fn a_sub_task_that_keeps_failing_is_given_up_after_the_attempts_it_is_allowed() {
    let failing = || {
        Attempt::Says(working::stub_reply(
            "failed",
            "Could not get the tests to pass.",
            Some("the same three tests fail"),
        ))
    };
    let agent = Attempting::taking([
        failing(),
        failing(),
        failing(),
        // The attempt that must not happen: three is the whole allowance.
        Attempt::Says(working::stub_answer("a fourth attempt nobody allowed")),
    ]);
    let mut session = a_session(&agent);

    let worked = session.run();

    assert_eq!(
        told(&worked).reported(),
        &working::Reported::Failed("the same three tests fail".to_owned())
    );
    assert_eq!(session.attempts(), WORKING_ATTEMPTS);
    assert_eq!(agent.turns(), WORKING_ATTEMPTS);
}

#[test]
fn an_answer_that_is_not_the_object_is_a_failure_that_keeps_what_was_said() {
    let prose = "I sharpened the knife. It took a while but it is sharp now.";
    let agent = Attempting::taking([
        Attempt::Says(prose.to_owned()),
        Attempt::Says(prose.to_owned()),
        Attempt::Says(prose.to_owned()),
    ]);
    let mut session = a_session(&agent);

    let worked = session.run();

    let accepted = told(&worked);
    assert!(
        matches!(accepted.reported(), working::Reported::Failed(_)),
        "{accepted:?}"
    );
    assert_eq!(accepted.unreadable(), Some(&working::Unreadable::NoObject));
    assert_eq!(
        accepted.reply(),
        prose,
        "the session's own last message is the only account of the attempt there is",
    );
    // Unreadable is a failure, so it is retried like one — and gives up where
    // one does.
    assert_eq!(session.attempts(), WORKING_ATTEMPTS);
}

#[test]
fn an_object_that_will_not_parse_is_the_same_failure_and_is_kept_too() {
    let malformed = r#"{"status": "done", "summary": "Sharpened it",}"#;
    let agent = Attempting::taking([
        Attempt::Says(malformed.to_owned()),
        Attempt::Says(working::stub_answer("Sharpened it, and said so properly.")),
    ]);
    let mut session = a_session(&agent);

    let worked = session.run();

    assert_eq!(told(&worked).reported(), &working::Reported::Done);
    assert_eq!(session.attempts(), 2);
    assert!(
        agent.sent()[1].contains("could not be read"),
        "the retry is told what was wrong with the answer: {}",
        agent.sent()[1],
    );
}

#[test]
fn a_turn_limit_is_taken_again_on_twice_the_turns() {
    let agent = Attempting::taking([
        Attempt::Failed(AT_THE_TURN_LIMIT.to_owned()),
        Attempt::Says(working::stub_answer("Finished it with the room to do it.")),
    ]);
    let mut session = a_session(&agent);

    let worked = session.run();

    assert_eq!(told(&worked).reported(), &working::Reported::Done);
    assert_eq!(session.attempts(), 2);
    assert_eq!(
        agent.bounds(),
        vec![WORKING_TURNS * 2],
        "the retry after a turn limit is the one that runs on different terms",
    );
    assert!(
        agent.sent()[1].contains("turn limit"),
        "the retry is told why the last attempt stopped: {}",
        agent.sent()[1],
    );
}

#[test]
fn a_second_turn_limit_doubles_again_and_the_third_is_the_end_of_it() {
    let agent = Attempting::taking([
        Attempt::Failed(AT_THE_TURN_LIMIT.to_owned()),
        Attempt::Failed(AT_THE_TURN_LIMIT.to_owned()),
        Attempt::Failed(AT_THE_TURN_LIMIT.to_owned()),
    ]);
    let mut session = a_session(&agent);

    let worked = session.run();

    assert_eq!(stopping(&worked), &Stopped::TurnLimit);
    assert_eq!(session.attempts(), WORKING_ATTEMPTS);
    assert_eq!(agent.bounds(), vec![WORKING_TURNS * 2, WORKING_TURNS * 4]);
}

#[test]
fn what_the_account_or_the_credential_refuses_is_named_and_not_taken_again() {
    let refusals = [
        (AT_THE_USAGE_LIMIT, Stopped::UsageLimit),
        (AT_THE_RATE_LIMIT, Stopped::RateLimit),
        (WITH_A_BAD_CREDENTIAL, Stopped::BadCredential),
    ];

    for (said, expected) in refusals {
        let agent = Attempting::taking([
            Attempt::Failed(said.to_owned()),
            // A second attempt would be a second refusal, and this is what
            // says warlock does not spend one finding that out.
            Attempt::Says(working::stub_answer("an attempt nobody allowed")),
        ]);
        let mut session = a_session(&agent);

        let worked = session.run();

        assert_eq!(stopping(&worked), &expected, "misread: {said}");
        assert_eq!(
            session.attempts(),
            1,
            "retried what cannot be retried: {said}"
        );
        assert_eq!(agent.turns(), 1);
        assert!(agent.bounds().is_empty());
    }
}

#[test]
fn a_failure_the_list_does_not_name_keeps_what_the_run_said_and_is_not_retried() {
    let agent = Attempting::taking([
        Attempt::Failed("Segmentation fault".to_owned()),
        Attempt::Says(working::stub_answer("an attempt nobody allowed")),
    ]);
    let mut session = a_session(&agent);

    let worked = session.run();

    let Stopped::Broke(said) = stopping(&worked) else {
        panic!("a crash was read as something warlock has a plan for: {worked:?}");
    };
    assert!(said.contains("Segmentation fault"), "{said}");
    assert_eq!(agent.turns(), 1);
}

#[test]
fn an_attempt_that_runs_out_of_clock_is_the_end_of_the_sub_task() {
    let agent = Attempting::taking([
        Attempt::RanLong,
        Attempt::Says(working::stub_answer("an attempt nobody allowed")),
    ]);
    let mut session = a_session(&agent);

    let worked = session.run();

    // Not retried: the clock is warlock's own bound, and half an hour that ran
    // out is not half an hour that would have been enough twice.
    assert_eq!(stopping(&worked), &Stopped::TimedOut);
    assert_eq!(agent.turns(), 1);
}

#[test]
fn a_cancelled_attempt_is_not_taken_again_and_is_not_blamed_on_the_session() {
    let agent = Attempting::taking([
        Attempt::Interrupted,
        Attempt::Says(working::stub_answer("an attempt nobody asked for")),
    ]);
    let mut session = a_session(&agent);

    let worked = session.run();

    assert_eq!(stopping(&worked), &Stopped::Cancelled);
    assert_eq!(agent.turns(), 1);
    assert!(agent.cancelled());
}

#[test]
fn the_handle_a_sub_task_session_hands_out_reaches_the_agent_it_runs() {
    let agent = Attempting::taking([Attempt::Says(working::stub_answer("nothing to do"))]);
    let session = a_session(&agent);

    session.cancel().cancel();

    assert!(
        agent.cancelled(),
        "the session's handle reached some other copy of the agent",
    );
}

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

    // The one session that may change the tree, run through the plumbing at the
    // top of the module rather than against a stand-in in memory: a small
    // program in `claude`'s place that keeps its argv, its environment and its
    // stdin, and answers with a result line the way the real one does.
    //
    // A child module for `turns`'s reason — the helpers above are reusable and
    // a second copy of them would drift.
    mod sub_task {
        use std::collections::HashMap;
        use std::io::Write as _;
        use std::process::{Command, Stdio};
        use std::{env, fs};

        use warlock_engine::working;

        use super::super::{value_of, words};
        use super::{clean_up, scratch};
        use crate::{
            ChatAgent, Stopped, WORKING_ATTEMPTS, WORKING_TURNS, Worked, Working, working_opening,
            working_system_prompt,
        };

        // Set by the shell that runs the stand-in, and so present in the
        // child's environment without anybody having put them there.
        const THE_SHELLS_OWN: [&str; 4] = ["_", "PWD", "OLDPWD", "SHLVL"];

        // Nothing a secret is ever spelt with should be anywhere in the vector.
        // Deliberately not the bare word `key`, which a path or a prompt can
        // hold innocently: these are the shapes a credential arrives in.
        const A_SECRET_LOOKS_LIKE: [&str; 9] = [
            "api_key", "api-key", "apikey", "x-api", "token", "secret", "bearer", "password",
            "sk-ant",
        ];

        #[test]
        fn nothing_of_a_key_reaches_the_child_that_works_a_sub_task() {
            let directory = scratch("sub-task-child");
            let (argv, environment, stdin) = (
                directory.join("argv"),
                directory.join("environment"),
                directory.join("stdin"),
            );
            let answered = working::stub_answer("Recorded what it was handed.");
            let result = serde_json::json!({
                "type": "result",
                "subtype": "success",
                "result": answered,
            })
            .to_string();
            // NUL-separated on both sides, so an argument or a value holding a
            // newline — the system prompt holds several — is still read back as
            // the one word it was.
            let stand_in = directory.join("recorder");
            written(
                &stand_in,
                &format!(
                    "#!/bin/sh\n\
                     for arg in \"$@\"; do printf '%s\\0' \"$arg\"; done > '{argv}'\n\
                     env -0 > '{environment}'\n\
                     cat > '{stdin}'\n\
                     printf '%s\\n' '{result}'\n",
                    argv = argv.display(),
                    environment = environment.display(),
                    stdin = stdin.display(),
                ),
            );

            let agent = ChatAgent::working(&working_system_prompt(
                "crates/warlock-tui/**",
                &["warlock-tui".to_owned()],
            ))
            .with_program(&stand_in);
            // Read before the run: the first child to spawn claims the session,
            // and the flag naming it flips from `--session-id` to `--resume`.
            let built = words(&agent.args());
            let opening = working_opening(
                "## Goal\nSharpen the knife.",
                "The knife is blunt",
                "Two people have cut themselves sawing with it.",
                &[],
            );

            let worked = Working::on(&agent, &opening).run();

            // The plumbing worked end to end: the prompt went down stdin on the
            // writer thread, the stream came back through the readers, and the
            // result was read as the contract's object.
            let Worked::Answered(accepted) = &worked else {
                panic!("the stand-in's result line did not come back as an answer: {worked:?}");
            };
            assert_eq!(accepted.reported(), &working::Reported::Done);
            assert_eq!(
                fs::read_to_string(&stdin).expect("the stand-in kept its stdin"),
                opening,
                "something other than the opening reached the child",
            );

            // The whole vector, word for word: what the child was given is what
            // warlock built and nothing else was appended on the way.
            let handed = nul_separated(&argv);
            assert_eq!(handed, built);
            for word in &handed {
                let word = word.to_lowercase();
                for shape in A_SECRET_LOOKS_LIKE {
                    assert!(
                        !word.contains(shape),
                        "`{shape}` in an argument of the sub-task session: {word}",
                    );
                }
            }

            // And the whole environment. Every variable the child has is one
            // this process already had, with the same value: warlock sets
            // nothing on the child, so there is nowhere for a key it holds to
            // travel. What the operator's own shell exports is the operator's
            // business and travels into every child they run.
            let mine: HashMap<String, String> = env::vars().collect();
            let held = nul_separated(&environment);
            for (name, value) in held.iter().map(String::as_str).filter_map(split) {
                if THE_SHELLS_OWN.contains(&name.as_str()) {
                    continue;
                }
                assert_eq!(
                    mine.get(&name),
                    Some(&value),
                    "`{name}` was put in the child's environment by warlock",
                );
            }

            clean_up(&directory);
        }

        #[test]
        fn a_real_turn_limit_is_read_off_the_stream_and_retried_on_twice_the_turns() {
            // The whole road from what `claude` actually does at `--max-turns`
            // to the flag the retry is spawned with: the CLI exits non-zero,
            // says nothing at all on stderr, and puts the stopping on the
            // result line — so this is what says the fallback in `judge`, the
            // reading in `stream::failure` and the classification are one
            // working path and not three plausible ones.
            let directory = scratch("sub-task-turn-limit");
            let argv = directory.join("argv");
            let at_the_limit = serde_json::json!({
                "type": "result",
                "subtype": "error_max_turns",
                "is_error": true,
                "errors": ["Reached maximum number of turns (60)"],
            })
            .to_string();
            let stand_in = directory.join("out-of-turns");
            written(
                &stand_in,
                &format!(
                    "#!/bin/sh\n\
                     for arg in \"$@\"; do printf '%s\\0' \"$arg\"; done > '{argv}'\n\
                     cat > /dev/null\n\
                     printf '%s\\n' '{at_the_limit}'\n\
                     exit 1\n",
                    argv = argv.display(),
                ),
            );

            let agent = ChatAgent::working("sharpen the knife").with_program(&stand_in);
            let opening = working_opening("## Goal\nSharpen it.", "Blunt", "It is blunt.", &[]);

            let mut session = Working::on(&agent, &opening);
            let worked = session.run();

            assert_eq!(worked, Worked::Halted(Stopped::TurnLimit));
            assert_eq!(session.attempts(), WORKING_ATTEMPTS);
            // The last attempt's own vector: the doubling is a flag the child
            // was spawned with, not a number warlock kept to itself.
            let last = nul_separated(&argv);
            assert_eq!(
                value_of(&last, "--max-turns"),
                Some((WORKING_TURNS * 4).to_string().as_str()),
            );

            clean_up(&directory);
        }

        /// Write the stand-in through a child of our own rather than with
        /// [`fs::write`], and make it runnable there too.
        ///
        /// Not fussiness: a whole test suite is running on other threads of
        /// this process, and on Linux a program cannot be `exec`ed while any
        /// process holds a writable handle on it. A file this process writes
        /// itself is inherited by whatever child another thread happens to
        /// spawn in that instant, and the run comes back `ETXTBSY` — "Text file
        /// busy" — on that thread's timing and nobody else's. The handle here
        /// belongs to a child that has already exited by the time the stand-in
        /// is spawned, so there is no window to lose.
        fn written(path: &std::path::Path, body: &str) {
            let mut child = Command::new("/bin/sh")
                .arg("-c")
                .arg(format!(
                    "cat > '{path}' && chmod 755 '{path}'",
                    path = path.display()
                ))
                .stdin(Stdio::piped())
                .spawn()
                .expect("a shell to write the stand-in with");
            child
                .stdin
                .take()
                .expect("stdin was piped")
                .write_all(body.as_bytes())
                .expect("the stand-in is written");
            let status = child.wait().expect("the writing shell is reaped");
            assert!(status.success(), "the stand-in was not written: {status}");
        }

        // Everything but the empty tail a trailing separator leaves. Empties in
        // the middle are kept: `--setting-sources ""` is an argument warlock
        // passes deliberately, and a reader that dropped it would be reading a
        // vector the child never got.
        fn nul_separated(path: &std::path::Path) -> Vec<String> {
            let text = fs::read_to_string(path).expect("the stand-in recorded what it was given");
            let mut parts: Vec<String> = text.split('\0').map(str::to_owned).collect();
            if parts.last().is_some_and(String::is_empty) {
                parts.pop();
            }
            parts
        }

        fn split(entry: &str) -> Option<(String, String)> {
            let (name, value) = entry.split_once('=')?;
            Some((name.to_owned(), value.to_owned()))
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

/// The one test in the crate that spawns the real `claude` and spends a model
/// call, which is why it is `#[ignore]`d and run by hand with
/// `cargo test -p warlock-tui -- --ignored --nocapture`. Everything else here
/// stands `claude` in with `/bin/sh`.
///
/// It exists because one thing about the CLI could not be read off its
/// documentation: whether a `PreToolUse` hook handed in on the invocation with
/// `--settings` still loads when `--setting-sources ""` says to load no settings
/// from anywhere. The finding is written up in
/// [the module doc](mod@crate::claude); this is what establishes it, and what
/// would catch the CLI changing its mind.
mod against_the_real_cli {
    use std::fs;
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::super::{kill_and_reap, watch};

    /// Generous, because this is a real model call deciding to use a real tool,
    /// and it is a backstop rather than an expectation: the probe takes well
    /// under a minute when it works.
    const PROBE_TIMEOUT: Duration = Duration::from_mins(3);

    /// What the hook answers with, and what makes the refusal this probe's own
    /// rather than any other gate on the machine.
    const REASON: &str = "warlock hook probe";

    /// Three ways the probe can come out, only one of which is an answer.
    #[derive(Debug)]
    enum Observed {
        /// The hook ran: it was handed the tool call, and it refused it.
        HookFired { payload: String, scratch: String },
        /// The hook did not run: the edit went through untouched.
        EditWentThrough,
        /// The session never reached for `Edit`, so nothing was asked of the
        /// hook and the run says nothing either way.
        NoEditAttempted { reply: String },
    }

    #[test]
    #[ignore = "spawns the real `claude` and spends a model call"]
    fn a_pre_tool_use_hook_given_with_settings_loads_under_no_setting_sources() {
        let observed = probe();

        match observed {
            Observed::HookFired { payload, scratch } => {
                assert!(
                    payload.contains("\"tool_name\":\"Edit\""),
                    "the hook fired, but on something other than `Edit`: {payload}"
                );
                assert_eq!(
                    scratch, BEFORE,
                    "the hook refused the edit and the edit happened anyway"
                );
            }
            Observed::EditWentThrough => panic!(
                "the `--settings` hook did not load under `--setting-sources \"\"`. \
                 The recorded decision holds: the sub-task session keeps the hook and \
                 gives up `--setting-sources`, never the reverse. Drop \
                 `--setting-sources` from the session's arguments and say so in the \
                 module doc."
            ),
            Observed::NoEditAttempted { reply } => panic!(
                "the session never called `Edit`, so the probe establishes nothing. \
                 Run it again; if it keeps happening the prompt or the tool grant has \
                 gone stale. The reply was:\n{reply}"
            ),
        }
    }

    const BEFORE: &str = "before\n";

    /// Run the probe: a session with `Edit`, a hook on `Edit` that records what
    /// it was handed and refuses it, and `--setting-sources ""` alongside.
    ///
    /// Nothing here depends on what the model *says*. The two observables are
    /// files: the payload the hook writes, and whether the scratch file moved.
    fn probe() -> Observed {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let work = directory.path().join("work");
        fs::create_dir(&work).expect("a working directory for the session");
        let scratch = work.join("scratch.txt");
        fs::write(&scratch, BEFORE).expect("the file the session is asked to edit");
        let payload = directory.path().join("payload.json");

        // The hook is a shell line rather than a script on disk: it needs no
        // execute bit, and a command the CLI runs through a shell can both
        // record its stdin and answer on its stdout.
        let denial = serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": REASON,
            }
        })
        .to_string();
        let command = format!(
            "cat > '{payload}'; printf '%s' '{denial}'",
            payload = payload.display()
        );
        // Built with `serde_json` so the escaping of the line above is the
        // library's problem and not a quoting puzzle written out by hand.
        let settings = serde_json::json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Edit",
                    "hooks": [{ "type": "command", "command": command }],
                }]
            }
        })
        .to_string();

        let reply = run(
            &work,
            &[
                "--print",
                "--tools",
                "Read,Edit",
                "--allowedTools",
                "Read",
                "Edit",
                "--setting-sources",
                "",
                "--settings",
                &settings,
                "Use the Edit tool to change the word before to after in \
                 scratch.txt. Do not use Bash.",
            ],
        );

        let after = fs::read_to_string(&scratch).expect("the scratch file is still there");
        match fs::read_to_string(&payload) {
            Ok(payload) => Observed::HookFired {
                payload,
                scratch: after,
            },
            Err(_) if after == BEFORE => Observed::NoEditAttempted { reply },
            Err(_) => Observed::EditWentThrough,
        }
    }

    /// Spawn `claude` and come back with what it printed, killed and reaped if
    /// it outstays [`PROBE_TIMEOUT`].
    ///
    /// The same shape as [`crate::claude`]'s own runs and for the same reasons:
    /// reader threads so a full pipe cannot deadlock the wait, and a polling
    /// waiter over a shared handle so the handle is still there to kill with.
    fn run(work: &std::path::Path, args: &[&str]) -> String {
        let mut child = Command::new("claude")
            .args(args)
            .current_dir(work)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("`claude` is on PATH: this test is opted into on a machine that has it");

        let stdout = child.stdout.take().expect("stdout was asked for");
        let stderr = child.stderr.take().expect("stderr was asked for");
        let readers = [reading(stdout), reading(stderr)];

        let child = Arc::new(Mutex::new(child));
        let (waiter, exits) = watch(&child);
        if exits.recv_timeout(PROBE_TIMEOUT).is_err() {
            kill_and_reap(&child);
        }
        let _ = waiter.join();

        let mut reply = String::new();
        for reader in readers {
            reply.push_str(&reader.join().expect("the reader thread ran"));
        }
        reply
    }

    fn reading<R: Read + Send + 'static>(mut source: R) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = source.read_to_string(&mut text);
            text
        })
    }
}
