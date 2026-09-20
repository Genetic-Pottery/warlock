use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{fs, io};

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Size;
use ratatui::{Frame, Terminal};
use warlock_engine::{Loaded, Manifest, Node, NodeState, Tree, load_tree, repository_root};
use warlock_tui::{App, Chrome, Focus, QuitConfirm, RecordPrompt, Row, ScopePrompt, tree_height};

use super::{Cli, Command, Error, FOR_CLAUDE_MD, ScopeCommand, Session, status_for};
use crate::chatting::Chat;
use crate::pacting::Pact;
use crate::query::spelled;
use crate::session::{Scope, Watched};
use crate::stubs::{Copying, Passing, Saying};
use crate::terminal::Screen;

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
fn init_and_config_are_the_subcommands() {
    assert_eq!(parse(&["init"]).unwrap().command, Some(Command::Init));
    assert_eq!(parse(&["config"]).unwrap().command, Some(Command::Config));
}

#[test]
fn a_listing_with_no_path_asks_about_the_whole_repository() {
    // `None` and not the working directory: what an omitted path means is
    // `query::list`'s to decide, and it decides on the repository root, so
    // the parser hands over the absence rather than filling it in.
    assert_eq!(
        parse(&["stale"]).unwrap().command,
        Some(Command::Stale {
            path: None,
            json: false
        })
    );
    assert_eq!(
        parse(&["fresh"]).unwrap().command,
        Some(Command::Fresh {
            path: None,
            json: false
        })
    );
}

#[test]
fn a_listing_takes_a_path_and_a_json_flag_in_either_order() {
    assert_eq!(
        parse(&["stale", "crates"]).unwrap().command,
        Some(Command::Stale {
            path: Some(PathBuf::from("crates")),
            json: false
        })
    );
    for args in [["stale", "crates", "--json"], ["stale", "--json", "crates"]] {
        assert_eq!(
            parse(&args).unwrap().command,
            Some(Command::Stale {
                path: Some(PathBuf::from("crates")),
                json: true
            }),
            "{args:?}"
        );
    }
    assert_eq!(
        parse(&["fresh", "--json"]).unwrap().command,
        Some(Command::Fresh {
            path: None,
            json: true
        })
    );
}

