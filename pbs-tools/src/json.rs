use anyhow::{bail, Error};
use serde_json::Value;

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
