use failure::*;
use std::collections::HashMap;
use serde_json::{json, Value};
use url::form_urlencoded;
use regex::Regex;
use std::fmt;
use std::sync::Arc;

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
    pub default: Option<bool>,
}

impl BooleanSchema {

    pub fn new(description: &'static str) -> Self {
        BooleanSchema {
            description: description,
            default: None,
        }
    }

    pub fn default(mut self, default: bool) -> Self {
        self.default = Some(default);
        self
    }
}

#[derive(Debug)]
pub struct IntegerSchema {
    pub description: &'static str,
    pub minimum: Option<isize>,
    pub maximum: Option<isize>,
    pub default: Option<isize>,
}

impl IntegerSchema {

    pub fn new(description: &'static str) -> Self {
        IntegerSchema {
            description: description,
            default: None,
            minimum: None,
            maximum: None,
        }
    }

    pub fn default(mut self, default: isize) -> Self {
        self.default = Some(default);
        self
    }

    pub fn minimum(mut self, minimum: isize) -> Self {
        self.minimum = Some(minimum);
        self
    }

    pub fn maximum(mut self, maximium: isize) -> Self {
        self.maximum = Some(maximium);
        self
    }
}


#[derive(Debug)]
pub struct StringSchema {
    pub description: &'static str,
    pub default: Option<&'static str>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub format: Option<Arc<ApiStringFormat>>,
}

impl StringSchema {

    pub fn new(description: &'static str) -> Self {
        StringSchema {
            description: description,
            default: None,
            min_length: None,
            max_length: None,
            format: None,
        }
    }

    pub fn default(mut self, text: &'static str) -> Self {
        self.default = Some(text);
        self
    }

    pub fn format(mut self, format: Arc<ApiStringFormat>) -> Self {
        self.format = Some(format);
        self
    }

    pub fn min_length(mut self, min_length: usize) -> Self {
        self.min_length = Some(min_length);
        self
    }

    pub fn max_length(mut self, max_length: usize) -> Self {
        self.max_length = Some(max_length);
        self
    }
}

#[derive(Debug)]
pub struct ArraySchema {
    pub description: &'static str,
    pub items: Arc<Schema>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
}

impl ArraySchema {

    pub fn new(description: &'static str, item_schema: Arc<Schema>) -> Self {
        ArraySchema {
            description: description,
            items: item_schema,
            min_length: None,
            max_length: None,
        }
    }

    pub fn min_length(mut self, min_length: usize) -> Self {
        self.min_length = Some(min_length);
        self
    }

    pub fn max_length(mut self, max_length: usize) -> Self {
        self.max_length = Some(max_length);
        self
    }
}

#[derive(Debug)]
pub struct ObjectSchema {
    pub description: &'static str,
    pub additional_properties: bool,
    pub properties: HashMap<&'static str, (bool, Arc<Schema>)>,
    pub default_key: Option<&'static str>,
}

impl ObjectSchema {

    pub fn new(description: &'static str) -> Self {
        let properties = HashMap::new();
        ObjectSchema {
            description: description,
            additional_properties: false,
            properties: properties,
            default_key: None,
        }
    }

    pub fn additional_properties(mut self, additional_properties: bool) -> Self {
        self.additional_properties = additional_properties;
        self
    }

    pub fn default_key(mut self, key: &'static str) -> Self {
        self.default_key = Some(key);
        self
    }

    pub fn required<S: Into<Arc<Schema>>>(mut self, name: &'static str, schema: S) -> Self {
        self.properties.insert(name, (false, schema.into()));
        self
    }

