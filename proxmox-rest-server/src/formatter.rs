//! Helpers to format response data
use std::collections::HashMap;

use anyhow::Error;
use serde_json::{json, Value};

use hyper::header;
use hyper::{Body, Response, StatusCode};

use proxmox_router::{HttpError, RpcEnvironment, SerializableReturn};
use proxmox_schema::ParameterError;

/// Extension to set error message for server side logging
pub(crate) struct ErrorMessageExtension(pub String);

/// Methods to format data and errors
pub trait OutputFormatter: Send + Sync {
    /// Transform json data into a http response
    fn format_data(&self, data: Value, rpcenv: &dyn RpcEnvironment) -> Response<Body>;

    /// Transform serializable data into a streaming http response
    fn format_data_streaming(
        &self,
        data: Box<dyn SerializableReturn + Send>,
        rpcenv: &dyn RpcEnvironment,
    ) -> Result<Response<Body>, Error>;

    /// Transform errors into a http response
    fn format_error(&self, err: Error) -> Response<Body>;

    /// Transform a [Result] into a http response
    fn format_result(
        &self,
        result: Result<Value, Error>,
        rpcenv: &dyn RpcEnvironment,
    ) -> Response<Body> {
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
        header::HeaderValue::from_static(JSON_CONTENT_TYPE),
    );

    response
}

fn json_data_response_streaming(body: Body) -> Result<Response<Body>, Error> {
    let response = Response::builder()
        .header(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(JSON_CONTENT_TYPE),
        )
        .body(body)?;
    Ok(response)
}

fn add_result_attributes(result: &mut Value, rpcenv: &dyn RpcEnvironment) {
    let attributes = match rpcenv.result_attrib().as_object() {
        Some(attr) => attr,
        None => return,
    };

    for (key, value) in attributes {
        result[key] = value.clone();
    }
}

fn start_data_streaming(
    value: Value,
    data: Box<dyn SerializableReturn + Send>,
) -> tokio::sync::mpsc::Receiver<Result<Vec<u8>, Error>> {
    let (writer, reader) = tokio::sync::mpsc::channel(1);

    tokio::task::spawn_blocking(move || {
        let output = proxmox_async::blocking::SenderWriter::from_sender(writer);
        let mut output = std::io::BufWriter::new(output);
        let mut serializer = serde_json::Serializer::new(&mut output);
        let _ = data.sender_serialize(&mut serializer, value);
    });

    reader
}

struct JsonFormatter();

/// Format data as ``application/json``
///
/// The returned json object contains the following properties:
///
/// * ``data``: The result data (on success)
///
/// Any result attributes set on ``rpcenv`` are also added to the object.
///
/// Errors generates a BAD_REQUEST containing the error
/// message as string.
pub static JSON_FORMATTER: &'static dyn OutputFormatter = &JsonFormatter();

impl OutputFormatter for JsonFormatter {
    fn format_data(&self, data: Value, rpcenv: &dyn RpcEnvironment) -> Response<Body> {
        let mut result = json!({ "data": data });

        add_result_attributes(&mut result, rpcenv);

        json_data_response(result)
    }

    fn format_data_streaming(
        &self,
        data: Box<dyn SerializableReturn + Send>,
        rpcenv: &dyn RpcEnvironment,
    ) -> Result<Response<Body>, Error> {
        let mut value = json!({});

        add_result_attributes(&mut value, rpcenv);

        let reader = start_data_streaming(value, data);
        let stream = tokio_stream::wrappers::ReceiverStream::new(reader);

        json_data_response_streaming(Body::wrap_stream(stream))
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
            header::HeaderValue::from_static(JSON_CONTENT_TYPE),
        );

        response
            .extensions_mut()
            .insert(ErrorMessageExtension(err.to_string()));

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

impl OutputFormatter for ExtJsFormatter {
    fn format_data(&self, data: Value, rpcenv: &dyn RpcEnvironment) -> Response<Body> {
        let mut result = json!({
            "data": data,
            "success": true
        });

        add_result_attributes(&mut result, rpcenv);

        json_data_response(result)
    }

    fn format_data_streaming(
        &self,
        data: Box<dyn SerializableReturn + Send>,
        rpcenv: &dyn RpcEnvironment,
    ) -> Result<Response<Body>, Error> {
        let mut value = json!({
            "success": true,
        });

        add_result_attributes(&mut value, rpcenv);

        let reader = start_data_streaming(value, data);
        let stream = tokio_stream::wrappers::ReceiverStream::new(reader);

        json_data_response_streaming(Body::wrap_stream(stream))
    }

    fn format_error(&self, err: Error) -> Response<Body> {
        let mut errors = HashMap::new();

        let message: String = match err.downcast::<ParameterError>() {
            Ok(param_err) => {
                for (name, err) in param_err {
                    errors.insert(name, err.to_string());
                }
                String::from("parameter verification errors")
            }
            Err(err) => err.to_string(),
        };

        let result = json!({
            "message": message,
            "errors": errors,
            "success": false
        });

        let mut response = json_data_response(result);

        response
            .extensions_mut()
            .insert(ErrorMessageExtension(message));

        response
    }
}
