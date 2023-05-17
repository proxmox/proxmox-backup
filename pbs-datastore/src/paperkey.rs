use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{bail, format_err, Error};
use serde::{Deserialize, Serialize};

use proxmox_schema::api;

use pbs_key_config::KeyConfig;

#[api()]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Paperkey output format
pub enum PaperkeyFormat {
    /// Format as Utf8 text. Includes QR codes as ascii-art.
    Text,
    /// Format as Html. Includes QR codes as SVG images.
    Html,
}

/// Generate a paper key (html or utf8 text)
///
/// This function takes an encryption key (either RSA private key
/// text, or `KeyConfig` json), and generates a printable text or html
/// page, including a scanable QR code to recover the key.
pub fn generate_paper_key<W: Write>(
    output: W,
    data: &str,
    subject: Option<String>,
    output_format: Option<PaperkeyFormat>,
) -> Result<(), Error> {
    let (data, is_master_key) = if data.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----\n")
        || data.starts_with("-----BEGIN RSA PRIVATE KEY-----\n")
    {
        let data = data.trim_end();
        if !(data.ends_with("\n-----END ENCRYPTED PRIVATE KEY-----")
            || data.ends_with("\n-----END RSA PRIVATE KEY-----"))
        {
            bail!("unexpected key format");
        }

        let lines: Vec<String> = data
            .lines()
            .map(|s| s.trim_end())
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        if lines.len() < 20 {
            bail!("unexpected key format");
        }

        (lines, true)
    } else {
        match serde_json::from_str::<KeyConfig>(data) {
            Ok(key_config) => {
                let lines = serde_json::to_string_pretty(&key_config)?
                    .lines()
                    .map(String::from)
                    .collect();

                (lines, false)
            }
            Err(err) => {
                log::error!("Couldn't parse data as KeyConfig - {}", err);
                bail!("Neither a PEM-formatted private key, nor a PBS key file.");
            }
        }
    };

    let format = output_format.unwrap_or(PaperkeyFormat::Html);

    match format {
        PaperkeyFormat::Html => paperkey_html(output, &data, subject, is_master_key),
        PaperkeyFormat::Text => paperkey_text(output, &data, subject, is_master_key),
    }
}

fn paperkey_html<W: Write>(
    mut output: W,
    lines: &[String],
    subject: Option<String>,
    is_master: bool,
) -> Result<(), Error> {
    let img_size_pt = 500;

    writeln!(output, "<!DOCTYPE html>")?;
    writeln!(output, "<html lang=\"en\">")?;
    writeln!(output, "<head>")?;
    writeln!(output, "<meta charset=\"utf-8\">")?;
    writeln!(
        output,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">"
    )?;
    writeln!(output, "<title>Proxmox Backup Paperkey</title>")?;
    writeln!(output, "<style type=\"text/css\">")?;

    writeln!(output, "  p {{")?;
    writeln!(output, "    font-size: 12pt;")?;
    writeln!(output, "    font-family: monospace;")?;
    writeln!(output, "    white-space: pre-wrap;")?;
    writeln!(output, "    line-break: anywhere;")?;
    writeln!(output, "  }}")?;

    writeln!(output, "</style>")?;

    writeln!(output, "</head>")?;

    writeln!(output, "<body>")?;

    if let Some(subject) = subject {
        writeln!(output, "<p>Subject: {}</p>", subject)?;
    }

    if is_master {
        const BLOCK_SIZE: usize = 20;

        for (block_nr, block) in lines.chunks(BLOCK_SIZE).enumerate() {
            writeln!(
                output,
                "<div style=\"page-break-inside: avoid;page-break-after: always\">"
            )?;
            writeln!(output, "<p>")?;

            for (i, line) in block.iter().enumerate() {
                writeln!(output, "{:02}: {}", i + block_nr * BLOCK_SIZE, line)?;
            }

            writeln!(output, "</p>")?;

            let qr_code = generate_qr_code("svg", block)?;
            let qr_code = base64::encode_config(qr_code, base64::STANDARD_NO_PAD);

            writeln!(output, "<center>")?;
            writeln!(output, "<img")?;
            writeln!(
                output,
                "width=\"{}pt\" height=\"{}pt\"",
                img_size_pt, img_size_pt
            )?;
            writeln!(output, "src=\"data:image/svg+xml;base64,{}\"/>", qr_code)?;
            writeln!(output, "</center>")?;
            writeln!(output, "</div>")?;
        }

        writeln!(output, "</body>")?;
        writeln!(output, "</html>")?;
        return Ok(());
    }

    writeln!(output, "<div style=\"page-break-inside: avoid\">")?;

    writeln!(output, "<p>")?;

    writeln!(output, "-----BEGIN PROXMOX BACKUP KEY-----")?;

    for line in lines {
        writeln!(output, "{}", line)?;
    }

    writeln!(output, "-----END PROXMOX BACKUP KEY-----")?;

    writeln!(output, "</p>")?;

    let qr_code = generate_qr_code("svg", lines)?;
    let qr_code = base64::encode_config(qr_code, base64::STANDARD_NO_PAD);

    writeln!(output, "<center>")?;
    writeln!(output, "<img")?;
    writeln!(
        output,
        "width=\"{}pt\" height=\"{}pt\"",
        img_size_pt, img_size_pt
    )?;
    writeln!(output, "src=\"data:image/svg+xml;base64,{}\"/>", qr_code)?;
    writeln!(output, "</center>")?;

    writeln!(output, "</div>")?;

    writeln!(output, "</body>")?;
    writeln!(output, "</html>")?;

    Ok(())
}

fn paperkey_text<W: Write>(
    mut output: W,
    lines: &[String],
    subject: Option<String>,
    is_private: bool,
) -> Result<(), Error> {
    if let Some(subject) = subject {
        writeln!(output, "Subject: {}\n", subject)?;
    }

    if is_private {
        const BLOCK_SIZE: usize = 5;

        for (block_nr, block) in lines.chunks(BLOCK_SIZE).enumerate() {
            for (i, line) in block.iter().enumerate() {
                writeln!(output, "{:-2}: {}", i + block_nr * BLOCK_SIZE, line)?;
            }
            let qr_code = generate_qr_code("utf8i", block)?;
            let qr_code = String::from_utf8(qr_code)
                .map_err(|_| format_err!("Failed to read qr code (got non-utf8 data)"))?;
            writeln!(output, "{}", qr_code)?;
            writeln!(output, "{}", char::from(12u8))?; // page break
        }
        return Ok(());
    }

    writeln!(output, "-----BEGIN PROXMOX BACKUP KEY-----")?;
    for line in lines {
        writeln!(output, "{}", line)?;
    }
    writeln!(output, "-----END PROXMOX BACKUP KEY-----")?;

    let qr_code = generate_qr_code("utf8i", lines)?;
    let qr_code = String::from_utf8(qr_code)
        .map_err(|_| format_err!("Failed to read qr code (got non-utf8 data)"))?;

    writeln!(output, "{}", qr_code)?;

    Ok(())
}

fn generate_qr_code(output_type: &str, lines: &[String]) -> Result<Vec<u8>, Error> {
    let mut child = Command::new("qrencode")
        .args(["-t", output_type, "-m0", "-s1", "-lm", "--output", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| format_err!("Failed to open stdin"))?;
        let data = lines.join("\n");
        stdin
            .write_all(data.as_bytes())
            .map_err(|_| format_err!("Failed to write to stdin"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|_| format_err!("Failed to read stdout"))?;

    let output = proxmox_sys::command::command_output(output, None)?;

    Ok(output)
}
