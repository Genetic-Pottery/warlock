use std::path::{Path, PathBuf};

use warlock_engine::{Manifest, PactEntry};

use super::{Crossing, crossings_in};
use crate::git::{Dirty, Touched, dirty_in};

// Never read: nothing on this path touches a disk.
const ROOT: &str = "/repo";

fn pacts(entries: &[(&str, Option<&str>)]) -> Manifest {
    Manifest::with_entries(entries.iter().map(|(module, scope)| {
        let entry = PactEntry::new(ROOT, module, format!("{module}/WARLOCK.md"))
            .expect("a relative module path is inside the root");
        match scope {
            Some(scope) => entry.with_scope(*scope),
            None => entry,
        }
    }))
}

fn root() -> PathBuf {
    PathBuf::from(ROOT)
}

fn held(sigils: &[&str]) -> Vec<String> {
    sigils.iter().map(|sigil| (*sigil).to_owned()).collect()
}

fn dirty(code: &str, path: &str) -> Dirty {
    Dirty {
        code: code.to_owned(),
        path: path.to_owned(),
        from: None,
    }
}

#[test]
fn a_rename_out_of_a_closed_scope_crosses_on_the_side_that_left() {
    let manifest = pacts(&[
        ("crates/engine", Some("data-plane")),
        ("crates/web", Some("web")),
    ]);
    let changed = [Dirty {
        code: "R ".to_owned(),
        path: "crates/web/src/route.rs".to_owned(),
        from: Some("crates/engine/src/route.rs".to_owned()),
    }];

    let crossings = crossings_in(
        &changed,
        &root(),
        &manifest,
        &held(["web"].as_slice()),
        Some("web"),
    );

    assert_eq!(
        crossings.crossed,
        [Crossing {
            path: "crates/engine/src/route.rs",
            scope: "data-plane",
        }],
        "the destination alone would say a file appeared and never that one left"
    );
    assert!(crossings.touched.is_empty(), "{:?}", crossings.touched);
}

#[test]
fn an_untracked_file_under_a_closed_scope_crosses() {
    let manifest = pacts(&[("crates/engine", Some("data-plane"))]);
    let changed = [dirty("??", "crates/engine/src/new.rs")];

    let crossings = crossings_in(&changed, &root(), &manifest, &held(&[]), Some("web"));

    assert_eq!(
        crossings.crossed,
        [Crossing {
            path: "crates/engine/src/new.rs",
            scope: "data-plane",
        }],
        "a file that was never added is still a file this session wrote"
    );
}

#[test]
fn a_path_holding_a_quote_or_a_backslash_is_classified_as_written() {
    // With `-z` git neither quotes the path nor escapes anything in it, so these
    // are literal `"` and `\` bytes in a file name and not the quoting a plain
    // `--porcelain` would have applied. A reader that stripped quotes here would
    // look up a path no manifest has.
    let manifest = pacts(&[("crates/engine", Some("data-plane"))]);
    let payload = b"?? crates/engine/say \"hi\".rs\0 M crates/engine/back\\slash.rs\0";

    let changed = dirty_in(&payload[..]);
    let crossings = crossings_in(&changed, &root(), &manifest, &held(&[]), None);

    assert_eq!(
        crossings.crossed,
        [
            Crossing {
                path: "crates/engine/say \"hi\".rs",
                scope: "data-plane",
            },
            Crossing {
                path: "crates/engine/back\\slash.rs",
                scope: "data-plane",
            },
        ]
    );
}

#[test]
fn a_non_ascii_path_keeps_its_bytes_and_a_non_utf8_one_arrives_replaced() {
    let manifest = pacts(&[("crates/moteur", Some("data-plane"))]);
    // The second path is not UTF-8 at all. `dirty_in` converts lossily, which is
    // the module's one lossy conversion, so what arrives here carries U+FFFD
    // where the bad byte was: it does not round-trip, and the scope is still the
    // right one because the segments above it are intact.
    let mut payload = b" M crates/moteur/caf\xc3\xa9.rs\0?? crates/moteur/".to_vec();
    payload.extend_from_slice(b"bad\xffname.rs\0");

    let changed = dirty_in(&payload);
    let crossings = crossings_in(&changed, &root(), &manifest, &held(&[]), None);

    assert_eq!(
        crossings.crossed,
        [
            Crossing {
                path: "crates/moteur/café.rs",
                scope: "data-plane",
            },
            Crossing {
                path: "crates/moteur/bad\u{fffd}name.rs",
                scope: "data-plane",
            },
        ]
    );
}

#[test]
fn a_path_under_no_scope_is_open_and_in_neither_list() {
    let manifest = pacts(&[
        ("crates", None),
        ("crates/engine", Some("data-plane")),
        ("docs", None),
    ]);
    let changed = [
        dirty(" M", "docs/adr/0001.md"),
        // Unpacted: no entry at or above it says anything.
        dirty("??", "scripts/release.sh"),
        // A sibling that merely shares a prefix with the scoped module.
        dirty(" M", "crates/engine-tools/src/lib.rs"),
    ];

    let crossings = crossings_in(&changed, &root(), &manifest, &held(&[]), None);

    assert!(crossings.is_empty(), "{crossings:?}");
}