    pub fn optional<S: Into<Arc<Schema>>>(mut self, name: &'static str, schema: S) -> Self {
        self.properties.insert(name, (true, schema.into()));
        self
    }
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

impl From<StringSchema> for Schema {
    fn from(string_schema: StringSchema) -> Self {
        Schema::String(string_schema)
    }
}

impl From<StringSchema> for Arc<Schema> {
    fn from(string_schema: StringSchema) -> Self {
        Arc::new(Schema::String(string_schema))
    }
}

impl From<BooleanSchema> for Schema {
    fn from(boolean_schema: BooleanSchema) -> Self {
        Schema::Boolean(boolean_schema)
    }
}

impl From<BooleanSchema> for Arc<Schema> {
    fn from(boolean_schema: BooleanSchema) -> Self {
        Arc::new(Schema::Boolean(boolean_schema))
    }
}

impl From<IntegerSchema> for Schema {
    fn from(integer_schema: IntegerSchema) -> Self {
        Schema::Integer(integer_schema)
    }
}

impl From<IntegerSchema> for Arc<Schema> {
    fn from(integer_schema: IntegerSchema) -> Self {
        Arc::new(Schema::Integer(integer_schema))
    }
}

impl From<ObjectSchema> for Schema {
    fn from(object_schema: ObjectSchema) -> Self {
        Schema::Object(object_schema)
    }
}

impl From<ObjectSchema> for Arc<Schema> {
    fn from(object_schema: ObjectSchema) -> Self {
        Arc::new(Schema::Object(object_schema))
    }
}

impl From<ArraySchema> for Schema {
    fn from(array_schema: ArraySchema) -> Self {
        Schema::Array(array_schema)
    }
}

impl From<ArraySchema> for Arc<Schema> {
    fn from(array_schema: ArraySchema) -> Self {
        Arc::new(Schema::Array(array_schema))
    }
}

pub enum ApiStringFormat {
    Enum(Vec<String>),
    Pattern(Box<Regex>),
    Complex(Arc<Schema>),
    VerifyFn(fn(&str) -> Result<(), Error>),
}

impl std::fmt::Debug for ApiStringFormat {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ApiStringFormat::VerifyFn(fnptr) => {
                write!(f, "VerifyFn({:p}", fnptr)
            }
            ApiStringFormat::Enum(strvec) => {
                write!(f, "Enum({:?}", strvec)
            }
            ApiStringFormat::Pattern(regex) => {
                write!(f, "Pattern({:?}", regex)
            }
            ApiStringFormat::Complex(schema) => {
                write!(f, "Complex({:?}", schema)
            }
        }
    }
}

pub fn parse_boolean(value_str: &str) -> Result<bool, Error> {
    match value_str.to_lowercase().as_str() {
        "1" | "on" | "yes" | "true" => Ok(true),
        "0" | "off" | "no" | "false" => Ok(false),
        _ => bail!("Unable to parse boolean option."),
    }
}

