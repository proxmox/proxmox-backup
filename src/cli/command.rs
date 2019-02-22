use failure::*;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

//use serde_json::Value;

use crate::api_schema::*;
use crate::api_schema::router::*;
//use crate::api_schema::config::*;
use super::environment::CliEnvironment;

use super::getopts;

#[derive(Copy, Clone)]
enum ParameterDisplayStyle {
    //Config,
    //SonfigSub,
    Arg,
    Fixed,
}

fn get_schema_type_text(schema: Arc<Schema>, _style: ParameterDisplayStyle) -> String {

    let type_text = match *schema {
        Schema::Null => String::from("<null>"), // should not happen
        Schema::String(_) => String::from("<string>"),
        Schema::Boolean(_) => String::from("<boolean>"),
        Schema::Integer(_) => String::from("<integer>"),
        Schema::Object(_) => String::from("<object>"),
        Schema::Array(_) => String::from("<array>"),
    };

    type_text
}

fn get_property_description(
    name: &str,
    schema: Arc<Schema>,
    style: ParameterDisplayStyle
) -> String {

    let type_text = get_schema_type_text(schema, style);

    let display_name = match style {
        ParameterDisplayStyle::Arg => {
            format!("--{}", name)
        }
        ParameterDisplayStyle::Fixed => {
            format!("<{}>", name)
        }
    };

    format!(" {:-10} {}", display_name, type_text)
}

fn generate_usage_str(prefix: &str, cli_cmd: &CliCommand, indent: &str) -> String {

    let arg_param = &cli_cmd.arg_param;
    let fixed_param = &cli_cmd.fixed_param;
    let properties = &cli_cmd.info.parameters.properties;

    let mut done_hash = HashSet::<&str>::new();
    let mut args = String::new();

    for positional_arg in arg_param {
        let (optional, _schema) = properties.get(positional_arg).unwrap();
        args.push(' ');
        if *optional { args.push('['); }
        args.push('<'); args.push_str(positional_arg); args.push('>');
        if *optional { args.push(']'); }

        //arg_descr.push_str(&get_property_description(positional_arg, schema.clone(), ParameterDisplayStyle::Fixed));
        done_hash.insert(positional_arg);
    }

    let mut options = String::new();

    let mut prop_names: Vec<&str> = properties.keys().map(|v| *v).collect();
    prop_names.sort();

    for prop in prop_names {
        let (optional, schema) = properties.get(prop).unwrap();
        if done_hash.contains(prop) { continue; }
        if fixed_param.contains(&prop) { continue; }

        let type_text = get_schema_type_text(schema.clone(), ParameterDisplayStyle::Arg);

        if *optional {

            options.push(' ');
            options.push_str(&get_property_description(prop, schema.clone(), ParameterDisplayStyle::Arg));

        } else {
            args.push_str("--"); args.push_str(prop);
            args.push(' ');
            args.push_str(&type_text);
        }

        done_hash.insert(prop);
    }


    format!("{}{}{}", indent, prefix, args)
}

fn print_simple_usage_error(prefix: &str, cli_cmd: &CliCommand, err: Error) {

    eprint!("Error: {}\nUsage: ", err);

    print_simple_usage(prefix, cli_cmd);
}

fn print_simple_usage(prefix: &str, cli_cmd: &CliCommand) {

    let usage =  generate_usage_str(prefix, cli_cmd, "");
    eprintln!("{}", usage);
}

fn handle_simple_command(prefix: &str, cli_cmd: &CliCommand, args: Vec<String>) {

    let (params, rest) = match getopts::parse_arguments(
        &args, &cli_cmd.arg_param, &cli_cmd.info.parameters) {
        Ok((p, r)) => (p, r),
        Err(err) => {
            print_simple_usage_error(prefix, cli_cmd, err.into());
            std::process::exit(-1);
        }
    };

    if !rest.is_empty() {
        let err = format_err!("got additional arguments: {:?}", rest);
        print_simple_usage_error(prefix, cli_cmd, err);
        std::process::exit(-1);
    }

    let mut rpcenv = CliEnvironment::new();

    match (cli_cmd.info.handler)(params, &cli_cmd.info, &mut rpcenv) {
        Ok(value) => {
            println!("Result: {}", serde_json::to_string_pretty(&value).unwrap());
        }
        Err(err) => {
            eprintln!("Error: {}", err);
        }
    }
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

fn print_nested_usage_error(prefix: &str, def: &CliCommandMap, err: Error) {

    eprintln!("Error: {}\n\nUsage:\n", err);

    print_nested_usage(prefix, def);
}

fn print_nested_usage(prefix: &str, def: &CliCommandMap) {

    let mut cmds: Vec<&String> = def.commands.keys().collect();
    cmds.sort();

    for cmd in cmds {
        let new_prefix = format!("{} {}", prefix, cmd);

        match def.commands.get(cmd).unwrap() {
            CommandLineInterface::Simple(cli_cmd) => {
                let usage =  generate_usage_str(&new_prefix, cli_cmd, "");
                eprintln!("{}", usage);
            }
            CommandLineInterface::Nested(map) => {
                print_nested_usage(&new_prefix, map);
            }
        }
    }

}

fn handle_nested_command(prefix: &str, def: &CliCommandMap, mut args: Vec<String>) {

    if args.len() < 1 {
        let mut cmds: Vec<&String> = def.commands.keys().collect();
        cmds.sort();

        let list = cmds.iter().fold(String::new(),|mut s,item| {
            if !s.is_empty() { s+= ", "; }
            s += item;
            s
        });

        let err = format_err!("no command specified.\nPossible commands: {}", list);
        print_nested_usage_error(prefix, def, err);
        std::process::exit(-1);
    }

    let command = args.remove(0);

    let sub_cmd = match find_command(def, &command) {
        Some(cmd) => cmd,
        None => {
            let err = format_err!("no such command '{}'", command);
            print_nested_usage_error(prefix, def, err);
            std::process::exit(-1);
        }
    };

    let new_prefix = format!("{} {}", prefix, command);

    match sub_cmd {
        CommandLineInterface::Simple(cli_cmd) => {
            handle_simple_command(&new_prefix, cli_cmd, args);
        }
        CommandLineInterface::Nested(map) => {
            handle_nested_command(&new_prefix, map, args);
        }
    }
}

fn print_property_completion(
    schema: &Schema,
    name: &str,
    completion_functions: &HashMap<String, CompletionFunction>,
    arg: &str)
{
    if let Some(callback) = completion_functions.get(name) {
        let list = (callback)(arg);
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

pub fn run_cli_command(def: &CommandLineInterface) {

    let mut args = std::env::args();

    let prefix = args.next().unwrap();
    let prefix = prefix.rsplit('/').next().unwrap(); // without path

    let args: Vec<String> = args.collect();

    if !args.is_empty() && args[0] == "bashcomplete" {
        print_bash_completion(def);
        return;
    }

    match def {
        CommandLineInterface::Simple(cli_cmd) => handle_simple_command(&prefix, cli_cmd, args),
        CommandLineInterface::Nested(map) => handle_nested_command(&prefix, map, args),
    };
}

pub type CompletionFunction = fn(&str) -> Vec<String>;

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
