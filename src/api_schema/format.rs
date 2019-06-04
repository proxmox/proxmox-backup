use failure::*;

use std::io::Write;
use crate::api_schema::*;
use crate::api_schema::router::*;

#[derive(Copy, Clone)]
pub enum ParameterDisplayStyle {
    Config,
    //SonfigSub,
    Arg,
    Fixed,
}

/// CLI usage information format
#[derive(Copy, Clone, PartialEq)]
pub enum DocumentationFormat {
    /// text, command line only (one line)
    Short,
    /// text, list all options
    Long,
    /// text, include description
    Full,
    /// like full, but in reStructuredText format
    ReST,
}

/// line wrapping to form simple list of paragraphs
pub fn wrap_text(initial_indent: &str, subsequent_indent: &str, text: &str, columns: usize) -> String {

    let wrapper1 = textwrap::Wrapper::new(columns)
        .initial_indent(initial_indent)
        .subsequent_indent(subsequent_indent);

    let wrapper2 = textwrap::Wrapper::new(columns)
        .initial_indent(subsequent_indent)
        .subsequent_indent(subsequent_indent);

    text.split("\n\n")
        .map(|p| p.trim())
        .filter(|p| { p.len() != 0 })
        .fold(String::new(), |mut acc, p| {
            if acc.len() == 0 {
                acc.push_str(&wrapper1.wrap(p).concat());
            } else {
                acc.push_str(&wrapper2.wrap(p).concat());
            }
            acc.push_str("\n\n");
            acc
        })
}

pub fn get_schema_type_text(schema: &Schema, _style: ParameterDisplayStyle) -> String {

    let type_text = match schema {
        Schema::Null => String::from("<null>"), // should not happen
        Schema::String(_) => String::from("<string>"),
        Schema::Boolean(_) => String::from("<boolean>"),
        Schema::Integer(integer_schema) => {
	    match (integer_schema.minimum, integer_schema.maximum) {
		(Some(min), Some(max)) => format!("<integer> ({} - {})", min, max),
		(Some(min), None) => format!("<integer> ({} - N)", min),
		(None, Some(max)) => format!("<integer> (-N - {})", max),
		_ => String::from("<integer>"),
	    }
	},
        Schema::Object(_) => String::from("<object>"),
        Schema::Array(_) => String::from("<array>"),
    };

    type_text
}

pub fn get_property_description(
    name: &str,
    schema: &Schema,
    style: ParameterDisplayStyle,
    format: DocumentationFormat,
) -> String {

    let type_text = get_schema_type_text(schema, style);

    let (descr, default) = match schema {
        Schema::Null => ("null", None),
        Schema::String(ref schema) => (schema.description, schema.default.map(|v| v.to_owned())),
        Schema::Boolean(ref schema) => (schema.description, schema.default.map(|v| v.to_string())),
        Schema::Integer(ref schema) => (schema.description, schema.default.map(|v| v.to_string())),
        Schema::Object(ref schema) => (schema.description, None),
        Schema::Array(ref schema) => (schema.description, None),
    };

    let default_text = match default {
        Some(text) =>  format!("   (default={})", text),
        None => String::new(),
    };

    if format == DocumentationFormat::ReST {

        let mut text = match style {
            ParameterDisplayStyle::Config => {
                format!(":``{} {}{}``:  ", name, type_text, default_text)
            }
            ParameterDisplayStyle::Arg => {
                format!(":``--{} {}{}``:  ", name, type_text, default_text)
            }
            ParameterDisplayStyle::Fixed => {
                format!(":``<{}> {}{}``:  ", name, type_text, default_text)
            }
        };

        text.push_str(&wrap_text("", "", descr, 80));
        text.push('\n');
        text.push('\n');

        text

    } else {

        let display_name = match style {
            ParameterDisplayStyle::Config => {
                format!("{}:", name)
            }
            ParameterDisplayStyle::Arg => {
                format!("--{}", name)
            }
            ParameterDisplayStyle::Fixed => {
                format!("<{}>", name)
            }
        };

        let mut text = format!(" {:-10} {}{}", display_name, type_text, default_text);
        let indent = "             ";
        text.push('\n');
        text.push_str(&wrap_text(indent, indent, descr, 80));
        text.push('\n');
        text.push('\n');

        text
    }
}

