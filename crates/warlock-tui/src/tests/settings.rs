use std::fs;
use std::path::Path;

use warlock_engine::{
    held_base_branch, held_sigils, load_key_binding, save_base_branch, save_key_binding,
    save_sigils, sigils_path,
};

use super::{Settings, sigils_line, sigils_under};
use crate::app::{Held, Sigils};
use crate::error::Error;
use crate::submission::{Branching, Sigiling};

fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn held(home: &Path, root: &Path) -> Vec<String> {
    held_sigils(home, root).expect("the config reads")
}

fn settings(home: &Path, root: &Path) -> Settings {
    Settings::at(Some(home.to_path_buf()), root)
}

fn broken(home: &Path, root: &Path) {
    let path = sigils_path(home, root);
    fs::create_dir_all(
        path.parent()
            .expect("the config lives in a project directory"),
    )
    .expect("the project directory");
    fs::write(&path, "not a config\n").expect("a file that is not TOML");
}

#[test]
fn add_keeps_what_is_held_folds_case_and_skips_repeats() {
    let (home, root) = (a_dir(), a_dir());
    save_sigils(home.path(), root.path(), &names(&["docs"])).unwrap();

    let said = settings(home.path(), root.path())
        .sigil(&Sigiling::Add(names(&["Warlock-Team", "docs", "*"])))
        .expect("every name is a sigil");

    assert_eq!(said, "holding `docs`, `warlock-team`, `*`");
    assert_eq!(
        held(home.path(), root.path()),
        names(&["docs", "warlock-team", "*"])
    );
}

#[test]
fn one_bad_name_writes_nothing() {
    let (home, root) = (a_dir(), a_dir());
    save_sigils(home.path(), root.path(), &names(&["docs"])).unwrap();

    let refused =
        settings(home.path(), root.path()).sigil(&Sigiling::Add(names(&["fine", "not ok"])));

    assert!(matches!(refused, Err(Error::Sigil { .. })), "{refused:?}");
    assert_eq!(held(home.path(), root.path()), names(&["docs"]));
}

#[test]
fn remove_takes_out_what_is_held_and_refuses_what_is_not() {
    let (home, root) = (a_dir(), a_dir());
    save_sigils(home.path(), root.path(), &names(&["docs", "warlock-team"])).unwrap();
    let settings = settings(home.path(), root.path());

    let refused = settings.sigil(&Sigiling::Remove(names(&["docs", "elsewhere"])));
    assert!(
        matches!(&refused, Err(Error::UnheldSigil { sigil }) if sigil == "elsewhere"),
        "{refused:?}"
    );
    assert_eq!(
        held(home.path(), root.path()),
        names(&["docs", "warlock-team"])
    );

    let removed = settings.sigil(&Sigiling::Remove(names(&["DOCS"]))).unwrap();
    assert_eq!(removed, "holding `warlock-team`");
    assert_eq!(held(home.path(), root.path()), names(&["warlock-team"]));
}

#[test]
fn clear_holds_nothing() {
    let (home, root) = (a_dir(), a_dir());
    save_sigils(home.path(), root.path(), &names(&["docs"])).unwrap();

    assert_eq!(
        settings(home.path(), root.path())
            .sigil(&Sigiling::Clear)
            .unwrap(),
        "holding nothing"
    );
    assert_eq!(held(home.path(), root.path()), Vec::<String>::new());
}

#[test]
fn show_names_what_is_held_and_writes_nothing() {
    let (home, root) = (a_dir(), a_dir());
    let settings = settings(home.path(), root.path());

    assert_eq!(settings.sigil(&Sigiling::Show).unwrap(), "holding nothing");
    assert!(!sigils_path(home.path(), root.path()).exists());

    save_sigils(home.path(), root.path(), &names(&["docs"])).unwrap();
    assert_eq!(settings.sigil(&Sigiling::Show).unwrap(), "holding `docs`");
}

#[test]
fn a_config_that_will_not_read_is_refused_rather_than_written_over() {
    let (home, root) = (a_dir(), a_dir());
    broken(home.path(), root.path());
    let settings = settings(home.path(), root.path());

    for sigiling in [Sigiling::Show, Sigiling::Add(names(&["docs"]))] {
        let refused = settings.sigil(&sigiling);
        assert!(matches!(refused, Err(Error::Sigils { .. })), "{refused:?}");
    }
    assert_eq!(
        fs::read_to_string(sigils_path(home.path(), root.path())).unwrap(),
        "not a config\n"
    );
}

#[test]
fn no_home_refuses_every_sigil_command_in_standing_s_words() {
    let settings = Settings::at(None, "/warlock/no/such/repository");

    for sigiling in [Sigiling::Show, Sigiling::Clear] {
        assert!(matches!(settings.sigil(&sigiling), Err(Error::NoHome)));
    }
    assert!(matches!(
        settings.branch(&Branching::Use("dev".to_owned()), None),
        Err(Error::NoHome)
    ));
    // Naming the base needs no file: with none, pulls go where the remote says.
    assert_eq!(
        settings.branch(&Branching::Show, Some("main")).unwrap(),
        "pulls start from and open pull requests against the remote's default branch, `main`"
    );
}

