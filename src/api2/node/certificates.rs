use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, format_err, Error};
use openssl::pkey::PKey;
use openssl::x509::X509;
use serde::{Deserialize, Serialize};

use proxmox_router::list_subdirs_api_method;
use proxmox_router::SubdirMap;
use proxmox_router::{Permission, Router, RpcEnvironment};
use proxmox_schema::api;
use proxmox_sys::{task_log, task_warn};

use pbs_api_types::{NODE_SCHEMA, PRIV_SYS_MODIFY};
use pbs_buildcfg::configdir;
use pbs_tools::cert;

use crate::acme::AcmeClient;
use crate::api2::types::AcmeDomain;
use crate::config::node::NodeConfig;
use crate::server::send_certificate_renewal_mail;
use proxmox_rest_server::WorkerTask;

pub const ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(SUBDIRS))
    .subdirs(SUBDIRS);

const SUBDIRS: SubdirMap = &[
    ("acme", &ACME_ROUTER),
    (
        "custom",
        &Router::new()
            .post(&API_METHOD_UPLOAD_CUSTOM_CERTIFICATE)
            .delete(&API_METHOD_DELETE_CUSTOM_CERTIFICATE),
    ),
    ("info", &Router::new().get(&API_METHOD_GET_INFO)),
];

const ACME_ROUTER: Router = Router::new()
    .get(&list_subdirs_api_method!(ACME_SUBDIRS))
    .subdirs(ACME_SUBDIRS);

const ACME_SUBDIRS: SubdirMap = &[(
    "certificate",
    &Router::new()
        .post(&API_METHOD_NEW_ACME_CERT)
        .put(&API_METHOD_RENEW_ACME_CERT),
)];

#[api(
    properties: {
        san: {
            type: Array,
            items: {
                description: "A SubjectAlternateName entry.",
                type: String,
            },
        },
    },
)]
/// Certificate information.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct CertificateInfo {
    /// Certificate file name.
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<String>,

    /// Certificate subject name.
    subject: String,

    /// List of certificate's SubjectAlternativeName entries.
    san: Vec<String>,

    /// Certificate issuer name.
    issuer: String,

    /// Certificate's notBefore timestamp (UNIX epoch).
    #[serde(skip_serializing_if = "Option::is_none")]
    notbefore: Option<i64>,

    /// Certificate's notAfter timestamp (UNIX epoch).
    #[serde(skip_serializing_if = "Option::is_none")]
    notafter: Option<i64>,

    /// Certificate in PEM format.
    #[serde(skip_serializing_if = "Option::is_none")]
    pem: Option<String>,

    /// Certificate's public key algorithm.
    public_key_type: String,

    /// Certificate's public key size if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    public_key_bits: Option<u32>,

    /// The SSL Fingerprint.
    #[serde(skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
}

impl TryFrom<&cert::CertInfo> for CertificateInfo {
    type Error = Error;

