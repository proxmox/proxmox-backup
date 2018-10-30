#![feature(plugin)]
#![plugin(phf_macros)]
extern crate phf;

extern crate failure;
use failure::*;

extern crate json_schema;
use json_schema::*;

extern crate serde_json;
#[macro_use]
extern crate serde_derive;

use serde_json::{json, Value};


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


struct ApiMethod {
    description: &'static str,
    properties: StaticPropertyMap,
    returns: Jss,
    handler: fn(Value) -> Result<Value, Error>,
}

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

type StaticSubdirMap = phf::Map<&'static str, &'static MethodInfo>;

struct MethodInfo {
    path: &'static str,
    get: Option<&'static ApiMethod>,
    subdirs: Option<&'static StaticSubdirMap>,
}

static API3_NODES: MethodInfo = MethodInfo {
    path: "",
    get: Some(&TEST_API_METHOD),
    subdirs: None,
};

static API3: MethodInfo = MethodInfo {
    path: "",
    get: Some(&TEST_API_METHOD),
    subdirs: Some(&phf_map!{"nodes" => &API3_NODES}),
};

fn main() {
    println!("Fast Static Type Definitions 1");

    for (k, v) in PARAMETERS1.entries() {
        println!("Parameter: {} Value: {:?}", k, v);
    }

}
