use super::{
    Accepted, Defect, Described, ENTRY_CHARS, ENTRY_MINIMUM, Entry, Evidence, Expected,
    FILE_PROMPT, Fill, LIST_CAP, MEND_PASSES, Mend, Mended, PURPOSE_CHARS, STAMP, SYNTHESIS_PROMPT,
    accept_file, accept_synthesis, check, fallback, human, lines_of, mend, mended, names_tool,
    render, stub_answer, synthesis_instructions,
};
use std::collections::{BTreeMap, BTreeSet};

use crate::agent::{ChildDocument, File, Request};

fn request() -> Request {
    Request::new("describe", "/repo/crates/engine")
        .with_files([
            File::present("lib.rs", *b"pub mod pact;\npub fn subtree_hash() {}\n"),
            File::present("Cargo.toml", *b"[package]\nname = \"engine\"\n"),
            File::elided(
                "app.rs",
                900_000,
                "pub fn draw() {}\n… 40000 lines elided …\n",
            ),
            File::omitted("Cargo.lock", 4_200_000),
            File::present("logo.png", vec![0x89, b'P', b'N', b'G', 0xff]),
        ])
        .with_child_documents([ChildDocument::new(
            "src",
            "# src\n\n## Where to look\n\n- hashing → `hash.rs` `subtree_hash`\n",
        )])
}

fn good() -> Fill {
    let mut fill = Fill::stub(&request());
    fill.purpose = "The engine crate: pacts, hashes and the manifest.".to_owned();
    fill.structure = vec![Entry::naming(
        "`lib.rs` re-exports `pact` for the crate.",
        "lib.rs",
    )];
    fill
}

fn defects(fill: &Fill) -> Vec<Defect> {
    let request = request();
    check(fill, &Expected::of(&request), &Described::default())
}

#[test]
fn a_synthesis_answer_that_fills_files_anyway_is_taken_at_its_lines() {
    // The model was asked for everything but the file lines, and answered
    // with them too. They are not a defect and they are not kept: the
    // document's `## Files` is the lines that were assembled and checked
    // one at a time, and an entry nothing asked for has no question behind
    // it to ask again.
    let request = Request::new("synthesise", "/repo/crates/engine")
        .with_files([File::omitted("lib.rs", 38)])
        .with_child_documents([ChildDocument::new("src", "# src\n\nA document below.\n")]);
    let lines = [(
        "lib.rs".to_owned(),
        "The crate root, and what it re-exports.".to_owned(),
    )]
    .into_iter()
    .collect();

    let answer = Fill {
        purpose: "The engine crate: pacts, hashes and the manifest.".to_owned(),
        directories: [(
            "src".to_owned(),
            "Where the hashing and the manifest live.".to_owned(),
        )]
        .into_iter()
        .collect(),
        files: [(
            "lib.rs".to_owned(),
            "a line the synthesis pass was never asked for".to_owned(),
        )]
        .into_iter()
        .collect(),
        ..Fill::default()
    };

    let accepted = accept_synthesis(
        &answer.to_json(),
        &lines,
        &Expected::of(&request),
        &Described::default(),
    );

    let Accepted::Filled(fill) = accepted else {
        panic!("the answer was turned down: {accepted:?}");
    };
    assert_eq!(fill.files, lines, "the assembled lines, not the answer's");
}

#[test]
fn a_stub_is_accepted_for_any_request() {
    // As a pass sees it: the prompt a pass runs under is what tells a
    // stand-in which shape to answer in.
    let request = request();
    let expected = Expected::of(&request);
    let synthesis = request.clone().with_prompt(SYNTHESIS_PROMPT);
    let lines = BTreeMap::new();
    assert!(matches!(
        accept_synthesis(
            &stub_answer(&synthesis),
            &lines,
            &expected,
            &Described::default()
        ),
        Accepted::Filled(_)
    ));

    let one = Request::new(FILE_PROMPT, "/repo/crates/engine")
        .with_files([File::present("lib.rs", *b"pub mod pact;\n")]);
    accept_file(
        &stub_answer(&one),
        "lib.rs",
        &Expected::of(&one),
        &Described::default(),
    )
    .expect("accepted too");

    // Anything that is neither kind of pass gets plain prose.
    assert!(!stub_answer(&request).trim_start().starts_with('{'));
}

#[test]
fn a_good_fill_is_accepted() {
    assert_eq!(defects(&good()), []);
}

#[test]
fn an_answer_wrapped_in_a_fence_or_a_sentence_is_still_read() {
    let request = request();
    let expected = Expected::of(&request);
    let json = good().to_json();
    for wrapped in [
        format!("```json\n{json}\n```"),
        format!("Here is the object:\n\n{json}\n\nLet me know if you need more."),
    ] {
        let accepted =
            accept_synthesis(&wrapped, &BTreeMap::new(), &expected, &Described::default());
        assert!(
            matches!(accepted, Accepted::Filled(_)),
            "read from the outermost braces: {accepted:?}"
        );
    }
}

#[test]
fn an_answer_that_is_not_an_object_is_one_defect() {
    let request = request();
    let expected = Expected::of(&request);
    for answer in [
        "",
        "no.",
        "# engine\n\nProse about the directory.",
        "[1, 2]",
        "{",
    ] {
        let outcome = accept_synthesis(answer, &BTreeMap::new(), &expected, &Described::default());
        assert!(
            matches!(outcome, Accepted::Unparsed(Defect::NotJson { .. })),
            "{answer:?}: {outcome:?}"
        );
    }
}

#[test]
fn a_bare_word_is_a_skipped_slot_and_not_an_entry() {
    // Measured: a pass answered `"duplicate"` for `clock.rs`, one word with
    // no fact in it, and every check of the day passed.
    let request = request();
    assert_eq!(
        accept_file(
            r#"{"line": "duplicate"}"#,
            "lib.rs",
            &Expected::of(&request),
            &Described::default()
        ),
        Err(vec![Defect::TooShort {
            field: "files[\"lib.rs\"]".to_owned(),
            chars: 9,
            minimum: ENTRY_MINIMUM
        }])
    );
}

#[test]
fn every_value_is_one_line_under_its_cap() {
    let mut fill = good();
    fill.purpose = "   ".to_owned();
    fill.directories.insert(
        "src".to_owned(),
        "two\nlines, and long enough besides".to_owned(),
    );
    fill.structure = vec![Entry::naming("x".repeat(ENTRY_CHARS + 1), "lib.rs")];
    let found = defects(&fill);
    assert!(found.contains(&Defect::Empty {
        field: "purpose".to_owned()
    }));
    assert!(found.contains(&Defect::Multiline {
        field: "directories[\"src\"]".to_owned()
    }));
    assert!(found.contains(&Defect::TooLong {
        field: "structure[0]".to_owned(),
        chars: ENTRY_CHARS + 1,
        cap: ENTRY_CHARS
    }));
    assert_eq!(found.len(), 3, "{found:?}");

    let request = request();
    assert_eq!(
        accept_file(
            &format!("{{\"line\": \"{}\"}}", "x".repeat(ENTRY_CHARS + 1)),
            "lib.rs",
            &Expected::of(&request),
            &Described::default()
        ),
        Err(vec![Defect::TooLong {
            field: "files[\"lib.rs\"]".to_owned(),
            chars: ENTRY_CHARS + 1,
            cap: ENTRY_CHARS
        }]),
        "a file's line is held to the same cap by the pass that writes it"
    );
}

