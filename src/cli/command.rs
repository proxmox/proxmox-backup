use failure::*;
use std::collections::HashMap;
use std::collections::HashSet;
use serde_json::Value;

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

/// CLI usage information format
#[derive(Copy, Clone, PartialEq)]
enum DocumentationFormat {
    /// text, command line only (one line)
    Short,
    /// text, list all options
    Long,
    /// text, include description
    Full,
    /// like full, but in reStructuredText format
    ReST,
}

fn get_schema_type_text(schema: &Schema, _style: ParameterDisplayStyle) -> String {

    let type_text = match schema {
        Schema::Null => String::from("<null>"), // should not happen
        Schema::String(_) => String::from("<string>"),
        Schema::Boolean(_) => String::from("<boolean>"),
        Schema::Integer(integer_schema) => {
            if integer_schema.minimum != None && integer_schema.maximum != None {
                format!("<integer> ({} - {})", integer_schema.minimum.unwrap(), integer_schema.maximum.unwrap())
            } else if integer_schema.minimum != None {
                format!("<integer> ({} - N)", integer_schema.minimum.unwrap())
            } else if integer_schema.maximum != None {
                format!("<integer> (-N - {})", integer_schema.maximum.unwrap())
            } else {
                String::from("<integer>")
            }
         },
        Schema::Object(_) => String::from("<object>"),
        Schema::Array(_) => String::from("<array>"),
    };

    type_text
}

fn get_property_description(
    name: &str,
    schema: &Schema,
    style: ParameterDisplayStyle,
    format: DocumentationFormat,
) -> String {

    let type_text = get_schema_type_text(schema, style);

    let (descr, default) = match schema {
        Schema::Null => ("null", None),
        Schema::String(ref schema) => (schema.description, schema.default.map(|v| v.to_owned())),
        Schema::Boolean(ref schema) => (schema.description, schema.default.map(|v| v.to_string())),
        Schema::Integer(ref schema) => (schema.description, schema.default.map(|v| v.to_string())),
        Schema::Object(ref schema) => (schema.description, None),
        Schema::Array(ref schema) => (schema.description, None),
    };

    let default_text = match default {
        Some(text) =>  format!("   (default={})", text),
        None => String::new(),
    };

    if format == DocumentationFormat::ReST {

        let mut text = match style {
           ParameterDisplayStyle::Arg => {
                format!(":``--{} {}{}``:  ", name, type_text, default_text)
            }
            ParameterDisplayStyle::Fixed => {
                format!(":``<{}> {}{}``:  ", name, type_text, default_text)
            }
        };

        text.push_str(descr);
        text.push('\n');
        text.push('\n');

        text

    } else {

        let display_name = match style {
            ParameterDisplayStyle::Arg => {
                format!("--{}", name)
            }
            ParameterDisplayStyle::Fixed => {
                format!("<{}>", name)
            }
        };

        // fixme: wrap text
        let mut text = format!(" {:-10} {}{}", display_name, type_text, default_text);
        let indent = "             ";
        text.push('\n');
        text.push_str(indent);
        text.push_str(descr);
        text.push('\n');
        text.push('\n');

        text
    }
}

