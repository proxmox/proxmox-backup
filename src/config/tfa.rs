use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, format_err, Error};
use nix::sys::stat::Mode;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::sign::Signer;
use serde::{de::Deserializer, Deserialize, Serialize};
use serde_json::Value;
use webauthn_rs::{proto::UserVerificationPolicy, Webauthn};

use webauthn_rs::proto::Credential as WebauthnCredential;

use proxmox::api::api;
use proxmox::api::schema::{Updatable, Updater};
use proxmox::sys::error::SysError;
use proxmox::tools::fs::CreateOptions;
use proxmox::tools::tfa::totp::Totp;
use proxmox::tools::tfa::u2f;
use proxmox::tools::uuid::Uuid;
use proxmox::tools::AsHex;

use crate::api2::types::Userid;

/// Mapping of userid to TFA entry.
pub type TfaUsers = HashMap<Userid, TfaUserData>;

const CONF_FILE: &str = configdir!("/tfa.json");
const LOCK_FILE: &str = configdir!("/tfa.json.lock");
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

const CHALLENGE_DATA_PATH: &str = rundir!("/tfa/challenges");

/// U2F registration challenges time out after 2 minutes.
const CHALLENGE_TIMEOUT: i64 = 2 * 60;

pub fn read_lock() -> Result<File, Error> {
    proxmox::tools::fs::open_file_locked(LOCK_FILE, LOCK_TIMEOUT, false)
}

pub fn write_lock() -> Result<File, Error> {
    proxmox::tools::fs::open_file_locked(LOCK_FILE, LOCK_TIMEOUT, true)
}

/// Read the TFA entries.
pub fn read() -> Result<TfaConfig, Error> {
    let file = match File::open(CONF_FILE) {
        Ok(file) => file,
        Err(ref err) if err.not_found() => return Ok(TfaConfig::default()),
        Err(err) => return Err(err.into()),
    };

    Ok(serde_json::from_reader(file)?)
}

/// Get the webauthn config with a digest.
///
/// This is meant only for configuration updates, which currently only means webauthn updates.
/// Since this is meant to be done only once (since changes will lock out users), this should be
/// used rarely, since the digest calculation is currently a bit more involved.
pub fn webauthn_config() -> Result<Option<(WebauthnConfig, [u8; 32])>, Error>{
    Ok(match read()?.webauthn {
        Some(wa) => {
            let digest = wa.digest()?;
            Some((wa, digest))
        }
        None => None,
    })
}

/// Requires the write lock to be held.
pub fn write(data: &TfaConfig) -> Result<(), Error> {
    let options = CreateOptions::new().perm(Mode::from_bits_truncate(0o0600));

    let json = serde_json::to_vec(data)?;
    proxmox::tools::fs::replace_file(CONF_FILE, &json, options)
}

#[derive(Deserialize, Serialize)]
pub struct U2fConfig {
    appid: String,
}

#[api]
#[derive(Clone, Deserialize, Serialize, Updater)]
#[serde(deny_unknown_fields)]
/// Server side webauthn server configuration.
pub struct WebauthnConfig {
    /// Relying party name. Any text identifier.
    ///
    /// Changing this *may* break existing credentials.
    rp: String,

    /// Site origin. Must be a `https://` URL (or `http://localhost`). Should contain the address
    /// users type in their browsers to access the web interface.
    ///
    /// Changing this *may* break existing credentials.
    origin: String,

    /// Relying part ID. Must be the domain name without protocol, port or location.
    ///
    /// Changing this *will* break existing credentials.
    id: String,
}

impl WebauthnConfig {
    pub fn digest(&self) -> Result<[u8; 32], Error> {
        let digest_data = crate::tools::json::to_canonical_json(&serde_json::to_value(self)?)?;
        Ok(openssl::sha::sha256(&digest_data))
    }
}

/// For now we just implement this on the configuration this way.
///
/// Note that we may consider changing this so `get_origin` returns the `Host:` header provided by
/// the connecting client.
impl webauthn_rs::WebauthnConfig for WebauthnConfig {
    fn get_relying_party_name(&self) -> String {
        self.rp.clone()
    }

    fn get_origin(&self) -> &String {
        &self.origin
    }

    fn get_relying_party_id(&self) -> String {
        self.id.clone()
    }
}

/// Helper to get a u2f instance from a u2f config, or `None` if there isn't one configured.
fn get_u2f(u2f: &Option<U2fConfig>) -> Option<u2f::U2f> {
    u2f.as_ref()
        .map(|cfg| u2f::U2f::new(cfg.appid.clone(), cfg.appid.clone()))
}

/// Helper to get a u2f instance from a u2f config.
///
/// This is outside of `TfaConfig` to not borrow its `&self`.
fn check_u2f(u2f: &Option<U2fConfig>) -> Result<u2f::U2f, Error> {
    get_u2f(u2f).ok_or_else(|| format_err!("no u2f configuration available"))
}

/// Helper to get a `Webauthn` instance from a `WebauthnConfig`, or `None` if there isn't one
/// configured.
fn get_webauthn(waconfig: &Option<WebauthnConfig>) -> Option<Webauthn<WebauthnConfig>> {
    waconfig.clone().map(Webauthn::new)
}

