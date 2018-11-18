use failure::*;
use std::collections::HashMap;
use serde_json::{json, Value};
use url::form_urlencoded;
use regex::Regex;
use std::fmt;

pub type PropertyMap = HashMap<&'static str, Schema>;

#[derive(Debug, Fail)]
pub struct ParameterError {
    error_list: Vec<Error>,
}

impl ParameterError {

    pub fn new() -> Self {
        Self { error_list: vec![] }
    }

    pub fn push(&mut self, value: Error) {
        self.error_list.push(value);
    }

    pub fn len(&self) -> usize {
        self.error_list.len()
    }
}

impl fmt::Display for ParameterError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let msg = self.error_list.iter().fold(String::from(""), |acc, item| {
            acc + &item.to_string() + "\n"
        });

        write!(f, "{}", msg)
    }
}

#[derive(Debug)]
pub struct BooleanSchema {
    pub description: &'static str,
    pub optional: bool,
    pub default: Option<bool>,
}

#[derive(Debug)]
pub struct IntegerSchema {
    pub description: &'static str,
    pub optional: bool,
    pub minimum: Option<isize>,
    pub maximum: Option<isize>,
    pub default: Option<isize>,
}

#[derive(Debug)]
pub struct StringSchema {
    pub description: &'static str,
    pub optional: bool,
    pub default: Option<&'static str>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub format: ApiStringFormat,
}

#[derive(Debug)]
pub struct ArraySchema {
    pub description: &'static str,
    pub optional: bool,
    pub items: Box<Schema>,
}

#[derive(Debug)]
pub struct ObjectSchema {
    pub description: &'static str,
    pub optional: bool,
    pub additional_properties: bool,
    pub properties: HashMap<&'static str, Schema>,
}

#[derive(Debug)]
pub enum Schema {
    Null,
    Boolean(BooleanSchema),
    Integer(IntegerSchema),
    String(StringSchema),
    Object(ObjectSchema),
    Array(ArraySchema),
}

pub const DEFAULTBOOL: BooleanSchema = BooleanSchema {
    description: "",
    optional: false,
    default: None,
};

#[macro_export]
macro_rules! Boolean {
    ($($name:ident => $e:expr),*) => {{
        Schema::Boolean(BooleanSchema { $($name: $e, )* ..DEFAULTBOOL})
    }}
}

pub const DEFAULTINTEGER: IntegerSchema = IntegerSchema {
    description: "",
    optional: false,
    default: None,
    minimum: None,
    maximum: None,
};

#[macro_export]
macro_rules! Integer {
    ($($name:ident => $e:expr),*) => {{
        Schema::Integer(IntegerSchema { $($name: $e, )* ..DEFAULTINTEGER})
    }}
}

pub const DEFAULTSTRING: StringSchema = StringSchema {
    description: "",
    optional: false,
    default: None,
    min_length: None,
    max_length: None,
    format: ApiStringFormat::None,
};

#[derive(Debug)]
pub enum ApiStringFormat {
    None,
    Enum(Vec<String>),
    Pattern(Box<Regex>),
    Complex(Box<Schema>),
}

#[macro_export]
macro_rules! ApiString {
    ($($name:ident => $e:expr),*) => {{
        Schema::String(StringSchema { $($name: $e, )* ..DEFAULTSTRING})
    }}
}


#[macro_export]
macro_rules! parameter {
    () => {{
        ObjectSchema {
            description: "",
            optional: false,
            additional_properties: false,
            properties: HashMap::<&'static str, Schema>::new(),
        }
    }};
    ($($name:ident => $e:expr),*) => {{
        ObjectSchema {
            description: "",
            optional: false,
            additional_properties: false,
            properties: {
                let mut map = HashMap::<&'static str, Schema>::new();
                $(
                    map.insert(stringify!($name), $e);
                )*
                map
            }
        }
    }};
}

pub fn parse_boolean(value_str: &str) -> Result<bool, Error> {
    match value_str.to_lowercase().as_str() {
        "1" | "on" | "yes" | "true" => Ok(true),
        "0" | "off" | "no" | "false" => Ok(false),
        _ => bail!("Unable to parse boolean option."),
    }
}

