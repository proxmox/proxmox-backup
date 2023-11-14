//! HTTP Client for the ACME protocol.

use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt;

use anyhow::{bail, format_err};
use bytes::Bytes;
use hyper::{Body, Request};
use nix::sys::stat::Mode;
use serde::{Deserialize, Serialize};

use proxmox_acme::account::AccountCreator;
use proxmox_acme::account::AccountData as AcmeAccountData;
use proxmox_acme::order::{Order, OrderData};
use proxmox_acme::Request as AcmeRequest;
use proxmox_acme::{Account, Authorization, Challenge, Directory, Error, ErrorResponse};
use proxmox_http::client::Client;
use proxmox_sys::fs::{replace_file, CreateOptions};

use crate::api2::types::AcmeAccountName;
use crate::config::acme::account_path;
use crate::tools::pbs_simple_http;

/// Our on-disk format inherited from PVE's proxmox-acme code.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountData {
    /// The account's location URL.
    location: String,

    /// The account data.
    account: AcmeAccountData,

    /// The private key as PEM formatted string.
    key: String,

    /// ToS URL the user agreed to.
    #[serde(skip_serializing_if = "Option::is_none")]
    tos: Option<String>,

    #[serde(skip_serializing_if = "is_false", default)]
    debug: bool,

    /// The directory's URL.
    directory_url: String,
}

#[inline]
fn is_false(b: &bool) -> bool {
    !*b
}

pub struct AcmeClient {
    directory_url: String,
    debug: bool,
    account_path: Option<String>,
    tos: Option<String>,
    account: Option<Account>,
    directory: Option<Directory>,
    nonce: Option<String>,
    http_client: Client,
}

impl AcmeClient {
    /// Create a new ACME client for a given ACME directory URL.
    pub fn new(directory_url: String) -> Self {
        Self {
            directory_url,
            debug: false,
            account_path: None,
            tos: None,
            account: None,
            directory: None,
            nonce: None,
            http_client: pbs_simple_http(None),
        }
    }

    /// Load an existing ACME account by name.
    pub async fn load(account_name: &AcmeAccountName) -> Result<Self, anyhow::Error> {
        let account_path = account_path(account_name.as_ref());
        let data = match tokio::fs::read(&account_path).await {
            Ok(data) => data,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                bail!("acme account '{}' does not exist", account_name)
            }
            Err(err) => bail!(
                "failed to load acme account from '{}' - {}",
                account_path,
                err
            ),
        };
        let data: AccountData = serde_json::from_slice(&data).map_err(|err| {
            format_err!(
                "failed to parse acme account from '{}' - {}",
                account_path,
                err
            )
        })?;

        let account = Account::from_parts(data.location, data.key, data.account);

        let mut me = Self::new(data.directory_url);
        me.debug = data.debug;
        me.account_path = Some(account_path);
        me.tos = data.tos;
        me.account = Some(account);

