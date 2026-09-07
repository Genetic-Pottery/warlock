use std::path::Path;

// An inline test region, matched by whole lines at column zero rather than by
// counting braces. A real matcher would have to know each language's strings,
// character literals, raw strings and comment forms, or it counts the `{` in
// `"a { b"` and runs off the end of the file — silently deleting real code. A
// top-level closer sits in column zero in every formatted file and every brace
// inside a string or a nested body is indented, so the anchor answers the same
// question with no lexer. It is wrong only in a file no formatter has seen,
// which is why an unfound closer gives up the elision instead of guessing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Block {
    opener: &'static str,
    // The line *after* the opener must contain this for the region to be real.
    // Rust is why the field exists: `#[cfg(test)]` also sits on `mod stubs;` and
    // on test-only helpers, and eliding from one of those to the next unindented
    // `}` would take a working chunk of the file with it.
    confirms: Option<&'static str>,
    closer: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Language {
    extensions: &'static [&'static str],
    test_suffixes: &'static [&'static str],
    test_prefixes: &'static [&'static str],
    blocks: &'static [Block],
    declarations: &'static [&'static str],
}

impl Language {
    fn is_test_file(&self, name: &str) -> bool {
        self.test_suffixes
            .iter()
            .any(|suffix| name.ends_with(suffix))
            || self
                .test_prefixes
                .iter()
                .any(|prefix| name.starts_with(prefix))
    }

    fn declares(&self, line: &str) -> bool {
        // Both the line as written and the line with its modifiers stripped,
        // because one word is a modifier in one language and the declaration
        // itself in another. `static` is the case that forced this: Java and
        // C# write `public static void main`, so it has to come off before
        // `void main` can be recognised, while Rust's `static TABLE: …` *is*
        // the declaration and stripping it leaves `TABLE: …`, which matches
        // nothing and drops a real constant out of the skeleton.
        //
        // Asking both ways costs one more `starts_with` and can only ever keep
        // a line that would have been dropped, never drop one that would have
        // been kept — the safe direction for a table whose whole job is to
        // leave the file's own declaration lines standing.
        let stripped = without_visibility(line);
        self.declarations
            .iter()
            .any(|prefix| line.starts_with(prefix) || stripped.starts_with(prefix))
    }
}

