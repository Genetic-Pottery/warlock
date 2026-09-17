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

// Every word a row above can open a declaration with, so that
// `first_identifier` steps over it to the name instead of returning it. A
// declaration prefix missing from here is recorded as the name of the thing it
// declares: Kotlin's `fun sum()` measured a file as declaring `fun`, and a list
// of names is now the only witness a synthesis pass has.
const KEYWORDS: &[&str] = &[
    "fn",
    "fun",
    "union",
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

    // Every name, and no cap on the list. This is two things at once and only
    // one of them is a list somebody reads: `render` prints the first
    // `DECLARED_SHOWN` of it and counts the rest, while `Evidence::knows` asks
    // it whether a name the pass used is real. Truncating here
    // truncated the *evidence*, and a synthesis pass — shown names and sizes,
    // never text — has no other witness to fall back on, so a correct name
    // past the cut was refused four times and its claim dropped from the
    // document. `ui.rs` declares 126 names; `areas` sits at 85.
    //
    // Public names still come first, in file order: that ordering is what
    // decides which sixteen `render` shows, which was always the real reason
    // for it.
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
        let Some(end) = closes_after(lines, block, index) else {
            outside.extend(&lines[index..]);
            break;
        };
        index = end + 1;
    }
    outside
}

// The closer is the next line equal to it at column zero. Searching from the
// line after the opener, so a one-line block cannot close on its own opener.
fn closes_after(lines: &[&str], block: &Block, opener: usize) -> Option<usize> {
    let from = opener + 1;
    lines
        .get(from..)?
        .iter()
        .position(|line| line.trim_end() == block.closer)
        .map(|offset| from + offset)
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

// A Go method declares its receiver before the name it declares — `func (r
// *Cart) Add(…)` — so reading left to right finds `r`, which is a name nobody
// looks anything up by and which stands where `Add` should be. This list is
// what a document prints as `· declares`, which is the part of a file line
// measured to carry a reader to the right symbol, so a receiver there is a
// slot spent on nothing.
//
// A parenthesised group between a declaration keyword and its name is the
// receiver and nothing else — no language in the table above writes anything
// else there — so stepping over one is enough. A group that never closes is
// left alone: `const (` opens a Go block and declares nothing on that line.
fn past_receiver(line: &str) -> &str {
    let Some((_, rest)) = line.split_once(char::is_whitespace) else {
        return line;
    };
    let rest = rest.trim_start();
    if !rest.starts_with('(') {
        return line;
    }
    rest.find(')')
        .map_or(line, |close| rest[close + 1..].trim_start())
}

fn first_identifier(line: &str) -> Option<&str> {
    let line = past_receiver(line);
    let separators =
        |c: char| c.is_whitespace() || matches!(c, '(' | '<' | '{' | ':' | '=' | ';' | ',' | '!');
    let word = line
        .split(separators)
        .find(|word| !word.is_empty() && !KEYWORDS.contains(word) && !VISIBILITY.contains(word))?;

    let identifier = word.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_'));
    let starts_like_one = identifier
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_');
    starts_like_one.then_some(identifier)
}

fn language_of(path: &Path) -> Option<&'static Language> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    TABLE
        .iter()
        .find(|language| language.extensions.contains(&extension.as_str()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Elided {
    pub(crate) text: String,
    pub(crate) dropped: u64,
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
    let after = byte_length(&kept);
    if after >= before {
        return None;
    }
    Some(Elided {
        dropped: before - after,
        text,
    })
}

fn byte_length(lines: &[impl AsRef<str>]) -> u64 {
    let content: usize = lines.iter().map(|line| line.as_ref().len()).sum();
    let separators = lines.len().saturating_sub(1);
    (content + separators) as u64
}

fn keep_declarations(language: &Language, lines: &[&str], from: usize, to: usize) -> Vec<String> {
    let mut kept: Vec<String> = lines[from..to]
        .iter()
        .filter(|line| language.declares(line.trim_start()))
        .map(|line| (*line).to_string())
        .collect();

    let dropped = (to - from) - kept.len();
    if dropped > 0 {
        kept.push(format!("… {dropped} lines of test bodies elided …"));
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

        let end = closes_after(lines, block, index)?;

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
#[path = "tests/languages.rs"]
mod tests;
