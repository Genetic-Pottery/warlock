use std::time::Duration;

use super::{COMMAND_TIMEOUT, Ran, Spawner};

// A name no directory on `PATH` can hold, so the lookup fails the way it does on
// a machine with no `gh` installed.
const NOT_A_PROGRAM: &str = "warlock-test-no-such-program-4b7e12";

#[test]
fn a_spawner_runs_under_the_module_clock_unless_it_is_told_otherwise() {
    assert_eq!(Spawner::new().timeout(), COMMAND_TIMEOUT);
    assert_eq!(Spawner::default().timeout(), COMMAND_TIMEOUT);
    assert_eq!(
        Spawner::new()
            .with_timeout(Duration::from_millis(250))
            .timeout(),
        Duration::from_millis(250)
    );
}

#[test]
fn an_outcome_reports_its_code_its_streams_and_whether_it_succeeded() {
    let clean = Ran::new(Some(0), "main\n", "");
    let refused = Ran::new(Some(128), "", "fatal: not a git repository\n");
    // No code at all: a child a signal ended, which is not a success however it
    // is asked.
    let signalled = Ran::new(None, "", "");

    assert!(clean.success());
    assert_eq!(clean.code(), Some(0));
    assert_eq!(clean.stdout(), b"main\n");
    assert_eq!(clean.stdout_text(), "main\n");

    assert!(!refused.success());
    assert_eq!(refused.code(), Some(128));
    assert_eq!(refused.stderr_text().trim(), "fatal: not a git repository");

    assert!(!signalled.success());
    assert_eq!(signalled.code(), None);
}

#[test]
fn output_that_is_not_utf_8_comes_back_as_the_bytes_it_was() {
    // `git status -z` names paths, and a path is not required to be UTF-8. The
    // bytes survive; only the text accessor is lossy.
    let ran = Ran::new(Some(0), vec![b'?', b'?', b' ', 0xff, 0x00], "");

    assert_eq!(ran.stdout(), [b'?', b'?', b' ', 0xff, 0x00]);
    assert!(ran.stdout_text().contains('\u{fffd}'));
}

// The stand-ins below are shell scripts, so the whole module is Unix-only. What is
// under test — the pipes, the closed stdin, the timeout, the kill — is not, but a
// portable stand-in would have to be a second binary to build.
#[cfg(unix)]
mod unix {
    use std::collections::HashMap;
    use std::ffi::{OsStr, OsString};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use std::{env, fs, process, thread};

    use super::super::{Error, Forge, Gh, Opened, PullRequest, Ran, Runs, Spawner};
    use super::NOT_A_PROGRAM;

    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    // `sh -c` stands in for `git` and `gh` throughout: this module is about the
    // spawn, not about either program.
    fn ran(script: &str, directory: &Path) -> Result<super::super::Ran, Error> {
        Spawner::new().run("/bin/sh".as_ref(), &args(&["-c", script]), directory)
    }

    // Hand-rolled rather than a dependency: this crate's manifest gains nothing
    // for a temp directory.
    fn scratch(name: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let unique = NEXT.fetch_add(1, Ordering::Relaxed);
        let directory =
            env::temp_dir().join(format!("warlock-git-{}-{name}-{unique}", process::id()));
        fs::create_dir_all(&directory).expect("a scratch directory under the temp directory");
        directory
    }

