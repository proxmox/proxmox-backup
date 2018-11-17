use crate::api::schema::*;

use failure::*;
use serde_json::{json, Value};

#[derive(Debug)]
enum RawArgument {
    Separator,
    Argument { value: String },
    Option { name: String, value: Option<String> },
}

fn parse_argument(arg: &str) -> RawArgument {

    let chars: Vec<char> = arg.chars().collect();

    let length = chars.len();

    if length >= 2 && chars[0] == '-' &&  chars[1] == '-' {

        if length == 2 { return RawArgument::Separator; }

        for start in 2..length  {
            if chars[start] == '=' {
                let name: String = chars[2..start].iter().collect();
                let value: String = chars[start+1..length].iter().collect();
                return RawArgument::Option { name, value: Some(value) }
            }
        }

        let name: String = chars[2..].iter().collect();
        return RawArgument::Option { name: name, value: None }

    }

    RawArgument::Argument { value: arg.to_string() }
}

pub fn parse_arguments(
    args: &Vec<String>,
    schema: &Schema,
) -> Result<(Value,Vec<String>), ParameterError> {

    let mut errors = ParameterError::new();

    let properties = match schema {
        Schema::Object(ObjectSchema { properties, .. }) => properties,
        _ => {
            errors.push(format_err!("parse arguments failed - got strange parameters (expected object schema)."));
            return Err(errors);
        },
    };

    let mut data: Vec<(String, String)> = vec![];
    let mut rest: Vec<String> = vec![];

    let mut pos = 0;

    let mut skip = false;

    loop {
        if skip {
            rest.push(args[pos].clone());
        } else {
            match parse_argument(&args[pos]) {
                RawArgument::Separator => {
                    skip = true;
                }
                RawArgument::Option { name, value } => {
                    match value {
                        None => {
                            if pos < args.len() {
                                if let RawArgument::Argument { value: next } = parse_argument(&args[pos+1]) {
                                    pos += 1;
                                    data.push((name, next));
                                } else {
                                    if let Some(Schema::Boolean(boolean_schema)) = properties.get::<str>(&name) {
                                        if let Some(default) = boolean_schema.default {
                                            if default == false {
                                                data.push((name, "true".to_string()));
                                            } else {
                                                errors.push(format_err!("parameter '{}': {}", name,
                                                                        "boolean requires argument."));
                                            }
                                        } else {
                                            data.push((name, "true".to_string()));
                                        }
                                    }
                                }
                            }
                        }
                        Some(v) => {
                            data.push((name, v));
                        }
                    }
                }
                RawArgument::Argument { value } => {
                    rest.push(value);
                }
            }
        }

        pos += 1;
        if pos >= args.len() { break; }
    }

    if errors.len() > 0 { return Err(errors); }

    let options = parse_parameter_strings(&data, schema, true)?;

    Ok((options,rest))
}
