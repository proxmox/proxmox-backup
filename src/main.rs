#![feature(plugin)]
#![plugin(phf_macros)]
extern crate phf;

extern crate failure;
use failure::*;

#[macro_use]
extern crate apitest;

use apitest::json_schema::*;
use apitest::api_info::*;


extern crate serde_json;
#[macro_use]
extern crate serde_derive;

use serde_json::{json, Value};

extern crate hyper;

use hyper::{Body, Request, Response, Server};
use hyper::rt::Future;
use hyper::service::service_fn_ok;

static PARAMETERS1: StaticPropertyMap = phf_map! {
    "force" => Boolean!{
        description => "Test for boolean options."
    },
    "text1" => ApiString!{
        description => "A simple text string.",
        min_length => Some(10),
        max_length => Some(30)
    },
    "count" => Integer!{
        description => "A counter for everything.",
        minimum => Some(0),
        maximum => Some(10)
    },
    "myarray1" => Array!{
        description => "Test Array of simple integers.",
        items => &PVE_VMID
    },
    "myarray2" => Jss::Array(JssArray {
        description: "Test Array of simple integers.",
        optional: Some(false),
        items: &Object!{description => "Empty Object."},
    }),
    "myobject" => Object!{
        description => "TEST Object.",
        properties => &phf_map!{
            "vmid" => Jss::Reference { reference: &PVE_VMID},
            "loop" => Integer!{
                description => "Totally useless thing.",
                optional => Some(false)
            }
        }
    },
    "emptyobject" => Object!{description => "Empty Object."},
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
    properties: phf_map! {
        "force" => Boolean!{
            description => "Test for boolean options."
        }
    },
    returns: Jss::Null,
    handler: test_api_handler,
};


static API3_NODES: MethodInfo = MethodInfo {
    get: Some(&TEST_API_METHOD),
    ..METHOD_INFO_DEFAULTS
};

static API3: MethodInfo = MethodInfo {
    get: Some(&TEST_API_METHOD),
    subdirs: Some(&phf_map!{"nodes" => &API3_NODES}),
    ..METHOD_INFO_DEFAULTS
};


fn hello_world(req: Request<Body>) -> Response<Body> {

    let method = req.method();
    let path = req.uri().path();

    println!("REQUEST {} {}", method, path);

    Response::new(Body::from("hello World!\n"))
}

fn main() {
    println!("Fast Static Type Definitions 1");

    for (k, v) in PARAMETERS1.entries() {
        println!("Parameter: {} Value: {:?}", k, v);
    }

    let addr = ([127, 0, 0, 1], 8007).into();

    let new_svc = || {
        // service_fn_ok converts our function into a `Service`
        service_fn_ok(hello_world)
    };

    let server = Server::bind(&addr)
        .serve(new_svc)
        .map_err(|e| eprintln!("server error: {}", e));

    // Run this server for... forever!
    hyper::rt::run(server);
}
