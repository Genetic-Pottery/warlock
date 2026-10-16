use std::path::{Path, PathBuf};

use warlock_engine::{Manifest, ScopedDirectory};

use super::{Crossing, crossings_in};
use crate::git::{Dirty, Touched, dirty_in};

// Never read: nothing on this path touches a disk.
const ROOT: &str = "/repo";

fn scoped(directories: &[(&str, &str)]) -> Manifest {
    Manifest::with_directories(directories.iter().map(|(module, scope)| {
        ScopedDirectory::new(ROOT, module, *scope)
            .expect("a relative module path is inside the root")
    }))
}

fn root() -> PathBuf {
    PathBuf::from(ROOT)
}

fn opened(open: &[&str]) -> Vec<String> {
    open.iter().map(|scope| (*scope).to_owned()).collect()
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
    let manifest = scoped(&[("crates/engine", "data-plane"), ("crates/web", "web")]);
    let changed = [Dirty {
        code: "R ".to_owned(),
        path: "crates/web/src/route.rs".to_owned(),
        from: Some("crates/engine/src/route.rs".to_owned()),
    }];

    let crossings = crossings_in(
        &changed,
        &root(),
        &manifest,
        &opened(["web"].as_slice()),
        Some("web"),
    );

    assert_eq!(
        crossings.crossed,
        [Crossing {
            path: "crates/engine/src/route.rs",
            scope: Some("data-plane"),
        }],
        "the destination alone would say a file appeared and never that one left"
    );
    assert!(crossings.touched.is_empty(), "{:?}", crossings.touched);
}

