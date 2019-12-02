use failure::*;

use proxmox::api::*;

use proxmox_backup::cli::*;

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
fn hello_command(
    verbose: Option<bool>,
) -> Result<(), Error> {
    if verbose.unwrap_or(false) {
        println!("Hello, how are you!");
    } else {
        println!("Hello!");
    }

    Ok(())
}

#[api(input: { properties: {} })]
/// Quit command. Exit the programm.
///
/// Returns: nothing
fn quit_command() -> Result<(), Error> {

    println!("Goodbye.");

    std::process::exit(0);
}

fn cli_definition() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("quit", CliCommand::new(&API_METHOD_QUIT_COMMAND).into())
        .insert("hello", CliCommand::new(&API_METHOD_HELLO_COMMAND).into())
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

        let _ = handle_command(helper.cmd_def(), "", args);

        rl.add_history_entry(line);
    }

    Ok(())
}
