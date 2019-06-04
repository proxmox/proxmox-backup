use crate::api_schema::*;

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

        text.push_str(descr);
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

        // fixme: wrap text
        let mut text = format!(" {:-10} {}{}", display_name, type_text, default_text);
        let indent = "             ";
        text.push('\n');
        text.push_str(indent);
        text.push_str(descr);
        text.push('\n');
        text.push('\n');

        text
    }
}