#[test]
fn the_purpose_has_its_own_longer_cap() {
    let mut fill = good();
    fill.purpose = "p".repeat(PURPOSE_CHARS);
    assert_eq!(defects(&fill), []);
    fill.purpose.push('p');
    assert_eq!(defects(&fill).len(), 1);
}

#[test]
fn a_list_over_the_cap_is_a_defect_and_so_is_each_bad_entry_in_it() {
    let mut fill = good();
    fill.structure = vec![Entry::naming("an entry long enough to count", "lib.rs"); LIST_CAP + 1];
    fill.structure[3] = Entry::naming(String::new(), "lib.rs");
    assert_eq!(
        defects(&fill),
        [
            Defect::TooMany {
                field: "structure".to_owned(),
                count: LIST_CAP + 1,
                cap: LIST_CAP
            },
            Defect::Empty {
                field: "structure[3]".to_owned()
            }
        ]
    );
}

#[test]
fn every_line_render_writes_is_a_line_lines_of_reads_back() {
    // The round trip the store rests on. If `render` ever writes a file row
    // this cannot read, a run takes that file to be unrecorded and pays for
    // a pass it did not need — which is why this compares against the fill
    // rather than against a string written out by hand here.
    let request = request();
    let expected = Expected::of(&request);
    let described = Described {
        declared: [("app.rs".to_owned(), vec!["draw".to_owned()])]
            .into_iter()
            .collect(),
        ..Described::default()
    };
    let fill = good();

    let read = lines_of(&render("engine", &fill, &expected, &described));

    for (path, entry) in &fill.files {
        assert_eq!(
            read.get(path).map(String::as_str),
            Some(entry.trim()),
            "`{path}` did not survive the page",
        );
    }
    // The two rows no pass wrote — a file that is not text and one that was
    // never sent — are read back as well. They are warlock's own words and
    // it would write them again identically, so there is nothing to gain by
    // telling them apart here.
    assert_eq!(read.len(), expected.files.len(), "{read:?}");
}

#[test]
fn a_row_that_is_not_a_file_line_is_left_out_rather_than_guessed_at() {
    let read = lines_of(
        "\n## Files\n\n\
             - `reading.rs` (40 B) — The reading half.\n\
             - a row with no backticks at all\n\
             - `writing.rs` (16 B)\n\
             - `shared.rs` (2 B) — \n",
    );

    assert_eq!(read.keys().collect::<Vec<_>>(), ["reading.rs"]);
}

#[test]
fn a_claim_naming_something_that_is_not_here_is_a_defect() {
    let mut fill = good();
    fill.structure = vec![Entry {
        line: "`load_tree` walks the repository from the crate root.".to_owned(),
        names: vec!["load_tree".to_owned()],
    }];

    assert_eq!(
        defects(&fill),
        [Defect::UnknownTarget {
            field: "structure[0].names[0]".to_owned(),
            name: "load_tree".to_owned(),
        }],
        "no file in the request holds that name, so the claim is not checkable",
    );
}

#[test]
fn a_claim_names_a_file_a_child_or_a_word_some_file_here_actually_holds() {
    let mut fill = good();
    for name in ["lib.rs", "src", "subtree_hash", "pact"] {
        fill.structure = vec![Entry::naming(
            "a fact about this directory, spelt out",
            name,
        )];
        assert_eq!(defects(&fill), [], "`{name}` is evidenced by the request");
    }
}

#[test]
fn a_name_the_request_cannot_vouch_for_is_taken_from_what_warlock_measured() {
    // A file the request could not carry — too big for the budget, or a
    // pass shown assembled lines rather than source. The request holds the
    // name and none of the text, so the claim's symbol is unfindable there.
    let request = Request::new("describe", "/repo/crates/engine")
        .with_files([File::omitted("huge.rs", 9_000_000)]);
    let expected = Expected::of(&request);
    let mut fill = Fill::stub(&request);
    fill.purpose = "The engine crate, in one line about what it is for.".to_owned();
    fill.structure = vec![Entry {
        line: "`huge.rs` hands every row through `walk_one_deep`.".to_owned(),
        names: vec!["walk_one_deep".to_owned()],
    }];

    assert_eq!(
        check(&fill, &expected, &Described::default()),
        [Defect::UnknownTarget {
            field: "structure[0].names[0]".to_owned(),
            name: "walk_one_deep".to_owned(),
        }],
        "nothing witnesses the name: not the request, and nothing measured",
    );

    let measured = Described {
        declared: [("huge.rs".to_owned(), vec!["walk_one_deep".to_owned()])]
            .into_iter()
            .collect(),
        ..Described::default()
    };
    assert_eq!(
        check(&fill, &expected, &measured),
        [],
        "warlock walked the directory and found the name, which is evidence \
             whatever the request ended up carrying",
    );
}

#[test]
fn a_name_neither_witness_has_ever_seen_is_still_refused() {
    let request = request();
    let expected = Expected::of(&request);
    let measured = Described {
        declared: [("lib.rs".to_owned(), vec!["subtree_hash".to_owned()])]
            .into_iter()
            .collect(),
        ..Described::default()
    };
    let mut fill = good();
    fill.structure = vec![Entry {
        line: "`lib.rs` hands the tree to `load_tree` on the way past.".to_owned(),
        names: vec!["load_tree".to_owned()],
    }];

    assert_eq!(
        check(&fill, &expected, &measured),
        [Defect::UnknownTarget {
            field: "structure[0].names[0]".to_owned(),
            name: "load_tree".to_owned(),
        }],
        "a second witness widens the evidence and does not retire the check",
    );
}

#[test]
fn a_structure_entry_must_name_something() {
    let mut fill = good();
    fill.structure = vec![Entry::of("The files here fit together somehow.")];
    assert_eq!(
        defects(&fill),
        [Defect::Empty {
            field: "structure[0].names".to_owned(),
        }],
        "a statement about how files fit together that names no file is not that statement",
    );
}

#[test]
fn an_entry_answered_as_a_bare_string_is_read_rather_than_refused() {
    let request = request();
    let expected = Expected::of(&request);
    let mut fill = good();
    fill.structure.clear();
    let answer = fill.to_json().replace(
        "\"structure\": []",
        "\"structure\": [\"a line where an object was asked for\"]",
    );

    let accepted = accept_synthesis(&answer, &fill.files, &expected, &Described::default());

    // Not `NotJson`: that is the one defect with nothing to repair from, and
    // spending a whole pass on a model's punctuation is what this avoids.
    assert_eq!(
        accepted,
        Accepted::Defective {
            fill: Fill {
                structure: vec![Entry::of("a line where an object was asked for")],
                ..fill
            },
            defects: vec![Defect::Empty {
                field: "structure[0].names".to_owned(),
            }],
        },
    );
}

#[test]
fn the_names_a_claim_carries_are_checked_and_never_written_out() {
    let request = request();
    let expected = Expected::of(&request);
    let described = Described::default();
    let mut bare = good();
    bare.structure = vec![Entry {
        line: "`lib.rs` re-exports `pact` for the crate.".to_owned(),
        names: vec![
            "lib.rs".to_owned(),
            "pact".to_owned(),
            "subtree_hash".to_owned(),
        ],
    }];

    assert_eq!(
        render("engine", &bare, &expected, &described),
        render("engine", &good(), &expected, &described),
        "the document is the same bytes whatever a claim names: `names` is \
             evidence for the check and never reaches the page",
    );
}

