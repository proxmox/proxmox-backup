use failure::*;

//use crate::tools;
use crate::api_schema::*;
use crate::api_schema::router::*;
use serde_json::{json, Value};

use crate::server;

fn list_tasks(
    param: Value,
    _info: &ApiMethod,
    rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let start = param["start"].as_u64().unwrap_or(0);
    let limit = param["limit"].as_u64().unwrap_or(50);
    let errors = param["errors"].as_bool().unwrap_or(false);

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
            )
        );

    route
}
