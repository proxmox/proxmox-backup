//! Proxmox Backup Server Configuration library
//!
//! This library contains helper to read, parse and write the
//! configuration files.

use failure::*;

use proxmox::tools::try_block;

use crate::buildcfg;

pub mod datastore;

/// Check configuration directory permissions
///
/// For security reasons, we want to make sure they are set correctly:
/// * owned by 'backup' user/group
/// * nobody else can read (mode 0700)
pub fn check_configdir_permissions() -> Result<(), Error> {
    let cfgdir = buildcfg::CONFIGDIR;

    let backup_user = crate::backup::backup_user()?;
    let backup_uid = backup_user.uid.as_raw();
    let backup_gid = backup_user.gid.as_raw();

    try_block!({
        let stat = nix::sys::stat::stat(cfgdir)?;

        if stat.st_uid != backup_uid {
            bail!("wrong user ({} != {})", stat.st_uid, backup_uid);
        }
        if stat.st_gid != backup_gid {
            bail!("wrong group ({} != {})", stat.st_gid, backup_gid);
        }

        let perm = stat.st_mode & 0o777;
        if perm != 0o700 {
            bail!("wrong permission ({:o} != {:o})", perm, 0o700);
        }
        Ok(())
    })
    .map_err(|err| {
        format_err!(
            "configuration directory '{}' permission problem - {}",
            cfgdir,
            err
        )
    })
}

pub fn create_configdir() -> Result<(), Error> {
    use nix::sys::stat::Mode;

    let cfgdir = buildcfg::CONFIGDIR;

    match nix::unistd::mkdir(cfgdir, Mode::from_bits_truncate(0o700)) {
        Ok(()) => {}
        Err(nix::Error::Sys(nix::errno::Errno::EEXIST)) => {
            check_configdir_permissions()?;
            return Ok(());
        }
        Err(err) => bail!(
            "unable to create configuration directory '{}' - {}",
            cfgdir,
            err
        ),
    }

    let backup_user = crate::backup::backup_user()?;

    nix::unistd::chown(cfgdir, Some(backup_user.uid), Some(backup_user.gid))
        .map_err(|err| {
            format_err!(
                "unable to set configuration directory '{}' permissions - {}",
                cfgdir,
                err
            )
        })
}
