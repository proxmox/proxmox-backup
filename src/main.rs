extern crate failure;
use failure::*;

use apitest::static_map::StaticMap;

use std::collections::HashMap;

#[macro_use]
extern crate apitest;

use apitest::json_schema::*;
use apitest::api_info::*;


extern crate serde_json;
#[macro_use]
extern crate serde_derive;

use serde_json::{json, Value};

extern crate url;

use url::form_urlencoded;

extern crate hyper;

use hyper::{Method, Body, Request, Response, Server, StatusCode};
use hyper::rt::Future;
use hyper::service::service_fn_ok;

static PARAMETERS1: StaticPropertyMap = StaticPropertyMap {
    entries: &[
        ("force", Boolean!{
            description => "Test for boolean options."
        }),
        ("text1", ApiString!{
            description => "A simple text string.",
            min_length => Some(10),
            max_length => Some(30)
        }),
        ("count", Integer!{
            description => "A counter for everything.",
            minimum => Some(0),
            maximum => Some(10)
        }),
        ("myarray1", Array!{
            description => "Test Array of simple integers.",
            items => &PVE_VMID
        }),
        ("myarray2", Jss::Array(JssArray {
            description: "Test Array of simple integers.",
            optional: Some(false),
            items: &Object!{description => "Empty Object."},
        })),
        ("myobject", Object!{
            description => "TEST Object.",
            properties => &StaticPropertyMap {
                entries: &[ 
                    ("vmid", Jss::Reference { reference: &PVE_VMID}),
                    ("loop", Integer!{
                        description => "Totally useless thing.",
                        optional => Some(false)
                    })
                ]
            }
        }),
        ("emptyobject", Object!{description => "Empty Object."}),
    ]
};



#[derive(Serialize, Deserialize)]
struct Myparam {
    test: bool,
}

fn test_api_handler(param: Value) -> Result<Value, Error> {
    println!("This is a test {}", param);

   // let force: Option<bool> = Some(false);

    //if let Some(force) = param.force {
    //}

    let force =  param["force"].as_bool()
        .ok_or_else(|| format_err!("meine fehlermeldung"))?;

    if let Some(force) = param["force"].as_bool() {
    }

    let tmp: Myparam = serde_json::from_value(param)?;


    Ok(json!(null))
}

static TEST_API_METHOD: ApiMethod = ApiMethod {
    description: "This is a simple test.",
    properties: StaticPropertyMap {
        entries: &[
            ("force", Boolean!{
                optional => Some(true),
                description => "Test for boolean options."
            })
        ]
    },
    returns: Jss::Null,
    handler: test_api_handler,
};


static API3_NODES: MethodInfo = MethodInfo {
    get: Some(&TEST_API_METHOD),
    ..METHOD_INFO_DEFAULTS
};

static API_ROOT: MethodInfo = MethodInfo {
    get: Some(&TEST_API_METHOD),
    subdirs: Some(&StaticSubdirMap {
        entries: &[
            ("nodes", &API3_NODES),
        ]
    }),
    ..METHOD_INFO_DEFAULTS
};

macro_rules! http_error {
    ($status:ident, $msg:expr) => {{
        let mut resp = Response::new(Body::from($msg));
        *resp.status_mut() = StatusCode::$status;
        return resp;
    }}
}

fn parse_query(query: &str) -> Value {

    println!("PARSE QUERY {}", query);

    // fixme: what about repeated parameters (arrays?)
    let mut raw_param = HashMap::new();
    for (k, v) in form_urlencoded::parse(query.as_bytes()) {
        println!("QUERY PARAM {} value {}", k, v);
        raw_param.insert(k, v);
    }
    println!("QUERY HASH {:?}", raw_param);

    return json!(null);
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

            if let Some(info) = find_method_info(&API_ROOT, &components[2..]) {
                println!("FOUND INFO");
                let api_method_opt = match method {
                    &Method::GET => info.get,
                    &Method::PUT => info.put,
                    &Method::POST => info.post,
                    &Method::DELETE => info.delete,
                    _ => None,
                };
                let api_method = match api_method_opt {
                    Some(m) => m,
                    _ => http_error!(NOT_FOUND, format!("No such method '{} {}'\n", method, path)),
                };

                // handle auth

                // extract param
                let param = match query {
                    Some(data) => parse_query(data),
                    None => json!({}),
                };

            } else {
                http_error!(NOT_FOUND, format!("No such path '{}'\n", path));
            }
        }
    }

    Response::new(Body::from("RETURN WEB GUI\n"))
}

fn main() {
    println!("Fast Static Type Definitions 1");

    for (k, v) in PARAMETERS1.entries {
        println!("Parameter: {} Value: {:?}", k, v);
    }

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