#[test]
fn naming_the_tool_is_a_defect_unless_the_files_name_it() {
    let mut fill = good();
    fill.purpose = "A toy ledger belonging to Warlock.".to_owned();
    fill.structure = vec![Entry::naming(
        "warlock-style grants, one per module",
        "lib.rs",
    )];
    assert_eq!(
        defects(&fill),
        [
            Defect::ToolNamed {
                field: "purpose".to_owned()
            },
            Defect::ToolNamed {
                field: "structure[0]".to_owned()
            },
        ]
    );

    // A child's document always carries the stamp, and the stamp is not the
    // files naming the tool.
    let parent = Request::new("describe", "/repo")
        .with_child_documents([ChildDocument::new("src", format!("{STAMP}\n# src\n"))]);
    let mut fill = Fill::stub(&parent);
    fill.purpose = "Warlock's source, in one crate.".to_owned();
    assert!(matches!(
        check(&fill, &Expected::of(&parent), &Described::default()).as_slice(),
        [Defect::ToolNamed { .. }]
    ));

    // Warlock's own repository names itself, and may say so.
    let own = Request::new("describe", "/repo")
        .with_files([File::present("lib.rs", *b"//! Core engine for warlock.\n")]);
    let mut fill = Fill::stub(&own);
    fill.purpose = "Warlock's engine crate, in one line.".to_owned();
    assert_eq!(check(&fill, &Expected::of(&own), &Described::default()), []);
}

#[test]
fn the_document_is_laid_out_by_warlock_and_not_by_the_pass() {
    let request = request();
    let expected = Expected::of(&request);
    let described = Described {
        declared: [
            (
                "lib.rs".to_owned(),
                vec!["pact".to_owned(), "subtree_hash".to_owned()],
            ),
            (
                "app.rs".to_owned(),
                // Two past the cap, so the line below pins the truncation
                // itself rather than the number it happens to sit at.
                (0..super::DECLARED_SHOWN + 2)
                    .map(|i| format!("draw{i}"))
                    .collect(),
            ),
        ]
        .into_iter()
        .collect(),
        ..Described::default()
    };
    let text = render("engine", &good(), &expected, &described);
    assert!(text.starts_with(STAMP), "{text}");
    assert_eq!(
        text.strip_prefix(STAMP).expect("stamped"),
        "\n# engine\n\n\
             The engine crate: pacts, hashes and the manifest.\n\
             \n## Files\n\n\
             - `Cargo.lock` (4.0 MB) — not read by the pass; name and size only\n\
             - `Cargo.toml` (26 B) — a stand-in entry, filled by a test double\n\
             - `app.rs` (878.9 KB) — a stand-in entry, filled by a test double · declares `draw0`, `draw1`, `draw2`, `draw3`, `draw4`, `draw5`, `draw6`, `draw7`, `draw8`, `draw9`, `draw10`, `draw11`, `draw12`, `draw13`, `draw14`, `draw15` (+2)\n\
             - `lib.rs` (39 B) — a stand-in entry, filled by a test double · declares `pact`, `subtree_hash`\n\
             - `logo.png` (5 B) — not text; name and size only\n\
             \n## Directories\n\n\
             - `src/` — a stand-in entry, filled by a test double\n\
             \n## Structure\n\n\
             - `lib.rs` re-exports `pact` for the crate.\n"
    );
}

#[test]
fn an_empty_list_is_no_section_at_all() {
    let bare = Request::new("describe", "/repo/empty");
    let mut fill = Fill::stub(&bare);
    fill.purpose = "Nothing here yet, but the directory exists.".to_owned();
    let text = render("empty", &fill, &Expected::of(&bare), &Described::default());
    assert_eq!(
        text.strip_prefix(STAMP),
        Some("\n# empty\n\nNothing here yet, but the directory exists.\n")
    );
    for heading in ["## Files", "## Directories", "## Structure", "## Where"] {
        assert!(!text.contains(heading), "{text}");
    }
}

#[test]
fn values_are_trimmed_on_the_way_out_and_not_on_the_way_in() {
    let request = request();
    let mut fill = good();
    fill.directories
        .insert("src".to_owned(), "  padded, but long enough  ".to_owned());
    assert_eq!(defects(&fill), [], "padding is not a defect");
    fill.files.insert(
        "lib.rs".to_owned(),
        "  padded, but long enough  ".to_owned(),
    );
    let text = render(
        "engine",
        &fill,
        &Expected::of(&request),
        &Described::default(),
    );
    assert!(
        text.contains("- `lib.rs` (39 B) — padded, but long enough\n"),
        "{text}"
    );
    assert!(
        text.contains("- `src/` — padded, but long enough\n"),
        "{text}"
    );
}

fn writing(files: [(&str, &str); 2]) -> Described {
    Described {
        tokens: files
            .into_iter()
            .map(|(path, text)| {
                (
                    path.to_owned(),
                    super::identifiers(text).map(str::to_owned).collect(),
                )
            })
            .collect(),
        ..Described::default()
    }
}

#[test]
fn the_road_without_the_text_checks_no_more_weakly_than_the_road_with_it() {
    // The invariant six bugs broke, in one test. Every one of them had the
    // same shape: a question about the *directory* answered from the
    // *request*. Before per-file a request carried every file's text, so
    // `Expected`'s witnesses were complete and the two were the same thing.
    // A synthesis request carries names and sizes, so each of those
    // witnesses silently answers "nothing", and every caller that trusted
    // one changed behaviour without a single test failing.
    //
    // So: the same directory, measured the same way, must reach the same
    // verdict whether or not the text went over. A new witness-shaped
    // question on `Expected` that forgets `Described` fails here.
    // `subtree_hash` is declared; `Digest` and `finalize` are only
    // *written*, which is the Java-method case — a prefix table reads no
    // declaration for them, so only the tokens witness them. Both kinds are
    // named below, so reverting either witness fails this test.
    let source = "pub fn subtree_hash() -> Digest {\n    hasher.finalize()\n}\n\
                      // Warlock grants a pact over a directory.\n";
    let described = Described {
        declared: [("hash.rs".to_owned(), vec!["subtree_hash".to_owned()])]
            .into_iter()
            .collect(),
        tokens: [(
            "hash.rs".to_owned(),
            super::identifiers(source).map(str::to_owned).collect(),
        )]
        .into_iter()
        .collect(),
    };

    let with_text = Request::new("describe", "/repo/crates/warlock-engine/src")
        .with_files([File::present("hash.rs", source.as_bytes().to_vec())]);
    let without = Request::new("synthesise", "/repo/crates/warlock-engine/src")
        .with_files([File::omitted("hash.rs", source.len() as u64)]);

    // The file lines are the one honest difference: the road with the text
    // is asked for them, the road without has them already. Everything
    // below the `## Files` line is what has to agree.
    let directory_wide = Fill {
        purpose: "Hashing for warlock's pacts: one digest per subtree.".to_owned(),
        structure: vec![
            Entry::naming(
                "`subtree_hash` digests everything at and below a directory.",
                "subtree_hash",
            ),
            Entry::naming("The digest comes back as a `Digest`.", "Digest"),
            Entry::naming("A digest is closed out by `finalize`.", "finalize"),
        ],
        ..Fill::default()
    };
    let lined = Fill {
        files: [(
            "hash.rs".to_owned(),
            "Digests a directory and everything below it.".to_owned(),
        )]
        .into_iter()
        .collect(),
        ..directory_wide.clone()
    };

    assert_eq!(
        check(&lined, &Expected::of(&with_text), &described),
        [],
        "the road with the text turned down a true claim"
    );
    assert_eq!(
        check(&directory_wide, &Expected::of(&without), &described),
        [],
        "the road without the text is weaker: a name, a symbol, or the \
             tool's own name went unwitnessed"
    );

    // And neither road is merely permissive: a name the directory does not
    // hold is refused on both.
    let invented = Fill {
        structure: vec![Entry::naming(
            "`load_tree` reads the repository.",
            "load_tree",
        )],
        ..directory_wide
    };
    let refused = [Defect::UnknownTarget {
        field: "structure[0].names[0]".to_owned(),
        name: "load_tree".to_owned(),
    }];
    assert_eq!(
        check(&invented, &Expected::of(&without), &described),
        refused
    );
    assert_eq!(
        check(
            &Fill {
                files: lined.files.clone(),
                ..invented
            },
            &Expected::of(&with_text),
            &described
        ),
        refused,
    );
}

