//! Types for user handling.
//!
//! We have [`Username`]s and [`Realm`]s. To uniquely identify a user, they must be combined into a [`Userid`].
//!
//! Since they're all string types, they're organized as follows:
//!
//! * [`Username`]: an owned user name. Internally a `String`.
//! * [`UsernameRef`]: a borrowed user name. Pairs with a `Username` the same way a `str` pairs
//!   with `String`, meaning you can only make references to it.
//! * [`Realm`]: an owned realm (`String` equivalent).
//! * [`RealmRef`]: a borrowed realm (`str` equivalent).
//! * [`Userid`]: an owned user id (`"user@realm"`). Note that this does not have a separate
//!   borrowed type.
//!
//! Note that `Username`s are not unique, therefore they do not implement `Eq` and cannot be
//! compared directly. If a direct comparison is really required, they can be compared as strings
//! via the `as_str()` method. [`Realm`]s and [`Userid`]s on the other hand can be compared with
//! each other, as in those two cases the comparison has meaning.

use std::borrow::Borrow;
use std::convert::TryFrom;
use std::fmt;

use anyhow::{bail, format_err, Error};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

use proxmox::api::api;
use proxmox::api::schema::{ApiStringFormat, Schema, StringSchema};
use proxmox::const_regex;

// we only allow a limited set of characters
// colon is not allowed, because we store usernames in
// colon separated lists)!
// slash is not allowed because it is used as pve API delimiter
// also see "man useradd"
macro_rules! USER_NAME_REGEX_STR { () => (r"(?:[^\s:/[:cntrl:]]+)") }
macro_rules! GROUP_NAME_REGEX_STR { () => (USER_NAME_REGEX_STR!()) }
macro_rules! USER_ID_REGEX_STR { () => (concat!(USER_NAME_REGEX_STR!(), r"@", PROXMOX_SAFE_ID_REGEX_STR!())) }

const_regex! {
    pub PROXMOX_USER_NAME_REGEX = concat!(r"^",  USER_NAME_REGEX_STR!(), r"$");
    pub PROXMOX_USER_ID_REGEX = concat!(r"^",  USER_ID_REGEX_STR!(), r"$");
    pub PROXMOX_GROUP_ID_REGEX = concat!(r"^",  GROUP_NAME_REGEX_STR!(), r"$");
}

pub const PROXMOX_USER_NAME_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_USER_NAME_REGEX);

pub const PROXMOX_USER_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_USER_ID_REGEX);

pub const PROXMOX_GROUP_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_GROUP_ID_REGEX);

pub const PROXMOX_GROUP_ID_SCHEMA: Schema = StringSchema::new("Group ID")
    .format(&PROXMOX_GROUP_ID_FORMAT)
    .min_length(3)
    .max_length(64)
    .schema();

pub const PROXMOX_AUTH_REALM_STRING_SCHEMA: StringSchema =
    StringSchema::new("Authentication domain ID")
        .format(&super::PROXMOX_SAFE_ID_FORMAT)
        .min_length(3)
        .max_length(32);
pub const PROXMOX_AUTH_REALM_SCHEMA: Schema = PROXMOX_AUTH_REALM_STRING_SCHEMA.schema();


#[api(
    type: String,
    format: &PROXMOX_USER_NAME_FORMAT,
)]
/// The user name part of a user id.
///
/// This alone does NOT uniquely identify the user and therefore does not implement `Eq`. In order
/// to compare user names directly, they need to be explicitly compared as strings by calling
/// `.as_str()`.
///
/// ```compile_fail
/// fn test(a: Username, b: Username) -> bool {
///     a == b // illegal and does not compile
/// }
/// ```
#[derive(Clone, Debug, Hash, Deserialize, Serialize)]
pub struct Username(String);

/// A reference to a user name part of a user id. This alone does NOT uniquely identify the user.
///
/// This is like a `str` to the `String` of a [`Username`].
#[derive(Debug, Hash)]
pub struct UsernameRef(str);

