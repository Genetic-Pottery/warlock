use std::path::PathBuf;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use warlock_tui::{Error, status_for};

use super::{Cli, Command, ScopeCommand};

// `try_parse_from` wants argv as the process gets it, program name and all,
// so the name is put back here and every test below reads as the words a
// person types.
fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    let typed = std::iter::once("warlock").chain(args.iter().copied());
    Cli::try_parse_from(typed)
}

// Walked one word at a time, so a nested pair — `["scope", "add"]` — is
// reached the way a reader types it. Cloned because `render_long_help` wants
// the command by mutable reference and the tests want several from one
// parser.
fn subcommand(names: &[&str]) -> clap::Command {
    let mut command = Cli::command();
    for name in names {
        command = command
            .find_subcommand(name)
            .unwrap_or_else(|| panic!("warlock has a `{name}` subcommand"))
            .clone();
    }

    command
}

#[test]
fn the_binary_is_named_warlock() {
    assert_eq!(env!("CARGO_BIN_NAME"), "warlock");
}

#[test]
fn the_parser_itself_is_well_formed() {
    // Every misuse of the derive that clap can catch — a duplicate long
    // flag, a subcommand named twice — is a panic here rather than in
    // somebody's terminal.
    Cli::command().debug_assert();
}

#[test]
fn no_arguments_opens_the_tree() {
    assert_eq!(parse(&[]).unwrap().command, None);
}

#[test]
fn open_takes_scopes_or_none_and_close_takes_at_least_one() {
    let names = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(
        parse(&["open"]).unwrap().command,
        Some(Command::Open { scopes: vec![] })
    );
    assert_eq!(
        parse(&["open", "a", "b"]).unwrap().command,
        Some(Command::Open {
            scopes: names(&["a", "b"])
        })
    );
    assert_eq!(
        parse(&["close", "a"]).unwrap().command,
        Some(Command::Close {
            scopes: names(&["a"])
        })
    );
    assert_eq!(parse(&["close"]).unwrap_err().exit_code(), 2);
    assert_eq!(
        parse(&["sigil"]).unwrap_err().kind(),
        ErrorKind::InvalidSubcommand
    );
}

#[test]
fn config_is_gone() {
    let error = parse(&["config"]).unwrap_err();
    assert_eq!(error.exit_code(), 2);
}