#[test]
fn a_leaf_that_writes_the_tools_name_may_say_so() {
    // Found on warlock's own repository. `Expected::mentions_tool` reads
    // `Shown::Text`, and a synthesis request carries no text — so a leaf,
    // having no child document either, always read as never mentioning
    // warlock and had every honest claim about it refused. The two leaf
    // `src` directories lost nine claims between them; their parents, which
    // do have child documents, lost none.
    let request = Request::new("synthesise", "/repo/crates/warlock-engine/src")
        .with_files([File::omitted("pact.rs", 900)]);
    let expected = Expected::of(&request);
    assert!(
        !Evidence::new(&expected, &Described::default()).mentions_tool(),
        "the request alone cannot know, which is the whole problem"
    );

    let described = writing([
        (
            "pact.rs",
            "// Warlock grants a pact over a directory.\npub fn pact_subtree() {}",
        ),
        ("hash.rs", "pub fn subtree_hash() {}"),
    ]);
    let fill = Fill {
        purpose: "Pacts and refreshes a subtree, recording what warlock granted.".to_owned(),
        ..Fill::default()
    };

    assert_eq!(
        check(&fill, &expected, &described),
        [],
        "a directory whose own files write the name may use it"
    );
    assert_eq!(
        check(&fill, &expected, &Described::default()),
        [Defect::ToolNamed {
            field: "purpose".to_owned(),
        }],
        "and one whose files never mention it still may not",
    );
}

#[test]
fn a_name_a_file_writes_witnesses_a_claim_the_table_cannot_read() {
    // The case the token witness exists for. A prefix table reads
    // `public class Invoice` and stops: strip the visibility off a Java
    // method and `long total()` opens with a type, which matches no row. So
    // `declared` holds only `Invoice`, and on the per-file road — where no
    // file's text is in the request — a true claim about `total` had no
    // witness at all and was dropped out of the document.
    let request = Request::new("synthesise", "/repo/services/billing")
        .with_files([File::omitted("Invoice.java", 400)]);
    let expected = Expected::of(&request);
    let described = writing([
        (
            "Invoice.java",
            "public class Invoice { public long total() {} }",
        ),
        ("Ledger.kt", "class Ledger { fun sum(): Long = 0 }"),
    ]);

    let fill = Fill {
        purpose: "Billing types: invoices, ledgers and what they total.".to_owned(),
        structure: vec![Entry::naming(
            "`total` sums the lines of an invoice.",
            "total",
        )],
        ..Fill::default()
    };

    assert_eq!(
        check(&fill, &expected, &described),
        [],
        "a name the file plainly writes was refused"
    );
    assert_eq!(
        check(&fill, &expected, &Described::default()),
        [Defect::UnknownTarget {
            field: "structure[0].names[0]".to_owned(),
            name: "total".to_owned(),
        }],
        "and without the measurement there is nothing to witness it",
    );
}

#[test]
fn a_name_merely_contained_in_a_longer_one_is_not_witnessed() {
    // Stricter than the witness it replaces, on purpose. The old road
    // accepted any name a file's text *contained*, so a file full of
    // `AuditApplyer` witnessed a claim about `Apply` — a symbol that is not
    // there.
    let request =
        Request::new("synthesise", "/repo/monolith").with_files([File::omitted("audit.go", 900)]);
    let described = writing([
        ("audit.go", "type AuditApplyer struct{}"),
        ("other.go", "package monolith"),
    ]);
    let fill = Fill {
        purpose: "Audit persistence wrappers, one per verb, over a shared store.".to_owned(),
        structure: vec![Entry::naming("`Apply` stamps and saves.", "Apply")],
        ..Fill::default()
    };

    assert_eq!(
        check(&fill, &Expected::of(&request), &described),
        [Defect::UnknownTarget {
            field: "structure[0].names[0]".to_owned(),
            name: "Apply".to_owned(),
        }]
    );
}

#[test]
fn a_qualified_name_is_met_by_a_file_writing_both_halves() {
    // Elixir and the like name a module and a function together. Cutting
    // the claim into identifiers the same way the file was cut is what lets
    // `Pipeline.Stage` be witnessed without loosening the check back into
    // substring containment.
    let request =
        Request::new("synthesise", "/repo/pipeline").with_files([File::omitted("stage.ex", 300)]);
    let described = writing([
        (
            "stage.ex",
            "defmodule Pipeline.Stage do\n  def sum(items), do: items\nend",
        ),
        ("stage_test.exs", "defmodule Pipeline.StageTest do\nend"),
    ]);
    let fill = Fill {
        purpose: "The pipeline stage that sums the items handed to it.".to_owned(),
        structure: vec![Entry::naming(
            "`Pipeline.Stage` sums the items it is given.",
            "Pipeline.Stage",
        )],
        ..Fill::default()
    };

    assert_eq!(check(&fill, &Expected::of(&request), &described), []);
}

#[test]
fn a_mend_keeps_the_lines_the_run_paid_for() {
    // The synthesis road, which is the only road: every file is in the
    // request by name and size, none by text, so `Expected::asked` is empty
    // and every assembled line is a key the pass was never asked about.
    // They still have to come out the other side — they are the document's
    // whole `## Files`, and each was settled by `accept_file` already.
    let request = Request::new("synthesise", "/repo/crates/engine")
        .with_files([File::omitted("lib.rs", 38), File::omitted("pact.rs", 90)])
        .with_child_documents([ChildDocument::new("src", "# src\n\nThe code.\n")]);
    let lines: BTreeMap<String, String> = [
        (
            "lib.rs".to_owned(),
            "The crate root and what it re-exports.".to_owned(),
        ),
        (
            "pact.rs".to_owned(),
            "Granting, refreshing and un-pacting a subtree.".to_owned(),
        ),
    ]
    .into_iter()
    .collect();

    // An answer bad enough to reach the mend: the purpose is too short, so
    // the attempts run out and `synthesise` falls through to here.
    let unusable = Fill {
        purpose: "Too short.".to_owned(),
        files: lines.clone(),
        ..Fill::default()
    };

    let (repaired, mends) = mend(&unusable, &Expected::of(&request), &Described::default());

    assert!(!mends.is_empty(), "the mend this is about did not happen");
    assert_eq!(
        repaired.files, lines,
        "the mend dropped the lines the run paid for"
    );
}

#[test]
fn a_refused_name_is_told_that_a_declared_name_would_have_done() {
    // The sentence is the only part a retry can act on. A claim naming a
    // symbol is told symbols are allowed and this is not one of them:
    // telling it the name must be "a file or subdirectory" sends the next
    // pass looking for the wrong thing.
    assert_eq!(
        Defect::UnknownTarget {
            field: "structure[0].names[8]".to_owned(),
            name: "areas".to_owned(),
        }
        .to_string(),
        "structure[0].names[8] names `areas`, which is not a file, a subdirectory, \
             or a name declared in one of them"
    );
}

