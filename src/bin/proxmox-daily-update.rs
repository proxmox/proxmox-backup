use anyhow::Error;
use serde_json::json;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_subscription::SubscriptionStatus;
use proxmox_sys::fs::CreateOptions;

use proxmox_backup::api2;

async fn wait_for_local_worker(upid_str: &str) -> Result<(), Error> {
    let upid: pbs_api_types::UPID = upid_str.parse()?;
    let poll_delay = core::time::Duration::from_millis(100);

    loop {
        if !proxmox_rest_server::worker_is_active_local(&upid) {
            break;
        }
        tokio::time::sleep(poll_delay).await;
    }
    Ok(())
}

/// Daily update
async fn do_update(rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let param = json!({});

    let method = &api2::node::subscription::API_METHOD_CHECK_SUBSCRIPTION;
    match method.handler {
        ApiHandler::Sync(handler) => {
            if let Err(err) = (handler)(param.clone(), method, rpcenv) {
                log::error!("Error checking subscription - {}", err);
            }
        }
        _ => unreachable!(),
    }
    let notify = match api2::node::subscription::get_subscription(param, rpcenv) {
        Ok(info) => info.status == SubscriptionStatus::Active,
        Err(err) => {
            log::error!("Error reading subscription - {}", err);
            false
        }
    };

    let param = json!({
        "notify": notify,
    });
    let method = &api2::node::apt::API_METHOD_APT_UPDATE_DATABASE;
    match method.handler {
        ApiHandler::Sync(handler) => match (handler)(param, method, rpcenv) {
            Err(err) => {
                log::error!("Error triggering apt database update - {}", err);
            }
            Ok(upid) => wait_for_local_worker(upid.as_str().unwrap()).await?,
        },
        _ => unreachable!(),
    };

    match check_acme_certificates(rpcenv).await {
        Ok(()) => (),
        Err(err) => {
            log::error!("error checking certificates: {}", err);
        }
    }

    // TODO: cleanup tasks like in PVE?

    Ok(())
}

async fn check_acme_certificates(rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let (config, _) = proxmox_backup::config::node::config()?;

    // do we even have any acme domains configures?
    if config.acme_domains().next().is_none() {
        return Ok(());
    }

    if !api2::node::certificates::cert_expires_soon()? {
        log::info!("Certificate does not expire within the next 30 days, not renewing.");
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

async fn run(rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let backup_user = pbs_config::backup_user()?;
    let file_opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);
    proxmox_rest_server::init_worker_tasks(
        pbs_buildcfg::PROXMOX_BACKUP_LOG_DIR_M!().into(),
        file_opts.clone(),
    )?;

    let mut command_sock = proxmox_rest_server::CommandSocket::new(
        proxmox_rest_server::our_ctrl_sock(),
        backup_user.gid,
    );
    proxmox_rest_server::register_task_control_commands(&mut command_sock)?;
    command_sock.spawn()?;

    do_update(rpcenv).await
}

fn main() {
    proxmox_backup::tools::setup_safe_path_env();

    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-daily-update"),
    ) {
        eprintln!("unable to inititialize syslog - {}", err);
    }

    let mut rpcenv = CliEnvironment::new();
    rpcenv.set_auth_id(Some(String::from("root@pam")));

    if let Err(err) = proxmox_async::runtime::main(run(&mut rpcenv)) {
        log::error!("error during update: {}", err);
        std::process::exit(1);
    }
}
