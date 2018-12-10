use failure::*;
use std::collections::HashMap;

use crate::api::schema::*;
use crate::api::router::*;
use crate::api::config::*;
use crate::getopts;

pub fn print_cli_usage() {

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

fn handle_nested_command(def: &HashMap<String, CommandLineInterface>, mut args: Vec<String>) -> Result<(), Error> {

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
        CommandLineInterface::Simple(cli_cmd) => {
            handle_simple_command(cli_cmd, args)?;
        }
        CommandLineInterface::Nested(map) => {
            handle_nested_command(map, args)?;
        }
    }

    Ok(())
}

pub fn run_cli_command(def: &CommandLineInterface) -> Result<(), Error> {

    let args: Vec<String> = std::env::args().skip(1).collect();

    match def {
        CommandLineInterface::Simple(cli_cmd) => handle_simple_command(cli_cmd, args),
        CommandLineInterface::Nested(map) => handle_nested_command(map, args),
    }
}

pub struct CliCommand {
    pub info: ApiMethod,
    pub arg_param: Vec<&'static str>,
    pub fixed_param: Vec<&'static str>,
}

pub enum CommandLineInterface {
    Simple(CliCommand),
    Nested(HashMap<String, CommandLineInterface>),
}