#[test]
fn the_words_for_documents_and_their_freshness_are_gone() {
    for word in ["pact", "refresh", "unpact", "stale", "fresh"] {
        let error = parse(&[word, "crates"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidSubcommand, "{word}");
        assert_eq!(error.exit_code(), 2, "{word}");
    }
}

#[test]
fn a_check_takes_the_path_it_is_a_check_of_and_a_json_flag_in_either_order() {
    assert_eq!(
        parse(&["check", "crates/engine"]).unwrap().command,
        Some(Command::Check {
            path: Some(PathBuf::from("crates/engine")),
            json: false,
            gate: false
        })
    );
    for args in [
        ["check", "crates/engine", "--json"],
        ["check", "--json", "crates/engine"],
    ] {
        assert_eq!(
            parse(&args).unwrap().command,
            Some(Command::Check {
                path: Some(PathBuf::from("crates/engine")),
                json: true,
                gate: false
            }),
            "{args:?}"
        );
    }
}

#[test]
fn a_gate_takes_the_path_in_either_order_and_takes_no_json() {
    for args in [
        ["check", "--gate", "crates/engine"],
        ["check", "crates/engine", "--gate"],
    ] {
        assert_eq!(
            parse(&args).unwrap().command,
            Some(Command::Check {
                path: Some(PathBuf::from("crates/engine")),
                json: false,
                gate: true
            }),
            "{args:?}"
        );
    }
    // A gate prints no envelope, so `--json` beside it is refused at the command
    // line rather than ignored: the silence would look like an empty answer to
    // whatever was about to read it.
    let error = parse(&["check", "--gate", "--json", "crates/engine"]).unwrap_err();
    assert!(error.use_stderr());
    assert_eq!(error.exit_code(), 2);
    // And the pathless gate parses, because its path arrives on stdin.
    assert_eq!(
        parse(&["check", "--gate"]).unwrap().command,
        Some(Command::Check {
            path: None,
            json: false,
            gate: true
        })
    );
}

#[test]
fn a_check_with_no_path_is_a_malformed_invocation_rather_than_a_whole_repository_answer() {
    // A check is a walk up from one place, so there is no
    // whole-repository answer for an absence to mean. Clap's refusal, so it
    // is a 2 and not warlock answering about something nobody named. The path
    // being an `Option` to clap is `--gate`'s doing and changes none of this:
    // `required_unless_present` is what keeps these three a 2.
    for args in [
        ["check"].as_slice(),
        ["check", "--json"].as_slice(),
        // And one path, as everywhere else here.
        ["check", "a", "b"].as_slice(),
    ] {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}

#[test]
fn no_flag_on_a_scope_write_gets_past_the_boundary_or_asks_for_an_object() {
    // Two absences, pinned where they are decided. There is no `--force`,
    // `--yes` or any other word that skips the scope check — `warlock
    // open` is the one road past a boundary, and a flag that existed
    // would be a second. And there is no `--json`: a write says what it did
    // as it does it and spends the status on whether it happened.
    let refused: [&[&str]; 6] = [
        &["scope", "add", "crates", "web", "--force"],
        &["scope", "add", "crates", "web", "--yes"],
        &["scope", "add", "crates", "web", "--json"],
        &["scope", "remove", "crates", "--force"],
        &["scope", "remove", "crates", "--json"],
        &["scope", "--force", "remove", "crates"],
    ];

    for args in refused {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}

#[test]
fn the_two_scope_writes_are_a_noun_and_a_verb_rather_than_two_words_run_together() {
    assert_eq!(
        parse(&["scope", "add", "crates/engine", "data-plane"])
            .unwrap()
            .command,
        Some(Command::Scope {
            command: ScopeCommand::Add {
                path: PathBuf::from("crates/engine"),
                scope: "data-plane".to_owned(),
                // Absent is what clap hands over for a tracker nobody named and
                // a flag nobody passed; whether either is legal is the
                // manifest's answer and is asked past the boundary, not here.
                tracker: None,
                team_key: None,
                base_url: None,
                project_key: None,
                review_state: None,
                label: None,
            }
        })
    );
    assert_eq!(
        parse(&["scope", "remove", "crates/engine"])
            .unwrap()
            .command,
        Some(Command::Scope {
            command: ScopeCommand::Remove {
                path: PathBuf::from("crates/engine"),
            }
        })
    );
}

// `warlock scope add crates/engine data-plane` with whichever tracker word and
// record flags a case is about, in the order the fields are declared in, so each
// assertion below reads as the words that change and not as the two positionals
// under them.
fn added(
    tracker: Option<&str>,
    team_key: Option<&str>,
    base_url: Option<&str>,
    project_key: Option<&str>,
    review_state: Option<&str>,
    label: Option<&str>,
) -> Command {
    Command::Scope {
        command: ScopeCommand::Add {
            path: PathBuf::from("crates/engine"),
            scope: "data-plane".to_owned(),
            tracker: tracker.map(str::to_owned),
            team_key: team_key.map(str::to_owned),
            base_url: base_url.map(str::to_owned),
            project_key: project_key.map(str::to_owned),
            review_state: review_state.map(str::to_owned),
            label: label.map(str::to_owned),
        },
    }
}

#[test]
fn the_tracker_and_the_record_flags_reach_the_add_exactly_as_they_were_typed() {
    assert_eq!(
        parse(&[
            "scope",
            "add",
            "crates/engine",
            "data-plane",
            "linear",
            "--team-key",
            "Data Plane",
            "--review-state",
            "In Review",
            "--label",
            "area/data-plane",
        ])
        .unwrap()
        .command,
        Some(added(
            Some("linear"),
            Some("Data Plane"),
            None,
            None,
            Some("In Review"),
            Some("area/data-plane")
        ))
    );
    // Jira's two flags are the same kind of word on the same command, so the
    // other tracker is a third positional and two more values rather than
    // another subcommand.
    assert_eq!(
        parse(&[
            "scope",
            "add",
            "crates/engine",
            "data-plane",
            "jira",
            "--base-url",
            "https://acme.atlassian.net/",
            "--project-key",
            "DP",
            "--review-state",
            "In Review",
            "--label",
            "area/data-plane",
        ])
        .unwrap()
        .command,
        Some(added(
            Some("jira"),
            None,
            Some("https://acme.atlassian.net/"),
            Some("DP"),
            Some("In Review"),
            Some("area/data-plane")
        ))
    );
    // The same invocation with the flags ahead of the positionals, because a
    // person retyping the command from the refusal that named them will put
    // them wherever the cursor was. The tracker is still the third word of the
    // three that are not flags.
    assert_eq!(
        parse(&[
            "scope",
            "add",
            "--label",
            "area/data-plane",
            "--review-state",
            "In Review",
            "--team-key",
            "Data Plane",
            "crates/engine",
            "data-plane",
            "linear",
        ])
        .unwrap()
        .command,
        Some(added(
            Some("linear"),
            Some("Data Plane"),
            None,
            None,
            Some("In Review"),
            Some("area/data-plane")
        ))
    );
}

#[test]
fn clap_judges_none_of_the_tracker_and_the_record_flags_and_requires_none_of_them() {
    // Nothing is trimmed and nothing is judged here, for the scope
    // positional's reason: a team, a site, a review state and a label belong to
    // somebody else's tracker, and a word that is no tracker at all — like
    // blank — is warlock's refusal to word, past the boundary, with the file
    // untouched.
    assert_eq!(
        parse(&[
            "scope",
            "add",
            "crates/engine",
            "data-plane",
            "Linear",
            "--team-key",
            "  ",
            "--review-state",
            "",
            "--label",
            " area/data-plane ",
        ])
        .unwrap()
        .command,
        Some(added(
            Some("Linear"),
            Some("  "),
            None,
            None,
            Some(""),
            Some(" area/data-plane ")
        ))
    );
    // And a subset parses, the tracker included, because whether any of them is
    // required depends on what `.warlock/scopes.toml` already records and clap
    // has not read it. A `required = true` here would refuse the two-argument
    // run that writes an already-recorded scope.
    assert_eq!(
        parse(&[
            "scope",
            "add",
            "crates/engine",
            "data-plane",
            "--team-key",
            "Data Plane",
        ])
        .unwrap()
        .command,
        Some(added(None, Some("Data Plane"), None, None, None, None))
    );
    assert_eq!(
        parse(&["scope", "add", "crates/engine", "data-plane", "jira"])
            .unwrap()
            .command,
        Some(added(Some("jira"), None, None, None, None, None))
    );
}

#[test]
fn a_record_flag_wants_a_value_on_an_add_and_buys_no_other_word() {
    // Each of the five takes a value, so the flag on its own is a value that
    // went missing rather than a switch; the key is asked for by its key's
    // spelling and `--team` is not a word the add has, so a team name typed
    // where a key goes is refused here rather than recorded; a clear records
    // nothing, so none of them is a word `remove` knows; and having passed
    // them buys nothing at the boundary — `--force`, `--yes` and `--json` are
    // refused beside a filled-in record exactly as they are without one.
    let malformed: [&[&str]; 9] = [
        &["scope", "add", "crates", "web", "--team-key"],
        &["scope", "add", "crates", "web", "--base-url"],
        &["scope", "add", "crates", "web", "--project-key"],
        &["scope", "add", "crates", "web", "--review-state"],
        &["scope", "add", "crates", "web", "--label"],
        &["scope", "add", "crates", "web", "--team", "Web"],
        &["scope", "remove", "crates", "--team-key", "WAR"],
        &["scope", "remove", "crates", "--base-url", "https://acme"],
        &["scope", "remove", "crates", "--review-state", "In Review"],
    ];

    for args in malformed {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }

    // The three override words, each beside a filled-in key, read as a loop
    // rather than as three more rows above: what changes between them is one
    // word on the end.
    for word in ["--force", "--yes", "--json"] {
        let error =
            parse(&["scope", "add", "crates", "web", "--team-key", "WEB", word]).unwrap_err();
        assert!(error.use_stderr(), "{word}");
        assert_eq!(error.exit_code(), 2, "{word}");
    }

    // And `--team` in particular is refused for not being a word rather than
    // for anything about the value beside it, so no alias and no hidden
    // spelling is quietly taking a team name.
    let error = parse(&["scope", "add", "crates", "web", "--team", "Web"]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnknownArgument);
}

#[test]
fn the_add_help_asks_for_a_tracker_and_a_team_key_and_says_whose_each_is() {
    // The help a reader meets without opening `linear.rs`, read off the
    // parser: the value is the key Linear's own `teams(filter: { key: ... })`
    // matches, so the prose names it a Linear team key and the placeholder is
    // spelled `TEAM_KEY`. A team name typed here resolves to nothing, and the
    // mistake surfaces at a push against a live workspace rather than at the
    // parse, which is why this surface says which of the two it wants. Each
    // other value is asserted beside it, so the wording of one cannot be spread
    // across the rest — and the two words a tracker may be are in the help,
    // because `TRACKER` alone says nothing about which ones they are.
    let help = subcommand(&["scope", "add"]).render_long_help().to_string();
    for said in [
        "[TRACKER]",
        "Which board a new scope's record files to: linear or jira",
        "The Linear team key a new scope's issues are filed to",
        "--team-key <TEAM_KEY>",
        "The Jira site a new scope's issues are filed to",
        "--base-url <BASE_URL>",
        "The Jira project key a new scope's issues are filed under",
        "--project-key <PROJECT_KEY>",
        "The review state a new scope's issues are routed to",
        "--review-state <REVIEW_STATE>",
        "The label a new scope's issues carry",
        "--label <LABEL>",
    ] {
        assert!(help.contains(said), "{said}: {help}");
    }

    // And neither the bare placeholder nor a word for a name is left anywhere
    // in it, so `TEAM` on its own cannot come back beside the key's spelling.
    for absent in ["<TEAM>", "team name"] {
        assert!(!help.contains(absent), "{absent}: {help}");
    }
}

#[test]
fn a_scope_is_taken_as_it_was_typed_and_judged_by_the_engine_rather_than_by_clap() {
    // Both of these are refusals — one is not a scope, the other is the
    // `Empty` rule — and both are warlock's to word and to spend a 1 on.
    // Clap's job is to hand over the string, so that what a reader typed is
    // what the engine's sentence is about.
    for typed in ["Data Plane", ""] {
        assert_eq!(
            parse(&["scope", "add", "crates", typed]).unwrap().command,
            Some(Command::Scope {
                command: ScopeCommand::Add {
                    path: PathBuf::from("crates"),
                    scope: typed.to_owned(),
                    tracker: None,
                    team_key: None,
                    base_url: None,
                    project_key: None,
                    review_state: None,
                    label: None,
                }
            }),
            "{typed:?}"
        );
    }
}

#[test]
fn a_scope_write_with_a_piece_missing_is_a_malformed_invocation() {
    // A bare `warlock scope` is a noun with nothing done to it, an `add`
    // with one argument is a scope that went missing rather than a clear —
    // clearing is `scope remove` — and a fourth argument is somebody
    // expecting something warlock does not do: the add's positionals are the
    // path, the scope and the tracker, and there is no fourth for a word to
    // land in. All of them are clap's 2.
    let malformed: [&[&str]; 6] = [
        &["scope"],
        &["scope", "add"],
        &["scope", "add", "crates"],
        &["scope", "add", "crates", "web", "linear", "extra"],
        &["scope", "remove"],
        &["scope", "remove", "crates", "web"],
    ];

    for args in malformed {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}

#[test]
fn a_brief_is_the_bare_word_and_takes_nothing_else() {
    assert_eq!(parse(&["brief"]).unwrap().command, Some(Command::Brief));

    // A path is what a reader who has confused this with `push` or `draft`
    // types, and a flag is somebody expecting a register to leave or an output
    // to parse — there is neither. Both are clap's 2 rather than an argument
    // quietly ignored.
    let malformed: [&[&str]; 3] = [
        &["brief", "docs/brief.md"],
        &["brief", "--json"],
        &["brief", "--scope", "data-plane"],
    ];
    for args in malformed {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}

fn pushed_brief(dry_run: bool) -> Command {
    Command::Push {
        scope: "data-plane".to_owned(),
        path: PathBuf::from("docs/brief.md"),
        dry_run,
    }
}

#[test]
fn a_push_takes_the_scope_then_the_brief_and_a_dry_run_flag() {
    assert_eq!(
        parse(&["push", "data-plane", "docs/brief.md"])
            .unwrap()
            .command,
        Some(pushed_brief(false))
    );
    for args in [
        ["push", "data-plane", "docs/brief.md", "--dry-run"],
        ["push", "--dry-run", "data-plane", "docs/brief.md"],
    ] {
        assert_eq!(
            parse(&args).unwrap().command,
            Some(pushed_brief(true)),
            "{args:?}"
        );
    }
}

#[test]
fn a_push_short_of_its_scope_or_its_brief_is_a_malformed_invocation() {
    // Clap's 2: a bare `push` files nothing, and a push that named only one of
    // the two is never guessed into the other.
    let malformed: [&[&str]; 6] = [
        &["push"],
        &["push", "--dry-run"],
        &["push", "docs/brief.md"],
        &["push", "data-plane", "a.md", "b.md"],
        &["push", "data-plane", "--scope", "web", "docs/brief.md"],
        &["push", "data-plane", "docs/brief.md", "--json"],
    ];

    for args in malformed {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }

    let command = subcommand(&["push"]);
    for argument in command.get_arguments().filter(|a| !a.is_positional()) {
        let long = argument.get_long().unwrap_or_default();
        assert!(
            ["help", "dry-run"].contains(&long),
            "`push` takes `--{long}`, which is not its one flag"
        );
    }
}

fn drafted(project: Option<&str>, dry_run: bool) -> Command {
    Command::Cut {
        scope: "data-plane".to_owned(),
        project: project.map(str::to_owned),
        dry_run,
    }
}

#[test]
fn a_draft_takes_a_scope_and_optionally_a_project_s_slug() {
    // The scope alone lists the planned projects; the slug names one.
    assert_eq!(
        parse(&["draft", "data-plane"]).unwrap().command,
        Some(drafted(None, false))
    );
    assert_eq!(
        parse(&["draft", "data-plane", "9e41c07a2b13"])
            .unwrap()
            .command,
        Some(drafted(Some("9e41c07a2b13"), false))
    );
    assert_eq!(
        parse(&["draft", "--dry-run", "data-plane", "9e41c07a2b13"])
            .unwrap()
            .command,
        Some(drafted(Some("9e41c07a2b13"), true))
    );
}

#[test]
fn a_draft_with_no_scope_or_a_third_word_is_a_malformed_invocation() {
    let malformed: [&[&str]; 5] = [
        &["draft"],
        &["draft", "--dry-run"],
        &["draft", "data-plane", "9e41c07a2b13", "extra"],
        &["draft", "data-plane", "--scope", "web"],
        &["draft", "data-plane", "--json"],
    ];

    for args in malformed {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }

    let command = subcommand(&["draft"]);
    for argument in command.get_arguments().filter(|a| !a.is_positional()) {
        let long = argument.get_long().unwrap_or_default();
        assert!(
            ["help", "dry-run"].contains(&long),
            "`draft` takes `--{long}`, which is not its one flag"
        );
    }
}

#[test]
fn the_draft_help_names_the_scope_the_slug_and_the_flag() {
    let help = subcommand(&["draft"]).render_long_help().to_string();
    for said in ["SCOPE", "SLUG", "--dry-run"] {
        assert!(help.contains(said), "{said}: {help}");
    }
}

// `warlock pull warlock-team` with whichever of the two flags a case is about,
// so each assertion below reads as the flags rather than as the positional under
// them — the push's and the draft's helper, one verb along.
fn pulled_scope(ticket: Option<&str>, dry_run: bool) -> Command {
    Command::Pull {
        scope: "warlock-team".to_owned(),
        ticket: ticket.map(str::to_owned),
        dry_run,
    }
}

#[test]
fn a_pull_takes_the_scope_whose_queue_it_reads_and_the_two_flags_that_go_with_it() {
    assert_eq!(
        parse(&["pull", "warlock-team"]).unwrap().command,
        Some(pulled_scope(None, false))
    );
    assert_eq!(
        parse(&["pull", "warlock-team", "--dry-run"])
            .unwrap()
            .command,
        Some(pulled_scope(None, true))
    );
    // `--ticket` takes an identifier and reaches warlock exactly as it was typed:
    // what a ticket may be called is the board's, and the queue's rules are what
    // judge it.
    assert_eq!(
        parse(&["pull", "warlock-team", "--ticket", "WAR-140"])
            .unwrap()
            .command,
        Some(pulled_scope(Some("WAR-140"), false))
    );
    // Both flags, in either order and either side of the scope, for the reason the
    // push's are pinned that way: a person retyping the command from a refusal
    // will put them wherever the cursor was.
    for args in [
        ["pull", "warlock-team", "--ticket", "WAR-140", "--dry-run"],
        ["pull", "--dry-run", "--ticket", "WAR-140", "warlock-team"],
    ] {
        assert_eq!(
            parse(&args).unwrap().command,
            Some(pulled_scope(Some("WAR-140"), true)),
            "{args:?}"
        );
    }
}

#[test]
fn a_pull_with_no_scope_or_with_two_is_a_malformed_invocation() {
    // Clap's 2, for the push's reason: a pull is about one scope's queue, and an
    // omitted scope is a command line that was never a request rather than
    // warlock guessing which board's work to start.
    let malformed: [&[&str]; 4] = [
        &["pull"],
        &["pull", "--dry-run"],
        &["pull", "warlock-team", "warlock-docs"],
        &["pull", "--ticket", "WAR-140"],
    ];

    for args in malformed {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}

#[test]
fn a_pull_asks_for_no_object_takes_no_any_and_no_word_beside_its_two_flags() {
    // No `--json`, matching the push and the draft: what a script reads afterwards
    // is the run record under the home directory, which is a file rather than a
    // stream to be caught. No `--any` either — taking somebody else's ticket is a
    // reassignment a human makes on the board.
    let malformed: [&[&str]; 6] = [
        &["pull", "warlock-team", "--json"],
        &["pull", "--json", "warlock-team"],
        &["pull", "warlock-team", "--any"],
        &["pull", "warlock-team", "--ticket"],
        &["pull", "warlock-team", "--dry-run=yes"],
        &["pull", "warlock-team", "--scope", "warlock-docs"],
    ];

    for args in malformed {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }

    // And the absence stated over the parser itself rather than over the spellings
    // above: `--ticket` and `--dry-run` are the only words a pull takes beside its
    // scope and clap's own help.
    let command = subcommand(&["pull"]);
    for argument in command.get_arguments().filter(|a| !a.is_positional()) {
        let long = argument.get_long().unwrap_or_default();
        assert!(
            ["help", "ticket", "dry-run"].contains(&long),
            "`pull` takes `--{long}`, which is none of its two flags"
        );
    }
}

#[test]
fn the_pull_help_names_both_of_its_flags_and_the_scope_it_wants() {
    let help = subcommand(&["pull"]).render_long_help().to_string();
    for said in ["SCOPE", "--ticket <TICKET>", "--dry-run"] {
        assert!(help.contains(said), "{said}: {help}");
    }
}

#[test]
fn the_pull_leaves_the_push_and_the_draft_spelled_exactly_as_they_were() {
    // The third verb sits beside the first two and shares their resolver, so this
    // is the guard against it being wired by editing them.
    assert_eq!(
        parse(&["push", "data-plane", "docs/brief.md"])
            .unwrap()
            .command,
        Some(pushed_brief(false))
    );
    assert_eq!(
        parse(&["draft", "data-plane", "--dry-run"])
            .unwrap()
            .command,
        Some(drafted(None, true))
    );
    for args in [
        ["push", "data-plane", "docs/brief.md", "--json"],
        ["draft", "data-plane", "9e41c07a2b13", "--json"],
    ] {
        assert_eq!(parse(&args).unwrap_err().exit_code(), 2, "{args:?}");
    }
}

#[test]
fn both_spellings_of_help_are_a_help_exit_that_succeeded() {
    // Not an error in the sense that matters: help was asked for, so it
    // goes to stdout and the process exits zero.
    for spelling in ["-h", "--help"] {
        let error = parse(&[spelling]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayHelp, "{spelling}");
        assert_eq!(error.exit_code(), 0, "{spelling}");
        assert!(!error.use_stderr(), "{spelling}");
    }
}

#[test]
fn both_spellings_of_version_print_the_crate_version_and_succeed() {
    for spelling in ["-V", "--version"] {
        let error = parse(&[spelling]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayVersion, "{spelling}");
        assert_eq!(error.exit_code(), 0, "{spelling}");
        assert_eq!(
            error.to_string().trim(),
            format!("warlock {}", env!("CARGO_PKG_VERSION")),
            "{spelling}"
        );
    }
}

#[test]
fn per_subcommand_help_is_a_help_exit_too() {
    for args in [
        ["open", "--help"].as_slice(),
        ["close", "--help"].as_slice(),
        ["check", "--help"].as_slice(),
        // The nested pair, asked for at both depths: `warlock scope --help`
        // is the noun's two verbs, and each verb has a help of its own.
        ["scope", "--help"].as_slice(),
        ["scope", "add", "--help"].as_slice(),
        ["scope", "remove", "--help"].as_slice(),
        // The other nested noun, asked for at both depths for the same reason,
        // and `key add` most of all: it is the one help a person reads before
        // typing a command that takes a credential.
        ["key", "--help"].as_slice(),
        ["key", "add", "--help"].as_slice(),
        ["key", "list", "--help"].as_slice(),
        // The two verbs that reach a board, whose help is the one a person
        // reads before typing a command that sends something.
        ["push", "--help"].as_slice(),
        ["draft", "--help"].as_slice(),
    ] {
        let error = parse(args).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayHelp, "{args:?}");
        assert_eq!(error.exit_code(), 0, "{args:?}");
    }
}

#[test]
fn each_subcommands_help_says_what_that_subcommand_does() {
    // The same `about` and `long_about = None` pair as on `Cli`, pinned one
    // subcommand at a time: without the `about` clap falls back to the doc
    // comment, and `warlock open --help` would answer with the doc comment —
    // the name back, which is not what a reader asked for.
    for (name, said) in [
        ("open", "AI"),
        ("close", "AI"),
        ("check", "scope"),
        ("scope", "scope"),
        ("key", "Linear"),
        // The two that reach a board say which direction they go in, because
        // that is the difference somebody typing one of them is choosing
        // between: a brief filed as a project, a project cut into tickets.
        ("push", "brief"),
        ("draft", "tickets"),
    ] {
        let mut command = Cli::command();
        let help = command
            .find_subcommand_mut(name)
            .unwrap_or_else(|| panic!("no `{name}` subcommand"))
            .render_long_help()
            .to_string();
        assert!(help.contains(said), "{name}: {help}");
        assert!(!help.contains("essays"), "{name}: {help}");
        assert!(help.lines().count() < 20, "{name}: {help}");
        // The doc comment above each variant is the command in backticks
        // and nothing an `about` here writes is, so a backtick in the help
        // is a doc comment clap lifted — the name back rather than what it
        // does.
        assert!(!help.contains('`'), "{name}: {help}");
    }
}

#[test]
fn a_word_warlock_does_not_have_is_refused_rather_than_opening_the_tree() {
    // The whole reason the dispatch exists: `warlock status` used to open
    // the tree, which reads as the typed command having run.
    for word in ["status", "nonsense", ""] {
        let error = parse(&[word]).unwrap_err();
        assert!(error.use_stderr(), "{word}");
        assert_eq!(error.exit_code(), 2, "{word}");
    }
}

#[test]
fn a_trailing_argument_is_refused_and_never_quietly_dropped() {
    // `warlock branch clear extra` typed by somebody who meant something by
    // `extra` must not run a `clear` that silently ignored it. A bare `close`
    // is refused too: it names no scope, and closing nothing is not an answer
    // to anything a person typed it for.
    let refused: [&[&str]; 2] = [&["branch", "clear", "extra"], &["close"]];
    for args in refused {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}

#[test]
fn a_parse_failure_and_a_warlock_failure_do_not_share_an_exit_status() {
    // The split `main` records: clap's refusals are 2, and warlock's own
    // failures are the 1 that `ExitCode::FAILURE` is. Later slices' scope
    // refusals have to be tellable from a typo by the status alone.
    assert_eq!(parse(&["status"]).unwrap_err().exit_code(), 2);
}

#[test]
fn help_prints_a_few_lines_rather_than_this_file() {
    // `long_about = None` is what stands between `warlock --help` and the
    // essays above; without it clap lifts the doc comments wholesale.
    let help = Cli::command().render_long_help().to_string();
    for subcommand in [
        "open", "close", "check", "scope", "key", "branch", "brief", "push", "draft", "pull",
        "resume",
    ] {
        assert!(help.contains(subcommand), "{subcommand}: {help}");
    }
    assert!(!help.contains("panic hook"), "{help}");
    // A row per subcommand plus the usage and options chrome: the ceiling is
    // what stops an `about` becoming a paragraph, so it moves by one when a
    // subcommand is added and never to make room for prose.
    assert!(help.lines().count() < 25, "{help}");
    // Every doc comment on `Cli` and its variants spells the command in
    // backticks, and no `about` above does, so a backtick reaching the help
    // is a doc comment that got lifted into it.
    assert!(!help.contains('`'), "{help}");
}

#[test]
fn the_five_statuses_a_write_can_leave_are_all_different_numbers() {
    // The vocabulary, held together in one place so that a script reading
    // only the status can tell them apart: a write that happened, one
    // warlock could not finish, one refused at the boundary with nothing
    // spent, a command line that was never a request, and a run somebody
    // stopped with Ctrl-C. Each is taken from the thing that really produces
    // it — `status_for` for warlock's own four, clap for the fifth — rather
    // than written down as a number.
    let completed = i32::from(status_for(&Ok(())));
    let could_not = i32::from(status_for(&Err(Error::NoRepository {
        start: PathBuf::from("/nowhere"),
        wanted: "open scopes in",
    })));
    let refused = i32::from(status_for(&Err(Error::ClosedScope {
        path: "crates/engine".to_owned(),
        scope: "data-plane".to_owned(),
    })));
    // The run somebody stopped: neither warlock's inability nor its verdict
    // on anything, and the one number here a shell already has a meaning
    // for — 128 plus SIGINT.
    let cancelled = i32::from(status_for(&Err(Error::Cancelled)));
    // Clap's, from a write invocation rather than a question's, because it
    // is a write's statuses that are being told apart.
    let malformed = parse(&["scope", "add", "crates"])
        .expect_err("a scope write with the scope missing is clap's")
        .exit_code();

    let vocabulary = [completed, could_not, refused, malformed, cancelled];
    assert_eq!(vocabulary, [0, 1, 3, 2, 130]);
    for (first, one) in vocabulary.iter().enumerate() {
        for (second, other) in vocabulary.iter().enumerate() {
            assert!(
                first == second || one != other,
                "two of the outcomes share a status: {vocabulary:?}"
            );
        }
    }
}

#[test]
fn no_argument_the_parser_accepts_gets_a_write_past_the_boundary() {
    // The absence stated over the parser itself rather than over a list of
    // spellings somebody thought of: `--force` is refused in the test above,
    // and this says there is no word at all — however spelled — that a write
    // takes besides its positionals, clap's own `--help`, and the five an
    // `add` writes a `[[scope]]` record from. Those five say where a new
    // scope's issues go; none of them reaches the boundary, which is asked
    // and answered before any of them is read. The one road past it is
    // `warlock open`, and an option here would be a second one.
    for names in [vec!["scope"], vec!["scope", "add"], vec!["scope", "remove"]] {
        let allowed: &[&str] = if names == ["scope", "add"] {
            &[
                "help",
                "team-key",
                "base-url",
                "project-key",
                "review-state",
                "label",
            ]
        } else {
            &["help"]
        };
        let mut command = subcommand(&names);
        // The positionals are the path, and the scope and the tracker on an
        // `add`; every other argument a write accepts has to be one of those.
        for argument in command.get_arguments().filter(|a| !a.is_positional()) {
            let long = argument.get_long().unwrap_or_default();
            assert!(
                allowed.contains(&long),
                "{names:?} takes `--{long}`, which is neither clap's help nor a record flag"
            );
        }

        // And the help a reader is shown offers none of the words an
        // override would be spelled with, in case one arrives later as a
        // subcommand rather than as a flag.
        let help = command.render_long_help().to_string().to_lowercase();
        for word in ["force", "override", "skip", "anyway", "ignore", "sudo"] {
            assert!(!help.contains(word), "{names:?}: {help}");
        }
    }
}

#[test]
fn a_malformed_check_is_clap_s_two() {
    // The status warlock never produces itself: clap exits the process with
    // it before `main` has anything to map, so a script can tell a typo from
    // a refusal by the status alone, without reading a word of either.
    let malformed: [&[&str]; 4] = [
        &["check"],
        &["check", "--json=yes", "here"],
        &["check", "here", "there"],
        &["check", "--nonsense", "here"],
    ];

    for args in malformed {
        let error = parse(args).unwrap_err();
        assert_eq!(error.exit_code(), 2, "{args:?}");
        assert!(error.use_stderr(), "{args:?}");
        // And distinct from the status warlock spends on its own failures,
        // which is the whole reason the split is worth keeping.
        assert_ne!(
            error.exit_code(),
            i32::from(status_for(&Ok(()))),
            "{args:?}"
        );
        assert_ne!(
            error.exit_code(),
            i32::from(status_for(&Err(Error::NoRepository {
                start: PathBuf::from("/nowhere"),
                wanted: "open scopes in",
            }))),
            "{args:?}"
        );
    }
}
