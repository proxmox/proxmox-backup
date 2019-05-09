use failure::*;
use lazy_static::lazy_static;
use std::sync::Arc;

use crate::api_schema::*;
use crate::tools::{self, common_regex};

lazy_static!{

    pub static ref IP_FORMAT: Arc<ApiStringFormat> = ApiStringFormat::Pattern(&common_regex::IP_REGEX).into();

    pub static ref PVE_CONFIG_DIGEST_FORMAT: Arc<ApiStringFormat> =
        ApiStringFormat::Pattern(&common_regex::SHA256_HEX_REGEX).into();

    pub static ref PVE_CONFIG_DIGEST_SCHEMA: Arc<Schema> =
        StringSchema::new("Prevent changes if current configuration file has different SHA256 digest. This can be used to prevent concurrent modifications.")
        .format(PVE_CONFIG_DIGEST_FORMAT.clone()).into();

    pub static ref NODE_SCHEMA: Arc<Schema> = Arc::new(
        StringSchema::new("Node name (or 'localhost')")
            .format(
                Arc::new(ApiStringFormat::VerifyFn(|node| {
                    if node == "localhost" || node == tools::nodename() {
                        Ok(())
                    } else {
                        Err(format_err!("no such node '{}'", node))
                    }
                }))
            )
            .into()
    );

    pub static ref SEARCH_DOMAIN_SCHEMA: Arc<Schema> =
        StringSchema::new("Search domain for host-name lookup.").into();

    pub static ref FIRST_DNS_SERVER_SCHEMA: Arc<Schema> =
        StringSchema::new("First name server IP address.")
        .format(IP_FORMAT.clone()).into();

    pub static ref SECOND_DNS_SERVER_SCHEMA: Arc<Schema> =
        StringSchema::new("Second name server IP address.")
        .format(IP_FORMAT.clone()).into();

    pub static ref THIRD_DNS_SERVER_SCHEMA: Arc<Schema> =
        StringSchema::new("Third name server IP address.")
        .format(IP_FORMAT.clone()).into();


}