fn dump_api_parameters(param: &ObjectSchema) -> String {

    let mut res = wrap_text("", "", param.description, 80);

    let properties = &param.properties;

    let mut prop_names: Vec<&str> = properties.keys().map(|v| *v).collect();
    prop_names.sort();

    let mut required_list: Vec<String> = vec![];
    let mut optional_list: Vec<String> = vec![];

    for prop in prop_names {
        let (optional, schema) = properties.get(prop).unwrap();

        let param_descr = get_property_description(
            prop, &schema, ParameterDisplayStyle::Config, DocumentationFormat::ReST);

        if *optional {
            optional_list.push(param_descr);
        } else {
            required_list.push(param_descr);
        }
    }

    if required_list.len() > 0 {

        res.push_str("\n*Required properties:*\n\n");

        for text in required_list {
            res.push_str(&text);
            res.push('\n');
        }

    }

    if optional_list.len() > 0 {

        res.push_str("\n*Optional properties:*\n\n");

        for text in optional_list {
            res.push_str(&text);
            res.push('\n');
        }
    }

    res
}

fn dump_api_return_schema(schema: &Schema) -> String {

    let mut res = String::from("*Returns*: ");

    let type_text = get_schema_type_text(schema, ParameterDisplayStyle::Config);
    res.push_str(&format!("**{}**\n\n", type_text));

    match schema {
        Schema::Null => {
            return res;
        }
        Schema::Boolean(schema) => {
            let description = wrap_text("", "", schema.description, 80);
            res.push_str(&description);
        }
        Schema::Integer(schema) => {
            let description = wrap_text("", "", schema.description, 80);
            res.push_str(&description);
        }
        Schema::String(schema) => {
            let description = wrap_text("", "", schema.description, 80);
            res.push_str(&description);
        }
        Schema::Array(schema) => {
            let description = wrap_text("", "", schema.description, 80);
            res.push_str(&description);
        }
        Schema::Object(obj_schema) => {
            res.push_str(&dump_api_parameters(obj_schema));

        }
    }

    res.push('\n');

    res
}

fn dump_method_definition(method: &str, path: &str, def: &MethodDefinition) -> Option<String> {

    match def {
        MethodDefinition::None => return None,
        MethodDefinition::Simple(simple_method) => {
            let param_descr = dump_api_parameters(&simple_method.parameters);

            let return_descr = dump_api_return_schema(&simple_method.returns);

            let res = format!("**{} {}**\n\n{}\n\n{}", method, path, param_descr, return_descr);
            return Some(res);
         }
        MethodDefinition::Async(async_method) => {
            let method = if method == "POST" { "UPLOAD" } else { method };
            let method = if method == "GET" { "DOWNLOAD" } else { method };

            let param_descr = dump_api_parameters(&async_method.parameters);

            let return_descr = dump_api_return_schema(&async_method.returns);

            let res = format!("**{} {}**\n\n{}\n\n{}", method, path, param_descr, return_descr);
            return Some(res);
        }
    }
}

pub fn dump_api(output: &mut dyn Write, router: &Router, path: &str, mut pos: usize) -> Result<(), Error> {

    let mut cond_print = |x| -> Result<_, Error> {
        if let Some(text) = x {
            if pos > 0 {
                writeln!(output, "-----\n")?;
            }
            writeln!(output, "{}", text)?;
            pos += 1;
        }
        Ok(())
    };

    cond_print(dump_method_definition("GET", path, &router.get))?;
    cond_print(dump_method_definition("POST", path, &router.post))?;
    cond_print(dump_method_definition("PUT", path, &router.put))?;
    cond_print(dump_method_definition("DELETE", path, &router.delete))?;

    match &router.subroute {
        SubRoute::None => return Ok(()),
        SubRoute::MatchAll { router, param_name } => {
            let sub_path = if path == "." {
                format!("<{}>", param_name)
            } else {
                format!("{}/<{}>", path, param_name)
            };
            dump_api(output, router, &sub_path, pos)?;
        }
        SubRoute::Hash(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_unstable_by(|a, b| a.cmp(b));
            for key in keys {
                let sub_router = &map[key];
                let sub_path = if path == "." { key.to_owned() } else { format!("{}/{}", path, key) };
                dump_api(output, sub_router, &sub_path, pos)?;
            }
        }
    }

    Ok(())
}