fn parse_simple_value(value_str: &str, schema: &Schema) -> Result<Value, Error> {

    let value = match schema {
        Schema::Null => {
            bail!("internal error - found Null schema.");
        }
        Schema::Boolean(_boolean_schema) => {
            let res = parse_boolean(value_str)?;
            Value::Bool(res)
        }
        Schema::Integer(integer_schema) => {
            let res: isize = value_str.parse()?;

            if let Some(minimum) = integer_schema.minimum {
                if res < minimum {
                    bail!("value must have a minimum value of {}", minimum);
                }
            }

            if let Some(maximum) = integer_schema.maximum {
                if res > maximum {
                    bail!("value must have a maximum value of {}", maximum);
                }
            }

            Value::Number(res.into())
        }
        Schema::String(string_schema) => {
            let res: String = value_str.into();
            let char_count = res.chars().count();

            if let Some(min_length) = string_schema.min_length {
                if char_count < min_length {
                    bail!("value must be at least {} characters long", min_length);
                }
            }

            if let Some(max_length) = string_schema.max_length {
                if char_count > max_length {
                    bail!("value may only be {} characters long", max_length);
                }
            }

            match string_schema.format {
                ApiStringFormat::None => { /* do nothing */ }
                ApiStringFormat::Pattern(ref regex) => {
                    if !regex.is_match(&res) {
                        bail!("value does not match the regex pattern");
                    }
                }
                ApiStringFormat::Enum(ref stringvec) => {
                    if stringvec.iter().find(|&e| *e == res) == None {
                        bail!("value is not defined in the enumeration.");
                    }
                }
                ApiStringFormat::Complex(ref _subschema) => {
                    bail!("implement me!");
                }
            }

            Value::String(res)
        }
        _ => bail!("unable to parse complex (sub) objects."),
    };
    Ok(value)
}

pub fn parse_parameter_strings(data: &Vec<(String, String)>, schema: &ObjectSchema, test_required: bool) -> Result<Value, ParameterError> {

    println!("QUERY Strings {:?}", data);

    let mut params = json!({});

    let mut errors = ParameterError::new();

    let properties = &schema.properties;
    let additional_properties = schema.additional_properties;

    for (key, value) in data {
        if let Some(prop_schema) = properties.get::<str>(key) {
            match prop_schema {
                Schema::Array(array_schema) => {
                    if params[key] == Value::Null {
                        params[key] = json!([]);
                    }
                    match params[key] {
                        Value::Array(ref mut array) => {
                            match parse_simple_value(value, &array_schema.items) {
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
            if additional_properties {
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

    if test_required && errors.len() == 0 {
        for (name, prop_schema) in properties {
            let optional = match prop_schema {
                Schema::Boolean(boolean_schema) => boolean_schema.optional,
                Schema::Integer(integer_schema) => integer_schema.optional,
                Schema::String(string_schema) => string_schema.optional,
                Schema::Array(array_schema) => array_schema.optional,
                Schema::Object(object_schema) => object_schema.optional,
                Schema::Null => true,
            };
            if optional == false && params[name] == Value::Null {
                errors.push(format_err!("parameter '{}': parameter is missing and it is not optional.", name));
            }
        }
    }

    if errors.len() > 0 {
        Err(errors)
    } else {
        Ok(params)
    }
}

pub fn parse_query_string(query: &str, schema: &ObjectSchema, test_required: bool) -> Result<Value,  ParameterError> {

    let param_list: Vec<(String, String)> =
        form_urlencoded::parse(query.as_bytes()).into_owned().collect();

    parse_parameter_strings(&param_list, schema, test_required)
}

#[test]
fn test_schema1() {
    let schema = Schema::Object(ObjectSchema {
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

    // TEST min_length and max_length

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

    // TEST regex pattern

    let schema = parameter!{name => ApiString!{
        optional => false,
        format => ApiStringFormat::Pattern(Box::new(Regex::new("test").unwrap()))
    }};

    let res = parse_query_string("name=abcd", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("name=ateststring", &schema, true);
    assert!(res.is_ok());

    let schema = parameter!{name => ApiString!{
        optional => false,
        format => ApiStringFormat::Pattern(Box::new(Regex::new("^test$").unwrap()))
    }};

    let res = parse_query_string("name=ateststring", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("name=test", &schema, true);
    assert!(res.is_ok());

    // TEST string enums

    let schema = parameter!{name => ApiString!{
        optional => false,
        format => ApiStringFormat::Enum(vec!["ev1".into(), "ev2".into()])
    }};

    let res = parse_query_string("name=noenum", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("name=ev1", &schema, true);
    assert!(res.is_ok());

    let res = parse_query_string("name=ev2", &schema, true);
    assert!(res.is_ok());

    let res = parse_query_string("name=ev3", &schema, true);
    assert!(res.is_err());

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
        myarray2 => &Schema::Array(ArraySchema {
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
