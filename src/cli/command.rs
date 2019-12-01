use failure::*;
use serde_json::Value;
use std::sync::Arc;
use std::cell::RefCell;

use proxmox::api::*;
use proxmox::api::format::*;
use proxmox::api::schema::*;
use proxmox::api::{ApiHandler, ApiMethod};

use super::environment::CliEnvironment;

use super::getopts;
use super::{CommandLineInterface, CliCommand, CliCommandMap, completion::*};
use super::format::*;

pub const OUTPUT_FORMAT: Schema =
    StringSchema::new("Output format.")
    .format(&ApiStringFormat::Enum(&["text", "json", "json-pretty"]))
    .schema();

pub fn handle_simple_command(
    _top_def: &CommandLineInterface,
    prefix: &str,
    cli_cmd: &CliCommand,
    args: Vec<String>,
) -> Result<(), Error> {

    let (params, rest) = match getopts::parse_arguments(
        &args, cli_cmd.arg_param, &cli_cmd.info.parameters) {
        Ok((p, r)) => (p, r),
        Err(err) => {
            let err_msg = err.to_string();
            print_simple_usage_error(prefix, cli_cmd, &err_msg);
            return Err(format_err!("{}", err_msg));
        }
    };

    if !rest.is_empty() {
        let err_msg = format!("got additional arguments: {:?}", rest);
        print_simple_usage_error(prefix, cli_cmd, &err_msg);
        return Err(format_err!("{}", err_msg));
    }

    let mut rpcenv = CliEnvironment::new();

    match cli_cmd.info.handler {
        ApiHandler::Sync(handler) => {
            match (handler)(params, &cli_cmd.info, &mut rpcenv) {
                Ok(value) => {
                    if value != Value::Null {
                        println!("Result: {}", serde_json::to_string_pretty(&value).unwrap());
                    }
                }
                Err(err) => {
                    eprintln!("Error: {}", err);
                    return Err(err);
                }
            }
        }
        ApiHandler::AsyncHttp(_) => {
            let err_msg =
                "CliHandler does not support ApiHandler::AsyncHttp - internal error";
            print_simple_usage_error(prefix, cli_cmd, err_msg);
            return Err(format_err!("{}", err_msg));
        }
    }

    Ok(())
}

pub fn handle_nested_command(
    top_def: &CommandLineInterface,
    prefix: &str,
    def: &CliCommandMap,
    mut args: Vec<String>,
) -> Result<(), Error> {

    if args.len() < 1 {
        let mut cmds: Vec<&String> = def.commands.keys().collect();
        cmds.sort();

        let list = cmds.iter().fold(String::new(),|mut s,item| {
            if !s.is_empty() { s+= ", "; }
            s += item;
            s
        });

        let err_msg = format!("no command specified.\nPossible commands: {}", list);
        print_nested_usage_error(prefix, def, &err_msg);
        return Err(format_err!("{}", err_msg));
    }

    let command = args.remove(0);

    let (_, sub_cmd) = match def.find_command(&command) {
        Some(cmd) => cmd,
        None => {
            let err_msg = format!("no such command '{}'", command);
            print_nested_usage_error(prefix, def, &err_msg);
            return Err(format_err!("{}", err_msg));
        }
    };

    let new_prefix = format!("{} {}", prefix, command);

    match sub_cmd {
        CommandLineInterface::Simple(cli_cmd) => {
            handle_simple_command(top_def, &new_prefix, cli_cmd, args)?;
        }
        CommandLineInterface::Nested(map) => {
            handle_nested_command(top_def, &new_prefix, map, args)?;
        }
    }

    Ok(())
}

const API_METHOD_COMMAND_HELP: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&help_command),
    &ObjectSchema::new(
        "Get help about specified command (or sub-command).",
        &[
            ( "command",
               true,
               &ArraySchema::new(
                   "Command. This may be a list in order to spefify nested sub-commands.",
                   &StringSchema::new("Name.").schema()
               ).schema()
            ),
            ( "verbose",
               true,
               &BooleanSchema::new("Verbose help.").schema()
            ),
        ],
    )
);

std::thread_local! {
    static HELP_CONTEXT: RefCell<Option<Arc<CommandLineInterface>>> = RefCell::new(None);
}

fn help_command(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let command: Vec<String> = param["command"].as_array().unwrap_or(&Vec::new())
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();


    let verbose = param["verbose"].as_bool();

    HELP_CONTEXT.with(|ctx| {
        match &*ctx.borrow() {
            Some(def) => {
                 print_help(def, String::from(""), &command, verbose);
            }
            None => {
                eprintln!("Sorry, help context not set - internal error.");
            }
        }
    });

    Ok(Value::Null)
}

fn set_help_context(def: Option<Arc<CommandLineInterface>>) {
    HELP_CONTEXT.with(|ctx| { *ctx.borrow_mut() = def; });
}

pub fn help_command_def() ->  CliCommand {
    CliCommand::new(&API_METHOD_COMMAND_HELP)
        .arg_param(&["command"])
}

pub fn handle_command(
    def: Arc<CommandLineInterface>,
    prefix: &str,
    args: Vec<String>,
) -> Result<(), Error> {

    set_help_context(Some(def.clone()));

    let result = match &*def {
        CommandLineInterface::Simple(ref cli_cmd) => {
            handle_simple_command(&def, &prefix, &cli_cmd, args)
        }
        CommandLineInterface::Nested(ref map) => {
            handle_nested_command(&def, &prefix, &map, args)
        }
    };

    set_help_context(None);

    result
}

pub fn run_cli_command(def: CommandLineInterface) {

    let def = match def {
        CommandLineInterface::Simple(cli_cmd) => CommandLineInterface::Simple(cli_cmd),
        CommandLineInterface::Nested(map) =>
            CommandLineInterface::Nested(map.insert("help", help_command_def().into())),
    };

    let mut args = std::env::args();

    let prefix = args.next().unwrap();
    let prefix = prefix.rsplit('/').next().unwrap(); // without path

    let args: Vec<String> = args.collect();

    if !args.is_empty() {
        if args[0] == "bashcomplete" {
            print_bash_completion(&def);
            return;
        }

        if args[0] == "printdoc" {
            let usage = match def {
                CommandLineInterface::Simple(cli_cmd) => {
                    generate_usage_str(&prefix, &cli_cmd,  DocumentationFormat::ReST, "")
                }
                CommandLineInterface::Nested(map) => {
                    generate_nested_usage(&prefix, &map, DocumentationFormat::ReST)
                }
            };
            println!("{}", usage);
            return;
        }
    }

    if let Err(_) = handle_command(Arc::new(def), &prefix, args) {
        std::process::exit(-1);
    }
}
