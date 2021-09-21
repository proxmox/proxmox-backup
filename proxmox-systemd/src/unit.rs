use std::process::Command;

use anyhow::{bail, format_err, Error};

fn run_command(mut command: Command) -> Result<(), Error> {
    let output = command
        .output()
        .map_err(|err| format_err!("failed to execute {:?} - {}", command, err))?;

    proxmox::try_block!({
        if !output.status.success() {
            match output.status.code() {
                Some(code) => {
                    if code != 0 {
                        let msg = String::from_utf8(output.stderr)
                            .map(|m| {
                                if m.is_empty() {
                                    String::from("no error message")
                                } else {
                                    m
                                }
                            })
                            .unwrap_or_else(|_| String::from("non utf8 error message (suppressed)"));

                        bail!("status code: {} - {}", code, msg);
                    }
                }
                None => bail!("terminated by signal"),
            }
        }
        Ok(())
    }).map_err(|err| format_err!("command {:?} failed - {}", command, err))?;

    Ok(())
}

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
        if (i == 0 && *c == b'.')
            || !(*c == b'_'
                || *c == b'.'
                || (*c >= b'0' && *c <= b'9')
                || (*c >= b'A' && *c <= b'Z')
                || (*c >= b'a' && *c <= b'z'))
        {
            escaped.push_str(&format!("\\x{:0x}", c));
        } else {
            escaped.push(*c as char);
        }
    }
    escaped
}

fn parse_hex_digit(d: u8) -> Result<u8, Error> {
    if d >= b'0' && d <= b'9' {
        return Ok(d - b'0');
    }
    if d >= b'A' && d <= b'F' {
        return Ok(d - b'A' + 10);
    }
    if d >= b'a' && d <= b'f' {
        return Ok(d - b'a' + 10);
    }
    bail!("got invalid hex digit");
}

/// Unescape strings used in systemd unit names
pub fn unescape_unit(text: &str) -> Result<String, Error> {
    let mut i = text.as_bytes();

    let mut data: Vec<u8> = Vec::new();

    loop {
        if i.is_empty() {
            break;
        }
        let next = i[0];
        if next == b'\\' {
            if i.len() < 4 {
                bail!("short input");
            }
            if i[1] != b'x' {
                bail!("unkwnown escape sequence");
            }
            let h1 = parse_hex_digit(i[2])?;
            let h0 = parse_hex_digit(i[3])?;
            data.push(h1 << 4 | h0);
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
    let mut command = std::process::Command::new("systemctl");
    command.arg("daemon-reload");

    run_command(command)?;

    Ok(())
}

pub fn disable_unit(unit: &str) -> Result<(), Error> {
    let mut command = std::process::Command::new("systemctl");
    command.arg("disable");
    command.arg(unit);

    run_command(command)?;

    Ok(())
}

pub fn enable_unit(unit: &str) -> Result<(), Error> {
    let mut command = std::process::Command::new("systemctl");
    command.arg("enable");
    command.arg(unit);

    run_command(command)?;

    Ok(())
}

pub fn start_unit(unit: &str) -> Result<(), Error> {
    let mut command = std::process::Command::new("systemctl");
    command.arg("start");
    command.arg(unit);

    run_command(command)?;

    Ok(())
}

pub fn stop_unit(unit: &str) -> Result<(), Error> {
    let mut command = std::process::Command::new("systemctl");
    command.arg("stop");
    command.arg(unit);

    run_command(command)?;

    Ok(())
}

pub fn reload_unit(unit: &str) -> Result<(), Error> {
    let mut command = std::process::Command::new("systemctl");
    command.arg("try-reload-or-restart");
    command.arg(unit);

    run_command(command)?;

    Ok(())
}

#[test]
fn test_escape_unit() -> Result<(), Error> {
    fn test_escape(i: &str, expected: &str, is_path: bool) {
        let escaped = escape_unit(i, is_path);
        assert_eq!(escaped, expected);
        let unescaped = unescape_unit(&escaped).unwrap();
        if is_path {
            let mut p = i.trim_matches('/');
            if p.is_empty() {
                p = "/";
            }
            assert_eq!(p, unescaped);
        } else {
            assert_eq!(i, unescaped);
        }
    }

    test_escape(".test", "\\x2etest", false);
    test_escape("t.est", "t.est", false);
    test_escape("_test_", "_test_", false);

    test_escape("/", "-", false);
    test_escape("//", "--", false);

    test_escape("/", "-", true);
    test_escape("//", "-", true);

    Ok(())
}