#[test]
fn the_synthesis_is_shown_the_child_it_must_key_by_and_is_never_shown_files() {
    // The shape the pass is told to return carries the children by name, so
    // there is a key to fill in rather than one to invent from the prose —
    // which spells a directory `crates/`, the way a rendered document does.
    // And no `files` slot at all: this pass is not asked about them.
    let request = Request::new("synthesise", "/repo")
        .with_child_documents([ChildDocument::new("crates", "# crates\n\nThe code.\n")]);
    let lines = [(
        "Cargo.toml".to_owned(),
        "The workspace manifest.".to_owned(),
    )]
    .into_iter()
    .collect();

    let text = synthesis_instructions("repo", &lines, &Expected::of(&request), &[]);

    assert!(
        text.ends_with(
            "{\"purpose\":\"\",\"directories\":{\"crates\":\"\"},\
                 \"structure\":[]}"
        ),
        "the shape is last and carries the child: {text}"
    );
    assert!(
        !text.contains("\"files\""),
        "a slot nobody asked about: {text}"
    );
}

#[test]
fn a_child_keyed_the_way_a_document_renders_it_is_corrected_and_kept() {
    // `render` writes a child as `crates/`, and every synthesis pass is
    // shown its children's documents — so the trailing slash comes back. It
    // used to reach `keyed` as a key nothing asked for, which is a panic in
    // a debug build and a dropped `## Directories` line in any build.
    let request = Request::new("synthesise", "/repo")
        .with_child_documents([ChildDocument::new("crates", "# crates\n\nThe code.\n")]);
    let answer = Fill {
        purpose: "The workspace root: two crates and the files that wire them.".to_owned(),
        directories: [(
            "crates/".to_owned(),
            "Both workspace members live here.".to_owned(),
        )]
        .into_iter()
        .collect(),
        ..Fill::default()
    };

    let accepted = accept_synthesis(
        &answer.to_json(),
        &BTreeMap::new(),
        &Expected::of(&request),
        &Described::default(),
    );

    let Accepted::Filled(fill) = accepted else {
        panic!("the answer was turned down: {accepted:?}");
    };
    assert_eq!(
        fill.directories.keys().collect::<Vec<_>>(),
        ["crates"],
        "the slash is corrected rather than the line dropped"
    );
}

#[test]
fn a_child_nothing_asked_about_is_dropped_and_not_faulted() {
    let request = Request::new("synthesise", "/repo")
        .with_child_documents([ChildDocument::new("crates", "# crates\n\nThe code.\n")]);
    let answer = Fill {
        purpose: "The workspace root: two crates and the files that wire them.".to_owned(),
        directories: [
            (
                "crates".to_owned(),
                "Both workspace members live here.".to_owned(),
            ),
            (
                "docs".to_owned(),
                "A directory with no document of its own.".to_owned(),
            ),
        ]
        .into_iter()
        .collect(),
        ..Fill::default()
    };

    let accepted = accept_synthesis(
        &answer.to_json(),
        &BTreeMap::new(),
        &Expected::of(&request),
        &Described::default(),
    );

    let Accepted::Filled(fill) = accepted else {
        panic!("an unasked child was faulted rather than dropped: {accepted:?}");
    };
    assert_eq!(fill.directories.keys().collect::<Vec<_>>(), ["crates"]);
}

#[test]
fn the_prompt_says_warlock_is_the_tool_and_not_the_project() {
    // Measured on a scratch crate that never mentions warlock: two of its
    // three documents called it "a toy freshness ledger belonging to
    // Warlock", because the instructions name warlock and ask for the
    // product the directory belongs to.
    for prompt in [SYNTHESIS_PROMPT, FILE_PROMPT] {
        assert!(
            prompt.contains("it is not the project being described"),
            "{prompt}"
        );
    }
    assert!(
        SYNTHESIS_PROMPT
            .contains("its name belongs in no value unless the files themselves use it"),
        "{SYNTHESIS_PROMPT}"
    );
    assert!(
        FILE_PROMPT.contains("its name belongs in the line only if the file itself uses it"),
        "{FILE_PROMPT}"
    );
}

#[test]
fn every_defect_reads_as_one_line_naming_its_slot() {
    let all = [
        Defect::NotJson {
            detail: "expected value at line 1".to_owned(),
        },
        Defect::Missing {
            field: "files[\"a\"]".to_owned(),
        },
        Defect::Empty {
            field: "purpose".to_owned(),
        },
        Defect::Multiline {
            field: "structure[0]".to_owned(),
        },
        Defect::TooShort {
            field: "files[\"c\"]".to_owned(),
            chars: 3,
            minimum: ENTRY_MINIMUM,
        },
        Defect::TooLong {
            field: "structure[1]".to_owned(),
            chars: 300,
            cap: ENTRY_CHARS,
        },
        Defect::TooMany {
            field: "structure".to_owned(),
            count: 20,
            cap: LIST_CAP,
        },
        Defect::UnknownTarget {
            field: "structure[0].names[0]".to_owned(),
            name: "x".to_owned(),
        },
        Defect::ToolNamed {
            field: "purpose".to_owned(),
        },
    ];
    for defect in all {
        let text = defect.to_string();
        assert!(!text.contains('\n'), "{text}");
        assert!(!text.is_empty());
    }
}

#[test]
fn sizes_read_the_way_a_person_reads_them() {
    assert_eq!(human(0), "0 B");
    assert_eq!(human(1023), "1023 B");
    assert_eq!(human(1024), "1.0 KB");
    assert_eq!(human(22_938), "22.4 KB");
    assert_eq!(human(1024 * 1024), "1.0 MB");
    assert_eq!(human(4_200_000), "4.0 MB");
}

#[test]
fn a_described_record_is_only_what_warlock_measured() {
    let described = Described {
        declared: [("a.rs".to_owned(), vec!["one".to_owned()])]
            .into_iter()
            .collect(),
        ..Described::default()
    };
    assert_eq!(described.declared["a.rs"], ["one"]);
    assert_eq!(Described::default().declared, BTreeMap::new());
}

fn declared() -> Described {
    Described {
        declared: [
            (
                "lib.rs".to_owned(),
                vec!["pact".to_owned(), "subtree_hash".to_owned()],
            ),
            (
                "app.rs".to_owned(),
                (0..20).map(|i| format!("draw{i}")).collect(),
            ),
        ]
        .into_iter()
        .collect(),
        ..Described::default()
    }
}

#[track_caller]
fn holds_the_shape(line: &str) {
    assert!(!line.contains(['\n', '\r']), "more than one line: {line:?}");
    let chars = line.chars().count();
    assert!(
        (ENTRY_MINIMUM..=ENTRY_CHARS).contains(&chars),
        "{chars} characters: {line:?}"
    );
}

#[test]
fn a_fallback_line_is_the_name_the_size_and_the_symbols() {
    let request = request();
    let expected = Expected::of(&request);
    let described = declared();

    assert_eq!(
        fallback::file("lib.rs", &expected, &described),
        "A 39 B file named `lib.rs`, declaring `pact`, `subtree_hash`."
    );
    // A file warlock extracted nothing from still gets its name and size,
    // and says only that and that it found no names.
    assert_eq!(
        fallback::file("Cargo.toml", &expected, &described),
        "A 26 B file named `Cargo.toml`, with no symbols extracted from it."
    );
    // The purpose is the directory's name and what is in it: five files,
    // one child. No sentence about what any of it is for.
    assert_eq!(
        fallback::purpose("engine", &expected, &Described::default()),
        "`engine` holds 5 files and 1 subdirectory."
    );
    assert_eq!(
        fallback::directory("src", &expected, &Described::default()),
        "A subdirectory named `src`."
    );

    let bare = Request::new("describe", "/repo/empty");
    assert_eq!(
        fallback::purpose("empty", &Expected::of(&bare), &Described::default()),
        "`empty` holds no files and no subdirectories."
    );
}

