use anyhow::Error;

use proxmox_router::cli::*;
use proxmox_schema::*;

#[api(
    input: {
        properties: {
            text: {
                type: String,
                description: "Some text.",
            }
        }
    },
)]
/// Echo command. Print the passed text.
///
/// Returns: nothing
fn echo_command(text: String) -> Result<(), Error> {
    println!("{}", text);
    Ok(())
}

#[api(
    input: {
        properties: {
            verbose: {
                type: Boolean,
                optional: true,
                description: "Verbose output.",
            }
        }
    },
)]
/// Hello command.
///
/// Returns: nothing
fn hello_command(verbose: Option<bool>) -> Result<(), Error> {
    if verbose.unwrap_or(false) {
        println!("Hello, how are you!");
    } else {
        println!("Hello!");
    }

    Ok(())
}

#[api(input: { properties: {} })]
/// Quit command. Exit the program.
///
/// Returns: nothing
fn quit_command() -> Result<(), Error> {
    println!("Goodbye.");

    std::process::exit(0);
}

fn cli_definition() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("quit", CliCommand::new(&API_METHOD_QUIT_COMMAND))
        .insert("hello", CliCommand::new(&API_METHOD_HELLO_COMMAND))
        .insert(
            "echo",
            CliCommand::new(&API_METHOD_ECHO_COMMAND).arg_param(&["text"]),
        )
        .insert_help();

    CommandLineInterface::Nested(cmd_def)
}

fn main() -> Result<(), Error> {
    let helper = CliHelper::new(cli_definition());

    let mut rl = rustyline::Editor::<CliHelper>::new();
    rl.set_helper(Some(helper));

    while let Ok(line) = rl.readline("# prompt: ") {
        let helper = rl.helper().unwrap();

        let args = shellword_split(&line)?;

        let rpcenv = CliEnvironment::new();
        let _ = handle_command(helper.cmd_def(), "", args, rpcenv, None);

        rl.add_history_entry(line);
    }

    Ok(())
}
