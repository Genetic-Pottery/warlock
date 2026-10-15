use std::path::PathBuf;

use super::{Node, Tree};

fn fixture() -> Tree {
    Tree::new(
        Node::new("repo").with_children([
            Node::new("repo/crates"),
            Node::new("repo/docs")
                .with_scope(Some("docs".to_owned()))
                .with_children([Node::new("repo/docs/adr").with_covering(Some("docs".to_owned()))]),
        ]),
    )
}

fn fixture_with_files() -> Tree {
    fn listing(node: &Node) -> Node {
        Node {
            children: node.children.iter().map(listing).collect(),
            files: vec![node.path.join("Cargo.toml"), node.path.join("notes.md")],
            ..node.clone()
        }
    }
    Tree::new(listing(&fixture().root))
}

#[test]
fn new_node_starts_childless_and_unscoped() {
    let node = Node::new("a");
    assert!(node.children.is_empty());
    assert!(node.files.is_empty());
    assert!(node.is_leaf());
    assert_eq!(node.scope, None);
    assert_eq!(node.covering, None);
    assert!(!node.is_covered());
}

#[test]
fn a_node_carries_its_own_scope_and_is_covered_by_it() {
    let node = Node::new("a");

    let scoped = node.clone().with_scope(Some("data-plane".to_owned()));
    assert_eq!(scoped.scope.as_deref(), Some("data-plane"));
    assert_eq!(scoped.covering.as_deref(), Some("data-plane"));
    assert!(scoped.is_covered());
    assert_eq!(scoped.children, node.children);
    assert_eq!(scoped.files, node.files);

    let unscoped = scoped.with_scope(None);
    assert_eq!(unscoped.scope, None, "the setter sets rather than latches");
    assert_eq!(unscoped.covering, None);
}

#[test]
fn a_node_can_be_covered_by_a_scope_it_does_not_carry() {
    let node = Node::new("a/src").with_covering(Some("data-plane".to_owned()));
    assert_eq!(node.scope, None);
    assert_eq!(node.covering.as_deref(), Some("data-plane"));
    assert!(node.is_covered());
    assert!(!node.with_covering(None).is_covered());
}

#[test]
fn a_node_written_without_scope_or_covering_reads_back_uncovered() {
    use serde_test::{Token, assert_de_tokens};

    assert_de_tokens(
        &Node::new("repo"),
        &[
            Token::Struct {
                name: "Node",
                len: 3,
            },
            Token::Str("path"),
            Token::Str("repo"),
            Token::Str("children"),
            Token::Seq { len: Some(0) },
            Token::SeqEnd,
            Token::Str("files"),
            Token::Seq { len: Some(0) },
            Token::SeqEnd,
            Token::StructEnd,
        ],
    );
}

#[test]
fn with_files_attaches_them_in_order_and_leaves_the_node_a_leaf() {
    let node = Node::new("a").with_files(["a/README.md", "a/Cargo.toml"].map(PathBuf::from));
    assert_eq!(
        node.files,
        [PathBuf::from("a/README.md"), PathBuf::from("a/Cargo.toml")],
        "stored as given: ordering is the caller's business",
    );
    assert!(
        node.is_leaf(),
        "files are a listing, not children, so they cannot unmake a leaf",
    );
}

#[test]
fn files_are_no_part_of_walking() {
    let listed: Vec<_> = fixture_with_files()
        .walk()
        .map(|(node, depth)| (node.path.clone(), depth))
        .collect();
    let bare: Vec<_> = fixture()
        .walk()
        .map(|(node, depth)| (node.path.clone(), depth))
        .collect();

    assert_eq!(listed, bare, "a file is never yielded as a node");
    assert!(
        fixture_with_files()
            .walk()
            .all(|(node, _)| node.files.len() == 2),
        "the files really are there to be missed",
    );
}

#[test]
fn with_children_attaches_them_in_order() {
    let tree = fixture();
    assert!(!tree.root.is_leaf());
    let names: Vec<_> = tree
        .root
        .children
        .iter()
        .map(|child| child.path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["repo/crates", "repo/docs"]);
}

#[test]
fn tree_owns_its_root() {
    let tree = fixture();
    assert_eq!(tree.root_path(), std::path::Path::new("repo"));
    assert_eq!(tree.root.scope, None);
}

