use std::fmt::Write as _;
use std::path::Path;

use super::{declared_names, elide, language_of};

#[test]
fn a_declaration_keyword_is_never_measured_as_the_name_it_declares() {
    // Every word a row can open a declaration with has to be in `KEYWORDS`,
    // or the extractor records the keyword and loses the name. `fun` was
    // missing, so a Kotlin file measured as declaring `fun` — and on the
    // per-file road that list is the only witness a claim has.
    assert_eq!(
        declared_names(
            Path::new("Ledger.kt"),
            "class Ledger\nfun newLedger(): Ledger = x\n"
        ),
        ["Ledger", "newLedger"]
    );
    assert_eq!(
        declared_names(Path::new("raw.rs"), "union Slot { a: u8 }\n"),
        ["Slot"]
    );
}

#[test]
fn a_go_method_is_measured_by_its_name_and_not_its_receiver() {
    // The receiver is not a name anyone looks up, and this list is what
    // `· declares` prints.
    let source = "func NewRetryApplyer(store *RetryStore) *RetryApplyer {}\n\
                      func (r *RetryApplyer) Apply(ctx context.Context) error {}\n";

    assert_eq!(
        declared_names(Path::new("retry.go"), source),
        ["NewRetryApplyer", "Apply"]
    );
}

#[test]
fn a_block_that_opens_with_a_bracket_declares_nothing_on_that_line() {
    // Go's `const (` and `var (` sit where a receiver would, and close on a
    // later line. Stepping over an unclosed group would read the next line's
    // text as this one's name.
    assert!(declared_names(Path::new("block.go"), "const (\n\tA = 1\n)\n").is_empty());
}

#[test]
fn every_declared_name_is_measured_however_many_there_are() {
    // This list is evidence before it is ever a rendered line: `render`
    // shows the first `DECLARED_SHOWN` of it and counts the rest, while
    // `Expected::knows` asks it whether a name a pass used is real. A
    // synthesis pass is shown no file text, so a name cut off the end of
    // this list has no other witness and is refused — which is a correct
    // claim dropped out of a document. `ui.rs` declares 126.
    let mut source = String::new();
    for index in 0..200 {
        let _ = writeln!(source, "fn helper_{index}() {{}}");
    }

    let names = declared_names(Path::new("wide.rs"), &source);

    assert_eq!(names.len(), 200, "the measurement was truncated");
    assert_eq!(names.last().map(String::as_str), Some("helper_199"));
}

#[test]
fn the_exports_of_a_file_are_measured_before_the_rest_of_it() {
    // The ordering survives the cap's removal, and it is what decides which
    // names `render` shows: a reader opens a file for what it exports.
    let source = "fn private_one() {}\npub fn exported() {}\nfn private_two() {}\n";

    assert_eq!(
        declared_names(Path::new("ordered.rs"), source),
        ["exported", "private_one", "private_two"]
    );
}

#[test]
fn an_unknown_extension_is_left_entirely_alone() {
    assert!(language_of(Path::new("a.wat")).is_none());
    assert!(elide(Path::new("a.wat"), "anything at all\n").is_none());
}
