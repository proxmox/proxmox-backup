use crate::api::schema::*;
use crate::api::router::*;
use crate::api::config::*;

use std::fmt;
use std::path::{PathBuf};
use std::sync::Arc;

use failure::*;
use serde_json::{json, Value};

use futures::future::{self, Either};
//use tokio::prelude::*;
//use tokio::timer::Delay;
use tokio::fs::File;
use tokio_codec;
//use bytes::{BytesMut, BufMut};

//use hyper::body::Payload;
use hyper::http::request::Parts;
use hyper::{Body, Request, Response, StatusCode};
use hyper::service::{Service, NewService};
use hyper::rt::{Future, Stream};
use hyper::header;

pub struct RestServer {
    pub api_config: Arc<ApiConfig>,
}

impl RestServer {

    pub fn new(api_config: ApiConfig) -> Self {
        Self { api_config: Arc::new(api_config) }
    }
}

impl NewService for RestServer
{
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type InitError = hyper::Error;
    type Service = ApiService;
    type Future = Box<Future<Item = Self::Service, Error = Self::InitError> + Send>;
    fn new_service(&self) -> Self::Future {
        Box::new(future::ok(ApiService { api_config: self.api_config.clone() }))
    }
}

pub struct ApiService {
    pub api_config: Arc<ApiConfig>,
}


impl Service for ApiService {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type Future = Box<Future<Item = Response<Body>, Error = Self::Error> + Send>;

    fn call(&mut self, req: Request<Self::ReqBody>) -> Self::Future {

        Box::new(handle_request(self.api_config.clone(), req).then(|result| {
            match result {
                Ok(res) => Ok::<_, hyper::Error>(res),
                Err(err) => {
                    if let Some(apierr) = err.downcast_ref::<HttpError>() {
                        let mut resp = Response::new(Body::from(apierr.message.clone()));
                        *resp.status_mut() = apierr.code;
                        Ok(resp)
                    } else {
                        let mut resp = Response::new(Body::from(err.to_string()));
                        *resp.status_mut() = StatusCode::BAD_REQUEST;
                        Ok(resp)
                    }
                }
            }
        }))
    }
}

type BoxFut = Box<Future<Item = Response<Body>, Error = failure::Error> + Send>;

#[derive(Debug, Fail)]
pub struct HttpError {
    pub code: StatusCode,
    pub message: String,
}

impl HttpError {
    pub fn new(code: StatusCode, message: String) -> Self {
        HttpError { code, message }
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Error {}: {}", self.code, self.message)
    }
}

macro_rules! http_err {
    ($status:ident, $msg:expr) => {{
        Error::from(HttpError::new(StatusCode::$status, $msg))
    }}
}

fn get_request_parameters_async(
    info: &'static ApiMethod,
    parts: Parts,
    req_body: Body,
) -> Box<Future<Item = Value, Error = failure::Error> + Send>
{
    let resp = req_body
        .map_err(|err| http_err!(BAD_REQUEST, format!("Promlems reading request body: {}", err)))
        .fold(Vec::new(), |mut acc, chunk| {
            if acc.len() + chunk.len() < 64*1024 { //fimxe: max request body size?
                acc.extend_from_slice(&*chunk);
                Ok(acc)
            }
            else { Err(http_err!(BAD_REQUEST, format!("Request body too large"))) }
        })
        .and_then(move |body| {

            let bytes = String::from_utf8(body.to_vec())?; // why copy??

            println!("GOT BODY {:?}", bytes);

            let mut test_required = true;

            let mut params = json!({});

            if bytes.len() > 0 {
                params = parse_query_string(&bytes, &info.parameters, true)?;
                test_required = false;
            }

            if let Some(query_str) = parts.uri.query() {
                let query_params = parse_query_string(query_str, &info.parameters, test_required)?;

                for (k, v) in query_params.as_object().unwrap() {
                    params[k] = v.clone(); // fixme: why clone()??
                }
            }

            println!("GOT PARAMS {}", params);
            Ok(params)
        });

    Box::new(resp)
}

