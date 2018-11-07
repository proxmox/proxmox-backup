use failure::*;
use std::collections::HashMap;
use serde_json::{json, Value};
use url::form_urlencoded;

pub type PropertyMap = HashMap<&'static str, Jss>;

#[derive(Debug)]
pub struct JssBoolean {
    pub description: &'static str,
    pub optional: bool,
    pub default: Option<bool>,
}

#[derive(Debug)]
pub struct JssInteger {
    pub description: &'static str,
    pub optional: bool,
    pub minimum: Option<isize>,
    pub maximum: Option<isize>,
    pub default: Option<isize>,
}

#[derive(Debug)]
pub struct JssString {
    pub description: &'static str,
    pub optional: bool,
    pub default: Option<&'static str>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
}

#[derive(Debug)]
pub struct JssArray {
    pub description: &'static str,
    pub optional: bool,
    pub items: Box<Jss>,
}

#[derive(Debug)]
pub struct JssObject {
    pub description: &'static str,
    pub optional: bool,
    pub additional_properties: bool,
    pub properties: HashMap<&'static str, Jss>,
}

#[derive(Debug)]
pub enum Jss {
    Null,
    Boolean(JssBoolean),
    Integer(JssInteger),
    String(JssString),
    Object(JssObject),
    Array(JssArray),
}

pub const DEFAULTBOOL: JssBoolean = JssBoolean {
    description: "",
    optional: false,
    default: None,
};

#[macro_export]
macro_rules! Boolean {
    ($($name:ident => $e:expr),*) => {{
        Jss::Boolean(JssBoolean { $($name: $e, )* ..DEFAULTBOOL})
    }}
}

pub const DEFAULTINTEGER: JssInteger = JssInteger {
    description: "",
    optional: false,
    default: None,
    minimum: None,
    maximum: None,
};

#[macro_export]
macro_rules! Integer {
    ($($name:ident => $e:expr),*) => {{
        Jss::Integer(JssInteger { $($name: $e, )* ..DEFAULTINTEGER})
    }}
}

pub const DEFAULTSTRING: JssString = JssString {
    description: "",
    optional: false,
    default: None,
    min_length: None,
    max_length: None,
};

#[macro_export]
macro_rules! ApiString {
    ($($name:ident => $e:expr),*) => {{
        Jss::String(JssString { $($name: $e, )* ..DEFAULTSTRING})
    }}
}

#[macro_export]
macro_rules! parameter {
    ($($name:ident => $e:expr),*) => {{
        let inner = JssObject {
            description: "",
            optional: false,
            additional_properties: false,
            properties: {
                let mut map = HashMap::<&'static str, Jss>::new();
                $(
                    map.insert(stringify!($name), $e);
                )*
                map
            }
        };

        Jss::Object(inner)
    }}
}

fn parse_simple_value(value_str: &str, schema: &Jss) -> Result<Value, Error> {

    let value = match schema {
        Jss::Null => {
            bail!("internal error - found Null schema.");
        }
        Jss::Boolean(jss_boolean) => {
            let res = match value_str.to_lowercase().as_str() {
                "1" | "on" | "yes" | "true" => true,
                "0" | "off" | "no" | "false" => false,
                _ => bail!("Unable to parse boolean option."),
            };
            Value::Bool(res)
        }
        Jss::Integer(jss_integer) => {
            let res: isize = value_str.parse()?;

            if let Some(minimum) = jss_integer.minimum {
                if res < minimum {
                    bail!("value must have a minimum value of {}", minimum);
                }
            }

            if let Some(maximum) = jss_integer.maximum {
                if res > maximum {
                    bail!("value must have a maximum value of {}", maximum);
                }
            }

            Value::Number(res.into())
        }
        Jss::String(jss_string) => {
            let res: String = value_str.into();
            let char_count = res.chars().count();

            if let Some(min_length) = jss_string.min_length {
                if char_count < min_length {
                    bail!("value must be at least {} characters long", min_length);
                }
            }

            if let Some(max_length) = jss_string.max_length {
                if char_count > max_length {
                    bail!("value may only be {} characters long", max_length);
                }
            }

            Value::String(res)
        }
        _ => bail!("unable to parse complex (sub) objects."),
    };
    Ok(value)
}

