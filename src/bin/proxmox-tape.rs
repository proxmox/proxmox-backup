use proxmox::{
    api::{
        cli::*,
        RpcEnvironment,
    },
};

mod proxmox_tape;
use proxmox_tape::*;

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("changer", changer_commands())
        .insert("drive", drive_commands())
        ;

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    proxmox_backup::tools::runtime::main(run_async_cli_command(cmd_def, rpcenv));
}