/// Helper to get a u2f instance from a u2f config.
///
/// This is outside of `TfaConfig` to not borrow its `&self`.
fn check_webauthn(waconfig: &Option<WebauthnConfig>) -> Result<Webauthn<WebauthnConfig>, Error> {
    get_webauthn(waconfig).ok_or_else(|| format_err!("no webauthn configuration available"))
}

/// TFA Configuration for this instance.
#[derive(Default, Deserialize, Serialize)]
pub struct TfaConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub u2f: Option<U2fConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub webauthn: Option<WebauthnConfig>,

    #[serde(skip_serializing_if = "TfaUsers::is_empty", default)]
    pub users: TfaUsers,
}

impl TfaConfig {
    /// Get a two factor authentication challenge for a user, if the user has TFA set up.
    pub fn login_challenge(&mut self, userid: &Userid) -> Result<Option<TfaChallenge>, Error> {
        match self.users.get_mut(userid) {
            Some(udata) => udata.challenge(
                userid,
                get_webauthn(&self.webauthn),
                get_u2f(&self.u2f).as_ref(),
            ),
            None => Ok(None),
        }
    }

    /// Get a u2f registration challenge.
    fn u2f_registration_challenge(
        &mut self,
        userid: &Userid,
        description: String,
    ) -> Result<String, Error> {
        let u2f = check_u2f(&self.u2f)?;

        self.users
            .entry(userid.clone())
            .or_default()
            .u2f_registration_challenge(userid, &u2f, description)
    }

    /// Finish a u2f registration challenge.
    fn u2f_registration_finish(
        &mut self,
        userid: &Userid,
        challenge: &str,
        response: &str,
    ) -> Result<String, Error> {
        let u2f = check_u2f(&self.u2f)?;

        match self.users.get_mut(userid) {
            Some(user) => user.u2f_registration_finish(userid, &u2f, challenge, response),
            None => bail!("no such challenge"),
        }
    }

    /// Get a webauthn registration challenge.
    fn webauthn_registration_challenge(
        &mut self,
        user: &Userid,
        description: String,
    ) -> Result<String, Error> {
        let webauthn = check_webauthn(&self.webauthn)?;

        self.users
            .entry(user.clone())
            .or_default()
            .webauthn_registration_challenge(webauthn, user, description)
    }

    /// Finish a webauthn registration challenge.
    fn webauthn_registration_finish(
        &mut self,
        userid: &Userid,
        challenge: &str,
        response: &str,
    ) -> Result<String, Error> {
        let webauthn = check_webauthn(&self.webauthn)?;

        let response: webauthn_rs::proto::RegisterPublicKeyCredential =
            serde_json::from_str(response)
                .map_err(|err| format_err!("error parsing challenge response: {}", err))?;

        match self.users.get_mut(userid) {
            Some(user) => user.webauthn_registration_finish(webauthn, userid, challenge, response),
            None => bail!("no such challenge"),
        }
    }

    /// Verify a TFA response.
    fn verify(
        &mut self,
        userid: &Userid,
        challenge: &TfaChallenge,
        response: TfaResponse,
    ) -> Result<(), Error> {
        match self.users.get_mut(userid) {
            Some(user) => match response {
                TfaResponse::Totp(value) => user.verify_totp(&value),
                TfaResponse::U2f(value) => match &challenge.u2f {
                    Some(challenge) => {
                        let u2f = check_u2f(&self.u2f)?;
                        user.verify_u2f(u2f, &challenge.challenge, value)
                    }
                    None => bail!("no u2f factor available for user '{}'", userid),
                },
                TfaResponse::Webauthn(value) => {
                    let webauthn = check_webauthn(&self.webauthn)?;
                    user.verify_webauthn(userid, webauthn, value)
                }
                TfaResponse::Recovery(value) => user.verify_recovery(&value),
            },
            None => bail!("no 2nd factor available for user '{}'", userid),
        }
    }

    /// Remove non-existent users.
    pub fn cleanup_users(&mut self, config: &proxmox::api::section_config::SectionConfigData) {
        use crate::config::user::User;
        self.users
            .retain(|user, _| config.lookup::<User>("user", user.as_str()).is_ok());
    }

    /// Remove a user. Returns `true` if the user actually existed.
    pub fn remove_user(&mut self, user: &Userid) -> bool {
        self.users.remove(user).is_some()
    }
}

#[api]
/// Over the API we only provide this part when querying a user's second factor list.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TfaInfo {
    /// The id used to reference this entry.
    pub id: String,

    /// User chosen description for this entry.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,

    /// Creation time of this entry as unix epoch.
    pub created: i64,

    /// Whether this TFA entry is currently enabled.
    #[serde(skip_serializing_if = "is_default_tfa_enable")]
    #[serde(default = "default_tfa_enable")]
    pub enable: bool,
}

impl TfaInfo {
    /// For recovery keys we have a fixed entry.
    pub(crate) fn recovery(created: i64) -> Self {
        Self {
            id: "recovery".to_string(),
            description: String::new(),
            enable: true,
            created,
        }
    }
}

