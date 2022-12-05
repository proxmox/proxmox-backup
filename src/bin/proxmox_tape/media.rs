use anyhow::Error;
use serde::Deserialize;
use serde_json::Value;

use proxmox_router::{cli::*, ApiHandler, RpcEnvironment};
use proxmox_schema::api;

use pbs_api_types::{
    MediaContentListFilter, MediaListEntry, MediaStatus, CHANGER_NAME_SCHEMA,
    MEDIA_POOL_NAME_SCHEMA,
};
use pbs_config::drive::complete_changer_name;
use pbs_config::media_pool::complete_pool_name;

use proxmox_backup::{
    api2,
    tape::{complete_media_label_text, complete_media_set_uuid, complete_media_uuid},
};

pub fn media_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert(
            "list",
            CliCommand::new(&API_METHOD_LIST_MEDIA)
                .completion_cb("pool", complete_pool_name)
                .completion_cb("update-status-changer", complete_changer_name),
        )
        .insert(
            "destroy",
            CliCommand::new(&api2::tape::media::API_METHOD_DESTROY_MEDIA)
                .arg_param(&["label-text"])
                .completion_cb("label-text", complete_media_label_text),
        )
        .insert(
            "content",
            CliCommand::new(&API_METHOD_LIST_CONTENT)
                .completion_cb("pool", complete_pool_name)
                .completion_cb("label-text", complete_media_label_text)
                .completion_cb("media", complete_media_uuid)
                .completion_cb("media-set", complete_media_set_uuid),
        );

    cmd_def.into()
}

#[api(
    input: {
        properties: {
            pool: {
                schema: MEDIA_POOL_NAME_SCHEMA,
                optional: true,
            },
            "update-status": {
                description: "Try to update tape library status (check what tapes are online).",
                type: bool,
                optional: true,
                default: true,
            },
            "update-status-changer": {
                // only update status for a single changer
                schema: CHANGER_NAME_SCHEMA,
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
async fn list_media(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let info = &api2::tape::media::API_METHOD_LIST_MEDIA;
    let mut data = match info.handler {
        ApiHandler::Async(handler) => (handler)(param, info, rpcenv).await?,
        _ => unreachable!(),
    };

    fn render_status(_value: &Value, record: &Value) -> Result<String, Error> {
        let record = MediaListEntry::deserialize(record)?;
        Ok(match record.status {
            MediaStatus::Damaged | MediaStatus::Retired => serde_json::to_value(record.status)?
                .as_str()
                .unwrap()
                .to_string(),
            _ => {
                if record.expired {
                    String::from("expired")
                } else {
                    serde_json::to_value(record.status)?
                        .as_str()
                        .unwrap()
                        .to_string()
                }
            }
        })
    }

    fn catalog_status(value: &Value, _record: &Value) -> Result<String, Error> {
        let catalog_ok = value.as_bool().unwrap();
        if catalog_ok {
            Ok(String::from("ok"))
        } else {
            Ok(String::from("missing"))
        }
    }
    let options = default_table_format_options()
        .sortby("pool", false)
        .sortby("media-set-uuid", false)
        .sortby("seq-nr", false)
        .sortby("label-text", false)
        .column(ColumnConfig::new("label-text"))
        .column(ColumnConfig::new("pool"))
        .column(ColumnConfig::new("media-set-name"))
        .column(ColumnConfig::new("seq-nr"))
        .column(ColumnConfig::new("status").renderer(render_status))
        .column(ColumnConfig::new("location"))
        .column(ColumnConfig::new("catalog").renderer(catalog_status))
        .column(ColumnConfig::new("uuid"))
        .column(ColumnConfig::new("media-set-uuid"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}

#[api(
    input: {
        properties: {
            "filter": {
                type: MediaContentListFilter,
                flatten: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// List media content
fn list_content(param: Value, rpcenv: &mut dyn RpcEnvironment) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let info = &api2::tape::media::API_METHOD_LIST_CONTENT;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .sortby("media-set-uuid", false)
        .sortby("seq-nr", false)
        .sortby("store", false)
        .sortby("snapshot", false)
        .sortby("backup-time", false)
        .column(ColumnConfig::new("label-text"))
        .column(ColumnConfig::new("pool"))
        .column(ColumnConfig::new("media-set-name"))
        .column(ColumnConfig::new("seq-nr"))
        .column(ColumnConfig::new("store"))
        .column(ColumnConfig::new("snapshot"))
        .column(ColumnConfig::new("media-set-uuid"));

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())
}
