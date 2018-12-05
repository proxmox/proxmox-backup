use failure::*;
use serde_json::{json, Value};

use hyper::{Body, Response, StatusCode};
use hyper::header;

pub struct OutputFormatter {

    pub format_result: fn(data: Result<Value, Error>) -> Response<Body>,
}

fn json_format_result(data: Result<Value, Error>) -> Response<Body> {

    let content_type = "application/json;charset=UTF-8";

    match data {
        Ok(value) => {
            let result = json!({
                "data": value
            });

            // todo: set result.total result.changes

            let json_str = result.to_string();

            let raw = json_str.into_bytes();

            let mut response = Response::new(raw.into());
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static(content_type));
            return response;
        }
        Err(err) => {
            let mut response = Response::new(Body::from(err.to_string()));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                header::HeaderValue::from_static(content_type));
            *response.status_mut() = StatusCode::BAD_REQUEST;
            return response;
        }
    }
}

pub static JSON_FORMATTER: OutputFormatter = OutputFormatter {
    format_result: json_format_result,
};

fn extjs_format_result(data: Result<Value, Error>) -> Response<Body> {

    let content_type = "application/json;charset=UTF-8";

    let result = match data {
        Ok(value) => {
            let result = json!({
                "data": value,
                "success": true
            });

            // todo: set result.total result.changes

            result
        }
        Err(err) => {
            let mut errors = vec![];

            errors.push(err.to_string());

            let result = json!({
                "errors": errors,
                "success": false
            });

            result
        }
    };

    let json_str = result.to_string();

    let raw = json_str.into_bytes();

    let mut response = Response::new(raw.into());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(content_type));
    response
}

pub static EXTJS_FORMATTER: OutputFormatter = OutputFormatter {
    format_result: extjs_format_result,
};