        Ok(me)
    }

    pub async fn new_account<'a>(
        &'a mut self,
        account_name: &AcmeAccountName,
        tos_agreed: bool,
        contact: Vec<String>,
        rsa_bits: Option<u32>,
        eab_creds: Option<(String, String)>,
    ) -> Result<&'a Account, anyhow::Error> {
        self.tos = if tos_agreed {
            self.terms_of_service_url().await?.map(str::to_owned)
        } else {
            None
        };

        let mut account = Account::creator()
            .set_contacts(contact)
            .agree_to_tos(tos_agreed);

        if let Some((eab_kid, eab_hmac_key)) = eab_creds {
            account = account.set_eab_credentials(eab_kid, eab_hmac_key)?;
        }

        let account = if let Some(bits) = rsa_bits {
            account.generate_rsa_key(bits)?
        } else {
            account.generate_ec_key()?
        };

        let _ = self.register_account(account).await?;

        crate::config::acme::make_acme_account_dir()?;
        let account_path = account_path(account_name.as_ref());
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&account_path)
            .map_err(|err| format_err!("failed to open {:?} for writing: {}", account_path, err))?;
        self.write_to(file).map_err(|err| {
            format_err!(
                "failed to write acme account to {:?}: {}",
                account_path,
                err
            )
        })?;
        self.account_path = Some(account_path);

        // unwrap: Setting `self.account` is literally this function's job, we just can't keep
        // the borrow from from `self.register_account()` active due to clashes.
        Ok(self.account.as_ref().unwrap())
    }

    fn save(&self) -> Result<(), anyhow::Error> {
        let mut data = Vec::<u8>::new();
        self.write_to(&mut data)?;
        let account_path = self.account_path.as_ref().ok_or_else(|| {
            format_err!("no account path set, cannot save updated account information")
        })?;
        crate::config::acme::make_acme_account_dir()?;
        replace_file(
            account_path,
            &data,
            CreateOptions::new()
                .perm(Mode::from_bits_truncate(0o600))
                .owner(nix::unistd::ROOT)
                .group(nix::unistd::Gid::from_raw(0)),
            true,
        )
    }

    /// Shortcut to `account().ok_or_else(...).key_authorization()`.
    pub fn key_authorization(&self, token: &str) -> Result<String, anyhow::Error> {
        Ok(Self::need_account(&self.account)?.key_authorization(token)?)
    }

    /// Shortcut to `account().ok_or_else(...).dns_01_txt_value()`.
    /// the key authorization value.
    pub fn dns_01_txt_value(&self, token: &str) -> Result<String, anyhow::Error> {
        Ok(Self::need_account(&self.account)?.dns_01_txt_value(token)?)
    }

    async fn register_account(
        &mut self,
        account: AccountCreator,
    ) -> Result<&Account, anyhow::Error> {
        let mut retry = retry();
        let mut response = loop {
            retry.tick()?;

            let (directory, nonce) = Self::get_dir_nonce(
                &mut self.http_client,
                &self.directory_url,
                &mut self.directory,
                &mut self.nonce,
            )
            .await?;
            let request = account.request(directory, nonce)?;
            match self.run_request(request).await {
                Ok(response) => break response,
                Err(err) if err.is_bad_nonce() => continue,
                Err(err) => return Err(err.into()),
            }
        };

        let account = account.response(response.location_required()?, &response.body)?;

        self.account = Some(account);
        Ok(self.account.as_ref().unwrap())
    }

    pub async fn update_account<T: Serialize>(
        &mut self,
        data: &T,
    ) -> Result<&Account, anyhow::Error> {
        let account = Self::need_account(&self.account)?;

        let mut retry = retry();
        let response = loop {
            retry.tick()?;

            let (_directory, nonce) = Self::get_dir_nonce(
                &mut self.http_client,
                &self.directory_url,
                &mut self.directory,
                &mut self.nonce,
            )
            .await?;

            let request = account.post_request(&account.location, nonce, data)?;
            match Self::execute(&mut self.http_client, request, &mut self.nonce).await {
                Ok(response) => break response,
                Err(err) if err.is_bad_nonce() => continue,
                Err(err) => return Err(err.into()),
            }
        };

        // unwrap: we've been keeping an immutable reference to it from the top of the method
        let _ = account;
        self.account.as_mut().unwrap().data = response.json()?;
        self.save()?;
        Ok(self.account.as_ref().unwrap())
    }

    pub async fn new_order<I>(&mut self, domains: I) -> Result<Order, anyhow::Error>
    where
        I: IntoIterator<Item = String>,
    {
        let account = Self::need_account(&self.account)?;

        let order = domains
            .into_iter()
            .fold(OrderData::new(), |order, domain| order.domain(domain));

        let mut retry = retry();
        loop {
            retry.tick()?;

            let (directory, nonce) = Self::get_dir_nonce(
                &mut self.http_client,
                &self.directory_url,
                &mut self.directory,
                &mut self.nonce,
            )
            .await?;

            let mut new_order = account.new_order(&order, directory, nonce)?;
            let mut response = match Self::execute(
                &mut self.http_client,
                new_order.request.take().unwrap(),
                &mut self.nonce,
            )
            .await
            {
                Ok(response) => response,
                Err(err) if err.is_bad_nonce() => continue,
                Err(err) => return Err(err.into()),
            };

            return Ok(
                new_order.response(response.location_required()?, response.bytes().as_ref())?
            );
        }
    }

    /// Low level "POST-as-GET" request.
    async fn post_as_get(&mut self, url: &str) -> Result<AcmeResponse, anyhow::Error> {
        let account = Self::need_account(&self.account)?;

        let mut retry = retry();
        loop {
            retry.tick()?;

            let (_directory, nonce) = Self::get_dir_nonce(
                &mut self.http_client,
                &self.directory_url,
                &mut self.directory,
                &mut self.nonce,
            )
            .await?;

            let request = account.get_request(url, nonce)?;
            match Self::execute(&mut self.http_client, request, &mut self.nonce).await {
                Ok(response) => return Ok(response),
                Err(err) if err.is_bad_nonce() => continue,
                Err(err) => return Err(err.into()),
            }
        }
    }

    /// Low level POST request.
    async fn post<T: Serialize>(
        &mut self,
        url: &str,
        data: &T,
    ) -> Result<AcmeResponse, anyhow::Error> {
        let account = Self::need_account(&self.account)?;

        let mut retry = retry();
        loop {
            retry.tick()?;

            let (_directory, nonce) = Self::get_dir_nonce(
                &mut self.http_client,
                &self.directory_url,
                &mut self.directory,
                &mut self.nonce,
            )
            .await?;

            let request = account.post_request(url, nonce, data)?;
            match Self::execute(&mut self.http_client, request, &mut self.nonce).await {
                Ok(response) => return Ok(response),
                Err(err) if err.is_bad_nonce() => continue,
                Err(err) => return Err(err.into()),
            }
        }
    }

    /// Request challenge validation. Afterwards, the challenge should be polled.
    pub async fn request_challenge_validation(
        &mut self,
        url: &str,
    ) -> Result<Challenge, anyhow::Error> {
        Ok(self
            .post(url, &serde_json::Value::Object(Default::default()))
            .await?
            .json()?)
    }

    /// Assuming the provided URL is an 'Authorization' URL, get and deserialize it.
    pub async fn get_authorization(&mut self, url: &str) -> Result<Authorization, anyhow::Error> {
        Ok(self.post_as_get(url).await?.json()?)
    }

    /// Assuming the provided URL is an 'Order' URL, get and deserialize it.
    pub async fn get_order(&mut self, url: &str) -> Result<OrderData, anyhow::Error> {
        Ok(self.post_as_get(url).await?.json()?)
    }

    /// Finalize an Order via its `finalize` URL property and the DER encoded CSR.
    pub async fn finalize(&mut self, url: &str, csr: &[u8]) -> Result<(), anyhow::Error> {
        let csr = base64::encode_config(csr, base64::URL_SAFE_NO_PAD);
        let data = serde_json::json!({ "csr": csr });
        self.post(url, &data).await?;
        Ok(())
    }

    /// Download a certificate via its 'certificate' URL property.
    ///
    /// The certificate will be a PEM certificate chain.
    pub async fn get_certificate(&mut self, url: &str) -> Result<Bytes, anyhow::Error> {
        Ok(self.post_as_get(url).await?.body)
    }

    /// Revoke an existing certificate (PEM or DER formatted).
    pub async fn revoke_certificate(
        &mut self,
        certificate: &[u8],
        reason: Option<u32>,
    ) -> Result<(), anyhow::Error> {
        // TODO: This can also work without an account.
        let account = Self::need_account(&self.account)?;

        let revocation = account.revoke_certificate(certificate, reason)?;

        let mut retry = retry();
        loop {
            retry.tick()?;

            let (directory, nonce) = Self::get_dir_nonce(
                &mut self.http_client,
                &self.directory_url,
                &mut self.directory,
                &mut self.nonce,
            )
            .await?;

            let request = revocation.request(directory, nonce)?;
            match Self::execute(&mut self.http_client, request, &mut self.nonce).await {
                Ok(_response) => return Ok(()),
                Err(err) if err.is_bad_nonce() => continue,
                Err(err) => return Err(err.into()),
            }
        }
    }

    fn need_account(account: &Option<Account>) -> Result<&Account, anyhow::Error> {
        account
            .as_ref()
            .ok_or_else(|| format_err!("cannot use client without an account"))
    }

    pub(crate) fn account(&self) -> Result<&Account, anyhow::Error> {
        Self::need_account(&self.account)
    }

    pub fn tos(&self) -> Option<&str> {
        self.tos.as_deref()
    }

    pub fn directory_url(&self) -> &str {
        &self.directory_url
    }

    fn to_account_data(&self) -> Result<AccountData, anyhow::Error> {
        let account = self.account()?;

        Ok(AccountData {
            location: account.location.clone(),
            key: account.private_key.clone(),
            account: AcmeAccountData {
                only_return_existing: false, // don't actually write this out in case it's set
                ..account.data.clone()
            },
            tos: self.tos.clone(),
            debug: self.debug,
            directory_url: self.directory_url.clone(),
        })
    }

    fn write_to<T: io::Write>(&self, out: T) -> Result<(), anyhow::Error> {
        let data = self.to_account_data()?;

        Ok(serde_json::to_writer_pretty(out, &data)?)
    }
}

