//! *pxar* Implementation
//!
//! This code implements a slightly modified version of the *catar*
//! format used in the [casync](https://github.com/systemd/casync)
//! toolkit (we are not 100% binary compatible). It is a file archive
//! format defined by 'Lennart Poettering', specially defined for
//! efficent deduplication.

//! Every archive contains items in the following order:
//!  * ENTRY             -- containing general stat() data and related bits
//!   * USER              -- user name as text, if enabled
//!   * GROUP             -- group name as text, if enabled
//!   * XATTR             -- one extended attribute
//!   * ...               -- more of these when there are multiple defined
//!   * ACL_USER          -- one USER ACL entry
//!   * ...               -- more of these when there are multiple defined
//!   * ACL_GROUP         -- one GROUP ACL entry
//!   * ...               -- more of these when there are multiple defined
//!   * ACL_GROUP_OBJ     -- The ACL_GROUP_OBJ
//!   * ACL_DEFAULT       -- The various default ACL fields if there's one defined
//!   * ACL_DEFAULT_USER  -- one USER ACL entry
//!   * ...               -- more of these when multiple are defined
//!   * ACL_DEFAULT_GROUP -- one GROUP ACL entry
//!   * ...               -- more of these when multiple are defined
//!   * FCAPS             -- file capability in Linux disk format
//!   * QUOTA_PROJECT_ID  -- the ext4/xfs quota project ID
//!   * PAYLOAD           -- file contents, if it is one
//!   * SYMLINK           -- symlink target, if it is one
//!   * DEVICE            -- device major/minor, if it is a block/char device
//!
//!   If we are serializing a directory, then this is followed by:
//!
//!   * FILENAME          -- name of the first directory entry (strictly ordered!)
//!   * <archive>         -- serialization of the first directory entry's metadata and contents,
//!  following the exact same archive format
//!   * FILENAME          -- name of the second directory entry (strictly ordered!)
//!   * <archive>         -- serialization of the second directory entry
//!   * ...
//!   * GOODBYE           -- lookup table at the end of a list of directory entries

pub mod binary_search_tree;
pub mod format_definition;
pub mod encoder;
pub mod decoder;

