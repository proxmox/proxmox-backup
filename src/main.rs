extern crate apitest;

use failure::*;
use std::collections::HashMap;

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

fn handle_static_file_download(filename: PathBuf) ->  BoxFut {

    let response = File::open(filename)
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
        })
        .or_else(|err| {
            // fixme: set content type and other headers
            Ok(Response::builder()
               .status(StatusCode::NOT_FOUND)
               .body(format!("File access problems: {}\n", err).into())
               .unwrap())
        });

    return Box::new(response);
}

fn handle_request(req: Request<Body>) -> BoxFut {

    let (parts, body) = req.into_parts();

    let method = parts.method.clone();
    let path = parts.uri.path();

    // normalize path
    let components: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
    let comp_len = components.len();
    let path = components.iter().fold(String::new(), |mut acc, chunk| {
        acc.push('/');
        acc.push_str(chunk);
        acc
    });

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
    }

    // not Auth for accessing files!
    if let Some(filename) = CACHED_DIRS.get(&path) {

        println!("SERVER STATIC FILE {:?}", path);
        return handle_static_file_download(filename.clone());
    }

    http_error_future!(NOT_FOUND, "Path not found.")
    //Box::new(ok(Response::new(Body::from("RETURN WEB GUI\n"))))
}

// add default dirs which includes jquery and bootstrap
// my $base = '/usr/share/libpve-http-server-perl';
// add_dirs($self->{dirs}, '/css/' => "$base/css/");
// add_dirs($self->{dirs}, '/js/' => "$base/js/");
// add_dirs($self->{dirs}, '/fonts/' => "$base/fonts/");

//use std::io;
use std::fs;
use std::path::{Path, PathBuf};

fn add_dirs(cache: &mut HashMap<String, PathBuf>, alias: String, path: &Path) -> Result<(), Error> {

    if path.is_dir() {
        for direntry in fs::read_dir(path)? {
            let entry = direntry?;
            let entry_path = entry.path();
            let file_type = entry.file_type()?;
            if let Some(file_name) = entry_path.file_name() {
                let newalias = alias.clone() + &String::from(file_name.to_string_lossy()); // fixme
                if file_type.is_dir() {
                    add_dirs(cache, newalias, entry_path.as_path())?;
                } else if file_type.is_file() {
                    cache.insert(newalias, entry_path);
                }
            }
        }
    }
    Ok(())
 }

fn initialize_directory_cache() -> HashMap<String, PathBuf> {

    let mut basedirs = HashMap::new();

    basedirs.insert("novnc", Path::new("/usr/share/novnc-pve"));
    basedirs.insert("extjs", Path::new("/usr/share/javascript/extjs"));
    basedirs.insert("fontawesome", Path::new("/usr/share/fonts-font-awesome"));
    basedirs.insert("xtermjs", Path::new("/usr/share/pve-xtermjs"));
    basedirs.insert("widgettoolkit", Path::new("/usr/share/javascript/proxmox-widget-toolkit"));

    let mut cache = HashMap::new();

    if let Err(err) = add_dirs(&mut cache, "/pve2/ext6/".into(), basedirs["extjs"]) {
        eprintln!("directory cache init error: {}", err);
    }

    cache
}

lazy_static!{
    static ref CACHED_DIRS: HashMap<String, PathBuf> = initialize_directory_cache();
}

lazy_static!{
    static ref ROUTER: MethodInfo = apitest::api3::router();
}

fn main() {
    println!("Fast Static Type Definitions 1");

    let count = CACHED_DIRS.iter().count();
    println!("Dircache contains {} entries.", count);

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