pub fn parse_parameter_strings(data: &Vec<(String, String)>, schema: &Jss, test_required: bool) -> Result<Value, Vec<Error>> {

    println!("QUERY Strings {:?}", data);

    let mut params = json!({});

    let mut errors: Vec<Error> = Vec::new();

    match schema {
        Jss::Object(JssObject { properties, additional_properties, .. })   => {
            for (key, value) in data {
                if let Some(prop_schema) = properties.get::<str>(key) {
                    match prop_schema {
                        Jss::Object(_) => {
                            errors.push(format_err!("parameter '{}': cant parse complex Objects.", key));
                        }
                        Jss::Array(jss_array) => {
                            if params[key] == Value::Null {
                                params[key] = json!([]);
                            }
                            match params[key] {
                                Value::Array(ref mut array) => {
                                    match parse_simple_value(value, &jss_array.items) {
                                        Ok(res) => array.push(res),
                                        Err(err) => errors.push(format_err!("parameter '{}': {}", key, err)),
                                    }
                                }
                                _ => errors.push(format_err!("parameter '{}': expected array - type missmatch", key)),
                            }
                        }
                        _ => {
                            match parse_simple_value(value, prop_schema) {
                                Ok(res) => {
                                    if params[key] == Value::Null {
                                        params[key] = res;
                                    } else {
                                         errors.push(format_err!("parameter '{}': duplicate parameter.", key));
                                    }
                                },
                                Err(err) => errors.push(format_err!("parameter '{}': {}", key, err)),
                            }
                        }

                    }
                } else {
                    if *additional_properties {
                        match params[key] {
                            Value::Null => {
                                params[key] = Value::String(value.to_owned());
                            },
                            Value::String(ref old) => {
                                params[key] = Value::Array(
                                    vec![Value::String(old.to_owned()),  Value::String(value.to_owned())]);
                            }
                            Value::Array(ref mut array) => {
                                array.push(Value::String(value.to_string()));
                            }
                            _ => errors.push(format_err!("parameter '{}': expected array - type missmatch", key)),
                        }
                    } else {
                        errors.push(format_err!("parameter '{}': schema does not allow additional properties.", key));
                    }
                }
            }

            if test_required {
                for (name, prop_schema) in properties {
                    let optional = match prop_schema {
                        Jss::Boolean(jss_boolean) => jss_boolean.optional,
                        Jss::Integer(jss_integer) => jss_integer.optional,
                        Jss::String(jss_string) => jss_string.optional,
                        Jss::Array(jss_array) => jss_array.optional,
                        Jss::Object(jss_object) => jss_object.optional,
                        Jss::Null => true,
                    };
                    if optional == false && params[name] == Value::Null {
                        errors.push(format_err!("parameter '{}': parameter is missing and it is not optional.", name));
                    }
                }
            }
        }
        _ => errors.push(format_err!("Got unexpected schema type in parse_parameter_strings.")),

    }

    if (errors.len() > 0) {
        Err(errors)
    } else {
        Ok(params)
    }
}

pub fn parse_query_string(query: &str, schema: &Jss, test_required: bool) -> Result<Value, Vec<Error>> {

    let param_list: Vec<(String, String)> =
        form_urlencoded::parse(query.as_bytes()).into_owned().collect();

    parse_parameter_strings(&param_list, schema, test_required)
}

#[test]
fn test_shema1() {
    let schema = Jss::Object(JssObject {
        description: "TEST",
        optional: false,
        additional_properties: false,
        properties: {
            let map = HashMap::new();

            map
        }
    });

    println!("TEST Schema: {:?}", schema);
}