#[test]
fn branch_use_sets_the_base_and_clear_goes_back_to_the_remotes_default() {
    let (home, root) = (a_dir(), a_dir());
    let settings = settings(home.path(), root.path());

    let said = settings
        .branch(&Branching::Use("develop".to_owned()), Some("main"))
        .expect("sets the base");
    assert_eq!(
        said,
        "pulls start from and open pull requests against `develop`"
    );
    assert_eq!(
        held_base_branch(home.path(), root.path())
            .expect("reads")
            .as_deref(),
        Some("develop")
    );
    assert_eq!(
        settings.branch(&Branching::Show, Some("main")).unwrap(),
        said
    );

    let said = settings
        .branch(&Branching::Clear, None)
        .expect("clears the base");
    assert!(said.ends_with("the remote's default branch"), "{said}");
    assert_eq!(
        held_base_branch(home.path(), root.path()).expect("reads"),
        None
    );
}

#[test]
fn setting_the_base_keeps_the_key_binding() {
    let (home, root) = (a_dir(), a_dir());
    save_key_binding(home.path(), root.path(), "work").expect("binds a key");

    settings(home.path(), root.path())
        .branch(&Branching::Use("develop".to_owned()), None)
        .expect("sets the base");

    assert_eq!(
        load_key_binding(home.path(), root.path())
            .expect("reads")
            .as_deref(),
        Some("work")
    );
}

#[test]
fn a_branch_name_with_a_space_is_refused_and_nothing_is_written() {
    let (home, root) = (a_dir(), a_dir());

    let refused =
        settings(home.path(), root.path()).branch(&Branching::Use("my branch".to_owned()), None);

    assert!(matches!(refused, Err(Error::Sigils { .. })), "{refused:?}");
    assert_eq!(
        held_base_branch(home.path(), root.path()).expect("reads"),
        None
    );
}

#[test]
fn written_says_which_checkout_and_which_file() {
    let (home, root) = (a_dir(), a_dir());
    let said = settings(home.path(), root.path()).written("holding `docs`");

    assert!(said.starts_with("holding `docs` for `"), "{said}");
    assert!(said.ends_with("config.toml`"), "{said}");
}

#[test]
fn a_snapshot_reads_both_settings_and_changed_waits_for_the_file_to_move() {
    let (home, root) = (a_dir(), a_dir());
    let mut settings = settings(home.path(), root.path());

    assert_eq!(settings.changed(), Some(Held::default()));
    assert_eq!(settings.changed(), None);

    save_sigils(home.path(), root.path(), &names(&["docs"])).unwrap();
    save_base_branch(home.path(), root.path(), Some("dev")).unwrap();
    assert_eq!(
        settings.changed(),
        Some(Held {
            sigils: Sigils::held(["docs"]),
            base: Some("dev".to_owned()),
        })
    );
}

#[test]
fn a_bare_pull_names_the_scopes_held_or_says_how_to_hold_one() {
    let (home, root) = (a_dir(), a_dir());
    let settings = settings(home.path(), root.path());

    let none = settings.pull_line().unwrap();
    assert!(none.contains("`/sigil add`"), "{none}");

    save_sigils(home.path(), root.path(), &names(&["docs", "web"])).unwrap();
    assert_eq!(
        settings.pull_line().unwrap(),
        "`/pull` takes a scope: this machine holds `docs`, `web`"
    );
}

#[test]
fn a_machine_that_never_ran_warlock_holds_nothing() {
    let (home, repo) = (a_dir(), a_dir());

    assert_eq!(
        sigils_under(Some(home.path()), repo.path()),
        Sigils::Nothing
    );
    assert_eq!(sigils_under(None, repo.path()), Sigils::Nothing);
}

#[test]
fn the_sigils_on_disk_are_the_sigils_the_header_states() {
    let (home, repo) = (a_dir(), a_dir());
    let held = names(&["billing", "web"]);
    save_sigils(home.path(), repo.path(), &held).expect("a config that writes");

    assert_eq!(
        sigils_under(Some(home.path()), repo.path()),
        Sigils::Held(held)
    );
}

#[test]
fn a_config_holding_the_empty_set_is_the_same_nothing_as_no_config_at_all() {
    let (home, repo) = (a_dir(), a_dir());
    save_sigils(home.path(), repo.path(), &[]).expect("a config that writes");

    assert_eq!(
        sigils_under(Some(home.path()), repo.path()),
        Sigils::Nothing
    );
}

#[test]
fn a_config_that_will_not_parse_is_said_rather_than_ending_the_session() {
    // Broken is never drawn as absent: the two mean opposite things about what
    // is on disk.
    let (home, repo) = (a_dir(), a_dir());
    broken(home.path(), repo.path());

    assert_eq!(
        sigils_under(Some(home.path()), repo.path()),
        Sigils::Unknown
    );
}

#[test]
fn the_holding_line_names_each_sigil_nothing_or_the_file_that_would_not_read() {
    assert_eq!(sigils_line(&Sigils::Nothing, None), "holding nothing");
    assert_eq!(
        sigils_line(&Sigils::held(["a", "b"]), None),
        "holding `a`, `b`"
    );
    assert_eq!(sigils_line(&Sigils::Unknown, None), "holding unknown");
    assert_eq!(
        sigils_line(&Sigils::Unknown, Some(Path::new("/h/config.toml"))),
        "holding unknown: `/h/config.toml` could not be read"
    );
}