    fn clean_up(directory: &Path) {
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn a_child_that_writes_more_than_a_pipeful_comes_back_whole() {
        // A pipe holds something like 64KiB, so a quarter of a megabyte is the
        // test: a reader that waited for the exit first would hang here and
        // pass every other test in this module. `git diff` and `git log` both
        // reach this size on anything real.
        //
        // The payload is built by the shell rather than written here, because a
        // quarter of a megabyte inside `sh -c` would be a single argument past
        // what the kernel will take.
        let ran = ran(
            "yes gribbleflix | head -n 20000; yes borogove | head -n 20000 >&2",
            Path::new("."),
        )
        .expect("the stand-in exits cleanly");

        assert!(ran.success());
        assert_eq!(ran.stdout().len(), 20_000 * "gribbleflix\n".len());
        assert_eq!(ran.stderr().len(), 20_000 * "borogove\n".len());
        assert_eq!(ran.stdout_text().lines().next(), Some("gribbleflix"));
    }

    #[test]
    fn a_non_zero_exit_carries_its_status_and_its_stderr() {
        // Not an error: which statuses are failures is the operation's business,
        // and `git diff --quiet` answers a question with one.
        let ran = ran("echo boom >&2; exit 3", Path::new(".")).expect("the stand-in ran");

        assert!(!ran.success());
        assert_eq!(ran.code(), Some(3));
        assert_eq!(ran.stderr_text().trim(), "boom");
        assert!(ran.stdout().is_empty());
    }

    #[test]
    fn a_child_that_reads_stdin_still_finishes() {
        // Stdin is closed rather than piped, so `cat` sees EOF at once. Given a
        // pipe nobody writes to it would sit there until the timeout, which is
        // what a `git` waiting for a passphrase or a merge message would do.
        let ran = Spawner::new()
            .with_timeout(Duration::from_secs(5))
            .run("/bin/cat".as_ref(), &[], Path::new("."))
            .expect("cat sees EOF on a closed stdin and exits");

        assert!(ran.success());
        assert!(ran.stdout().is_empty());
    }

    #[test]
    fn the_child_runs_in_the_directory_it_was_given() {
        let directory = scratch("cwd");
        fs::write(directory.join("marker.txt"), "here").expect("a file to look for");

        let ran = ran("ls", &directory).expect("ls exits cleanly and prints a name");

        assert!(
            ran.stdout_text().contains("marker.txt"),
            "the child ran somewhere else: {}",
            ran.stdout_text()
        );
        clean_up(&directory);
    }

    #[test]
    fn the_ambient_environment_reaches_the_child() {
        // `gh` authenticates from the environment and its own config, so the
        // child's environment is inherited whole. Warlock adds nothing to it —
        // that is the other half of the rule, and the half a test cannot prove
        // by looking — but an `env_clear` slipped in here would break every
        // pull request on a machine where `gh` works from the shell, and this is
        // what would fail.
        let path = env::var("PATH").expect("the test process has a PATH");

        let ran = ran(r#"printf '%s' "$PATH""#, Path::new(".")).expect("the stand-in ran");

        assert_eq!(ran.stdout_text(), path);
    }

    #[test]
    fn a_child_that_never_exits_is_killed_at_the_timeout() {
        let directory = scratch("hang");
        let ticks = directory.join("ticks");
        // Never exits on its own, and says so in a file: whether it is still
        // running after the call is a question the test can ask.
        let spawner = Spawner::new().with_timeout(Duration::from_millis(250));

        let started = Instant::now();
        let error = spawner
            .run(
                "/bin/sh".as_ref(),
                &args(&["-c", "while :; do echo tick >> ticks; sleep 0.05; done"]),
                &directory,
            )
            .expect_err("this stand-in never finishes");
        let elapsed = started.elapsed();

        match error {
            Error::TimedOut { after } => assert_eq!(after, Duration::from_millis(250)),
            other => panic!("expected a timeout, got {other:?}"),
        }
        assert!(
            elapsed < Duration::from_secs(10),
            "the call waited {elapsed:?}, far past its timeout"
        );

        let before = fs::metadata(&ticks).map_or(0, |file| file.len());
        thread::sleep(Duration::from_millis(300));
        let after = fs::metadata(&ticks).map_or(0, |file| file.len());
        assert_eq!(
            before, after,
            "the child outlived the call that gave up on it"
        );
        clean_up(&directory);
    }

    // The kill is only half of it — a child nobody waits on stays in the process
    // table. `/proc` is where that is visible, so this test alone is Linux-only;
    // the kill itself is covered on every Unix above.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_timed_out_child_is_reaped_not_left_a_zombie() {
        let directory = scratch("reap");
        let spawner = Spawner::new().with_timeout(Duration::from_millis(250));

        let started = Instant::now();
        let error = spawner
            .run(
                "/bin/sh".as_ref(),
                &args(&["-c", "echo $$ > pid; sleep 30"]),
                &directory,
            )
            .expect_err("this stand-in sleeps far past its timeout");
        let elapsed = started.elapsed();

        assert!(matches!(error, Error::TimedOut { .. }), "{error:?}");
        assert!(
            elapsed < Duration::from_secs(20),
            "the call outlasted the sleep it was supposed to cut short: {elapsed:?}"
        );

        let pid = fs::read_to_string(directory.join("pid")).expect("the child wrote its pid");
        let pid = pid.trim();
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "process {pid} is still in the table: killed but never reaped"
        );
        clean_up(&directory);
    }

