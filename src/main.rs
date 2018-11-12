extern crate apitest;

use failure::*;
use std::collections::HashMap;
//use std::io;
use std::fs;
use std::path::{Path,PathBuf};

//use std::collections::HashMap;
use lazy_static::lazy_static;

//use apitest::json_schema::*;
use apitest::api_info::*;
use apitest::json_schema::*;

//use serde_derive::{Serialize, Deserialize};
use serde_json::{json, Value};

use futures::future::*;
//use tokio::prelude::*;
//use tokio::timer::Delay;
use tokio::fs::File;
use tokio_codec;
//use bytes::{BytesMut, BufMut};

//use hyper::body::Payload;
use hyper::http::request::Parts;
use hyper::{Method, Body, Request, Response, Server, StatusCode};
use hyper::rt::{Future, Stream};
use hyper::service::service_fn;
use hyper::header;

//use std::time::{Duration, Instant};

type BoxFut = Box<Future<Item = Response<Body>, Error = failure::Error> + Send>;

macro_rules! error_response {
    ($status:ident, $msg:expr) => {{
        let mut resp = Response::new(Body::from($msg));
        *resp.status_mut() = StatusCode::$status;
        resp
    }}
}

macro_rules! http_error_future {
    ($status:ident, $msg:expr) => {{
        let resp = error_response!($status, $msg);
        return Box::new(ok(resp));
    }}
}

fn get_request_parameters_async<'a>(
    info: &'a ApiMethod,
    parts: Parts,
    req_body: Body,
) -> Box<Future<Item = Value, Error = failure::Error> + Send + 'a>
{
    let resp = req_body
        .map_err(|err| format_err!("Promlems reading request body: {}", err))
        .fold(Vec::new(), |mut acc, chunk| {
            if acc.len() + chunk.len() < 64*1024 { //fimxe: max request body size?
                acc.extend_from_slice(&*chunk);
                ok(acc)
            } else {
                err(format_err!("Request body too large"))
            }
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

fn handle_async_api_request<'a>(
    info: &'a ApiMethod,
    parts: Parts,
    req_body: Body,
) -> Box<Future<Item = Response<Body>, Error = failure::Error> + Send + 'a>
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

            (info.async_handler)(params, info)
        });

    Box::new(resp)
}

fn handle_sync_api_request<'a>(
    info: &'a ApiMethod,
    parts: Parts,
    req_body: Body,
) -> Box<Future<Item = Response<Body>, Error = failure::Error> + Send + 'a>
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

            let res = (info.handler)(params, info)?;

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
                Err(err) => Ok(error_response!(BAD_REQUEST, err.to_string()))
            }
        });

    Box::new(resp)
}

fn simple_static_file_download(filename: PathBuf) ->  BoxFut {

    Box::new(File::open(filename)
        .map_err(|err| format_err!("File open failed: {}", err))
        .and_then(|file| {
            let buf: Vec<u8> = Vec::new();
            tokio::io::read_to_end(file, buf)
                .map_err(|err| format_err!("File read failed: {}", err))
                .and_then(|data| Ok(Response::new(data.1.into())))
        }))
}

fn chuncked_static_file_download(filename: PathBuf) ->  BoxFut {

    Box::new(File::open(filename)
        .map_err(|err| format_err!("File open failed: {}", err))
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
        .map_err(|err| format_err!("File access problems: {}", err))
        .and_then(|metadata| {
            if metadata.len() < 1024*32 {
                Either::A(simple_static_file_download(filename))
            } else {
                Either::B(chuncked_static_file_download(filename))
             }
        });

    return Box::new(response);
}

fn handle_request(req: Request<Body>) -> BoxFut {

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
            http_error_future!(BAD_REQUEST, "Path contains illegal components.\n");
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
                http_error_future!(BAD_REQUEST, format!("Unsupported output format '{}'.", format))
            }

            if let Some(info) = ROUTER.find_method(&components[2..]) {
                println!("FOUND INFO");
                let api_method_opt = match method {
                    Method::GET => &info.get,
                    Method::PUT => &info.put,
                    Method::POST => &info.post,
                    Method::DELETE => &info.delete,
                    _ => &None,
                };
                let api_method = match api_method_opt {
                    Some(m) => m,
                    _ => http_error_future!(NOT_FOUND, format!("No such method '{}'.", method)),
                };

                // fixme: handle auth

                //return handle_sync_api_request(api_method, parts, body);
                return handle_async_api_request(api_method, parts, body);
            }
        }
    } else {
        // not Auth for accessing files!

        let mut prefix = String::new();
        let mut filename = PathBuf::from("/var/www"); // fixme
        if comp_len >= 1 {
            prefix.push_str(components[0]);
            if let Some(subdir) = DIR_ALIASES.get(&prefix) {
                filename.push(subdir);
                for i in 1..comp_len { filename.push(components[i]) }
            }
        }
        return handle_static_file_download(filename);
    }


    http_error_future!(NOT_FOUND, "Path not found.")
    //Box::new(ok(Response::new(Body::from("RETURN WEB GUI\n"))))
}

// add default dirs which includes jquery and bootstrap
// my $base = '/usr/share/libpve-http-server-perl';
// add_dirs($self->{dirs}, '/css/' => "$base/css/");
// add_dirs($self->{dirs}, '/js/' => "$base/js/");
// add_dirs($self->{dirs}, '/fonts/' => "$base/fonts/");


fn initialize_directory_aliases() -> HashMap<String, PathBuf> {

    let mut basedirs:  HashMap<String, PathBuf> = HashMap::new();

    let mut add_directory_alias = |name, path| {
        basedirs.insert(String::from(name), PathBuf::from(path));
    };

    add_directory_alias("novnc", "/usr/share/novnc-pve");
    add_directory_alias("extjs", "/usr/share/javascript/extjs");
    add_directory_alias("fontawesome", "/usr/share/fonts-font-awesome");
    add_directory_alias("xtermjs", "/usr/share/pve-xtermjs");
    add_directory_alias("widgettoolkit", "/usr/share/javascript/proxmox-widget-toolkit");

    basedirs
}

lazy_static!{
    static ref DIR_ALIASES: HashMap<String, PathBuf> = initialize_directory_aliases();
}

lazy_static!{
    static ref ROUTER: MethodInfo = apitest::api3::router();
}

fn main() {
    println!("Fast Static Type Definitions 1");

    let addr = ([127, 0, 0, 1], 8007).into();

    let new_svc = || {
        service_fn(|req| {
            // clumsy way to convert failure::Error to Response
            handle_request(req).then(|result| -> Result<Response<Body>, String> {
                match result {
                    Ok(res) => Ok(res),
                    Err(err) => Ok(error_response!(BAD_REQUEST, err.to_string())),
                }
            })
        })
    };

    let server = Server::bind(&addr)
        .serve(new_svc)
        .map_err(|e| eprintln!("server error: {}", e));

    // Run this server for... forever!
    hyper::rt::run(server);
}