fn handle_sync_api_request(
    info: &'static ApiMethod,
    parts: Parts,
    req_body: Body,
) -> BoxFut
{
    let params = get_request_parameters_async(info, parts, req_body);

    let resp = params
        .and_then(move |params| {

            println!("GOT PARAMS {}", params);

            /*
            let when = Instant::now() + Duration::from_millis(3000);
            let task = Delay::new(when).then(|_| {
                println!("A LAZY TASK");
                ok(())
            });

            tokio::spawn(task);
             */

            let res = (*info.handler)(params, info)?;

            Ok(res)

        }).then(|result| {
            match result {
                Ok(ref value) => {
                    let json_str = value.to_string();

                    Ok(Response::builder()
                       .status(StatusCode::OK)
                       .header(header::CONTENT_TYPE, "application/json")
                       .body(Body::from(json_str))?)
                }
                Err(err) => Err(http_err!(BAD_REQUEST, err.to_string()))
            }
        });

    Box::new(resp)
}

fn simple_static_file_download(filename: PathBuf) ->  BoxFut {

    Box::new(File::open(filename)
        .map_err(|err| http_err!(BAD_REQUEST, format!("File open failed: {}", err)))
        .and_then(|file| {
            let buf: Vec<u8> = Vec::new();
            tokio::io::read_to_end(file, buf)
                .map_err(|err| http_err!(BAD_REQUEST, format!("File read failed: {}", err)))
                .and_then(|data| Ok(Response::new(data.1.into())))
        }))
}

fn chuncked_static_file_download(filename: PathBuf) ->  BoxFut {

    Box::new(File::open(filename)
        .map_err(|err| http_err!(BAD_REQUEST, format!("File open failed: {}", err)))
        .and_then(|file| {
            let payload = tokio_codec::FramedRead::new(file, tokio_codec::BytesCodec::new()).
                map(|bytes| {
                    //sigh - howto avoid copy here? or the whole map() ??
                    hyper::Chunk::from(bytes.to_vec())
                });
            let body = Body::wrap_stream(payload);
            // fixme: set content type and other headers
            Ok(Response::builder()
               .status(StatusCode::OK)
               .body(body)
               .unwrap())
        }))
}

fn handle_static_file_download(filename: PathBuf) ->  BoxFut {

    let response = tokio::fs::metadata(filename.clone())
        .map_err(|err| http_err!(BAD_REQUEST, format!("File access problems: {}", err)))
        .and_then(|metadata| {
            if metadata.len() < 1024*32 {
                Either::A(simple_static_file_download(filename))
            } else {
                Either::B(chuncked_static_file_download(filename))
             }
        });

    return Box::new(response);
}

pub fn handle_request(api: Arc<ApiConfig>, req: Request<Body>) -> BoxFut {

    let (parts, body) = req.into_parts();

    let method = parts.method.clone();
    let path = parts.uri.path();

    // normalize path
    // do not allow ".", "..", or hidden files ".XXXX"
    // also remove empty path components

    let items = path.split('/');
    let mut path = String::new();
    let mut components = vec![];

    for name in items {
        if name.is_empty() { continue; }
        if name.starts_with(".") {
            return Box::new(future::err(http_err!(BAD_REQUEST, "Path contains illegal components.".to_string())));
        }
        path.push('/');
        path.push_str(name);
        components.push(name);
    }

    let comp_len = components.len();

    println!("REQUEST {} {}", method, path);
    println!("COMPO {:?}", components);

    if comp_len >= 1 && components[0] == "api3" {
        println!("GOT API REQUEST");
        if comp_len >= 2 {
            let format = components[1];
            if format != "json" {
                return Box::new(future::err(http_err!(BAD_REQUEST, format!("Unsupported output format '{}'.", format))))
            }

            if let Some(api_method) = api.find_method(&components[2..], method) {
                // fixme: handle auth
                return handle_sync_api_request(api_method, parts, body);
            }
        }
    } else {
        // not Auth for accessing files!

        let filename = api.find_alias(&components);
        return handle_static_file_download(filename);
    }

    Box::new(future::err(http_err!(NOT_FOUND, "Path not found.".to_string())))
}

