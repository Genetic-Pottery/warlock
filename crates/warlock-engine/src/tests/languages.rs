use std::fmt::Write as _;
use std::path::Path;

use super::{COMMENTS, TABLE, comments_of, declared_names, elide, language_of, without_comments};

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
    assert!(without_comments(Path::new("a.wat"), "// not stripped\n").is_none());
}

#[test]
fn every_language_row_still_has_a_comment_form() {
    // The two tables are keyed separately and nothing in the type system ties
    // them together, so a language can be described for eliding and skeletons
    // while its comments quietly go on counting as evidence. That is the state
    // the whole module was in for C, and it is the state adding a `TABLE` row
    // and forgetting `COMMENTS` puts one language back into.
    for language in TABLE {
        for extension in language.extensions {
            assert!(
                comments_of(Path::new(&format!("a.{extension}"))).is_some(),
                ".{extension} is described but its comments are evidence"
            );
        }
    }
}

#[test]
fn no_extension_is_claimed_by_two_comment_rows() {
    // `comments_of` takes the first match, so a duplicate is a row that looks
    // like it is doing something and is not.
    let mut seen: Vec<&str> = Vec::new();
    for (extensions, _) in COMMENTS {
        for extension in *extensions {
            assert!(!seen.contains(extension), ".{extension} is claimed twice");
            seen.push(extension);
        }
    }
}

#[test]
fn a_comment_stops_being_evidence_in_a_language_with_no_declarations_described() {
    // The fixture's own `legacy/` is exactly this: a `.c` and a `.cpp` whose
    // comments were evidence because covering them used to mean inventing C's
    // declaration prefixes as well. Nothing in `TABLE` claims either extension
    // and nothing has to.
    assert!(language_of(Path::new("codec.c")).is_none());

    let c = without_comments(
        Path::new("codec.c"),
        "/* A unicorn maintains this file. */\nconst int CODEC_VERSION = 4;\n",
    )
    .expect("a .c file has a comment form");
    assert!(!c.contains("unicorn"), "{c}");
    assert!(c.contains("CODEC_VERSION"), "{c}");

    let cpp = without_comments(
        Path::new("decoder.cpp"),
        "// Validates every Posting.\nclass Decoder {\n};\n",
    )
    .expect("a .cpp file has a comment form");
    assert!(!cpp.contains("Posting"), "{cpp}");
    assert!(cpp.contains("Decoder"), "{cpp}");

    // The shape the fixture actually plants, and the one a C file is most
    // likely to carry: a block opened on one line and closed several later,
    // with every line between it starred. Closing per line instead of tracking
    // the opener across them strips the first line and leaves the claim.
    let banner = without_comments(
        Path::new("codec.c"),
        "/* Legacy codec.\n \
         * Every Frame is stamped with LEDGER_VERSION as encode() writes it.\n \
         */\nint encode(const Frame *frame) { return 0; }\n",
    )
    .expect("a .c file has a comment form");
    assert!(!banner.contains("LEDGER_VERSION"), "{banner}");
    assert!(banner.contains("int encode"), "{banner}");
}

#[test]
fn each_comment_form_added_strips_its_own_marker() {
    // One case per marker shape in the table, because a wrong closer or a
    // marker in the wrong field fails silently: the text comes back whole and
    // a lie in it still witnesses a claim.
    let cases = [
        (
            "main.tf",
            "# a griffin applies this\nresource \"x\" {}\n",
            "griffin",
            "resource",
        ),
        (
            "up.sql",
            "-- a griffin wrote this migration\nSELECT id FROM t;\n",
            "griffin",
            "SELECT",
        ),
        (
            "init.lua",
            "--[[ a griffin\nsits here ]] local x = 1\n",
            "griffin",
            "local x",
        ),
        (
            "Main.hs",
            "{- a griffin -}\nmain :: IO ()\n",
            "griffin",
            "main",
        ),
        (
            "core.clj",
            "; a griffin\n(defn post [] nil)\n",
            "griffin",
            "defn post",
        ),
        (
            "index.html",
            "<!-- a griffin -->\n<div id=\"post\"></div>\n",
            "griffin",
            "div",
        ),
        (
            "build.sh",
            "# a griffin builds this\ncargo build\n",
            "griffin",
            "cargo build",
        ),
        (
            "stats.jl",
            "#= a griffin =#\nfunction post() end\n",
            "griffin",
            "function post",
        ),
    ];

    for (name, text, prose, code) in cases {
        let kept = without_comments(Path::new(name), text)
            .unwrap_or_else(|| panic!("{name} has no comment form"));
        assert!(!kept.contains(prose), "{name} kept its comment: {kept}");
        assert!(kept.contains(code), "{name} lost its code: {kept}");
    }
}

#[test]
fn a_css_url_survives_because_css_has_no_line_comment() {
    // `//` is not a comment in plain CSS, so treating it as one cuts a real
    // declaration off at the scheme. The preprocessor dialects are the ones
    // that have it, and this holds the distinction the table draws.
    let css = without_comments(
        Path::new("site.css"),
        "/* a griffin */\nbody { background: url(https://x.test/a.png); }\n",
    )
    .expect("a .css file has a comment form");
    assert!(!css.contains("griffin"), "{css}");
    assert!(css.contains("https://x.test/a.png"), "{css}");

    let scss = without_comments(Path::new("site.scss"), "// a griffin\n$gap: 4px;\n")
        .expect("a .scss file has a comment form");
    assert!(!scss.contains("griffin"), "{scss}");
    assert!(scss.contains("$gap"), "{scss}");
}
