use proxmox_router::{
    cli::{run_cli_command, CliCommandMap, CliEnvironment},
    RpcEnvironment,
};

mod proxmox_backup_debug;
use proxmox_backup_debug::*;

fn main() {
    let cmd_def = CliCommandMap::new()
        .insert("inspect", inspect::inspect_commands())
        .insert("recover", recover::recover_commands())
        .insert("api", api::api_commands());

    let uid = nix::unistd::Uid::current();
    let username = match nix::unistd::User::from_uid(uid) {
        Ok(Some(user)) => user.name,
        _ => "root@pam".to_string(),
    };
    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(format!("{}@pam", username)));

    run_cli_command(cmd_def, rpcenv, Some(|future| pbs_runtime::main(future)));
}