/// A TFA entry for a user.
///
/// This simply connects a raw registration to a non optional descriptive text chosen by the user.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TfaEntry<T> {
    #[serde(flatten)]
    pub info: TfaInfo,

    /// The actual entry.
    entry: T,
}

impl<T> TfaEntry<T> {
    /// Create an entry with a description. The id will be autogenerated.
    fn new(description: String, entry: T) -> Self {
        Self {
            info: TfaInfo {
                id: Uuid::generate().to_string(),
                enable: true,
                description,
                created: proxmox::tools::time::epoch_i64(),
            },
            entry,
        }
    }
}

trait IsExpired {
    fn is_expired(&self, at_epoch: i64) -> bool;
}

/// A u2f registration challenge.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct U2fRegistrationChallenge {
    /// JSON formatted challenge string.
    challenge: String,

    /// The description chosen by the user for this registration.
    description: String,

    /// When the challenge was created as unix epoch. They are supposed to be short-lived.
    created: i64,
}

impl U2fRegistrationChallenge {
    pub fn new(challenge: String, description: String) -> Self {
        Self {
            challenge,
            description,
            created: proxmox::tools::time::epoch_i64(),
        }
    }
}

impl IsExpired for U2fRegistrationChallenge {
    fn is_expired(&self, at_epoch: i64) -> bool {
        self.created < at_epoch
    }
}

/// A webauthn registration challenge.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebauthnRegistrationChallenge {
    /// Server side registration state data.
    state: webauthn_rs::RegistrationState,

    /// While this is basically the content of a `RegistrationState`, the webauthn-rs crate doesn't
    /// make this public.
    challenge: String,

    /// The description chosen by the user for this registration.
    description: String,

    /// When the challenge was created as unix epoch. They are supposed to be short-lived.
    created: i64,
}

impl WebauthnRegistrationChallenge {
    pub fn new(
        state: webauthn_rs::RegistrationState,
        challenge: String,
        description: String,
    ) -> Self {
        Self {
            state,
            challenge,
            description,
            created: proxmox::tools::time::epoch_i64(),
        }
    }
}

impl IsExpired for WebauthnRegistrationChallenge {
    fn is_expired(&self, at_epoch: i64) -> bool {
        self.created < at_epoch
    }
}

/// A webauthn authentication challenge.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebauthnAuthChallenge {
    /// Server side authentication state.
    state: webauthn_rs::AuthenticationState,

    /// While this is basically the content of a `AuthenticationState`, the webauthn-rs crate
    /// doesn't make this public.
    challenge: String,

    /// When the challenge was created as unix epoch. They are supposed to be short-lived.
    created: i64,
}

impl WebauthnAuthChallenge {
    pub fn new(state: webauthn_rs::AuthenticationState, challenge: String) -> Self {
        Self {
            state,
            challenge,
            created: proxmox::tools::time::epoch_i64(),
        }
    }
}

impl IsExpired for WebauthnAuthChallenge {
    fn is_expired(&self, at_epoch: i64) -> bool {
        self.created < at_epoch
    }
}

/// Active TFA challenges per user, stored in `CHALLENGE_DATA_PATH`.
#[derive(Default, Deserialize, Serialize)]
pub struct TfaUserChallenges {
    /// Active u2f registration challenges for a user.
    ///
    /// Expired values are automatically filtered out while parsing the tfa configuration file.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[serde(deserialize_with = "filter_expired_challenge")]
    u2f_registrations: Vec<U2fRegistrationChallenge>,

    /// Active webauthn registration challenges for a user.
    ///
    /// Expired values are automatically filtered out while parsing the tfa configuration file.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[serde(deserialize_with = "filter_expired_challenge")]
    webauthn_registrations: Vec<WebauthnRegistrationChallenge>,

    /// Active webauthn registration challenges for a user.
    ///
    /// Expired values are automatically filtered out while parsing the tfa configuration file.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[serde(deserialize_with = "filter_expired_challenge")]
    webauthn_auths: Vec<WebauthnAuthChallenge>,
}

/// Container of `TfaUserChallenges` with the corresponding file lock guard.
///
/// TODO: Implement a general file lock guarded struct container in the `proxmox` crate.
pub struct TfaUserChallengeData {
    inner: TfaUserChallenges,
    path: PathBuf,
    lock: File,
}

impl TfaUserChallengeData {
    /// Build the path to the challenge data file for a user.
    fn challenge_data_path(userid: &Userid) -> PathBuf {
        PathBuf::from(format!("{}/{}", CHALLENGE_DATA_PATH, userid))
    }

    /// Load the user's current challenges with the intent to create a challenge (create the file
    /// if it does not exist), and keep a lock on the file.
    fn open(userid: &Userid) -> Result<Self, Error> {
        crate::tools::create_run_dir()?;
        let options = CreateOptions::new().perm(Mode::from_bits_truncate(0o0600));
        proxmox::tools::fs::create_path(CHALLENGE_DATA_PATH, Some(options.clone()), Some(options))
            .map_err(|err| {
                format_err!(
                    "failed to crate challenge data dir {:?}: {}",
                    CHALLENGE_DATA_PATH,
                    err
                )
            })?;

        let path = Self::challenge_data_path(userid);

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|err| format_err!("failed to create challenge file {:?}: {}", path, err))?;

