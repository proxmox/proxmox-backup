use std::borrow::Borrow;

use anyhow::Error;
use serde_json::Value;

use proxmox_human_byte::HumanByte;

pub fn strip_server_file_extension(name: &str) -> &str {
    if name.ends_with(".didx") || name.ends_with(".fidx") || name.ends_with(".blob") {
        &name[..name.len() - 5]
    } else {
        name // should not happen
    }
}

pub fn render_backup_file_list<S: Borrow<str>>(files: &[S]) -> String {
    let mut files: Vec<&str> = files
        .iter()
        .map(|v| strip_server_file_extension(v.borrow()))
        .collect();

    files.sort_unstable();

    files.join(" ")
}

pub fn render_epoch(value: &Value, _record: &Value) -> Result<String, Error> {
    if value.is_null() {
        return Ok(String::new());
    }
    let text = match value.as_i64() {
        Some(epoch) => {
            if let Ok(epoch_string) = proxmox_time::strftime_local("%c", epoch) {
                epoch_string
            } else {
                epoch.to_string()
            }
        }
        None => value.to_string(),
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

pub fn render_bool_with_default_true(value: &Value, _record: &Value) -> Result<String, Error> {
    let value = value.as_bool().unwrap_or(true);
    Ok((if value { "1" } else { "0" }).to_string())
}

pub fn render_bytes_human_readable(value: &Value, _record: &Value) -> Result<String, Error> {
    if value.is_null() {
        return Ok(String::new());
    }
    let text = match value.as_u64() {
        Some(bytes) => HumanByte::from(bytes).to_string(),
        None => value.to_string(),
    };
    Ok(text)
}
