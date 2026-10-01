use std::path::{Path, PathBuf};

use warlock_engine::{
    Filed, Manifest, briefs, claude_md, manifest, manifest_path, resolve_filing, scope, sigils,
};

use super::{Error, one_line, status_for};
use crate::brief::{ScopeBlockError, brief_at};
use crate::linear::Error as LinearError;
use crate::query::spelled;
use crate::rescope::ScopeRefusal;
use crate::template::Error as TemplateError;
// The tails themselves, not copies of them: these used to be re-typed here
// as literals, so rewording either original left this suite passing on a
// sentence nothing said any more.
use crate::standing::{FOR_CLAUDE_MD, FOR_CUT, FOR_SIGILS};

const PROBLEM: &str = "`/repo/crates/engine` could not be hashed and is stale: \
                           could not read `/repo/crates/engine/src/lib.rs`, so the \
                           subtree has no hash: permission denied";

#[test]
fn a_message_that_is_already_one_line_is_left_alone() {
    assert_eq!(one_line("permission denied"), "permission denied");
    assert_eq!(one_line("  padded  \n"), "padded");
    assert_eq!(one_line(""), "");
}

#[test]
fn a_parser_diagnostic_keeps_where_and_why_and_drops_the_snippet() {
    // Exactly the shape `toml` produces for a manifest that will not
    // parse, wrapped in the engine's own wording.
    let message = "could not read the pact manifest: malformed pact manifest: \
                       TOML parse error at line 1, column 5\n  |\n1 | not a manifest\n  \
                       |     ^\nkey with no value, expected `=`\n";

    assert_eq!(
        one_line(message),
        "could not read the pact manifest: malformed pact manifest: TOML parse \
             error at line 1, column 5: key with no value, expected `=`",
    );
}

#[test]
fn a_load_with_no_problems_is_not_an_error() {
    assert!(Error::from_problems(&[]).is_none());
}

#[test]
fn one_problem_is_reported_as_it_worded_itself() {
    let error = Error::Problems {
        first: PROBLEM.to_owned(),
        rest: 0,
    };

    assert_eq!(error.to_string(), PROBLEM);
}

#[test]
fn further_problems_are_counted_rather_than_listed() {
    let error = Error::Problems {
        first: PROBLEM.to_owned(),
        rest: 3,
    };

    assert_eq!(error.to_string(), format!("{PROBLEM} (and 3 more like it)"));
}

// The assertion both lists below are held to, in one place so that neither can
// drift into testing something weaker than the other.
fn each_prints_as_one_line(errors: impl IntoIterator<Item = Error>) {
    for error in errors {
        let message = error.to_string();
        assert!(!message.contains('\n'), "{error:?} wrapped: {message}");
        assert!(!message.is_empty(), "{error:?} said nothing");
    }
}