        proxmox::tools::fs::lock_file(&mut file, true, None)?;

        // the file may be empty, so read to a temporary buffer first:
        let mut data = Vec::with_capacity(4096);

        file.read_to_end(&mut data).map_err(|err| {
            format_err!("failed to read challenge data for user {}: {}", userid, err)
        })?;

        let inner = if data.is_empty() {
            Default::default()
        } else {
            serde_json::from_slice(&data).map_err(|err| {
                format_err!(
                    "failed to parse challenge data for user {}: {}",
                    userid,
                    err
                )
            })?
        };

        Ok(Self {
            inner,
            path,
            lock: file,
        })
    }

    /// `open` without creating the file if it doesn't exist, to finish WA authentications.
    fn open_no_create(userid: &Userid) -> Result<Option<Self>, Error> {
        let path = Self::challenge_data_path(userid);
        let mut file = match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err.into()),
        };

        proxmox::tools::fs::lock_file(&mut file, true, None)?;

        let inner = serde_json::from_reader(&mut file).map_err(|err| {
            format_err!("failed to read challenge data for user {}: {}", userid, err)
        })?;

        Ok(Some(Self {
            inner,
            path,
            lock: file,
        }))
    }

    /// Rewind & truncate the file for an update.
    fn rewind(&mut self) -> Result<(), Error> {
        let pos = self.lock.seek(SeekFrom::Start(0))?;
        if pos != 0 {
            bail!(
                "unexpected result trying to rewind file, position is {}",
                pos
            );
        }

        proxmox::c_try!(unsafe { libc::ftruncate(self.lock.as_raw_fd(), 0) });

        Ok(())
    }

    /// Save the current data. Note that we do not replace the file here since we lock the file
    /// itself, as it is in `/run`, and the typical error case for this particular situation
    /// (machine loses power) simply prevents some login, but that'll probably fail anyway for
    /// other reasons then...
    ///
    /// This currently consumes selfe as we never perform more than 1 insertion/removal, and this
    /// way also unlocks early.
    fn save(mut self) -> Result<(), Error> {
        self.rewind()?;

        serde_json::to_writer(&mut &self.lock, &self.inner).map_err(|err| {
            format_err!("failed to update challenge file {:?}: {}", self.path, err)
        })?;

        Ok(())
    }

    /// Finish a u2f registration. The challenge should correspond to an output of
    /// `u2f_registration_challenge` (which is a stringified `RegistrationChallenge`). The response
    /// should come directly from the client.
    fn u2f_registration_finish(
        &mut self,
        u2f: &u2f::U2f,
        challenge: &str,
        response: &str,
    ) -> Result<TfaEntry<u2f::Registration>, Error> {
        let expire_before = proxmox::tools::time::epoch_i64() - CHALLENGE_TIMEOUT;

        let index = self
            .inner
            .u2f_registrations
            .iter()
            .position(|r| r.challenge == challenge)
            .ok_or_else(|| format_err!("no such challenge"))?;

        let reg = &self.inner.u2f_registrations[index];
        if reg.is_expired(expire_before) {
            bail!("no such challenge");
        }

        // the verify call only takes the actual challenge string, so we have to extract it
        // (u2f::RegistrationChallenge did not always implement Deserialize...)
        let chobj: Value = serde_json::from_str(challenge)
            .map_err(|err| format_err!("error parsing original registration challenge: {}", err))?;
        let challenge = chobj["challenge"]
            .as_str()
            .ok_or_else(|| format_err!("invalid registration challenge"))?;

        let (mut reg, description) = match u2f.registration_verify(challenge, response)? {
            None => bail!("verification failed"),
            Some(reg) => {
                let entry = self.inner.u2f_registrations.remove(index);
                (reg, entry.description)
            }
        };

        // we do not care about the attestation certificates, so don't store them
        reg.certificate.clear();

        Ok(TfaEntry::new(description, reg))
    }

    /// Finish a webauthn registration. The challenge should correspond to an output of
    /// `webauthn_registration_challenge`. The response should come directly from the client.
    fn webauthn_registration_finish(
        &mut self,
        webauthn: Webauthn<WebauthnConfig>,
        challenge: &str,
        response: webauthn_rs::proto::RegisterPublicKeyCredential,
        existing_registrations: &[TfaEntry<WebauthnCredential>],
    ) -> Result<TfaEntry<WebauthnCredential>, Error> {
        let expire_before = proxmox::tools::time::epoch_i64() - CHALLENGE_TIMEOUT;

        let index = self
            .inner
            .webauthn_registrations
            .iter()
            .position(|r| r.challenge == challenge)
            .ok_or_else(|| format_err!("no such challenge"))?;

        let reg = self.inner.webauthn_registrations.remove(index);
        if reg.is_expired(expire_before) {
            bail!("no such challenge");
        }

        let credential =
            webauthn.register_credential(response, reg.state, |id| -> Result<bool, ()> {
                Ok(existing_registrations
                    .iter()
                    .any(|cred| cred.entry.cred_id == *id))
            })?;

        Ok(TfaEntry::new(reg.description, credential))
    }
}

