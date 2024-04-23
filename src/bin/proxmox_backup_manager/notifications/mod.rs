use proxmox_router::cli::{CliCommandMap, CommandLineInterface};

mod targets;

pub fn notification_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new().insert("target", targets::commands());

    cmd_def.into()
}