#[test]
fn a_held_scope_that_is_not_the_pulled_one_is_touched_and_never_crossed() {
    let manifest = pacts(&[
        ("crates/engine", Some("data-plane")),
        ("crates/web", Some("web")),
        ("crates/billing", Some("billing")),
    ]);
    let changed = [
        dirty(" M", "crates/web/src/page.rs"),
        dirty(" M", "crates/billing/src/invoice.rs"),
        dirty(" M", "crates/billing/src/tax.rs"),
        dirty(" M", "crates/engine/src/lib.rs"),
    ];

    let crossings = crossings_in(
        &changed,
        &root(),
        &manifest,
        &held(["web", "billing"].as_slice()),
        Some("web"),
    );

    assert_eq!(
        crossings.touched,
        [Touched {
            scope: "billing",
            paths: vec!["crates/billing/src/invoice.rs", "crates/billing/src/tax.rs"],
        }],
        "the scope pulled under is not reported, and a held scope groups its paths once"
    );
    assert_eq!(
        crossings.crossed,
        [Crossing {
            path: "crates/engine/src/lib.rs",
            scope: "data-plane",
        }],
        "holding two sigils says nothing about a third scope"
    );
}

#[test]
fn the_wildcard_opens_a_foreign_scope_without_making_it_the_pulled_one() {
    let manifest = pacts(&[("crates/engine", Some("data-plane"))]);
    let changed = [dirty(" M", "crates/engine/src/lib.rs")];

    let crossings = crossings_in(
        &changed,
        &root(),
        &manifest,
        &held(["*"].as_slice()),
        Some("web"),
    );

    assert!(crossings.crossed.is_empty(), "{:?}", crossings.crossed);
    assert_eq!(
        crossings.touched,
        [Touched {
            scope: "data-plane",
            paths: vec!["crates/engine/src/lib.rs"],
        }],
        "a machine holding everything still reached past the scope it was pulled under"
    );
}

#[test]
fn a_machine_holding_nothing_crosses_every_scope_and_no_open_path() {
    let manifest = pacts(&[
        ("crates/engine", Some("data-plane")),
        ("crates/web", Some("web")),
        ("docs", None),
    ]);
    let changed = [
        dirty(" M", "crates/web/src/page.rs"),
        dirty(" M", "crates/engine/src/lib.rs"),
        dirty(" M", "docs/adr/0002.md"),
    ];

    let crossings = crossings_in(&changed, &root(), &manifest, &held(&[]), None);

    assert_eq!(
        crossings.crossed,
        [
            Crossing {
                path: "crates/web/src/page.rs",
                scope: "web",
            },
            Crossing {
                path: "crates/engine/src/lib.rs",
                scope: "data-plane",
            },
        ],
        "a sigil is what opens a scope, so holding none opens none"
    );
    assert!(crossings.touched.is_empty(), "{:?}", crossings.touched);
}

#[test]
fn the_same_path_named_twice_is_reported_once_and_in_the_order_git_gave() {
    // A rename whose source is also the destination of another entry: the two
    // sides meet on one path, and a reason that named it twice would read as two
    // findings.
    let manifest = pacts(&[("crates/engine", Some("data-plane"))]);
    let changed = [
        Dirty {
            code: "R ".to_owned(),
            path: "crates/engine/b.rs".to_owned(),
            from: Some("crates/engine/a.rs".to_owned()),
        },
        dirty(" M", "crates/engine/a.rs"),
    ];

    let crossings = crossings_in(&changed, &root(), &manifest, &held(&[]), None);

    assert_eq!(
        crossings
            .crossed
            .iter()
            .map(|crossing| crossing.path)
            .collect::<Vec<&str>>(),
        ["crates/engine/b.rs", "crates/engine/a.rs"]
    );
}

#[test]
fn a_path_reaching_outside_the_root_is_not_a_boundary_answer() {
    let manifest = pacts(&[("crates/engine", Some("data-plane"))]);
    let changed = [dirty(" M", "../elsewhere/crates/engine/src/lib.rs")];

    let crossings = crossings_in(
        &changed,
        Path::new(ROOT),
        &manifest,
        &held(&[]),
        Some("web"),
    );

    assert!(
        crossings.is_empty(),
        "a caller that passed the wrong root has a mistake to report, not a crossing: {crossings:?}"
    );
}

// The reading half. A stand-in `Runs` under a real `Git`, rather than a stand-in
// `Repository`: the argument vector is half of what is under test, and only the
// real `Git` builds one.
mod after {
    use std::ffi::{OsStr, OsString};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use super::super::crossings_after;
    use super::{Crossing, ROOT, held, pacts, root};
    use crate::git::{Dirty, Error, Git, Ran, Runs, Touched};

