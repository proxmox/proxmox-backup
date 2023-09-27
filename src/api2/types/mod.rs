//! API Type Definitions

use anyhow::bail;

use proxmox_schema::*;

mod acme;
pub use acme::*;

// File names: may not contain slashes, may not start with "."
pub const FILENAME_FORMAT: ApiStringFormat = ApiStringFormat::VerifyFn(|name| {
    if name.starts_with('.') {
        bail!("file names may not start with '.'");
    }
    if name.contains('/') {
        bail!("file names may not contain slashes");
    }
    Ok(())
});

// Regression tests

#[test]
fn test_cert_fingerprint_schema() -> Result<(), anyhow::Error> {
    let schema = pbs_api_types::CERT_FINGERPRINT_SHA256_SCHEMA;

    let invalid_fingerprints = [
        "86:88:7c:be:26:77:a5:62:67:d9:06:f5:e4::61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
        "88:7C:BE:26:77:a5:62:67:D9:06:f5:e4:14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
        "86:88:7c:be:26:77:a5:62:67:d9:06:f5:e4::14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8:ff",
        "XX:88:7c:be:26:77:a5:62:67:d9:06:f5:e4::14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
        "86:88:Y4:be:26:77:a5:62:67:d9:06:f5:e4:14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
        "86:88:0:be:26:77:a5:62:67:d9:06:f5:e4:14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
    ];

    for fingerprint in invalid_fingerprints.iter() {
        if schema.parse_simple_value(fingerprint).is_ok() {
            bail!(
                "test fingerprint '{}' failed -  got Ok() while exception an error.",
                fingerprint
            );
        }
    }

    let valid_fingerprints = [
        "86:88:7c:be:26:77:a5:62:67:d9:06:f5:e4:14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
        "86:88:7C:BE:26:77:a5:62:67:D9:06:f5:e4:14:61:3e:20:dc:cd:43:92:07:7f:fb:65:54:6c:ff:d2:96:36:f8",
    ];

    for fingerprint in valid_fingerprints.iter() {
        let v = match schema.parse_simple_value(fingerprint) {
            Ok(v) => v,
            Err(err) => {
                bail!("unable to parse fingerprint '{}' - {}", fingerprint, err);
            }
        };

        if v != serde_json::json!(fingerprint) {
            bail!(
                "unable to parse fingerprint '{}' - got wrong value {:?}",
                fingerprint,
                v
            );
        }
    }

    Ok(())
}

#[test]
fn test_proxmox_user_id_schema() -> Result<(), anyhow::Error> {
    use pbs_api_types::Userid;

    let invalid_user_ids = [
        "x",                                                                 // too short
        "xx",                                                                // too short
        "xxx",                                                               // no realm
        "xxx@",                                                              // no realm
        "xx x@test",                                                         // contains space
        "xx\nx@test", // contains control character
        "x:xx@test",  // contains collon
        "xx/x@test",  // contains slash
        "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx@test", // too long
    ];

    for name in invalid_user_ids.iter() {
        if Userid::API_SCHEMA.parse_simple_value(name).is_ok() {
            bail!(
                "test userid '{}' failed -  got Ok() while exception an error.",
                name
            );
        }
    }

    let valid_user_ids = [
        "xxx@y",
        "name@y",
        "xxx@test-it.com",
        "xxx@_T_E_S_T-it.com",
        "x_x-x.x@test-it.com",
    ];

    for name in valid_user_ids.iter() {
        let v = match Userid::API_SCHEMA.parse_simple_value(name) {
            Ok(v) => v,
            Err(err) => {
                bail!("unable to parse userid '{}' - {}", name, err);
            }
        };

        if v != serde_json::json!(name) {
            bail!(
                "unable to parse userid '{}' - got wrong value {:?}",
                name,
                v
            );
        }
    }

    Ok(())
}

pub const HTTP_PROXY_SCHEMA: Schema =
    StringSchema::new("HTTP proxy configuration [http://]<host>[:port]")
        .format(&ApiStringFormat::VerifyFn(|s| {
            proxmox_http::ProxyConfig::parse_proxy_url(s)?;
            Ok(())
        }))
        .min_length(1)
        .max_length(128)
        .type_text("[http://]<host>[:port]")
        .schema();
