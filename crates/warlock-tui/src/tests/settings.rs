use std::fs;
use std::path::Path;

use warlock_engine::{
    config_path, held_base_branch, load_key_binding, open_scopes, save_base_branch,
    save_key_binding, save_open_scopes,
};

use super::{Said, Settings, open_line};
use crate::app::{Held, OpenScopes};
use crate::error::Error;
use crate::submission::{Branching, Opening};

fn a_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}

fn names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

fn opened(home: &Path, root: &Path) -> Vec<String> {
    open_scopes(home, root).expect("the config reads")
}

fn settings(home: &Path, root: &Path) -> Settings {
    Settings::at(Some(home.to_path_buf()), root)
}

fn broken(home: &Path, root: &Path) {
    let path = config_path(home, root);
    fs::create_dir_all(
        path.parent()
            .expect("the config lives in a project directory"),
    )
    .expect("the project directory");
    fs::write(&path, "not a config\n").expect("a file that is not TOML");
}

#[test]
fn open_keeps_what_is_open_folds_case_and_skips_repeats() {
    let (home, root) = (a_dir(), a_dir());
    save_open_scopes(home.path(), root.path(), &names(&["docs"])).unwrap();

    let said = settings(home.path(), root.path())
        .open(&Opening::Open(names(&["Warlock-Team", "docs", "*"])))
        .expect("every name is a scope or the wildcard");

    assert_eq!(
        said,
        Said::Wrote("AI can work in `docs`, `warlock-team`, `*`".to_owned())
    );
    assert_eq!(
        opened(home.path(), root.path()),
        names(&["docs", "warlock-team", "*"])
    );
}

#[test]
fn one_bad_name_writes_nothing() {
    let (home, root) = (a_dir(), a_dir());
    save_open_scopes(home.path(), root.path(), &names(&["docs"])).unwrap();

    let refused =
        settings(home.path(), root.path()).open(&Opening::Open(names(&["fine", "not ok"])));

    assert!(
        matches!(refused, Err(Error::ScopeName { .. })),
        "{refused:?}"
    );
    assert_eq!(opened(home.path(), root.path()), names(&["docs"]));
}

#[test]
fn close_takes_out_what_is_open_and_refuses_what_is_not() {
    let (home, root) = (a_dir(), a_dir());
    save_open_scopes(home.path(), root.path(), &names(&["docs", "warlock-team"])).unwrap();
    let settings = settings(home.path(), root.path());

    let refused = settings.open(&Opening::Close(names(&["docs", "elsewhere"])));
    assert!(
        matches!(&refused, Err(Error::NotOpen { scope }) if scope == "elsewhere"),
        "{refused:?}"
    );
    assert_eq!(
        opened(home.path(), root.path()),
        names(&["docs", "warlock-team"])
    );

    let removed = settings.open(&Opening::Close(names(&["DOCS"]))).unwrap();
    assert_eq!(removed.line(), "AI can work in `warlock-team`");
    assert_eq!(opened(home.path(), root.path()), names(&["warlock-team"]));
}

#[test]
fn show_names_what_is_open_and_writes_nothing() {
    let (home, root) = (a_dir(), a_dir());
    let settings = settings(home.path(), root.path());

    assert_eq!(
        settings.open(&Opening::Show).unwrap(),
        Said::Shown("AI can work in no scope".to_owned())
    );
    assert!(!config_path(home.path(), root.path()).exists());

    save_open_scopes(home.path(), root.path(), &names(&["docs"])).unwrap();
    assert_eq!(
        settings.open(&Opening::Show).unwrap().line(),
        "AI can work in `docs`"
    );
}

#[test]
fn a_config_that_will_not_read_is_refused_rather_than_written_over() {
    let (home, root) = (a_dir(), a_dir());
    broken(home.path(), root.path());
    let settings = settings(home.path(), root.path());

    for opening in [Opening::Show, Opening::Open(names(&["docs"]))] {
        let refused = settings.open(&opening);
        assert!(matches!(refused, Err(Error::Config { .. })), "{refused:?}");
    }
    assert_eq!(
        fs::read_to_string(config_path(home.path(), root.path())).unwrap(),
        "not a config\n"
    );
}

#[test]
fn no_home_refuses_every_open_and_close_in_standing_s_words() {
    let settings = Settings::at(None, "/warlock/no/such/repository");

    for opening in [Opening::Show, Opening::Close(names(&["docs"]))] {
        assert!(matches!(settings.open(&opening), Err(Error::NoHome)));
    }
    assert!(matches!(
        settings.branch(&Branching::Use("dev".to_owned()), None),
        Err(Error::NoHome)
    ));
    // Naming the base needs no file: with none, pulls go where the remote says.
    assert_eq!(
        settings.branch(&Branching::Show, Some("main")).unwrap(),
        Said::Shown(
            "pulls start from and open pull requests against the remote's default branch, `main`"
                .to_owned()
        )
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
        Said::Wrote("pulls start from and open pull requests against `develop`".to_owned())
    );
    assert_eq!(
        held_base_branch(home.path(), root.path())
            .expect("reads")
            .as_deref(),
        Some("develop")
    );
    assert_eq!(
        settings
            .branch(&Branching::Show, Some("main"))
            .unwrap()
            .line(),
        said.line()
    );

    let said = settings
        .branch(&Branching::Clear, None)
        .expect("clears the base");
    assert!(
        said.line().ends_with("the remote's default branch"),
        "{said:?}"
    );
    assert_eq!(
        held_base_branch(home.path(), root.path()).expect("reads"),
        None
    );
}

