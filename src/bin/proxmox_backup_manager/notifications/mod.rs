use proxmox_router::cli::{CliCommandMap, CommandLineInterface};

mod matchers;
mod targets;

pub fn notification_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("target", targets::commands())
        .insert("matcher", matchers::commands());

    cmd_def.into()
}
