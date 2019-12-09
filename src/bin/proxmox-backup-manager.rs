extern crate proxmox_backup;

use proxmox::api::cli::*;

fn datastore_commands() -> CommandLineInterface {

    use proxmox_backup::config;
    use proxmox_backup::api2;

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&api2::config::datastore::GET))
        .insert("create",
                CliCommand::new(&api2::config::datastore::POST)
                .arg_param(&["name", "path"])
        )
        .insert("remove",
                CliCommand::new(&api2::config::datastore::DELETE)
                .arg_param(&["name"])
                .completion_cb("name", config::datastore::complete_datastore_name)
        );

    cmd_def.into()
}



fn garbage_collection_commands() -> CommandLineInterface {

    use proxmox_backup::config;
    use proxmox_backup::api2;

    let cmd_def = CliCommandMap::new()
        .insert("status",
                CliCommand::new(&api2::admin::datastore::API_METHOD_GARBAGE_COLLECTION_STATUS)
                .arg_param(&["store"])
                .completion_cb("store", config::datastore::complete_datastore_name)
        )
        .insert("start",
                CliCommand::new(&api2::admin::datastore::API_METHOD_START_GARBAGE_COLLECTION)
                .arg_param(&["store"])
                .completion_cb("store", config::datastore::complete_datastore_name)
        );

    cmd_def.into()
}

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("datastore", datastore_commands())
        .insert("garbage-collection", garbage_collection_commands());

    run_cli_command(cmd_def);
}
