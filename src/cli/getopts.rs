use crate::api_schema::*;

use failure::*;

use serde_json::Value;

#[derive(Debug)]
enum RawArgument {
    Separator,
    Argument { value: String },
    Option { name: String, value: Option<String> },
}

fn parse_argument(arg: &str) -> RawArgument {
    let bytes = arg.as_bytes();

    let length = bytes.len();

    if length < 2 || bytes[0] != b'-' {
        return RawArgument::Argument {
            value: arg.to_string(),
        };
    }

    let mut first = 1;

    if bytes[1] == b'-' {
        if length == 2 {
            return RawArgument::Separator;
        }
        first = 2;
    }

    for start in first..length {
        if bytes[start] == b'=' {
            // Since we take a &str, we know the contents of it are valid utf8.
            // Since bytes[start] == b'=', we know the byte beginning at start is a single-byte
            // code pointer. We also know that 'first' points exactly after a single-byte code
            // point as it points to the first byte after a hyphen.
            // Therefore we know arg[first..start] is valid utf-8, therefore it is safe to use
            // get_unchecked() to speed things up.
            return RawArgument::Option {
                name: unsafe { arg.get_unchecked(first..start).to_string() },
                value: Some(unsafe { arg.get_unchecked((start + 1)..).to_string() }),
            };
        }
    }

    RawArgument::Option {
        name: unsafe { arg.get_unchecked(first..).to_string() },
        value: None,
    }
}

/// parse as many arguments as possible into a Vec<String, String>. This does not
/// verify the schema.
/// Returns parsed data and the rest as separate array
pub (crate) fn parse_argument_list<T: AsRef<str>>(
    args: &[T],
    schema: &ObjectSchema,
    errors: &mut ParameterError,
) -> (Vec<(String, String)>, Vec<String>) {

    let mut data: Vec<(String, String)> = vec![];
    let mut rest: Vec<String> = vec![];

    let mut pos = 0;

    let properties = &schema.properties;

    while pos < args.len() {
        match parse_argument(args[pos].as_ref()) {
            RawArgument::Separator => {
                break;
            }
            RawArgument::Option { name, value } => match value {
                None => {
                    let mut want_bool = false;
                    let mut can_default = false;
                    if let Some((_optional, param_schema)) = properties.get::<str>(&name) {
                        if let Schema::Boolean(boolean_schema) = param_schema.as_ref() {
                            want_bool = true;
                            if let Some(default) = boolean_schema.default {
                                if default == false {
                                    can_default = true;
                                }
                            } else {
                                can_default = true;
                            }
                        }
                    }

                    let mut next_is_argument = false;
                    let mut next_is_bool = false;

                    if (pos + 1) < args.len() {
                        let next = args[pos + 1].as_ref();
                        if let RawArgument::Argument { .. } = parse_argument(next) {
                            next_is_argument = true;
                            if let Ok(_) = parse_boolean(next) {
                                next_is_bool = true;
                            }
                        }
                    }

                    if want_bool {
                        if next_is_bool {
                            pos += 1;
                            data.push((name, args[pos].as_ref().to_string()));
                        } else if can_default {
                            data.push((name, "true".to_string()));
                        } else {
                            errors.push(format_err!("parameter '{}': {}", name,
                                                    "missing boolean value."));
                        }

                    } else if next_is_argument {
                        pos += 1;
                        data.push((name, args[pos].as_ref().to_string()));
                    } else {
                        errors.push(format_err!("parameter '{}': {}", name,
                                                "missing parameter value."));
                    }
                }
                Some(v) => {
                    data.push((name, v));
                }
            },
            RawArgument::Argument { value } => {
                rest.push(value);
            }
        }

        pos += 1;
    }

    rest.reserve(args.len() - pos);
    for i in &args[pos..] {
        rest.push(i.as_ref().to_string());
    }

    (data, rest)
}

/// Parses command line arguments using a `Schema`
///
/// Returns parsed options as json object, together with the
/// list of additional command line arguments.
pub fn parse_arguments<T: AsRef<str>>(
    args: &[T],
    arg_param: &Vec<&'static str>,
    schema: &ObjectSchema,
) -> Result<(Value, Vec<String>), ParameterError> {
    let mut errors = ParameterError::new();

    let properties = &schema.properties;

    // first check if all arg_param exists in schema

    let mut last_arg_param_is_optional = false;
    let mut last_arg_param_is_array = false;

    for i in 0..arg_param.len() {
        let name = arg_param[i];
        if let Some((optional, param_schema)) = properties.get::<str>(&name) {
            if i == arg_param.len() -1 {
                last_arg_param_is_optional = *optional;
                if let Schema::Array(_) = param_schema.as_ref() {
                    last_arg_param_is_array = true;
                }
            } else if *optional {
                panic!("positional argument '{}' may not be optional", name);
            }
        } else {
            panic!("no such property '{}' in schema", name);
        }
    }

    let (mut data, mut rest) = parse_argument_list(args, schema, &mut errors);

    for i in 0..arg_param.len() {

        let name = arg_param[i];
        let is_last_arg_param = i == (arg_param.len() - 1);

        if rest.len() == 0 {
            if !(is_last_arg_param && last_arg_param_is_optional) {
                errors.push(format_err!("missing argument '{}'", name));
            }
        } else if is_last_arg_param && last_arg_param_is_array {
            for value in rest {
                data.push((name.to_string(), value));
            }
            rest = vec![];
        } else {
            data.push((name.to_string(), rest.remove(0)));
        }
    }

    if errors.len() > 0 {
        return Err(errors);
    }

    let options = parse_parameter_strings(&data, schema, true)?;

    Ok((options, rest))
}

#[test]
fn test_boolean_arg() {
    let schema =  ObjectSchema::new("Parameters:")
        .required(
            "enable", BooleanSchema::new("Enable")
        );

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
        let res = parse_arguments(&args, &vec![], &schema);
        assert!(res.is_ok());
        if let Ok((options, rest)) = res {
            assert!(options["enable"] == expect);
            assert!(rest.len() == 0);
        }
    }
}

#[test]
fn test_argument_paramenter() {
    let schema = ObjectSchema::new("Parameters:")
        .required("enable", BooleanSchema::new("Enable."))
        .required("storage", StringSchema::new("Storage."));

    let args = vec!["-enable", "local"];
    let res = parse_arguments(&args, &vec!["storage"], &schema);
    assert!(res.is_ok());
    if let Ok((options, rest)) = res {
        assert!(options["enable"] == true);
        assert!(options["storage"] == "local");
        assert!(rest.len() == 0);
    }
}
