use kohiro::store::{Store, StoreError};
use tempfile::tempdir;

fn temp_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().unwrap();
    let store = Store::open(&dir.path().join("kohiro.db")).unwrap();
    (dir, store)
}

#[test]
fn bootstrap_and_lookup_are_idempotent() {
    let (_dir, store) = temp_store();

    assert!(store
        .user_by_fingerprint("SHA256:missing")
        .unwrap()
        .is_none());

    store
        .bootstrap("admin", "SHA256:key", "admin@example")
        .unwrap();
    store
        .bootstrap("admin", "SHA256:key", "admin@example")
        .unwrap();

    let user = store.user_by_fingerprint("SHA256:key").unwrap().unwrap();
    assert_eq!(user.username, "admin");
    assert!(user.is_admin);
}

#[test]
fn repos_permissions_and_visibility_match_go_store() {
    let (_dir, store) = temp_store();
    let owner = store.add_user("owner", false).unwrap();
    let writer = store.add_user("writer", false).unwrap();

    assert!(store.get_repo("owner", "missing").unwrap().is_none());

    let repo = store.ensure_repo(owner.id, "demo").unwrap();
    let same = store.ensure_repo(owner.id, "demo").unwrap();
    assert_eq!(repo.id, same.id);
    assert_eq!(repo.owner_id, owner.id);
    assert_eq!(repo.name, "demo");
    assert!(!repo.public);

    assert!(!store.has_write_access(writer.id, repo.id));
    store.grant_write(writer.id, repo.id).unwrap();
    assert!(store.has_write_access(writer.id, repo.id));

    store.set_public("owner", "demo", true).unwrap();
    assert!(store.get_repo("owner", "demo").unwrap().unwrap().public);
    store.set_public("owner", "demo", false).unwrap();
    assert!(!store.get_repo("owner", "demo").unwrap().unwrap().public);

    assert!(matches!(
        store.set_public("owner", "missing", true),
        Err(StoreError::NotFound)
    ));
}

#[test]
fn tui_store_queries_match_go_store() {
    let (_dir, store) = temp_store();
    let admin = store.add_user("admin", true).unwrap();
    let owner = store.add_user("owner", false).unwrap();
    let third = store.add_user("third", false).unwrap();

    store.ensure_repo(owner.id, "pub").unwrap();
    store.set_public("owner", "pub", true).unwrap();
    let private = store.ensure_repo(owner.id, "priv").unwrap();
    store.grant_write(third.id, private.id).unwrap();

    // Owner sees both owned repos.
    let mut owner_repos: Vec<String> = store
        .list_repos_for_user(owner.id)
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    owner_repos.sort();
    assert_eq!(owner_repos, vec!["priv", "pub"]);

    // Third user sees public + the repo it was granted write on.
    let mut third_repos: Vec<String> = store
        .list_repos_for_user(third.id)
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    third_repos.sort();
    assert_eq!(third_repos, vec!["priv", "pub"]);

    // Admin sees everything.
    assert_eq!(store.list_repos_for_user(admin.id).unwrap().len(), 2);

    // Public listing is public-only.
    let pubs: Vec<String> = store
        .list_public_repos()
        .unwrap()
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(pubs, vec!["pub"]);

    // add_key_strict: free → Ok(false) + count increments.
    assert_eq!(store.key_count(owner.id).unwrap(), 0);
    assert!(!store.add_key_strict(owner.id, "SHA256:abc", "c").unwrap());
    assert_eq!(store.key_count(owner.id).unwrap(), 1);
    // Same user, same fingerprint → Ok(true), no duplicate.
    assert!(store.add_key_strict(owner.id, "SHA256:abc", "c").unwrap());
    assert_eq!(store.key_count(owner.id).unwrap(), 1);
    // Other user claiming the same fingerprint is rejected.
    assert!(matches!(
        store.add_key_strict(third.id, "SHA256:abc", "c"),
        Err(StoreError::KeyClaimedByOther)
    ));

    // remove_key only affects the owning user.
    let key_id = store.list_keys_for_user(owner.id).unwrap()[0].id;
    assert!(matches!(
        store.remove_key(third.id, key_id),
        Err(StoreError::NotFound)
    ));

    // delete_repo removes the repo; a second delete is NotFound.
    store.delete_repo("owner", "pub").unwrap();
    assert!(store.get_repo("owner", "pub").unwrap().is_none());
    assert!(matches!(
        store.delete_repo("owner", "pub"),
        Err(StoreError::NotFound)
    ));
}
