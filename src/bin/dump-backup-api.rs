use anyhow::{Error};

use proxmox::api::format::*;

use proxmox_backup::api2;

fn main() -> Result<(), Error> {

    let api = api2::backup::BACKUP_API_ROUTER;

    dump_api(&mut std::io::stdout(), &api, ".", 0)?;

    Ok(())
}