// Rows to add rather than a language to teach: warlock documents whatever a
// person points it at, so a new language is a new row and never new code. An
// extension no row claims is left completely alone — the file goes to the pass
// whole and the cheaper rungs of `fitting` meet the budget instead. A
// brace-counting fallback applied hopefully to a language nobody described was
// rejected: adding a row can then only make warlock cheaper on that language and
// never wrong on another, which is what lets this table grow one row at a time
// out of real repositories instead of having to be complete before it is useful.
static TABLE: &[Language] = &[
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
            // The item kinds a skeleton used to drop on the floor. A constant
            // is public API a reader looks up by name — warlock's own
            // ENTRY_CHARS, ATTEMPTS and PER_FILE_BYTE_CAP are all consts, and
            // none of them survived into a document before this line — and a
            // trait is the shape of a seam, which is the thing a map is most
            // often asked for. Every other row in this table already carries
            // its language's equivalents: zig has `const`, go has `type`,
            // typescript has `const`, `type` and `interface`. Rust was the
            // one row that did not.
            "const ",
            "static ",
            "trait ",
            "type ",
            "union ",
            "macro_rules!",
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
    // TypeScript and JavaScript. The ESM/CJS extensions are here because a real
    // repository turned out to keep its only browser spec in a `.spec.mjs` — an
    // extension no row claimed, so the file was sent whole and 641 of its
    // neighbours with it.
    //
    // Known limitation, deliberately not fixed here: a test file recognised
    // only by the directory holding it (`__tests__/thing.ts`, the Jest
    // convention) is not matched, because the table asks about names and this
    // is a question about paths. Adding a directory rule with no repository
    // here to check it against would be exactly the untested mechanism the
    // module docs argue against; the cost of missing it is a file sent whole,
    // which is the safe direction.
    Language {
        extensions: &["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"],
        test_suffixes: &[
            ".test.ts",
            ".test.tsx",
            ".test.js",
            ".test.jsx",
            ".spec.ts",
            ".spec.tsx",
            ".spec.js",
            ".spec.jsx",
            ".test.mjs",
            ".spec.mjs",
            ".test.cjs",
            ".spec.cjs",
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

const VISIBILITY: &[&str] = &[
    "pub",
    "pub(crate)",
    "pub(super)",
    "async",
    "unsafe",
    "extern",
    "export",
    "default",
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "final",
    "abstract",
    "override",
    "sealed",
    "open",
];

const KEYWORDS: &[&str] = &[
    "fn",
    "struct",
    "enum",
    "trait",
    "type",
    "mod",
    "impl",
    "class",
    "def",
    "defp",
    "defmodule",
    "func",
    "function",
    "let",
    "var",
    "const",
    "interface",
    "data",
    "object",
    "void",
    "test",
    "describe",
    "it",
    "module",
];

pub(crate) fn declared_names(path: &Path, text: &str) -> Vec<String> {
    const CAP: usize = 64;
    let Some(language) = language_of(path) else {
        return Vec::new();
    };
    // A test file declares tests, and a test's name is a sentence about the
    // code under test rather than a name anyone will look up in this file.
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| language.is_test_file(name))
    {
        return Vec::new();
    }
    let lines: Vec<&str> = text.lines().collect();

    // Public names first, then the rest, each in file order: what a reader
    // opens a file for is usually what it exports, and the rendered list is
    // capped, so the exports are the names that survive the cap.
    let mut public: Vec<String> = Vec::new();
    let mut private: Vec<String> = Vec::new();
    for line in outside_blocks(language, &lines) {
        let trimmed = line.trim_start();
        let rest = without_visibility(trimmed);
        let visible = rest.len() != trimmed.len();
        // An `impl` block names a type declared elsewhere, and a comment or
        // attribute prefix introduces nothing.
        if rest.starts_with(['/', '@', '#'])
            || rest.starts_with("impl ")
            || !language.declares(rest)
        {
            continue;
        }
        let Some(name) = first_identifier(rest) else {
            continue;
        };
        if public.iter().chain(&private).any(|known| known == name) {
            continue;
        }
        if visible {
            public.push(name.to_owned());
        } else {
            private.push(name.to_owned());
        }
    }
    public.extend(private);
    public.truncate(CAP);
    public
}

fn outside_blocks<'a>(language: &Language, lines: &'a [&'a str]) -> Vec<&'a str> {
    let mut outside = Vec::with_capacity(lines.len());
    let mut index = 0usize;
    while index < lines.len() {
        let Some(block) = opens_here(language, lines, index) else {
            outside.push(lines[index]);
            index += 1;
            continue;
        };
        let Some(end) = lines
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, line)| line.trim_end() == block.closer)
            .map(|(at, _)| at)
        else {
            outside.extend(&lines[index..]);
            break;
        };
        index = end + 1;
    }
    outside
}

fn without_visibility(line: &str) -> &str {
    let mut rest = line;
    loop {
        let word = rest.split(char::is_whitespace).next().unwrap_or("");
        if word.is_empty() || !VISIBILITY.contains(&word) {
            return rest;
        }
        rest = rest[word.len()..].trim_start();
    }
}

fn first_identifier(line: &str) -> Option<&str> {
    let separators =
        |c: char| c.is_whitespace() || matches!(c, '(' | '<' | '{' | ':' | '=' | ';' | ',' | '!');
    for word in line.split(separators) {
        if word.is_empty() || KEYWORDS.contains(&word) || VISIBILITY.contains(&word) {
            continue;
        }
        let identifier = word.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_'));
        let starts_like_one = identifier
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_');
        return starts_like_one.then_some(identifier);
    }
    None
}

fn language_of(path: &Path) -> Option<&'static Language> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    TABLE
        .iter()
        .find(|language| language.extensions.contains(&extension.as_str()))
}