#[test]
fn every_fallback_line_holds_one_line_and_both_caps() {
    let request = request();
    let expected = Expected::of(&request);
    let described = declared();

    for path in [
        "lib.rs",
        "Cargo.toml",
        "app.rs",
        "Cargo.lock",
        "logo.png",
        // A name the request does not carry: no size to state, and the
        // line is still a line.
        "not-here.rs",
        "a",
        "",
    ] {
        holds_the_shape(&fallback::file(path, &expected, &described));
    }
    for name in ["engine", "a", ""] {
        holds_the_shape(&fallback::purpose(name, &expected, &Described::default()));
        holds_the_shape(&fallback::directory(name, &expected, &Described::default()));
    }

    // The symbol list stops at the cap rather than being cut through a
    // name: twenty long declarations do not run the line over.
    let long = Described {
        declared: [(
            "lib.rs".to_owned(),
            (0..20).map(|i| format!("a_long_symbol_name_{i}")).collect(),
        )]
        .into_iter()
        .collect(),
        ..Described::default()
    };
    let line = fallback::file("lib.rs", &expected, &long);
    holds_the_shape(&line);
    assert!(line.ends_with("`."), "cut between names: {line:?}");
}

#[test]
fn short_facts_are_padded_and_a_long_line_is_cut_to_the_cap() {
    let padded = fallback::fit("`a`, 5 B.", "A file in this directory.");
    holds_the_shape(&padded);
    assert!(padded.starts_with("`a`, 5 B."), "{padded}");

    let cut = fallback::fit(&"x".repeat(ENTRY_CHARS + 50), "pad");
    assert_eq!(cut.chars().count(), ENTRY_CHARS);

    // Whatever the facts, the value is one line.
    assert_eq!(
        fallback::fit("a name\nover\ttwo lines and some", "pad"),
        "a name over two lines and some"
    );
}

#[test]
fn a_name_warlock_shares_does_not_trip_the_tool_rule() {
    // The rule is a substring test, so a line naming `warlock.rs` cannot be
    // quoted out of it: while the rule is live the name is left out
    // instead, and `render` prints the path either way.
    let request = Request::new("describe", "/repo/crates").with_files([File::present(
        "warlock.rs",
        *b"pub fn draw() {}\npub fn run() {}\n",
    )]);
    let expected = Expected::of(&request);
    assert!(
        !Evidence::new(&expected, &Described::default()).mentions_tool(),
        "the rule is live here"
    );
    let described = Described {
        declared: [(
            "warlock.rs".to_owned(),
            vec!["draw".to_owned(), "warlock_run".to_owned()],
        )]
        .into_iter()
        .collect(),
        ..Described::default()
    };

    let line = fallback::file("warlock.rs", &expected, &described);
    assert!(!names_tool(&line), "{line}");
    assert!(line.contains("`draw`"), "the other symbols stay: {line}");
    holds_the_shape(&line);
    for named in [
        fallback::purpose("warlock", &expected, &Described::default()),
        fallback::directory("warlock-tui", &expected, &Described::default()),
    ] {
        assert!(!names_tool(&named), "{named}");
        holds_the_shape(&named);
    }

    // And the rule the caller applies cannot fire on a fill built of them.
    let fill = Fill {
        purpose: fallback::purpose("crates", &expected, &Described::default()),
        files: [(
            "warlock.rs".to_owned(),
            fallback::file("warlock.rs", &expected, &described),
        )]
        .into_iter()
        .collect(),
        ..Fill::default()
    };
    assert_eq!(check(&fill, &expected, &Described::default()), []);

    // Where the files use the word the rule has stood down, and the facts
    // go in whole.
    let own = Request::new("describe", "/repo")
        .with_files([File::present("warlock.rs", *b"//! The warlock engine.\n")]);
    let own = Expected::of(&own);
    assert!(Evidence::new(&own, &Described::default()).mentions_tool());
    let line = fallback::file("warlock.rs", &own, &described);
    assert!(line.contains("`warlock.rs`"), "{line}");
    assert!(line.contains("`warlock_run`"), "{line}");
    assert!(
        fallback::purpose("warlock", &own, &Described::default()).contains("`warlock`"),
        "the directory names itself too"
    );
}

#[test]
fn no_fallback_line_speaks_in_the_stubs_words() {
    let request = request();
    let expected = Expected::of(&request);
    let described = declared();
    let lines = [
        fallback::file("lib.rs", &expected, &described),
        fallback::file("logo.png", &expected, &described),
        fallback::purpose("engine", &expected, &Described::default()),
        fallback::directory("src", &expected, &Described::default()),
    ];
    // `Fill::stub` is a test double's wording; a repaired document says
    // what warlock measured, and never that.
    for line in &lines {
        let lowered = line.to_ascii_lowercase();
        for wording in ["a stand-in entry", "stand-in", "test double"] {
            assert!(!lowered.contains(wording), "{wording:?} in {line:?}");
        }
    }
}

// The mend over the fixture directory, asserting the property every one of
// these tests shares: what comes back is not itself defective.
#[track_caller]
fn mend_of(fill: &Fill) -> (Fill, Vec<Mend>) {
    let request = request();
    let expected = Expected::of(&request);
    let (repaired, mends) = mend(fill, &expected, &declared());
    assert_eq!(
        check(&repaired, &expected, &Described::default()),
        [],
        "a mended fill is not itself defective: {mends:?}"
    );
    (repaired, mends)
}

#[test]
fn an_empty_value_falls_back_to_what_warlock_measured() {
    let request = request();
    let expected = Expected::of(&request);
    let mut fill = good();
    fill.purpose = "   ".to_owned();
    fill.directories.insert("src".to_owned(), String::new());
    let (repaired, mends) = mend_of(&fill);
    assert_eq!(
        repaired.purpose,
        fallback::purpose("engine", &expected, &Described::default())
    );
    assert_eq!(
        repaired.directories["src"],
        fallback::directory("src", &expected, &Described::default())
    );
    assert_eq!(
        mends,
        [
            Mend {
                field: "purpose".to_owned(),
                done: Mended::Supplied
            },
            Mend {
                field: "directories[\"src\"]".to_owned(),
                done: Mended::Supplied
            },
        ]
    );
}

#[test]
fn a_value_under_the_floor_falls_back_to_what_warlock_measured() {
    let request = request();
    let mut fill = good();
    fill.directories
        .insert("src".to_owned(), "duplicate".to_owned());
    let (repaired, mends) = mend_of(&fill);
    assert_eq!(
        repaired.directories["src"],
        fallback::directory("src", &Expected::of(&request), &Described::default())
    );
    assert_eq!(mends.first().map(|mend| mend.done), Some(Mended::Supplied));
    assert_eq!(mends.len(), 1, "{mends:?}");
}

#[test]
fn a_value_over_more_than_one_line_keeps_the_first() {
    let mut fill = good();
    fill.directories.insert(
        "src".to_owned(),
        "\nthe source tree, in one line\nand a second the cap would have allowed".to_owned(),
    );
    let (repaired, mends) = mend_of(&fill);
    assert_eq!(repaired.directories["src"], "the source tree, in one line");
    assert_eq!(
        mends,
        [Mend {
            field: "directories[\"src\"]".to_owned(),
            done: Mended::FirstLine
        }]
    );
    assert_eq!(
        mends[0].to_string(),
        "directories[\"src\"] ran to more than one line and keeps its first"
    );
}

