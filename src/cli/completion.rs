use super::*;

use proxmox::api::schema::*;

fn record_done_argument(
    done: &mut HashMap<String, String>,
    parameters: &ObjectSchema,
    key: &str,
    value: &str
) {

    if let Some((_, schema)) = parameters.lookup(key) {
        match schema {
            Schema::Array(_) => { /* do nothing ?? */ }
            _ => { done.insert(key.to_owned(), value.to_owned()); }
        }
    }
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

fn get_simple_completion(
    cli_cmd: &CliCommand,
    done: &mut HashMap<String, String>,
    arg_param: &[&str], // we remove done arguments
    args: &[String],
) -> Vec<String> {
    // fixme: arg_param, fixed_param
    //eprintln!("COMPL: {:?} {:?} {}", arg_param, args, args.len());

    if !arg_param.is_empty() {
        let prop_name = arg_param[0];
        if args.len() > 1 {
            record_done_argument(done, cli_cmd.info.parameters, prop_name, &args[0]);
            return get_simple_completion(cli_cmd, done, &arg_param[1..], &args[1..]);
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
        if cli_cmd.arg_param.contains(name) { continue; }
        let option = String::from("--") + name;
        if option.starts_with(prefix) {
            completions.push(option);
        }
    }
    completions
}

fn get_help_completion(
    def: &CommandLineInterface,
    help_cmd: &CliCommand,
    args: &[String],
) -> Vec<String> {

    let mut done = HashMap::new();

    match def {
        CommandLineInterface::Simple(_) => {
            return get_simple_completion(help_cmd, &mut done, &[], args);
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
                if let Some(sub_cmd) = map.commands.get(first) { // do exact match here
                    return get_help_completion(sub_cmd, help_cmd, &args[1..]);
                }
                return Vec::new();
            }

            if first.starts_with("-") {
                return get_simple_completion(help_cmd, &mut done, &[], args);
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

fn get_nested_completion(
    def: &CommandLineInterface,
    args: &[String],
) -> Vec<String> {

    match def {
        CommandLineInterface::Simple(cli_cmd) => {
            let mut done: HashMap<String, String> = HashMap::new();
            cli_cmd.fixed_param.iter().for_each(|(key, value)| {
                record_done_argument(&mut done, &cli_cmd.info.parameters, &key, &value);
            });
            return get_simple_completion(cli_cmd, &mut done, &cli_cmd.arg_param, args);
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
                if let Some((_, sub_cmd)) = map.find_command(first) {
                    return get_nested_completion(sub_cmd, &args[1..]);
                }
                return Vec::new();
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

/// Helper to generate bash completions.
///
/// This helper extracts the command line from environment variable
/// set by ``bash``, namely ``COMP_LINE`` and ``COMP_POINT``. This is
/// passed to ``get_completions()``. Returned values are printed to
/// ``stdout``.
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

/// Compute possible completions for a partial command
pub fn get_completions(
    cmd_def: &CommandLineInterface,
    line: &str,
    skip_first: bool,
) -> (usize, Vec<String>) {

    let (mut args, start ) = match shellword_split_unclosed(line, false) {
        (mut args, None) => {
            args.push("".into());
            (args, line.len())
        }
        (mut args, Some((start , arg, _quote))) => {
            args.push(arg);
            (args, start)
        }
    };

    if skip_first {

        if args.len() == 0 { return (0, Vec::new()); }

        args.remove(0); // no need for program name
    }

    let completions = if !args.is_empty() && args[0] == "help" {
        get_help_completion(cmd_def, &help_command_def(), &args[1..])
    } else {
        get_nested_completion(cmd_def, &args)
    };

    (start, completions)
}
