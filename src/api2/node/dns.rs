use std::sync::{Arc, Mutex};

use failure::*;
use lazy_static::lazy_static;
use openssl::sha;
use regex::Regex;
use serde_json::{json, Value};

use proxmox::{sortable, identity};
use proxmox::api::{ApiHandler, ApiMethod, Router, RpcEnvironment};
use proxmox::api::schema::*;
use proxmox::tools::fs::{file_get_contents, replace_file, CreateOptions};
use proxmox::{IPRE, IPV4RE, IPV6RE, IPV4OCTET, IPV6H16, IPV6LS32};

use crate::api2::types::*;

static RESOLV_CONF_FN: &str = "/etc/resolv.conf";

pub fn read_etc_resolv_conf() -> Result<Value, Error> {

    let mut result = json!({});

    let mut nscount = 0;

    let raw = file_get_contents(RESOLV_CONF_FN)?;

    result["digest"] = Value::from(proxmox::tools::digest_to_hex(&sha::sha256(&raw)));

    let data = String::from_utf8(raw)?;

    lazy_static! {
        static ref DOMAIN_REGEX: Regex = Regex::new(r"^\s*(?:search|domain)\s+(\S+)\s*").unwrap();
        static ref SERVER_REGEX: Regex = Regex::new(
            concat!(r"^\s*nameserver\s+(", IPRE!(),  r")\s*")).unwrap();
    }

    for line in data.lines() {

        if let Some(caps) = DOMAIN_REGEX.captures(&line) {
            result["search"] = Value::from(&caps[1]);
        } else if let Some(caps) = SERVER_REGEX.captures(&line) {
            nscount += 1;
            if nscount > 3 { continue };
            let nameserver = &caps[1];
            let id = format!("dns{}", nscount);
            result[id] = Value::from(nameserver);
        }
    }

    Ok(result)
}

fn update_dns(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    lazy_static! {
        static ref MUTEX: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
    }

    let _guard = MUTEX.lock();

    let search = crate::tools::required_string_param(&param, "search")?;

    let raw = file_get_contents(RESOLV_CONF_FN)?;
    let old_digest = proxmox::tools::digest_to_hex(&sha::sha256(&raw));

    if let Some(digest) = param["digest"].as_str() {
        crate::tools::assert_if_modified(&old_digest, &digest)?;
    }

    let old_data = String::from_utf8(raw)?;

    let mut data = format!("search {}\n", search);

    for opt in &["dns1", "dns2", "dns3"] {
        if let Some(server) = param[opt].as_str() {
            data.push_str(&format!("nameserver {}\n", server));
        }
    }

    // append other data
    lazy_static! {
        static ref SKIP_REGEX: Regex = Regex::new(r"^(search|domain|nameserver)\s+").unwrap();
    }
    for line in old_data.lines() {
        if SKIP_REGEX.is_match(line) { continue; }
        data.push_str(line);
        data.push('\n');
    }

    replace_file(RESOLV_CONF_FN, data.as_bytes(), CreateOptions::new())?;

    Ok(Value::Null)
}

fn get_dns(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    read_etc_resolv_conf()
}

#[sortable]
pub const ROUTER: Router = Router::new()
    .get(
        &ApiMethod::new(
            &ApiHandler::Sync(&get_dns),
            &ObjectSchema::new(
                "Read DNS settings.",
                &sorted!([ ("node", false, &NODE_SCHEMA) ]),
            )
        ).returns(
            &ObjectSchema::new(
                "Returns DNS server IPs and sreach domain.",
                &sorted!([
                    ("digest", false, &PROXMOX_CONFIG_DIGEST_SCHEMA),
                    ("search", true, &SEARCH_DOMAIN_SCHEMA),
                    ("dns1", true, &FIRST_DNS_SERVER_SCHEMA),
                    ("dns2", true, &SECOND_DNS_SERVER_SCHEMA),
                    ("dns3", true, &THIRD_DNS_SERVER_SCHEMA),
                ]),
            ).schema()
        )
    )
    .put(
        &ApiMethod::new(
            &ApiHandler::Sync(&update_dns),
            &ObjectSchema::new(
                "Returns DNS server IPs and sreach domain.",
                &sorted!([
                    ("node", false, &NODE_SCHEMA),
                    ("search", false, &SEARCH_DOMAIN_SCHEMA),
                    ("dns1", true, &FIRST_DNS_SERVER_SCHEMA),
                    ("dns2", true, &SECOND_DNS_SERVER_SCHEMA),
                    ("dns3", true, &THIRD_DNS_SERVER_SCHEMA),
                    ("digest", true, &PROXMOX_CONFIG_DIGEST_SCHEMA),
                ]),
            )
        ).protected(true)
    );
