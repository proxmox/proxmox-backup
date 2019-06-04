use failure::*;

use proxmox_backup::api2;
use proxmox_backup::api_schema::format::*;

fn main() -> Result<(), Error> {

    let api = api2::admin::datastore::backup::backup_api();

    dump_api(&mut std::io::stdout(), &api, ".", 0)?;

    Ok(())
}
