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
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use std::{env, fs, process, thread};

    use super::super::{Error, Runs, Spawner};
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