#[test]
fn test_query_string() {

    let schema = parameter!{name => ApiString!{ optional => false }};

    let res = parse_query_string("", &schema, true);
    assert!(res.is_err());

    let schema = parameter!{name => ApiString!{ optional => true }};

    let res = parse_query_string("", &schema, true);
    assert!(res.is_ok());

    let schema = parameter!{name => ApiString!{
        optional => false,
        min_length => Some(5),
        max_length => Some(10)

    }};

    let res = parse_query_string("name=abcd", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("name=abcde", &schema, true);
    assert!(res.is_ok());

    let res = parse_query_string("name=abcdefghijk", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("name=abcdefghij", &schema, true);
    assert!(res.is_ok());



}

#[test]
fn test_query_integer() {

    let schema = parameter!{count => Integer!{ optional => false }};

    let res = parse_query_string("", &schema, true);
    assert!(res.is_err());

    let schema = parameter!{count => Integer!{
        optional => true,
        minimum => Some(-3),
        maximum => Some(50)
    }};

    let res = parse_query_string("", &schema, true);
    assert!(res.is_ok());

    let res = parse_query_string("count=abc", &schema, false);
    assert!(res.is_err());

    let res = parse_query_string("count=30", &schema, false);
    assert!(res.is_ok());

    let res = parse_query_string("count=-1", &schema, false);
    assert!(res.is_ok());

    let res = parse_query_string("count=300", &schema, false);
    assert!(res.is_err());

    let res = parse_query_string("count=-30", &schema, false);
    assert!(res.is_err());

    let res = parse_query_string("count=50", &schema, false);
    assert!(res.is_ok());

    let res = parse_query_string("count=-3", &schema, false);
    assert!(res.is_ok());
}

#[test]
fn test_query_boolean() {

    let schema = parameter!{force => Boolean!{ optional => false }};

    let res = parse_query_string("", &schema, true);
    assert!(res.is_err());

    let schema = parameter!{force => Boolean!{ optional => true }};

    let res = parse_query_string("", &schema, true);
    assert!(res.is_ok());

    let res = parse_query_string("a=b", &schema, true);
    assert!(res.is_err());


    let res = parse_query_string("force", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("force=yes", &schema, true);
    assert!(res.is_ok());
    let res = parse_query_string("force=1", &schema, true);
    assert!(res.is_ok());
    let res = parse_query_string("force=On", &schema, true);
    assert!(res.is_ok());
    let res = parse_query_string("force=TRUE", &schema, true);
    assert!(res.is_ok());
    let res = parse_query_string("force=TREU", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("force=NO", &schema, true);
    assert!(res.is_ok());
    let res = parse_query_string("force=0", &schema, true);
    assert!(res.is_ok());
    let res = parse_query_string("force=off", &schema, true);
    assert!(res.is_ok());
    let res = parse_query_string("force=False", &schema, true);
    assert!(res.is_ok());
}


/*
#[test]
fn test_shema1() {
    static PARAMETERS1: PropertyMap = propertymap!{
        force => &Boolean!{
            description => "Test for boolean options."
        },
        text1 => &ApiString!{
            description => "A simple text string.",
            min_length => Some(10),
            max_length => Some(30)
        },
        count => &Integer!{
            description => "A counter for everything.",
            minimum => Some(0),
            maximum => Some(10)
        },
        myarray1 => &Array!{
            description => "Test Array of simple integers.",
            items => &PVE_VMID
        },
        myarray2 => &Jss::Array(JssArray {
            description: "Test Array of simple integers.",
            optional: Some(false),
            items: &Object!{description => "Empty Object."},
        }),
        myobject => &Object!{
            description => "TEST Object.",
            properties => &propertymap!{
                vmid => &PVE_VMID,
                loop => &Integer!{
                    description => "Totally useless thing.",
                    optional => Some(false)
                }
            }
        },
        emptyobject => &Object!{description => "Empty Object."}
    };

    for (k, v) in PARAMETERS1.entries {
        println!("Parameter: {} Value: {:?}", k, v);
    }


}
*/
