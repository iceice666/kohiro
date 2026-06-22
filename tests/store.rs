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
