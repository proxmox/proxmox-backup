use anyhow::{bail, format_err, Error};
use serde_json::Value;
pub fn json_object_to_query(data: Value) -> Result<String, Error> {
    let mut query = url::form_urlencoded::Serializer::new(String::new());

    let object = data.as_object().ok_or_else(|| {
        format_err!("json_object_to_query: got wrong data type (expected object).")
    })?;

    for (key, value) in object {
        match value {
            Value::Bool(b) => {
                query.append_pair(key, &b.to_string());
            }
            Value::Number(n) => {
                query.append_pair(key, &n.to_string());
            }
            Value::String(s) => {
                query.append_pair(key, s);
            }
            Value::Array(arr) => {
                for element in arr {
                    match element {
                        Value::Bool(b) => {
                            query.append_pair(key, &b.to_string());
                        }
                        Value::Number(n) => {
                            query.append_pair(key, &n.to_string());
                        }
                        Value::String(s) => {
                            query.append_pair(key, s);
                        }
                        _ => bail!(
                            "json_object_to_query: unable to handle complex array data types."
                        ),
                    }
                }
            }
            _ => bail!("json_object_to_query: unable to handle complex data types."),
        }
    }

    Ok(query.finish())
}

pub fn required_string_param<'a>(param: &'a Value, name: &str) -> Result<&'a str, Error> {
    match param[name].as_str() {
        Some(s) => Ok(s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_string_property<'a>(param: &'a Value, name: &str) -> Result<&'a str, Error> {
    match param[name].as_str() {
        Some(s) => Ok(s),
        None => bail!("missing property '{}'", name),
    }
}

pub fn required_integer_param(param: &Value, name: &str) -> Result<i64, Error> {
    match param[name].as_i64() {
        Some(s) => Ok(s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_integer_property(param: &Value, name: &str) -> Result<i64, Error> {
    match param[name].as_i64() {
        Some(s) => Ok(s),
        None => bail!("missing property '{}'", name),
    }
}

pub fn required_array_param<'a>(param: &'a Value, name: &str) -> Result<&'a [Value], Error> {
    match param[name].as_array() {
        Some(s) => Ok(s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_array_property<'a>(param: &'a Value, name: &str) -> Result<&'a [Value], Error> {
    match param[name].as_array() {
        Some(s) => Ok(s),
        None => bail!("missing property '{}'", name),
    }
}