#[doc(hidden)]
/// ```compile_fail
/// let a: Username = unsafe { std::mem::zeroed() };
/// let b: Username = unsafe { std::mem::zeroed() };
/// let _ = <Username as PartialEq>::eq(&a, &b);
/// ```
///
/// ```compile_fail
/// let a: &UsernameRef = unsafe { std::mem::zeroed() };
/// let b: &UsernameRef = unsafe { std::mem::zeroed() };
/// let _ = <&UsernameRef as PartialEq>::eq(a, b);
/// ```
///
/// ```compile_fail
/// let a: &UsernameRef = unsafe { std::mem::zeroed() };
/// let b: &UsernameRef = unsafe { std::mem::zeroed() };
/// let _ = <&UsernameRef as PartialEq>::eq(&a, &b);
/// ```
struct _AssertNoEqImpl;

impl UsernameRef {
    fn new(s: &str) -> &Self {
        unsafe { &*(s as *const str as *const UsernameRef) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for Username {
    type Target = UsernameRef;

    fn deref(&self) -> &UsernameRef {
        self.borrow()
    }
}

impl Borrow<UsernameRef> for Username {
    fn borrow(&self) -> &UsernameRef {
        UsernameRef::new(self.0.as_str())
    }
}

impl AsRef<UsernameRef> for Username {
    fn as_ref(&self) -> &UsernameRef {
        self.borrow()
    }
}

impl ToOwned for UsernameRef {
    type Owned = Username;

    fn to_owned(&self) -> Self::Owned {
        Username(self.0.to_owned())
    }
}

impl TryFrom<String> for Username {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Error> {
        if !PROXMOX_USER_NAME_REGEX.is_match(&s) {
            bail!("invalid user name");
        }

        Ok(Self(s))
    }
}

impl<'a> TryFrom<&'a str> for &'a UsernameRef {
    type Error = Error;

    fn try_from(s: &'a str) -> Result<&'a UsernameRef, Error> {
        if !PROXMOX_USER_NAME_REGEX.is_match(s) {
            bail!("invalid name in user id");
        }

        Ok(UsernameRef::new(s))
    }
}

#[api(schema: PROXMOX_AUTH_REALM_SCHEMA)]
/// An authentication realm.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub struct Realm(String);

/// A reference to an authentication realm.
///
/// This is like a `str` to the `String` of a `Realm`.
#[derive(Debug, Hash, Eq, PartialEq)]
pub struct RealmRef(str);

impl RealmRef {
    fn new(s: &str) -> &Self {
        unsafe { &*(s as *const str as *const RealmRef) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for Realm {
    type Target = RealmRef;

    fn deref(&self) -> &RealmRef {
        self.borrow()
    }
}

impl Borrow<RealmRef> for Realm {
    fn borrow(&self) -> &RealmRef {
        RealmRef::new(self.0.as_str())
    }
}

impl AsRef<RealmRef> for Realm {
    fn as_ref(&self) -> &RealmRef {
        self.borrow()
    }
}

impl ToOwned for RealmRef {
    type Owned = Realm;

    fn to_owned(&self) -> Self::Owned {
        Realm(self.0.to_owned())
    }
}

impl TryFrom<String> for Realm {
    type Error = Error;

    fn try_from(s: String) -> Result<Self, Error> {
        PROXMOX_AUTH_REALM_STRING_SCHEMA.check_constraints(&s)
            .map_err(|_| format_err!("invalid realm"))?;

        Ok(Self(s))
    }
}

impl<'a> TryFrom<&'a str> for &'a RealmRef {
    type Error = Error;

    fn try_from(s: &'a str) -> Result<&'a RealmRef, Error> {
        PROXMOX_AUTH_REALM_STRING_SCHEMA.check_constraints(s)
            .map_err(|_| format_err!("invalid realm"))?;

        Ok(RealmRef::new(s))
    }
}

impl PartialEq<str> for Realm {
    fn eq(&self, rhs: &str) -> bool {
        self.0 == rhs
    }
}

impl PartialEq<&str> for Realm {
    fn eq(&self, rhs: &&str) -> bool {
        self.0 == *rhs
    }
}

impl PartialEq<str> for RealmRef {
    fn eq(&self, rhs: &str) -> bool {
        self.0 == *rhs
    }
}

impl PartialEq<&str> for RealmRef {
    fn eq(&self, rhs: &&str) -> bool {
        self.0 == **rhs
    }
}

impl PartialEq<RealmRef> for Realm {
    fn eq(&self, rhs: &RealmRef) -> bool {
        self.0 == &rhs.0
    }
}

impl PartialEq<Realm> for RealmRef {
    fn eq(&self, rhs: &Realm) -> bool {
        self.0 == rhs.0
    }
}

impl PartialEq<Realm> for &RealmRef {
    fn eq(&self, rhs: &Realm) -> bool {
        (*self).0 == rhs.0
    }
}

/// A complete user id consting of a user name and a realm.
#[derive(Clone, Debug, Hash)]
pub struct Userid {
    data: String,
    name_len: usize,
    //name: Username,
    //realm: Realm,
}

impl Userid {
    pub const API_SCHEMA: Schema = StringSchema::new("User ID")
        .format(&PROXMOX_USER_ID_FORMAT)
        .min_length(3)
        .max_length(64)
        .schema();

    const fn new(data: String, name_len: usize) -> Self {
        Self { data, name_len }
    }

    pub fn name(&self) -> &UsernameRef {
        UsernameRef::new(&self.data[..self.name_len])
    }

    pub fn realm(&self) -> &RealmRef {
        RealmRef::new(&self.data[(self.name_len + 1)..])
    }

    pub fn as_str(&self) -> &str {
        &self.data
    }

    /// Get the "backup@pam" user id.
    pub fn backup_userid() -> &'static Self {
        &*BACKUP_USERID
    }

    /// Get the "root@pam" user id.
    pub fn root_userid() -> &'static Self {
        &*ROOT_USERID
    }
}

lazy_static! {
    pub static ref BACKUP_USERID: Userid = Userid::new("backup@pam".to_string(), 6);
    pub static ref ROOT_USERID: Userid = Userid::new("root@pam".to_string(), 4);
}

impl Eq for Userid {}

impl PartialEq for Userid {
    fn eq(&self, rhs: &Self) -> bool {
        self.data == rhs.data && self.name_len == rhs.name_len
    }
}

impl From<(Username, Realm)> for Userid {
    fn from(parts: (Username, Realm)) -> Self {
        Self::from((parts.0.as_ref(), parts.1.as_ref()))
    }
}

impl From<(&UsernameRef, &RealmRef)> for Userid {
    fn from(parts: (&UsernameRef, &RealmRef)) -> Self {
        let data = format!("{}@{}", parts.0.as_str(), parts.1.as_str());
        let name_len = parts.0.as_str().len();
        Self { data, name_len }
    }
}

impl fmt::Display for Userid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.data.fmt(f)
    }
}

impl std::str::FromStr for Userid {
    type Err = Error;

    fn from_str(id: &str) -> Result<Self, Error> {
        let (name, realm) = match id.as_bytes().iter().rposition(|&b| b == b'@') {
            Some(pos) => (&id[..pos], &id[(pos + 1)..]),
            None => bail!("not a valid user id"),
        };

        PROXMOX_AUTH_REALM_STRING_SCHEMA.check_constraints(realm)
            .map_err(|_| format_err!("invalid realm in user id"))?;

        Ok(Self::from((UsernameRef::new(name), RealmRef::new(realm))))
    }
}

impl TryFrom<String> for Userid {
    type Error = Error;

    fn try_from(data: String) -> Result<Self, Error> {
        let name_len = data
            .as_bytes()
            .iter()
            .rposition(|&b| b == b'@')
            .ok_or_else(|| format_err!("not a valid user id"))?;

        PROXMOX_AUTH_REALM_STRING_SCHEMA.check_constraints(&data[(name_len + 1)..])
            .map_err(|_| format_err!("invalid realm in user id"))?;

        Ok(Self { data, name_len })
    }
}

impl PartialEq<str> for Userid {
    fn eq(&self, rhs: &str) -> bool {
        self.data == *rhs
    }
}

impl PartialEq<&str> for Userid {
    fn eq(&self, rhs: &&str) -> bool {
        *self == **rhs
    }
}

impl PartialEq<String> for Userid {
    fn eq(&self, rhs: &String) -> bool {
        self == rhs.as_str()
    }
}

proxmox::forward_deserialize_to_from_str!(Userid);
proxmox::forward_serialize_to_display!(Userid);