fn generate_usage_str(
    prefix: &str,
    cli_cmd: &CliCommand,
    format: DocumentationFormat,
    indent: &str) -> String {

    let arg_param = &cli_cmd.arg_param;
    let fixed_param = &cli_cmd.fixed_param;
    let properties = &cli_cmd.info.parameters.properties;
    let description = &cli_cmd.info.parameters.description;

    let mut done_hash = HashSet::<&str>::new();
    let mut args = String::new();

    for positional_arg in arg_param {
        match properties.get(positional_arg) {
            Some((optional, schema)) => {
                args.push(' ');

                let is_array = if let Schema::Array(_) = schema.as_ref() { true } else { false };
                if *optional { args.push('['); }
                if is_array { args.push('{'); }
                args.push('<'); args.push_str(positional_arg); args.push('>');
                if is_array { args.push('}'); }
                if *optional { args.push(']'); }

                done_hash.insert(positional_arg);
            }
            None => panic!("no such property '{}' in schema", positional_arg),
        }
    }

    let mut arg_descr = String::new();
    for positional_arg in arg_param {
        let (_optional, schema) = properties.get(positional_arg).unwrap();
        let param_descr = get_property_description(
            positional_arg, &schema, ParameterDisplayStyle::Fixed, format);
        arg_descr.push_str(&param_descr);
    }

    let mut options = String::new();

    let mut prop_names: Vec<&str> = properties.keys().map(|v| *v).collect();
    prop_names.sort();

    for prop in prop_names {
        let (optional, schema) = properties.get(prop).unwrap();
        if done_hash.contains(prop) { continue; }
        if fixed_param.contains_key(&prop) { continue; }

        let type_text = get_schema_type_text(&schema, ParameterDisplayStyle::Arg);

        if *optional {

            if options.len() > 0 { options.push('\n'); }
            options.push_str(&get_property_description(prop, &schema, ParameterDisplayStyle::Arg, format));

        } else {
            args.push_str("--"); args.push_str(prop);
            args.push(' ');
            args.push_str(&type_text);
        }

        done_hash.insert(prop);
    }

    let option_indicator = if options.len() > 0 { " [OPTIONS]" } else { "" };

    let mut text = match format {
        DocumentationFormat::Short => {
            return format!("{}{}{}{}\n\n", indent, prefix, args, option_indicator);
        }
        DocumentationFormat::Long => {
            format!("{}{}{}{}\n\n", indent, prefix, args, option_indicator)
        }
        DocumentationFormat::Full => {
            format!("{}{}{}{}\n\n{}\n\n", indent, prefix, args, option_indicator, description)
        }
        DocumentationFormat::ReST => {
            format!("``{} {}{}``\n\n{}\n\n", prefix, args.trim(), option_indicator, description)
        }
    };

    if arg_descr.len() > 0 {
        text.push_str(&arg_descr);
        text.push('\n');
    }
    if options.len() > 0 {
        text.push_str(&options);
        text.push('\n');
    }
    text
}

fn print_simple_usage_error(prefix: &str, cli_cmd: &CliCommand, err: Error) {

    let usage =  generate_usage_str(prefix, cli_cmd, DocumentationFormat::Long, "");
    eprint!("Error: {}\nUsage: {}", err, usage);
}

fn print_help(
    top_def: &CommandLineInterface,
    mut prefix: String,
    args: &Vec<String>,
    verbose: Option<bool>,
) {
    let mut iface = top_def;

    for cmd in args {
        if let CommandLineInterface::Nested(map) = iface {
            if let Some(subcmd) = find_command(map, cmd) {
                iface = subcmd;
                prefix.push(' ');
                prefix.push_str(cmd);
                continue;
            }
        }
        eprintln!("no such command '{}'", cmd);
        std::process::exit(-1);
    }

    let format = match verbose.unwrap_or(false) {
        true => DocumentationFormat::Full,
        false => DocumentationFormat::Short,
    };

    match iface {
        CommandLineInterface::Nested(map) => {
            println!("Usage:\n\n{}", generate_nested_usage(&prefix, map, format));
        }
        CommandLineInterface::Simple(cli_cmd) => {
            println!("Usage: {}", generate_usage_str(&prefix, cli_cmd, format, ""));
        }
    }
}

fn handle_simple_command(
    top_def: &CommandLineInterface,
    prefix: &str,
    cli_cmd: &CliCommand,
    args: Vec<String>,
) {

    let (params, rest) = match getopts::parse_arguments(
        &args, &cli_cmd.arg_param, &cli_cmd.info.parameters) {
        Ok((p, r)) => (p, r),
        Err(err) => {
            print_simple_usage_error(prefix, cli_cmd, err.into());
            std::process::exit(-1);
        }
    };

    if (cli_cmd.info.handler as *const fn()) == (dummy_help as *const fn()) {
        let prefix = prefix.split(' ').next().unwrap().to_string();
        print_help(top_def, prefix, &rest, params["verbose"].as_bool());
        return;
    }

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

    let usage = generate_nested_usage(prefix, def, DocumentationFormat::Short);

    eprintln!("Error: {}\n\nUsage:\n\n{}", err, usage);
}

