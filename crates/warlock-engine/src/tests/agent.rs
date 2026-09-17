use std::error::Error as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Agent, ChildDocument, Error, File, Request, Response};

struct Canned {
    markdown: &'static str,
    seen: std::cell::RefCell<Vec<(String, PathBuf)>>,
}

impl Canned {
    fn new(markdown: &'static str) -> Self {
        Self {
            markdown,
            seen: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl Agent for Canned {
    fn run(&self, request: &Request) -> Result<Response, Error> {
        self.seen.borrow_mut().push((
            request.prompt().to_owned(),
            request.directory().to_path_buf(),
        ));
        Ok(Response::new(self.markdown))
    }
}

struct Refuses;

impl Agent for Refuses {
    fn run(&self, _request: &Request) -> Result<Response, Error> {
        Err(Error::NotFound {
            program: "claude".to_owned(),
        })
    }
}

fn every_variant() -> Vec<Error> {
    vec![
        Error::NotFound {
            program: "claude".to_owned(),
        },
        Error::Failed {
            code: Some(2),
            stderr: "Invalid API key\nRun `claude login` first\n".to_owned(),
        },
        Error::Failed {
            code: None,
            stderr: String::new(),
        },
        Error::EmptyOutput,
        Error::TimedOut {
            after: Duration::from_mins(5),
        },
        Error::Io {
            source: std::io::Error::other("broken pipe"),
        },
    ]
}

#[test]
fn a_fake_agent_answers_with_canned_markdown() {
    let agent = Canned::new("# engine\n\nCore engine for warlock.\n");
    let request = Request::new("describe this directory", "crates/warlock-engine");

    let response = agent.run(&request).expect("the fake always answers");

    assert_eq!(response.text(), "# engine\n\nCore engine for warlock.\n");
    assert_eq!(
        response.clone().into_text(),
        response.text(),
        "owned and borrowed are the same text"
    );
    assert_eq!(
        agent.seen.borrow().as_slice(),
        [(
            "describe this directory".to_owned(),
            PathBuf::from("crates/warlock-engine"),
        )],
        "the request reaches the implementation whole"
    );
}

#[test]
fn the_same_agent_runs_more_than_one_pass() {
    // `&self`, not `&mut self`: pacting a subtree is many passes through
    // one implementation.
    let agent = Canned::new("# module\n");
    for directory in ["crates/warlock-engine", "crates/warlock-tui"] {
        let response = agent
            .run(&Request::new("summarise", directory))
            .expect("the fake always answers");
        assert_eq!(response.text(), "# module\n");
    }
    assert_eq!(agent.seen.borrow().len(), 2);
}

#[test]
fn a_failing_agent_comes_back_as_an_error_not_a_document() {
    let error = Refuses
        .run(&Request::new("summarise", "."))
        .expect_err("this fake reaches nothing");

    assert!(matches!(error, Error::NotFound { .. }), "{error:?}");
}

#[test]
fn a_request_carries_its_prompt_and_directory() {
    let request = Request::new("summarise this module", "/repo/crates/engine");

    assert_eq!(request.prompt(), "summarise this module");
    assert_eq!(request.directory(), Path::new("/repo/crates/engine"));
    assert_eq!(request, request.clone());
}

#[test]
fn a_request_needs_no_files_or_child_documents_to_exist() {
    // `new` alone is still a whole request: everything the widening added
    // is optional, so nothing that built one before has to change.
    let request = Request::new("summarise this module", "/repo/crates/engine");

    assert!(request.files().is_empty());
    assert!(request.child_documents().is_empty());
}

#[test]
fn a_request_round_trips_its_files() {
    let request = Request::new("summarise", "/repo/crates/engine").with_files([
        File::present("src/lib.rs", *b"//! Core engine.\n"),
        File::present("logo.png", vec![0x89, b'P', b'N', b'G', 0x00, 0xff]),
    ]);

    let files = request.files();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].path(), "src/lib.rs");
    assert_eq!(files[0].bytes(), Some(&b"//! Core engine.\n"[..]));
    assert_eq!(files[0].size(), 17);
    assert!(!files[0].is_omitted());
    assert_eq!(
        files[1].bytes(),
        Some(&[0x89, b'P', b'N', b'G', 0x00, 0xff][..]),
        "bytes, not text: a file in a directory need not be UTF-8"
    );
}

#[test]
fn an_omitted_file_is_listed_by_name_and_size_with_no_bytes() {
    let request =
        Request::new("summarise", "/repo").with_files([File::omitted("Cargo.lock", 4_200_000)]);

    let file = &request.files()[0];
    assert_eq!(file.path(), "Cargo.lock");
    assert_eq!(file.size(), 4_200_000);
    assert!(file.is_omitted());
    assert_eq!(
        file.bytes(),
        None,
        "never truncated: an omitted file has no bytes at all, not some of them"
    );
}

#[test]
fn the_states_answer_the_same_questions_differently() {
    // One table, so no state can quietly start answering like another.
    let present = File::present("src/lib.rs", *b"//! Core engine.\n");
    let omitted = File::omitted("Cargo.lock", 4_200_000);
    let elided = File::elided("scope.rs", 40_000, "pub fn covering() {}\n");

    assert_eq!(
        [
            present.bytes().is_some(),
            omitted.bytes().is_some(),
            elided.bytes().is_some(),
        ],
        [true, false, false]
    );
    assert_eq!(
        [present.size(), omitted.size(), elided.size()],
        [17, 4_200_000, 40_000]
    );
    assert_eq!(
        [present.kept(), omitted.kept(), elided.kept()],
        [None, None, Some("pub fn covering() {}\n")]
    );
    assert_eq!(
        [
            present.is_omitted(),
            omitted.is_omitted(),
            elided.is_omitted(),
        ],
        [false, true, false]
    );
}

#[test]
fn a_request_round_trips_its_child_documents() {
    let request = Request::new("summarise", "/repo/crates/engine").with_child_documents([
        ChildDocument::new("src", "# src\n\nThe code.\n"),
        ChildDocument::new("tests", "# tests\n"),
    ]);

    let children = request.child_documents();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].directory(), "src");
    assert_eq!(children[0].text(), "# src\n\nThe code.\n");
    assert_eq!(children[1].directory(), "tests");
}

