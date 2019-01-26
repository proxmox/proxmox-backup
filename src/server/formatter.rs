use failure::*;
use serde_json::{json, Value};

use crate::api::router::RpcEnvironment;

use hyper::{Body, Response, StatusCode};
use hyper::header;

pub struct OutputFormatter {

    pub format_result: fn(data: Value, rpcenv: &RpcEnvironment) -> Response<Body>,

    pub format_error: fn(err: Error) -> Response<Body>,
}

static json_content_type: &str = "application/json;charset=UTF-8";


fn json_response(result: Value) -> Response<Body> {

    let json_str = result.to_string();

    let raw = json_str.into_bytes();

    let mut response = Response::new(raw.into());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(json_content_type));

    response
}

fn json_format_result(data: Value, rpcenv: &RpcEnvironment) -> Response<Body> {

    let mut result = json!({
        "data": data
    });

    if let Some(total) = rpcenv.get_result_attrib("total").and_then(|v| v.as_u64()) {
        result["total"] = Value::from(total);
    }

    if let Some(changes) = rpcenv.get_result_attrib("changes") {
        result["changes"] = changes.clone();
    }

    json_response(result)
}

fn json_format_error(err: Error) -> Response<Body> {

    let mut response = Response::new(Body::from(err.to_string()));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(json_content_type));
    *response.status_mut() = StatusCode::BAD_REQUEST;

    response
}

pub static JSON_FORMATTER: OutputFormatter = OutputFormatter {
    format_result: json_format_result,
    format_error: json_format_error,
};

fn extjs_format_result(data: Value, rpcenv: &RpcEnvironment) -> Response<Body> {

    let mut result = json!({
        "data": data,
        "success": true
    });

    if let Some(total) = rpcenv.get_result_attrib("total").and_then(|v| v.as_u64()) {
        result["total"] = Value::from(total);
    }

    if let Some(changes) = rpcenv.get_result_attrib("changes") {
        result["changes"] = changes.clone();
    }


    json_response(result)
}

fn extjs_format_error(err: Error) -> Response<Body> {

    let mut errors = vec![];
    errors.push(err.to_string());

    let result = json!({
        "errors": errors,
        "success": false
    });

    json_response(result)
}

pub static EXTJS_FORMATTER: OutputFormatter = OutputFormatter {
    format_result: extjs_format_result,
    format_error: extjs_format_error,
};
