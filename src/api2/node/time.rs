use std::mem::{self, MaybeUninit};

use chrono::prelude::*;
use failure::*;
use serde_json::{json, Value};

use proxmox::{sortable, identity};
use proxmox::api::{ApiHandler, ApiMethod, Router, RpcEnvironment};
use proxmox::api::schema::*;
use proxmox::tools::fs::{file_read_firstline, replace_file, CreateOptions};

use crate::api2::types::*;

fn read_etc_localtime() -> Result<String, Error> {
    // use /etc/timezone
    if let Ok(line) = file_read_firstline("/etc/timezone") {
        return Ok(line.trim().to_owned());
    }

    // otherwise guess from the /etc/localtime symlink
    let mut buf = MaybeUninit::<[u8; 64]>::uninit();
    let len = unsafe {
        libc::readlink(
            "/etc/localtime".as_ptr() as *const _,
            buf.as_mut_ptr() as *mut _,
            mem::size_of_val(&buf),
        )
    };
    if len <= 0 {
        bail!("failed to guess timezone");
    }
    let len = len as usize;
    let buf = unsafe {
        (*buf.as_mut_ptr())[len] = 0;
        buf.assume_init()
    };
    let link = std::str::from_utf8(&buf[..len])?;
    match link.rfind("/zoneinfo/") {
        Some(pos) => Ok(link[(pos + 10)..].to_string()),
        None => Ok(link.to_string()),
    }
}

fn get_time(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let datetime = Local::now();
    let offset = datetime.offset();
    let time = datetime.timestamp();
    let localtime = time + (offset.fix().local_minus_utc() as i64);

    Ok(json!({
        "timezone": read_etc_localtime()?,
        "time": time,
        "localtime": localtime,
    }))
}

fn set_timezone(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    let timezone = crate::tools::required_string_param(&param, "timezone")?;

    let path = std::path::PathBuf::from(format!("/usr/share/zoneinfo/{}", timezone));

    if !path.exists() {
        bail!("No such timezone.");
    }

    replace_file("/etc/timezone", timezone.as_bytes(), CreateOptions::new())?;

    let _ = std::fs::remove_file("/etc/localtime");

    use std::os::unix::fs::symlink;
    symlink(path, "/etc/localtime")?;

    Ok(Value::Null)
}

#[sortable]
pub const ROUTER: Router = Router::new()
    .get(
        &ApiMethod::new(
            &ApiHandler::Sync(&get_time),
            &ObjectSchema::new(
                "Read server time and time zone settings.",
                &sorted!([ ("node", false, &NODE_SCHEMA) ]),
            )
        ).returns(
            &ObjectSchema::new(
                "Returns server time and timezone.",
                &sorted!([
                    ("timezone", false, &StringSchema::new("Time zone").schema()),
                    ("time", false, &IntegerSchema::new("Seconds since 1970-01-01 00:00:00 UTC.")
                     .minimum(1_297_163_644)
                     .schema()
                    ),
                    ("localtime", false, &IntegerSchema::new("Seconds since 1970-01-01 00:00:00 UTC. (local time)")
                     .minimum(1_297_163_644)
                     .schema()
                    ),
                ]),
            ).schema()
        )
    )
    .put(
        &ApiMethod::new(
            &ApiHandler::Sync(&set_timezone),
            &ObjectSchema::new(
                "Set time zone.",
                &sorted!([
                    ("node", false, &NODE_SCHEMA),
                    ("timezone", false, &StringSchema::new(
                        "Time zone. The file '/usr/share/zoneinfo/zone.tab' contains the list of valid names.")
                     .schema()
                    ),
                ]),
            )
        ).protected(true).reload_timezone(true)
    );