struct AcmeResponse {
    body: Bytes,
    location: Option<String>,
    got_nonce: bool,
}

impl AcmeResponse {
    /// Convenience helper to assert that a location header was part of the response.
    fn location_required(&mut self) -> Result<String, anyhow::Error> {
        self.location
            .take()
            .ok_or_else(|| format_err!("missing Location header"))
    }

    /// Convenience shortcut to perform json deserialization of the returned body.
    fn json<T: for<'a> Deserialize<'a>>(&self) -> Result<T, Error> {
        Ok(serde_json::from_slice(&self.body)?)
    }

    /// Convenience shortcut to get the body as bytes.
    fn bytes(&self) -> &[u8] {
        &self.body
    }
}

impl AcmeClient {
    /// Non-self-borrowing run_request version for borrow workarounds.
    async fn execute(
        http_client: &mut Client,
        request: AcmeRequest,
        nonce: &mut Option<String>,
    ) -> Result<AcmeResponse, Error> {
        let req_builder = Request::builder().method(request.method).uri(&request.url);

        let http_request = if !request.content_type.is_empty() {
            req_builder
                .header("Content-Type", request.content_type)
                .header("Content-Length", request.body.len())
                .body(request.body.into())
        } else {
            req_builder.body(Body::empty())
        }
        .map_err(|err| Error::Custom(format!("failed to create http request: {}", err)))?;

        let response = http_client
            .request(http_request)
            .await
            .map_err(|err| Error::Custom(err.to_string()))?;
        let (parts, body) = response.into_parts();

        let status = parts.status.as_u16();
        let body = hyper::body::to_bytes(body)
            .await
            .map_err(|err| Error::Custom(format!("failed to retrieve response body: {}", err)))?;

        let got_nonce = if let Some(new_nonce) = parts.headers.get(proxmox_acme::REPLAY_NONCE) {
            let new_nonce = new_nonce.to_str().map_err(|err| {
                Error::Client(format!(
                    "received invalid replay-nonce header from ACME server: {}",
                    err
                ))
            })?;
            *nonce = Some(new_nonce.to_owned());
            true
        } else {
            false
        };

        if parts.status.is_success() {
            if status != request.expected {
                return Err(Error::InvalidApi(format!(
                    "ACME server responded with unexpected status code: {:?}",
                    parts.status
                )));
            }

            let location = parts
                .headers
                .get("Location")
                .map(|header| {
                    header.to_str().map(str::to_owned).map_err(|err| {
                        Error::Client(format!(
                            "received invalid location header from ACME server: {}",
                            err
                        ))
                    })
                })
                .transpose()?;

            return Ok(AcmeResponse {
                body,
                location,
                got_nonce,
            });
        }

        let error: ErrorResponse = serde_json::from_slice(&body).map_err(|err| {
            Error::Client(format!(
                "error status with improper error ACME response: {}",
                err
            ))
        })?;

        if error.ty == proxmox_acme::error::BAD_NONCE {
            if !got_nonce {
                return Err(Error::InvalidApi(
                    "badNonce without a new Replay-Nonce header".to_string(),
                ));
            }
            return Err(Error::BadNonce);
        }

        Err(Error::Api(error))
    }