#[test]
fn walk_is_depth_first_with_parents_before_children() {
    let tree = fixture();
    let visited: Vec<(String, usize)> = tree
        .walk()
        .map(|(node, depth)| (node.path.to_string_lossy().into_owned(), depth))
        .collect();
    assert_eq!(
        visited,
        [
            ("repo".to_owned(), 0),
            ("repo/crates".to_owned(), 1),
            ("repo/docs".to_owned(), 1),
            ("repo/docs/adr".to_owned(), 2),
        ]
    );
}

#[test]
fn walk_of_a_lone_node_yields_only_the_root_at_depth_zero() {
    let tree = Tree::new(Node::new("solo"));
    let visited: Vec<_> = tree
        .walk()
        .map(|(node, depth)| (node.path.clone(), depth))
        .collect();
    assert_eq!(visited, [(PathBuf::from("solo"), 0)]);
}

#[test]
fn find_reaches_the_root_and_the_deepest_node() {
    let tree = fixture();
    assert_eq!(
        tree.find("repo").map(|node| node.path.clone()),
        Some(PathBuf::from("repo"))
    );
    let adr = tree.find("repo/docs/adr").expect("the deepest node");
    assert_eq!(adr.scope, None);
    assert_eq!(adr.covering.as_deref(), Some("docs"));
}

#[test]
fn find_returns_none_for_a_path_not_in_the_tree() {
    let tree = fixture();
    assert!(tree.find("repo/nowhere").is_none());
    // Paths are compared as stored: no normalisation, no filesystem.
    assert!(tree.find("./repo").is_none());
}

#[test]
fn children_nest_more_than_one_level() {
    let tree = fixture();
    let docs = &tree.root.children[1];
    assert_eq!(docs.children.len(), 1);
    assert_eq!(docs.children[0].path, PathBuf::from("repo/docs/adr"));
    assert!(docs.children[0].is_leaf());
}

#[test]
fn a_tree_carrying_files_survives_a_serde_round_trip() {
    use serde_test::{Token, assert_tokens};

    // A root with one child, each listing files, so the round trip covers
    // a nested node and a `files` list that is neither empty nor shared.
    // The tokens are written out by hand rather than derived from the
    // value: a renamed or reordered field fails here instead of passing
    // silently. No format is involved, so nothing below fixes an on-disk
    // representation.
    let tree = Tree::new(
        Node::new("repo")
            .with_scope(Some("data-plane".to_owned()))
            .with_children([Node::new("repo/docs")
                .with_covering(Some("data-plane".to_owned()))
                .with_files([PathBuf::from("repo/docs/adr.md")])])
            .with_files([
                PathBuf::from("repo/Cargo.toml"),
                PathBuf::from("repo/README.md"),
            ]),
    );

    assert_tokens(
        &tree,
        &[
            Token::Struct {
                name: "Tree",
                len: 1,
            },
            Token::Str("root"),
            Token::Struct {
                name: "Node",
                len: 5,
            },
            Token::Str("path"),
            Token::Str("repo"),
            Token::Str("scope"),
            Token::Some,
            Token::Str("data-plane"),
            Token::Str("covering"),
            Token::Some,
            Token::Str("data-plane"),
            Token::Str("children"),
            Token::Seq { len: Some(1) },
            Token::Struct {
                name: "Node",
                len: 5,
            },
            Token::Str("path"),
            Token::Str("repo/docs"),
            Token::Str("scope"),
            Token::None,
            Token::Str("covering"),
            Token::Some,
            Token::Str("data-plane"),
            Token::Str("children"),
            Token::Seq { len: Some(0) },
            Token::SeqEnd,
            Token::Str("files"),
            Token::Seq { len: Some(1) },
            Token::Str("repo/docs/adr.md"),
            Token::SeqEnd,
            Token::StructEnd,
            Token::SeqEnd,
            Token::Str("files"),
            Token::Seq { len: Some(2) },
            Token::Str("repo/Cargo.toml"),
            Token::Str("repo/README.md"),
            Token::SeqEnd,
            Token::StructEnd,
            Token::StructEnd,
        ],
    );
}
