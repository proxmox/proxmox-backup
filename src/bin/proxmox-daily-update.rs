use anyhow::Error;
use serde_json::{json, Value};

use proxmox::api::{cli::*, RpcEnvironment, ApiHandler};

use proxmox_backup::api2;
use proxmox_backup::tools::subscription;

async fn wait_for_local_worker(upid_str: &str) -> Result<(), Error> {
    let upid: proxmox_backup::server::UPID = upid_str.parse()?;
    let sleep_duration = core::time::Duration::new(0, 100_000_000);

    loop {
        if !proxmox_backup::server::worker_is_active_local(&upid) {
            break;
        }
        tokio::time::delay_for(sleep_duration).await;
    }
    Ok(())
}

/// Daily update
async fn do_update(
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let param = json!({});

    let method = &api2::node::subscription::API_METHOD_CHECK_SUBSCRIPTION;
    let _res = match method.handler {
        ApiHandler::Sync(handler) => (handler)(param, method, rpcenv)?,
        _ => unreachable!(),
    };

    let notify = match subscription::read_subscription() {
        Ok(Some(subscription)) => subscription.status == subscription::SubscriptionStatus::ACTIVE,
        Ok(None) => false,
        Err(err) => {
            eprintln!("Error reading subscription - {}", err);
            false
        },
    };

    let param = json!({
        "notify": notify,
    });
    let method = &api2::node::apt::API_METHOD_APT_UPDATE_DATABASE;
    let upid = match method.handler {
        ApiHandler::Sync(handler) => (handler)(param, method, rpcenv)?,
        _ => unreachable!(),
    };
    wait_for_local_worker(upid.as_str().unwrap()).await?;

    // TODO: certificate checks/renewal/... ?

    // TODO: cleanup tasks like in PVE?

    Ok(Value::Null)
}

fn main() {
    proxmox_backup::tools::setup_safe_path_env();

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    match proxmox_backup::tools::runtime::main(do_update(&mut rpcenv)) {
        Err(err) => {
            eprintln!("error during update: {}", err);
            std::process::exit(1);
        },
        _ => (),
    }
}