pub(crate) fn skeleton(path: &Path, text: &str) -> Option<Elided> {
    let language = language_of(path)?;
    let lines: Vec<&str> = text.lines().collect();
    let kept = keep_declarations_marked(language, &lines, 0, lines.len(), "bodies");

    let before = byte_length(&lines);
    let after = byte_length(&kept.iter().map(String::as_str).collect::<Vec<_>>());
    if after >= before {
        return None;
    }
    Some(Elided {
        dropped: before - after,
        text: kept.join("\n"),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Elided {
    pub(crate) text: String,
    pub(crate) dropped: u64,
}

fn marker_for(lines: usize, dropped: &str) -> String {
    format!("… {lines} lines of {dropped} elided …")
}

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

fn byte_length(lines: &[&str]) -> u64 {
    let content: usize = lines.iter().map(|line| line.len()).sum();
    let separators = lines.len().saturating_sub(1);
    (content + separators) as u64
}

fn keep_declarations(language: &Language, lines: &[&str], from: usize, to: usize) -> Vec<String> {
    keep_declarations_marked(language, lines, from, to, "test bodies")
}
fn keep_declarations_marked(
    language: &Language,
    lines: &[&str],
    from: usize,
    to: usize,
    dropped_are: &str,
) -> Vec<String> {
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
        kept.push(marker_for(dropped, dropped_are));
    }
    kept
}

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

    #[test]
    fn declared_names_are_the_identifiers_on_declaring_lines_and_nothing_else() {
        let text = "\
//! Docs.
use std::fs;

pub struct Manifest {
    entries: Vec<PactEntry>,
}

pub(crate) fn to_manifest_path(root: &Path) -> String {
    let inner = 1;
    fn nested() {}
    inner.to_string()
}

impl Manifest {
    pub fn load() {}
}

pub async fn later() {}
enum State { A, B }
#[test]
fn a_test_name_is_a_declaration_too() {}
";
        assert_eq!(
            super::declared_names(Path::new("manifest.rs"), text),
            [
                "Manifest",
                "to_manifest_path",
                "load",
                "later",
                "nested",
                "State",
                "a_test_name_is_a_declaration_too",
            ],
            "public names first, then the rest, each in file order; `impl Manifest` names \
             nothing new"
        );
    }

    #[test]
    fn a_skeleton_is_every_declaration_line_of_the_file_and_nothing_invented() {
        let source = "\
//! Docs.
use std::fs;

pub struct Manifest {
    entries: Vec<PactEntry>,
}

pub(crate) fn to_manifest_path(root: &Path) -> String {
    let inner = 1;
    inner.to_string()
}

impl Manifest {
    pub fn load() {}
}
";
        let skeleton = super::skeleton(Path::new("manifest.rs"), source).expect("reducible");
        for line in skeleton.text.lines() {
            assert!(
                source.lines().any(|original| original == line) || line.contains("elided"),
                "every line is the file's own: {line}"
            );
        }
        assert!(
            skeleton.text.contains("pub struct Manifest {"),
            "{}",
            skeleton.text
        );
        assert!(
            skeleton
                .text
                .contains("pub(crate) fn to_manifest_path(root: &Path) -> String {"),
            "a signature survives whole, visibility and arguments and all: {}",
            skeleton.text
        );
        assert!(
            !skeleton.text.contains("inner.to_string()"),
            "bodies go: {}",
            skeleton.text
        );
        assert!(skeleton.dropped > 0);
    }

    #[test]
    fn a_file_the_table_does_not_know_has_no_skeleton() {
        assert!(super::skeleton(Path::new("Cargo.lock"), "[[package]]\nname = \"x\"\n").is_none());
        assert!(super::skeleton(Path::new("notes.txt"), "fn looks_like_rust() {}").is_none());
    }

    #[test]
    fn names_inside_a_test_block_and_in_a_test_file_are_not_declarations() {
        let text = "\
pub fn work() {}

#[cfg(test)]
mod tests {
    fn helper() {}

    #[test]
    fn it_works() {}
}

pub fn after() {}
";
        assert_eq!(
            super::declared_names(Path::new("a.rs"), text),
            ["work", "after"]
        );
        assert!(
            super::declared_names(Path::new("a_test.go"), "func TestX(t *testing.T) {}").is_empty()
        );
    }

    #[test]
    fn a_language_the_table_does_not_know_declares_nothing() {
        assert!(
            super::declared_names(Path::new("notes.txt"), "fn looks_like_rust() {}").is_empty()
        );
        assert!(
            super::declared_names(Path::new("Cargo.lock"), "[[package]]\nname = \"x\"").is_empty()
        );
    }

    #[test]
    fn a_rust_skeleton_keeps_constants_traits_and_type_aliases() {
        // The gap that failed a real refresh: `pub const COALESCED_RELOADS`
        // lives in warlock's own watch.rs, a pass named it in a lookup, and
        // the answer was rejected because the skeleton had dropped the line
        // the symbol is on. A constant is public API and a trait is the shape
        // of a seam; a map that cannot name either is worth less than one that
        // can.
        let source = "\
pub const COALESCED_RELOADS: usize = 1;
static TABLE: &[u8] = &[];
pub trait Agent {
    fn run(&self) -> u8;
}
pub type Reply = Result<u8, ()>;
macro_rules! shout {
    () => {};
}
pub fn ordinary() -> u8 {
    let a = 1;
    let b = 2;
    let c = a + b;
    let d = c * 2;
    let e = d - 1;
    e
}
";
        let kept = super::skeleton(Path::new("watch.rs"), source)
            .expect("a rust file is reducible")
            .text;

        for symbol in [
            "COALESCED_RELOADS",
            "TABLE",
            "Agent",
            "Reply",
            "shout",
            "ordinary",
        ] {
            assert!(
                kept.contains(symbol),
                "`{symbol}` has to survive the skeleton, or a lookup naming it \
                 cannot be verified: {kept}",
            );
        }
    }

    #[test]
    fn python_and_go_declarations_come_out_by_their_own_keywords() {
        assert_eq!(
            super::declared_names(
                Path::new("app.py"),
                "@route\ndef handler(req):\n    pass\nclass Store:\n    pass\n"
            ),
            ["handler", "Store"]
        );
        assert_eq!(
            super::declared_names(
                Path::new("main.go"),
                "// Package main.\nfunc main() {}\ntype Ledger struct{}\n"
            ),
            ["main", "Ledger"]
        );
    }

    #[test]
    fn declared_names_are_deduplicated_and_capped() {
        let mut text = "fn same() {}\n".repeat(3);
        for i in 0..100 {
            text.push_str("fn f");
            text.push_str(&i.to_string());
            text.push_str("() {}\n");
        }
        let names = super::declared_names(Path::new("many.rs"), &text);
        assert_eq!(names[0], "same");
        assert_eq!(names.len(), 64);
        assert_eq!(names.iter().filter(|n| *n == "same").count(), 1);
    }
}
