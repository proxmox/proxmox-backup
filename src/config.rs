//! Proxmox Backup Server Configuration library
//!
//! This library contains helper to read, parse and write the
//! configuration files.

use failure::*;

pub mod datastore;

use crate::tools;
use crate::buildcfg;

/// Check configuration directory permissions
///
/// For security reasons, we want to make sure they are set correctly:
/// * owned by 'backup' user/group
/// * nobody else can read (mode 0700)
pub fn check_confidir_permissions() -> Result<(), Error> {

    let cfgdir = buildcfg::CONFIGDIR;
    let (backup_uid, backup_gid) = tools::getpwnam_ugid("backup")?;

    try_block!({
        let stat = nix::sys::stat::stat(cfgdir)?;

        if stat.st_uid != backup_uid {
            bail!("wrong user ({} != {})",  stat.st_uid, backup_uid);
        }
        if stat.st_gid != backup_gid {
            bail!("wrong group ({} != {})",  stat.st_gid, backup_gid);
        }

        let perm = stat.st_mode & 0o777;
        if perm != 0o700 {
            bail!("wrong permission ({:o} != {:o})",  perm, 0o700);
        }
        Ok(())
    }).map_err(|err| format_err!("configuration directory '{}' permission problem - {}", cfgdir, err))
}

pub fn create_configdir() -> Result<(), Error> {

    use nix::sys::stat::Mode;

    let cfgdir = buildcfg::CONFIGDIR;
    let (backup_uid, backup_gid) = tools::getpwnam_ugid("backup")?;

    match nix::unistd::mkdir(cfgdir, Mode::from_bits_truncate(0o700)) {
        Ok(()) => {},
        Err(nix::Error::Sys(nix::errno::Errno::EEXIST)) => {
            check_confidir_permissions()?;
            return Ok(());
        },
        Err(err) => bail!("unable to create configuration directory '{}' - {}", cfgdir, err),
    }

    try_block!({
        let uid = nix::unistd::Uid::from_raw(backup_uid);
        let gid = nix::unistd::Gid::from_raw(backup_gid);

        nix::unistd::chown(cfgdir, Some(uid), Some(gid))?;

        Ok(())
    }).map_err(|err: Error| format_err!(
        "unable to set configuration directory '{}' permissions - {}", cfgdir, err))
}
