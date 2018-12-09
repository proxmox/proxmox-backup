extern crate apitest;

use failure::*;
use std::collections::HashMap;

//use std::sync::Arc;

use apitest::api::schema::*;
use apitest::api::router::*;
use apitest::api::config::*;
use apitest::getopts;

use apitest::api3;

fn print_cli_usage() {

    eprintln!("Usage: TODO");
}

type CliCommandDefinition = HashMap<String, CliCommand>;

fn run_cli_command(def: &CliCommandDefinition) -> Result<(), Error> {

    let mut args: Vec<String> = std::env::args().skip(1).collect();

    if args.len() < 1 {
        bail!("no command specified.");
    }

    let command = args.remove(0);

    let cli_cmd = match def.get(&command) {
        Some(cmd) => cmd,
        None => {
            bail!("no such command '{}'", command);
        }
    };

    let (params, rest) = getopts::parse_arguments(
        &args, &cli_cmd.arg_param, &cli_cmd.info.parameters)?;

    if !rest.is_empty() {
        bail!("got additional arguments: {:?}", rest);
    }

    let res = (cli_cmd.info.handler)(params,  &cli_cmd.info)?;

    println!("Result: {}", serde_json::to_string_pretty(&res).unwrap());

    Ok(())
}

struct CliCommand {
    info: ApiMethod,
    arg_param: Vec<&'static str>,
    fixed_param: Vec<&'static str>,
}

fn main() {

    let mut cmd_def = HashMap::new();

    cmd_def.insert("list".to_owned(), CliCommand {
        info: api3::config::datastore::get(),
        arg_param: vec![],
        fixed_param: vec![],
    });

    cmd_def.insert("create".to_owned(), CliCommand {
        info: api3::config::datastore::post(),
        arg_param: vec!["name", "path"],
        fixed_param: vec![],
    });

    cmd_def.insert("remove".to_owned(), CliCommand {
        info: api3::config::datastore::delete(),
        arg_param: vec!["name"],
        fixed_param: vec![],
    });

    if let Err(err) = run_cli_command(&cmd_def) {
        eprintln!("Error: {}", err);
        print_cli_usage();
        std::process::exit(-1);
    }

}