    /// Low-level API to run an n API request. This automatically updates the current nonce!
    async fn run_request(&mut self, request: AcmeRequest) -> Result<AcmeResponse, Error> {
        Self::execute(&mut self.http_client, request, &mut self.nonce).await
    }

    pub async fn directory(&mut self) -> Result<&Directory, Error> {
        Ok(Self::get_directory(
            &mut self.http_client,
            &self.directory_url,
            &mut self.directory,
            &mut self.nonce,
        )
        .await?
        .0)
    }

    async fn get_directory<'a, 'b>(
        http_client: &mut Client,
        directory_url: &str,
        directory: &'a mut Option<Directory>,
        nonce: &'b mut Option<String>,
    ) -> Result<(&'a Directory, Option<&'b str>), Error> {
        if let Some(d) = directory {
            return Ok((d, nonce.as_deref()));
        }

        let response = Self::execute(
            http_client,
            AcmeRequest {
                url: directory_url.to_string(),
                method: "GET",
                content_type: "",
                body: String::new(),
                expected: 200,
            },
            nonce,
        )
        .await?;

        *directory = Some(Directory::from_parts(
            directory_url.to_string(),
            response.json()?,
        ));

        Ok((directory.as_ref().unwrap(), nonce.as_deref()))
    }

    /// Like `get_directory`, but if the directory provides no nonce, also performs a `HEAD`
    /// request on the new nonce URL.
    async fn get_dir_nonce<'a, 'b>(
        http_client: &mut Client,
        directory_url: &str,
        directory: &'a mut Option<Directory>,
        nonce: &'b mut Option<String>,
    ) -> Result<(&'a Directory, &'b str), Error> {
        // this let construct is a lifetime workaround:
        let _ = Self::get_directory(http_client, directory_url, directory, nonce).await?;
        let dir = directory.as_ref().unwrap(); // the above fails if it couldn't fill this option
        if nonce.is_none() {
            // this is also a lifetime issue...
            let _ = Self::get_nonce(http_client, nonce, dir.new_nonce_url()).await?;
        };
        Ok((dir, nonce.as_deref().unwrap()))
    }

    pub async fn terms_of_service_url(&mut self) -> Result<Option<&str>, Error> {
        Ok(self.directory().await?.terms_of_service_url())
    }

    async fn get_nonce<'a>(
        http_client: &mut Client,
        nonce: &'a mut Option<String>,
        new_nonce_url: &str,
    ) -> Result<&'a str, Error> {
        let response = Self::execute(
            http_client,
            AcmeRequest {
                url: new_nonce_url.to_owned(),
                method: "HEAD",
                content_type: "",
                body: String::new(),
                expected: 200,
            },
            nonce,
        )
        .await?;

        if !response.got_nonce {
            return Err(Error::InvalidApi(
                "no new nonce received from new nonce URL".to_string(),
            ));
        }

        nonce
            .as_deref()
            .ok_or_else(|| Error::Client("failed to update nonce".to_string()))
    }
}

/// bad nonce retry count helper
struct Retry(usize);

const fn retry() -> Retry {
    Retry(0)
}

impl Retry {
    fn tick(&mut self) -> Result<(), Error> {
        if self.0 >= 3 {
            Err(Error::Client("kept getting a badNonce error!".to_string()))
        } else {
            self.0 += 1;
            Ok(())
        }
    }
}
