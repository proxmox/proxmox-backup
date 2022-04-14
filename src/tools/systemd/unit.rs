use std::process::Command;

use anyhow::{bail, format_err, Error};

fn run_command(mut command: Command) -> Result<(), Error> {
    let output = command
        .output()
        .map_err(|err| format_err!("failed to execute {:?} - {}", command, err))?;

    proxmox_lang::try_block!({
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
                            .unwrap_or_else(|_| {
                                String::from("non utf8 error message (suppressed)")
                            });

                        bail!("status code: {} - {}", code, msg);
                    }
                }
                None => bail!("terminated by signal"),
            }
        }
        Ok(())
    })
    .map_err(|err| format_err!("command {:?} failed - {}", command, err))?;

    Ok(())
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
        use proxmox_sys::systemd::{escape_unit, unescape_unit};

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
