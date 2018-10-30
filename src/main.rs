#![feature(plugin)]
#![plugin(phf_macros)]

extern crate failure;

extern crate phf;


use failure::Error;

type StaticPropertyMap = phf::Map<&'static str, ApiTypeDef>;

#[derive(Debug)]
struct ApiTypeDefBoolean {
    description: &'static str,
    optional: Option<bool>,
    default: Option<bool>,
}

#[derive(Debug)]
struct ApiTypeDefInteger {
    description: &'static str,
    optional: Option<bool>,
    minimum: Option<usize>,
    maximum: Option<usize>,
    default: Option<usize>,
}

#[derive(Debug)]
struct ApiTypeDefString {
    description: &'static str,
    optional: Option<bool>,
    default: Option<&'static str>,
    min_length: Option<usize>,
    max_length: Option<usize>,
}

#[derive(Debug)]
struct ApiTypeDefArray {
    description: &'static str,
    optional: Option<bool>,
    items: &'static ApiTypeDef,
}

#[derive(Debug)]
struct ApiTypeDefObject {
    description: &'static str,
    optional: Option<bool>,
    additional_properties: Option<bool>,
    properties: &'static StaticPropertyMap,
}

#[derive(Debug)]
enum ApiTypeDef {
    Null,
    Boolean(ApiTypeDefBoolean),
    Integer(ApiTypeDefInteger),
    String(ApiTypeDefString),
    Object(ApiTypeDefObject),
    Array(ApiTypeDefArray),
    Reference { reference: &'static ApiTypeDef },
}

static DEFAULTBOOL: ApiTypeDefBoolean = ApiTypeDefBoolean {
    description: "",
    optional: None,
    default: None,
};

macro_rules! Boolean {
    ($($name:ident => $e:expr),*) => {{
        ApiTypeDef::Boolean(ApiTypeDefBoolean { $($name: $e, )* ..DEFAULTBOOL})
    }}
}

static DEFAULTINTEGER: ApiTypeDefInteger = ApiTypeDefInteger {
    description: "",
    optional: None,
    default: None,
    minimum: None,
    maximum: None,
};

macro_rules! Integer {
    ($($name:ident => $e:expr),*) => {{
        ApiTypeDef::Integer(ApiTypeDefInteger { $($name: $e, )* ..DEFAULTINTEGER})
    }}
}

static DEFAULTSTRING: ApiTypeDefString = ApiTypeDefString {
    description: "",
    optional: None,
    default: None,
    min_length: None,
    max_length: None,
};

macro_rules! ApiString {
    ($($name:ident => $e:expr),*) => {{
        ApiTypeDef::String(ApiTypeDefString { $($name: $e, )* ..DEFAULTSTRING})
    }}
}

static DEFAULTARRAY: ApiTypeDefArray = ApiTypeDefArray {
    description: "",
    optional: None,
    items: &ApiTypeDef::Null, // is this a reasonable default??
};

macro_rules! Array {
    ($($name:ident => $e:expr),*) => {{
        ApiTypeDef::Array(ApiTypeDefArray { $($name: $e, )* ..DEFAULTARRAY})
    }}
}

static EMPTYOBJECT: StaticPropertyMap = phf_map!{};

static DEFAULTOBJECT: ApiTypeDefObject = ApiTypeDefObject {
    description: "",
    optional: None,
    additional_properties: None,
    properties: &EMPTYOBJECT, // is this a reasonable default??
};

macro_rules! Object {
    ($($name:ident => $e:expr),*) => {{
        ApiTypeDef::Object(ApiTypeDefObject { $($name: $e, )* ..DEFAULTOBJECT})
    }}
}


// Standard Option Definitions
static PVE_VMID: ApiTypeDef = Integer!{
    description => "The (unique) ID of the VM.",
    minimum => Some(1)
};


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
    "myarray2" => ApiTypeDef::Array(ApiTypeDefArray {
        description: "Test Array of simple integers.",
        optional: Some(false),
        items: &Object!{description => "Empty Object."},
    }),
    "myobject" => Object!{
        description => "TEST Object.",
        properties => &phf_map!{
            "vmid" => ApiTypeDef::Reference { reference: &PVE_VMID},
            "loop" => Integer!{
                description => "Totally useless thing.",
                optional => Some(false)
            }
        }
    },
    "emptyobject" => Object!{description => "Empty Object."},
};


fn main() {
    println!("Fast Static Type Definitions 1");

    for (k, v) in PARAMETERS1.entries() {
        println!("Parameter: {} Value: {:?}", k, v);
    }
    
}
