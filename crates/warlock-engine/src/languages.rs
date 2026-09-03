//! Which bytes of a source file a documenter does not need: the per-language
//! table behind the elision rung of [`fitting`](crate::fitting)'s ladder.
//!
//! A pass writing a `WARLOCK.md` is asked what a directory is, what its parts
//! do together, and what a reader has to know before changing anything. Test
//! bodies answer none of that, and in a well-tested repository they are most of
//! the bytes — in this one, 57% of everything under the two `src/` directories.
//! Test *names* are the opposite: `refuses_when_scope_closed` is a sentence
//! about behaviour in three words, and dropping it would lose exactly the thing
//! the prompt asks for. So the rung this module serves keeps the names and
//! gives up the bodies, and buys back the room the budget was otherwise going
//! to take out of the source.
//!
//! # This is a table, not a parser
//!
//! Warlock documents whatever a person points it at, and that is not going to
//! be Rust. Zig, Go, TypeScript, Python and assembly all have to work, so the
//! knowledge here is arranged as rows to add rather than a language to teach:
//! one [`Language`] per row, found by file extension, saying the two things
//! that vary between languages and nothing else.
//!
//! * **Where the tests are.** Most languages put them in files of their own —
//!   Go's `_test.go`, TypeScript's `.test.ts`, Python's `test_*.py` — and for
//!   those the answer is a predicate over the file name ([`Language::is_test_file`]).
//!   A few put them inline in the file they test — Rust's `#[cfg(test)] mod
//!   tests`, Zig's `test "..." { }` — and for those the answer is a pair of
//!   anchored delimiters ([`Block`]).
//! * **What a declaration looks like**, so the names survive the body
//!   ([`Language::declares`]).
//!
//! **An extension with no row is left completely alone.** Not a guess, not a
//! brace-counting heuristic applied hopefully to a language nobody described:
//! the file goes to the pass whole, exactly as it does today, and the rungs
//! below the elision one — summarise, then name and size — are still there to
//! meet the budget. Adding a language makes warlock cheaper on that language
//! and can never make it wrong on another, which is the property that lets the
//! table grow one row at a time from real repositories instead of having to be
//! complete before it is useful.
//!
//! # Why the delimiters are anchored to column zero
//!
//! [`Block`] finds an inline test region by matching an opening line and then
//! the next line equal to its closer, both at the start of a line with no
//! indentation. That is not brace matching and deliberately not: a real matcher
//! has to know the language's strings, character literals, raw strings and
//! comment forms, or it counts a `{` inside `"a { b"` and runs off the end of
//! the file. Getting that wrong silently deletes real code.
//!
//! The anchor sidesteps all of it. A top-level item's closing brace sits in
//! column zero in every formatted file, and a brace *inside* a string or a
//! nested body is indented, so the first unindented closer after an unindented
//! opener is the end of the item — with no lexer, and wrong only in a file no
//! formatter has ever seen. That is a real limitation and it is the reason
//! [`elide`] answers `None` rather than guessing when it cannot find the
//! closer: an unterminated block is a file this module does not understand, and
//! the honest response is to hand the pass the whole thing.
//!
//! # What elision is, next to the rungs around it
//!
//! Every line the pass receives is a line the file really contains, in the
//! order it contains it, with a marker standing where the dropped lines were.
//! Nothing is rewritten, nothing is paraphrased, and nothing is cut mid-way
//! through a line — so this is not the truncation `fitting` forbids, which is a
//! file stopped at an arbitrary byte with no notice that anything is missing.
//! An elided file says what it dropped and how much of it there was, which is
//! the same bargain a listed file makes about its contents and a summarised one
//! makes about its prose.

use std::path::Path;

/// A region of a file that is opened and closed by whole lines at column zero.
///
/// The inline half of the table: Rust's `#[cfg(test)] mod tests { … }` and
/// Zig's `test "name" { … }` are both "a line that starts it, a line that ends
/// it, neither indented". See the [module docs](self) for why the anchoring is
/// the whole trick and what it costs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Block {
    /// A line at column zero that may open the region — Rust's `#[cfg(test)]`.
    /// Matched exactly, after trailing whitespace is trimmed.
    opener: &'static str,
    /// What the line *after* the opener must begin with for the region to be
    /// real, or `None` where the opener is the whole of it.
    ///
    /// Rust needs this and it is the reason the field exists: `#[cfg(test)]`
    /// sits on `mod stubs;` and on test-only helper functions as well as on the
    /// test module, and eliding to the next unindented `}` from one of those
    /// would take a working chunk of the file with it.
    confirms: Option<&'static str>,
    /// The line at column zero that ends the region, matched exactly.
    closer: &'static str,
}

