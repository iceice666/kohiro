use crate::store::{Store, User};
use russh::keys::PublicKey;

pub fn fingerprint_of(pk: &PublicKey) -> String {
    pk.fingerprint(Default::default()).to_string()
}

pub fn user_from_fingerprint(store: &Store, fp: &str) -> Option<User> {
    store.user_by_fingerprint(fp).ok().flatten()
}

pub fn parse_repo(path: &str) -> Option<(String, String)> {
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    if !valid_component(owner) || !valid_component(name) {
        return None;
    }
    Some((owner.to_owned(), name.to_owned()))
}

fn valid_component(component: &str) -> bool {
    !component.is_empty()
        && !component.contains('/')
        && !component.contains("..")
        && !component.starts_with('.')
        && !component.contains('\0')
        && !component.chars().any(|ch| ch.is_ascii_control())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    None,
    ReadOnly,
    ReadWrite,
}

pub fn git_access(store: &Store, user: Option<&User>, owner: &str, name: &str) -> Access {
    if let Some(user) = user {
        if user.is_admin || user.username == owner {
            return Access::ReadWrite;
        }
        let Ok(Some(repo)) = store.get_repo(owner, name) else {
            return Access::None;
        };
        if store.has_write_access(user.id, repo.id) {
            return Access::ReadWrite;
        }
        if repo.public {
            return Access::ReadOnly;
        }
        return Access::None;
    }

    if can_read(store, None, owner, name) {
        Access::ReadOnly
    } else {
        Access::None
    }
}

pub fn can_read(store: &Store, user: Option<&User>, owner: &str, name: &str) -> bool {
    if let Some(user) = user {
        if user.is_admin || user.username == owner {
            return true;
        }
        let Ok(Some(repo)) = store.get_repo(owner, name) else {
            return false;
        };
        return store.has_write_access(user.id, repo.id) || repo.public;
    }

    matches!(store.get_repo(owner, name), Ok(Some(repo)) if repo.public)
}

pub fn can_write(store: &Store, user: Option<&User>, owner: &str, name: &str) -> bool {
    let Some(user) = user else {
        return false;
    };
    if user.is_admin || user.username == owner {
        return true;
    }
    let Ok(Some(repo)) = store.get_repo(owner, name) else {
        return false;
    };
    store.has_write_access(user.id, repo.id)
}
