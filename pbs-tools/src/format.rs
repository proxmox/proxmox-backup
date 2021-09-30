use std::borrow::Borrow;

use anyhow::{Error};
use serde_json::Value;

pub fn strip_server_file_extension(name: &str) -> &str {
    if name.ends_with(".didx") || name.ends_with(".fidx") || name.ends_with(".blob") {
        &name[..name.len()-5]
    } else {
        name // should not happen
    }
}

pub fn render_backup_file_list<S: Borrow<str>>(files: &[S]) -> String {
    let mut files: Vec<&str> = files.iter()
        .map(|v| strip_server_file_extension(v.borrow()))
        .collect();

    files.sort();

    files.join(" ")
}

pub fn render_epoch(value: &Value, _record: &Value) -> Result<String, Error> {
    if value.is_null() { return Ok(String::new()); }
    let text = match value.as_i64() {
        Some(epoch) => {
            if let Ok(epoch_string) = proxmox::tools::time::strftime_local("%c", epoch as i64) {
                epoch_string
            } else {
                epoch.to_string()
            }
        },
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

pub fn render_bool_with_default_true(value: &Value, _record: &Value) -> Result<String, Error> {
    let value = value.as_bool().unwrap_or(true);
    Ok((if value { "1" } else { "0" }).to_string())
}

pub fn render_bytes_human_readable(value: &Value, _record: &Value) -> Result<String, Error> {
    if value.is_null() { return Ok(String::new()); }
    let text = match value.as_u64() {
        Some(bytes) => {
            HumanByte::from(bytes).to_string()
        }
        None => {
            value.to_string()
        }
    };
    Ok(text)
}

pub struct HumanByte {
    b: usize,
}
impl std::fmt::Display for HumanByte {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.b < 1024 {
            return write!(f, "{} B", self.b);
        }
        let kb: f64 = self.b as f64 / 1024.0;
        if kb < 1024.0 {
            return write!(f, "{:.2} KiB", kb);
        }
        let mb: f64 = kb / 1024.0;
        if mb < 1024.0 {
            return write!(f, "{:.2} MiB", mb);
        }
        let gb: f64 = mb / 1024.0;
        if gb < 1024.0 {
            return write!(f, "{:.2} GiB", gb);
        }
        let tb: f64 = gb / 1024.0;
        if tb < 1024.0 {
            return write!(f, "{:.2} TiB", tb);
        }
        let pb: f64 = tb / 1024.0;
        return write!(f, "{:.2} PiB", pb);
    }
}
impl From<usize> for HumanByte {
    fn from(v: usize) -> Self {
        HumanByte { b: v }
    }
}
impl From<u64> for HumanByte {
    fn from(v: u64) -> Self {
        HumanByte { b: v as usize }
    }
}

pub fn as_fingerprint(bytes: &[u8]) -> String {
    proxmox::tools::digest_to_hex(bytes)
        .as_bytes()
        .chunks(2)
        .map(|v| std::str::from_utf8(v).unwrap())
        .collect::<Vec<&str>>().join(":")
}

pub mod bytes_as_fingerprint {
    use serde::{Deserialize, Serializer, Deserializer};

    pub fn serialize<S>(
        bytes: &[u8; 32],
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = super::as_fingerprint(bytes);
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut s = String::deserialize(deserializer)?;
        s.retain(|c| c != ':');
        proxmox::tools::hex_to_digest(&s).map_err(serde::de::Error::custom)
    }
}

#[test]
fn correct_byte_convert() {
    fn convert(b: usize) -> String {
         HumanByte::from(b).to_string()
    }
    assert_eq!(convert(1023), "1023 B");
    assert_eq!(convert(1<<10), "1.00 KiB");
    assert_eq!(convert(1<<20), "1.00 MiB");
    assert_eq!(convert((1<<30) + 103 * (1<<20)), "1.10 GiB");
    assert_eq!(convert((2<<50) + 500 * (1<<40)), "2.49 PiB");
}