/// One language's answer to where its tests are and what a declaration in it
/// looks like.
///
/// Rows live in [`TABLE`] and are found by [`language_of`]. Every field is data
/// rather than code on purpose: a new language is a new row, reviewable at a
/// glance, and nothing in this module has to be understood to add one.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Language {
    /// The file extensions this row claims, lowercase and without the dot.
    extensions: &'static [&'static str],
    /// Filename suffixes that make a file a test file whole — Go's `_test.go`,
    /// TypeScript's `.test.ts`. Empty where the language has no such
    /// convention.
    test_suffixes: &'static [&'static str],
    /// Filename prefixes that do the same — Python's `test_`. Empty where the
    /// language has no such convention.
    test_prefixes: &'static [&'static str],
    /// Inline test regions, for the languages that put tests in the file they
    /// test. Empty for the many that do not.
    blocks: &'static [Block],
    /// Line prefixes, after indentation is trimmed, that introduce something
    /// worth keeping when the body around them is dropped.
    declarations: &'static [&'static str],
}

impl Language {
    /// Whether this file is a test file entire, judged by its name alone.
    ///
    /// The name and not the contents, because that is what the convention
    /// actually is: Go does not look inside `foo_test.go` to decide it is a
    /// test, and neither does any runner for the other languages here.
    fn is_test_file(&self, name: &str) -> bool {
        self.test_suffixes
            .iter()
            .any(|suffix| name.ends_with(suffix))
            || self
                .test_prefixes
                .iter()
                .any(|prefix| name.starts_with(prefix))
    }

    /// Whether `line` — already trimmed of its indentation — introduces
    /// something whose name is worth keeping.
    fn declares(&self, line: &str) -> bool {
        self.declarations
            .iter()
            .any(|prefix| line.starts_with(prefix))
    }
}

/// Every language warlock knows how to make cheaper.
///
/// Ordered by nothing in particular; [`language_of`] matches on extension, so
/// no two rows may claim the same one. Adding a row is the whole of adding a
/// language — see the [module docs](self).
static TABLE: &[Language] = &[
    // Rust. Tests live inline, under `#[cfg(test)]`, and the attribute also
    // appears on `mod stubs;` and on test-only helpers — hence `confirms`.
    Language {
        extensions: &["rs"],
        test_suffixes: &[],
        test_prefixes: &[],
        blocks: &[Block {
            opener: "#[cfg(test)]",
            confirms: Some("mod "),
            closer: "}",
        }],
        declarations: &[
            "fn ",
            "pub fn ",
            "async fn ",
            "pub async fn ",
            "#[test]",
            "#[tokio::test]",
            "struct ",
            "enum ",
            "impl ",
        ],
    },
    // Zig. `test "name" { … }` sits at the top level of the file it tests.
    Language {
        extensions: &["zig"],
        test_suffixes: &[],
        test_prefixes: &[],
        blocks: &[Block {
            opener: "test ",
            confirms: None,
            closer: "}",
        }],
        declarations: &["fn ", "pub fn ", "const ", "test "],
    },
    // Go. `_test.go` is the toolchain's own rule, not a convention.
    Language {
        extensions: &["go"],
        test_suffixes: &["_test.go"],
        test_prefixes: &[],
        blocks: &[],
        declarations: &["func ", "type ", "//"],
    },
    // TypeScript and JavaScript, in their four extensions and both of the two
    // naming conventions every runner in that ecosystem accepts.
    Language {
        extensions: &["ts", "tsx", "js", "jsx", "mts", "cts"],
        test_suffixes: &[
            ".test.ts",
            ".test.tsx",
            ".test.js",
            ".test.jsx",
            ".spec.ts",
            ".spec.tsx",
            ".spec.js",
            ".spec.jsx",
        ],
        test_prefixes: &[],
        blocks: &[],
        declarations: &[
            "export ",
            "function ",
            "class ",
            "interface ",
            "type ",
            "const ",
            "describe(",
            "it(",
            "test(",
        ],
    },
    // Python. Both halves of pytest's discovery rule.
    Language {
        extensions: &["py"],
        test_suffixes: &["_test.py"],
        test_prefixes: &["test_"],
        blocks: &[],
        declarations: &["def ", "async def ", "class ", "@"],
    },
    // Ruby.
    Language {
        extensions: &["rb"],
        test_suffixes: &["_spec.rb", "_test.rb"],
        test_prefixes: &[],
        blocks: &[],
        declarations: &["def ", "class ", "module ", "describe ", "it "],
    },
    // Java, Kotlin, C# and Swift, which share a suffix convention closely
    // enough that one row serves and the declarations overlap almost entirely.
    Language {
        extensions: &["java", "kt", "cs", "swift"],
        test_suffixes: &[
            "Test.java",
            "Tests.java",
            "Test.kt",
            "Tests.kt",
            "Test.cs",
            "Tests.cs",
            "Test.swift",
            "Tests.swift",
        ],
        test_prefixes: &[],
        blocks: &[],
        declarations: &[
            "public ",
            "private ",
            "internal ",
            "protected ",
            "class ",
            "struct ",
            "func ",
            "fun ",
            "@",
        ],
    },
    // Elixir, whose test files are the only `.exs` most projects have.
    Language {
        extensions: &["ex", "exs"],
        test_suffixes: &["_test.exs"],
        test_prefixes: &[],
        blocks: &[],
        declarations: &["def ", "defp ", "defmodule ", "test ", "describe "],
    },
];

