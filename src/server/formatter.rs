use failure::*;
use serde_json::{json, Value};

pub struct OutputFormatter {

    pub format_result: fn(data: &Value) -> (Vec<u8>, &'static str),
}

fn json_format_result(data: &Value) -> (Vec<u8>, &'static str) {

    let content_type = "application/json;charset=UTF-8";

    let result = json!({
        "data": data
    });

    // todo: set result.total result.changes

    let json_str = result.to_string();

    let raw = json_str.into_bytes();

    (raw, content_type)
}

pub static JSON_FORMATTER: OutputFormatter = OutputFormatter {
    format_result: json_format_result,
};


fn extjs_format_result(data: &Value) -> (Vec<u8>, &'static str) {

    let content_type = "application/json;charset=UTF-8";

    let result = json!({
        "data": data,
        "success": true
    });

    // todo: set result.total result.changes

    let json_str = result.to_string();

    let raw = json_str.into_bytes();

    (raw, content_type)
}

pub static EXTJS_FORMATTER: OutputFormatter = OutputFormatter {
    format_result: extjs_format_result,
};
