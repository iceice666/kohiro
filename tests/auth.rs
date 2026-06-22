use kohiro::auth::{self, Access};
use kohiro::store::Store;
use tempfile::tempdir;

fn args(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|s| s.to_string()).collect()
}

#[test]
fn repo_parser_rejects_traversal_and_ambiguous_paths() {
    assert_eq!(
        auth::parse_repo("owner/repo.git"),
        Some(("owner".into(), "repo".into()))
    );
    for path in args(&["../x", "a/", "/a", "a/b/../c", "a/.b", "a/b/c"]) {
        assert!(auth::parse_repo(&path).is_none(), "accepted {path}");
    }
}

#[test]
fn access_rules_match_go_hooks() {
    let dir = tempdir().unwrap();
    let store = Store::open(&dir.path().join("kohiro.db")).unwrap();

    let admin = store.add_user("admin", true).unwrap();
    let owner = store.add_user("owner", false).unwrap();
    let outsider = store.add_user("outsider", false).unwrap();

    let private = store.ensure_repo(owner.id, "private").unwrap();
    store.ensure_repo(owner.id, "public").unwrap();
    store.set_public("owner", "public", true).unwrap();

    assert_eq!(
        auth::git_access(&store, Some(&owner), "owner", "private"),
        Access::ReadWrite
    );
    assert_eq!(
        auth::git_access(&store, Some(&admin), "owner", "private"),
        Access::ReadWrite
    );
    assert_eq!(
        auth::git_access(&store, Some(&outsider), "owner", "public"),
        Access::ReadOnly
    );
    assert_eq!(
        auth::git_access(&store, None, "owner", "public"),
        Access::ReadOnly
    );
    assert_eq!(
        auth::git_access(&store, Some(&outsider), "owner", "private"),
        Access::None
    );
    assert_eq!(
        auth::git_access(&store, None, "owner", "private"),
        Access::None
    );

    store.grant_write(outsider.id, private.id).unwrap();
    assert_eq!(
        auth::git_access(&store, Some(&outsider), "owner", "private"),
        Access::ReadWrite
    );
}