fn generate_nested_usage(prefix: &str, def: &CliCommandMap, format: DocumentationFormat) -> String {

    let mut cmds: Vec<&String> = def.commands.keys().collect();
    cmds.sort();

    let mut usage = String::new();

    for cmd in cmds {
        let new_prefix = format!("{} {}", prefix, cmd);

        match def.commands.get(cmd).unwrap() {
            CommandLineInterface::Simple(cli_cmd) => {
                if usage.len() > 0 && format == DocumentationFormat::ReST {
                    usage.push_str("----\n\n");
                }
                usage.push_str(&generate_usage_str(&new_prefix, cli_cmd, format, ""));
            }
            CommandLineInterface::Nested(map) => {
                usage.push_str(&generate_nested_usage(&new_prefix, map, format));
            }
        }
    }

    usage
}

fn handle_nested_command(
    top_def: &CommandLineInterface,
    prefix: &str,
    def: &CliCommandMap,
    mut args: Vec<String>,
) {

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
            handle_simple_command(top_def, &new_prefix, cli_cmd, args);
        }
        CommandLineInterface::Nested(map) => {
            handle_nested_command(top_def, &new_prefix, map, args);
        }
    }
}

fn print_property_completion(
    schema: &Schema,
    name: &str,
    completion_functions: &HashMap<String, CompletionFunction>,
    arg: &str,
    param: &HashMap<String, String>,
) {
    if let Some(callback) = completion_functions.get(name) {
        let list = (callback)(arg, param);
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

fn record_done_argument(done: &mut HashMap<String, String>, parameters: &ObjectSchema, key: &str, value: &str) {

    if let Some((_, schema)) = parameters.properties.get::<str>(key) {
        match schema.as_ref() {
            Schema::Array(_) => { /* do nothing ?? */ }
            _ => { done.insert(key.to_owned(), value.to_owned()); }
        }
    }
}

fn print_simple_completion(
    cli_cmd: &CliCommand,
    done: &mut HashMap<String, String>,
    all_arg_param: &[&str], // this is always the full list
    arg_param: &[&str], // we remove done arguments
    args: &[String],
) {
    // fixme: arg_param, fixed_param
    //eprintln!("COMPL: {:?} {:?} {}", arg_param, args, args.len());

    if !arg_param.is_empty() {
        let prop_name = arg_param[0];
        if args.len() > 1 {
            record_done_argument(done, &cli_cmd.info.parameters, prop_name, &args[0]);
            print_simple_completion(cli_cmd, done, arg_param, &arg_param[1..], &args[1..]);
            return;
        } else if args.len() == 1 {
            record_done_argument(done, &cli_cmd.info.parameters, prop_name, &args[0]);
            if let Some((_, schema)) = cli_cmd.info.parameters.properties.get(prop_name) {
                print_property_completion(schema, prop_name, &cli_cmd.completion_functions, &args[0], done);
            }
        }
        return;
    }
    if args.is_empty() { return; }

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
            if let Some((_, schema)) = cli_cmd.info.parameters.properties.get(prop_name) {
                print_property_completion(schema, prop_name, &cli_cmd.completion_functions, &prefix, done);
            }
            return;
        }
    }

    for (name, (_optional, _schema)) in &cli_cmd.info.parameters.properties {
        if done.contains_key(*name) { continue; }
        if all_arg_param.contains(name) { continue; }
        let option = String::from("--") + name;
        if option.starts_with(prefix) {
            println!("{}", option);
        }
    }
}

