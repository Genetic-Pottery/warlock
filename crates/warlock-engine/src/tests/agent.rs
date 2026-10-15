use std::error::Error as _;
use std::time::Duration;

use super::Error;

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