// Split from the list below rather than kept as one, because one array of every
// variant had outgrown being readable in a screen. The line is where a variant
// gets its words: these quote something raised outside this crate — an
// `io::Error`, an engine error, a rule, another library — so the flattening is
// what keeps them on one line.
#[test]
fn every_message_quoting_another_error_is_one_line_so_it_prints_as_one() {
    each_prints_as_one_line([
        Error::WorkingDirectory {
            source: std::io::Error::other("boom"),
        },
        Error::Terminal {
            source: std::io::Error::other("boom"),
        },
        Error::Manifest {
            source: manifest::Error::Io {
                path: PathBuf::from("/repo/.warlock/pacts.toml"),
                source: std::io::Error::other("boom"),
            },
        },
        Error::Unspellable {
            source: manifest::Error::PathOutsideRoot {
                root: PathBuf::from("/repo"),
                path: PathBuf::from("/elsewhere"),
            },
        },
        Error::Unspellable {
            source: manifest::Error::NonUtf8Path {
                path: PathBuf::from("/repo/odd"),
            },
        },
        Error::ClaudeMd {
            source: claude_md::Error::Write {
                path: PathBuf::from("/repo/CLAUDE.md"),
                source: std::io::Error::other("boom"),
            },
        },
        Error::ClaudeMd {
            source: claude_md::Error::NotText {
                path: PathBuf::from("/repo/CLAUDE.md"),
            },
        },
        // The two files `warlock brief` reads before it sends anything. Both
        // quote the filesystem, and the brief config can quote the TOML parser
        // as well, which is what the flattening is for.
        Error::Template {
            source: TemplateError {
                path: PathBuf::from("/repo/.warlock/brief-template.md"),
                source: std::io::Error::other("boom"),
            },
        },
        Error::Briefs {
            source: briefs::Error::Io {
                path: PathBuf::from("/repo/.warlock/briefs.toml"),
                source: std::io::Error::other("boom"),
            },
        },
        Error::Briefs {
            source: briefs::Error::AbsoluteDirectory {
                path: PathBuf::from("/repo/.warlock/briefs.toml"),
                directory: "/elsewhere".to_owned(),
            },
        },
        Error::Prompt {
            source: std::io::Error::other("boom"),
        },
        Error::Sigil {
            entered: "Data Plane!".to_owned(),
            rule: scope::Rule::Character { character: '!' },
        },
        Error::Sigils {
            source: sigils::Error::NotFound {
                path: PathBuf::from("/home/someone/.warlock/repo-abc/config.toml"),
            },
        },
        Error::Sigils {
            source: sigils::Error::Io {
                path: PathBuf::from("/home/someone/.warlock/repo-abc/config.toml"),
                source: std::io::Error::other("boom"),
            },
        },
        Error::KeyName {
            name: "Acme!".to_owned(),
            rule: scope::Rule::Character { character: '!' },
        },
        // Built through the engine rather than by hand: `keys::Error` is
        // `#[non_exhaustive]`, so nothing outside that crate can name a variant
        // in a constructor. A name the store refuses is judged before anything
        // is opened, so this reaches no filesystem.
        Error::Keys {
            source: warlock_engine::save_key(Path::new("/nowhere"), "Acme!", "lin_api_example")
                .expect_err("a name that is not a scope name is refused"),
        },
        Error::Scope {
            refusal: ScopeRefusal::Rule {
                rule: scope::Rule::Empty,
            },
        },
        Error::Signal {
            source: ctrlc::Error::MultipleHandlers,
        },
        Error::Clipboard {
            source: arboard::Error::ClipboardOccupied,
        },
        Error::ScopeBlock {
            source: ScopeBlockError::NoScope,
        },
        Error::ScopeBlock {
            source: ScopeBlockError::Circle {
                slices: vec!["1. the door".to_owned(), "2. the room".to_owned()],
            },
        },
    ]);
}

// The other half: these carry values warlock was handed — a path, a name, a
// count — and word the whole sentence themselves, so what is held here is that
// no wording above grew a newline of its own.
#[test]
fn every_message_warlock_words_itself_is_one_line_so_it_prints_as_one() {
    each_prints_as_one_line([
        Error::Problems {
            first: PROBLEM.to_owned(),
            rest: 2,
        },
        Error::NoRepository {
            start: PathBuf::from("/elsewhere"),
            wanted: FOR_CLAUDE_MD,
        },
        Error::NoRepository {
            start: PathBuf::from("/elsewhere"),
            wanted: FOR_SIGILS,
        },
        Error::NoHome,
        Error::NoKey {
            name: "acme".to_owned(),
        },
        Error::UnknownKey {
            name: "acme".to_owned(),
            wanted: "bind",
        },
        Error::ClosedScope {
            path: "crates/engine".to_owned(),
            scope: "data-plane".to_owned(),
        },
        Error::Scope {
            refusal: ScopeRefusal::NoPact {
                module: "crates/engine".to_owned(),
            },
        },
        Error::Failures {
            failed: 3,
            total: 12,
        },
        Error::Failures {
            failed: 1,
            total: 1,
        },
        Error::AllCut {
            path: "docs/brief.md".to_owned(),
        },
        Error::Cancelled,
    ]);
}

