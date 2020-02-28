use failure::*;
use serde_json::Value;
use chrono::{Local, TimeZone};

pub fn strip_server_file_expenstion(name: &str) -> String {

    if name.ends_with(".didx") || name.ends_with(".fidx") || name.ends_with(".blob") {
        name[..name.len()-5].to_owned()
    } else {
        name.to_owned() // should not happen
    }
}

pub fn render_backup_file_list(files: &[String]) -> String {
    let mut files: Vec<String> = files.iter()
        .map(|v| strip_server_file_expenstion(&v))
        .collect();

    files.sort();

    super::join(&files, ' ')
}

pub fn render_epoch(value: &Value, _record: &Value) -> Result<String, Error> {
    if value.is_null() { return Ok(String::new()); }
    let text = match value.as_i64() {
        Some(epoch) => {
            Local.timestamp(epoch, 0).format("%c").to_string()
        }
        None => {
            value.to_string()
        }
    };
    Ok(text)
}

pub fn render_task_status(value: &Value, record: &Value) -> Result<String, Error> {
    if record["endtime"].is_null() {
        Ok(value.as_str().unwrap_or("running").to_string())
    } else {
        Ok(value.as_str().unwrap_or("unknown").to_string())
    }
}
