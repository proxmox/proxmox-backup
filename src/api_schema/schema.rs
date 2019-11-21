use failure::*;
use serde_json::{json, Value};
use url::form_urlencoded;
use std::fmt;

#[derive(Default, Debug, Fail)]
pub struct ParameterError {
    error_list: Vec<Error>,
}

/// Error type for schema validation
///
/// The validation functions may produce several error message,
/// i.e. when validation objects, it can produce one message for each
/// erroneous object property.

// fixme: record parameter names, to make it usefull to display errord
// on HTML forms.
impl ParameterError {

    pub fn new() -> Self {
        Self { error_list: Vec::new() }
    }

    pub fn push(&mut self, value: Error) {
        self.error_list.push(value);
    }

    pub fn len(&self) -> usize {
        self.error_list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Display for ParameterError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {

        let mut msg = String::new();

        if !self.is_empty() {
            msg.push_str("parameter verification errors\n\n");
        }

        msg.push_str(&self.error_list.iter().fold(String::from(""), |acc, item| {
            acc + &item.to_string() + "\n"
        }));

        write!(f, "{}", msg)
    }
}

#[derive(Debug)]
pub struct BooleanSchema {
    pub description: &'static str,
    pub default: Option<bool>,
}

impl BooleanSchema {

    pub const fn new(description: &'static str) -> Self {
        BooleanSchema {
            description,
            default: None,
        }
    }

    pub const fn default(mut self, default: bool) -> Self {
        self.default = Some(default);
        self
    }

    pub const fn schema(self) -> Schema {
        Schema::Boolean(self)
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

    pub const fn new(description: &'static str) -> Self {
        IntegerSchema {
            description,
            default: None,
            minimum: None,
            maximum: None,
        }
    }

    pub const fn default(mut self, default: isize) -> Self {
        self.default = Some(default);
        self
    }

    pub const fn minimum(mut self, minimum: isize) -> Self {
        self.minimum = Some(minimum);
        self
    }

    pub const fn maximum(mut self, maximium: isize) -> Self {
        self.maximum = Some(maximium);
        self
    }

    pub const fn schema(self) -> Schema {
        Schema::Integer(self)
    }

    fn check_constraints(&self, value: isize) -> Result<(), Error> {

        if let Some(minimum) = self.minimum {
            if value < minimum {
                bail!("value must have a minimum value of {} (got {})", minimum, value);
            }
        }

        if let Some(maximum) = self.maximum {
            if value > maximum {
                bail!("value must have a maximum value of {} (got {})", maximum, value);
            }
        }

        Ok(())
    }
}

/// Helper to represent const regular expressions
///
/// This is mostly a workaround, unless we can create const_fn Regex.
pub struct ConstRegexPattern {
    pub regex_string: &'static str,
    pub regex_obj: fn() -> &'static regex::Regex,
}

impl std::fmt::Debug for ConstRegexPattern {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.regex_string)
    }
}

