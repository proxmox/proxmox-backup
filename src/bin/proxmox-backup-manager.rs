extern crate proxmox_backup;

//use proxmox_backup::api3;
use proxmox_backup::cli::command::*;

fn datastore_commands() -> CommandLineInterface {

    use proxmox_backup::api3::config::datastore;

    let cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(datastore::get()).into())
        .insert("create",
                CliCommand::new(datastore::post())
                .arg_param(vec!["name", "path"])
                .into())
        .insert("remove",
                CliCommand::new(datastore::delete())
                .arg_param(vec!["name"])
                .completion_cb("name", proxmox_backup::config::datastore::complete_datastore_name)
                .into());

    cmd_def.into()
}

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("datastore".to_owned(), datastore_commands());

    if let Err(err) = run_cli_command(&cmd_def.into()) {
        eprintln!("Error: {}", err);
        print_cli_usage();
        std::process::exit(-1);
    }

}