#[test]
fn a_value_over_its_cap_is_cut_to_it_counting_characters() {
    let mut fill = good();
    fill.directories
        .insert("src".to_owned(), "x".repeat(ENTRY_CHARS + 20));
    fill.purpose = "é".repeat(PURPOSE_CHARS + 1);
    let (repaired, mends) = mend_of(&fill);
    assert_eq!(repaired.directories["src"].chars().count(), ENTRY_CHARS);
    // Characters, and a character boundary: the purpose is multibyte, so a
    // byte cut would either panic or land inside an `é`.
    assert_eq!(repaired.purpose.chars().count(), PURPOSE_CHARS);
    assert_eq!(repaired.purpose.len(), PURPOSE_CHARS * 2);
    assert_eq!(
        mends,
        [
            Mend {
                field: "purpose".to_owned(),
                done: Mended::Cut {
                    from: PURPOSE_CHARS + 1,
                    to: PURPOSE_CHARS
                }
            },
            Mend {
                field: "directories[\"src\"]".to_owned(),
                done: Mended::Cut {
                    from: ENTRY_CHARS + 20,
                    to: ENTRY_CHARS
                }
            },
        ]
    );
    assert_eq!(
        mends[1].to_string(),
        "directories[\"src\"] was 300 characters and was cut to 280",
        "the line a run reports, from the brief"
    );
}

#[test]
fn a_list_over_its_cap_keeps_its_first_entries() {
    let mut fill = good();
    fill.structure = (0..LIST_CAP + 2)
        .map(|index| {
            Entry::naming(
                format!("an entry long enough to count, the {index}th"),
                "lib.rs",
            )
        })
        .collect();
    let (repaired, mends) = mend_of(&fill);
    assert_eq!(repaired.structure.len(), LIST_CAP);
    assert_eq!(repaired.structure, fill.structure[..LIST_CAP]);
    assert_eq!(
        mends,
        [Mend {
            field: "structure".to_owned(),
            done: Mended::Shortened {
                from: LIST_CAP + 2,
                to: LIST_CAP
            }
        }]
    );
}

#[test]
fn a_value_naming_the_tool_is_dropped_and_falls_to_the_next_rule() {
    let request = request();
    let expected = Expected::of(&request);
    let mut fill = good();
    fill.directories.insert(
        "src".to_owned(),
        "the source of the warlock engine, and long enough besides".to_owned(),
    );
    fill.structure = vec![Entry::naming(
        "warlock-style grants, one per module and then some",
        "lib.rs",
    )];
    // The purpose is the one slot with no next rule under it: a document
    // without one is not a document, so it falls straight to the fallback.
    fill.purpose = "A toy freshness ledger belonging to Warlock.".to_owned();
    let (repaired, mends) = mend_of(&fill);
    assert_eq!(
        repaired.directories["src"],
        fallback::directory("src", &expected, &Described::default())
    );
    assert_eq!(
        repaired.purpose,
        fallback::purpose("engine", &expected, &Described::default())
    );
    assert!(
        repaired.structure.is_empty(),
        "a list entry has no fallback"
    );
    assert_eq!(
        mends,
        [
            Mend {
                field: "purpose".to_owned(),
                done: Mended::Supplied
            },
            Mend {
                field: "directories[\"src\"]".to_owned(),
                done: Mended::Dropped
            },
            Mend {
                field: "structure[0]".to_owned(),
                done: Mended::Dropped
            },
            Mend {
                field: "directories[\"src\"]".to_owned(),
                done: Mended::Supplied
            },
        ],
        "the drop and the fall are two records, in the order they happened"
    );
}

#[test]
fn an_entry_the_request_never_asked_for_is_gone_before_the_first_check() {
    let mut fill = good();
    fill.directories.insert(
        "target".to_owned(),
        "the build directory, unasked for".to_owned(),
    );
    // Not a mend: nothing was repaired, an answer to a question nobody
    // asked was thrown away. Were it left in, `keyed`'s debug assertion
    // would fire on the mend's own first `check`.
    let (repaired, mends) = mend_of(&fill);
    assert!(!repaired.directories.contains_key("target"));
    assert_eq!(
        repaired.directories,
        good().directories,
        "the asked-for entries stay"
    );
    assert_eq!(mends, []);
}

#[test]
fn the_mend_settles_inside_its_bound_and_two_rules_cannot_spin() {
    const {
        assert!(
            MEND_PASSES >= 3,
            "the longest chain is drop, fall back, check clean"
        );
    }
    let request = request();
    let expected = Expected::of(&request);

    // The longest chain a rule here can start: a value both over the cap
    // and naming the tool. The drop takes it, the fallback answers the gap
    // the drop left, and the third look finds nothing — the two rules do
    // not hand the slot back and forth.
    let mut fill = good();
    fill.directories
        .insert("src".to_owned(), format!("warlock{}", "x".repeat(400)));
    let (repaired, mends, passes) = mended(&fill, &expected, &declared());
    assert_eq!(check(&repaired, &expected, &Described::default()), []);
    assert_eq!(
        mends.iter().map(|mend| mend.done).collect::<Vec<_>>(),
        [Mended::Dropped, Mended::Supplied],
        "twice over the same slot and then done: {mends:?}"
    );
    assert_eq!(passes, 2);
    assert!(
        passes < MEND_PASSES,
        "the bound is a stop, not a schedule: {passes} of {MEND_PASSES}"
    );

    // Every rule at once, on every slot, still settles inside the bound.
    let mut fill = good();
    fill.purpose = "Warlock's own\nledger.".to_owned();
    fill.directories.insert("src".to_owned(), String::new());
    fill.structure = vec![Entry::of(String::new()); LIST_CAP + 3];
    fill.structure.push(Entry::naming(
        "a claim about a name that is not here",
        "missing",
    ));
    let (repaired, _, passes) = mended(&fill, &expected, &declared());
    assert_eq!(check(&repaired, &expected, &Described::default()), []);
    assert!(passes <= MEND_PASSES, "{passes}");
}

fn variant(defect: &Defect) -> &'static str {
    match defect {
        Defect::NotJson { .. } => "NotJson",
        Defect::Missing { .. } => "Missing",
        Defect::Empty { .. } => "Empty",
        Defect::Multiline { .. } => "Multiline",
        Defect::TooShort { .. } => "TooShort",
        Defect::TooLong { .. } => "TooLong",
        Defect::TooMany { .. } => "TooMany",
        Defect::UnknownTarget { .. } => "UnknownTarget",
        Defect::ToolNamed { .. } => "ToolNamed",
    }
}

