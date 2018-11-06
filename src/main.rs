extern crate apitest;

use failure::*;

use std::collections::HashMap;
use lazy_static::lazy_static;

//use apitest::json_schema::*;
use apitest::api_info::*;
use apitest::json_schema::*;

//use serde_derive::{Serialize, Deserialize};
use serde_json::{json, Value};

use url::form_urlencoded;

use hyper::{Method, Body, Request, Response, Server, StatusCode};
use hyper::rt::Future;
use hyper::service::service_fn_ok;

macro_rules! http_error {
    ($status:ident, $msg:expr) => {{
        let mut resp = Response::new(Body::from($msg));
        *resp.status_mut() = StatusCode::$status;
        return resp;
    }}
}


fn handle_request(req: Request<Body>) -> Response<Body> {

    let method = req.method();
    let path = req.uri().path();
    let query = req.uri().query();
    let components: Vec<&str> = path.split('/').filter(|x| !x.is_empty()).collect();
    let comp_len = components.len();

    println!("REQUEST {} {}", method, path);
    println!("COMPO {:?}", components);

    if comp_len >= 1 && components[0] == "api3" {
        println!("GOT API REQUEST");
        if comp_len >= 2 {
            let format = components[1];
            if format != "json" {
                http_error!(NOT_FOUND, format!("Unsupported format '{}'\n", format))
            }

            if let Some(info) = ROUTER.find_method(&components[2..]) {
                println!("FOUND INFO");
                let api_method_opt = match method {
                    &Method::GET => &info.get,
                    &Method::PUT => &info.put,
                    &Method::POST => &info.post,
                    &Method::DELETE => &info.delete,
                    _ => &None,
                };
                let api_method = match api_method_opt {
                    Some(m) => m,
                    _ => http_error!(NOT_FOUND, format!("No such method '{} {}'\n", method, path)),
                };

                // handle auth

                // extract param
                let param = match query {
                    Some(data) => {
                        let param_list: Vec<(String, String)> =
                            form_urlencoded::parse(data.as_bytes()).into_owned().collect();

                        match parse_parameter_strings(&param_list, &api_method.parameters) {
                            Ok(query) => query,
                            Err(ref error_list) => {
                                let mut msg = String::from("");
                                for item in error_list {
                                    msg = msg + &item.to_string() + "\n";
                                }
                                http_error!(BAD_REQUEST, msg);
                            }
                        }
                    }
                    None => json!({}),
                };

                match (api_method.handler)(param) {
                    Ok(res) => {
                        let json_str = res.to_string();
                        return Response::new(Body::from(json_str));
                    }
                    Err(err) => {
                        http_error!(NOT_FOUND, format!("Method returned error '{}'\n", err));
                    }
                }

            } else {
                http_error!(NOT_FOUND, format!("No such path '{}'\n", path));
            }
        }
    }

    Response::new(Body::from("RETURN WEB GUI\n"))
}

lazy_static!{
    static ref ROUTER: MethodInfo = apitest::api3::router();
}

fn main() {
    println!("Fast Static Type Definitions 1");

    let addr = ([127, 0, 0, 1], 8007).into();

    let new_svc = || {
        // service_fn_ok converts our function into a `Service`
        service_fn_ok(handle_request)
    };

    let server = Server::bind(&addr)
        .serve(new_svc)
        .map_err(|e| eprintln!("server error: {}", e));

    // Run this server for... forever!
    hyper::rt::run(server);
}
