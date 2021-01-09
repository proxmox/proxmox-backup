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
            MediaStatus,
            MediaListEntry,
        },
        tape::media::MediaContentListFilter,
    },
    tape::{
        complete_media_changer_id,
        complete_media_uuid,
        complete_media_set_uuid,
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
        .insert(
            "destroy",
            CliCommand::new(&api2::tape::media::API_METHOD_DESTROY_MEDIA)
                .arg_param(&["changer-id"])
                .completion_cb("changer-id", complete_media_changer_id)
        )
        .insert(
            "content",
            CliCommand::new(&API_METHOD_LIST_CONTENT)
                .completion_cb("pool", complete_pool_name)
                .completion_cb("changer-id", complete_media_changer_id)
                .completion_cb("media", complete_media_uuid)
                .completion_cb("media-set", complete_media_set_uuid)
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
        .sortby("changer-id", false)
        .column(ColumnConfig::new("changer-id"))
        .column(ColumnConfig::new("pool"))
        .column(ColumnConfig::new("media-set-name"))
        .column(ColumnConfig::new("seq-nr"))
        .column(ColumnConfig::new("status").renderer(render_status))
        .column(ColumnConfig::new("location"))
        .column(ColumnConfig::new("catalog").renderer(catalog_status))
        .column(ColumnConfig::new("uuid"))
        .column(ColumnConfig::new("media-set-uuid"))
        ;

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
fn list_content(
    param: Value,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<(), Error> {

    let output_format = get_output_format(&param);
    let info = &api2::tape::media::API_METHOD_LIST_CONTENT;
    let mut data = match info.handler {
        ApiHandler::Sync(handler) => (handler)(param, info, rpcenv)?,
        _ => unreachable!(),
    };

    let options = default_table_format_options()
        .sortby("media-set-uuid", false)
        .sortby("seq-nr", false)
        .sortby("snapshot", false)
        .sortby("backup-time", false)
        .column(ColumnConfig::new("changer-id"))
        .column(ColumnConfig::new("pool"))
        .column(ColumnConfig::new("media-set-name"))
        .column(ColumnConfig::new("seq-nr"))
        .column(ColumnConfig::new("snapshot"))
        .column(ColumnConfig::new("media-set-uuid"))
        ;

    format_and_print_result_full(&mut data, &info.returns, &output_format, &options);

    Ok(())

}
