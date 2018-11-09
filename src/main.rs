extern crate apitest;

use failure::*;

use std::collections::HashMap;
use lazy_static::lazy_static;

//use apitest::json_schema::*;
use apitest::api_info::*;
use apitest::json_schema::*;

//use serde_derive::{Serialize, Deserialize};
use serde_json::{json, Value};


use hyper::body::Payload;
use hyper::http::request::Parts;
use hyper::{Method, Body, Request, Response, Server, StatusCode};
use hyper::rt::{Future, Stream};
use hyper::service::service_fn;

use futures::prelude::*;
use futures::future;

use http;
use std::io;

type BoxFut = Box<Future<Item = Response<Body>, Error = hyper::Error> + Send>;

macro_rules! http_error {
    ($status:ident, $msg:expr) => {{
        let mut resp = Response::new(Body::from($msg));
        *resp.status_mut() = StatusCode::$status;
        return resp
    }}
}
macro_rules! http_error_future {
    ($status:ident, $msg:expr) => {{
        let mut resp = Response::new(Body::from($msg));
        *resp.status_mut() = StatusCode::$status;
        return Box::new(futures::future::ok(resp));
    }}
}

fn handle_api_request<'a>(
    info: &'a ApiMethod, parts: Parts, req_body: Body, query: Option<String>)
    -> Box<Future<Item = Response<Body>, Error = hyper::Error> + Send + 'a>
{

    let entire_body = req_body.concat2();

    let resp = entire_body.map(move |body| {
        let bytes = match String::from_utf8(body.to_vec()) { // why copy??
            Ok(v) => v,
            Err(err) => http_error!(NOT_FOUND, err.to_string()),
        };

        println!("GOT BODY {:?}", parts);

        match parse_query_string(&bytes, &info.parameters, true) {
            Ok(res) => {
                let json_str = res.to_string();
                return Response::new(Body::from(json_str));
            }
            Err(err) => {
                http_error!(NOT_FOUND, format!("Method returned error '{:?}'\n", err));
            }
        }

    });

    Box::new(resp)
}

fn handle_request(req: Request<Body>) -> BoxFut {

    let (parts, body) = req.into_parts();

    let method = parts.method.clone();
    let path = parts.uri.path();
    let query = parts.uri.query().map(|x| x.to_owned());

    let components: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
    let comp_len = components.len();

    println!("REQUEST {} {}", method, path);
    println!("COMPO {:?}", components);

    if comp_len >= 1 && components[0] == "api3" {
        println!("GOT API REQUEST");
        if comp_len >= 2 {
            let format = components[1];
            if format != "json" {
                http_error_future!(NOT_FOUND, format!("Unsupported format '{}'\n", format))
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
                    _ => http_error_future!(NOT_FOUND, format!("No such method '{} {}'\n", method, path)),
                };

                // fixme: handle auth


                let res = handle_api_request(api_method, parts, body, query);
                return res;

                /*
                // extract param
                let param = match query {
                    Some(data) => {
                        match parse_query_string(data, &api_method.parameters, true) {
                            Ok(query) => query,
                            Err(ref error_list) => {
                                let msg = error_list.iter().fold(String::from(""), |acc, item| {
                                    acc + &item.to_string() + "\n"
                                });
                                http_error_future!(BAD_REQUEST, msg);
                            }
                        }
                    }
                    None => json!({}),
                };


                /*if body.is_end_stream() {
                    println!("NO BODY");
                }*/

                match (api_method.handler)(param, &api_method) {
                    Ok(res) => {
                        let json_str = res.to_string();
                        return Response::new(Body::from(json_str));
                    }
                    Err(err) => {
                        http_error_future!(NOT_FOUND, format!("Method returned error '{}'\n", err));
                    }
                }

*/
            } else {
                http_error_future!(NOT_FOUND, format!("No such path '{}'\n", path));
            }
        }
    }

    Box::new(future::ok(Response::new(Body::from("RETURN WEB GUI\n"))))
}

lazy_static!{
    static ref ROUTER: MethodInfo = apitest::api3::router();
}

fn main() {
    println!("Fast Static Type Definitions 1");

    let addr = ([127, 0, 0, 1], 8007).into();

    let new_svc = || {
        // service_fn_ok converts our function into a `Service`
        service_fn(handle_request)
    };

    let server = Server::bind(&addr)
        .serve(new_svc)
        .map_err(|e| eprintln!("server error: {}", e));

    // Run this server for... forever!
    hyper::rt::run(server);
}
