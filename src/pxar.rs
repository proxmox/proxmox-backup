//! *pxar* Implementation (proxmox file archive format)
//!
//! This code implements a slightly modified version of the *catar*
//! format used in the [casync](https://github.com/systemd/casync)
//! toolkit (we are not 100\% binary compatible). It is a file archive
//! format defined by 'Lennart Poettering', specially defined for
//! efficient deduplication.

//! Every archive contains items in the following order:
//!  * `ENTRY`              -- containing general stat() data and related bits
//!   * `USER`              -- user name as text, if enabled
//!   * `GROUP`             -- group name as text, if enabled
//!   * `XATTR`             -- one extended attribute
//!   * ...                 -- more of these when there are multiple defined
//!   * `ACL_USER`          -- one `USER ACL` entry
//!   * ...                 -- more of these when there are multiple defined
//!   * `ACL_GROUP`         -- one `GROUP ACL` entry
//!   * ...                 -- more of these when there are multiple defined
//!   * `ACL_GROUP_OBJ`     -- The `ACL_GROUP_OBJ`
//!   * `ACL_DEFAULT`       -- The various default ACL fields if there's one defined
//!   * `ACL_DEFAULT_USER`  -- one USER ACL entry
//!   * ...                 -- more of these when multiple are defined
//!   * `ACL_DEFAULT_GROUP` -- one GROUP ACL entry
//!   * ...                 -- more of these when multiple are defined
//!   * `FCAPS`             -- file capability in Linux disk format
//!   * `QUOTA_PROJECT_ID`  -- the ext4/xfs quota project ID
//!   * `PAYLOAD`           -- file contents, if it is one
//!   * `SYMLINK`           -- symlink target, if it is one
//!   * `DEVICE`            -- device major/minor, if it is a block/char device
//!
//!   If we are serializing a directory, then this is followed by:
//!
//!   * `FILENAME`          -- name of the first directory entry (strictly ordered!)
//!   * `<archive>`         -- serialization of the first directory entry's metadata and contents,
//!  following the exact same archive format
//!   * `FILENAME`          -- name of the second directory entry (strictly ordered!)
//!   * `<archive>`         -- serialization of the second directory entry
//!   * ...
//!   * `GOODBYE`           -- lookup table at the end of a list of directory entries

//!
//! The original format has no way to deal with hardlinks, so we
//! extended the format by a special `HARDLINK` tag, which can replace
//! an `ENTRY` tag. The `HARDLINK` tag contains an 64bit offset which
//! points to the linked `ENTRY` inside the archive, followed by the
//! full path name of that `ENTRY`. `HARDLINK`s may not have further data
//! (user, group, acl, ...) because this is already defined by the
//! linked `ENTRY`.

mod binary_search_tree;
pub use binary_search_tree::*;

pub mod flags;
pub use flags::*;

mod format_definition;
pub use format_definition::*;

mod encoder;
pub use encoder::*;

mod sequential_decoder;
pub use sequential_decoder::*;

mod decoder;
pub use decoder::*;

mod match_pattern;
pub use match_pattern::*;

mod dir_stack;
pub use dir_stack::*;

pub mod fuse;
pub use fuse::*;

pub mod catalog;

mod helper;
