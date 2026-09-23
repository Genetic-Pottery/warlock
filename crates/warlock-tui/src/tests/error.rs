use std::path::{Path, PathBuf};

use warlock_engine::{claude_md, manifest, scope, sigils};
use warlock_tui::ScopeBlockError;

use super::{Error, one_line};
use crate::rescope::ScopeRefusal;
use crate::status_for;
// The tails themselves, not copies of them: these used to be re-typed here
// as literals, so rewording either original left this suite passing on a
// sentence nothing said any more.
use crate::standing::{FOR_CLAUDE_MD, FOR_PULL, FOR_SIGILS};

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
        "every slice of the project filed for `docs/brief.md` is already cut, so there is \
         nothing to draft: `.warlock/filed.toml` holds a record for each of them, and \
         warlock cuts a slice once"
    );
}

// Both of the refusals added for a pull, held to the register every other one
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
fn pull_outside_a_repository_says_the_same_thing_about_cutting_a_brief() {
    // The one `.git` fact again, with the tail a pull asks for on the end: the
    // record that says which project a brief became is spelled against a root,
    // so there is nothing to resolve without one.
    let error = Error::NoRepository {
        start: PathBuf::from("/elsewhere"),
        wanted: FOR_PULL,
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
