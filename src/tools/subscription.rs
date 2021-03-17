use anyhow::{Error, format_err, bail};
use lazy_static::lazy_static;
use serde_json::json;
use serde::{Deserialize, Serialize};
use regex::Regex;

use proxmox::api::api;

use crate::tools;
use crate::tools::http;
use proxmox::tools::fs::{replace_file, CreateOptions};

/// How long the local key is valid for in between remote checks
pub const MAX_LOCAL_KEY_AGE: i64 = 15 * 24 * 3600;
const MAX_KEY_CHECK_FAILURE_AGE: i64 = 5 * 24 * 3600;

const SHARED_KEY_DATA: &str = "kjfdlskfhiuewhfk947368";
const SUBSCRIPTION_FN: &str = "/etc/proxmox-backup/subscription";
const APT_AUTH_FN: &str = "/etc/apt/auth.conf.d/pbs.conf";

#[api()]
#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
/// Subscription status
pub enum SubscriptionStatus {
    // FIXME: remove?
    /// newly set subscription, not yet checked
    NEW,
    /// no subscription set
    NOTFOUND,
    /// subscription set and active
    ACTIVE,
    /// subscription set but invalid for this server
    INVALID,
}
impl Default for SubscriptionStatus {
    fn default() -> Self { SubscriptionStatus::NOTFOUND }
}
impl std::fmt::Display for SubscriptionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubscriptionStatus::NEW => write!(f, "New"),
            SubscriptionStatus::NOTFOUND => write!(f, "NotFound"),
            SubscriptionStatus::ACTIVE => write!(f, "Active"),
            SubscriptionStatus::INVALID => write!(f, "Invalid"),
        }
    }
}

#[api(
    properties: {
        status: {
            type: SubscriptionStatus,
        },
    },
)]
#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
/// Proxmox subscription information
pub struct SubscriptionInfo {
    /// Subscription status from the last check
    pub status: SubscriptionStatus,
    /// the server ID, if permitted to access
    #[serde(skip_serializing_if="Option::is_none")]
    pub serverid: Option<String>,
    /// timestamp of the last check done
    #[serde(skip_serializing_if="Option::is_none")]
    pub checktime: Option<i64>,
    /// the subscription key, if set and permitted to access
    #[serde(skip_serializing_if="Option::is_none")]
    pub key: Option<String>,
    /// a more human readable status message
    #[serde(skip_serializing_if="Option::is_none")]
    pub message: Option<String>,
    /// human readable productname of the set subscription
    #[serde(skip_serializing_if="Option::is_none")]
    pub productname: Option<String>,
    /// register date of the set subscription
    #[serde(skip_serializing_if="Option::is_none")]
    pub regdate: Option<String>,
    /// next due date of the set subscription
    #[serde(skip_serializing_if="Option::is_none")]
    pub nextduedate: Option<String>,
    /// URL to the web shop
    #[serde(skip_serializing_if="Option::is_none")]
    pub url: Option<String>,
}

async fn register_subscription(
    key: &str,
    server_id: &str,
    checktime: i64
) -> Result<(String, String), Error> {
    // WHCMS sample code feeds the key into this, but it's just a challenge, so keep it simple
    let rand = proxmox::tools::bin_to_hex(&proxmox::sys::linux::random_data(16)?);
    let challenge = format!("{}{}", checktime, rand);

    let params = json!({
        "licensekey": key,
        "dir": server_id,
        "domain": "www.proxmox.com",
        "ip": "localhost",
        "check_token": challenge,
    });
    let uri = "https://shop.maurer-it.com/modules/servers/licensing/verify.php";
    let query = tools::json_object_to_query(params)?;
    let response = http::post(uri, Some(query), Some("application/x-www-form-urlencoded")).await?;
    let body = http::response_body_string(response).await?;

    Ok((body, challenge))
}

fn parse_status(value: &str) -> SubscriptionStatus {
    match value.to_lowercase().as_str() {
        "active" => SubscriptionStatus::ACTIVE,
        "new" => SubscriptionStatus::NEW,
        "notfound" => SubscriptionStatus::NOTFOUND,
        "invalid" => SubscriptionStatus::INVALID,
         _ => SubscriptionStatus::INVALID,
    }
}

