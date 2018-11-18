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

        if chars[0] == '-' {
            let mut first = 1;

            if chars[1] == '-' {
                if length == 2 { return RawArgument::Separator; }
                first = 2;
           }

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
    arg_param: &Vec<String>,
    schema: &ObjectSchema,
) -> Result<(Value,Vec<String>), ParameterError> {

    let mut errors = ParameterError::new();

    let properties = &schema.properties;

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

                            let mut next_is_argument = false;
                            let mut next_is_bool = false;

                            if (pos + 1) < args.len() {
                                let next = &args[pos+1];
                                 if let RawArgument::Argument { value: _} = parse_argument(next) {
                                    next_is_argument = true;
                                    if let Ok(_) = parse_boolean(next) { next_is_bool = true; }
                                }
                            }

                            if want_bool {
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

                                if next_is_argument {
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

    for i in 0..arg_param.len() {
        if rest.len() > i {
            data.push((arg_param[i].clone(), rest[i].clone()));
        } else {
            errors.push(format_err!("missing argument '{}'", arg_param[i]));
        }
    }

    if errors.len() > 0 { return Err(errors); }

    if arg_param.len() > 0 {
        rest = rest[arg_param.len()..].to_vec();
    }

    let options = parse_parameter_strings(&data, schema, true)?;

    Ok((options,rest))
}


#[test]
fn test_boolean_arg() {

    let schema = parameter!{enable => Boolean!{ optional => false }};

    let mut variants: Vec<(Vec<&str>, bool)> = vec![];
    variants.push((vec!["-enable"], true));
    variants.push((vec!["-enable=1"], true));
    variants.push((vec!["-enable", "yes"], true));
    variants.push((vec!["-enable", "Yes"], true));
    variants.push((vec!["--enable", "1"], true));
    variants.push((vec!["--enable", "ON"], true));
    variants.push((vec!["--enable", "true"], true));

    variants.push((vec!["--enable", "0"], false));
    variants.push((vec!["--enable", "no"], false));
    variants.push((vec!["--enable", "off"], false));
    variants.push((vec!["--enable", "false"], false));

    for (args, expect) in variants {
        let string_args = args.iter().map(|s| s.to_string()).collect();
        let res = parse_arguments(&string_args, &vec![], &schema);
        assert!(res.is_ok());
        if let Ok((options, rest)) = res {
            assert!(options["enable"] == expect);
            assert!(rest.len() == 0);
        }
    }
}

#[test]
fn test_argument_paramenter() {

    let schema = parameter!{
        enable => Boolean!{ optional => false },
        storage => ApiString!{ optional => false }
    };

    let args = vec!["-enable", "local"];
    let string_args = args.iter().map(|s| s.to_string()).collect();
    let res = parse_arguments(&string_args, &vec!["storage".to_string()], &schema);
    assert!(res.is_ok());
    if let Ok((options, rest)) = res {
        assert!(options["enable"] == true);
        assert!(options["storage"] == "local");
        assert!(rest.len() == 0);
    }
}