/// TFA data for a user.
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "kebab-case")]
pub struct TfaUserData {
    /// Totp keys for a user.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub(crate) totp: Vec<TfaEntry<Totp>>,

    /// Registered u2f tokens for a user.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub(crate) u2f: Vec<TfaEntry<u2f::Registration>>,

    /// Registered webauthn tokens for a user.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub(crate) webauthn: Vec<TfaEntry<WebauthnCredential>>,

    /// Recovery keys. (Unordered OTP values).
    #[serde(skip_serializing_if = "Recovery::option_is_empty", default)]
    pub(crate) recovery: Option<Recovery>,
}

impl TfaUserData {
    /// Shortcut to get the recovery entry only if it is not empty!
    pub fn recovery(&self) -> Option<&Recovery> {
        if Recovery::option_is_empty(&self.recovery) {
            None
        } else {
            self.recovery.as_ref()
        }
    }

    /// `true` if no second factors exist
    pub fn is_empty(&self) -> bool {
        self.totp.is_empty()
            && self.u2f.is_empty()
            && self.webauthn.is_empty()
            && self.recovery().is_none()
    }

    /// Find an entry by id, except for the "recovery" entry which we're currently treating
    /// specially.
    pub fn find_entry_mut<'a>(&'a mut self, id: &str) -> Option<&'a mut TfaInfo> {
        for entry in &mut self.totp {
            if entry.info.id == id {
                return Some(&mut entry.info);
            }
        }

        for entry in &mut self.webauthn {
            if entry.info.id == id {
                return Some(&mut entry.info);
            }
        }

        for entry in &mut self.u2f {
            if entry.info.id == id {
                return Some(&mut entry.info);
            }
        }

        None
    }

    /// Create a u2f registration challenge.
    ///
    /// The description is required at this point already mostly to better be able to identify such
    /// challenges in the tfa config file if necessary. The user otherwise has no access to this
    /// information at this point, as the challenge is identified by its actual challenge data
    /// instead.
    fn u2f_registration_challenge(
        &mut self,
        userid: &Userid,
        u2f: &u2f::U2f,
        description: String,
    ) -> Result<String, Error> {
        let challenge = serde_json::to_string(&u2f.registration_challenge()?)?;

        let mut data = TfaUserChallengeData::open(userid)?;
        data.inner
            .u2f_registrations
            .push(U2fRegistrationChallenge::new(
                challenge.clone(),
                description,
            ));
        data.save()?;

        Ok(challenge)
    }

    fn u2f_registration_finish(
        &mut self,
        userid: &Userid,
        u2f: &u2f::U2f,
        challenge: &str,
        response: &str,
    ) -> Result<String, Error> {
        let mut data = TfaUserChallengeData::open(userid)?;
        let entry = data.u2f_registration_finish(u2f, challenge, response)?;
        data.save()?;

        let id = entry.info.id.clone();
        self.u2f.push(entry);
        Ok(id)
    }

    /// Create a webauthn registration challenge.
    ///
    /// The description is required at this point already mostly to better be able to identify such
    /// challenges in the tfa config file if necessary. The user otherwise has no access to this
    /// information at this point, as the challenge is identified by its actual challenge data
    /// instead.
    fn webauthn_registration_challenge(
        &mut self,
        mut webauthn: Webauthn<WebauthnConfig>,
        userid: &Userid,
        description: String,
    ) -> Result<String, Error> {
        let cred_ids: Vec<_> = self
            .enabled_webauthn_entries()
            .map(|cred| cred.cred_id.clone())
            .collect();

        let userid_str = userid.to_string();
        let (challenge, state) = webauthn.generate_challenge_register_options(
            userid_str.as_bytes().to_vec(),
            userid_str.clone(),
            userid_str.clone(),
            Some(cred_ids),
            Some(UserVerificationPolicy::Discouraged),
        )?;

        let challenge_string = challenge.public_key.challenge.to_string();
        let challenge = serde_json::to_string(&challenge)?;

        let mut data = TfaUserChallengeData::open(userid)?;
        data.inner
            .webauthn_registrations
            .push(WebauthnRegistrationChallenge::new(
                state,
                challenge_string,
                description,
            ));
        data.save()?;

        Ok(challenge)
    }

    /// Finish a webauthn registration. The challenge should correspond to an output of
    /// `webauthn_registration_challenge`. The response should come directly from the client.
    fn webauthn_registration_finish(
        &mut self,
        webauthn: Webauthn<WebauthnConfig>,
        userid: &Userid,
        challenge: &str,
        response: webauthn_rs::proto::RegisterPublicKeyCredential,
    ) -> Result<String, Error> {
        let mut data = TfaUserChallengeData::open(userid)?;
        let entry =
            data.webauthn_registration_finish(webauthn, challenge, response, &self.webauthn)?;
        data.save()?;

        let id = entry.info.id.clone();
        self.webauthn.push(entry);
        Ok(id)
    }

    /// Generate a generic TFA challenge. See the [`TfaChallenge`] description for details.
    pub fn challenge(
        &mut self,
        userid: &Userid,
        webauthn: Option<Webauthn<WebauthnConfig>>,
        u2f: Option<&u2f::U2f>,
    ) -> Result<Option<TfaChallenge>, Error> {
        if self.is_empty() {
            return Ok(None);
        }

        Ok(Some(TfaChallenge {
            totp: self.totp.iter().any(|e| e.info.enable),
            recovery: RecoveryState::from(&self.recovery),
            webauthn: match webauthn {
                Some(webauthn) => self.webauthn_challenge(userid, webauthn)?,
                None => None,
            },
            u2f: match u2f {
                Some(u2f) => self.u2f_challenge(u2f)?,
                None => None,
            },
        }))
    }

    /// Helper to iterate over enabled totp entries.
    fn enabled_totp_entries(&self) -> impl Iterator<Item = &Totp> {
        self.totp
            .iter()
            .filter_map(|e| if e.info.enable { Some(&e.entry) } else { None })
    }

    /// Helper to iterate over enabled u2f entries.
    fn enabled_u2f_entries(&self) -> impl Iterator<Item = &u2f::Registration> {
        self.u2f
            .iter()
            .filter_map(|e| if e.info.enable { Some(&e.entry) } else { None })
    }

    /// Helper to iterate over enabled u2f entries.
    fn enabled_webauthn_entries(&self) -> impl Iterator<Item = &WebauthnCredential> {
        self.webauthn
            .iter()
            .filter_map(|e| if e.info.enable { Some(&e.entry) } else { None })
    }

    /// Generate an optional u2f challenge.
    fn u2f_challenge(&self, u2f: &u2f::U2f) -> Result<Option<U2fChallenge>, Error> {
        if self.u2f.is_empty() {
            return Ok(None);
        }

        let keys: Vec<u2f::RegisteredKey> = self
            .enabled_u2f_entries()
            .map(|registration| registration.key.clone())
            .collect();

        if keys.is_empty() {
            return Ok(None);
        }

        Ok(Some(U2fChallenge {
            challenge: u2f.auth_challenge()?,
            keys,
        }))
    }

    /// Generate an optional webauthn challenge.
    fn webauthn_challenge(
        &mut self,
        userid: &Userid,
        mut webauthn: Webauthn<WebauthnConfig>,
    ) -> Result<Option<webauthn_rs::proto::RequestChallengeResponse>, Error> {
        if self.webauthn.is_empty() {
            return Ok(None);
        }

        let creds: Vec<_> = self.enabled_webauthn_entries().map(Clone::clone).collect();

        if creds.is_empty() {
            return Ok(None);
        }

        let (challenge, state) = webauthn
            .generate_challenge_authenticate(creds, Some(UserVerificationPolicy::Discouraged))?;
        let challenge_string = challenge.public_key.challenge.to_string();
        let mut data = TfaUserChallengeData::open(userid)?;
        data.inner
            .webauthn_auths
            .push(WebauthnAuthChallenge::new(state, challenge_string));
        data.save()?;

        Ok(Some(challenge))
    }

    /// Verify a totp challenge. The `value` should be the totp digits as plain text.
    fn verify_totp(&self, value: &str) -> Result<(), Error> {
        let now = std::time::SystemTime::now();

        for entry in self.enabled_totp_entries() {
            if entry.verify(value, now, -1..=1)?.is_some() {
                return Ok(());
            }
        }

        bail!("totp verification failed");
    }

    /// Verify a u2f response.
    fn verify_u2f(
        &self,
        u2f: u2f::U2f,
        challenge: &u2f::AuthChallenge,
        response: Value,
    ) -> Result<(), Error> {
        let response: u2f::AuthResponse = serde_json::from_value(response)
            .map_err(|err| format_err!("invalid u2f response: {}", err))?;

        if let Some(entry) = self
            .enabled_u2f_entries()
            .find(|e| e.key.key_handle == response.key_handle())
        {
            if u2f
                .auth_verify_obj(&entry.public_key, &challenge.challenge, response)?
                .is_some()
            {
                return Ok(());
            }
        }

        bail!("u2f verification failed");
    }

    /// Verify a webauthn response.
    fn verify_webauthn(
        &mut self,
        userid: &Userid,
        mut webauthn: Webauthn<WebauthnConfig>,
        mut response: Value,
    ) -> Result<(), Error> {
        let expire_before = proxmox::tools::time::epoch_i64() - CHALLENGE_TIMEOUT;

        let challenge = match response
            .as_object_mut()
            .ok_or_else(|| format_err!("invalid response, must be a json object"))?
            .remove("challenge")
            .ok_or_else(|| format_err!("missing challenge data in response"))?
        {
            Value::String(s) => s,
            _ => bail!("invalid challenge data in response"),
        };

        let response: webauthn_rs::proto::PublicKeyCredential = serde_json::from_value(response)
            .map_err(|err| format_err!("invalid webauthn response: {}", err))?;

        let mut data = match TfaUserChallengeData::open_no_create(userid)? {
            Some(data) => data,
            None => bail!("no such challenge"),
        };

        let index = data
            .inner
            .webauthn_auths
            .iter()
            .position(|r| r.challenge == challenge)
            .ok_or_else(|| format_err!("no such challenge"))?;

        let challenge = data.inner.webauthn_auths.remove(index);
        if challenge.is_expired(expire_before) {
            bail!("no such challenge");
        }

        // we don't allow re-trying the challenge, so make the removal persistent now:
        data.save()
            .map_err(|err| format_err!("failed to save challenge file: {}", err))?;

        match webauthn.authenticate_credential(response, challenge.state)? {
            Some((_cred, _counter)) => Ok(()),
            None => bail!("webauthn authentication failed"),
        }
    }

    /// Verify a recovery key.
    ///
    /// NOTE: If successful, the key will automatically be removed from the list of available
    /// recovery keys, so the configuration needs to be saved afterwards!
    fn verify_recovery(&mut self, value: &str) -> Result<(), Error> {
        if let Some(r) = &mut self.recovery {
            if r.verify(value)? {
                return Ok(());
            }
        }
        bail!("recovery verification failed");
    }

    /// Add a new set of recovery keys. There can only be 1 set of keys at a time.
    fn add_recovery(&mut self) -> Result<Vec<String>, Error> {
        if self.recovery.is_some() {
            bail!("user already has recovery keys");
        }

        let (recovery, original) = Recovery::generate()?;

        self.recovery = Some(recovery);

        Ok(original)
    }
}

