use anyhow::{Error};
use serde_json::Value;

use proxmox::{
    api::{
        api,
        cli::*,
        RpcEnvironment,
        ApiHandler,
    },
};

use proxmox_backup::{
    api2::{
        self,
        types::{
            MEDIA_POOL_NAME_SCHEMA,
            MediaLocationKind,
            MediaStatus,
            MediaListEntry,
        },
    },
    config::{
        media_pool::complete_pool_name,
    },
};

pub fn media_commands() -> CommandLineInterface {

    let cmd_def = CliCommandMap::new()
        .insert(
            "list",
            CliCommand::new(&API_METHOD_LIST_MEDIA)
                .completion_cb("pool", complete_pool_name)
        )
        ;

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
                optional: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// List pool media
async fn list_media(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);
    let info = &api2::tape::media::API_METHOD_LIST_MEDIA;
    let mut data = match info.handler {
        ApiHandler::Async(handler) => (handler)(param, info, rpcenv).await?,
        _ => unreachable!(),
    };

    fn render_location(_value: &Value, record: &Value) -> Result<String, Error> {
        let record: MediaListEntry = serde_json::from_value(record.clone())?;
        Ok(match record.location {
            MediaLocationKind::Online =>  {
                record.location_hint.unwrap_or(String::from("-"))
            }
            MediaLocationKind::Offline => String::from("offline"),
            MediaLocationKind::Vault => {
                format!("V({})", record.location_hint.unwrap_or(String::from("-")))
            }
        })
    }

    fn render_status(_value: &Value, record: &Value) -> Result<String, Error> {
        let record: MediaListEntry = serde_json::from_value(record.clone())?;
        Ok(match record.status {
            MediaStatus::Damaged | MediaStatus::Retired => {
                serde_json::to_value(&record.status)?
                .as_str().unwrap()
                .to_string()
            }
            _ => {
                if record.expired {
                    String::from("expired")
                } else {
                    serde_json::to_value(&record.status)?
                    .as_str().unwrap()
                        .to_string()
                }
            }
        })
    }

    let options = default_table_format_options()
        .sortby("pool", false)
        .sortby("media-set-uuid", false)
        .sortby("seq-nr", false)
        .sortby("changer-id", false)
        .column(ColumnConfig::new("changer-id"))
        .column(ColumnConfig::new("pool"))
        .column(ColumnConfig::new("media-set-name"))
        .column(ColumnConfig::new("seq-nr"))
        .column(ColumnConfig::new("status").renderer(render_status))
        .column(ColumnConfig::new("location").renderer(render_location))
        .column(ColumnConfig::new("uuid"))
        .column(ColumnConfig::new("media-set-uuid"))
        ;

    format_and_print_result_full(&mut data, info.returns, &output_format, &options);

    Ok(())
}
