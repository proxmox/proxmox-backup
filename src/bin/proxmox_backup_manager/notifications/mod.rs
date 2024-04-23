use proxmox_router::cli::{CliCommandMap, CommandLineInterface};

mod gotify;
mod matchers;
mod sendmail;
mod smtp;
mod targets;

pub fn notification_commands() -> CommandLineInterface {
    let endpoint_def = CliCommandMap::new()
        .insert("gotify", gotify::commands())
        .insert("sendmail", sendmail::commands())
        .insert("smtp", smtp::commands());

    let cmd_def = CliCommandMap::new()
        .insert("endpoint", endpoint_def)
        .insert("matcher", matchers::commands())
        .insert("target", targets::commands());

    cmd_def.into()
}