/// The row claiming `path`'s extension, or `None` where nothing does.
///
/// `None` is the ordinary answer and not a failure: it means warlock has
/// nothing to say about this kind of file and will send it whole.
fn language_of(path: &Path) -> Option<&'static Language> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    TABLE
        .iter()
        .find(|language| language.extensions.contains(&extension.as_str()))
}

/// What [`elide`] managed to leave out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Elided {
    /// The file as the pass will see it: every surviving line verbatim, with a
    /// marker where each dropped region was.
    pub(crate) text: String,
    /// How many bytes of the original are not in `text`. Reported to the pass
    /// and to the observer, so the saving is a stated fact rather than a
    /// silent one.
    pub(crate) dropped: u64,
}

/// The marker left standing where a dropped region was.
///
/// Written into the text rather than reported only alongside it, because the
/// pass reads the text and the point is that it can see the hole: a file that
/// simply stopped having a test module would look like a file that never had
/// one, which is the false silence the prompt spends a paragraph forbidding.
///
/// Lines rather than bytes, because the request already tells the pass the
/// file's real size on disk and a second number in the middle of the text
/// would only invite arithmetic against it.
fn marker(lines: usize) -> String {
    format!("… {lines} lines of test bodies elided …")
}

/// Drop what a documenter does not need from `text`, or answer `None` if there
/// is nothing this module knows how to drop.
///
/// `None` covers every case where the file is best sent as it is: an extension
/// with no row, a language whose tests live elsewhere, a file with no test
/// region in it, and — deliberately — a block whose closer never arrives, which
/// is a file no formatter has touched and not one to guess at.
///
/// A **test file entire** keeps only its declaration lines, which for a test
/// file is very nearly its list of test names. A file with **inline test
/// blocks** keeps everything outside them untouched and, inside them, the same
/// declaration lines. Either way the answer is `Some` only when it is really
/// smaller than what came in.
pub(crate) fn elide(path: &Path, text: &str) -> Option<Elided> {
    let language = language_of(path)?;
    let name = path.file_name()?.to_str()?;

    let lines: Vec<&str> = text.lines().collect();
    let kept = if language.is_test_file(name) {
        keep_declarations(language, &lines, 0, lines.len())
    } else {
        keep_outside_blocks(language, &lines)?
    };

    let text = kept.join("\n");
    let before = byte_length(&lines);
    let after = byte_length(&kept.iter().map(String::as_str).collect::<Vec<_>>());
    if after >= before {
        return None;
    }
    Some(Elided {
        dropped: before - after,
        text,
    })
}

/// What a slice of lines costs once rejoined with newlines, counted the same
/// way on both sides of an elision so the saving is a real comparison.
fn byte_length(lines: &[&str]) -> u64 {
    let content: usize = lines.iter().map(|line| line.len()).sum();
    let separators = lines.len().saturating_sub(1);
    (content + separators) as u64
}

