extern crate proxmox_backup;

//use proxmox_backup::api2;
use proxmox_backup::cli::*;

fn datastore_commands() -> CommandLineInterface {

    use proxmox_backup::config;
    use proxmox_backup::api2;

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(api2::config::datastore::get()).into())
        .insert("create",
                CliCommand::new(api2::config::datastore::post())
                .arg_param(vec!["name", "path"])
                .into())
        .insert("remove",
                CliCommand::new(api2::config::datastore::delete())
                .arg_param(vec!["name"])
                .completion_cb("name", config::datastore::complete_datastore_name)
                .into());

    cmd_def.into()
}



fn garbage_collection_commands() -> CommandLineInterface {

    use proxmox_backup::config;
    use proxmox_backup::api2;

    let cmd_def = CliCommandMap::new()
        .insert("status",
                CliCommand::new(api2::admin::datastore::api_method_garbage_collection_status())
                .arg_param(vec!["store"])
                .completion_cb("store", config::datastore::complete_datastore_name)
                .into())
        .insert("start",
                CliCommand::new(api2::admin::datastore::api_method_start_garbage_collection())
                .arg_param(vec!["store"])
                .completion_cb("store", config::datastore::complete_datastore_name)
                .into());

    cmd_def.into()
}

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("datastore".to_owned(), datastore_commands())
        .insert("garbage-collection".to_owned(), garbage_collection_commands());

    run_cli_command(cmd_def.into());
}
