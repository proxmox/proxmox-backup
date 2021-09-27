//! Helpers to format response data

use anyhow::{Error};
use serde_json::{json, Value};

use hyper::{Body, Response, StatusCode};
use hyper::header;

use proxmox::api::{HttpError, RpcEnvironment};

/// Extension to set error message for server side logging
pub(crate) struct ErrorMessageExtension(pub String);

/// Methods to format data and errors
pub trait OutputFormatter: Send + Sync {
    /// Transform json data into a http response
    fn format_data(&self, data: Value, rpcenv: &dyn RpcEnvironment) -> Response<Body>;

    /// Transform errors into a http response
    fn format_error(&self, err: Error) -> Response<Body>;

    /// Transform a [Result] into a http response
    fn format_result(&self, result: Result<Value, Error>, rpcenv: &dyn RpcEnvironment) -> Response<Body> {
        match result {
            Ok(data) => self.format_data(data, rpcenv),
            Err(err) => self.format_error(err),
        }
    }
}

static JSON_CONTENT_TYPE: &str = "application/json;charset=UTF-8";

fn json_data_response(data: Value) -> Response<Body> {

    let json_str = data.to_string();

    let raw = json_str.into_bytes();

    let mut response = Response::new(raw.into());
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(JSON_CONTENT_TYPE));

    response
}

fn add_result_attributes(result: &mut Value, rpcenv: &dyn RpcEnvironment)
{
    let attributes = match rpcenv.result_attrib().as_object() {
        Some(attr) => attr,
        None => return,
    };

    for (key, value) in attributes {
        result[key] = value.clone();
    }
}


struct JsonFormatter();

/// Format data as ``application/json``
///
/// Errors generates a BAD_REQUEST containing the error
/// message as string.
pub static JSON_FORMATTER: &'static dyn OutputFormatter = &JsonFormatter();

impl  OutputFormatter for JsonFormatter {

    fn format_data(&self, data: Value, rpcenv: &dyn RpcEnvironment) -> Response<Body> {

        let mut result = json!({
            "data": data
        });

        add_result_attributes(&mut result, rpcenv);

        json_data_response(result)
    }

    fn format_error(&self, err: Error) -> Response<Body> {

        let mut response = if let Some(apierr) = err.downcast_ref::<HttpError>() {
            let mut resp = Response::new(Body::from(apierr.message.clone()));
            *resp.status_mut() = apierr.code;
            resp
        } else {
            let mut resp = Response::new(Body::from(err.to_string()));
            *resp.status_mut() = StatusCode::BAD_REQUEST;
            resp
        };

        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(JSON_CONTENT_TYPE));

        response.extensions_mut().insert(ErrorMessageExtension(err.to_string()));

        response
    }
}

/// Format data as ExtJS compatible ``application/json``
///
/// The returned json object contains the following properties:
///
/// * ``success``: boolean attribute indicating the success.
///
/// * ``data``: The result data (on success)
///
/// * ``message``: The error message (on failure)
///
/// * ``errors``: detailed list of errors (if available)
///
/// Any result attributes set on ``rpcenv`` are also added to the object.
///
/// Please note that errors return status code OK, but setting success
/// to false.
pub static EXTJS_FORMATTER: &'static dyn OutputFormatter = &ExtJsFormatter();

struct ExtJsFormatter();

impl  OutputFormatter for ExtJsFormatter {

    fn format_data(&self, data: Value, rpcenv: &dyn RpcEnvironment) -> Response<Body> {

        let mut result = json!({
            "data": data,
            "success": true
        });

        add_result_attributes(&mut result, rpcenv);

        json_data_response(result)
    }

    fn format_error(&self, err: Error) -> Response<Body> {

        let mut errors = vec![];

        let message = err.to_string();
        errors.push(&message);

        let result = json!({
            "message": message,
            "errors": errors,
            "success": false
        });

        let mut response = json_data_response(result);

        response.extensions_mut().insert(ErrorMessageExtension(message));

        response
    }
}
