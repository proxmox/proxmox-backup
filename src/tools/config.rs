//! Our 'key: value' config format.

use std::io::Write;

use anyhow::{bail, format_err, Error};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use proxmox::api::schema::{
    parse_property_string, parse_simple_value, verify_json_object, ObjectSchemaType, Schema,
};

type Object = serde_json::Map<String, Value>;

fn object_schema(schema: &'static Schema) -> Result<&'static dyn ObjectSchemaType, Error> {
    Ok(match schema {
        Schema::Object(schema) => schema,
        Schema::AllOf(schema) => schema,
        _ => bail!("invalid schema for config, must be an object schema"),
    })
}

/// Parse a full string representing a config file.
pub fn from_str<T: for<'de> Deserialize<'de>>(
    input: &str,
    schema: &'static Schema,
) -> Result<T, Error> {
    Ok(serde_json::from_value(value_from_str(input, schema)?)?)
}

/// Parse a full string representing a config file.
pub fn value_from_str(input: &str, schema: &'static Schema) -> Result<Value, Error> {
    let schema = object_schema(schema)?;

    let mut config = Object::new();

    for (lineno, line) in input.lines().enumerate() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        parse_line(&mut config, line, schema)
            .map_err(|err| format_err!("line {}: {}", lineno, err))?;
    }

    Ok(Value::Object(config))
}

/// Parse a single `key: value` line from a config file.
fn parse_line(
    config: &mut Object,
    line: &str,
    schema: &'static dyn ObjectSchemaType,
) -> Result<(), Error> {
    if line.starts_with('#') || line.is_empty() {
        return Ok(());
    }

    let colon = line
        .find(':')
        .ok_or_else(|| format_err!("missing colon to separate key from value"))?;
    if colon == 0 {
        bail!("empty key not allowed");
    }

    let key = &line[..colon];
    let value = line[(colon + 1)..].trim_start();

    parse_key_value(config, key, value, schema)
}

/// Lookup the key in the schema, parse the value and insert it into the config object.
fn parse_key_value(
    config: &mut Object,
    key: &str,
    value: &str,
    schema: &'static dyn ObjectSchemaType,
) -> Result<(), Error> {
    let schema = match schema.lookup(key) {
        Some((_optional, schema)) => Some(schema),
        None if schema.additional_properties() => None,
        None => bail!(
            "invalid key '{}' and schema does not allow additional properties",
            key
        ),
    };

    let value = parse_value(value, schema)?;
    config.insert(key.to_owned(), value);
    Ok(())
}

/// For this we can just reuse the schema's "parse_simple_value".
///
/// "Additional" properties (`None` schema) will simply become strings.
///
/// Note that this does not handle Object or Array types at all, so if we want to support them
/// natively without going over a `String` type, we can add this here.
fn parse_value(value: &str, schema: Option<&'static Schema>) -> Result<Value, Error> {
    match schema {
        None => Ok(Value::String(value.to_owned())),
        Some(schema) => parse_simple_value(value, schema),
    }
}

/// Parse a string as a property string into a deserializable type. This is just a short wrapper
/// around deserializing the s
pub fn from_property_string<T>(input: &str, schema: &'static Schema) -> Result<T, Error>
where
    T: for<'de> Deserialize<'de>,
{
    Ok(serde_json::from_value(parse_property_string(
        input, schema,
    )?)?)
}

/// Serialize a data structure using a 'key: value' config file format.
pub fn to_bytes<T: Serialize>(value: &T, schema: &'static Schema) -> Result<Vec<u8>, Error> {
    value_to_bytes(&serde_json::to_value(value)?, schema)
}

/// Serialize a json value using a 'key: value' config file format.
pub fn value_to_bytes(value: &Value, schema: &'static Schema) -> Result<Vec<u8>, Error> {
    let schema = object_schema(schema)?;

    verify_json_object(value, schema)?;

    let object = value
        .as_object()
        .ok_or_else(|| format_err!("value must be an object"))?;

    let mut out = Vec::new();
    object_to_writer(&mut out, object)?;
    Ok(out)
}

/// Note: the object must have already been verified at this point.
fn object_to_writer(output: &mut dyn Write, object: &Object) -> Result<(), Error> {
    for (key, value) in object.iter() {
        match value {
            Value::Null => continue, // delete this entry
            Value::Bool(v) => writeln!(output, "{}: {}", key, v)?,
            Value::String(v) => writeln!(output, "{}: {}", key, v)?,
            Value::Number(v) => writeln!(output, "{}: {}", key, v)?,
            Value::Array(_) => bail!("arrays are not supported in config files"),
            Value::Object(_) => bail!("complex objects are not supported in config files"),
        }
    }
    Ok(())
}

#[test]
fn test() {
    // let's just reuse some schema we actually have available:
    use crate::config::node::NodeConfig;

    const NODE_CONFIG: &str = "\
        acme: account=pebble\n\
        acmedomain0: test1.invalid.local,plugin=power\n\
        acmedomain1: test2.invalid.local\n\
    ";

    let data: NodeConfig = from_str(NODE_CONFIG, &NodeConfig::API_SCHEMA)
        .expect("failed to parse simple node config");

    let config = to_bytes(&data, &NodeConfig::API_SCHEMA)
        .expect("failed to serialize node config");

    assert_eq!(config, NODE_CONFIG.as_bytes());
}
