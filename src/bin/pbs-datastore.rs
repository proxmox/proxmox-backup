extern crate apitest;

use std::collections::HashMap;

use apitest::api3;
use apitest::cli::command::*;

fn datastore_commands() -> CommandLineInterface {

    let mut cmd_def = HashMap::<String, CommandLineInterface>::new();

    cmd_def.insert("list".to_owned(), CliCommand {
        info: api3::config::datastore::get(),
        arg_param: vec![],
        fixed_param: vec![],
    }.into());

    cmd_def.insert("create".to_owned(), CliCommand {
        info: api3::config::datastore::post(),
        arg_param: vec!["name", "path"],
        fixed_param: vec![],
    }.into());

    cmd_def.insert("remove".to_owned(), CliCommand {
        info: api3::config::datastore::delete(),
        arg_param: vec!["name"],
        fixed_param: vec![],
    }.into());

    cmd_def.into()
}

fn main() {

    let mut cmd_def = HashMap::new();

    cmd_def.insert("datastore".to_owned(), datastore_commands());

    if let Err(err) = run_cli_command(&CommandLineInterface::Nested(cmd_def)) {
        eprintln!("Error: {}", err);
        print_cli_usage();
        std::process::exit(-1);
    }

}