fn parse_property_string(value_str: &str, schema: &Schema) -> Result<Value, Error> {

    println!("Parse property string: {}", value_str);

    let mut param_list: Vec<(String, String)> = vec![];

    match schema {
        Schema::Object(object_schema) => {
            for key_val in value_str.split(',').filter(|s| !s.is_empty()) {
                let kv: Vec<&str> = key_val.splitn(2, '=').collect();
                if kv.len() == 2 {
                    param_list.push((kv[0].into(), kv[1].into()));
                } else {
                    if let Some(key) = object_schema.default_key {
                        param_list.push((key.into(), kv[0].into()));
                    } else {
                        bail!("Value without key, but schema does not define a default key.");
                    }
                }
            }

            return parse_parameter_strings(&param_list, &object_schema, true)
                .map_err(Error::from);

        }
        Schema::Array(array_schema) => {
            let mut array : Vec<Value> = vec![];
            for value in value_str.split(',').filter(|s| !s.is_empty()) {
                match parse_simple_value(value, &array_schema.items) {
                    Ok(res) => array.push(res),
                    Err(err) => bail!("unable to parse array element: {}", err),
                }
            }

            if let Some(min_length) = array_schema.min_length {
                if array.len() < min_length {
                    bail!("array must contain at least {} elements", min_length);
                }
            }

            if let Some(max_length) = array_schema.max_length {
                if array.len() > max_length {
                    bail!("array may only contain {} elements", max_length);
                }
            }

            return Ok(array.into());
        }
        _ => {
            bail!("Got unexpetec schema type.")
        }
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

            if let Some(ref format) = string_schema.format {
                match format.as_ref() {
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
                    ApiStringFormat::Complex(ref subschema) => {
                        parse_property_string(&res, subschema)?;
                    }
                    ApiStringFormat::VerifyFn(verify_fn) => {
                        verify_fn(&res)?;
                    }
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
        if let Some((_optional, prop_schema)) = properties.get::<str>(key) {
            match prop_schema.as_ref() {
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
        for (name, (optional, _prop_schema)) in properties {
            if *optional == false && params[name] == Value::Null {
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
        additional_properties: false,
        properties: {
            let map = HashMap::new();

            map
        },
        default_key: None,
    });

    println!("TEST Schema: {:?}", schema);
}

#[test]
fn test_query_string() {

    let schema = ObjectSchema::new("Parameters.")
        .required("name", StringSchema::new("Name."));

    let res = parse_query_string("", &schema, true);
    assert!(res.is_err());

    let schema = ObjectSchema::new("Parameters.")
        .optional("name", StringSchema::new("Name."));

    let res = parse_query_string("", &schema, true);
    assert!(res.is_ok());

    // TEST min_length and max_length

    let schema = ObjectSchema::new("Parameters.")
        .required(
            "name", StringSchema::new("Name.")
                .min_length(5)
                .max_length(10)
        );

    let res = parse_query_string("name=abcd", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("name=abcde", &schema, true);
    assert!(res.is_ok());

    let res = parse_query_string("name=abcdefghijk", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("name=abcdefghij", &schema, true);
    assert!(res.is_ok());

    // TEST regex pattern

    let schema = ObjectSchema::new("Parameters.")
        .required(
            "name", StringSchema::new("Name.")
                .format(Arc::new(ApiStringFormat::Pattern(Box::new(Regex::new("test").unwrap()))))
        );

    let res = parse_query_string("name=abcd", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("name=ateststring", &schema, true);
    assert!(res.is_ok());

    let schema = ObjectSchema::new("Parameters.")
        .required(
            "name", StringSchema::new("Name.")
                .format(Arc::new(ApiStringFormat::Pattern(Box::new(Regex::new("^test$").unwrap()))))
        );

    let res = parse_query_string("name=ateststring", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("name=test", &schema, true);
    assert!(res.is_ok());

    // TEST string enums

    let schema = ObjectSchema::new("Parameters.")
        .required(
            "name", StringSchema::new("Name.")
                .format(Arc::new(ApiStringFormat::Enum(vec!["ev1".into(), "ev2".into()])))
        );

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

    let schema = ObjectSchema::new("Parameters.")
        .required(
            "count" , IntegerSchema::new("Count.")
        );

    let res = parse_query_string("", &schema, true);
    assert!(res.is_err());

    let schema = ObjectSchema::new("Parameters.")
        .optional(
            "count", IntegerSchema::new("Count.")
                .minimum(-3)
                .maximum(50)
        );

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

    let schema = ObjectSchema::new("Parameters.")
        .required(
            "force", BooleanSchema::new("Force.")
        );

    let res = parse_query_string("", &schema, true);
    assert!(res.is_err());

    let schema = ObjectSchema::new("Parameters.")
        .optional(
            "force", BooleanSchema::new("Force.")
        );

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

#[test]
fn test_verify_function() {

    let schema = ObjectSchema::new("Parameters.")
        .required(
            "p1", StringSchema::new("P1")
                .format(ApiStringFormat::VerifyFn(|value| {
                    if value == "test" { return Ok(()) };
                    bail!("format error");
                }).into())
        );

    let res = parse_query_string("p1=tes", &schema, true);
    assert!(res.is_err());
    let res = parse_query_string("p1=test", &schema, true);
    assert!(res.is_ok());
}

#[test]
fn test_verify_complex_object() {

    let nic_models = Arc::new(ApiStringFormat::Enum(
        vec!["e1000".into(), "virtio".into()]));

    let param_schema: Arc<Schema> = ObjectSchema::new("Properties.")
        .default_key("model")
        .required("model", StringSchema::new("Ethernet device Model.")
                  .format(nic_models))
        .optional("enable", BooleanSchema::new("Enable device."))
        .into();

    let schema = ObjectSchema::new("Parameters.")
        .required(
            "net0", StringSchema::new("First Network device.")
                .format(ApiStringFormat::Complex(param_schema).into())
        );

    let res = parse_query_string("", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("test=abc", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("net0=model=abc", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("net0=model=virtio", &schema, true);
     assert!(res.is_ok());

    let res = parse_query_string("net0=model=virtio,enable=1", &schema, true);
    assert!(res.is_ok());

    let res = parse_query_string("net0=virtio,enable=no", &schema, true);
    assert!(res.is_ok());
}

#[test]
fn test_verify_complex_array() {

    let param_schema: Arc<Schema> = ArraySchema::new(
        "Integer List.", Arc::new(IntegerSchema::new("Soemething").into()))
        .into();

    let schema = ObjectSchema::new("Parameters.")
        .required(
            "list", StringSchema::new("A list on integers, comma separated.")
                .format(ApiStringFormat::Complex(param_schema).into())
        );

    let res = parse_query_string("", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("list=", &schema, true);
    assert!(res.is_ok());

    let res = parse_query_string("list=abc", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("list=1", &schema, true);
    assert!(res.is_ok());

    let res = parse_query_string("list=2,3,4,5", &schema, true);
    assert!(res.is_ok());

    let param_schema: Arc<Schema> = ArraySchema::new(
        "Integer List.", Arc::new(IntegerSchema::new("Soemething").into()))
        .min_length(1)
        .max_length(3)
        .into();

    let schema = ObjectSchema::new("Parameters.")
        .required(
            "list", StringSchema::new("A list on integers, comma separated.")
                .format(ApiStringFormat::Complex(param_schema).into())
        );

    let res = parse_query_string("list=", &schema, true);
    assert!(res.is_err());

    let res = parse_query_string("list=1,2,3", &schema, true);
    assert!(res.is_ok());

    let res = parse_query_string("list=2,3,4,5", &schema, true);
    assert!(res.is_err());
}
