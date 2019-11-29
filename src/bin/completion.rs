use failure::*;
use serde_json::Value;

use proxmox::{sortable, identity};
use proxmox::api::*;
use proxmox::api::schema::*;

use proxmox_backup::cli::*;

#[sortable]
const API_METHOD_TEST_COMMAND: ApiMethod = ApiMethod::new(
    &ApiHandler::Sync(&test_command),
    &ObjectSchema::new(
        "Test command.",
        &sorted!([
            ( "verbose", true, &BooleanSchema::new("Verbose output.").schema() ),
        ])
    )
);

fn test_command(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {


    Ok(Value::Null)
}

fn command_map() -> CliCommandMap {
    let cmd_def = CliCommandMap::new()
        .insert("ls", CliCommand::new(&API_METHOD_TEST_COMMAND).into())
        .insert("test", CliCommand::new(&API_METHOD_TEST_COMMAND).into())
        .insert("help", help_command_def().into());

    cmd_def
}


fn main() -> Result<(), Error> {

    let def = CommandLineInterface::Nested(command_map());

    let helper = CliHelper::new(def);

    let config = rustyline::config::Builder::new()
    //.completion_type(rustyline::config::CompletionType::List)
    //.completion_prompt_limit(0)
        .build();

    let mut rl = rustyline::Editor::<CliHelper>::with_config(config);
    rl.set_helper(Some(helper));

    while let Ok(line) = rl.readline("# prompt: ") {
        let helper = rl.helper().unwrap();
        // readline already handles tabs, so here we only split on spaces
        let args = shellword_split(&line)?;

        let def = helper.cmd_def();
        let _ = match def {
            CommandLineInterface::Simple(ref cli_cmd) => {
                handle_simple_command(def, "", &cli_cmd, args)
            }
            CommandLineInterface::Nested(ref map) => {
                handle_nested_command(def, "", &map, args)
            }
        };
    }

    Ok(())
}