type Mutation = (&'static str, fn(&mut Fill));

// One per repairable defect, and a few that collide on purpose: two
// mutations over the same slot are how a repair comes to answer a slot
// another repair already moved.
fn mutations() -> [Mutation; 11] {
    [
        ("Missing", |fill| {
            fill.directories.remove("src");
        }),
        ("Empty", |fill| fill.purpose = "  ".to_owned()),
        ("Empty", |fill| {
            fill.structure.push(Entry::naming("   ", "lib.rs"));
        }),
        ("Multiline", |fill| {
            fill.directories.insert(
                "src".to_owned(),
                "the source, and long enough\nand a second line".to_owned(),
            );
        }),
        ("TooShort", |fill| {
            fill.purpose = "duplicate".to_owned();
        }),
        ("TooLong", |fill| {
            fill.structure
                .push(Entry::naming("x".repeat(ENTRY_CHARS + 40), "lib.rs"));
        }),
        ("TooLong", |fill| {
            fill.purpose = "é".repeat(PURPOSE_CHARS + 9);
        }),
        ("TooMany", |fill| {
            fill.structure =
                vec![Entry::naming("an entry long enough to count", "lib.rs"); LIST_CAP + 2];
        }),
        ("UnknownTarget", |fill| {
            fill.structure.push(Entry::naming(
                "`load_tree` walks the repository from the crate root.",
                "load_tree",
            ));
        }),
        ("ToolNamed", |fill| {
            fill.structure.push(Entry::naming(
                "the crate root of the warlock engine, and long enough",
                "lib.rs",
            ));
        }),
        ("ToolNamed", |fill| {
            fill.directories.insert(
                "src".to_owned(),
                "warlock's own source, in one line".to_owned(),
            );
        }),
    ]
}

#[test]
fn a_mended_fill_is_never_defective_whatever_was_wrong_with_it() {
    let mutations = mutations();
    let request = request();
    let expected = Expected::of(&request);
    let described = declared();
    let mut covered: BTreeSet<&'static str> = BTreeSet::new();
    // A fixed seed and a plain congruential generator: this crate takes no
    // dependency for a coin toss, and a property test that cannot be
    // reproduced from its own source is not much of one.
    let mut state: u64 = 0x5eed_1234_5678_9abc;
    for _ in 0..512 {
        let mut fill = good();
        let mut applied: Vec<&str> = Vec::new();
        for (name, mutate) in &mutations {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            if (state >> 60) & 1 == 1 {
                mutate(&mut fill);
                applied.push(name);
            }
        }
        let defects = check(&fill, &expected, &Described::default());
        for defect in &defects {
            covered.insert(variant(defect));
        }
        let (repaired, mends, passes) = mended(&fill, &expected, &described);
        assert_eq!(
            check(&repaired, &expected, &Described::default()),
            [],
            "{applied:?} left {mends:?} and still a defect"
        );
        assert!(passes <= MEND_PASSES, "{applied:?} took {passes} passes");
        assert_eq!(
            mends.is_empty(),
            defects.is_empty(),
            "{applied:?}: a defect is a mend and nothing else is"
        );
    }
    assert_eq!(
        covered,
        BTreeSet::from([
            "Missing",
            "Empty",
            "Multiline",
            "TooShort",
            "TooLong",
            "TooMany",
            "UnknownTarget",
            "ToolNamed",
        ]),
        "every repairable defect was generated, and `NotJson` cannot be: \
             the mend is handed a fill, not an answer"
    );
}

#[test]
fn a_multibyte_name_is_cut_on_a_character_boundary() {
    let name = format!("{}.rs", "é".repeat(400));
    let symbol = "🜁_très_long_identifiant_déclaré".repeat(20);
    let request = Request::new("describe", "/repo/x")
        .with_files([File::present(name.clone(), *b"pub fn draw() {}\n")]);
    let expected = Expected::of(&request);
    let described = Described {
        declared: [(name.clone(), vec![symbol.clone(), symbol])]
            .into_iter()
            .collect(),
        ..Described::default()
    };

    // No panic, and the count is characters rather than bytes.
    let line = fallback::file(&name, &expected, &described);
    holds_the_shape(&line);
    assert_eq!(line.chars().count(), ENTRY_CHARS);
    assert!(line.len() > ENTRY_CHARS, "multibyte: {} bytes", line.len());

    holds_the_shape(&fallback::purpose(&name, &expected, &Described::default()));
    holds_the_shape(&fallback::directory(
        &name,
        &expected,
        &Described::default(),
    ));
}

#[test]
fn a_files_line_may_not_assert_a_mechanism_only_a_comment_claims() {
    // The one planted lie that ever reached a document. balance.rs's module
    // comment says every Posting is validated by `Decoder::decode()`; the file
    // holds no such call and `Decoder` is declared two directories away, so
    // every name in the claim is real somewhere and only the mechanism is
    // invented. It survived because a comment counted as evidence and because
    // nothing checked a files line at all.
    let request = Request::new("describe", "/repo/engine/core").with_files([File::present(
        "balance.rs",
        *b"//! Every Posting is validated by Decoder::decode() before VAULT_LIMIT.\n\
           \n\
           pub const VAULT_LIMIT: usize = 512;\n\
           pub fn is_settled(open: usize) -> bool {\n\
               open == 0\n\
           }\n",
    )]);
    let expected = Expected::of(&request);
    let described = Described::default();

    let mut fill = Fill::stub(&request);
    fill.purpose = "Balance checks over the ledger's open accounts.".to_owned();
    fill.structure = Vec::new();
    fill.files.insert(
        "balance.rs".to_owned(),
        "is_settled(open) reports whether the open count is zero; VAULT_LIMIT (512) \
         caps postings validated by Decoder::decode()."
            .to_owned(),
    );

    let defects = check(&fill, &expected, &described);
    assert!(
        defects.iter().any(|defect| matches!(
            defect,
            Defect::UnknownTarget { name, .. } if name == "Decoder::decode"
        )),
        "the invented call is refused: {defects:?}"
    );
    // The same line's true call is not: `is_settled` is in the code, and a
    // check that took the whole line down would be refusing both.
    assert!(
        !defects.iter().any(|defect| matches!(
            defect,
            Defect::UnknownTarget { name, .. } if name == "is_settled"
        )),
        "a call the file does declare stands: {defects:?}"
    );

    let (mended, mends) = mend(&fill, &expected, &described);
    assert!(
        check(&mended, &expected, &described).is_empty(),
        "a mended fill is not defective"
    );
    let line = mended.files.get("balance.rs").expect("a line per file");
    assert!(!line.contains("Decoder"), "the claim is gone: {line}");
    assert!(
        mends
            .iter()
            .any(|mend| mend.field == "files[\"balance.rs\"]"),
        "the mend says which line it replaced: {mends:?}"
    );

    let document = render("core", &mended, &expected, &described);
    assert!(!document.contains("Decoder"), "{document}");
    assert!(!document.contains("— \n"), "no dangling dash: {document}");
}

#[test]
fn a_comment_is_not_a_declaration_but_the_pass_still_reads_it() {
    // The narrow claim. Stripping comments changes what a name may rest on and
    // nothing else: the request still carries every byte of the file, so a pass
    // still reads the comment and may still say what it claims — attributed.
    let text = *b"//! A gorilla reconciles balances overnight.\npub fn post() {}\n";
    let request =
        Request::new("describe", "/repo/engine/core").with_files([File::present("ledger.rs", text)]);
    let expected = Expected::of(&request);

    let sent = request.files()[0].bytes().expect("the text is sent whole");
    assert!(
        String::from_utf8_lossy(sent).contains("gorilla"),
        "the comment reaches the model untouched"
    );

    let mut fill = Fill::stub(&request);
    fill.purpose = "The ledger core.".to_owned();
    fill.structure = Vec::new();
    fill.files.insert(
        "ledger.rs".to_owned(),
        "Posts entries with reconcile(); a gorilla comment aside.".to_owned(),
    );
    let defects = check(&fill, &expected, &Described::default());
    assert!(
        defects.iter().any(|defect| matches!(
            defect,
            Defect::UnknownTarget { name, .. } if name == "reconcile"
        )),
        "a call that only the comment supports is refused: {defects:?}"
    );
}