fn print_help_completion(def: &CommandLineInterface, help_cmd: &CliCommand, args: &[String]) {

    let mut done = HashMap::new();

    match def {
        CommandLineInterface::Simple(_) => {
            print_simple_completion(help_cmd, &mut done, &help_cmd.arg_param, &help_cmd.arg_param, args);
            return;
        }
        CommandLineInterface::Nested(map) => {
            if args.is_empty() {
                for cmd in map.commands.keys() {
                    println!("{}", cmd);
                }
                return;
            }

            let first = &args[0];

            if first.starts_with("-") {
                print_simple_completion(help_cmd, &mut done, &help_cmd.arg_param, &help_cmd.arg_param, args);
                return;
            }

            if let Some(sub_cmd) = map.commands.get(first) {
                print_help_completion(sub_cmd, help_cmd, &args[1..]);
                return;
            }

            for cmd in map.commands.keys() {
                if cmd.starts_with(first) {
                    println!("{}", cmd);
                }
            }
        }
    }
}

fn print_nested_completion(def: &CommandLineInterface, args: &[String]) {

    match def {
        CommandLineInterface::Simple(cli_cmd) => {
            let mut done: HashMap<String, String> = HashMap::new();
            cli_cmd.fixed_param.iter().for_each(|(key, value)| {
                record_done_argument(&mut done, &cli_cmd.info.parameters, &key, &value);
            });
            print_simple_completion(cli_cmd, &mut done, &cli_cmd.arg_param, &cli_cmd.arg_param, args);
            return;
        }
        CommandLineInterface::Nested(map) => {
            if args.is_empty() {
                for cmd in map.commands.keys() {
                    println!("{}", cmd);
                }
                return;
            }
            let first = &args[0];
            if args.len() > 1 {
                if let Some(sub_cmd) = map.commands.get(first) {
                    print_nested_completion(sub_cmd, &args[1..]);
                    return;
                }
            }
            for cmd in map.commands.keys() {
                if cmd.starts_with(first) {
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

    if args.len() == 0 { return; }

    args.remove(0); //no need for program name

    if cmdline.ends_with(char::is_whitespace) {
        //eprintln!("CMDLINE {:?}", cmdline);
        args.push("".into());
    }

    if !args.is_empty() && args[0] == "help" {
        print_help_completion(def, &help_command_def(), &args[1..]);
    } else {
        print_nested_completion(def, &args);
    }
}

fn help_command_def() ->  CliCommand {
    CliCommand::new(
        ApiMethod::new(
            dummy_help,
            ObjectSchema::new("Get help about specified command.")
                .optional("verbose", BooleanSchema::new("Verbose help."))
        )
    )
}

pub fn run_cli_command(def: CommandLineInterface) {

    let def = match def {
        CommandLineInterface::Simple(cli_cmd) => CommandLineInterface::Simple(cli_cmd),
        CommandLineInterface::Nested(map) =>
            CommandLineInterface::Nested(map.insert("help", help_command_def().into())),
    };

    let top_def = &def; // we pass this to the help function ...

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

    match def {
        CommandLineInterface::Simple(ref cli_cmd) => handle_simple_command(top_def, &prefix, &cli_cmd, args),
        CommandLineInterface::Nested(ref map) => handle_nested_command(top_def, &prefix, &map, args),
    };
}

pub type CompletionFunction = fn(&str, &HashMap<String, String>) -> Vec<String>;

pub struct CliCommand {
    pub info: ApiMethod,
    pub arg_param: Vec<&'static str>,
    pub fixed_param: HashMap<&'static str, String>,
    pub completion_functions: HashMap<String, CompletionFunction>,
}

impl CliCommand {

    pub fn new(info: ApiMethod) -> Self {
        Self {
            info, arg_param: vec![],
            fixed_param: HashMap::new(),
            completion_functions: HashMap::new(),
        }
    }

    pub fn arg_param(mut self, names: Vec<&'static str>) -> Self {
        self.arg_param = names;
        self
    }

    pub fn fixed_param(mut self, key: &'static str, value: String) -> Self {
        self.fixed_param.insert(key, value);
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

fn dummy_help(_param: Value, _info: &ApiMethod, _rpcenv: &mut RpcEnvironment) -> Result<Value, Error> {
    panic!("internal error"); // this is just a place holder - never call this
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
