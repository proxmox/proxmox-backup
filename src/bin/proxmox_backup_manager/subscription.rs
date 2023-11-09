use anyhow::{bail, Error};
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;
use proxmox_subscription::{ProductType, SubscriptionInfo};

use proxmox_backup::api2::{self, node::subscription::subscription_file_opts};

use pbs_buildcfg::PROXMOX_BACKUP_SUBSCRIPTION_FN;

#[api(
    input: {
        properties: {
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    }
)]
/// Read subscription info.
fn get(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<Value, Error> {
    let output_format = get_output_format(&param);

    let info = &api2::node::subscription::API_METHOD_GET_SUBSCRIPTION;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options();
    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            "data": {
                type: String,
                description: "base64-encoded signed subscription info"
            },
        }
    }
)]
/// (Internal use only!) Set a signed subscription info blob as offline key
pub fn set_offline_subscription_key(data: String) -> Result<(), Error> {
    let mut info: SubscriptionInfo = serde_json::from_slice(&base64::decode(data)?)?;
    if !info.is_signed() {
        bail!("Offline subscription key must be signed!");
    }

    let product_type = info.get_product_type()?;
    if product_type != ProductType::Pbs {
        bail!("Subscription is not a PBS subscription ({product_type})!");
    }

    info.check_signature(&[proxmox_subscription::files::DEFAULT_SIGNING_KEY]);
    info.check_age(false);
    info.check_server_id();
    proxmox_subscription::files::write_subscription(
        PROXMOX_BACKUP_SUBSCRIPTION_FN,
        subscription_file_opts()?,
        &info,
    )?;
    Ok(())
}

pub fn subscription_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("get", CliCommand::new(&API_METHOD_GET))
        .insert(
            "set",
            CliCommand::new(&api2::node::subscription::API_METHOD_SET_SUBSCRIPTION)
                .fixed_param("node", "localhost".into())
                .arg_param(&["key"]),
        )
        .insert(
            "set-offline-key",
            CliCommand::new(&API_METHOD_SET_OFFLINE_SUBSCRIPTION_KEY).arg_param(&["data"]),
        )
        .insert(
            "update",
            CliCommand::new(&api2::node::subscription::API_METHOD_CHECK_SUBSCRIPTION)
                .fixed_param("node", "localhost".into()),
        )
        .insert(
            "remove",
            CliCommand::new(&api2::node::subscription::API_METHOD_DELETE_SUBSCRIPTION)
                .fixed_param("node", "localhost".into()),
        );

    cmd_def.into()
}
