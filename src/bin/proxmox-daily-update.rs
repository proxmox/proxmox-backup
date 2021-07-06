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
        tokio::time::sleep(sleep_duration).await;
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

    match check_acme_certificates(rpcenv).await {
        Ok(()) => (),
        Err(err) => {
            eprintln!("error checking certificates: {}", err);
        }
    }

    // TODO: cleanup tasks like in PVE?

    Ok(Value::Null)
}

async fn check_acme_certificates(rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let (config, _) = proxmox_backup::config::node::config()?;

    // do we even have any acme domains configures?
    if config.acme_domains().next().is_none() {
        return Ok(());
    }

    if !api2::node::certificates::cert_expires_soon()? {
        println!("Certificate does not expire within the next 30 days, not renewing.");
        return Ok(());
    }

    let info = &api2::node::certificates::API_METHOD_RENEW_ACME_CERT;
    let result = match info.handler {
        ApiHandler::Sync(handler) => (handler)(json!({}), info, rpcenv)?,
        _ => unreachable!(),
    };
    wait_for_local_worker(result.as_str().unwrap()).await?;

    Ok(())
}

fn main() {
    proxmox_backup::tools::setup_safe_path_env();

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    if let Err(err) = pbs_runtime::main(do_update(&mut rpcenv)) {
        eprintln!("error during update: {}", err);
        std::process::exit(1);
    }
}