/// Every line of `lines[from..to]` that introduces something, plus one marker
/// standing for everything dropped.
fn keep_declarations(language: &Language, lines: &[&str], from: usize, to: usize) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    let mut dropped = 0usize;

    for line in &lines[from..to] {
        if language.declares(line.trim_start()) {
            kept.push((*line).to_string());
        } else {
            dropped += 1;
        }
    }
    if dropped > 0 {
        kept.push(marker(dropped));
    }
    kept
}

/// Keep the file whole except inside its inline test blocks, where only
/// declaration lines survive.
///
/// Answers `None` when the file has no block at all — nothing to do — and when
/// a block opens and never closes, which is the unformatted file the module
/// docs decline to guess at.
fn keep_outside_blocks(language: &Language, lines: &[&str]) -> Option<Vec<String>> {
    if language.blocks.is_empty() {
        return None;
    }

    let mut kept: Vec<String> = Vec::new();
    let mut index = 0usize;
    let mut found = false;

    while index < lines.len() {
        let line = lines[index];
        let Some(block) = opens_here(language, lines, index) else {
            kept.push(line.to_string());
            index += 1;
            continue;
        };

        // The closer is the next line equal to it at column zero. Searching
        // from the line after the opener, so a one-line block cannot close on
        // its own opener.
        let end = lines
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, line)| line.trim_end() == block.closer)
            .map(|(at, _)| at)?;

        found = true;
        // The opening lines stay: they say a test module is here, which is a
        // fact about the file, and the marker inside says what was in it.
        let head = block.confirms.map_or(1, |_| 2);
        for line in &lines[index..index + head] {
            kept.push((*line).to_string());
        }
        kept.extend(keep_declarations(language, lines, index + head, end));
        kept.push(lines[end].to_string());
        index = end + 1;
    }

    found.then_some(kept)
}