fn parse_register_response(
    body: &str,
    key: String,
    server_id: String,
    checktime: i64,
    challenge: &str,
) -> Result<SubscriptionInfo, Error> {
    lazy_static! {
        static ref ATTR_RE: Regex = Regex::new(r"<([^>]+)>([^<]+)</[^>]+>").unwrap();
    }

    let mut info = SubscriptionInfo {
        key: Some(key),
        status: SubscriptionStatus::NOTFOUND,
        checktime: Some(checktime),
        url: Some("https://www.proxmox.com/en/proxmox-backup-server/pricing".into()),
        ..Default::default()
    };
    let mut md5hash = String::new();
    let is_server_id = |id: &&str| *id == server_id;

    for caps in ATTR_RE.captures_iter(body) {
        let (key, value) = (&caps[1], &caps[2]);
        match key {
            "status" => info.status = parse_status(value),
            "productname" => info.productname = Some(value.into()),
            "regdate" => info.regdate = Some(value.into()),
            "nextduedate" => info.nextduedate = Some(value.into()),
            "message" if value == "Directory Invalid" =>
                info.message = Some("Invalid Server ID".into()),
            "message" => info.message = Some(value.into()),
            "validdirectory" => {
                if value.split(',').find(is_server_id) == None {
                    bail!("Server ID does not match");
                }
                info.serverid = Some(server_id.to_owned());
            },
            "md5hash" => md5hash = value.to_owned(),
            _ => (),
        }
    }

    if let SubscriptionStatus::ACTIVE = info.status {
        let response_raw = format!("{}{}", SHARED_KEY_DATA, challenge);
        let expected = proxmox::tools::bin_to_hex(&tools::md5sum(response_raw.as_bytes())?);
        if expected != md5hash {
            bail!("Subscription API challenge failed, expected {} != got {}", expected, md5hash);
        }
    }
    Ok(info)
}

#[test]
fn test_parse_register_response() -> Result<(), Error> {
    let response = r#"
<status>Active</status>
<companyname>Proxmox</companyname>
<serviceid>41108</serviceid>
<productid>71</productid>
<productname>Proxmox Backup Server Test Subscription -1 year</productname>
<regdate>2020-09-19 00:00:00</regdate>
<nextduedate>2021-09-19</nextduedate>
<billingcycle>Annually</billingcycle>
<validdomain>proxmox.com,www.proxmox.com</validdomain>
<validdirectory>830000000123456789ABCDEF00000042</validdirectory>
<customfields>Notes=Test Key!</customfields>
<addons></addons>
<md5hash>969f4df84fe157ee4f5a2f71950ad154</md5hash>
"#;
    let key = "pbst-123456789a".to_string();
    let server_id = "830000000123456789ABCDEF00000042".to_string();
    let checktime = 1600000000;
    let salt = "cf44486bddb6ad0145732642c45b2957";

    let info = parse_register_response(response, key.to_owned(), server_id.to_owned(), checktime, salt)?;

    assert_eq!(info, SubscriptionInfo {
        key: Some(key),
        serverid: Some(server_id),
        status: SubscriptionStatus::ACTIVE,
        checktime: Some(checktime),
        url: Some("https://www.proxmox.com/en/proxmox-backup-server/pricing".into()),
        message: None,
        nextduedate: Some("2021-09-19".into()),
        regdate: Some("2020-09-19 00:00:00".into()),
        productname: Some("Proxmox Backup Server Test Subscription -1 year".into()),
    });
    Ok(())
}

/// queries the up to date subscription status and parses the response
pub fn check_subscription(key: String, server_id: String) -> Result<SubscriptionInfo, Error> {

    let now = proxmox::tools::time::epoch_i64();

    let (response, challenge) = tools::runtime::block_on(register_subscription(&key, &server_id, now))
        .map_err(|err| format_err!("Error checking subscription: {}", err))?;

    parse_register_response(&response, key, server_id, now, &challenge)
        .map_err(|err| format_err!("Error parsing subscription check response: {}", err))
}

