use anyhow::{bail, format_err, Error};

/// Helper to check result from std::process::Command output
///
/// The exit_code_check() function should return true if the exit code
/// is considered successful.
pub fn command_output(
    output: std::process::Output,
    exit_code_check: Option<fn(i32) -> bool>,
) -> Result<Vec<u8>, Error> {
    if !output.status.success() {
        match output.status.code() {
            Some(code) => {
                let is_ok = match exit_code_check {
                    Some(check_fn) => check_fn(code),
                    None => code == 0,
                };
                if !is_ok {
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

    Ok(output.stdout)
}

/// Helper to check result from std::process::Command output, returns String.
///
/// The exit_code_check() function should return true if the exit code
/// is considered successful.
pub fn command_output_as_string(
    output: std::process::Output,
    exit_code_check: Option<fn(i32) -> bool>,
) -> Result<String, Error> {
    let output = command_output(output, exit_code_check)?;
    let output = String::from_utf8(output)?;
    Ok(output)
}

pub fn run_command(
    mut command: std::process::Command,
    exit_code_check: Option<fn(i32) -> bool>,
) -> Result<String, Error> {
    let output = command
        .output()
        .map_err(|err| format_err!("failed to execute {:?} - {}", command, err))?;

    let output = command_output_as_string(output, exit_code_check)
        .map_err(|err| format_err!("command {:?} failed - {}", command, err))?;

    Ok(output)
}