/// Macro to generate a ConstRegexPattern
#[macro_export]
macro_rules! const_regex {
    () =>   {};
    ($(#[$attr:meta])* pub ($($vis:tt)+) $name:ident = $regex:expr; $($rest:tt)*) =>  {
        const_regex! { (pub ($($vis)+)) $(#[$attr])* $name = $regex; $($rest)* }
    };
    ($(#[$attr:meta])* pub $name:ident = $regex:expr; $($rest:tt)*) =>  {
        const_regex! { (pub) $(#[$attr])* $name = $regex; $($rest)* }
    };
    ($(#[$attr:meta])* $name:ident = $regex:expr; $($rest:tt)*) =>  {
        const_regex! { () $(#[$attr])* $name = $regex; $($rest)* }
    };
    (
        ($($pub:tt)*) $(#[$attr:meta])* $name:ident = $regex:expr;
        $($rest:tt)*
    ) =>  {
        $(#[$attr])* $($pub)* const $name: ConstRegexPattern = ConstRegexPattern {
            regex_string: $regex,
            regex_obj: (|| ->   &'static regex::Regex {
                lazy_static::lazy_static! {
                    static ref SCHEMA: regex::Regex = regex::Regex::new($regex).unwrap();
                }
                &SCHEMA
            })
        };

        const_regex! { $($rest)* }
    };
}

#[derive(Debug)]
pub struct StringSchema {
    pub description: &'static str,
    pub default: Option<&'static str>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub format: Option<&'static ApiStringFormat>,
}

impl StringSchema {

    pub const fn new(description: &'static str) -> Self {
        StringSchema {
            description,
            default: None,
            min_length: None,
            max_length: None,
            format: None,
        }
    }

    pub const fn default(mut self, text: &'static str) -> Self {
        self.default = Some(text);
        self
    }

    pub const fn format(mut self, format: &'static ApiStringFormat) -> Self {
        self.format = Some(format);
        self
    }

    pub const fn min_length(mut self, min_length: usize) -> Self {
        self.min_length = Some(min_length);
        self
    }

    pub const fn max_length(mut self, max_length: usize) -> Self {
        self.max_length = Some(max_length);
        self
    }

    pub const fn schema(self) -> Schema {
        Schema::String(self)
    }
    
    fn check_length(&self, length: usize) -> Result<(), Error> {

        if let Some(min_length) = self.min_length {
            if length < min_length {
                bail!("value must be at least {} characters long", min_length);
            }
        }

        if let Some(max_length) = self.max_length {
            if length > max_length {
                bail!("value may only be {} characters long", max_length);
            }
        }

        Ok(())
    }

    pub fn check_constraints(&self, value: &str) -> Result<(), Error> {

        self.check_length(value.chars().count())?;

        if let Some(ref format) = self.format {
            match format {
                ApiStringFormat::Pattern(regex) => {
                    if !(regex.regex_obj)().is_match(value) {
                        bail!("value does not match the regex pattern");
                    }
                }
                ApiStringFormat::Enum(stringvec) => {
                    if stringvec.iter().find(|&e| *e == value) == None {
                        bail!("value '{}' is not defined in the enumeration.", value);
                    }
                }
                ApiStringFormat::Complex(subschema) => {
                    parse_property_string(value, subschema)?;
                }
                ApiStringFormat::VerifyFn(verify_fn) => {
                    verify_fn(value)?;
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct ArraySchema {
    pub description: &'static str,
    pub items: &'static Schema,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
}

impl ArraySchema {

    pub const fn new(description: &'static str, item_schema: &'static Schema) -> Self {
        ArraySchema {
            description,
            items: item_schema,
            min_length: None,
            max_length: None,
        }
    }

    pub const fn min_length(mut self, min_length: usize) -> Self {
        self.min_length = Some(min_length);
        self
    }

    pub const fn max_length(mut self, max_length: usize) -> Self {
        self.max_length = Some(max_length);
        self
    }

    pub const fn schema(self) -> Schema {
        Schema::Array(self)
    }

    fn check_length(&self, length: usize) -> Result<(), Error> {

        if let Some(min_length) = self.min_length {
            if length < min_length {
                bail!("array must contain at least {} elements", min_length);
            }
        }

        if let Some(max_length) = self.max_length {
            if length > max_length {
                bail!("array may only contain {} elements", max_length);
            }
        }

        Ok(())
    }
}

/// Lookup table to Schema properties
/// 
/// Stores a sorted list of (name, optional, schema) tuples:
///
/// name: The name of the property
/// optional: Set when the property is optional
/// schema: Property type schema
///
/// NOTE: The list has to be storted by name, because we use
/// a binary search to find items.
///
/// This is a workaround unless RUST can const_fn Hash::new()
pub type SchemaPropertyMap = &'static [(&'static str, bool, &'static Schema)];

#[derive(Debug)]
pub struct ObjectSchema {
    pub description: &'static str,
    pub additional_properties: bool,
    pub properties: SchemaPropertyMap,
    pub default_key: Option<&'static str>,
}

impl ObjectSchema {

    pub const fn new(description: &'static str,  properties: SchemaPropertyMap) -> Self {
        ObjectSchema {
            description,
            properties,
            additional_properties: false,
            default_key: None,
        }
    }

    pub const fn additional_properties(mut self, additional_properties: bool) -> Self {
        self.additional_properties = additional_properties;
        self
    }

    pub const fn default_key(mut self, key: &'static str) -> Self {
        self.default_key = Some(key);
        self
    }

    pub const fn schema(self) -> Schema {
        Schema::Object(self)
    }

    pub fn lookup(&self, key: &str) -> Option<(bool, &Schema)> {
        if let Ok(ind) = self.properties.binary_search_by_key(&key, |(name, _, _)| name) {
            let (_name, optional, prop_schema) = self.properties[ind];
            Some((optional, prop_schema))
        } else {
            None
        }
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

pub enum ApiStringFormat {
    Enum(&'static [&'static str]),
    Pattern(&'static ConstRegexPattern),
    Complex(&'static Schema),
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
                } else if let Some(key) = object_schema.default_key {
                    param_list.push((key.into(), kv[0].into()));
                } else {
                    bail!("Value without key, but schema does not define a default key.");
                }
            }

            parse_parameter_strings(&param_list, &object_schema, true)
                .map_err(Error::from)

        }
        Schema::Array(array_schema) => {
            let mut array : Vec<Value> = vec![];
            for value in value_str.split(',').filter(|s| !s.is_empty()) {
                match parse_simple_value(value, &array_schema.items) {
                    Ok(res) => array.push(res),
                    Err(err) => bail!("unable to parse array element: {}", err),
                }
            }
            array_schema.check_length(array.len())?;

            Ok(array.into())
        }
        _ => {
            bail!("Got unexpetec schema type.")
        }
    }
}

pub fn parse_simple_value(value_str: &str, schema: &Schema) -> Result<Value, Error> {

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
            integer_schema.check_constraints(res)?;
            Value::Number(res.into())
        }
        Schema::String(string_schema) => {
            string_schema.check_constraints(value_str)?;
            Value::String(value_str.into())
        }
        _ => bail!("unable to parse complex (sub) objects."),
    };
    Ok(value)
}

pub fn parse_parameter_strings(data: &[(String, String)], schema: &ObjectSchema, test_required: bool) -> Result<Value, ParameterError> {

    let mut params = json!({});

    let mut errors = ParameterError::new();

    let additional_properties = schema.additional_properties;

    for (key, value) in data {
        if let Some((_optional, prop_schema)) = schema.lookup(&key) {
            match prop_schema {
                Schema::Array(array_schema) => {
                    if params[key] == Value::Null {
                        params[key] = json!([]);
                    }
                    match params[key] {
                        Value::Array(ref mut array) => {
                            match parse_simple_value(value, &array_schema.items) {
                                Ok(res) => array.push(res), // fixme: check_length??
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
        } else if additional_properties {
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

    if test_required && errors.len() == 0 {
        for (name, optional, _prop_schema) in schema.properties {
            if !(*optional) && params[name] == Value::Null {
                errors.push(format_err!("parameter '{}': parameter is missing and it is not optional.", name));
            }
        }
    }

    if !errors.is_empty() {
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

pub fn verify_json(data: &Value, schema: &Schema) -> Result<(), Error> {

    match schema {
        Schema::Object(object_schema) => {
            verify_json_object(data, &object_schema)?;
        }
        Schema::Array(array_schema) => {
            verify_json_array(data, &array_schema)?;
        }
        Schema::Null => {
            if !data.is_null() {
                bail!("Expected Null, but value is not Null.");
            }
        }
        Schema::Boolean(boolean_schema) => verify_json_boolean(data, &boolean_schema)?,
        Schema::Integer(integer_schema) => verify_json_integer(data, &integer_schema)?,
        Schema::String(string_schema) => verify_json_string(data, &string_schema)?,
    }
    Ok(())
}

pub fn verify_json_string(data: &Value, schema: &StringSchema) -> Result<(), Error> {
    if let Some(value) = data.as_str() {
        schema.check_constraints(value)
    } else {
        bail!("Expected string value.");
    }
}

pub fn verify_json_boolean(data: &Value, _schema: &BooleanSchema) -> Result<(), Error> {
    if !data.is_boolean() {
        bail!("Expected boolean value.");
    }
    Ok(())
}

pub fn verify_json_integer(data: &Value, schema: &IntegerSchema) -> Result<(), Error> {
    if let Some(value) = data.as_i64() {
        schema.check_constraints(value as isize)
    } else {
        bail!("Expected integer value.");
    }
}

pub fn verify_json_array(data: &Value, schema: &ArraySchema) -> Result<(), Error> {

    let list = match data {
        Value::Array(ref list) => list,
        Value::Object(_) => bail!("Expected array - got object."),
        _ => bail!("Expected array - got scalar value."),
    };

    schema.check_length(list.len())?;

    for item in list {
        verify_json(item, &schema.items)?;
    }

    Ok(())
}

pub fn verify_json_object(data: &Value, schema: &ObjectSchema) -> Result<(), Error> {

    let map = match data {
        Value::Object(ref map) => map,
        Value::Array(_) => bail!("Expected object - got array."),
        _ => bail!("Expected object - got scalar value."),
    };

    let additional_properties = schema.additional_properties;

    for (key, value) in map {
        if let Some((_optional, prop_schema)) = schema.lookup(&key) {
            match prop_schema {
                Schema::Object(object_schema) => {
                    verify_json_object(value, object_schema)?;
                }
                Schema::Array(array_schema) => {
                    verify_json_array(value, array_schema)?;
                }
                _ => verify_json(value, prop_schema)?,
            }
        } else if !additional_properties {
            bail!("property '{}': schema does not allow additional properties.", key);
        }
    }

    for (name, optional, _prop_schema) in schema.properties {
        if !(*optional) && data[name] == Value::Null {
            bail!("property '{}': property is missing and it is not optional.", name);
        }
    }

    Ok(())
}

#[test]
fn test_schema1() {
    let schema = Schema::Object(ObjectSchema {
        description: "TEST",
        additional_properties: false,
        properties: &[],
        default_key: None,
    });

    println!("TEST Schema: {:?}", schema);
}

#[test]
fn test_query_string() {

    {
        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[("name", false, &StringSchema::new("Name.").schema())]
        );

        let res = parse_query_string("", &SCHEMA, true);
        assert!(res.is_err());
    }

    {
        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[("name", true, &StringSchema::new("Name.").schema())]
        );
    
        let res = parse_query_string("", &SCHEMA, true);
        assert!(res.is_ok());
    }
    
    // TEST min_length and max_length
    {
        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[
                ("name", true, &StringSchema::new("Name.")
                 .min_length(5)
                 .max_length(10)
                 .schema()
                ),
            ]);

        let res = parse_query_string("name=abcd", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("name=abcde", &SCHEMA, true);
        assert!(res.is_ok());

        let res = parse_query_string("name=abcdefghijk", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("name=abcdefghij", &SCHEMA, true);
        assert!(res.is_ok());
    }
    
    // TEST regex pattern
    const_regex! {
        TEST_REGEX = "test";
        TEST2_REGEX = "^test$";
    }

    {
        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[
                ("name", false, &StringSchema::new("Name.")
                 .format(&ApiStringFormat::Pattern(&TEST_REGEX))
                 .schema()
                ),
            ]);
        
        let res = parse_query_string("name=abcd", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("name=ateststring", &SCHEMA, true);
        assert!(res.is_ok());
    }

    {
        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[
                ("name", false, &StringSchema::new("Name.")
                 .format(&ApiStringFormat::Pattern(&TEST2_REGEX))
                 .schema()
                ),
            ]);

        let res = parse_query_string("name=ateststring", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("name=test", &SCHEMA, true);
        assert!(res.is_ok());
    }
    
    // TEST string enums
    {
        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[
                ("name", false, &StringSchema::new("Name.")
                 .format(&ApiStringFormat::Enum(&["ev1", "ev2"]))
                 .schema()
                ),
            ]);

        let res = parse_query_string("name=noenum", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("name=ev1", &SCHEMA, true);
        assert!(res.is_ok());

        let res = parse_query_string("name=ev2", &SCHEMA, true);
        assert!(res.is_ok());

        let res = parse_query_string("name=ev3", &SCHEMA, true);
        assert!(res.is_err());
    }
}

#[test]
fn test_query_integer() {

    {
        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[
                ("count", false, &IntegerSchema::new("Count.").schema()),
            ]);

        let res = parse_query_string("", &SCHEMA, true);
        assert!(res.is_err());
    }

    {
        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[
                ("count", true, &IntegerSchema::new("Count.")
                 .minimum(-3)
                 .maximum(50)
                 .schema()
                ),
            ]);
        
        let res = parse_query_string("", &SCHEMA, true);
        assert!(res.is_ok());

        let res = parse_query_string("count=abc", &SCHEMA, false);
        assert!(res.is_err());

        let res = parse_query_string("count=30", &SCHEMA, false);
        assert!(res.is_ok());

        let res = parse_query_string("count=-1", &SCHEMA, false);
        assert!(res.is_ok());

        let res = parse_query_string("count=300", &SCHEMA, false);
        assert!(res.is_err());

        let res = parse_query_string("count=-30", &SCHEMA, false);
        assert!(res.is_err());

        let res = parse_query_string("count=50", &SCHEMA, false);
        assert!(res.is_ok());

        let res = parse_query_string("count=-3", &SCHEMA, false);
        assert!(res.is_ok());
    }
}

#[test]
fn test_query_boolean() {

    {
        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[
                ("force", false, &BooleanSchema::new("Force.").schema()),
            ]);

        let res = parse_query_string("", &SCHEMA, true);
        assert!(res.is_err());
    }

    {
        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[
                ("force", true, &BooleanSchema::new("Force.").schema()),
            ]);
    
        let res = parse_query_string("", &SCHEMA, true);
        assert!(res.is_ok());

        let res = parse_query_string("a=b", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("force", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("force=yes", &SCHEMA, true);
        assert!(res.is_ok());
        let res = parse_query_string("force=1", &SCHEMA, true);
        assert!(res.is_ok());
        let res = parse_query_string("force=On", &SCHEMA, true);
        assert!(res.is_ok());
        let res = parse_query_string("force=TRUE", &SCHEMA, true);
        assert!(res.is_ok());
        let res = parse_query_string("force=TREU", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("force=NO", &SCHEMA, true);
        assert!(res.is_ok());
        let res = parse_query_string("force=0", &SCHEMA, true);
        assert!(res.is_ok());
        let res = parse_query_string("force=off", &SCHEMA, true);
        assert!(res.is_ok());
        let res = parse_query_string("force=False", &SCHEMA, true);
        assert!(res.is_ok());
    }
}

#[test]
fn test_verify_function() {

    const SCHEMA: ObjectSchema = ObjectSchema::new(
        "Parameters.",
        &[
            ("p1", false, &StringSchema::new("P1")
             .format(&ApiStringFormat::VerifyFn(|value| {
                 if value == "test" { return Ok(()) };
                 bail!("format error");
             }))
             .schema()
            ),
        ]);

    let res = parse_query_string("p1=tes", &SCHEMA, true);
    assert!(res.is_err());
    let res = parse_query_string("p1=test", &SCHEMA, true);
    assert!(res.is_ok());
}

#[test]
fn test_verify_complex_object() {

    const NIC_MODELS: ApiStringFormat = ApiStringFormat::Enum(&["e1000", "virtio"]);

    const PARAM_SCHEMA: Schema = ObjectSchema::new(
        "Properties.",
        &[
            ("enable", true, &BooleanSchema::new("Enable device.").schema()),
            ("model", false, &StringSchema::new("Ethernet device Model.")
             .format(&NIC_MODELS)
             .schema()
            ),
         ])
        .default_key("model")
        .schema();

    const SCHEMA: ObjectSchema = ObjectSchema::new(
        "Parameters.",
        &[
            ("net0", false, &StringSchema::new("First Network device.")
             .format(&ApiStringFormat::Complex(&PARAM_SCHEMA))
             .schema()
            ),
        ]);

    let res = parse_query_string("", &SCHEMA, true);
    assert!(res.is_err());

    let res = parse_query_string("test=abc", &SCHEMA, true);
    assert!(res.is_err());

    let res = parse_query_string("net0=model=abc", &SCHEMA, true);
    assert!(res.is_err());

    let res = parse_query_string("net0=model=virtio", &SCHEMA, true);
    assert!(res.is_ok());

    let res = parse_query_string("net0=model=virtio,enable=1", &SCHEMA, true);
    assert!(res.is_ok());

    let res = parse_query_string("net0=virtio,enable=no", &SCHEMA, true);
    assert!(res.is_ok());
}

#[test]
fn test_verify_complex_array() {

    {
        const PARAM_SCHEMA: Schema = ArraySchema::new(
            "Integer List.", &IntegerSchema::new("Soemething").schema())
            .schema();

        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[
                ("list", false, &StringSchema::new("A list on integers, comma separated.")
                 .format(&ApiStringFormat::Complex(&PARAM_SCHEMA))
                 .schema()
                ),
            ]);

        let res = parse_query_string("", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("list=", &SCHEMA, true);
        assert!(res.is_ok());

        let res = parse_query_string("list=abc", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("list=1", &SCHEMA, true);
        assert!(res.is_ok());

        let res = parse_query_string("list=2,3,4,5", &SCHEMA, true);
        assert!(res.is_ok());
    }

    {

        const PARAM_SCHEMA: Schema = ArraySchema::new(
            "Integer List.", &IntegerSchema::new("Soemething").schema())
            .min_length(1)
            .max_length(3)
            .schema();

        const SCHEMA: ObjectSchema = ObjectSchema::new(
            "Parameters.",
            &[
                ("list", false, &StringSchema::new("A list on integers, comma separated.")
                 .format(&ApiStringFormat::Complex(&PARAM_SCHEMA))
                 .schema()
                ),
            ]);

        let res = parse_query_string("list=", &SCHEMA, true);
        assert!(res.is_err());

        let res = parse_query_string("list=1,2,3", &SCHEMA, true);
        assert!(res.is_ok());

        let res = parse_query_string("list=2,3,4,5", &SCHEMA, true);
        assert!(res.is_err());
    }
}