/// reads in subscription information and does a basic integrity verification
pub fn read_subscription() -> Result<Option<SubscriptionInfo>, Error> {

    let cfg = proxmox::tools::fs::file_read_optional_string(&SUBSCRIPTION_FN)?;
    let cfg = if let Some(cfg) = cfg { cfg } else { return Ok(None); };

    let mut cfg = cfg.lines();

    // first line is key in plain
    let _key = if let Some(key) = cfg.next() { key } else { return Ok(None) };
    // second line is checksum of encoded data
    let checksum = if let Some(csum) = cfg.next() { csum } else { return Ok(None) };

    let encoded: String = cfg.collect::<String>();
    let decoded = base64::decode(encoded.to_owned())?;
    let decoded = std::str::from_utf8(&decoded)?;

    let info: SubscriptionInfo = serde_json::from_str(decoded)?;

    let new_checksum = format!("{}{}{}", info.checktime.unwrap_or(0), encoded, SHARED_KEY_DATA);
    let new_checksum = base64::encode(tools::md5sum(new_checksum.as_bytes())?);

    if checksum != new_checksum {
        bail!("stored checksum doesn't matches computed one '{}' != '{}'", checksum, new_checksum);
    }

    let age = proxmox::tools::time::epoch_i64() - info.checktime.unwrap_or(0);
    if age < -5400 { // allow some delta for DST changes or time syncs, 1.5h
        bail!("Last check time to far in the future.");
    } else if age > MAX_LOCAL_KEY_AGE + MAX_KEY_CHECK_FAILURE_AGE {
        if let SubscriptionStatus::ACTIVE = info.status {
            bail!("subscription information too old");
        }
    }

    Ok(Some(info))
}

/// writes out subscription status
pub fn write_subscription(info: SubscriptionInfo) -> Result<(), Error> {
    let key = info.key.to_owned();
    let server_id = info.serverid.to_owned();

    let raw = if info.key == None || info.checktime == None {
        String::new()
    } else if let SubscriptionStatus::NEW = info.status {
        format!("{}\n", info.key.unwrap())
    } else {
        let encoded = base64::encode(serde_json::to_string(&info)?);
        let csum = format!("{}{}{}", info.checktime.unwrap_or(0), encoded, SHARED_KEY_DATA);
        let csum = base64::encode(tools::md5sum(csum.as_bytes())?);
        format!("{}\n{}\n{}\n", info.key.unwrap(), csum, encoded)
    };

    let backup_user = crate::backup::backup_user()?;
    let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
    let file_opts = CreateOptions::new()
        .perm(mode)
        .owner(nix::unistd::ROOT)
        .group(backup_user.gid);

    let subscription_file = std::path::Path::new(SUBSCRIPTION_FN);
    replace_file(subscription_file, raw.as_bytes(), file_opts)?;

    update_apt_auth(key, server_id)?;

    Ok(())
}

/// deletes subscription from server
pub fn delete_subscription() -> Result<(), Error> {
    let subscription_file = std::path::Path::new(SUBSCRIPTION_FN);
    nix::unistd::unlink(subscription_file)?;
    update_apt_auth(None, None)?;
    Ok(())
}

/// updates apt authentication for repo access
pub fn update_apt_auth(key: Option<String>, password: Option<String>) -> Result<(), Error> {
    let auth_conf = std::path::Path::new(APT_AUTH_FN);
    match (key, password) {
        (Some(key), Some(password)) => {
            let conf = format!(
                "machine enterprise.proxmox.com/debian/pbs\n login {}\n password {}\n",
                key,
                password,
            );
            let mode = nix::sys::stat::Mode::from_bits_truncate(0o0640);
            let file_opts = CreateOptions::new()
                .perm(mode)
                .owner(nix::unistd::ROOT);

            // we use a namespaced .conf file, so just overwrite..
            replace_file(auth_conf, conf.as_bytes(), file_opts)
                .map_err(|e| format_err!("Error saving apt auth config - {}", e))?;
        }
        _ => match nix::unistd::unlink(auth_conf) {
            Ok(()) => Ok(()),
            Err(nix::Error::Sys(nix::errno::Errno::ENOENT)) => Ok(()), // ignore not existing
            Err(err) => Err(err),
        }.map_err(|e| format_err!("Error clearing apt auth config - {}", e))?,
    }
    Ok(())
}