#[test]
fn a_listing_answers_about_one_path_rather_than_a_list_of_them() {
    // `warlock stale a b` is somebody expecting one of two things warlock
    // does not do — several paths, or a second flag spelled as a word — and
    // either way an answer about `a` alone would be an answer to a question
    // nobody asked. Clap's refusal, so it is a 2 and not a 1.
    for args in [
        ["stale", "a", "b"].as_slice(),
        ["fresh", "a", "b"].as_slice(),
        ["stale", "--jsonn"].as_slice(),
    ] {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}

#[test]
fn a_check_takes_the_path_it_is_a_check_of_and_a_json_flag_in_either_order() {
    assert_eq!(
        parse(&["check", "crates/engine"]).unwrap().command,
        Some(Command::Check {
            path: PathBuf::from("crates/engine"),
            json: false
        })
    );
    for args in [
        ["check", "crates/engine", "--json"],
        ["check", "--json", "crates/engine"],
    ] {
        assert_eq!(
            parse(&args).unwrap().command,
            Some(Command::Check {
                path: PathBuf::from("crates/engine"),
                json: true
            }),
            "{args:?}"
        );
    }
}

#[test]
fn a_check_with_no_path_is_a_malformed_invocation_rather_than_a_whole_repository_answer() {
    // Unlike the two listings, whose omitted path means the repository
    // root: a check is a walk up from one place, so there is no
    // whole-repository answer for an absence to mean. Clap's refusal, so it
    // is a 2 and not warlock answering about something nobody named.
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
fn an_unpact_takes_the_one_directory_it_un_pacts_and_takes_it_from_the_reader() {
    assert_eq!(
        parse(&["unpact", "crates/engine"]).unwrap().command,
        Some(Command::Unpact {
            path: PathBuf::from("crates/engine")
        })
    );
    // The whole-manifest edit is a path somebody typed, and it is spelled
    // like any other: the parser has no default standing behind it, so the
    // largest edit warlock can make is never the one a missing argument
    // makes by itself.
    assert_eq!(
        parse(&["unpact", "."]).unwrap().command,
        Some(Command::Unpact {
            path: PathBuf::from(".")
        })
    );
}

#[test]
fn an_unpact_with_no_path_or_with_two_is_a_malformed_invocation() {
    // An omitted path is not the repository root here, unlike the two
    // listings: it is clap's 2, for the reason above.
    let malformed: [&[&str]; 3] = [
        &["unpact"],
        &["unpact", "a", "b"],
        &["unpact", "--nonsense"],
    ];

    for args in malformed {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}

#[test]
fn the_two_runs_take_the_one_subtree_they_descend_and_take_it_from_the_reader() {
    assert_eq!(
        parse(&["pact", "crates/engine"]).unwrap().command,
        Some(Command::Pact {
            path: PathBuf::from("crates/engine")
        })
    );
    assert_eq!(
        parse(&["refresh", "crates/engine"]).unwrap().command,
        Some(Command::Refresh {
            path: PathBuf::from("crates/engine")
        })
    );
    // The whole repository, spelled by somebody who meant it. The largest
    // run warlock can start — minutes of passes over every directory there
    // is — is never the one an omitted argument starts by itself.
    assert_eq!(
        parse(&["pact", "."]).unwrap().command,
        Some(Command::Pact {
            path: PathBuf::from(".")
        })
    );
}

#[test]
fn a_run_with_no_path_or_with_two_is_a_malformed_invocation() {
    // Clap's 2, for the un-pact's reason with the money on it: a run that
    // guessed at what to descend would have spent the tokens before anybody
    // could say it guessed wrong.
    let malformed: [&[&str]; 6] = [
        &["pact"],
        &["pact", "a", "b"],
        &["pact", "--nonsense"],
        &["refresh"],
        &["refresh", "a", "b"],
        &["refresh", "--nonsense"],
    ];

    for args in malformed {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}

#[test]
fn no_flag_on_a_write_or_a_run_gets_past_the_boundary_or_asks_for_an_object() {
    // Two absences, pinned where they are decided. There is no `--force`,
    // `--yes` or any other word that skips the scope check — `warlock
    // config` is the one road past a boundary, and a flag that existed
    // would be a second. And there is no `--json`: the three questions
    // answer in objects because something reads their answers, while these
    // five say what they did as they do it and spend the status on whether
    // it happened. The two runs are here for the stronger form of the first
    // reason: a flag past their boundary would spend somebody's tokens
    // rewriting somebody else's documents.
    let refused: [&[&str]; 14] = [
        &["pact", "crates", "--force"],
        &["pact", "--force", "crates"],
        &["pact", "crates", "--json"],
        &["refresh", "crates", "--force"],
        &["refresh", "crates", "--json"],
        &["unpact", "crates", "--force"],
        &["unpact", "--force", "crates"],
        &["unpact", "crates", "--json"],
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
                // Absent is what clap hands over for a flag nobody passed;
                // whether that is legal is the manifest's answer and is asked
                // past the boundary, not here.
                team: None,
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

// `warlock scope add crates/engine data-plane` with whichever of the three
// record flags a case is about, so each assertion below reads as the flags and
// not as the two positionals under them.
fn added(team: Option<&str>, review_state: Option<&str>, label: Option<&str>) -> Command {
    Command::Scope {
        command: ScopeCommand::Add {
            path: PathBuf::from("crates/engine"),
            scope: "data-plane".to_owned(),
            team: team.map(str::to_owned),
            review_state: review_state.map(str::to_owned),
            label: label.map(str::to_owned),
        },
    }
}

#[test]
fn the_three_record_flags_reach_the_add_exactly_as_they_were_typed() {
    assert_eq!(
        parse(&[
            "scope",
            "add",
            "crates/engine",
            "data-plane",
            "--team",
            "Data Plane",
            "--review-state",
            "In Review",
            "--label",
            "area/data-plane",
        ])
        .unwrap()
        .command,
        Some(added(
            Some("Data Plane"),
            Some("In Review"),
            Some("area/data-plane")
        ))
    );
    // The same invocation with the flags ahead of the positionals, because a
    // person retyping the command from the refusal that named them will put
    // them wherever the cursor was.
    assert_eq!(
        parse(&[
            "scope",
            "add",
            "--label",
            "area/data-plane",
            "--review-state",
            "In Review",
            "--team",
            "Data Plane",
            "crates/engine",
            "data-plane",
        ])
        .unwrap()
        .command,
        Some(added(
            Some("Data Plane"),
            Some("In Review"),
            Some("area/data-plane")
        ))
    );
    // Nothing is trimmed and nothing is judged here, for the scope
    // positional's reason: a team, a review state and a label belong to
    // somebody else's tracker, and blank is warlock's refusal to word, past
    // the boundary, with the file untouched.
    assert_eq!(
        parse(&[
            "scope",
            "add",
            "crates/engine",
            "data-plane",
            "--team",
            "  ",
            "--review-state",
            "",
            "--label",
            " area/data-plane ",
        ])
        .unwrap()
        .command,
        Some(added(Some("  "), Some(""), Some(" area/data-plane ")))
    );
    // And a subset parses, because whether the three are required depends on
    // what `.warlock/pacts.toml` already records and clap has not read it. A
    // `required = true` here would refuse the flagless run that writes an
    // already-recorded scope.
    assert_eq!(
        parse(&[
            "scope",
            "add",
            "crates/engine",
            "data-plane",
            "--team",
            "Data Plane",
        ])
        .unwrap()
        .command,
        Some(added(Some("Data Plane"), None, None))
    );
}

#[test]
fn a_record_flag_wants_a_value_on_an_add_and_buys_no_other_word() {
    // Each of the three takes a value, so the flag on its own is a value that
    // went missing rather than a switch; a clear records nothing, so none of
    // them is a word `remove` knows; and having passed them buys nothing at
    // the boundary — `--force`, `--yes` and `--json` are refused beside a
    // filled-in record exactly as they are without one.
    let malformed: [&[&str]; 8] = [
        &["scope", "add", "crates", "web", "--team"],
        &["scope", "add", "crates", "web", "--review-state"],
        &["scope", "add", "crates", "web", "--label"],
        &["scope", "remove", "crates", "--team", "Data Plane"],
        &["scope", "remove", "crates", "--review-state", "In Review"],
        &["scope", "add", "crates", "web", "--team", "Web", "--force"],
        &["scope", "add", "crates", "web", "--team", "Web", "--yes"],
        &["scope", "add", "crates", "web", "--team", "Web", "--json"],
    ];

    for args in malformed {
        let error = parse(args).unwrap_err();
        assert!(error.use_stderr(), "{args:?}");
        assert_eq!(error.exit_code(), 2, "{args:?}");
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
                    team: None,
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
    // clearing is `scope remove` — and a third argument is somebody
    // expecting something warlock does not do. All three are clap's 2.
    let malformed: [&[&str]; 6] = [
        &["scope"],
        &["scope", "add"],
        &["scope", "add", "crates"],
        &["scope", "add", "crates", "web", "extra"],
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
fn per_subcommand_help_is_a_help_exit_too() {
    for args in [
        ["init", "--help"].as_slice(),
        ["config", "--help"].as_slice(),
        ["stale", "--help"].as_slice(),
        ["fresh", "--help"].as_slice(),
        ["check", "--help"].as_slice(),
        ["unpact", "--help"].as_slice(),
        // The two runs: the help for a command that spends minutes and
        // tokens is the one a reader is likeliest to ask for before typing
        // it for real.
        ["pact", "--help"].as_slice(),
        ["refresh", "--help"].as_slice(),
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
    // comment, and `warlock init --help` answers "`warlock init`." — the
    // name back, which is not what a reader asked for.
    for (name, said) in [
        ("init", "CLAUDE.md"),
        ("config", "sigils"),
        ("stale", "stale"),
        ("fresh", "fresh"),
        ("check", "scope"),
        ("unpact", "pact"),
        // The two runs say what they leave behind and which directories
        // they spend a pass on, because that is the difference somebody
        // typing one of them is choosing between.
        ("pact", "WARLOCK.md"),
        ("refresh", "stale"),
        ("scope", "scope"),
        ("key", "Linear"),
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
        // is a doc comment clap lifted — which for `stale` would be the
        // name back rather than what it does.
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
fn version_is_refused_because_warlock_does_not_have_one_yet() {
    // Deliberate, and recorded on `Cli`: no version is declared, so the
    // flag is an unrecognized argument like any other word warlock does not
    // have rather than a half-truth about which warlock this is.
    for spelling in ["--version", "-V"] {
        let error = parse(&[spelling]).unwrap_err();
        assert_ne!(error.kind(), ErrorKind::DisplayVersion, "{spelling}");
        assert_eq!(error.exit_code(), 2, "{spelling}");
    }
}

#[test]
fn a_trailing_argument_is_refused_and_never_quietly_dropped() {
    // `warlock init extra` typed by somebody who meant something by `extra`
    // must not run an `init` that silently ignored it.
    let refused: [&[&str]; 4] = [
        &["init", "extra"],
        &["init", "init", "init"],
        &["config", "extra"],
        // The one somebody will try: the sigils are typed at `config`'s
        // prompt, where the answer that clears them can be explained before
        // it is given, and never as an argument.
        &["config", "data-plane"],
    ];
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
        "init", "config", "stale", "fresh", "check", "unpact", "pact", "refresh", "scope", "key",
        "push",
    ] {
        assert!(help.contains(subcommand), "{subcommand}: {help}");
    }
    assert!(!help.contains("panic hook"), "{help}");
    // A row per subcommand plus the usage and options chrome: the ceiling is
    // what stops an `about` becoming a paragraph, so it moves by one when a
    // subcommand is added and never to make room for prose.
    assert!(help.lines().count() < 22, "{help}");
    // Every doc comment on `Cli` and its variants spells the command in
    // backticks, and no `about` above does, so a backtick reaching the help
    // is a doc comment that got lifted into it.
    assert!(!help.contains('`'), "{help}");
}

#[test]
fn an_answered_question_is_a_zero_whatever_the_answer_was() {
    // The half of the exit contract that carries the verdicts: warlock ran
    // the query and the answer is in the output, so the status says the
    // question was answered and nothing more. The two answers that read
    // most like failures and are not — an empty listing and a scope closed
    // to this machine — are `Ok(())` where they are produced, pinned in
    // `query::tests` and `check::tests` against this same function.
    assert_eq!(status_for(&Ok(())), 0);
}

#[test]
fn a_question_warlock_could_not_answer_is_a_one_and_never_a_two() {
    // The other half: no repository above the working directory, a load
    // that could not colour what it was asked about, and a path with no
    // repository-relative spelling — the last of them built by the very
    // function the three subcommands spell their paths through, so this is
    // the refusal a reader would actually get.
    let refusals = [
        Error::NoRepository {
            start: PathBuf::from("/nowhere"),
            wanted: FOR_CLAUDE_MD,
        },
        Error::Problems {
            first: "`/repo/docs`: `WARLOCK.md` could not be read".to_owned(),
            rest: 2,
        },
        spelled(Path::new("/repo"), Path::new("/elsewhere"))
            .expect_err("a path outside the repository has no manifest form"),
    ];

    for refusal in refusals {
        let said = refusal.to_string();
        assert_eq!(status_for(&Err(refusal)), 1, "{said}");
        // One line, because `main` prints it as one line with a `warlock: `
        // in front of it.
        assert!(!said.contains('\n'), "{said}");
    }
}

#[test]
fn a_boundary_this_machine_does_not_hold_is_a_three_and_nothing_else_is() {
    // The write half of the contract, and the only verdict of warlock's own
    // that is neither a 0 nor a 1: nothing was spent, so it is not a
    // failure, and re-running it will never work, so it is not something to
    // read on stderr and try again. Pinned here beside the other statuses;
    // the three write commands pin their own ends in `edits::tests`.
    let refusal = Error::ClosedScope {
        path: "crates/engine".to_owned(),
        scope: "data-plane".to_owned(),
    };
    assert_eq!(status_for(&Err(refusal)), 3);

    // And it is the refusal's alone. The descendant refusal keeps a 1 by
    // the argument on `status_for`, and so does everything warlock could
    // not do.
    assert_eq!(
        status_for(&Err(Error::ClosedScopeBelow {
            path: ".".to_owned(),
            scopes: vec!["platform".to_owned()],
        })),
        1
    );
    assert_eq!(
        status_for(&Err(Error::NoRepository {
            start: PathBuf::from("/nowhere"),
            wanted: FOR_CLAUDE_MD,
        })),
        1
    );
}

#[test]
fn the_six_statuses_a_write_can_leave_are_all_different_numbers() {
    // The vocabulary, held together in one place so that a script reading
    // only the status can tell them apart: a write that happened, one
    // warlock could not finish, one refused at the boundary with nothing
    // spent, a command line that was never a request, a run that descended a
    // subtree and came back with some of its directories failed, and a run
    // somebody stopped with Ctrl-C. Each is taken from the thing that really
    // produces it — `status_for` for warlock's own five, clap for the sixth
    // — rather than written down as a number.
    let completed = i32::from(status_for(&Ok(())));
    let could_not = i32::from(status_for(&Err(Error::NoRepository {
        start: PathBuf::from("/nowhere"),
        wanted: FOR_CLAUDE_MD,
    })));
    let refused = i32::from(status_for(&Err(Error::ClosedScope {
        path: "crates/engine".to_owned(),
        scope: "data-plane".to_owned(),
    })));
    // The half-worked run: the manifest is saved and the documents that
    // could be written are written, so this is neither the 0 of a run with
    // nothing wrong with it nor the 1 of a warlock that could not do the
    // thing.
    let with_failures = i32::from(status_for(&Err(Error::Failures {
        failed: 3,
        total: 12,
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

    let vocabulary = [
        completed,
        could_not,
        refused,
        malformed,
        with_failures,
        cancelled,
    ];
    assert_eq!(vocabulary, [0, 1, 3, 2, 4, 130]);
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
    // takes besides its positionals, clap's own `--help`, and the three an
    // `add` writes a `[[scope]]` record from. Those three say where a new
    // scope's issues go; none of them reaches the boundary, which is asked
    // and answered before any of them is read. The one road past it is
    // `warlock config`, and an option here would be a second one.
    for names in [
        vec!["unpact"],
        vec!["scope"],
        vec!["scope", "add"],
        vec!["scope", "remove"],
    ] {
        let allowed: &[&str] = if names == ["scope", "add"] {
            &["help", "team", "review-state", "label"]
        } else {
            &["help"]
        };
        let mut command = subcommand(&names);
        // The positionals are the path, and the scope on an `add`; every
        // other argument a write accepts has to be one of those.
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
fn a_malformed_invocation_is_clap_s_two_across_all_three_questions() {
    // The third status, and the one warlock never produces itself: clap
    // exits the process with it before `main` has anything to map. Held
    // here over each of the three so that a script can tell a typo from a
    // refusal by the status alone, without reading a word of either.
    let malformed: [&[&str]; 6] = [
        &["stale", "--nonsense"],
        &["stale", "here", "there"],
        &["fresh", "--json=yes"],
        &["check"],
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
                wanted: FOR_CLAUDE_MD,
            }))),
            "{args:?}"
        );
    }
}

#[derive(Debug)]
struct FakeScreen {
    terminal: Terminal<TestBackend>,
    suspensions: Vec<bool>,
    reported: Vec<bool>,
}

impl FakeScreen {
    fn of(width: u16, height: u16) -> Self {
        Self {
            terminal: Terminal::new(TestBackend::new(width, height))
                .expect("a test backend never fails to start"),
            suspensions: Vec::new(),
            reported: Vec::new(),
        }
    }
}

impl Screen for FakeScreen {
    fn size(&self) -> io::Result<Size> {
        // A `TestBackend` cannot fail, so its error type is `Infallible`
        // and there is nothing here for warlock to handle.
        Ok(self.terminal.size().expect("a test backend never fails"))
    }

    fn draw<F: FnOnce(&mut Frame<'_>)>(&mut self, render: F) -> io::Result<()> {
        self.terminal
            .draw(render)
            .expect("a test backend never fails");
        Ok(())
    }

    fn suspended<T, F: FnOnce() -> T>(&mut self, mouse: bool, body: F) -> io::Result<T> {
        self.suspensions.push(mouse);
        Ok(body())
    }

    fn report_mouse(&mut self, on: bool) -> io::Result<()> {
        self.reported.push(on);
        Ok(())
    }
}

type Driven = Session<FakeScreen, Passing, Saying, Copying>;

fn driving(app: App, scope: Scope, tree: &Tree) -> Driven {
    let watched = Watched::start(&scope, tree);
    let root = scope.repo_root.clone();
    Session {
        app,
        screen: FakeScreen::of(80, 24),
        scope,
        manifest: Manifest::new(),
        pact: Pact::with_agent(Passing::filling()),
        chat: Chat::with_agent(root, Saying::answering(ANSWER)),
        clipboard: Copying::taking(),
        confirm: QuitConfirm::default(),
        prompt: ScopePrompt::default(),
        record: RecordPrompt::default(),
        drag: None,
        said: None,
        document: None,
        mouse_captured: true,
        watched,
    }
}

fn session(rows: Vec<Row>) -> Driven {
    let root = PathBuf::from("/warlock/no/such/repository");
    let scope = Scope {
        chrome: Chrome::of(&root, &root),
        root: root.clone(),
        repo_root: root.clone(),
    };
    // A one-node tree for the watcher to be started over. Nothing is there,
    // so no watcher is granted and `Watching` says why — which is exactly
    // the state a session runs in when the platform refuses one, and costs
    // these tests nothing.
    let tree = Tree::new(Node::new(&root, None::<PathBuf>, NodeState::Unpacted));
    driving(App::from_rows(rows), scope, &tree)
}

fn pressed(driven: &mut Driven, key: KeyEvent) -> bool {
    driven
        .press(key, Instant::now())
        .expect("no key pressed here writes to a terminal")
}

const ANSWER: &str = "The tree, the manifest and the pact.";

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn directory(path: &str) -> Row {
    Row::new(0, path, None, NodeState::Unpacted)
}

const AT_MOST: Duration = Duration::from_secs(5);

fn a_repository() -> tempfile::TempDir {
    let scratch = tempfile::tempdir().expect("a temporary directory");
    for (path, text) in [
        (".git/HEAD", "ref: refs/heads/main\n"),
        ("crates/engine/src/lib.rs", "//! Core engine.\n"),
    ] {
        let at = scratch.path().join(path);
        fs::create_dir_all(at.parent().expect("every path here has a parent"))
            .expect("a scratch directory is writable");
        fs::write(&at, text).expect("a scratch file is writable");
    }
    scratch
}

// `session` over a real scratch repository, built the way `run` builds one
// and, like it, with the tree read first.
fn session_over(root: &Path) -> Driven {
    let Loaded { tree, .. } = load_tree(root).expect("a scratch repository loads");
    let repo_root = repository_root(tree.root_path()).expect("the load found a repository");
    let scope = Scope {
        chrome: Chrome::of(&repo_root, tree.root_path()),
        root: tree.root_path().to_path_buf(),
        repo_root,
    };
    driving(App::from_tree(&tree), scope, &tree)
}

fn rounds_until_settled(driven: &mut Driven) {
    let waited = Instant::now();
    while driven.pact.running() && waited.elapsed() < AT_MOST {
        let size = driven.size().expect("the fake screen has a size");
        driven.draw(size).expect("the fake screen draws");
        driven.keep_up();
    }
    assert!(!driven.pact.running(), "the run never finished");
}

#[test]
fn a_round_tells_the_app_the_size_the_frame_is_being_cut_at() {
    let mut driven = session(vec![directory("/repo/crates")]);
    let size = driven.size().expect("the fake screen has a size");

    driven.draw(size).expect("the fake screen draws");

    assert_eq!(
        driven.app.viewport_height(),
        usize::from(tree_height(size)),
        "the app was told the height this frame gives the tree"
    );
}

#[test]
fn pressing_the_pact_key_descends_the_subtree_and_lands_its_documents() {
    let repo = a_repository();
    let mut driven = session_over(repo.path());

    assert!(pressed(&mut driven, key(KeyCode::Char('p'))));
    assert!(driven.pact.running(), "the press started a run");

    rounds_until_settled(&mut driven);

    assert!(
        repo.path().join("WARLOCK.md").is_file(),
        "the root was never documented"
    );
    assert!(
        repo.path().join("crates/engine/src/WARLOCK.md").is_file(),
        "the descent stopped short of the deepest directory"
    );
    assert_eq!(
        driven.manifest.entries().len(),
        4,
        "every directory the walk produced should have been granted"
    );
}

#[test]
fn a_scope_written_outside_warlock_survives_the_next_run_after_a_reload() {
    // `git pull`, or `warlock scope add` in another terminal: the manifest on
    // disk moves while warlock is up. The reload that follows is what every
    // key after it has to act on, or the next save writes the old copy back
    // over the edit.
    let repo = a_repository();
    let mut driven = session_over(repo.path());
    pressed(&mut driven, key(KeyCode::Char('p')));
    rounds_until_settled(&mut driven);

    let outside = Manifest::load(repo.path()).expect("the run saved a manifest");
    let outside = Manifest::with_entries(outside.entries().iter().map(|entry| {
        if entry.module() == "crates/engine/src" {
            entry.clone().with_scope("data-plane")
        } else {
            entry.clone()
        }
    }));
    outside.save(repo.path()).expect("saves");
    fs::write(
        repo.path().join("crates/engine/src/lib.rs"),
        "//! Core engine, revised.\n",
    )
    .expect("a scratch file is writable");
    crate::session::reload(&mut driven.app, &driven.scope, &mut driven.manifest);

    pressed(&mut driven, key(KeyCode::Char('r')));
    rounds_until_settled(&mut driven);

    let after = Manifest::load(repo.path()).expect("a manifest that reads");
    assert_eq!(
        after
            .entry("crates/engine/src")
            .and_then(warlock_engine::PactEntry::scope),
        Some("data-plane"),
        "the refresh wrote warlock's stale copy back over the outside edit"
    );
}

#[test]
fn a_second_press_of_the_pact_key_takes_the_whole_subtree_back_out() {
    let repo = a_repository();
    let mut driven = session_over(repo.path());
    pressed(&mut driven, key(KeyCode::Char('p')));
    rounds_until_settled(&mut driven);

    pressed(&mut driven, key(KeyCode::Char('p')));
    rounds_until_settled(&mut driven);

    assert_eq!(
        driven.manifest.entries().len(),
        0,
        "un-pacting left entries behind"
    );
    assert!(
        repo.path().join("WARLOCK.md").is_file(),
        "un-pacting deleted a document, which it has never done"
    );
}

#[test]
fn the_quit_key_opens_the_question_rather_than_leaving() {
    let mut driven = session(vec![directory("/repo/crates")]);

    assert!(
        pressed(&mut driven, key(KeyCode::Char('q'))),
        "the session goes on"
    );
    assert_eq!(
        driven.confirm,
        QuitConfirm::open(),
        "and the question is up with No lit"
    );
}

#[test]
fn ctrl_c_leaves_without_asking() {
    let mut driven = session(vec![directory("/repo/crates")]);

    let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!pressed(&mut driven, key), "the session is over");
    assert_eq!(
        driven.confirm,
        QuitConfirm::Closed,
        "and no question was ever asked"
    );
}

#[test]
fn answering_no_puts_the_question_down_and_stays() {
    let mut driven = session(vec![directory("/repo/crates")]);
    pressed(&mut driven, key(KeyCode::Char('q')));

    assert!(
        pressed(&mut driven, key(KeyCode::Enter)),
        "the session goes on"
    );
    assert_eq!(
        driven.confirm,
        QuitConfirm::Closed,
        "and the question is down"
    );
}

#[test]
fn the_focus_key_moves_the_keyboard_on() {
    let mut driven = session(vec![directory("/repo/crates")]);
    let before = driven.app.focus();

    assert!(pressed(&mut driven, key(KeyCode::Tab)));
    assert_ne!(driven.app.focus(), before, "the focus moved");
}

#[test]
fn the_mouse_key_flips_what_the_loop_is_holding() {
    let mut driven = session(vec![directory("/repo/crates")]);

    assert!(pressed(&mut driven, key(KeyCode::Char('m'))));
    assert!(!driven.mouse_captured, "reporting was turned off");

    assert!(pressed(&mut driven, key(KeyCode::Char('m'))));
    assert!(driven.mouse_captured, "and back on again");

    assert_eq!(
        driven.screen.reported,
        [false, true],
        "and the terminal was told each time, through the screen rather \
             than past it"
    );
}

#[test]
fn an_edit_over_a_directory_never_asks_for_the_screen() {
    let mut driven = session(vec![directory("/repo/crates")]);

    assert!(pressed(&mut driven, key(KeyCode::Char('e'))));
    assert!(
        driven.screen.suspensions.is_empty(),
        "a row that is not a file is refused before any child is run"
    );
    assert!(
        driven.app.message().is_some(),
        "and the refusal is said rather than swallowed"
    );
}

#[test]
fn the_scope_prompt_swallows_the_pact_key() {
    let mut driven = session(vec![directory("/repo/crates")]);
    driven.prompt = ScopePrompt::open("crates", "");

    assert!(pressed(&mut driven, key(KeyCode::Char('p'))));
    let field = driven
        .prompt
        .field()
        .expect("the window is still up over the directory it opened on");
    assert_eq!(field.text(), "p", "the key was typed, not pressed");
}

#[test]
fn a_key_the_window_does_not_want_puts_it_down_and_starts_nothing() {
    let mut driven = session(vec![directory("/repo/crates")]);
    driven.prompt = ScopePrompt::open("crates", "");

    assert!(pressed(&mut driven, key(KeyCode::Esc)));
    assert_eq!(
        driven.prompt,
        ScopePrompt::Closed,
        "Esc closes the window rather than quitting warlock"
    );
}

#[test]
fn the_record_window_swallows_the_scope_key_itself() {
    // The one key that could reopen a window over the very scope being
    // recorded, and it does not: with the second window up `s` is a letter in
    // the focused field, like every other binding the loop would otherwise
    // answer.
    let mut driven = session(vec![directory("/repo/crates")]);
    driven.record = RecordPrompt::open("crates", "billing");

    assert!(pressed(&mut driven, key(KeyCode::Char('s'))));
    assert_eq!(
        driven.prompt,
        ScopePrompt::Closed,
        "the first window came back up over the name being recorded"
    );
    let form = driven
        .record
        .form()
        .expect("the window is still up over the scope it opened on");
    assert_eq!(form.focused().text(), "s", "the key was typed, not pressed");
}

#[test]
fn esc_puts_the_record_window_down_and_writes_nothing() {
    let mut driven = session(vec![directory("/repo/crates")]);
    driven.record = RecordPrompt::open("crates", "billing");

    assert!(pressed(&mut driven, key(KeyCode::Esc)));
    assert_eq!(
        driven.record,
        RecordPrompt::Closed,
        "Esc closes the window rather than quitting warlock"
    );
    // The session's manifest is the empty one `driving` starts it on, and an
    // Esc that had written would have replaced it. The repository root these
    // tests run over does not exist, so a write would have failed loudly too.
    assert_eq!(driven.manifest, Manifest::new());
}

mod copying {
    use super::{Copying, directory, session};

    // Not "no clipboard here": what arboard hands over is some other
    // program's complaint, and the footer has one line to say it on.
    const REFUSED: &str = "no clipboard on this session\nnothing was listening";

    #[test]
    fn a_copy_that_lands_says_how_much_went() {
        let mut driven = session(vec![directory("/repo/crates")]);

        driven.copy("crates/engine");

        assert_eq!(
            driven.clipboard.copied(),
            ["crates/engine"],
            "the text never reached the clipboard"
        );
        assert_eq!(driven.app.message(), Some("copied 13 characters"));
    }

    #[test]
    fn one_character_is_counted_in_the_singular() {
        let mut driven = session(vec![directory("/repo/crates")]);

        driven.copy("p");

        assert_eq!(driven.app.message(), Some("copied 1 character"));
    }

    #[test]
    fn characters_are_counted_rather_than_the_bytes_utf_8_spells_them_with() {
        let mut driven = session(vec![directory("/repo/crates")]);

        // Five characters and seven bytes: a count of bytes would tell a
        // reader something about UTF-8 rather than about what they copied.
        let text = "péché";
        assert_ne!(text.len(), text.chars().count(), "this text is all ASCII");
        driven.copy(text);

        assert_eq!(driven.app.message(), Some("copied 5 characters"));
    }

    #[test]
    fn a_clipboard_that_refuses_says_so_on_one_line_and_claims_nothing() {
        let mut driven = session(vec![directory("/repo/crates")]);
        driven.clipboard = Copying::refusing(REFUSED);

        driven.copy("crates/engine");

        let said = driven
            .app
            .message()
            .expect("a copy that did not happen is said rather than swallowed");
        assert!(
            !said.contains('\n'),
            "the footer has one line and this wrapped: {said}"
        );
        assert!(
            said.starts_with("nothing was copied"),
            "the footer claims something happened: {said}"
        );
        assert!(
            !said.contains("character"),
            "a failed copy counted characters onto the clipboard: {said}"
        );
        assert!(
            said.contains("no clipboard on this session"),
            "what the clipboard said was thrown away: {said}"
        );
        assert!(
            driven.clipboard.copied().is_empty(),
            "a refused copy left text on the clipboard anyway"
        );
    }
}

mod forgetting {
    use std::time::Instant;

    use super::super::MESSAGE_LIFETIME;
    use super::{directory, session};

    #[test]
    fn a_message_stays_up_for_its_lifetime_and_is_gone_after_it() {
        let base = Instant::now();
        let mut driven = session(vec![directory("/repo/crates")]);

        driven.copy("crates/engine");
        driven.forget_stale_message(base);

        driven.forget_stale_message(base + MESSAGE_LIFETIME / 2);
        assert_eq!(
            driven.app.message(),
            Some("copied 13 characters"),
            "the footer dropped what it was told before its time"
        );

        driven.forget_stale_message(base + MESSAGE_LIFETIME);
        assert_eq!(
            driven.app.message(),
            None,
            "the footer is still claiming a copy that has passed"
        );
    }

    #[test]
    fn the_same_sentence_said_again_gets_its_own_lifetime() {
        let base = Instant::now();
        let mut driven = session(vec![directory("/repo/crates")]);

        driven.copy("crates/engine");
        driven.forget_stale_message(base);

        // Most of the way through the first saying's life, the same text
        // again, so the footer's line is the same line to the character.
        // Nothing about the words says it is new, which is what the count
        // rather than a comparison of them is for.
        let again = base + MESSAGE_LIFETIME / 2;
        driven.copy("crates/engine");
        driven.forget_stale_message(again);

        driven.forget_stale_message(base + MESSAGE_LIFETIME);
        assert_eq!(
            driven.app.message(),
            Some("copied 13 characters"),
            "the second copy was timed from the first one's saying"
        );

        driven.forget_stale_message(again + MESSAGE_LIFETIME);
        assert_eq!(driven.app.message(), None);
    }

    #[test]
    fn a_footer_with_nothing_on_it_is_left_alone() {
        let base = Instant::now();
        let mut driven = session(vec![directory("/repo/crates")]);

        driven.forget_stale_message(base);
        driven.forget_stale_message(base + MESSAGE_LIFETIME);

        assert_eq!(driven.app.message(), None);
    }
}

mod dragging {
    use ratatui::crossterm::event::{
        KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use ratatui::layout::Size;

    use super::{ANSWER, Copying, Driven, Focus, Instant, directory, key, pressed, session};

    const PRESS: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
    const DRAG: MouseEventKind = MouseEventKind::Drag(MouseButton::Left);
    const RELEASE: MouseEventKind = MouseEventKind::Up(MouseButton::Left);

    // Piece 0 of the conversation these tests drag over, so what a drag
    // across the word `does` copies is `does` — the question as it was
    // asked, not the row the panel drew it on.
    const ASKED: &str = "what does the engine do?";

    const REFUSED: &str = "no clipboard on this session\nnothing was listening";

    // A session with a turn in the conversation, drawn once: a pointer event
    // is read against the frame it landed on, and the frame is what tells
    // the panel its width and height.
    fn conversing(now: Instant) -> (Driven, Size) {
        let mut driven = session(vec![directory("/repo/crates")]);
        driven.app.panel_mut().start_turn(ASKED, now);
        driven.app.panel_mut().answer_turn(ANSWER, now);
        let size = redrawn(&mut driven);
        (driven, size)
    }

    fn redrawn(driven: &mut Driven) -> Size {
        let size = driven.size().expect("the fake screen has a size");
        driven.draw(size).expect("the fake screen draws");
        size
    }

    // Where a word the frame drew is, as screen cells: the column its first
    // character landed on and the row it landed in. Read off the drawn frame
    // rather than worked out from the layout, so these tests point at the
    // cells a reader would point at.
    fn drawn_at(driven: &Driven, word: &str) -> (u16, u16) {
        found_at(driven, word).unwrap_or_else(|| panic!("the frame never drew {word:?}"))
    }

    // The same search, handing back the absence rather than panicking on it,
    // for the tests that turn on a line *not* being on screen yet.
    fn found_at(driven: &Driven, word: &str) -> Option<(u16, u16)> {
        let buffer = driven.screen.terminal.backend().buffer();
        let area = buffer.area;
        (0..area.height).find_map(|row| {
            let line: String = (0..area.width).map(|x| buffer[(x, row)].symbol()).collect();
            let byte = line.find(word)?;
            let column = line[..byte].chars().count();
            let column = u16::try_from(column).expect("a column of the frame");
            Some((column, row))
        })
    }

    fn point(driven: &mut Driven, kind: MouseEventKind, at: (u16, u16), size: Size, now: Instant) {
        let (column, row) = at;
        driven.point(
            MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
            size,
            now,
        );
    }

    #[test]
    fn a_drag_across_the_conversation_copies_what_the_highlight_covers() {
        let now = Instant::now();
        let (mut driven, size) = conversing(now);
        let (column, row) = drawn_at(&driven, "does");

        point(&mut driven, PRESS, (column, row), size, now);
        point(&mut driven, DRAG, (column + 2, row), size, now);
        // The space after the `s`, which is the character the drag stopped
        // on and the end of `does`.
        point(&mut driven, DRAG, (column + 4, row), size, now);
        point(&mut driven, RELEASE, (column + 4, row), size, now);

        assert_eq!(
            driven.clipboard.copied(),
            ["does"],
            "the release copied something other than what the drag covered"
        );
        assert_eq!(driven.app.message(), Some("copied 4 characters"));
        // The one thing on screen saying what went: a footer line over a
        // card with nothing highlighted would leave the reader taking
        // warlock's word for it.
        assert_eq!(
            crate::selected_text(&driven.app),
            "does",
            "the copy took the highlight down with it"
        );
        assert_eq!(
            driven.app.focus(),
            Focus::Panel,
            "the press no longer points the keys at the pane it landed in"
        );
    }

    #[test]
    fn a_drag_whose_copy_is_refused_says_so_and_claims_nothing() {
        let now = Instant::now();
        let (mut driven, size) = conversing(now);
        driven.clipboard = Copying::refusing(REFUSED);
        let (column, row) = drawn_at(&driven, "does");

        point(&mut driven, PRESS, (column, row), size, now);
        point(&mut driven, DRAG, (column + 4, row), size, now);
        point(&mut driven, RELEASE, (column + 4, row), size, now);

        let said = driven
            .app
            .message()
            .expect("a copy that did not happen is said rather than swallowed");
        assert!(
            !said.contains('\n'),
            "the footer has one line and this wrapped: {said}"
        );
        assert!(
            !said.contains("character"),
            "the footer counted characters onto a clipboard that refused: {said}"
        );
        assert!(
            said.contains("no clipboard on this session"),
            "what the clipboard said was thrown away: {said}"
        );
        assert!(
            driven.clipboard.copied().is_empty(),
            "a refused copy left text on the clipboard anyway"
        );
        assert_eq!(
            crate::selected_text(&driven.app),
            "does",
            "the highlight came down over a copy that never happened"
        );
    }

    #[test]
    fn a_press_nobody_dragged_from_copies_nothing_and_says_nothing() {
        let now = Instant::now();
        let (mut driven, size) = conversing(now);
        let (column, row) = drawn_at(&driven, "does");

        point(&mut driven, PRESS, (column, row), size, now);
        point(&mut driven, RELEASE, (column, row), size, now);

        assert!(
            driven.clipboard.copied().is_empty(),
            "a press with no drag after it copied the character under it"
        );
        assert_eq!(
            driven.app.message(),
            None,
            "a copy that never happened was reported anyway"
        );
        assert_eq!(
            crate::selected_text(&driven.app),
            String::new(),
            "a press with no drag after it highlighted text"
        );
        assert_eq!(
            driven.app.focus(),
            Focus::Panel,
            "a press in the panel stopped taking the keys"
        );
    }

    #[test]
    fn a_drag_over_the_tree_copies_nothing_and_selects_its_row() {
        let now = Instant::now();
        let (mut driven, size) = conversing(now);
        let (column, row) = drawn_at(&driven, "crates");

        point(&mut driven, PRESS, (column, row), size, now);
        point(&mut driven, DRAG, (column + 3, row), size, now);
        point(&mut driven, RELEASE, (column + 3, row), size, now);

        assert!(
            driven.clipboard.copied().is_empty(),
            "a drag down the tree column copied something"
        );
        assert_eq!(
            driven.app.message(),
            None,
            "a drag over the tree wrote a line on the footer"
        );
        assert_eq!(
            driven.app.selection(),
            None,
            "a drag over the tree highlighted the conversation"
        );
        assert_eq!(
            driven.app.focus(),
            Focus::Tree,
            "a press on a row stopped pointing the keys at the tree"
        );
    }

    #[test]
    fn a_drag_over_another_card_copies_nothing_and_says_nothing() {
        let now = Instant::now();
        // The size this one drags at is the one the frame with the document
        // on it was drawn at, below.
        let (mut driven, _) = conversing(now);
        // The word is in the document rather than the conversation, so the
        // cells the drag covers are cells of the card that is showing.
        driven.app.show_document(["what a document does"], false);
        assert!(
            !driven.app.panel().showing_thread(),
            "the document card never took the conversation's place"
        );
        let size = redrawn(&mut driven);
        let said_before = driven.app.message().map(str::to_owned);
        let (column, row) = drawn_at(&driven, "does");

        point(&mut driven, PRESS, (column, row), size, now);
        point(&mut driven, DRAG, (column + 4, row), size, now);
        point(&mut driven, RELEASE, (column + 4, row), size, now);

        assert!(
            driven.clipboard.copied().is_empty(),
            "a drag over the document card copied a line of it"
        );
        assert_eq!(
            driven.app.message().map(str::to_owned),
            said_before,
            "a drag over the document card wrote a line on the footer"
        );
        assert_eq!(
            driven.app.selection(),
            None,
            "a drag over the document card highlighted the conversation behind it"
        );
    }

    // Typed into the composer so there is a field on screen to press in, and
    // a word to find it by.
    const DRAFT: &str = "draft";

    // A conversation several screens tall, so there is somewhere for the
    // card to scroll, drawn once at the size the drags below land on.
    fn scrollback(now: Instant) -> (Driven, Size) {
        let mut driven = session(vec![directory("/repo/crates")]);
        for turn in 0..12 {
            let asked = format!("question {turn} about the engine");
            driven.app.panel_mut().start_turn(&asked, now);
            driven.app.panel_mut().answer_turn(ANSWER, now);
        }
        let size = redrawn(&mut driven);
        assert!(
            driven.app.panel().scroll_offset() > 0,
            "the conversation fits on the card: nothing here would scroll"
        );
        (driven, size)
    }

    // The card wound back to its first line, where a drag downwards has the
    // whole conversation below it.
    fn wound_back(driven: &mut Driven) -> Size {
        driven.app.scroll_panel_up(usize::MAX);
        redrawn(driven)
    }

    // A point below every row of the card: the footer, which is inside the
    // screen and outside the panel.
    fn below_the_card(size: Size, column: u16) -> (u16, u16) {
        (column, size.height - 1)
    }

    // A draft typed into the composer, so that the field is drawn and there
    // is somewhere in it to press. Typed rather than set, because the
    // composer only takes letters with the keys pointed at it and that is
    // the state a reader presses in it from.
    fn drafting(driven: &mut Driven) {
        driven.app.set_focus(Focus::Composer);
        for letter in DRAFT.chars() {
            assert!(
                pressed(driven, key(KeyCode::Char(letter))),
                "typing into the composer ended the session"
            );
        }
    }

    // The tick, driven by hand: `run`'s loop calls `drag_scroll` once a
    // round whether or not an event arrived, so a round with nothing in it
    // is `drag_scroll` on its own.
    mod past_the_edge {
        use ratatui::crossterm::event::MouseEventKind;

        use super::{
            DRAFT, DRAG, Driven, Focus, Instant, PRESS, RELEASE, below_the_card, drafting,
            drawn_at, found_at, point, scrollback, wound_back,
        };
        use crate::rows_per_tick;

        // A pointer moved with nothing held down, which is what a reader
        // whose hand is off the button sends as they cross the footer.
        const MOVED: MouseEventKind = MouseEventKind::Moved;

        fn covered(driven: &Driven) -> usize {
            crate::selected_text(&driven.app).chars().count()
        }

        #[test]
        fn one_row_past_the_edge_is_a_row_a_tick_and_far_past_it_is_several() {
            assert_eq!(
                rows_per_tick(1),
                1,
                "the row just past the edge is not the slow, aimable one"
            );
            assert_eq!(rows_per_tick(6), 2, "the middle of the curve moved");
            assert_eq!(
                rows_per_tick(40),
                5,
                "a pointer dragged to the bottom of the terminal is not at the ceiling"
            );
        }

        #[test]
        fn a_drag_held_below_the_card_keeps_scrolling_and_takes_the_highlight_with_it() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, below_the_card(size, column), size, now);
            let anchored = driven.app.panel().scroll_offset();
            assert_eq!(
                covered(&driven),
                0,
                "the drag past the edge highlighted text off its own event"
            );

            driven.drag_scroll();
            let after_one = driven.app.panel().scroll_offset();
            assert!(
                after_one > anchored,
                "the tick left the card where the drag did: {after_one}"
            );
            let after_one_covered = covered(&driven);
            assert!(
                after_one_covered > 0,
                "the card scrolled out from under the highlight"
            );

            // No further event: the pointer is being held still, which is
            // the whole reason this runs off the tick.
            driven.drag_scroll();
            assert!(
                driven.app.panel().scroll_offset() > after_one,
                "the scrolling stopped when the pointer did"
            );
            assert!(
                covered(&driven) > after_one_covered,
                "the highlight stopped growing while the card went on scrolling"
            );
        }

        #[test]
        fn the_copy_on_release_takes_in_what_the_ticks_scrolled_into_view() {
            // A turn far enough down the conversation that the frame the
            // press lands on has not drawn it: nothing but the ticks can
            // bring it inside the highlight, so finding it on the clipboard
            // is the scrolling and the copy proving each other.
            const LATER: &str = "question 11";

            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question 0");
            let past = below_the_card(size, column);
            assert!(
                found_at(&driven, LATER).is_none(),
                "the whole conversation is on screen already: \
                     {LATER} needs no scrolling to reach"
            );

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, past, size, now);
            let anchored = driven.app.panel().scroll_offset();
            for _ in 0..200 {
                driven.drag_scroll();
            }
            assert!(
                driven.app.panel().scroll_offset() > anchored,
                "the ticks left the card where the drag did"
            );
            assert!(covered(&driven) > 0, "the ticks grew no highlight");
            point(&mut driven, RELEASE, past, size, now);

            let [copied] = driven.clipboard.copied() else {
                panic!(
                    "the release past the edge copied something other than once: {:?}",
                    driven.clipboard.copied()
                )
            };
            assert!(
                copied.starts_with("question 0"),
                "the copy began somewhere other than where the press did: {copied:?}"
            );
            assert!(
                copied.contains(LATER),
                "the copy stopped at the edge the drag left rather than at \
                     the line the ticks reached: {copied:?}"
            );
        }

        #[test]
        fn the_scrolling_stops_at_the_end_of_the_thread() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, below_the_card(size, column), size, now);
            for _ in 0..200 {
                driven.drag_scroll();
            }

            assert_eq!(
                driven.app.panel().lines_below(),
                0,
                "the ticks left the card short of the end of the conversation"
            );
            let end = driven.app.panel().scroll_offset();
            let held = covered(&driven);

            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                end,
                "the card scrolled past the last line of the conversation"
            );
            assert_eq!(
                covered(&driven),
                held,
                "the highlight went on growing over a card that had stopped"
            );
        }

        #[test]
        fn a_drag_held_above_the_card_scrolls_the_other_way_and_stops_at_the_top() {
            let now = Instant::now();
            // Left where a conversation sits: at the newest line, with
            // everything else above it.
            let (mut driven, size) = scrollback(now);
            let (column, row) = drawn_at(&driven, "engine");
            let at_the_end = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            // Row zero is the top border of the panes, which is past every
            // row of the card.
            point(&mut driven, DRAG, (column, 0), size, now);
            driven.drag_scroll();

            assert!(
                driven.app.panel().scroll_offset() < at_the_end,
                "the tick scrolled the wrong way for a pointer above the card"
            );
            assert!(
                covered(&driven) > 0,
                "the highlight did not follow the card upwards"
            );

            for _ in 0..200 {
                driven.drag_scroll();
            }
            let top = driven.app.panel().scroll_offset();
            assert_eq!(top, 0, "the ticks stopped short of the first line");

            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                0,
                "the card scrolled above its first line"
            );
        }

        #[test]
        fn the_release_that_ends_the_drag_ends_the_scrolling() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question");
            let past = below_the_card(size, column);

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, past, size, now);
            driven.drag_scroll();
            point(&mut driven, RELEASE, past, size, now);
            let let_go = driven.app.panel().scroll_offset();
            let held = covered(&driven);

            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                let_go,
                "the card went on scrolling after the button came up"
            );
            assert_eq!(
                covered(&driven),
                held,
                "the highlight went on growing after the button came up"
            );
            assert_eq!(
                driven.clipboard.copied().len(),
                1,
                "the release past the edge copied something other than once"
            );
        }

        #[test]
        fn a_round_with_no_button_held_scrolls_nothing() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let where_it_was = driven.app.panel().scroll_offset();

            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                where_it_was,
                "a card nobody is dragging over scrolled by itself"
            );
            assert_eq!(
                driven.app.selection(),
                None,
                "a tick with no drag behind it highlighted something"
            );
        }

        #[test]
        fn a_press_on_the_tree_dragged_past_the_card_scrolls_nothing() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (tree_column, tree_row) = drawn_at(&driven, "crates");
            let (column, _) = drawn_at(&driven, "question");
            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (tree_column, tree_row), size, now);
            point(&mut driven, DRAG, below_the_card(size, column), size, now);
            driven.drag_scroll();
            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                where_it_was,
                "a drag that began in the tree scrolled the conversation"
            );
            assert_eq!(
                driven.app.selection(),
                None,
                "a drag that began in the tree highlighted the conversation"
            );
        }

        #[test]
        fn a_press_on_the_composer_dragged_past_the_card_scrolls_nothing() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            drafting(&mut driven);
            // Pointed away again, so that the press taking the keys back is
            // this test's proof that it landed in the field rather than on a
            // line of the conversation behind it.
            driven.app.set_focus(Focus::Tree);
            let size = wound_back(&mut driven);
            let field = drawn_at(&driven, DRAFT);
            let (column, _) = drawn_at(&driven, "question");
            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, field, size, now);
            assert_eq!(
                driven.app.focus(),
                Focus::Composer,
                "the press landed somewhere other than the composer"
            );
            point(&mut driven, DRAG, below_the_card(size, column), size, now);
            driven.drag_scroll();
            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                where_it_was,
                "a drag that began in the composer scrolled the conversation"
            );
            assert_eq!(
                driven.app.selection(),
                None,
                "a drag that began in the composer highlighted the conversation"
            );
        }

        #[test]
        fn a_pointer_past_the_edge_with_no_button_held_scrolls_nothing() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, _) = drawn_at(&driven, "question");
            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, MOVED, below_the_card(size, column), size, now);
            driven.drag_scroll();
            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                where_it_was,
                "a pointer crossing the footer with nothing held down \
                     scrolled the conversation"
            );
            assert_eq!(
                driven.app.selection(),
                None,
                "a pointer crossing the footer with nothing held down \
                     highlighted the conversation"
            );
        }

        #[test]
        fn a_card_put_up_while_the_button_is_held_is_not_scrolled_by_it() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, below_the_card(size, column), size, now);
            // The one way another card can take the conversation's place
            // without the button coming up first: a key pressed mid-drag.
            driven
                .app
                .show_document(std::iter::repeat_n("a line of the file", 200), false);
            let where_it_was = driven.app.panel().scroll_offset();

            driven.drag_scroll();
            driven.drag_scroll();

            assert_eq!(
                driven.app.panel().scroll_offset(),
                where_it_was,
                "the held drag scrolled the document that replaced the conversation"
            );
        }
    }

    // Following, held off for the length of a drag. A turn is long, so
    // copying an earlier answer while a later one arrives is an ordinary
    // thing to want, and a card that went on pulling itself to the newest
    // line would take the text out from under the pointer mid-gesture.
    mod pausing {
        use warlock_tui::Activity;
        use warlock_tui::panel::Showing;

        use super::{
            DRAFT, DRAG, Driven, Instant, PRESS, RELEASE, below_the_card, drafting, drawn_at,
            found_at, point, redrawn, scrollback, wound_back,
        };

        // A word nothing in the conversation has until the appends below put
        // it there, short enough that the panel cannot wrap it: finding it on
        // the frame is the card having gone to the newest line and nothing
        // else.
        const NEWEST: &str = "ozymandias";

        // Everything a live turn puts into the conversation: the question, a
        // line of activity, one of warlock's own notes, the answer. The
        // question and the note go through `Card::accrue`, which is the one
        // path that sets following; the other two are written into the turn
        // already there and ride the flag it left.
        fn a_turn_arrives(driven: &mut Driven, now: Instant) {
            let panel = driven.app.panel_mut();
            panel.start_turn("question 12 about the engine", now);
            panel.record_turn(&Activity::Thinking, now);
            panel.note("warlock has something to say", now);
            panel.answer_turn(NEWEST, now);
        }

        // The conversation growing under a gesture that is not a drag held
        // over it, which has to move the card to the newest line exactly as
        // it does with no button down anywhere. `before` is where the
        // conversation's window was, read by the caller while the card was
        // showing: some of these gestures put another card in front of it,
        // and a turn arriving brings the conversation back.
        fn the_conversation_still_follows(driven: &mut Driven, before: usize, now: Instant) {
            a_turn_arrives(driven, now);

            let panel = driven.app.panel();
            assert!(
                panel.showing_thread(),
                "a turn arriving left another card in front of the conversation"
            );
            let after = panel.scroll_offset();
            assert!(
                after > before,
                "the conversation stayed where it was: {before} to {after}"
            );
            assert!(
                panel.follows(),
                "the card came out of the gesture not following"
            );
            assert_eq!(
                panel.lines_below(),
                0,
                "the card stopped short of the newest line"
            );
        }

        #[test]
        fn a_held_drag_keeps_the_card_still_while_the_conversation_grows() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            // An earlier part of the conversation, with everything the turn
            // is about to say far below it.
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question 0");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 8, row), size, now);
            let offset = driven.app.panel().scroll_offset();
            let rows = driven.app.panel().window(now);
            let covered = crate::selected_text(&driven.app);
            assert!(
                !covered.is_empty(),
                "the drag covered nothing: there is no highlight here to disturb"
            );

            a_turn_arrives(&mut driven, now);

            assert_eq!(
                driven.app.panel().scroll_offset(),
                offset,
                "the arriving turn pulled the held card to the newest line"
            );
            assert_eq!(
                driven.app.panel().window(now),
                rows,
                "the rows under the pointer changed while the button was held"
            );
            assert_eq!(
                crate::selected_text(&driven.app),
                covered,
                "what the highlight covers changed under the held drag"
            );
            redrawn(&mut driven);
            assert!(
                found_at(&driven, NEWEST).is_none(),
                "the frame drew the newest line over a drag held on an earlier one"
            );
        }

        #[test]
        fn the_release_hands_the_newest_line_back_mid_turn() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            // A question out and unanswered, which is the state a reader
            // copies an earlier answer in. The card is following it, so the
            // drag below is over the newest screenful — an earlier part of
            // the conversation than the answer still to come, and a card
            // that was following when the button went down.
            driven
                .app
                .panel_mut()
                .start_turn("question 12 about the engine", now);
            let size = redrawn(&mut driven);
            let (column, row) = drawn_at(&driven, "question 12");
            let held = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 8, row), size, now);
            // A line of its own, unlike the turn's first activity line,
            // which takes the place of the one the log draws for a turn
            // that has heard nothing yet.
            driven
                .app
                .panel_mut()
                .note("warlock has something to say", now);

            assert_eq!(
                driven.app.panel().scroll_offset(),
                held,
                "the note pulled the held card down"
            );

            point(&mut driven, RELEASE, (column + 8, row), size, now);

            let released = driven.app.panel().scroll_offset();
            assert!(
                released > held,
                "the card came out of the drag still parked where it was held"
            );

            driven.app.panel_mut().answer_turn(NEWEST, now);

            assert!(
                driven.app.panel().scroll_offset() > released,
                "the answer left the card where the drag had it"
            );
            assert_eq!(
                driven.app.panel().lines_below(),
                0,
                "the card stopped short of the newest line"
            );
            redrawn(&mut driven);
            assert!(
                found_at(&driven, NEWEST).is_some(),
                "the answer that ended the turn was never drawn"
            );
        }

        #[test]
        fn a_release_past_the_card_hands_the_newest_line_back_too() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question 0");
            let past = below_the_card(size, column);

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, past, size, now);
            // The button let go out here, where there is no cell of the card
            // under it: a release all the same, and the end of the hold.
            point(&mut driven, RELEASE, past, size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_card_put_up_mid_drag_does_not_leave_the_conversation_held() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let size = wound_back(&mut driven);
            let (column, row) = drawn_at(&driven, "question 0");

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 8, row), size, now);
            // A key pressed with the button still down, which is the one way
            // another card takes the conversation's place mid-drag.
            driven.app.show_document(["what a document does"], false);
            point(&mut driven, RELEASE, (column + 8, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_press_nobody_dragged_from_leaves_the_card_following() {
            let now = Instant::now();
            let (mut driven, size) = scrollback(now);
            let (column, row) = drawn_at(&driven, "question");

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, RELEASE, (column, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_drag_that_began_in_the_tree_leaves_the_card_following() {
            let now = Instant::now();
            let (mut driven, size) = scrollback(now);
            let (column, row) = drawn_at(&driven, "crates");

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 3, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_drag_that_began_in_the_composer_leaves_the_card_following() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            drafting(&mut driven);
            let size = redrawn(&mut driven);
            let field = drawn_at(&driven, DRAFT);
            let (column, row) = drawn_at(&driven, "question");

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, field, size, now);
            point(&mut driven, DRAG, (column, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_drag_that_began_on_the_footer_leaves_the_card_following() {
            let now = Instant::now();
            let (mut driven, size) = scrollback(now);
            let (column, row) = drawn_at(&driven, "question");

            let where_it_was = driven.app.panel().scroll_offset();

            point(&mut driven, PRESS, below_the_card(size, column), size, now);
            point(&mut driven, DRAG, (column, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_drag_over_the_document_card_leaves_the_conversation_following() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            // Read while the conversation is still the card showing, which
            // is the one moment the panel answers for it.
            let where_it_was = driven.app.panel().scroll_offset();
            driven.app.show_document(["what a document does"], false);
            let size = redrawn(&mut driven);
            let (column, row) = drawn_at(&driven, "document");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 4, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }

        #[test]
        fn a_drag_over_the_account_card_leaves_the_conversation_following() {
            let now = Instant::now();
            let (mut driven, _) = scrollback(now);
            let where_it_was = driven.app.panel().scroll_offset();
            driven.app.start_account(now);
            driven
                .app
                .panel_mut()
                .write_run(|account| account.open_section("crates/engine", now));
            // The conversation has content, so a run does not put its own
            // account up: the swap key is what a reader would press, and this
            // is that press without the keyboard.
            driven.app.panel_mut().show(Showing::Account);
            let size = redrawn(&mut driven);
            let (column, row) = drawn_at(&driven, "crates/engine");

            point(&mut driven, PRESS, (column, row), size, now);
            point(&mut driven, DRAG, (column + 4, row), size, now);

            the_conversation_still_follows(&mut driven, where_it_was, now);
        }
    }
}

mod pasting {
    use super::{Focus, Instant, directory, session};
    use crate::chatting::Asked;

    #[test]
    fn a_paste_with_the_keyboard_off_the_composer_changes_nothing_anywhere() {
        let mut driven = session(vec![directory("/repo/crates"), directory("/repo/docs")]);
        // Where a session opens: the keys are commands and there is no
        // draft for anything to land in.
        assert_ne!(
            driven.app.focus(),
            Focus::Composer,
            "this test is about the keyboard being somewhere else"
        );
        let selected = driven.app.selected();
        let mode = driven.app.panel().mode();

        driven.paste("crates\ndocs\n");

        assert_eq!(
            driven.chat.composer().draft(),
            "",
            "a paste aimed at nothing was typed into the field anyway"
        );
        assert_eq!(
            driven.app.selected(),
            selected,
            "the pasted lines moved the tree's selection"
        );
        assert_eq!(
            driven.app.panel().mode(),
            mode,
            "the pasted lines changed register"
        );
        assert!(
            driven.app.message().is_none(),
            "a paste nobody can act on said something on the footer"
        );
        assert!(!driven.chat.answering(), "a paste started a turn");
    }

    #[test]
    fn a_paste_at_a_muted_field_leaves_the_draft_byte_for_byte() {
        let mut driven = session(vec![directory("/repo/crates")]);
        driven.app.set_focus(Focus::Composer);
        driven.paste("half a question");

        // A question put out without going past the field, which is what
        // leaves a draft standing under the muting: a submit would have
        // emptied it on the way through, and `Chat::settle_field` — still
        // the one thing that sets the flag — is what `say` calls.
        driven.chat.say(
            &mut driven.app,
            "what is a pact?",
            "what is a pact?",
            Asked::Answer,
            Instant::now(),
        );
        assert!(
            driven.chat.composer().is_muted(),
            "a question is out and the field still types"
        );

        driven.paste("\nand the rest of it");

        assert_eq!(
            driven.chat.composer().draft(),
            "half a question",
            "the muted field took a paste"
        );
        assert!(
            driven.chat.composer().is_muted(),
            "a paste handed the keyboard back mid-turn"
        );
    }

    #[test]
    fn a_multi_line_paste_lands_whole_and_asks_nothing() {
        let mut driven = session(vec![directory("/repo/crates")]);
        driven.app.set_focus(Focus::Composer);

        // The block that used to send line one and lose the other two.
        driven.paste("what is a pact?\nand what is a scope?\nand a sigil?");

        assert_eq!(
            driven.chat.composer().draft(),
            "what is a pact?\nand what is a scope?\nand a sigil?",
            "the pasted lines did not all reach the draft"
        );
        assert!(
            !driven.chat.answering(),
            "a newline in a paste started a turn"
        );
        assert!(
            driven.app.panel().thread().is_none(),
            "a paste sent something: there is a conversation and nobody asked for one"
        );
        assert!(
            !driven.chat.composer().is_muted(),
            "a paste muted the field"
        );
    }
}
