use failure::*;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::cell::RefCell;

use proxmox::api::*;
use proxmox::api::format::*;
use proxmox::api::schema::*;
use proxmox::api::{ApiHandler, ApiMethod};

use super::environment::CliEnvironment;

use super::getopts;
use super::{CommandLineInterface, CliCommand, CliCommandMap, CompletionFunction};
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

    let sub_cmd = match def.find_command(&command) {
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

fn get_property_completion(
    schema: &Schema,
    name: &str,
    completion_functions: &HashMap<String, CompletionFunction>,
    arg: &str,
    param: &HashMap<String, String>,
) -> Vec<String> {

    if let Some(callback) = completion_functions.get(name) {
        let list = (callback)(arg, param);
        let mut completions = Vec::new();
        for value in list {
            if value.starts_with(arg) {
                completions.push(value);
            }
        }
        return completions;
    }

    if let Schema::String(StringSchema { format: Some(format),  ..} ) = schema {
        if let ApiStringFormat::Enum(list) = format {
            let mut completions = Vec::new();
            for value in list.iter() {
                if value.starts_with(arg) {
                    completions.push(value.to_string());
                }
            }
            return completions;
        }
    }
    return Vec::new();
}

fn record_done_argument(done: &mut HashMap<String, String>, parameters: &ObjectSchema, key: &str, value: &str) {

    if let Some((_, schema)) = parameters.lookup(key) {
        match schema {
            Schema::Array(_) => { /* do nothing ?? */ }
            _ => { done.insert(key.to_owned(), value.to_owned()); }
        }
    }
}

pub fn get_simple_completion(
    cli_cmd: &CliCommand,
    done: &mut HashMap<String, String>,
    all_arg_param: &[&str], // this is always the full list
    arg_param: &[&str], // we remove done arguments
    args: &[String],
) -> Vec<String> {
    // fixme: arg_param, fixed_param
    //eprintln!("COMPL: {:?} {:?} {}", arg_param, args, args.len());

    if !arg_param.is_empty() {
        let prop_name = arg_param[0];
        if args.len() > 1 {
            record_done_argument(done, cli_cmd.info.parameters, prop_name, &args[0]);
            return get_simple_completion(cli_cmd, done, arg_param, &arg_param[1..], &args[1..]);
        } else if args.len() == 1 {
            record_done_argument(done, cli_cmd.info.parameters, prop_name, &args[0]);
            if let Some((_, schema)) = cli_cmd.info.parameters.lookup(prop_name) {
                return get_property_completion(schema, prop_name, &cli_cmd.completion_functions, &args[0], done);
            }
        }
        return Vec::new();
    }
    if args.is_empty() { return Vec::new(); }

    // Try to parse all argumnets but last, record args already done
    if args.len() > 1 {
        let mut errors = ParameterError::new(); // we simply ignore any parsing errors here
        let (data, _rest) = getopts::parse_argument_list(&args[0..args.len()-1], &cli_cmd.info.parameters, &mut errors);
        for (key, value) in &data {
            record_done_argument(done, &cli_cmd.info.parameters, key, value);
        }
    }

    let prefix = &args[args.len()-1]; // match on last arg

    // complete option-name or option-value ?
    if !prefix.starts_with("-") && args.len() > 1 {
        let last = &args[args.len()-2];
        if last.starts_with("--") && last.len() > 2 {
            let prop_name = &last[2..];
            if let Some((_, schema)) = cli_cmd.info.parameters.lookup(prop_name) {
                return get_property_completion(schema, prop_name, &cli_cmd.completion_functions, &prefix, done);
            }
            return Vec::new();
        }
    }

    let mut completions = Vec::new();
    for (name, _optional, _schema) in cli_cmd.info.parameters.properties {
        if done.contains_key(*name) { continue; }
        if all_arg_param.contains(name) { continue; }
        let option = String::from("--") + name;
        if option.starts_with(prefix) {
            completions.push(option);
        }
    }
    completions
}

pub fn get_help_completion(
    def: &CommandLineInterface,
    help_cmd: &CliCommand,
    args: &[String],
) -> Vec<String> {

    let mut done = HashMap::new();

    match def {
        CommandLineInterface::Simple(_) => {
            return get_simple_completion(help_cmd, &mut done, help_cmd.arg_param, &[], args);
        }
        CommandLineInterface::Nested(map) => {
            if args.is_empty() {
                let mut completions = Vec::new();
                for cmd in map.commands.keys() {
                    completions.push(cmd.to_string());
                }
                return completions;
            }

            let first = &args[0];

            if first.starts_with("-") {
                return get_simple_completion(help_cmd, &mut done, help_cmd.arg_param, &[], args);
            }

            if let Some(sub_cmd) = map.commands.get(first) {
                return get_help_completion(sub_cmd, help_cmd, &args[1..]);
            }

            let mut completions = Vec::new();
            for cmd in map.commands.keys() {
                if cmd.starts_with(first) {
                    completions.push(cmd.to_string());
                }
            }
            return completions;
        }
    }
}

pub fn get_nested_completion(
    def: &CommandLineInterface,
    args: &[String],
) -> Vec<String> {

    match def {
        CommandLineInterface::Simple(cli_cmd) => {
            let mut done: HashMap<String, String> = HashMap::new();
            cli_cmd.fixed_param.iter().for_each(|(key, value)| {
                record_done_argument(&mut done, &cli_cmd.info.parameters, &key, &value);
            });
            return get_simple_completion(cli_cmd, &mut done, cli_cmd.arg_param, &cli_cmd.arg_param, args);
        }
        CommandLineInterface::Nested(map) => {
            if args.is_empty() {
                let mut completions = Vec::new();
                for cmd in map.commands.keys() {
                    completions.push(cmd.to_string());
                }
                return completions;
            }
            let first = &args[0];
            if args.len() > 1 {
                if let Some(sub_cmd) = map.commands.get(first) {
                    return get_nested_completion(sub_cmd, &args[1..]);
                }
            }
            let mut completions = Vec::new();
            for cmd in map.commands.keys() {
                if cmd.starts_with(first) {
                    completions.push(cmd.to_string());
                }
            }
            return completions;
        }
    }
}

pub fn print_bash_completion(def: &CommandLineInterface) {

    let comp_point: usize = match std::env::var("COMP_POINT") {
        Ok(val) => {
            match usize::from_str_radix(&val, 10) {
                Ok(i) => i,
                Err(_) => return,
            }
        }
        Err(_) => return,
    };

    let cmdline = match std::env::var("COMP_LINE") {
        Ok(val) => val[0..comp_point].to_owned(),
        Err(_) => return,
    };

    let (_start, completions) = super::get_completions(def, &cmdline, true);

    for item in completions {
        println!("{}", item);
    }
}

const API_METHOD_COMMAND_HELP: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&help_command),
    &ObjectSchema::new(
        "Get help about specified command.",
        &[
            ( "command",
               true,
               &StringSchema::new("Command name.").schema()
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


    let command = param["command"].as_str();
    let verbose = param["verbose"].as_bool();

    HELP_CONTEXT.with(|ctx| {
        match &*ctx.borrow() {
            Some(def) => {
                let mut args = Vec::new();
                // TODO: Handle multilevel sub commands
                if let Some(command) = command {
                    args.push(command.to_string());
                }

                print_help(def, String::from(""), &args, verbose);
            }
            None => {
                eprintln!("Sorry, help context not set - internal error.");
            }
        }
    });

    Ok(Value::Null)
}

pub fn set_help_context(def: Option<Arc<CommandLineInterface>>) {
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