#[test]
fn an_untracked_file_under_a_closed_scope_crosses() {
    let manifest = scoped(&[("crates/engine", "data-plane")]);
    let changed = [dirty("??", "crates/engine/src/new.rs")];

    let crossings = crossings_in(&changed, &root(), &manifest, &opened(&[]), Some("web"));

    assert_eq!(
        crossings.crossed,
        [Crossing {
            path: "crates/engine/src/new.rs",
            scope: Some("data-plane"),
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
    let manifest = scoped(&[("crates/engine", "data-plane")]);
    let payload = b"?? crates/engine/say \"hi\".rs\0 M crates/engine/back\\slash.rs\0";

    let changed = dirty_in(&payload[..]);
    let crossings = crossings_in(&changed, &root(), &manifest, &opened(&[]), None);

    assert_eq!(
        crossings.crossed,
        [
            Crossing {
                path: "crates/engine/say \"hi\".rs",
                scope: Some("data-plane"),
            },
            Crossing {
                path: "crates/engine/back\\slash.rs",
                scope: Some("data-plane"),
            },
        ]
    );
}

#[test]
fn a_non_ascii_path_keeps_its_bytes_and_a_non_utf8_one_arrives_replaced() {
    let manifest = scoped(&[("crates/moteur", "data-plane")]);
    // The second path is not UTF-8 at all. `dirty_in` converts lossily, which is
    // the module's one lossy conversion, so what arrives here carries U+FFFD
    // where the bad byte was: it does not round-trip, and the scope is still the
    // right one because the segments above it are intact.
    let mut payload = b" M crates/moteur/caf\xc3\xa9.rs\0?? crates/moteur/".to_vec();
    payload.extend_from_slice(b"bad\xffname.rs\0");

    let changed = dirty_in(&payload);
    let crossings = crossings_in(&changed, &root(), &manifest, &opened(&[]), None);

    assert_eq!(
        crossings.crossed,
        [
            Crossing {
                path: "crates/moteur/café.rs",
                scope: Some("data-plane"),
            },
            Crossing {
                path: "crates/moteur/bad\u{fffd}name.rs",
                scope: Some("data-plane"),
            },
        ]
    );
}

#[test]
fn a_path_under_no_scope_crosses_without_a_scope_to_name() {
    let manifest = scoped(&[("crates/engine", "data-plane")]);
    let changed = [
        dirty("??", "scripts/release.sh"),
        // A sibling that merely shares a prefix with the scoped module.
        dirty(" M", "crates/engine-tools/src/lib.rs"),
    ];

    let crossings = crossings_in(
        &changed,
        &root(),
        &manifest,
        &opened(["data-plane"].as_slice()),
        Some("data-plane"),
    );

    assert_eq!(
        crossings.crossed,
        [
            Crossing {
                path: "scripts/release.sh",
                scope: None,
            },
            Crossing {
                path: "crates/engine-tools/src/lib.rs",
                scope: None,
            },
        ],
        "an unscoped path is closed to everyone, so writing there crosses"
    );
    assert!(crossings.touched.is_empty(), "{:?}", crossings.touched);
}

#[test]
fn the_wildcard_does_not_open_a_path_under_no_scope() {
    let manifest = scoped(&[("crates/engine", "data-plane")]);
    let changed = [dirty(" M", "docs/adr/0001.md")];

    let crossings = crossings_in(
        &changed,
        &root(),
        &manifest,
        &opened(["*"].as_slice()),
        None,
    );

    assert_eq!(
        crossings.crossed,
        [Crossing {
            path: "docs/adr/0001.md",
            scope: None,
        }],
        "the wildcard opens every scope, and a path under none is not in one"
    );
}

#[test]
fn an_open_scope_that_is_not_the_pulled_one_is_touched_and_never_crossed() {
    let manifest = scoped(&[
        ("crates/engine", "data-plane"),
        ("crates/web", "web"),
        ("crates/billing", "billing"),
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
        &opened(["web", "billing"].as_slice()),
        Some("web"),
    );

    assert_eq!(
        crossings.touched,
        [Touched {
            scope: "billing",
            paths: vec!["crates/billing/src/invoice.rs", "crates/billing/src/tax.rs"],
        }],
        "the scope pulled under is not reported, and an open scope groups its paths once"
    );
    assert_eq!(
        crossings.crossed,
        [Crossing {
            path: "crates/engine/src/lib.rs",
            scope: Some("data-plane"),
        }],
        "two open scopes say nothing about a third scope"
    );
}

#[test]
fn the_wildcard_opens_a_foreign_scope_without_making_it_the_pulled_one() {
    let manifest = scoped(&[("crates/engine", "data-plane")]);
    let changed = [dirty(" M", "crates/engine/src/lib.rs")];

    let crossings = crossings_in(
        &changed,
        &root(),
        &manifest,
        &opened(["*"].as_slice()),
        Some("web"),
    );

    assert!(crossings.crossed.is_empty(), "{:?}", crossings.crossed);
    assert_eq!(
        crossings.touched,
        [Touched {
            scope: "data-plane",
            paths: vec!["crates/engine/src/lib.rs"],
        }],
        "a machine with every scope open still reached past the scope it was pulled under"
    );
}

#[test]
fn a_machine_with_no_scope_open_crosses_every_scope() {
    let manifest = scoped(&[("crates/engine", "data-plane"), ("crates/web", "web")]);
    let changed = [
        dirty(" M", "crates/web/src/page.rs"),
        dirty(" M", "crates/engine/src/lib.rs"),
        dirty(" M", "docs/adr/0002.md"),
    ];

    let crossings = crossings_in(&changed, &root(), &manifest, &opened(&[]), None);

    assert_eq!(
        crossings.crossed,
        [
            Crossing {
                path: "crates/web/src/page.rs",
                scope: Some("web"),
            },
            Crossing {
                path: "crates/engine/src/lib.rs",
                scope: Some("data-plane"),
            },
            Crossing {
                path: "docs/adr/0002.md",
                scope: None,
            },
        ],
        "no scope is open, so every scope is crossed"
    );
    assert!(crossings.touched.is_empty(), "{:?}", crossings.touched);
}

#[test]
fn the_same_path_named_twice_is_reported_once_and_in_the_order_git_gave() {
    // A rename whose source is also the destination of another entry: the two
    // sides meet on one path, and a reason that named it twice would read as two
    // findings.
    let manifest = scoped(&[("crates/engine", "data-plane")]);
    let changed = [
        Dirty {
            code: "R ".to_owned(),
            path: "crates/engine/b.rs".to_owned(),
            from: Some("crates/engine/a.rs".to_owned()),
        },
        dirty(" M", "crates/engine/a.rs"),
    ];

    let crossings = crossings_in(&changed, &root(), &manifest, &opened(&[]), None);

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
fn a_path_reaching_outside_the_root_reads_as_under_no_scope() {
    let manifest = scoped(&[("crates/engine", "data-plane")]);
    let changed = [dirty(" M", "../elsewhere/crates/engine/src/lib.rs")];

    let crossings = crossings_in(
        &changed,
        Path::new(ROOT),
        &manifest,
        &opened(["data-plane"].as_slice()),
        Some("data-plane"),
    );

    assert_eq!(
        crossings.crossed,
        [Crossing {
            path: "../elsewhere/crates/engine/src/lib.rs",
            scope: None,
        }],
        "a path no scope can be found for is closed, never quietly open"
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
    use super::{Crossing, ROOT, opened, root, scoped};
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
        // prints: a file moved out of a scope that is closed on this machine, an
        // untracked file under one that is open but was not pulled under, and an
        // edit under the pulled scope itself.
        let scripted = Scripted::says(
            &b"R  crates/web/src/route.rs\0crates/engine/src/route.rs\0\
?? crates/cli/src/new.rs\0 M crates/web/src/page.rs\0"[..],
        );
        let checkout = scripted.checkout();
        let manifest = scoped(&[
            ("crates/engine", "data-plane"),
            ("crates/web", "web"),
            ("crates/cli", "cli"),
        ]);
        let mut changed = Vec::new();

        let crossings = crossings_after(
            &checkout,
            &mut changed,
            &root(),
            &manifest,
            &opened(["web", "cli"].as_slice()),
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
                scope: Some("data-plane"),
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
        let manifest = scoped(&[("docs", "docs")]);
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
            &opened(["docs"].as_slice()),
            Some("docs"),
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
        let manifest = scoped(&[("crates/engine", "data-plane")]);
        let mut changed = Vec::new();

        let error = crossings_after(
            &scripted.checkout(),
            &mut changed,
            &root(),
            &manifest,
            &opened(&[]),
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
