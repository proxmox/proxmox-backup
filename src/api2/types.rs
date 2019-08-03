use failure::*;
use lazy_static::lazy_static;
use std::sync::Arc;

use crate::api_schema::*;
use proxmox::tools::common_regex;

lazy_static!{

    // File names: may not contain slashes, may not start with "."
    pub static ref FILENAME_FORMAT: Arc<ApiStringFormat> = Arc::new(ApiStringFormat::VerifyFn(|name| {
        if name.starts_with('.') {
            bail!("file names may not start with '.'");
        }
        if name.contains('/') {
            bail!("file names may not contain slashes");
        }
        Ok(())
    })).into();

    pub static ref IP_FORMAT: Arc<ApiStringFormat> = ApiStringFormat::Pattern(&common_regex::IP_REGEX).into();

    pub static ref PVE_CONFIG_DIGEST_FORMAT: Arc<ApiStringFormat> =
        ApiStringFormat::Pattern(&common_regex::SHA256_HEX_REGEX).into();

    pub static ref PVE_CONFIG_DIGEST_SCHEMA: Arc<Schema> =
        StringSchema::new("Prevent changes if current configuration file has different SHA256 digest. This can be used to prevent concurrent modifications.")
        .format(PVE_CONFIG_DIGEST_FORMAT.clone()).into();

    pub static ref CHUNK_DIGEST_FORMAT: Arc<ApiStringFormat> =
        ApiStringFormat::Pattern(&common_regex::SHA256_HEX_REGEX).into();

    pub static ref CHUNK_DIGEST_SCHEMA: Arc<Schema> =
        StringSchema::new("Chunk digest (SHA256).")
        .format(CHUNK_DIGEST_FORMAT.clone()).into();

    pub static ref NODE_SCHEMA: Arc<Schema> = Arc::new(
        StringSchema::new("Node name (or 'localhost')")
            .format(
                Arc::new(ApiStringFormat::VerifyFn(|node| {
                    if node == "localhost" || node == proxmox::tools::nodename() {
                        Ok(())
                    } else {
                        bail!("no such node '{}'", node);
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

    pub static ref BACKUP_ARCHIVE_NAME_SCHEMA: Arc<Schema> =
        StringSchema::new("Backup archive name.")
        .format(FILENAME_FORMAT.clone()).into();

    pub static ref BACKUP_TYPE_SCHEMA: Arc<Schema> =
        StringSchema::new("Backup type.")
        .format(Arc::new(ApiStringFormat::Enum(&["vm", "ct", "host"])))
        .into();

    pub static ref BACKUP_ID_SCHEMA: Arc<Schema> =
        StringSchema::new("Backup ID.")
        .format(FILENAME_FORMAT.clone())
        .into();

    pub static ref BACKUP_TIME_SCHEMA: Arc<Schema> =
        IntegerSchema::new("Backup time (Unix epoch.)")
        .minimum(1547797308)
        .into();

}