// The three ways a project is not a scope to cut, asserted as the parser's own
// sentences: this variant carries them rather than wording anything, so a test
// of its own words would be a test of nothing.
#[test]
fn a_project_with_no_scope_to_cut_says_what_the_parser_said() {
    assert_eq!(
        Error::ScopeBlock {
            source: ScopeBlockError::NoScope,
        }
        .to_string(),
        "this project has no `## Scope` heading, so there is nothing to cut into slices"
    );
    assert_eq!(
        Error::ScopeBlock {
            source: ScopeBlockError::NoSlices,
        }
        .to_string(),
        "this project's `## Scope` section has no `### ` slice headings, so there is \
         nothing to cut"
    );

    // Named as the document shows them, because the parser names them that way
    // and nothing here re-words the list.
    let circle = ScopeBlockError::Circle {
        slices: vec!["1. the door".to_owned(), "2. the room".to_owned()],
    };
    let said = circle.to_string();

    assert_eq!(Error::ScopeBlock { source: circle }.to_string(), said);
    assert_eq!(
        said,
        "these slices wait on each other, so there is no order to cut them in: \
         1. the door and 2. the room"
    );
}

#[test]
fn a_project_with_every_slice_cut_names_the_brief_and_the_file_recording_them() {
    let error = Error::AllCut {
        path: "docs/brief.md".to_owned(),
    };

    assert_eq!(
        error.to_string(),
        "every slice of the project filed for `docs/brief.md` is already cut or skipped, so \
         there is nothing to draft: `.warlock/filed.toml` holds a record for each of them, and \
         warlock offers a slice once — retitle a skipped slice in the brief to have it offered \
         again"
    );
}

// Both of the refusals added for a cut, held to the register every other one
// of its refusals takes: the ordinary 1, and never the boundary's 3 — a script
// reading one of these as "ask for a sigil" would be sent to `warlock config`
// over a brief with no `## Scope` heading.
#[test]
fn neither_new_cut_refusal_spends_the_boundarys_status() {
    for error in [
        Error::ScopeBlock {
            source: ScopeBlockError::NoScope,
        },
        Error::ScopeBlock {
            source: ScopeBlockError::NoSlices,
        },
        Error::ScopeBlock {
            source: ScopeBlockError::Circle {
                slices: vec!["1. the door".to_owned()],
            },
        },
        Error::AllCut {
            path: "docs/brief.md".to_owned(),
        },
    ] {
        let outcome = Err(error);

        assert_eq!(status_for(&outcome), 1, "{outcome:?}");
        assert_ne!(status_for(&outcome), 3, "{outcome:?}");
    }
}

#[test]
fn a_clipboard_that_refused_says_what_it_cost_and_prints_as_one_line() {
    // What an unknown clipboard failure carries is another program's
    // output — a helper the compositor started — so it arrives with
    // newlines in it and leaves here without them.
    let error = Error::Clipboard {
        source: arboard::Error::Unknown {
            description: "could not reach the compositor\nno such file".to_owned(),
        },
    };

    assert_eq!(
        error.to_string(),
        "nothing was copied: Unknown error while interacting with the \
             clipboard: could not reach the compositor: no such file"
    );
}

#[test]
fn a_manifest_that_cannot_be_saved_says_so_in_the_engines_words() {
    let error = Error::Manifest {
        source: manifest::Error::Io {
            path: PathBuf::from("/repo/.warlock/pacts.toml"),
            source: std::io::Error::other("permission denied"),
        },
    };

    assert_eq!(
        error.to_string(),
        "could not read or write `/repo/.warlock/pacts.toml`: permission denied"
    );
}

