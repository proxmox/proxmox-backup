extern crate apitest;

use failure::*;

//use std::collections::HashMap;
use lazy_static::lazy_static;

//use apitest::json_schema::*;
use apitest::api_info::*;
use apitest::json_schema::*;

//use serde_derive::{Serialize, Deserialize};
use serde_json::{json, Value};

use tokio::prelude::*;
use tokio::timer::Delay;

//use hyper::body::Payload;
use hyper::http::request::Parts;
use hyper::{Method, Body, Request, Response, Server, StatusCode};
use hyper::rt::{Future, Stream};
use hyper::service::service_fn;

use futures::future::*;

use std::time::{Duration, Instant};

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
    let resp = req_body.concat2()
        .map_err(|err| format_err!("Promlems reading request body: {}", err))
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
                       .header("ContentType", "application/json")
                       .body(Body::from(json_str))?)
                }
                Err(err) => Ok(error_response!(BAD_REQUEST, err.to_string()))
            }
        });

    Box::new(resp)
}

fn handle_request(req: Request<Body>) -> BoxFut {

    let (parts, body) = req.into_parts();

    let method = parts.method.clone();
    let path = parts.uri.path();

    let components: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
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

            } else {
                http_error_future!(NOT_FOUND, "Path not found.");
            }
        }
    }

    Box::new(ok(Response::new(Body::from("RETURN WEB GUI\n"))))
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
