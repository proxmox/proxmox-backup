use failure::*;
use std::collections::HashMap;
use std::collections::HashSet;

use crate::api::schema::*;
use crate::api::router::*;
//use crate::api::config::*;
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

fn find_command<'a>(def: &'a CliCommandMap, name: &str) -> Option<&'a CommandLineInterface> {

    if let Some(sub_cmd) = def.commands.get(name) {
        return Some(sub_cmd);
    };

    let mut matches: Vec<&str> = vec![];

    for cmd in def.commands.keys() {
        if cmd.starts_with(name) {
             matches.push(cmd); }
    }

    if matches.len() != 1 { return None; }

    if let Some(sub_cmd) = def.commands.get(matches[0]) {
        return Some(sub_cmd);
    };

    None
}

fn handle_nested_command(def: &CliCommandMap, mut args: Vec<String>) -> Result<(), Error> {

    if args.len() < 1 {
        let mut cmds: Vec<&String> = def.commands.keys().collect();
        cmds.sort();

        let list = cmds.iter().fold(String::new(),|mut s,item| {
            if !s.is_empty() { s+= ", "; }
            s += item;
            s
        });

        bail!("expected command argument, but no command specified.\nPossible commands: {}", list);
    }

    let command = args.remove(0);

    let sub_cmd = match find_command(def, &command) {
        Some(cmd) => cmd,
        None => bail!("no such command '{}'", command),
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

fn print_property_completion(
    schema: &Schema,
    name: &str,
    completion_functions: &HashMap<String, CompletionFunction>,
    arg: &str)
{
    if let Some(callback) = completion_functions.get(name) {
        let list = (callback)();
        for value in list {
            if value.starts_with(arg) {
                println!("{}", value);
            }
        }
        return;
    }

    if let Schema::String(StringSchema { format: Some(format),  ..} ) = schema {
        if let ApiStringFormat::Enum(list) = format.as_ref() {
            for value in list {
                if value.starts_with(arg) {
                    println!("{}", value);
                }
            }
            return;
        }
    }
    println!("");
}

fn record_done_arguments(done: &mut HashSet<String>, parameters: &ObjectSchema, list: &[String]) {

    for arg in list {
        if arg.starts_with("--") && arg.len() > 2 {
            let prop_name = arg[2..].to_owned();
            if let Some((_, schema)) = parameters.properties.get::<str>(&prop_name) {
                match schema.as_ref() {
                    Schema::Array(_) => { /* do nothing */ }
                    _ => { done.insert(prop_name); }
                }
            }
        }
    }
}

fn print_simple_completion(
    cli_cmd: &CliCommand,
    done: &mut HashSet<String>,
    arg_param: &[&str],
    mut args: Vec<String>,
) {
    // fixme: arg_param, fixed_param
    //eprintln!("COMPL: {:?} {:?} {}", arg_param, args, args.len());

    if !arg_param.is_empty() {
        let prop_name = arg_param[0];
        done.insert(prop_name.into());
        if args.len() > 1 {
            args.remove(0);
            print_simple_completion(cli_cmd, done, &arg_param[1..], args);
            return;
        } else if args.len() == 1 {
            if let Some((_, schema)) = cli_cmd.info.parameters.properties.get(prop_name) {
                print_property_completion(schema, prop_name, &cli_cmd.completion_functions, &args[0]);
            }
        }
        return;
    }
    if args.is_empty() { return; }

    record_done_arguments(done, &cli_cmd.info.parameters, &args);

    let prefix = args.pop().unwrap(); // match on last arg

    // complete option-name or option-value ?
    if !prefix.starts_with("-") && args.len() > 0 {
        let last = &args[args.len()-1];
        if last.starts_with("--") && last.len() > 2 {
            let prop_name = &last[2..];
            if let Some((_, schema)) = cli_cmd.info.parameters.properties.get(prop_name) {
                print_property_completion(schema, prop_name, &cli_cmd.completion_functions, &prefix);
            }
            return;
        }
    }

    for (name, (_optional, _schema)) in &cli_cmd.info.parameters.properties {
        if done.contains(*name) { continue; }
        let option = String::from("--") + name;
        if option.starts_with(&prefix) {
            println!("{}", option);
        }
    }
}

fn print_nested_completion(def: &CommandLineInterface, mut args: Vec<String>) {

    match def {
        CommandLineInterface::Simple(cli_cmd) => {
            let mut done = HashSet::new();
            let fixed: Vec<String> = cli_cmd.fixed_param.iter().map(|s| s.to_string()).collect();
            record_done_arguments(&mut done, &cli_cmd.info.parameters, &fixed);
            print_simple_completion(cli_cmd, &mut done, &cli_cmd.arg_param, args);
            return;
        }
        CommandLineInterface::Nested(map) => {
            if args.is_empty() {
                for cmd in map.commands.keys() {
                    println!("{}", cmd);
                }
                return;
            }
            let first = args.remove(0);
            if let Some(sub_cmd) = map.commands.get(&first) {
                print_nested_completion(sub_cmd, args);
                return;
            }
            for cmd in map.commands.keys() {
                if cmd.starts_with(&first) {
                    println!("{}", cmd);
                }
            }
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


    let mut args = match shellwords::split(&cmdline) {
        Ok(v) => v,
        Err(_) => return,
    };

    args.remove(0); //no need for program name

    if cmdline.ends_with(char::is_whitespace) {
        //eprintln!("CMDLINE {:?}", cmdline);
        args.push("".into());
    }

    //eprintln!("COMP_ARGS {:?}", args);

    print_nested_completion(def, args);
}

pub fn run_cli_command(def: &CommandLineInterface) -> Result<(), Error> {

    let args: Vec<String> = std::env::args().skip(1).collect();

    if !args.is_empty() && args[0] == "bashcomplete" {
        print_bash_completion(def);
        return Ok(());
    }

    match def {
        CommandLineInterface::Simple(cli_cmd) => handle_simple_command(cli_cmd, args),
        CommandLineInterface::Nested(map) => handle_nested_command(map, args),
    }
}

pub type CompletionFunction = fn() -> Vec<String>;

pub struct CliCommand {
    pub info: ApiMethod,
    pub arg_param: Vec<&'static str>,
    pub fixed_param: Vec<&'static str>,
    pub completion_functions: HashMap<String, CompletionFunction>,
}

impl CliCommand {

    pub fn new(info: ApiMethod) -> Self {
        Self {
            info, arg_param: vec![],
            fixed_param: vec![],
            completion_functions: HashMap::new(),
        }
    }

    pub fn arg_param(mut self, names: Vec<&'static str>) -> Self {
        self.arg_param = names;
        self
    }

    pub fn fixed_param(mut self, args: Vec<&'static str>) -> Self {
        self.fixed_param = args;
        self
    }

    pub fn completion_cb(mut self, param_name: &str, cb:  CompletionFunction) -> Self {
        self.completion_functions.insert(param_name.into(), cb);
        self
    }
}

pub struct CliCommandMap {
    pub commands: HashMap<String, CommandLineInterface>,
}

impl CliCommandMap {

    pub fn new() -> Self {
        Self { commands: HashMap:: new() }
    }

    pub fn insert<S: Into<String>>(mut self, name: S, cli: CommandLineInterface) -> Self {
        self.commands.insert(name.into(), cli);
        self
    }
}

pub enum CommandLineInterface {
    Simple(CliCommand),
    Nested(CliCommandMap),
}

impl From<CliCommand> for CommandLineInterface {
    fn from(cli_cmd: CliCommand) -> Self {
         CommandLineInterface::Simple(cli_cmd)
    }
}

impl From<CliCommandMap> for CommandLineInterface {
    fn from(list: CliCommandMap) -> Self {
        CommandLineInterface::Nested(list)
    }
}
