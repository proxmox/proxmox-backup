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

fn get_time(_param: Value, _info: &ApiMethod) -> Result<Value, Error> {

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

pub fn router() -> Router {

    let route = Router::new()
        .get(ApiMethod::new(
            get_time,
            ObjectSchema::new("Read server time and time zone settings.")));

    route
}
