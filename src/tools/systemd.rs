pub mod types;
pub mod config;

mod parse_time;
pub mod tm_editor;
pub mod time;

use anyhow::{bail, Error};

pub const SYSTEMCTL_BIN_PATH: &str = "/usr/bin/systemctl";

/// Escape strings for usage in systemd unit names
pub fn escape_unit(mut unit: &str, is_path: bool) -> String {

    if is_path {
        unit = unit.trim_matches('/');
        if unit.is_empty() {
            return String::from("-");
        }
    }

    let unit = unit.as_bytes();

    let mut escaped = String::new();

    for (i, c) in unit.iter().enumerate() {
        if *c == b'/' {
            escaped.push('-');
            continue;
        }
        if (i == 0 && *c == b'.') || !((*c >= b'0' && *c <= b'9') || (*c >= b'A' && *c <= b'Z') || (*c >= b'a' && *c <= b'z')) {
            escaped.push_str(&format!("\\x{:0x}", c));
        } else {
            escaped.push(*c as char);
        }
    }
    escaped
}

fn parse_hex_digit(d: u8) ->  Result<u8, Error> {
    if d >= b'0' && d <= b'9' { return Ok(d - b'0'); }
    if d >= b'A' && d <= b'F' {  return Ok(d - b'A' + 10); }
    if d >= b'a' && d <= b'f' { return Ok(d - b'a' + 10); }
    bail!("got invalid hex digit");
}

/// Unescape strings used in systemd unit names
pub fn unescape_unit(text: &str) -> Result<String, Error> {

    let mut i = text.as_bytes();

    let mut data: Vec<u8> = Vec::new();

    loop {
        if i.is_empty() { break; }
        let next = i[0];
        if next == b'\\' {
            if i.len() < 4 { bail!("short input"); }
            if i[1] != b'x' { bail!("unkwnown escape sequence"); }
            let h1 = parse_hex_digit(i[2])?;
            let h0 = parse_hex_digit(i[3])?;
            data.push(h1<<4|h0);
            i = &i[4..]
        } else if next == b'-' {
            data.push(b'/');
            i = &i[1..]
        } else {
            data.push(next);
            i = &i[1..]
        }
    }

    let text = String::from_utf8(data)?;

    Ok(text)
}

pub fn reload_daemon() -> Result<(), Error> {

    let mut command = std::process::Command::new(SYSTEMCTL_BIN_PATH);
    command.arg("daemon-reload");

    crate::tools::run_command(command, None)?;

    Ok(())
}

pub fn enable_unit(unit: &str) -> Result<(), Error> {

    let mut command = std::process::Command::new(SYSTEMCTL_BIN_PATH);
    command.arg("enable");
    command.arg(unit);

    crate::tools::run_command(command, None)?;

    Ok(())
}

pub fn start_unit(unit: &str) -> Result<(), Error> {

    let mut command = std::process::Command::new(SYSTEMCTL_BIN_PATH);
    command.arg("start");
    command.arg(unit);

    crate::tools::run_command(command, None)?;

    Ok(())
}

pub fn stop_unit(unit: &str) -> Result<(), Error> {

    let mut command = std::process::Command::new(SYSTEMCTL_BIN_PATH);
    command.arg("stop");
    command.arg(unit);

    crate::tools::run_command(command, None)?;

    Ok(())
}
