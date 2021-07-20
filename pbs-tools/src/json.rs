use anyhow::{bail, format_err, Error};
use serde_json::Value;

// Generate canonical json
pub fn to_canonical_json(value: &Value) -> Result<Vec<u8>, Error> {
    let mut data = Vec::new();
    write_canonical_json(value, &mut data)?;
    Ok(data)
}

pub fn write_canonical_json(value: &Value, output: &mut Vec<u8>) -> Result<(), Error> {
    match value {
        Value::Null => bail!("got unexpected null value"),
        Value::String(_) | Value::Number(_) | Value::Bool(_) => {
            serde_json::to_writer(output, &value)?;
        }
        Value::Array(list) => {
            output.push(b'[');
            let mut iter = list.iter();
            if let Some(item) = iter.next() {
                write_canonical_json(item, output)?;
                for item in iter {
                    output.push(b',');
                    write_canonical_json(item, output)?;
                }
            }
            output.push(b']');
        }
        Value::Object(map) => {
            output.push(b'{');
            let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
            keys.sort_unstable();
            let mut iter = keys.into_iter();
            if let Some(key) = iter.next() {
                serde_json::to_writer(&mut *output, &key)?;
                output.push(b':');
                write_canonical_json(&map[key], output)?;
                for key in iter {
                    output.push(b',');
                    serde_json::to_writer(&mut *output, &key)?;
                    output.push(b':');
                    write_canonical_json(&map[key], output)?;
                }
            }
            output.push(b'}');
        }
    }
    Ok(())
}

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
                query.append_pair(key, &s);
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
                            query.append_pair(key, &s);
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
        Some(s) => Ok(&s),
        None => bail!("missing parameter '{}'", name),
    }
}

pub fn required_array_property<'a>(param: &'a Value, name: &str) -> Result<&'a [Value], Error> {
    match param[name].as_array() {
        Some(s) => Ok(&s),
        None => bail!("missing property '{}'", name),
    }
}
