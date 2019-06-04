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