/// The block opening at `lines[index]`, if one does.
///
/// Both halves are checked here rather than at the call site so the confirming
/// line — the thing that tells `#[cfg(test)] mod tests {` apart from
/// `#[cfg(test)] mod stubs;` — can never be forgotten by a future caller.
fn opens_here(language: &Language, lines: &[&str], index: usize) -> Option<&'static Block> {
    let line = lines[index].trim_end();
    language.blocks.iter().find(|block| {
        if !line.starts_with(block.opener) || line.len() != line.trim_start().len() {
            return false;
        }
        match block.confirms {
            None => line.ends_with('{'),
            Some(confirms) => lines.get(index + 1).is_some_and(|next| {
                let next = next.trim_end();
                next.ends_with('{') && next.split_whitespace().any(|word| confirms.trim() == word)
            }),
        }
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{elide, language_of};

    #[test]
    fn an_unknown_extension_is_left_entirely_alone() {
        assert!(language_of(Path::new("a.wat")).is_none());
        assert!(elide(Path::new("a.wat"), "anything at all\n").is_none());
    }

    #[test]
    fn a_rust_file_keeps_everything_outside_its_test_module() {
        let source = "\
//! A module.

pub fn work() -> u32 {
    let braces = \"a { that is not code\";
    braces.len() as u32
}

#[cfg(test)]
mod tests {
    use super::work;

    #[test]
    fn it_works() {
        assert_eq!(work(), 20);
    }
}
";
        let elided = elide(Path::new("a.rs"), source).expect("a test module is elidable");

        assert!(
            elided
                .text
                .contains("let braces = \"a { that is not code\";"),
            "code outside the block is untouched, brace in a string and all: {}",
            elided.text
        );
        assert!(
            elided.text.contains("fn it_works()"),
            "the test's name is the thing worth keeping: {}",
            elided.text
        );
        assert!(
            !elided.text.contains("assert_eq!(work(), 20)"),
            "the body is what is given up: {}",
            elided.text
        );
        assert!(elided.dropped > 0, "and the saving is reported");
    }

    #[test]
    fn cfg_test_on_something_that_is_not_a_module_elides_nothing() {
        // `#[cfg(test)] mod stubs;` and `#[cfg(test)] fn helper()` both appear
        // in this repository above real code. Eliding to the next unindented
        // `}` from either would take that code with it.
        let source = "\
#[cfg(test)]
mod stubs;

pub fn real() -> u32 {
    7
}
";
        assert!(
            elide(Path::new("a.rs"), source).is_none(),
            "the attribute alone does not open a block"
        );
    }

    #[test]
    fn a_block_that_never_closes_is_not_guessed_at() {
        let source = "#[cfg(test)]\nmod tests {\n    fn hanging() {\n";
        assert!(
            elide(Path::new("a.rs"), source).is_none(),
            "an unterminated block is a file to send whole, not one to cut"
        );
    }

    #[test]
    fn a_go_test_file_keeps_its_test_names() {
        let source = "\
package thing

func TestScopeCloses(t *testing.T) {
\tstate := Load()
\tif Closed(state) {
\t\tt.Fatal(\"expected open\")
\t}
\tif !Open(state) {
\t\tt.Fatal(\"expected open\")
\t}
}
";
        let elided = elide(Path::new("scope_test.go"), source).expect("a _test.go is elidable");

        assert!(
            elided.text.contains("func TestScopeCloses"),
            "{}",
            elided.text
        );
        assert!(!elided.text.contains("t.Fatal"), "{}", elided.text);
    }

    #[test]
    fn an_ordinary_go_file_is_left_alone() {
        // Go's tests are elsewhere, so there is nothing in a `.go` file to drop.
        let source = "package thing\n\nfunc Work() int {\n\treturn 7\n}\n";
        assert!(elide(Path::new("scope.go"), source).is_none());
    }

    #[test]
    fn typescript_and_python_test_files_are_recognised_by_name() {
        let ts = "describe('x', () => {\n  it('works', () => {\n    const a = compute();\n    const b = other();\n    expect(a).toBe(1);\n    expect(b).toBe(2);\n  });\n});\n";
        assert!(elide(Path::new("x.test.ts"), ts).is_some());
        assert!(elide(Path::new("x.ts"), ts).is_none(), "not a test file");

        let py = "def test_it_works():\n    first = compute()\n    second = other()\n    assert first == 7\n    assert second == 8\n";
        assert!(elide(Path::new("test_thing.py"), py).is_some());
        assert!(
            elide(Path::new("thing.py"), py).is_none(),
            "not a test file"
        );
    }

    #[test]
    fn a_zig_test_block_is_elided_without_a_confirming_line() {
        let source = "\
pub fn work() u32 {
    return 7;
}

test \"work returns seven\" {
    const first = work();
    const second = work();
    try std.testing.expectEqual(@as(u32, 7), first);
    try std.testing.expectEqual(@as(u32, 7), second);
}
";
        let elided = elide(Path::new("a.zig"), source).expect("a zig test block is elidable");

        assert!(elided.text.contains("pub fn work()"), "{}", elided.text);
        assert!(
            elided.text.contains("test \"work returns seven\" {"),
            "the test's name survives: {}",
            elided.text
        );
        assert!(
            !elided.text.contains("expectEqual"),
            "the body does not: {}",
            elided.text
        );
    }

    #[test]
    fn every_kept_line_is_a_line_of_the_original() {
        // The property that makes this elision rather than truncation or
        // paraphrase: nothing in the answer was invented except the marker.
        let source = "\
pub fn work() -> u32 {
    7
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let one = super::work();
        let two = super::work();
        assert_eq!(one, 7);
        assert_eq!(two, 7);
        assert_eq!(one, two);
    }
}
";
        let elided = elide(Path::new("a.rs"), source).expect("elidable");
        for line in elided.text.lines() {
            assert!(
                source.lines().any(|original| original == line) || line.contains("elided"),
                "invented a line: {line}"
            );
        }
    }

    #[test]
    fn two_test_modules_in_one_file_are_both_elided() {
        // `writing.rs` in this workspace has `mod tests` and `mod writes`.
        let source = "\
pub fn work() {}

#[cfg(test)]
mod tests {
    #[test]
    fn one() {
        let kept = 1;
        let kept = kept + 1;
        let kept = kept + 1;
        assert_eq!(kept, 3);
    }
}

pub fn between() {}

#[cfg(test)]
mod writes {
    #[test]
    fn two() {
        let kept = 2;
        let kept = kept + 2;
        let kept = kept + 2;
        assert_eq!(kept, 6);
    }
}
";
        let elided = elide(Path::new("a.rs"), source).expect("elidable");

        assert!(
            elided.text.contains("pub fn between() {}"),
            "code between two blocks survives: {}",
            elided.text
        );
        assert!(elided.text.contains("fn one()"), "{}", elided.text);
        assert!(elided.text.contains("fn two()"), "{}", elided.text);
        assert!(!elided.text.contains("let kept"), "{}", elided.text);
    }
}