#[test]
fn a_path_with_no_repository_relative_form_says_which_root_it_is_not_inside() {
    // What `warlock stale /elsewhere` prints. The engine's sentence is
    // already the whole fact, so nothing is wrapped around it.
    let error = Error::Unspellable {
        source: manifest::Error::PathOutsideRoot {
            root: PathBuf::from("/repo"),
            path: PathBuf::from("/elsewhere"),
        },
    };

    assert_eq!(
        error.to_string(),
        "`/elsewhere` is not inside the manifest root `/repo`"
    );
}

#[test]
fn init_outside_a_repository_says_what_was_looked_for_and_where() {
    let error = Error::NoRepository {
        start: PathBuf::from("/elsewhere"),
        wanted: FOR_CLAUDE_MD,
    };

    assert_eq!(
        error.to_string(),
        "no `.git` directory in `/elsewhere` or any of its parents, so there is \
             no repository root to write `CLAUDE.md` at"
    );
}

#[test]
fn config_outside_a_repository_says_the_same_thing_about_sigils() {
    // One fact about `.git`, worded once, with what the reader asked for on
    // the end of it.
    let error = Error::NoRepository {
        start: PathBuf::from("/elsewhere"),
        wanted: FOR_SIGILS,
    };

    assert_eq!(
        error.to_string(),
        "no `.git` directory in `/elsewhere` or any of its parents, so there is \
             no repository root to hold sigils for"
    );
}

#[test]
fn cut_outside_a_repository_says_the_same_thing_about_cutting_a_brief() {
    // The one `.git` fact again, with the tail a cut asks for on the end: the
    // record that says which project a brief became is spelled against a root,
    // so there is nothing to resolve without one.
    let error = Error::NoRepository {
        start: PathBuf::from("/elsewhere"),
        wanted: FOR_CUT,
    };

    assert_eq!(
        error.to_string(),
        "no `.git` directory in `/elsewhere` or any of its parents, so there is \
             no repository root to cut a brief from"
    );
}

#[test]
fn a_string_that_is_not_a_sigil_names_itself_and_the_rule_it_broke() {
    let error = Error::Sigil {
        entered: "Data-Plane!".to_owned(),
        rule: scope::Rule::Character { character: '!' },
    };

    assert_eq!(
        error.to_string(),
        "`Data-Plane!` is not a sigil, so nothing was written: a scope holds \
             only lowercase letters, digits, `-` and `_`, and this one holds `!`"
    );
}

#[test]
fn a_scope_refusal_is_the_refusals_own_sentence_and_nothing_wrapped_round_it() {
    let refusal = ScopeRefusal::NoPact {
        module: "crates/engine".to_owned(),
    };

    assert_eq!(
        Error::Scope {
            refusal: refusal.clone()
        }
        .to_string(),
        refusal.to_string()
    );
}

#[test]
fn no_home_says_which_variables_were_looked_at_and_what_to_do() {
    assert_eq!(
        Error::NoHome.to_string(),
        "neither `HOME` nor `USERPROFILE` is set, so there is no home directory \
             to keep the sigils for this repository under: set `HOME` and run \
             `warlock config` again"
    );
}

#[test]
fn a_sigil_config_that_cannot_be_read_says_so_in_the_engines_words() {
    let error = Error::Sigils {
        source: sigils::Error::Io {
            path: PathBuf::from("/home/someone/.warlock/repo-abc/config.toml"),
            source: std::io::Error::other("permission denied"),
        },
    };

    assert_eq!(
        error.to_string(),
        "could not read or write `/home/someone/.warlock/repo-abc/config.toml`: \
             permission denied"
    );
}