    fn try_from(info: &cert::CertInfo) -> Result<Self, Self::Error> {
        let pubkey = info.public_key()?;

        Ok(Self {
            filename: None,
            subject: info.subject_name()?,
            san: info
                .subject_alt_names()
                .map(|san| {
                    san.into_iter()
                        // FIXME: Support `.ipaddress()`?
                        .filter_map(|name| name.dnsname().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            issuer: info.issuer_name()?,
            notbefore: info.not_before_unix().ok(),
            notafter: info.not_after_unix().ok(),
            pem: None,
            public_key_type: openssl::nid::Nid::from_raw(pubkey.id().as_raw())
                .long_name()
                .unwrap_or("<unsupported key type>")
                .to_owned(),
            public_key_bits: Some(pubkey.bits()),
            fingerprint: Some(info.fingerprint()?),
        })
    }
}

fn get_certificate_pem() -> Result<String, Error> {
    let cert_path = configdir!("/proxy.pem");
    let cert_pem = proxmox_sys::fs::file_get_contents(cert_path)?;
    String::from_utf8(cert_pem)
        .map_err(|_| format_err!("certificate in {:?} is not a valid PEM file", cert_path))
}

// to deduplicate error messages
fn pem_to_cert_info(pem: &[u8]) -> Result<cert::CertInfo, Error> {
    cert::CertInfo::from_pem(pem)
        .map_err(|err| format_err!("error loading proxy certificate: {}", err))
}

#[api(
    input: {
        properties: {
            node: { schema: NODE_SCHEMA },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    returns: {
        type: Array,
        items: { type: CertificateInfo },
        description: "List of certificate infos.",
    },
)]
/// Get certificate info.
pub fn get_info() -> Result<Vec<CertificateInfo>, Error> {
    let cert_pem = get_certificate_pem()?;
    let cert = pem_to_cert_info(cert_pem.as_bytes())?;

    Ok(vec![CertificateInfo {
        filename: Some("proxy.pem".to_string()), // we only have the one
        pem: Some(cert_pem),
        ..CertificateInfo::try_from(&cert)?
    }])
}

#[api(
    input: {
        properties: {
            node: { schema: NODE_SCHEMA },
            certificates: { description: "PEM encoded certificate (chain)." },
            key: { description: "PEM encoded private key." },
            // FIXME: widget-toolkit should have an option to disable using these 2 parameters...
            restart: {
                description: "UI compatibility parameter, ignored",
                type: Boolean,
                optional: true,
                default: false,
            },
            force: {
                description: "Force replacement of existing files.",
                type: Boolean,
                optional: true,
                default: false,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    returns: {
        type: Array,
        items: { type: CertificateInfo },
        description: "List of certificate infos.",
    },
    protected: true,
)]
/// Upload a custom certificate.
pub async fn upload_custom_certificate(
    certificates: String,
    key: String,
) -> Result<Vec<CertificateInfo>, Error> {
    let certificates = X509::stack_from_pem(certificates.as_bytes())
        .map_err(|err| format_err!("failed to decode certificate chain: {}", err))?;
    let key = PKey::private_key_from_pem(key.as_bytes())
        .map_err(|err| format_err!("failed to parse private key: {}", err))?;

    let certificates = certificates
        .into_iter()
        .try_fold(Vec::<u8>::new(), |mut stack, cert| -> Result<_, Error> {
            if !stack.is_empty() {
                stack.push(b'\n');
            }
            stack.extend(cert.to_pem()?);
            Ok(stack)
        })
        .map_err(|err| format_err!("error formatting certificate chain as PEM: {}", err))?;

    let key = key.private_key_to_pem_pkcs8()?;

    crate::config::set_proxy_certificate(&certificates, &key)?;
    crate::server::reload_proxy_certificate().await?;

    get_info()
}

#[api(
    input: {
        properties: {
            node: { schema: NODE_SCHEMA },
            restart: {
                description: "UI compatibility parameter, ignored",
                type: Boolean,
                optional: true,
                default: false,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Delete the current certificate and regenerate a self signed one.
pub async fn delete_custom_certificate() -> Result<(), Error> {
    let cert_path = configdir!("/proxy.pem");
    // Here we fail since if this fails nothing else breaks anyway
    std::fs::remove_file(cert_path)
        .map_err(|err| format_err!("failed to unlink {:?} - {}", cert_path, err))?;

    let key_path = configdir!("/proxy.key");
    if let Err(err) = std::fs::remove_file(key_path) {
        // Here we just log since the certificate is already gone and we'd rather try to generate
        // the self-signed certificate even if this fails:
        log::error!(
            "failed to remove certificate private key {:?} - {}",
            key_path,
            err
        );
    }

    crate::config::update_self_signed_cert(true)?;
    crate::server::reload_proxy_certificate().await?;

    Ok(())
}

struct OrderedCertificate {
    certificate: hyper::body::Bytes,
    private_key_pem: Vec<u8>,
}

async fn order_certificate(
    worker: Arc<WorkerTask>,
    node_config: &NodeConfig,
) -> Result<Option<OrderedCertificate>, Error> {
    use proxmox_acme::authorization::Status;
    use proxmox_acme::order::Identifier;

    let domains = node_config.acme_domains().try_fold(
        Vec::<AcmeDomain>::new(),
        |mut acc, domain| -> Result<_, Error> {
            let mut domain = domain?;
            domain.domain.make_ascii_lowercase();
            if let Some(alias) = &mut domain.alias {
                alias.make_ascii_lowercase();
            }
            acc.push(domain);
            Ok(acc)
        },
    )?;

    let get_domain_config = |domain: &str| {
        domains
            .iter()
            .find(|d| d.domain == domain)
            .ok_or_else(|| format_err!("no config for domain '{}'", domain))
    };

    if domains.is_empty() {
        task_log!(
            worker,
            "No domains configured to be ordered from an ACME server."
        );
        return Ok(None);
    }

    let (plugins, _) = crate::config::acme::plugin::config()?;

    let mut acme = node_config.acme_client().await?;

    task_log!(worker, "Placing ACME order");
    let order = acme
        .new_order(domains.iter().map(|d| d.domain.to_ascii_lowercase()))
        .await?;
    task_log!(worker, "Order URL: {}", order.location);

    let identifiers: Vec<String> = order
        .data
        .identifiers
        .iter()
        .map(|identifier| match identifier {
            Identifier::Dns(domain) => domain.clone(),
        })
        .collect();

    for auth_url in &order.data.authorizations {
        task_log!(worker, "Getting authorization details from '{}'", auth_url);
        let mut auth = acme.get_authorization(auth_url).await?;

        let domain = match &mut auth.identifier {
            Identifier::Dns(domain) => domain.to_ascii_lowercase(),
        };

        if auth.status == Status::Valid {
            task_log!(worker, "{} is already validated!", domain);
            continue;
        }

        task_log!(worker, "The validation for {} is pending", domain);
        let domain_config: &AcmeDomain = get_domain_config(&domain)?;
        let plugin_id = domain_config.plugin.as_deref().unwrap_or("standalone");
        let mut plugin_cfg =
            crate::acme::get_acme_plugin(&plugins, plugin_id)?.ok_or_else(|| {
                format_err!("plugin '{}' for domain '{}' not found!", plugin_id, domain)
            })?;

        task_log!(worker, "Setting up validation plugin");
        let validation_url = plugin_cfg
            .setup(&mut acme, &auth, domain_config, Arc::clone(&worker))
            .await?;

        let result = request_validation(&worker, &mut acme, auth_url, validation_url).await;

        if let Err(err) = plugin_cfg
            .teardown(&mut acme, &auth, domain_config, Arc::clone(&worker))
            .await
        {
            task_warn!(
                worker,
                "Failed to teardown plugin '{}' for domain '{}' - {}",
                plugin_id,
                domain,
                err
            );
        }

        result?;
    }

    task_log!(worker, "All domains validated");
    task_log!(worker, "Creating CSR");

    let csr = proxmox_acme::util::Csr::generate(&identifiers, &Default::default())?;
    let mut finalize_error_cnt = 0u8;
    let order_url = &order.location;
    let mut order;
    loop {
        use proxmox_acme::order::Status;

        order = acme.get_order(order_url).await?;

        match order.status {
            Status::Pending => {
                task_log!(worker, "still pending, trying to finalize anyway");
                let finalize = order
                    .finalize
                    .as_deref()
                    .ok_or_else(|| format_err!("missing 'finalize' URL in order"))?;
                if let Err(err) = acme.finalize(finalize, &csr.data).await {
                    if finalize_error_cnt >= 5 {
                        return Err(err);
                    }

                    finalize_error_cnt += 1;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Status::Ready => {
                task_log!(worker, "order is ready, finalizing");
                let finalize = order
                    .finalize
                    .as_deref()
                    .ok_or_else(|| format_err!("missing 'finalize' URL in order"))?;
                acme.finalize(finalize, &csr.data).await?;
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            Status::Processing => {
                task_log!(worker, "still processing, trying again in 30 seconds");
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            Status::Valid => {
                task_log!(worker, "valid");
                break;
            }
            other => bail!("order status: {:?}", other),
        }
    }

    task_log!(worker, "Downloading certificate");
    let certificate = acme
        .get_certificate(
            order
                .certificate
                .as_deref()
                .ok_or_else(|| format_err!("missing certificate url in finalized order"))?,
        )
        .await?;

    Ok(Some(OrderedCertificate {
        certificate,
        private_key_pem: csr.private_key_pem,
    }))
}

async fn request_validation(
    worker: &WorkerTask,
    acme: &mut AcmeClient,
    auth_url: &str,
    validation_url: &str,
) -> Result<(), Error> {
    task_log!(worker, "Triggering validation");
    acme.request_challenge_validation(validation_url).await?;

    task_log!(worker, "Sleeping for 5 seconds");
    tokio::time::sleep(Duration::from_secs(5)).await;

    loop {
        use proxmox_acme::authorization::Status;

        let auth = acme.get_authorization(auth_url).await?;
        match auth.status {
            Status::Pending => {
                task_log!(
                    worker,
                    "Status is still 'pending', trying again in 10 seconds"
                );
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            Status::Valid => return Ok(()),
            other => bail!(
                "validating challenge '{}' failed - status: {:?}",
                validation_url,
                other
            ),
        }
    }
}

#[api(
    input: {
        properties: {
            node: { schema: NODE_SCHEMA },
            force: {
                description: "Force replacement of existing files.",
                type: Boolean,
                optional: true,
                default: false,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Order a new ACME certificate.
pub fn new_acme_cert(force: bool, rpcenv: &mut dyn RpcEnvironment) -> Result<String, Error> {
    spawn_certificate_worker("acme-new-cert", force, rpcenv)
}

#[api(
    input: {
        properties: {
            node: { schema: NODE_SCHEMA },
            force: {
                description: "Force replacement of existing files.",
                type: Boolean,
                optional: true,
                default: false,
            },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Renew the current ACME certificate if it expires within 30 days (or always if the `force`
/// parameter is set).
pub fn renew_acme_cert(force: bool, rpcenv: &mut dyn RpcEnvironment) -> Result<String, Error> {
    if !cert_expires_soon()? && !force {
        bail!("Certificate does not expire within the next 30 days and 'force' is not set.")
    }

    spawn_certificate_worker("acme-renew-cert", force, rpcenv)
}

/// Check whether the current certificate expires within the next 30 days.
pub fn cert_expires_soon() -> Result<bool, Error> {
    let cert = pem_to_cert_info(get_certificate_pem()?.as_bytes())?;
    cert.is_expired_after_epoch(proxmox_time::epoch_i64() + 30 * 24 * 60 * 60)
        .map_err(|err| format_err!("Failed to check certificate expiration date: {}", err))
}

fn spawn_certificate_worker(
    name: &'static str,
    force: bool,
    rpcenv: &mut dyn RpcEnvironment,
) -> Result<String, Error> {
    // We only have 1 certificate path in PBS which makes figuring out whether or not it is a
    // custom one too hard... We keep the parameter because the widget-toolkit may be using it...
    let _ = force;

    let (node_config, _digest) = crate::config::node::config()?;

    let auth_id = rpcenv.get_auth_id().unwrap();

    WorkerTask::spawn(name, None, auth_id, true, move |worker| async move {
        let work = || async {
            if let Some(cert) = order_certificate(worker, &node_config).await? {
                crate::config::set_proxy_certificate(&cert.certificate, &cert.private_key_pem)?;
                crate::server::reload_proxy_certificate().await?;
            }

            Ok(())
        };

        let res = work().await;

        send_certificate_renewal_mail(&res)?;

        res
    })
}

#[api(
    input: {
        properties: {
            node: { schema: NODE_SCHEMA },
        },
    },
    access: {
        permission: &Permission::Privilege(&["system", "certificates"], PRIV_SYS_MODIFY, false),
    },
    protected: true,
)]
/// Renew the current ACME certificate if it expires within 30 days (or always if the `force`
/// parameter is set).
pub fn revoke_acme_cert(rpcenv: &mut dyn RpcEnvironment) -> Result<String, Error> {
    let (node_config, _digest) = crate::config::node::config()?;

    let cert_pem = get_certificate_pem()?;

    let auth_id = rpcenv.get_auth_id().unwrap();

    WorkerTask::spawn(
        "acme-revoke-cert",
        None,
        auth_id,
        true,
        move |worker| async move {
            task_log!(worker, "Loading ACME account");
            let mut acme = node_config.acme_client().await?;
            task_log!(worker, "Revoking old certificate");
            acme.revoke_certificate(cert_pem.as_bytes(), None).await?;
            task_log!(
                worker,
                "Deleting certificate and regenerating a self-signed one"
            );
            delete_custom_certificate().await?;
            Ok(())
        },
    )
}