    struct Shared {
        vectors: Mutex<Vec<Vec<String>>>,
        // One answer and not a queue: a reading that ran `git` twice is a bug
        // this stand-in should fail on rather than script for.
        answer: Mutex<Option<Ran>>,
    }

    // Cloned into the `Git` and kept by the test, the way `tests/git.rs` does it,
    // so the vectors can be read after the checkout has taken its runner by
    // value.
    #[derive(Clone)]
    struct Scripted {
        shared: Arc<Shared>,
    }

    impl Scripted {
        fn says(stdout: impl Into<Vec<u8>>) -> Self {
            Self::answering(Ran::new(Some(0), stdout, ""))
        }

        fn answering(answer: Ran) -> Self {
            Self {
                shared: Arc::new(Shared {
                    vectors: Mutex::new(Vec::new()),
                    answer: Mutex::new(Some(answer)),
                }),
            }
        }

        fn checkout(&self) -> Git<Self> {
            Git::new(self.clone(), ROOT)
        }

        fn only(&self) -> Vec<String> {
            let vectors = self.shared.vectors.lock().expect("vectors");
            assert_eq!(vectors.len(), 1, "expected one call: {vectors:?}");
            vectors[0].clone()
        }
    }

    impl Runs for Scripted {
        fn run(&self, program: &OsStr, args: &[OsString], _: &Path) -> Result<Ran, Error> {
            assert_eq!(program, OsStr::new("git"));
            self.shared.vectors.lock().expect("vectors").push(
                args.iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
            );
            Ok(self
                .shared
                .answer
                .lock()
                .expect("answer")
                .take()
                .expect("one status call and no more"))
        }
    }

    #[test]
    fn the_tree_is_read_with_one_status_call_and_judged_on_what_it_said() {
        // The bytes a real `git status --porcelain=v1 -z --untracked-files=all`
        // prints: a file moved out of a scope this machine does not hold, an
        // untracked file under one it holds but did not pull under, and an edit
        // under the pulled scope itself.
        let scripted = Scripted::says(
            &b"R  crates/web/src/route.rs\0crates/engine/src/route.rs\0\
?? crates/cli/src/new.rs\0 M crates/web/src/page.rs\0"[..],
        );
        let checkout = scripted.checkout();
        let manifest = pacts(&[
            ("crates/engine", Some("data-plane")),
            ("crates/web", Some("web")),
            ("crates/cli", Some("cli")),
        ]);
        let mut changed = Vec::new();

        let crossings = crossings_after(
            &checkout,
            &mut changed,
            &root(),
            &manifest,
            &held(["web", "cli"].as_slice()),
            Some("web"),
        )
        .expect("the scripted status");

        assert_eq!(
            scripted.only(),
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            "one vector, and the one the ticket names: no diff and no second status"
        );
        assert_eq!(
            crossings.crossed,
            [Crossing {
                path: "crates/engine/src/route.rs",
                scope: "data-plane",
            }]
        );
        assert_eq!(
            crossings.touched,
            [Touched {
                scope: "cli",
                paths: vec!["crates/cli/src/new.rs"],
            }]
        );
    }

    #[test]
    fn what_git_said_is_left_in_the_entries_the_caller_lent() {
        // The crossings name their paths by borrowing out of these, which is why
        // the entries are the caller's; whatever was in them before is gone.
        let scripted = Scripted::says(&b" M docs/adr/0002.md\0"[..]);
        let manifest = pacts(&[("docs", None)]);
        let mut changed = vec![Dirty {
            code: "??".to_owned(),
            path: "a-reading-from-before.rs".to_owned(),
            from: None,
        }];

        let crossings = crossings_after(
            &scripted.checkout(),
            &mut changed,
            &root(),
            &manifest,
            &held(&[]),
            None,
        )
        .expect("the scripted status");

        assert!(crossings.is_empty(), "{crossings:?}");
        assert_eq!(
            changed,
            [Dirty {
                code: " M".to_owned(),
                path: "docs/adr/0002.md".to_owned(),
                from: None,
            }]
        );
    }

    #[test]
    fn a_git_that_refuses_the_status_is_the_error_and_not_a_clean_tree() {
        // The failure this check exists for, inverted: a status nobody could
        // read, taken for a session that crossed nothing, and the run committing
        // over the top of it.
        let scripted =
            Scripted::answering(Ran::new(Some(128), "", "fatal: not a git repository\n"));
        let manifest = pacts(&[("crates/engine", Some("data-plane"))]);
        let mut changed = Vec::new();

        let error = crossings_after(
            &scripted.checkout(),
            &mut changed,
            &root(),
            &manifest,
            &held(&[]),
            None,
        )
        .expect_err("a refused status is not an answer about the tree");

        assert!(
            matches!(
                error,
                Error::Refused {
                    code: Some(128),
                    ..
                }
            ),
            "{error:?}"
        );
        assert!(
            error.to_string().contains("not a git repository"),
            "{error}"
        );
    }
}
