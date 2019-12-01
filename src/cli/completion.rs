use super::*;

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