#[test]
fn the_builders_add_rather_than_replace() {
    let request = Request::new("summarise", "/repo")
        .with_files([File::present("a.rs", *b"a")])
        .with_child_documents([ChildDocument::new("one", "# one\n")])
        .with_files([File::omitted("b.bin", 9)])
        .with_child_documents([ChildDocument::new("two", "# two\n")]);

    assert_eq!(
        request
            .files()
            .iter()
            .map(File::path)
            .collect::<Vec<_>>()
            .as_slice(),
        ["a.rs", "b.bin"]
    );
    assert_eq!(
        request
            .child_documents()
            .iter()
            .map(ChildDocument::directory)
            .collect::<Vec<_>>()
            .as_slice(),
        ["one", "two"]
    );
    assert_eq!(request, request.clone(), "and the whole thing is a value");
}

#[test]
fn a_request_has_no_slot_for_the_directorys_own_previous_document() {
    // The assertion that has to keep holding: a pass is shown the files
    // and the children's documents and nothing its predecessor wrote. The
    // type has no field for one, so the only way a previous document could
    // reach a pass is as an ordinary file — and `fitting` drops it from the
    // listing. What is checked here is the half this type owns.
    let request = Request::new("summarise", "/repo/crates/engine")
        .with_files([File::present("src/lib.rs", *b"//! Core engine.\n")])
        .with_child_documents([ChildDocument::new("src", "# src\n")]);

    assert_eq!(request.files().len(), 1);
    assert!(
        request
            .child_documents()
            .iter()
            .all(|child| child.directory() != "." && child.directory() != "WARLOCK.md"),
        "a directory's own document is not one of its children's"
    );
}

#[test]
fn the_prompt_can_be_replaced_once_the_context_is_known() {
    let request = Request::new("placeholder", "/repo")
        .with_files([File::present("a.rs", *b"a")])
        .with_prompt("the real instructions");
    assert_eq!(request.prompt(), "the real instructions");
    assert_eq!(request.files().len(), 1);
}

#[test]
fn a_request_with_context_still_reaches_an_agent_whole() {
    let agent = Canned::new("# engine\n");
    let request = Request::new("describe this directory", "crates/warlock-engine")
        .with_files([File::present("src/lib.rs", *b"//! Core engine.\n")])
        .with_child_documents([ChildDocument::new("src", "# src\n")]);

    agent.run(&request).expect("the fake always answers");

    assert_eq!(
        agent.seen.borrow().as_slice(),
        [(
            "describe this directory".to_owned(),
            PathBuf::from("crates/warlock-engine"),
        )],
        "widening the request breaks no existing implementation of the trait"
    );
}

#[test]
fn every_error_variant_displays_on_one_line() {
    for error in every_variant() {
        let rendered = error.to_string();
        assert!(!rendered.is_empty(), "{error:?} says nothing");
        assert!(
            !rendered.contains('\n'),
            "{error:?} renders across lines: {rendered}"
        );
    }
}

#[test]
fn the_missing_binary_is_named_rather_than_an_errno() {
    let error = Error::NotFound {
        program: "claude".to_owned(),
    };
    let rendered = error.to_string();

    assert!(rendered.contains("claude"), "{rendered}");
    assert!(rendered.contains("PATH"), "{rendered}");
    assert!(
        !rendered.contains("os error"),
        "a bare io::Error is exactly what this variant exists to avoid: {rendered}"
    );
}

#[test]
fn a_non_zero_exit_shows_its_status_and_flattens_its_stderr() {
    let error = Error::Failed {
        code: Some(2),
        stderr: "Invalid API key\nRun `claude login` first\n".to_owned(),
    };
    let rendered = error.to_string();

    assert!(rendered.contains('2'), "{rendered}");
    assert!(
        rendered.contains("Invalid API key Run `claude login` first"),
        "multi-line stderr is flattened, not dropped: {rendered}"
    );
}

#[test]
fn an_enormous_stderr_is_excerpted() {
    let error = Error::Failed {
        code: Some(1),
        stderr: "boom ".repeat(10_000),
    };
    let rendered = error.to_string();

    assert!(rendered.chars().count() < 300, "{rendered}");
    assert!(
        rendered.contains('…'),
        "the excerpt says it was cut: {rendered}"
    );
}

#[test]
fn a_timeout_says_how_long_it_waited_and_is_not_an_exit() {
    let error = Error::TimedOut {
        after: Duration::from_mins(5),
    };

    assert!(error.to_string().contains("300"), "{error}");
    assert!(
        !matches!(error, Error::Failed { .. }),
        "a hang is its own answer, not an exit code"
    );
}

#[test]
fn only_the_io_variant_has_a_source() {
    for error in every_variant() {
        let expected = matches!(error, Error::Io { .. });
        assert_eq!(
            error.source().is_some(),
            expected,
            "{error:?} disagrees about having a source"
        );
    }
    let io = Error::Io {
        source: std::io::Error::other("broken pipe"),
    };
    assert_eq!(
        io.source().expect("an io error has a source").to_string(),
        "broken pipe"
    );
}