#[test]
fn setting_the_base_keeps_the_key_binding() {
    let (home, root) = (a_dir(), a_dir());
    save_key_binding(home.path(), root.path(), "data-plane", "work").expect("binds a key");

    settings(home.path(), root.path())
        .branch(&Branching::Use("develop".to_owned()), None)
        .expect("sets the base");

    assert_eq!(
        load_key_binding(home.path(), root.path(), "data-plane")
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

    assert!(matches!(refused, Err(Error::Config { .. })), "{refused:?}");
    assert_eq!(
        held_base_branch(home.path(), root.path()).expect("reads"),
        None
    );
}

#[test]
fn written_says_which_checkout_and_which_file() {
    let (home, root) = (a_dir(), a_dir());
    let said = settings(home.path(), root.path()).written("AI can work in `docs`");

    assert!(said.starts_with("AI can work in `docs` for `"), "{said}");
    assert!(said.ends_with("config.toml`"), "{said}");
}

#[test]
fn a_snapshot_reads_both_settings_and_changed_waits_for_the_file_to_move() {
    let (home, root) = (a_dir(), a_dir());
    let mut settings = settings(home.path(), root.path());

    assert_eq!(settings.changed(), Some(Held::default()));
    assert_eq!(settings.changed(), None);

    save_open_scopes(home.path(), root.path(), &names(&["docs"])).unwrap();
    save_base_branch(home.path(), root.path(), Some("dev")).unwrap();
    assert_eq!(
        settings.changed(),
        Some(Held {
            open: OpenScopes::named(["docs"]),
            base: Some("dev".to_owned()),
        })
    );
}

#[test]
fn a_bare_pull_names_the_open_scopes_or_says_how_to_open_one() {
    let (home, root) = (a_dir(), a_dir());
    let settings = settings(home.path(), root.path());

    let none = settings.pull_line().unwrap();
    assert!(none.contains("`/open <scope>`"), "{none}");

    save_open_scopes(home.path(), root.path(), &names(&["docs", "web"])).unwrap();
    assert_eq!(
        settings.pull_line().unwrap(),
        "`/pull` takes a scope: AI can work in `docs`, `web`"
    );
}

#[test]
fn a_machine_that_never_ran_warlock_has_nothing_open() {
    let (home, repo) = (a_dir(), a_dir());

    assert_eq!(
        settings(home.path(), repo.path()).open_or_unknown(),
        OpenScopes::Nothing
    );
    assert_eq!(
        Settings::at(None, repo.path()).open_or_unknown(),
        OpenScopes::Nothing
    );
}

#[test]
fn the_open_scopes_on_disk_are_the_ones_the_header_states() {
    let (home, repo) = (a_dir(), a_dir());
    let open = names(&["billing", "web"]);
    save_open_scopes(home.path(), repo.path(), &open).expect("a config that writes");

    assert_eq!(
        settings(home.path(), repo.path()).open_or_unknown(),
        OpenScopes::Named(open)
    );
}

#[test]
fn a_config_holding_the_empty_set_is_the_same_nothing_as_no_config_at_all() {
    let (home, repo) = (a_dir(), a_dir());
    save_open_scopes(home.path(), repo.path(), &[]).expect("a config that writes");

    assert_eq!(
        settings(home.path(), repo.path()).open_or_unknown(),
        OpenScopes::Nothing
    );
}

#[test]
fn a_config_that_will_not_parse_is_said_rather_than_ending_the_session() {
    // Broken is never drawn as absent: the two mean opposite things about what
    // is on disk.
    let (home, repo) = (a_dir(), a_dir());
    broken(home.path(), repo.path());

    assert_eq!(
        settings(home.path(), repo.path()).open_or_unknown(),
        OpenScopes::Unknown
    );
}

#[test]
fn the_open_line_names_each_open_scope_nothing_or_the_file_that_would_not_read() {
    assert_eq!(
        open_line(&OpenScopes::Nothing, None),
        "AI can work in no scope"
    );
    assert_eq!(
        open_line(&OpenScopes::named(["a", "b"]), None),
        "AI can work in `a`, `b`"
    );
    assert_eq!(
        open_line(&OpenScopes::Unknown, None),
        "where AI can work is unknown"
    );
    assert_eq!(
        open_line(&OpenScopes::Unknown, Some(Path::new("/h/config.toml"))),
        "where AI can work is unknown: `/h/config.toml` could not be read"
    );
}