/// Recovery entries. We use HMAC-SHA256 with a random secret as a salted hash replacement.
#[derive(Deserialize, Serialize)]
pub struct Recovery {
    /// "Salt" used for the key HMAC.
    secret: String,

    /// Recovery key entries are HMACs of the original data. When used up they will become `None`
    /// since the user is presented an enumerated list of codes, so we know the indices of used and
    /// unused codes.
    entries: Vec<Option<String>>,

    /// Creation timestamp as a unix epoch.
    pub created: i64,
}

impl Recovery {
    /// Generate recovery keys and return the recovery entry along with the original string
    /// entries.
    fn generate() -> Result<(Self, Vec<String>), Error> {
        let mut secret = [0u8; 8];
        proxmox::sys::linux::fill_with_random_data(&mut secret)?;

        let mut this = Self {
            secret: AsHex(&secret).to_string(),
            entries: Vec::with_capacity(10),
            created: proxmox::tools::time::epoch_i64(),
        };

        let mut original = Vec::new();

        let mut key_data = [0u8; 80]; // 10 keys of 12 bytes
        proxmox::sys::linux::fill_with_random_data(&mut key_data)?;
        for b in key_data.chunks(8) {
            let entry = format!(
                "{}-{}-{}-{}",
                AsHex(&b[0..2]),
                AsHex(&b[2..4]),
                AsHex(&b[4..6]),
                AsHex(&b[6..8]),
            );

            this.entries.push(Some(this.hash(entry.as_bytes())?));
            original.push(entry);
        }

        Ok((this, original))
    }

