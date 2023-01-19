use std::sync::{Arc, Mutex};

use ::serde::{Deserialize, Serialize};
use anyhow::Error;
use lazy_static::lazy_static;
use openssl::sha;
use regex::Regex;
use serde_json::{json, Value};

use pbs_api_types::{IPRE, IPV4OCTET, IPV4RE, IPV6H16, IPV6LS32, IPV6RE};
use proxmox_router::{ApiMethod, Permission, Router, RpcEnvironment};
use proxmox_schema::api;
use proxmox_sys::fs::{file_get_contents, replace_file, CreateOptions};

use pbs_api_types::{
    FIRST_DNS_SERVER_SCHEMA, NODE_SCHEMA, PRIV_SYS_AUDIT, PRIV_SYS_MODIFY,
    PROXMOX_CONFIG_DIGEST_SCHEMA, SEARCH_DOMAIN_SCHEMA, SECOND_DNS_SERVER_SCHEMA,
    THIRD_DNS_SERVER_SCHEMA,
};

static RESOLV_CONF_FN: &str = "/etc/resolv.conf";

#[api()]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Deletable property name
pub enum DeletableProperty {
    /// Delete first nameserver entry
    Dns1,
    /// Delete second nameserver entry
    Dns2,
    /// Delete third nameserver entry
    Dns3,
}

pub fn read_etc_resolv_conf() -> Result<Value, Error> {
    let mut result = json!({});

    let mut nscount = 0;

    let raw = file_get_contents(RESOLV_CONF_FN)?;

    result["digest"] = Value::from(hex::encode(sha::sha256(&raw)));

    let data = String::from_utf8(raw)?;

    lazy_static! {
        static ref DOMAIN_REGEX: Regex = Regex::new(r"^\s*(?:search|domain)\s+(\S+)\s*").unwrap();
        static ref SERVER_REGEX: Regex =
            Regex::new(concat!(r"^\s*nameserver\s+(", IPRE!(), r")\s*")).unwrap();
    }

    let mut options = String::new();

    for line in data.lines() {
        if let Some(caps) = DOMAIN_REGEX.captures(line) {
            result["search"] = Value::from(&caps[1]);
        } else if let Some(caps) = SERVER_REGEX.captures(line) {
            nscount += 1;
            if nscount > 3 {
                continue;
            };
            let nameserver = &caps[1];
            let id = format!("dns{}", nscount);
            result[id] = Value::from(nameserver);
        } else {
            if !options.is_empty() {
                options.push('\n');
            }
            options.push_str(line);
        }
    }

    if !options.is_empty() {
        result["options"] = options.into();
    }

    Ok(result)
}

#[api(
    protected: true,
    input: {
        description: "Update DNS settings.",
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
            search: {
                schema: SEARCH_DOMAIN_SCHEMA,
                optional: true,
            },
            dns1: {
                optional: true,
                schema: FIRST_DNS_SERVER_SCHEMA,
            },
            dns2: {
                optional: true,
                schema: SECOND_DNS_SERVER_SCHEMA,
            },
            dns3: {
                optional: true,
                schema: THIRD_DNS_SERVER_SCHEMA,
            },
            delete: {
                description: "List of properties to delete.",
                type: Array,
                optional: true,
                items: {
                    type: DeletableProperty,
                }
            },
            digest: {
                optional: true,
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "network", "dns"], PRIV_SYS_MODIFY, false),
    }
)]
/// Update DNS settings
pub fn update_dns(
    search: Option<String>,
    dns1: Option<String>,
    dns2: Option<String>,
    dns3: Option<String>,
    delete: Option<Vec<DeletableProperty>>,
    digest: Option<String>,
) -> Result<Value, Error> {
    lazy_static! {
        static ref MUTEX: Arc<Mutex<()>> = Arc::new(Mutex::new(()));
    }

    let _guard = MUTEX.lock();

    let mut config = read_etc_resolv_conf()?;
    let old_digest = config["digest"].as_str().unwrap();

    if let Some(digest) = digest {
        crate::tools::assert_if_modified(old_digest, &digest)?;
    }

    if let Some(delete) = delete {
        for delete_prop in delete {
            let config = config.as_object_mut().unwrap();
            match delete_prop {
                DeletableProperty::Dns1 => {
                    config.remove("dns1");
                }
                DeletableProperty::Dns2 => {
                    config.remove("dns2");
                }
                DeletableProperty::Dns3 => {
                    config.remove("dns3");
                }
            }
        }
    }

    if let Some(search) = search {
        config["search"] = search.into();
    }
    if let Some(dns1) = dns1 {
        config["dns1"] = dns1.into();
    }
    if let Some(dns2) = dns2 {
        config["dns2"] = dns2.into();
    }
    if let Some(dns3) = dns3 {
        config["dns3"] = dns3.into();
    }

    let mut data = String::new();

    use std::fmt::Write as _;
    if let Some(search) = config["search"].as_str() {
        let _ = writeln!(data, "search {}", search);
    }
    for opt in &["dns1", "dns2", "dns3"] {
        if let Some(server) = config[opt].as_str() {
            let _ = writeln!(data, "nameserver {}", server);
        }
    }
    if let Some(options) = config["options"].as_str() {
        data.push_str(options);
    }

    replace_file(RESOLV_CONF_FN, data.as_bytes(), CreateOptions::new(), true)?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            node: {
                schema: NODE_SCHEMA,
            },
        },
    },
    returns: {
        description: "Returns DNS server IPs and sreach domain.",
        type: Object,
        properties: {
            digest: {
                schema: PROXMOX_CONFIG_DIGEST_SCHEMA,
            },
            search: {
                optional: true,
                schema: SEARCH_DOMAIN_SCHEMA,
            },
            dns1: {
                optional: true,
                schema: FIRST_DNS_SERVER_SCHEMA,
            },
            dns2: {
                optional: true,
                schema: SECOND_DNS_SERVER_SCHEMA,
            },
            dns3: {
                optional: true,
                schema: THIRD_DNS_SERVER_SCHEMA,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "network", "dns"], PRIV_SYS_AUDIT, false),
    }
)]
/// Read DNS settings.
pub fn get_dns(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {
    read_etc_resolv_conf()
}

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_GET_DNS)
    .put(&API_METHOD_UPDATE_DNS);