    #[test]
    fn a_program_no_path_holds_is_reported_as_missing() {
        let error = Spawner::new()
            .run(NOT_A_PROGRAM.as_ref(), &[], Path::new("."))
            .expect_err("nothing runs a program that is not there");

        match error {
            Error::NotFound { program } => assert_eq!(program, NOT_A_PROGRAM),
            other => panic!("expected a missing program, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_directory_is_io_rather_than_a_missing_program() {
        // The syscall says `NotFound` for both, and only one of them means the
        // machine has no `gh` on it.
        let error = ran("true", Path::new("/warlock/no/such/directory"))
            .expect_err("nothing can run in a directory that is not there");

        assert!(matches!(error, Error::Io { .. }), "{error:?}");
    }

    #[test]
    fn nothing_is_added_to_a_child_s_environment() {
        // The other half of the no-key rule, and the half that is about the
        // environment rather than the argument vector — `operations` has the
        // arguments. Warlock holds a Linear key; no `git` or `gh` child may be
        // handed one, and the way that is kept true is that nothing is written
        // into a child's environment at all. Inherited whole and untouched:
        // `gh` reads its own credentials from out there, so the fix for a key
        // leaking is never an `env_clear`.
        let mine: HashMap<String, String> = env::vars().collect();

        let ran = Spawner::new()
            .run("env".as_ref(), &[], Path::new("."))
            .expect("`env` prints the environment it was given");

        assert!(ran.success());
        let printed = ran.stdout_text();
        let mut seen = 0;
        for line in printed.lines() {
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            // A value with a newline in it prints over several lines, and only
            // the first of them is a name. Skipping the rest costs nothing:
            // something *added* would be a whole entry of its own.
            if name.is_empty()
                || !name
                    .chars()
                    .all(|letter| letter.is_ascii_alphanumeric() || letter == '_')
            {
                continue;
            }
            seen += 1;
            let Some(ours) = mine.get(name) else {
                panic!("the child was given `{name}`, which this process does not have");
            };
            if !ours.contains('\n') {
                assert_eq!(ours, value, "the child's `{name}` is not this process's");
            }
        }
        assert!(seen > 1, "`env` printed nothing worth checking: {printed}");
    }

    // A real [`Spawner`] pointed at a program of the test's choosing. The
    // missing-`gh` path is worth reaching through an actual failed lookup
    // rather than a scripted one: whether the operating system's answer is the
    // one the module turns into an outcome is exactly what a fake cannot say.
    struct Instead(&'static str);

    impl Runs for Instead {
        fn run(&self, _program: &OsStr, args: &[OsString], directory: &Path) -> Result<Ran, Error> {
            Spawner::new().run(self.0.as_ref(), args, directory)
        }
    }

    fn pull_request() -> PullRequest<'static> {
        PullRequest {
            base: "main",
            head: "war-137/run-git-and-gh-behind-a-seam",
            title: "WAR-137: Run git and gh behind a seam",
            body: "The pull request and the ticket's review state are the human gate.\n",
        }
    }

    #[test]
    fn a_gh_that_no_path_holds_is_an_outcome_rather_than_an_error() {
        let opened = Gh::new(Instead(NOT_A_PROGRAM), ".")
            .open_pull_request(pull_request())
            .expect("a machine with no `gh` on it has still done the work");

        assert_eq!(opened, Opened::NoGh);
        assert_eq!(opened.url(), None);
    }

    #[test]
    fn a_gh_that_is_there_and_fails_is_a_failure_and_not_the_missing_one() {
        // `/bin/sh pr create …` is a program that exists, runs, and says no —
        // the case the outcome above must not swallow.
        let error = Gh::new(Instead("/bin/sh"), ".")
            .open_pull_request(pull_request())
            .expect_err("a `gh` that ran and refused is a failure");

        match error {
            Error::Refused { command, code, .. } => {
                assert_eq!(
                    command,
                    "gh pr create --base main --head war-137/run-git-and-gh-behind-a-seam"
                );
                assert_ne!(code, Some(0));
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_failure_arrives_at_once_rather_than_after_the_timeout() {
        // The timeout is a backstop, not a delay every refusal pays.
        let started = Instant::now();
        let error = Spawner::new()
            .run(NOT_A_PROGRAM.as_ref(), &[], Path::new("."))
            .expect_err("this program does not exist");
        let elapsed = started.elapsed();

        assert!(matches!(error, Error::NotFound { .. }), "{error:?}");
        assert!(
            elapsed < Duration::from_secs(1),
            "a missing program took {elapsed:?} to report"
        );
    }
}

mod rendering {
    use super::super::{
        Finished, HUMAN_GATE, LeftStale, Touched, pull_request_body, pull_request_title,
    };

    #[test]
    fn a_title_is_the_ticket_then_its_title() {
        assert_eq!(
            pull_request_title("WAR-131", "The Linear queue query"),
            "WAR-131: The Linear queue query"
        );
    }

    #[test]
    fn a_title_carries_no_whitespace_from_the_board() {
        // A newline in `gh pr create --title` is a pull request nobody can find
        // by name again.
        assert_eq!(
            pull_request_title(" WAR-131\n", "The Linear queue query\n"),
            "WAR-131: The Linear queue query"
        );
    }

    #[test]
    fn a_body_gives_the_ticket_the_sub_tasks_what_was_crossed_and_what_is_stale() {
        let body = pull_request_body(
            "The queue query, ordered oldest first.\n\nEvery skipped ticket names its blocker.\n",
            &[
                Finished {
                    id: "WAR-131.01",
                    goal: "Add the issues query beside fetch_project",
                    summary: "`issues` sends one query and parses the blockers.",
                },
                Finished {
                    id: "WAR-131.02",
                    goal: "Pick the next ticket from the queue",
                    summary: "The oldest unblocked ticket wins; the rest are named with what holds them.",
                },
            ],
            &[Touched {
                scope: "control-plane",
                paths: vec!["crates/control/src/lib.rs", "crates/control/src/plane.rs"],
            }],
            &[LeftStale {
                directory: "crates/control",
                reason: "closed to this machine, which holds no `control-plane` sigil",
            }],
        );

        assert_eq!(
            body,
            "The queue query, ordered oldest first.\n\
             \n\
             Every skipped ticket names its blocker.\n\
             \n\
             ## Sub-tasks\n\
             \n\
             ### WAR-131.01 Add the issues query beside fetch_project\n\
             \n\
             `issues` sends one query and parses the blockers.\n\
             \n\
             ### WAR-131.02 Pick the next ticket from the queue\n\
             \n\
             The oldest unblocked ticket wins; the rest are named with what holds them.\n\
             \n\
             ## Scopes touched\n\
             \n\
             Held on this machine, and not the scope this ticket was pulled under.\n\
             \n\
             ### control-plane\n\
             \n\
             - `crates/control/src/lib.rs`\n\
             - `crates/control/src/plane.rs`\n\
             \n\
             ## Directories left stale\n\
             \n\
             - `crates/control` — closed to this machine, which holds no `control-plane` sigil\n\
             \n\
             The pull request and the ticket's review state are the human gate: warlock merges nothing, closes nothing and moves nothing past review.\n"
        );
        assert!(body.contains(HUMAN_GATE));
    }

    #[test]
    fn a_run_that_crossed_nothing_and_left_nothing_stale_has_no_headings_for_them() {
        let body = pull_request_body(
            "The queue query, ordered oldest first.",
            &[Finished {
                id: "WAR-131.01",
                goal: "Add the issues query beside fetch_project",
                summary: "`issues` sends one query and parses the blockers.",
            }],
            &[],
            &[],
        );

        assert_eq!(
            body,
            "The queue query, ordered oldest first.\n\
             \n\
             ## Sub-tasks\n\
             \n\
             ### WAR-131.01 Add the issues query beside fetch_project\n\
             \n\
             `issues` sends one query and parses the blockers.\n\
             \n\
             The pull request and the ticket's review state are the human gate: warlock merges nothing, closes nothing and moves nothing past review.\n"
        );
    }

    #[test]
    fn the_emptiest_run_still_says_where_the_human_gate_is() {
        // What a comment on the ticket looks like with nothing to report: one
        // sentence, and no blank lines or bare headings above it.
        let body = pull_request_body("", &[], &[], &[]);

        assert_eq!(body, format!("{HUMAN_GATE}\n"));
        assert!(body.contains(HUMAN_GATE));
    }

    #[test]
    fn a_sub_task_with_no_summary_keeps_its_heading_and_gains_no_blank_paragraph() {
        let body = pull_request_body(
            "",
            &[Finished {
                id: "WAR-131.01",
                goal: "Add the issues query beside fetch_project",
                summary: "   ",
            }],
            &[],
            &[],
        );

        assert_eq!(
            body,
            format!(
                "## Sub-tasks\n\
                 \n\
                 ### WAR-131.01 Add the issues query beside fetch_project\n\
                 \n\
                 {HUMAN_GATE}\n"
            )
        );
    }

    #[test]
    fn several_scopes_and_several_stale_directories_each_get_their_own_list() {
        let body = pull_request_body(
            "",
            &[],
            &[
                Touched {
                    scope: "control-plane",
                    paths: vec!["crates/control/src/lib.rs"],
                },
                Touched {
                    scope: "web",
                    paths: vec!["web/app.ts", "web/index.html"],
                },
            ],
            &[
                LeftStale {
                    directory: "crates/control",
                    reason: "closed to this machine",
                },
                LeftStale {
                    directory: "web",
                    reason: "the pass failed",
                },
            ],
        );

        assert_eq!(
            body,
            format!(
                "## Scopes touched\n\
                 \n\
                 Held on this machine, and not the scope this ticket was pulled under.\n\
                 \n\
                 ### control-plane\n\
                 \n\
                 - `crates/control/src/lib.rs`\n\
                 \n\
                 ### web\n\
                 \n\
                 - `web/app.ts`\n\
                 - `web/index.html`\n\
                 \n\
                 ## Directories left stale\n\
                 \n\
                 - `crates/control` — closed to this machine\n\
                 - `web` — the pass failed\n\
                 \n\
                 {HUMAN_GATE}\n"
            )
        );
    }
}

// Every operation, driven through a runner that records the argument vector and
// scripts the answer: no repository, no network, and no `git` on the machine.
mod operations {
    use std::collections::VecDeque;
    use std::ffi::{OsStr, OsString};
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use super::super::{
        Commit, Dirty, Error, Forge, Gh, Git, Head, Opened, PullRequest, Ran, Repository, Runs,
        branch_name, commit_message,
    };

    // The directory every call below has to have run in, and one no machine
    // has: a test that reached a real checkout would fail here first.
    const CHECKOUT: &str = "/warlock/no/such/checkout";

    #[derive(Debug, PartialEq, Eq)]
    struct Call {
        program: String,
        args: Vec<String>,
        directory: PathBuf,
    }

    #[derive(Default)]
    struct Shared {
        calls: Mutex<Vec<Call>>,
        answers: Mutex<VecDeque<Result<Ran, Error>>>,
    }

    // Cloned into the `Git` and kept by the test, so the vectors can be read
    // after the operation has taken its runner by value.
    #[derive(Clone, Default)]
    struct Fake {
        shared: Arc<Shared>,
    }

    impl Fake {
        fn new() -> Self {
            Self::default()
        }

        // An unscripted call answers with a clean success, so a test only says
        // what it is about.
        fn says(self, stdout: impl Into<Vec<u8>>) -> Self {
            self.answers(Ok(Ran::new(Some(0), stdout, "")))
        }

        fn refuses(self, code: i32, stderr: &str) -> Self {
            self.answers(Ok(Ran::new(Some(code), "", stderr)))
        }

        fn answers(self, answer: Result<Ran, Error>) -> Self {
            self.shared
                .answers
                .lock()
                .expect("answers")
                .push_back(answer);
            self
        }

        fn checkout(&self) -> Git<Self> {
            Git::new(self.clone(), CHECKOUT)
        }

        fn forge(&self) -> Gh<Self> {
            Gh::new(self.clone(), CHECKOUT)
        }

        fn vectors(&self) -> Vec<Vec<String>> {
            self.shared
                .calls
                .lock()
                .expect("calls")
                .iter()
                .map(|call| call.args.clone())
                .collect()
        }

        fn only(&self) -> Vec<String> {
            let mut vectors = self.vectors();
            assert_eq!(vectors.len(), 1, "expected exactly one call: {vectors:?}");
            vectors.remove(0)
        }
    }

    impl Runs for Fake {
        fn run(&self, program: &OsStr, args: &[OsString], directory: &Path) -> Result<Ran, Error> {
            self.shared.calls.lock().expect("calls").push(Call {
                program: program.to_string_lossy().into_owned(),
                args: args
                    .iter()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect(),
                directory: directory.to_path_buf(),
            });
            self.shared
                .answers
                .lock()
                .expect("answers")
                .pop_front()
                .unwrap_or_else(|| Ok(Ran::new(Some(0), "", "")))
        }
    }

    fn words(vector: &[String]) -> Vec<&str> {
        vector.iter().map(String::as_str).collect()
    }

    #[test]
    fn every_operation_runs_git_in_the_directory_it_was_given() {
        let fake = Fake::new().says("deadbeef\n");

        let _ = fake.checkout().head().expect("the scripted commit");

        let calls = fake.shared.calls.lock().expect("calls");
        assert_eq!(calls[0].program, "git");
        assert_eq!(calls[0].directory, Path::new(CHECKOUT));
    }

    #[test]
    fn a_clean_tree_is_an_empty_answer_to_one_status_call() {
        let fake = Fake::new().says("");

        let dirty = fake.checkout().dirty().expect("a scripted clean status");

        assert!(dirty.is_empty(), "{dirty:?}");
        assert_eq!(
            words(&fake.only()),
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"]
        );
    }

    #[test]
    fn a_dirty_tree_comes_back_as_the_paths_that_are_dirty() {
        // The bytes a real `git status --porcelain=v1 -z --untracked-files=all`
        // printed: a rename, a modification, and two untracked files, one of
        // them inside a directory that is itself new.
        let fake = Fake::new().says(
            &b"R  renamed.txt\0old.txt\0 M sub/new.txt\0?? fresh/deep.txt\0?? untracked.txt\0"[..],
        );

        let dirty = fake.checkout().dirty().expect("the scripted status");

        assert_eq!(
            dirty,
            vec![
                Dirty {
                    code: "R ".to_owned(),
                    path: "renamed.txt".to_owned(),
                    from: Some("old.txt".to_owned()),
                },
                Dirty {
                    code: " M".to_owned(),
                    path: "sub/new.txt".to_owned(),
                    from: None,
                },
                Dirty {
                    code: "??".to_owned(),
                    path: "fresh/deep.txt".to_owned(),
                    from: None,
                },
                Dirty {
                    code: "??".to_owned(),
                    path: "untracked.txt".to_owned(),
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn a_rename_names_both_sides_and_does_not_swallow_the_entry_after_it() {
        // The old path is a field of its own rather than part of the record, so
        // a reader that took every field for an entry would report `old.txt` as
        // dirty in its own right and lose the modification after it.
        let fake =
            Fake::new().says(&b"R  after.rs\0before.rs\0C  copy.rs\0source.rs\0 M last.rs\0"[..]);

        let dirty = fake.checkout().dirty().expect("the scripted status");

        assert_eq!(dirty.len(), 3);
        assert_eq!(dirty[0].path, "after.rs");
        assert_eq!(dirty[0].from.as_deref(), Some("before.rs"));
        assert_eq!(dirty[1].from.as_deref(), Some("source.rs"));
        assert_eq!(dirty[2].path, "last.rs");
        assert_eq!(dirty[2].from, None);
        assert_eq!(dirty[0].to_string(), "R  after.rs (was before.rs)");
    }

    #[test]
    fn a_status_that_cannot_be_run_is_the_refusal_git_gave() {
        let fake = Fake::new().refuses(128, "fatal: not a git repository\n");

        let error = fake
            .checkout()
            .dirty()
            .expect_err("a scripted refusal is not a clean tree");

        match error {
            Error::Refused {
                command,
                code,
                message,
            } => {
                assert_eq!(
                    command,
                    "git status --porcelain=v1 -z --untracked-files=all"
                );
                assert_eq!(code, Some(128));
                assert_eq!(message, "fatal: not a git repository");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_default_branch_is_read_off_the_local_head_ref() {
        let fake = Fake::new().says("origin/main\n");

        let branch = fake
            .checkout()
            .default_branch()
            .expect("the scripted default branch");

        assert_eq!(branch, "main");
        assert_eq!(
            words(&fake.only()),
            [
                "symbolic-ref",
                "--quiet",
                "--short",
                "refs/remotes/origin/HEAD"
            ]
        );
    }

    #[test]
    fn a_default_branch_that_is_not_main_is_read_as_what_it_is() {
        // The test that matters: nothing here knows the word `main`.
        for named in ["origin/master", "origin/trunk", "origin/develop"] {
            let fake = Fake::new().says(format!("{named}\n"));

            let branch = fake.checkout().default_branch().expect("the scripted ref");

            assert_eq!(branch, named.trim_start_matches("origin/"));
        }
    }

    #[test]
    fn an_unset_head_ref_falls_through_to_the_remote_rather_than_to_a_guess() {
        let fake = Fake::new().refuses(1, "").says(
            "* remote origin\n  Fetch URL: git@example.com:team/repo.git\n  HEAD branch: trunk\n",
        );

        let branch = fake
            .checkout()
            .default_branch()
            .expect("the remote names one");

        assert_eq!(branch, "trunk");
        let vectors = fake.vectors();
        assert_eq!(vectors.len(), 2);
        assert_eq!(words(&vectors[1]), ["remote", "show", "origin"]);
    }

    #[test]
    fn a_default_branch_nothing_names_is_reported_rather_than_guessed_past() {
        let fake = Fake::new()
            .refuses(1, "")
            .says("* remote origin\n  HEAD branch: (unknown)\n");

        let error = fake
            .checkout()
            .default_branch()
            .expect_err("nothing named a default branch");

        let said = error.to_string();
        assert!(
            !said.contains("main"),
            "the halt offered a guess instead of an answer: {said}"
        );
        assert!(
            said.contains("git remote set-head origin --auto"),
            "the halt says nothing the reader can do: {said}"
        );
        match error {
            Error::Unreadable { what, .. } => {
                assert_eq!(what, "the default branch of `origin`");
            }
            other => panic!("expected an unreadable default branch, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_git_stays_a_missing_program_rather_than_becoming_a_refusal() {
        let fake = Fake::new().answers(Err(Error::NotFound {
            program: "git".to_owned(),
        }));

        let error = fake.checkout().head().expect_err("there is no git here");

        assert!(matches!(error, Error::NotFound { .. }), "{error:?}");
    }

    #[test]
    fn head_is_read_as_the_commit_it_is_on() {
        let fake = Fake::new().says("4e724822589a889678ef4d920a03a69e67d977e8\n");

        let head = fake.checkout().head().expect("the scripted commit");

        assert_eq!(
            head,
            Commit::new("4e724822589a889678ef4d920a03a69e67d977e8")
        );
        assert_eq!(head.short(), "4e724822");
        assert_eq!(words(&fake.only()), ["rev-parse", "HEAD"]);
    }

    #[test]
    fn a_head_that_moved_is_reported_with_the_commit_it_moved_to() {
        let before = Fake::new().says("1111111111111111111111111111111111111111\n");
        let after = Fake::new().says("2222222222222222222222222222222222222222\n");

        let before = before.checkout().head().expect("the commit before");
        let after = after.checkout().head().expect("the commit after");

        assert_eq!(
            Head::between(&before, &after),
            Head::Moved {
                from: before.clone(),
                to: after.clone(),
            }
        );
        match Head::between(&before, &after) {
            Head::Moved { to, .. } => assert_eq!(to.short(), "22222222"),
            Head::Unmoved => panic!("the session committed and nobody noticed"),
        }
        assert_eq!(Head::between(&after, &after), Head::Unmoved);
    }

    #[test]
    fn a_recorded_branch_is_switched_to_and_a_new_one_is_cut_by_a_separate_call() {
        let resumed = Fake::new();
        resumed
            .checkout()
            .switch_to("war-137/a-halted-run")
            .expect("the branch is there");
        assert_eq!(words(&resumed.only()), ["switch", "war-137/a-halted-run"]);

        let fresh = Fake::new();
        fresh
            .checkout()
            .cut_branch("war-137/a-new-run", "main")
            .expect("the branch is cut");
        assert_eq!(
            words(&fresh.only()),
            ["switch", "--create", "war-137/a-new-run", "main"]
        );
    }

    #[test]
    fn the_default_branch_is_brought_up_to_date_by_fast_forward_only() {
        let fake = Fake::new();

        fake.checkout().catch_up("trunk").expect("a fast-forward");

        assert_eq!(
            words(&fake.only()),
            ["pull", "--ff-only", "origin", "trunk"]
        );
    }

    #[test]
    fn a_commit_stages_everything_and_then_commits_the_message_given_whole() {
        let fake = Fake::new();
        let message = commit_message(
            "WAR-137",
            "WAR-137.02",
            "Render the pull request title and body from structured inputs",
        );

        fake.checkout()
            .commit_all(&message)
            .expect("both calls succeed");

        let vectors = fake.vectors();
        assert_eq!(words(&vectors[0]), ["add", "-A"]);
        assert_eq!(
            vectors[1],
            vec![
                "commit".to_owned(),
                "-m".to_owned(),
                "WAR-137 WAR-137.02: Render the pull request title and body from structured inputs"
                    .to_owned(),
            ]
        );
    }

    #[test]
    fn a_commit_message_is_the_ticket_the_sub_task_and_the_goal() {
        assert_eq!(
            commit_message("WAR-137", "WAR-137.02", "Render the pull request"),
            "WAR-137 WAR-137.02: Render the pull request"
        );
        // A goal read off a brief arrives with the line it was on.
        assert_eq!(
            commit_message(" WAR-137 ", "WAR-137.02\n", "Render the pull request\n"),
            "WAR-137 WAR-137.02: Render the pull request"
        );
        // A goal that wrapped is one line in a commit message: a blank second
        // line would make the rest of it a body.
        assert_eq!(
            commit_message(
                "WAR-137",
                "WAR-137.03",
                "Put every git operation\nbehind a seam"
            ),
            "WAR-137 WAR-137.03: Put every git operation behind a seam"
        );
    }

    #[test]
    fn a_commit_with_nothing_to_commit_is_the_refusal_and_what_git_said_on_stdout() {
        // `git commit` complains on stdout and exits non-zero, so an error that
        // only read stderr would be empty.
        let fake = Fake::new().says("").answers(Ok(Ran::new(
            Some(1),
            "nothing to commit, working tree clean\n",
            "",
        )));

        let error = fake
            .checkout()
            .commit_all("WAR-137 WAR-137.03: A sub-task that changed nothing")
            .expect_err("git refused");

        match error {
            Error::Refused { message, .. } => {
                assert_eq!(message, "nothing to commit, working tree clean");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_branch_s_changed_paths_are_the_merge_base_diff_of_the_local_default_branch() {
        // Two dots would be the difference between the tips, so whatever landed
        // on `main` after this branch was cut would arrive as a path the branch
        // changed; three is the merge base.
        let fake = Fake::new().says(&b"crates/engine/src/read.rs\0"[..]);

        let changed = fake
            .checkout()
            .changed_against("main")
            .expect("the scripted diff");

        assert_eq!(changed, ["crates/engine/src/read.rs"]);
        assert_eq!(
            words(&fake.only()),
            [
                "diff",
                "--name-only",
                "--no-renames",
                "-z",
                "main...HEAD",
                "--"
            ]
        );
    }

    #[test]
    fn a_default_branch_that_is_not_main_is_the_one_the_diff_is_taken_against() {
        for named in ["master", "trunk", "develop"] {
            let fake = Fake::new().says("");

            fake.checkout()
                .changed_against(named)
                .expect("the scripted diff");

            assert_eq!(words(&fake.only())[4], format!("{named}...HEAD"));
        }
    }

    #[test]
    fn a_deletion_and_both_sides_of_a_rename_reach_the_caller() {
        // What `--no-renames` buys: the file that left `crates/engine` is named
        // there as well as where it landed, so the directory it left is known
        // to have changed. With rename detection on, `--name-only` would print
        // the new path alone and that directory would look untouched.
        let fake = Fake::new().says(
            &b"crates/engine/src/gone.rs\0crates/engine/src/old.rs\0crates/tui/src/new.rs\0"[..],
        );

        let changed = fake
            .checkout()
            .changed_against("main")
            .expect("the scripted diff");

        assert_eq!(
            changed,
            [
                "crates/engine/src/gone.rs",
                "crates/engine/src/old.rs",
                "crates/tui/src/new.rs",
            ]
        );
    }

    #[test]
    fn a_branch_that_changed_nothing_is_an_empty_answer_and_not_a_failure() {
        let fake = Fake::new().says("");

        let changed = fake
            .checkout()
            .changed_against("main")
            .expect("an empty diff is an answer");

        assert!(changed.is_empty(), "{changed:?}");
    }

    #[test]
    fn a_diff_git_refused_is_the_refusal_and_never_an_empty_list_of_paths() {
        // The opposite answers: "the diff failed" and "the branch changed
        // nothing" would leave every stale directory unrefreshed and say so
        // nowhere.
        let fake = Fake::new().refuses(128, "fatal: ambiguous argument 'main...HEAD'\n");

        let error = fake
            .checkout()
            .changed_against("main")
            .expect_err("a scripted refusal is not an unchanged branch");

        match error {
            Error::Refused { command, code, .. } => {
                assert_eq!(
                    command,
                    "git diff --name-only --no-renames -z main...HEAD --"
                );
                assert_eq!(code, Some(128));
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_documents_only_commit_names_its_paths_on_the_add_and_on_the_commit() {
        let fake = Fake::new();
        let paths = [
            "crates/engine/WARLOCK.md".to_owned(),
            ".warlock/pacts.toml".to_owned(),
        ];

        fake.checkout()
            .commit_paths("WAR-141: refresh WARLOCK.md", &paths)
            .expect("both calls succeed");

        let vectors = fake.vectors();
        assert_eq!(
            words(&vectors[0]),
            [
                "add",
                "--",
                "crates/engine/WARLOCK.md",
                ".warlock/pacts.toml"
            ]
        );
        // The pathspec on the commit is what keeps code out of it: anything
        // else the index holds is not in this commit.
        assert_eq!(
            words(&vectors[1]),
            [
                "commit",
                "-m",
                "WAR-141: refresh WARLOCK.md",
                "--",
                "crates/engine/WARLOCK.md",
                ".warlock/pacts.toml",
            ]
        );
    }

    #[test]
    fn a_commit_of_named_paths_stages_nothing_else_and_carries_no_sweep() {
        let fake = Fake::new();

        fake.checkout()
            .commit_paths("WAR-141: refresh WARLOCK.md", &["a/WARLOCK.md".to_owned()])
            .expect("both calls succeed");

        for vector in fake.vectors() {
            assert!(
                !words(&vector).contains(&"-A"),
                "a documents-only commit never stages everything: {vector:?}"
            );
        }
    }

    #[test]
    fn a_commit_of_no_paths_is_refused_before_any_git_runs() {
        // `git commit -m <message> --` with an empty pathspec commits the index,
        // which is the one way this call could sweep in code.
        let fake = Fake::new();

        let error = fake
            .checkout()
            .commit_paths("WAR-141: refresh WARLOCK.md", &[])
            .expect_err("a commit of nothing is not a commit");

        assert!(matches!(error, Error::Empty { .. }), "{error:?}");
        assert!(fake.vectors().is_empty(), "{:?}", fake.vectors());
    }

    #[test]
    fn a_refresh_commit_with_nothing_to_commit_is_the_refusal_git_gave() {
        let fake = Fake::new().says("").answers(Ok(Ran::new(
            Some(1),
            "nothing to commit, working tree clean\n",
            "",
        )));

        let error = fake
            .checkout()
            .commit_paths(
                "WAR-141: refresh WARLOCK.md",
                &[".warlock/pacts.toml".to_owned()],
            )
            .expect_err("git refused");

        match error {
            Error::Refused { message, .. } => {
                assert_eq!(message, "nothing to commit, working tree clean");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_push_sets_the_upstream_and_carries_no_force() {
        let fake = Fake::new();

        fake.checkout()
            .publish("war-137/run-git-and-gh-behind-a-seam")
            .expect("the push succeeds");

        assert_eq!(
            words(&fake.only()),
            [
                "push",
                "--set-upstream",
                "origin",
                "war-137/run-git-and-gh-behind-a-seam"
            ]
        );
    }

    #[test]
    fn a_pull_request_is_opened_against_the_default_branch_with_the_branch_as_head() {
        let fake = Fake::new().says("https://github.com/team/repo/pull/12\n");

        let opened = fake
            .forge()
            .open_pull_request(PullRequest {
                base: "trunk",
                head: BRANCH,
                title: TITLE,
                body: BODY,
            })
            .expect("the scripted pull request");

        assert_eq!(
            opened,
            Opened::At {
                url: "https://github.com/team/repo/pull/12".to_owned()
            }
        );
        assert_eq!(opened.url(), Some("https://github.com/team/repo/pull/12"));

        let calls = fake.shared.calls.lock().expect("calls");
        assert_eq!(calls[0].program, "gh");
        assert_eq!(calls[0].directory, Path::new(CHECKOUT));
        assert_eq!(
            calls[0].args,
            vec![
                "pr".to_owned(),
                "create".to_owned(),
                "--base".to_owned(),
                // The detected default branch, not `gh`'s own idea of one, and
                // never `main`.
                "trunk".to_owned(),
                "--head".to_owned(),
                BRANCH.to_owned(),
                "--title".to_owned(),
                TITLE.to_owned(),
                "--body".to_owned(),
                // Whole, with its blank lines and its markdown: a body is one
                // argument and never a shell word.
                BODY.to_owned(),
            ]
        );
    }

    #[test]
    fn the_url_is_the_last_one_gh_printed_and_not_the_first_thing_that_looked_like_one() {
        let fake = Fake::new().says(
            "Creating pull request for war-137/work into main in team/repo\n\
             see https://docs.github.com/pull-requests\n\
             \n\
             https://github.com/team/repo/pull/12\n",
        );

        let opened = fake
            .forge()
            .open_pull_request(request())
            .expect("the scripted pull request");

        assert_eq!(opened.url(), Some("https://github.com/team/repo/pull/12"));
    }

    #[test]
    fn a_gh_no_path_holds_is_an_outcome_and_a_gh_that_refuses_is_a_failure() {
        // The distinction the whole variant exists for. Absent: the work is
        // done, the branch is pushed, and the caller comments the body on the
        // ticket instead. Present and saying no: something went wrong and the
        // run has to be told.
        let absent = Fake::new().answers(Err(Error::NotFound {
            program: "gh".to_owned(),
        }));
        let refusing = Fake::new().refuses(1, "pull request already exists for this branch\n");

        let opened = absent
            .forge()
            .open_pull_request(request())
            .expect("a missing `gh` is not a failed run");
        let error = refusing
            .forge()
            .open_pull_request(request())
            .expect_err("a `gh` that ran and said no is a failed run");

        assert_eq!(opened, Opened::NoGh);
        match error {
            Error::Refused {
                command,
                code,
                message,
            } => {
                // The two branches and not the body: a halt that quoted the
                // rendered document back would bury what `gh` said under it.
                assert_eq!(command, format!("gh pr create --base main --head {BRANCH}"));
                assert_eq!(code, Some(1));
                assert_eq!(message, "pull request already exists for this branch");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_gh_that_opened_something_and_said_where_is_not_a_success_without_the_url() {
        let fake = Fake::new().says("Creating pull request for war-137/work into main\n");

        let error = fake
            .forge()
            .open_pull_request(request())
            .expect_err("there is no pull request to report");

        match error {
            Error::Unreadable { what, saw } => {
                assert_eq!(what, "the pull request's URL");
                assert_eq!(saw, "Creating pull request for war-137/work into main");
            }
            other => panic!("expected an unreadable URL, got {other:?}"),
        }
    }

    const BASE: &str = "main";
    const BRANCH: &str = "war-137/run-git-and-gh-behind-a-seam";
    const MESSAGE: &str = "WAR-137 WAR-137.04: Open the pull request behind the same seam";
    const TITLE: &str = "WAR-137: Run git and gh behind a seam";
    const BODY: &str = "## Sub-tasks\n\n### WAR-137.04 Open the pull request\n";

    fn request() -> PullRequest<'static> {
        PullRequest {
            base: BASE,
            head: BRANCH,
            title: TITLE,
            body: BODY,
        }
    }

    // Every operation the module has, `git` and `gh` alike, driven once, and
    // every argument vector the lot of them produced. Both audits below read
    // it: the rules they check are about the whole module, and a rule proved
    // against the one operation it is easiest to break in is a rule a tenth
    // operation walks straight past.
    fn every_vector() -> Vec<Vec<String>> {
        let mut fake = Fake::new()
            .says("")
            .says("origin/main\n")
            .says("1111111111111111111111111111111111111111\n");
        // The six plain successes between the three answers above and the URL
        // below: `switch`, `pull`, `switch --create`, `add`, `commit`, `push`.
        for _ in 0..6 {
            fake = fake.says("");
        }
        let fake = fake.says("https://github.com/team/repo/pull/12\n");

        let checkout = fake.checkout();
        let _ = checkout.dirty().expect("a clean tree");
        let _ = checkout.default_branch().expect("a default branch");
        let _ = checkout.head().expect("a commit");
        checkout.switch_to(BRANCH).expect("a switch");
        checkout.catch_up(BASE).expect("a fast-forward");
        checkout.cut_branch(BRANCH, BASE).expect("a cut");
        checkout.commit_all(MESSAGE).expect("a commit");
        checkout.publish(BRANCH).expect("a push");
        let _ = fake
            .forge()
            .open_pull_request(request())
            .expect("a pull request");

        let vectors = fake.vectors();
        assert_eq!(
            vectors.len(),
            10,
            "an operation was added or lost: {vectors:?}"
        );
        vectors
    }

    #[test]
    fn nothing_the_module_runs_stashes_resets_cleans_merges_forces_or_deletes() {
        for vector in every_vector() {
            for argument in &vector {
                assert!(
                    !matches!(
                        argument.as_str(),
                        "stash"
                            | "reset"
                            | "clean"
                            | "merge"
                            | "rebase"
                            | "revert"
                            | "checkout"
                            | "-f"
                            | "-d"
                            | "-D"
                            | "-u"
                    ),
                    "`{}` is not warlock's to run",
                    vector.join(" ")
                );
                assert!(
                    !argument.starts_with("--force")
                        && argument != "--delete"
                        && argument != "--amend"
                        && argument != "--hard",
                    "`{}` carries a flag this module never passes",
                    vector.join(" ")
                );
            }
        }
    }

    #[test]
    fn no_child_is_given_a_word_neither_the_module_nor_its_caller_supplied() {
        // The arguments half of the no-key rule; the environment half is in
        // `unix`. Nothing in this module is ever handed a key — there is
        // nowhere in it for one to arrive — and this is what keeps it so. An
        // operation that grew a `-c http.extraheader=…`, read a token out of
        // the environment behind its caller's back, or spliced one into a
        // remote URL would put a word in one of these vectors that is neither
        // `git`'s, `gh`'s, nor one of the five the test passed in.
        const OWN_WORDS: [&str; 29] = [
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
            "remote",
            "show",
            "origin",
            "switch",
            "--create",
            "pull",
            "--ff-only",
            "rev-parse",
            "HEAD",
            "add",
            "-A",
            "commit",
            "-m",
            "push",
            "--set-upstream",
            "pr",
            "create",
            "--base",
            "--head",
            "--title",
            "--body",
        ];
        let passed_in = [BASE, BRANCH, MESSAGE, TITLE, BODY];

        for vector in every_vector() {
            for argument in &vector {
                assert!(
                    OWN_WORDS.contains(&argument.as_str())
                        || passed_in.contains(&argument.as_str()),
                    "`{}` carries `{argument}`, which came from neither the caller nor the module",
                    vector.join(" ")
                );
            }
        }
    }

    #[test]
    fn a_branch_name_is_the_ticket_lowercased_then_the_title_slugged() {
        assert_eq!(
            branch_name("WAR", 137, "Run git and gh behind a seam"),
            "war-137/run-git-and-gh-behind-a-seam"
        );
    }

    #[test]
    fn a_title_with_punctuation_mixed_case_and_runs_of_spaces_slugs_to_single_hyphens() {
        assert_eq!(
            branch_name("WAR", 42, "  Cut   the  Branch: commits, push & THE PR!  "),
            "war-42/cut-the-branch-commits-push-the-pr"
        );
        assert_eq!(
            branch_name("war", 7, "A/B — testing (v2.0)"),
            "war-7/a-b-testing-v2-0"
        );
    }

    #[test]
    fn a_long_title_is_cut_at_a_word_rather_than_mid_word() {
        let name = branch_name(
            "WAR",
            137,
            "Run git and gh behind a seam: branch, commits, push, pull request",
        );

        assert_eq!(
            name,
            "war-137/run-git-and-gh-behind-a-seam-branch-commits-push"
        );
        assert!(!name.ends_with('-'));
    }

    #[test]
    fn a_first_word_longer_than_the_cap_is_kept_whole_because_half_of_it_is_not_a_name() {
        let long = "Supercalifragilisticexpialidociousandthensomemoreforgoodmeasureindeed yes";

        assert_eq!(
            branch_name("WAR", 1, long),
            "war-1/supercalifragilisticexpialidociousandthensomemoreforgoodmeasureindeed"
        );
    }

    #[test]
    fn a_title_that_slugs_to_nothing_still_makes_a_branch_a_human_can_type() {
        // `war-137/` is not a ref, and a ticket titled in a script this rule
        // drops entirely is still a ticket somebody pulled.
        assert_eq!(branch_name("WAR", 137, "  —  !!  "), "war-137/untitled");
        assert_eq!(branch_name("WAR", 137, ""), "war-137/untitled");
    }
}
