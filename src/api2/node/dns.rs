use failure::*;


use crate::tools;
use crate::tools::common_regex;

use crate::api::schema::*;
use crate::api::router::*;

use lazy_static::lazy_static;

use std::io::{BufRead, BufReader};

use serde_json::{json, Value};

static RESOLV_CONF_FN: &str = "/etc/resolv.conf";

fn read_etc_resolv_conf() -> Result<Value, Error> {

    let mut result = json!({});

    let mut nscount = 0;

    let file = std::fs::File::open(RESOLV_CONF_FN)?;
    let mut reader = BufReader::new(file);

    let test = IPRE!();

    lazy_static! {
        static ref DOMAIN_REGEX: regex::Regex = regex::Regex::new(r"^\s*(?:search|domain)\s+(\S+)\s*").unwrap();
        static ref SERVER_REGEX: regex::Regex = regex::Regex::new(
            concat!(r"^\s*nameserver\s+(", IPRE!(),  r")\s*")).unwrap();
    }

    for line in reader.lines() {
        let line = line?;

        if let Some(m) = DOMAIN_REGEX.find(&line) {
            let domain = m.as_str();
            result["search"] = Value::from(domain);
        } else if let Some(m) = SERVER_REGEX.find(&line) {
            nscount += 1;
            if nscount > 3 { continue };
            let nameserver = m.as_str();
            let id = format!("dns{}", nscount);
            result[id] = Value::from(m.as_str());
        }
    }

    Ok(result)
}

fn get_dns(_param: Value, _info: &ApiMethod) -> Result<Value, Error> {

    read_etc_resolv_conf()
}

pub fn router() -> Router {

    let route = Router::new()
        .get(
            ApiMethod::new(
                get_dns,
                ObjectSchema::new("Read DNS settings.")
            ).returns(
                ObjectSchema::new("Returns DNS server IPs and sreach domain.")
                    .optional("search", StringSchema::new("Search domain for host-name lookup."))
                    .optional("dns1", StringSchema::new("First name server IP address."))
                    .optional("dns2", StringSchema::new("Second name server IP address."))
                    .optional("dns3", StringSchema::new("Third name server IP address."))
            )
        );

    route
}
