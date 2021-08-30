use proxmox::api::cli::*;

mod proxmox_backup_debug;
use proxmox_backup_debug::{inspect_commands, recover_commands};

fn main() {
    proxmox_backup::tools::setup_safe_path_env();

    let cmd_def = CliCommandMap::new()
        .insert("inspect", inspect_commands())
        .insert("recover", recover_commands());

    let rpcenv = CliEnvironment::new();
    run_cli_command(cmd_def, rpcenv, Some(|future| pbs_runtime::main(future)));
}
