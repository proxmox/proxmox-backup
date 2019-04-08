use failure::*;

use crate::tools;
use crate::api_schema::*;
use crate::api_schema::router::*;
use serde_json::{json, Value};
use std::sync::Arc;
use std::fs::File;
use std::io::{BufRead,BufReader};

use crate::server::{self, UPID};

fn get_task_status(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let upid = extract_upid(&param)?;

    let result = if crate::server::worker_is_active(&upid) {
        json!({
            "status": "running",
        })
    } else {
         json!({
            "status": "running",
        })
    };

    Ok(result)
}

fn extract_upid(param: &Value) -> Result<UPID, Error> {

    let upid_str = tools::required_string_param(&param, "upid")?;

    let upid = match upid_str.parse::<UPID>() {
        Ok(v) => v,
        Err(err) => bail!("unable to parse UPID '{}' - {}", upid_str, err),
    };

    Ok(upid)
}

fn read_task_log(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let upid = extract_upid(&param)?;
    let start = param["start"].as_u64().unwrap_or(0);
    let mut limit = param["limit"].as_u64().unwrap_or(50);
    let mut count: u64 = 0;

    let path = upid.log_path();

    let file = File::open(path)?;

    let mut lines: Vec<Value> = vec![];

    for line in BufReader::new(file).lines() {
        match line {
            Ok(line) => {
                count += 1;
                if count < start { continue };
	        if limit <= 0 { continue };

                lines.push(json!({ "n": count, "t": line }));

                limit -= 1;
            }
            Err(err) => {
                log::error!("reading task log failed: {}", err);
                break;
            }
        }
    }

    Ok(json!(lines))
}

fn list_tasks(
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let start = param["start"].as_u64().unwrap_or(0);
    let limit = param["limit"].as_u64().unwrap_or(50);
    let errors = param["errors"].as_bool().unwrap_or(false);

    let userfilter = param["userfilter"].as_str();

    let list = server::read_task_list()?;

    let mut result = vec![];

    let mut count = 0;

    for info in list.iter() {
        let mut entry = json!({
            "upid": info.upid_str,
            "node": "localhost",
            "pid": info.upid.pid,
            "pstart": info.upid.pstart,
            "starttime": info.upid.starttime,
            "type": info.upid.worker_type,
            "id": info.upid.worker_id,
            "user": info.upid.username,
        });

        if let Some(username) = userfilter {
            if !info.upid.username.contains(username) { continue; }
        }

        if let Some(ref state) = info.state {
            if errors && state.1 == "OK" {
                continue;
            }

            entry["endtime"] = Value::from(state.0);
            entry["status"] = Value::from(state.1.clone());
        }

        if (count as u64) < start {
            count += 1;
            continue;
        } else {
            count += 1;
        }

        if (result.len() as u64) < limit { result.push(entry); };
    }

    rpcenv.set_result_attrib("total", Value::from(count));

    Ok(json!(result))
}

pub fn router() -> Router {

    let upid_schema : Arc<Schema> = Arc::new(
        StringSchema::new("Unique Process/Task ID.")
            .max_length(256)
            .into()
    );

    let upid_api = Router::new()
        .get(ApiMethod::new(
            |_,_,_| {
                let mut result = vec![];
                for cmd in &["log", "status"] {
                    result.push(json!({"subdir": cmd }));
                }
                Ok(Value::from(result))
            },
            ObjectSchema::new("Directory index.")
                .required("upid", upid_schema.clone()))
        )
        .subdir(
            "log", Router::new()
                .get(
                    ApiMethod::new(
                        read_task_log,
                        ObjectSchema::new("Read task log.")
                            .required("upid", upid_schema.clone())
                            .optional(
                                "start",
                                IntegerSchema::new("Start at this line.")
                                    .minimum(0)
                                    .default(0)
                            )
                            .optional(
                                "limit",
                                IntegerSchema::new("Only list this amount of lines.")
                                    .minimum(0)
                                    .default(50)
                            )
                    )
                )
        )
        .subdir(
            "status", Router::new()
                .get(
                    ApiMethod::new(
                        get_task_status,
                        ObjectSchema::new("Get task status.")
                            .required("upid", upid_schema.clone()))
                )
        );


    let route = Router::new()
        .get(ApiMethod::new(
            list_tasks,
            ObjectSchema::new("List tasks.")
                .optional(
                    "start",
                    IntegerSchema::new("List tasks beginning from this offset.")
                        .minimum(0)
                        .default(0)
                )
                .optional(
                    "limit",
                    IntegerSchema::new("Only list this amount of tasks.")
                        .minimum(0)
                        .default(50)
                )
                .optional(
                    "errors",
                    BooleanSchema::new("Only list erroneous tasks.")
                )
                .optional(
                    "userfilter",
                    StringSchema::new("Only list tasks from this user.")
                )
           )
        )
        .match_all("upid", upid_api);

    route
}
