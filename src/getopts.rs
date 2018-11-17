use crate::api::schema::*;

use failure::*;
use std::collections::HashMap;
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

    if length >= 2 {

        if length == 2 { return RawArgument::Separator; }

        if chars[0] == '-' {
            let first = if chars[1] == '-' { 2 } else { 1 };

            for start in first..length  {
                if chars[start] == '=' {
                    let name: String = chars[first..start].iter().collect();
                    let value: String = chars[start+1..length].iter().collect();
                    return RawArgument::Option { name, value: Some(value) }
                }
            }

            let name: String = chars[first..].iter().collect();
            return RawArgument::Option { name: name, value: None }
        }
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
                            let param_schema = properties.get::<str>(&name);
                            let (want_bool, can_default) = match param_schema {
                                Some(Schema::Boolean(boolean_schema)) => {
                                    if let Some(default) = boolean_schema.default {
                                        if default == true { (true, false); }
                                    }
                                    (true, true)
                                }
                                _ => (false, false),
                            };

                            if want_bool {

                                let mut next_is_bool = false;
                                if (pos + 1) < args.len() {
                                    let next = &args[pos+1];
                                    if let Ok(_) = parse_boolean(next) { next_is_bool = true; }
                                }

                                if next_is_bool {
                                    pos += 1;
                                    data.push((name, args[pos].clone()));
                                } else if can_default {
                                   data.push((name, "true".to_string()));
                                } else {
                                    errors.push(format_err!("parameter '{}': {}", name,
                                                            "missing boolean value."));
                                }

                            } else {

                                if (pos + 1) < args.len() {
                                    pos += 1;
                                    data.push((name, args[pos].clone()));
                                } else {
                                    errors.push(format_err!("parameter '{}': {}", name,
                                                            "missing parameter value."));
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


#[test]
fn test_boolean_arg() {

    let schema = parameter!{enable => Boolean!{ optional => false }};

    let mut variants: Vec<Vec<&str>> = vec![];
    variants.push(vec!["-enable"]);
    variants.push(vec!["-enable=1"]);
    variants.push(vec!["-enable", "yes"]);
    variants.push(vec!["--enable", "1"]);

    for args in variants {
        let string_args = args.iter().map(|s| s.to_string()).collect();
        let res = parse_arguments(&string_args, &schema);
        println!("RES: {:?}", res);
        assert!(res.is_ok());
        if let Ok((options, rest)) = res {
            assert!(options["enable"] == true);
            assert!(rest.len() == 0);
        }
    }

    //Ok((options, rest)) => {

}
