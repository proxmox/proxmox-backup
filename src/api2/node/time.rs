use failure::*;

use crate::tools;
use crate::api::schema::*;
use crate::api::router::*;
use serde_json::{json, Value};

use chrono::prelude::*;

fn read_etc_localtime() -> Result<String, Error> {

    let line = tools::file_read_firstline("/etc/timezone")?;

    Ok(line.trim().to_owned())
}

fn get_time(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
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

extern "C"  { fn tzset(); }

// Note:: this needs root rights ??

fn set_timezone(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let timezone = tools::required_string_param(&param, "timezone")?;

    let path = std::path::PathBuf::from(format!("/usr/share/zoneinfo/{}", timezone));

    if !path.exists() {
        bail!("No such timezone.");
    }

    tools::file_set_contents("/etc/timezone", timezone.as_bytes(), None)?;

    let _ = std::fs::remove_file("/etc/localtime");

    use std::os::unix::fs::symlink;
    symlink(path, "/etc/localtime")?;

    unsafe { tzset() };

    Ok(Value::Null)
}

pub fn router() -> Router {

    let route = Router::new()
        .get(
            ApiMethod::new(
                get_time,
                ObjectSchema::new("Read server time and time zone settings.")
            ).returns(
                ObjectSchema::new("Returns server time and timezone.")
                    .required("timezone", StringSchema::new("Time zone"))
                    .required("time", IntegerSchema::new("Seconds since 1970-01-01 00:00:00 UTC.")
                              .minimum(1297163644))
                    .required("localtime", IntegerSchema::new("Seconds since 1970-01-01 00:00:00 UTC. (local time)")
                              .minimum(1297163644))
            )
        )
        .put(
            ApiMethod::new(
                set_timezone,
                ObjectSchema::new("Set time zone.")
                    .required("timezone", StringSchema::new("Time zone. The file '/usr/share/zoneinfo/zone.tab' contains the list of valid names."))
            ).protected(true)
        );


    route
}