    /// Perform HMAC-SHA256 on the data and return the result as a hex string.
    fn hash(&self, data: &[u8]) -> Result<String, Error> {
        let secret = PKey::hmac(self.secret.as_bytes())
            .map_err(|err| format_err!("error instantiating hmac key: {}", err))?;

        let mut signer = Signer::new(MessageDigest::sha256(), &secret)
            .map_err(|err| format_err!("error instantiating hmac signer: {}", err))?;

        let hmac = signer
            .sign_oneshot_to_vec(data)
            .map_err(|err| format_err!("error calculating hmac: {}", err))?;

        Ok(AsHex(&hmac).to_string())
    }

    /// Iterator over available keys.
    fn available(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().filter_map(Option::as_deref)
    }

    /// Count the available keys.
    fn count_available(&self) -> usize {
        self.available().count()
    }

    /// Convenience serde method to check if either the option is `None` or the content `is_empty`.
    fn option_is_empty(this: &Option<Self>) -> bool {
        this.as_ref()
            .map_or(true, |this| this.count_available() == 0)
    }

    /// Verify a key and remove it. Returns whether the key was valid. Errors on openssl errors.
    fn verify(&mut self, key: &str) -> Result<bool, Error> {
        let hash = self.hash(key.as_bytes())?;
        for entry in &mut self.entries {
            if entry.as_ref() == Some(&hash) {
                *entry = None;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Serde helper using our `FilteredVecVisitor` to filter out expired entries directly at load
/// time.
fn filter_expired_challenge<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + IsExpired,
{
    let expire_before = proxmox::tools::time::epoch_i64() - CHALLENGE_TIMEOUT;
    Ok(
        deserializer.deserialize_seq(crate::tools::serde_filter::FilteredVecVisitor::new(
            "a challenge entry",
            move |reg: &T| !reg.is_expired(expire_before),
        ))?,
    )
}

/// Get an optional TFA challenge for a user.
pub fn login_challenge(userid: &Userid) -> Result<Option<TfaChallenge>, Error> {
    let _lock = write_lock()?;

    let mut data = read()?;
    Ok(match data.login_challenge(userid)? {
        Some(challenge) => {
            write(&data)?;
            Some(challenge)
        }
        None => None,
    })
}

/// Add a TOTP entry for a user. Returns the ID.
pub fn add_totp(userid: &Userid, description: String, value: Totp) -> Result<String, Error> {
    let _lock = write_lock();
    let mut data = read()?;
    let entry = TfaEntry::new(description, value);
    let id = entry.info.id.clone();
    data.users
        .entry(userid.clone())
        .or_default()
        .totp
        .push(entry);
    write(&data)?;
    Ok(id)
}

/// Add recovery tokens for the user. Returns the token list.
pub fn add_recovery(userid: &Userid) -> Result<Vec<String>, Error> {
    let _lock = write_lock();

    let mut data = read()?;
    let out = data
        .users
        .entry(userid.clone())
        .or_default()
        .add_recovery()?;
    write(&data)?;
    Ok(out)
}

/// Add a u2f registration challenge for a user.
pub fn add_u2f_registration(userid: &Userid, description: String) -> Result<String, Error> {
    let _lock = crate::config::tfa::write_lock();
    let mut data = read()?;
    let challenge = data.u2f_registration_challenge(userid, description)?;
    write(&data)?;
    Ok(challenge)
}

/// Finish a u2f registration challenge for a user.
pub fn finish_u2f_registration(
    userid: &Userid,
    challenge: &str,
    response: &str,
) -> Result<String, Error> {
    let _lock = crate::config::tfa::write_lock();
    let mut data = read()?;
    let id = data.u2f_registration_finish(userid, challenge, response)?;
    write(&data)?;
    Ok(id)
}

/// Add a webauthn registration challenge for a user.
pub fn add_webauthn_registration(userid: &Userid, description: String) -> Result<String, Error> {
    let _lock = crate::config::tfa::write_lock();
    let mut data = read()?;
    let challenge = data.webauthn_registration_challenge(userid, description)?;
    write(&data)?;
    Ok(challenge)
}

/// Finish a webauthn registration challenge for a user.
pub fn finish_webauthn_registration(
    userid: &Userid,
    challenge: &str,
    response: &str,
) -> Result<String, Error> {
    let _lock = crate::config::tfa::write_lock();
    let mut data = read()?;
    let id = data.webauthn_registration_finish(userid, challenge, response)?;
    write(&data)?;
    Ok(id)
}

/// Verify a TFA challenge.
pub fn verify_challenge(
    userid: &Userid,
    challenge: &TfaChallenge,
    response: TfaResponse,
) -> Result<(), Error> {
    let _lock = crate::config::tfa::write_lock();
    let mut data = read()?;
    data.verify(userid, challenge, response)?;
    write(&data)?;
    Ok(())
}

/// Used to inform the user about the recovery code status.
///
/// This contains the available key indices.
#[derive(Clone, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct RecoveryState(Vec<usize>);

impl RecoveryState {
    fn is_unavailable(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&Option<Recovery>> for RecoveryState {
    fn from(r: &Option<Recovery>) -> Self {
        match r {
            Some(r) => Self::from(r),
            None => Self::default(),
        }
    }
}

impl From<&Recovery> for RecoveryState {
    fn from(r: &Recovery) -> Self {
        Self(
            r.entries
                .iter()
                .enumerate()
                .filter_map(|(idx, key)| if key.is_some() { Some(idx) } else { None })
                .collect(),
        )
    }
}

/// When sending a TFA challenge to the user, we include information about what kind of challenge
/// the user may perform. If webauthn credentials are available, a webauthn challenge will be
/// included.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct TfaChallenge {
    /// True if the user has TOTP devices.
    totp: bool,

    /// Whether there are recovery keys available.
    #[serde(skip_serializing_if = "RecoveryState::is_unavailable", default)]
    recovery: RecoveryState,

    /// If the user has any u2f tokens registered, this will contain the U2F challenge data.
    #[serde(skip_serializing_if = "Option::is_none")]
    u2f: Option<U2fChallenge>,

    /// If the user has any webauthn credentials registered, this will contain the corresponding
    /// challenge data.
    #[serde(skip_serializing_if = "Option::is_none", skip_deserializing)]
    webauthn: Option<webauthn_rs::proto::RequestChallengeResponse>,
}

/// Data used for u2f challenges.
#[derive(Deserialize, Serialize)]
pub struct U2fChallenge {
    /// AppID and challenge data.
    challenge: u2f::AuthChallenge,

    /// Available tokens/keys.
    keys: Vec<u2f::RegisteredKey>,
}

/// A user's response to a TFA challenge.
pub enum TfaResponse {
    Totp(String),
    U2f(Value),
    Webauthn(Value),
    Recovery(String),
}

impl std::str::FromStr for TfaResponse {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        Ok(if let Some(totp) = s.strip_prefix("totp:") {
            TfaResponse::Totp(totp.to_string())
        } else if let Some(u2f) = s.strip_prefix("u2f:") {
            TfaResponse::U2f(serde_json::from_str(u2f)?)
        } else if let Some(webauthn) = s.strip_prefix("webauthn:") {
            TfaResponse::Webauthn(serde_json::from_str(webauthn)?)
        } else if let Some(recovery) = s.strip_prefix("recovery:") {
            TfaResponse::Recovery(recovery.to_string())
        } else {
            bail!("invalid tfa response");
        })
    }
}

const fn default_tfa_enable() -> bool {
    true
}

const fn is_default_tfa_enable(v: &bool) -> bool {
    *v
}
