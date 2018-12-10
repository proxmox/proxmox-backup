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


fn handle_simple_command(cli_cmd: &CliCommand, args: Vec<String>) -> Result<(), Error> {

    let (params, rest) = getopts::parse_arguments(
        &args, &cli_cmd.arg_param, &cli_cmd.info.parameters)?;

    if !rest.is_empty() {
        bail!("got additional arguments: {:?}", rest);
    }

    let res = (cli_cmd.info.handler)(params,  &cli_cmd.info)?;

    println!("Result: {}", serde_json::to_string_pretty(&res).unwrap());

    Ok(())
}

fn handle_nested_command(def: &HashMap<String, CmdDef>, mut args: Vec<String>) -> Result<(), Error> {

    if args.len() < 1 {
        let mut cmds: Vec<&String> = def.keys().collect();
        cmds.sort();

        let list = cmds.iter().fold(String::new(),|mut s,item| {
            if !s.is_empty() { s+= ", "; }
            s += item;
            s
        });

        bail!("expected command argument, but no command specified.\nPossible commands: {}", list);
    }

    let command = args.remove(0);

    let sub_cmd = match def.get(&command) {
        Some(cmd) => cmd,
        None => {
            bail!("no such command '{}'", command);
        }
    };

    match sub_cmd {
        CmdDef::Simple(cli_cmd) => {
            handle_simple_command(cli_cmd, args)?;
        }
        CmdDef::Nested(map) => {
            handle_nested_command(map, args)?;
        }
    }

    Ok(())
}

fn run_cli_command(def: &CmdDef) -> Result<(), Error> {

    let args: Vec<String> = std::env::args().skip(1).collect();

    match def {
        CmdDef::Simple(cli_cmd) => handle_simple_command(cli_cmd, args),
        CmdDef::Nested(map) => handle_nested_command(map, args),
    }
}

struct CliCommand {
    info: ApiMethod,
    arg_param: Vec<&'static str>,
    fixed_param: Vec<&'static str>,
}

enum CmdDef {
    Simple(CliCommand),
    Nested(HashMap<String, CmdDef>),
}

fn datastore_commands() -> CmdDef {

    let mut cmd_def = HashMap::new();

    cmd_def.insert("list".to_owned(), CmdDef::Simple(CliCommand {
        info: api3::config::datastore::get(),
        arg_param: vec![],
        fixed_param: vec![],
    }));

    cmd_def.insert("create".to_owned(), CmdDef::Simple(CliCommand {
        info: api3::config::datastore::post(),
        arg_param: vec!["name", "path"],
        fixed_param: vec![],
    }));

    cmd_def.insert("remove".to_owned(), CmdDef::Simple(CliCommand {
        info: api3::config::datastore::delete(),
        arg_param: vec!["name"],
        fixed_param: vec![],
    }));

    CmdDef::Nested(cmd_def)
}

fn main() {

    let mut cmd_def = HashMap::new();

    cmd_def.insert("datastore".to_owned(), datastore_commands());

    if let Err(err) = run_cli_command(&CmdDef::Nested(cmd_def)) {
        eprintln!("Error: {}", err);
        print_cli_usage();
        std::process::exit(-1);
    }

}