#[test]
fn a_claude_md_that_cannot_be_written_says_so_in_the_engines_words() {
    let error = Error::ClaudeMd {
        source: claude_md::Error::Write {
            path: PathBuf::from("/repo/CLAUDE.md"),
            source: std::io::Error::other("permission denied"),
        },
    };

    assert_eq!(
        error.to_string(),
        "could not write `/repo/CLAUDE.md`: permission denied"
    );
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
fn the_statuses_the_older_subcommands_leave_are_where_they_were() {
    // Pinned as a table rather than left to be noticed, because `status_for`
    // grows a variant every time warlock grows a verb: a refusal added to the
    // catch-all must not move anything already sorted above it. One error per
    // register, each taken from the subcommand that really produces it.
    let statuses = [
        (Ok(()), 0),
        // The three questions and the writes, which are all the ordinary 1.
        (
            Err(Error::NoRepository {
                start: PathBuf::from("/nowhere"),
                wanted: FOR_CLAUDE_MD,
            }),
            1,
        ),
        (
            Err(Error::Problems {
                first: "`/repo/docs`: `WARLOCK.md` could not be read".to_owned(),
                rest: 2,
            }),
            1,
        ),
        (Err(Error::NoHome), 1),
        (
            Err(Error::Scope {
                refusal: ScopeRefusal::NoPact {
                    module: "crates/engine".to_owned(),
                },
            }),
            1,
        ),
        (
            Err(Error::UnknownKey {
                name: "work".to_owned(),
                wanted: "bind",
            }),
            1,
        ),
        // The un-pact's downward refusal, which is deliberately not the
        // boundary's number.
        (
            Err(Error::ClosedScopeBelow {
                path: ".".to_owned(),
                scopes: vec!["platform".to_owned()],
            }),
            1,
        ),
        // The boundary itself, and the two a run leaves behind.
        (
            Err(Error::ClosedScope {
                path: "crates/engine".to_owned(),
                scope: "data-plane".to_owned(),
            }),
            3,
        ),
        (
            Err(Error::Failures {
                failed: 3,
                total: 12,
            }),
            4,
        ),
        (Err(Error::Cancelled), 130),
    ];

    for (outcome, expected) in statuses {
        assert_eq!(status_for(&outcome), expected, "{outcome:?}");
    }
}

#[test]
fn every_refusal_a_push_has_is_the_ordinary_one_and_never_the_boundarys_three() {
    // The one subcommand that sends anything anywhere, and none of what it
    // refuses is the boundary's **3**: the sigil picks a board rather than
    // opening a directory, so a script reading a 3 as "ask for a sigil" must
    // never be sent there by a brief that would not parse or a team key Linear
    // does not know. Each of these is built by the thing that really produces
    // it where that is cheap, so a re-wrapping in `push.rs` would fail this.
    let repo = tempfile::tempdir().expect("a temporary directory");
    let home = tempfile::tempdir().expect("a temporary directory");
    let url = "https://linear.app/acme/project/a-brief-1a2b3c";
    let refusals = [
        Error::Filing {
            source: resolve_filing(&Manifest::new(), repo.path(), home.path(), None)
                .expect_err("a machine that holds no sigil files nowhere"),
        },
        Error::Brief {
            source: brief_at(repo.path(), repo.path().join("docs/brief.md"))
                .expect_err("there is no document at that path"),
        },
        Error::Filed {
            source: Filed::load(repo.path()).expect_err("this repository has filed nothing"),
        },
        Error::AlreadyFiled {
            path: "docs/brief.md".to_owned(),
            url: url.to_owned(),
        },
        Error::UnknownTeam {
            team: "WAR".to_owned(),
            path: manifest_path(repo.path()),
        },
        Error::Linear {
            source: LinearError::Status { code: 401 },
        },
        Error::Unfiled {
            url: url.to_owned(),
            source: Box::new(
                Filed::load(repo.path()).expect_err("this repository has filed nothing"),
            ),
        },
    ];

    for refusal in refusals {
        let said = refusal.to_string();
        let outcome = Err(refusal);
        assert_eq!(status_for(&outcome), 1, "{said}");
        assert_ne!(status_for(&outcome), 3, "{said}");
        // One line, because `main` prints it as one line with a `warlock: ` in
        // front of it.
        assert!(!said.contains('\n'), "{said}");
    }
}
