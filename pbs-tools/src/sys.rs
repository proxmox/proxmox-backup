//! System level helpers.

use nix::unistd::{Gid, Group, Uid, User};

/// Query a user by name but only unless built with `#[cfg(test)]`.
///
/// This is to avoid having regression tests query the users of development machines which may
/// not be compatible with PBS or privileged enough.
pub fn query_user(user_name: &str) -> Result<Option<User>, nix::Error> {
    if cfg!(test) {
        Ok(Some(
            User::from_uid(Uid::current())?.expect("current user does not exist"),
        ))
    } else {
        User::from_name(user_name)
    }
}

/// Query a group by name but only unless built with `#[cfg(test)]`.
///
/// This is to avoid having regression tests query the groups of development machines which may
/// not be compatible with PBS or privileged enough.
pub fn query_group(group_name: &str) -> Result<Option<Group>, nix::Error> {
    if cfg!(test) {
        Ok(Some(
            Group::from_gid(Gid::current())?.expect("current group does not exist"),
        ))
    } else {
        Group::from_name(group_name)
    }
}
