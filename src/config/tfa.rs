use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

use anyhow::{bail, format_err, Error};
use nix::sys::stat::Mode;

use proxmox_sys::error::SysError;
use proxmox_sys::fs::CreateOptions;
use proxmox_tfa::totp::Totp;

pub use proxmox_tfa::api::{
    TfaChallenge, TfaConfig, TfaResponse, UserChallengeAccess, WebauthnConfig,
    WebauthnConfigUpdater,
};

use pbs_api_types::{User, Userid};
use pbs_buildcfg::configdir;
use pbs_config::{open_backup_lockfile, BackupLockGuard};

const CONF_FILE: &str = configdir!("/tfa.json");
const LOCK_FILE: &str = configdir!("/tfa.json.lock");

const CHALLENGE_DATA_PATH: &str = pbs_buildcfg::rundir!("/tfa/challenges");

pub fn read_lock() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(LOCK_FILE, None, false)
}

pub fn write_lock() -> Result<BackupLockGuard, Error> {
    open_backup_lockfile(LOCK_FILE, None, true)
}

/// Read the TFA entries.
pub fn read() -> Result<TfaConfig, Error> {
    let file = match File::open(CONF_FILE) {
        Ok(file) => file,
        Err(ref err) if err.not_found() => return Ok(TfaConfig::default()),
        Err(err) => return Err(err.into()),
    };

    Ok(serde_json::from_reader(io::BufReader::new(file))?)
}

pub(crate) fn webauthn_config_digest(config: &WebauthnConfig) -> Result<[u8; 32], Error> {
    let digest_data = proxmox_serde::json::to_canonical_json(&serde_json::to_value(config)?)?;
    Ok(openssl::sha::sha256(&digest_data))
}

/// Get the webauthn config with a digest.
///
/// This is meant only for configuration updates, which currently only means webauthn updates.
/// Since this is meant to be done only once (since changes will lock out users), this should be
/// used rarely, since the digest calculation is currently a bit more involved.
pub fn webauthn_config() -> Result<Option<(WebauthnConfig, [u8; 32])>, Error> {
    Ok(match read()?.webauthn {
        Some(wa) => {
            let digest = webauthn_config_digest(&wa)?;
            Some((wa, digest))
        }
        None => None,
    })
}

/// Requires the write lock to be held.
pub fn write(data: &TfaConfig) -> Result<(), Error> {
    let options = CreateOptions::new().perm(Mode::from_bits_truncate(0o0600));

    let json = serde_json::to_vec(data)?;
    proxmox_sys::fs::replace_file(CONF_FILE, &json, options, true)
}

/// Cleanup non-existent users from the tfa config.
pub fn cleanup_users(data: &mut TfaConfig, config: &proxmox_section_config::SectionConfigData) {
    data.users
        .retain(|user, _| config.lookup::<User>("user", user.as_str()).is_ok());
}

/// Container of `TfaUserChallenges` with the corresponding file lock guard.
///
/// TODO: Implement a general file lock guarded struct container in the `proxmox` crate.
pub struct TfaUserChallengeData {
    inner: proxmox_tfa::api::TfaUserChallenges,
    path: PathBuf,
    lock: File,
}

fn challenge_data_path_str(userid: &str) -> PathBuf {
    PathBuf::from(format!("{}/{}", CHALLENGE_DATA_PATH, userid))
}

impl TfaUserChallengeData {
    /// Rewind & truncate the file for an update.
    fn rewind(&mut self) -> Result<(), Error> {
        let pos = self.lock.seek(SeekFrom::Start(0))?;
        if pos != 0 {
            bail!(
                "unexpected result trying to rewind file, position is {}",
                pos
            );
        }

        proxmox_sys::c_try!(unsafe { libc::ftruncate(self.lock.as_raw_fd(), 0) });

        Ok(())
    }

    /// Save the current data. Note that we do not replace the file here since we lock the file
    /// itself, as it is in `/run`, and the typical error case for this particular situation
    /// (machine loses power) simply prevents some login, but that'll probably fail anyway for
    /// other reasons then...
    fn save(&mut self) -> Result<(), Error> {
        self.rewind()?;

        serde_json::to_writer(io::BufWriter::new(&mut &self.lock), &self.inner).map_err(|err| {
            format_err!("failed to update challenge file {:?}: {}", self.path, err)
        })?;

        Ok(())
    }
}

/// Add a TOTP entry for a user. Returns the ID.
pub fn add_totp(userid: &Userid, description: String, value: Totp) -> Result<String, Error> {
    let _lock = write_lock();
    let mut data = read()?;
    let id = data.add_totp(userid.as_str(), description, value);
    write(&data)?;
    Ok(id)
}

/// Add recovery tokens for the user. Returns the token list.
pub fn add_recovery(userid: &Userid) -> Result<Vec<String>, Error> {
    let _lock = write_lock();

    let mut data = read()?;
    let out = data.add_recovery(userid.as_str())?;
    write(&data)?;
    Ok(out)
}

/// Add a u2f registration challenge for a user.
pub fn add_u2f_registration(userid: &Userid, description: String) -> Result<String, Error> {
    let _lock = crate::config::tfa::write_lock();
    let mut data = read()?;
    let challenge = data.u2f_registration_challenge(&UserAccess, userid.as_str(), description)?;
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
    let id = data.u2f_registration_finish(&UserAccess, userid.as_str(), challenge, response)?;
    write(&data)?;
    Ok(id)
}

/// Add a webauthn registration challenge for a user.
pub fn add_webauthn_registration(userid: &Userid, description: String) -> Result<String, Error> {
    let _lock = crate::config::tfa::write_lock();
    let mut data = read()?;
    let challenge =
        data.webauthn_registration_challenge(&UserAccess, userid.as_str(), description, None)?;
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
    let id =
        data.webauthn_registration_finish(&UserAccess, userid.as_str(), challenge, response, None)?;
    write(&data)?;
    Ok(id)
}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct UserAccess;

/// Build th
impl proxmox_tfa::api::OpenUserChallengeData for UserAccess {
    /// Load the user's current challenges with the intent to create a challenge (create the file
    /// if it does not exist), and keep a lock on the file.
    fn open(&self, userid: &str) -> Result<Box<dyn UserChallengeAccess>, Error> {
        crate::server::create_run_dir()?;
        let options = CreateOptions::new().perm(Mode::from_bits_truncate(0o0600));
        proxmox_sys::fs::create_path(CHALLENGE_DATA_PATH, Some(options.clone()), Some(options))
            .map_err(|err| {
                format_err!(
                    "failed to crate challenge data dir {:?}: {}",
                    CHALLENGE_DATA_PATH,
                    err
                )
            })?;

        let path = challenge_data_path_str(userid);

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|err| format_err!("failed to create challenge file {:?}: {}", path, err))?;

        proxmox_sys::fs::lock_file(&mut file, true, None)?;

        // the file may be empty, so read to a temporary buffer first:
        let mut data = Vec::with_capacity(4096);

        file.read_to_end(&mut data).map_err(|err| {
            format_err!("failed to read challenge data for user {}: {}", userid, err)
        })?;

        let inner = if data.is_empty() {
            Default::default()
        } else {
            match serde_json::from_slice(&data) {
                Ok(inner) => inner,
                Err(err) => {
                    eprintln!(
                        "failed to parse challenge data for user {}: {}",
                        userid, err
                    );
                    Default::default()
                }
            }
        };

        Ok(Box::new(TfaUserChallengeData {
            inner,
            path,
            lock: file,
        }))
    }

    /// `open` without creating the file if it doesn't exist, to finish WA authentications.
    fn open_no_create(&self, userid: &str) -> Result<Option<Box<dyn UserChallengeAccess>>, Error> {
        let path = challenge_data_path_str(userid);
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

        proxmox_sys::fs::lock_file(&mut file, true, None)?;

        let inner = serde_json::from_reader(io::BufReader::new(&mut file)).map_err(|err| {
            format_err!("failed to read challenge data for user {}: {}", userid, err)
        })?;

        Ok(Some(Box::new(TfaUserChallengeData {
            inner,
            path,
            lock: file,
        })))
    }

    /// `remove` user data if it exists.
    fn remove(&self, userid: &str) -> Result<bool, Error> {
        let path = challenge_data_path_str(userid);
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(err) if err.not_found() => Ok(false),
            Err(err) => Err(err.into()),
        }
    }

    fn enable_lockout(&self) -> bool {
        true
    }
}

impl proxmox_tfa::api::UserChallengeAccess for TfaUserChallengeData {
    fn get_mut(&mut self) -> &mut proxmox_tfa::api::TfaUserChallenges {
        &mut self.inner
    }

    fn save(&mut self) -> Result<(), Error> {
        TfaUserChallengeData::save(self)
    }
}

// shell completion helper
pub fn complete_tfa_id(_arg: &str, param: &HashMap<String, String>) -> Vec<String> {
    let mut results = Vec::new();

    let data = match read() {
        Ok(data) => data,
        Err(_err) => return results,
    };
    let user = match param
        .get("userid")
        .and_then(|user_name| data.users.get(user_name))
    {
        Some(user) => user,
        None => return results,
    };

    results.extend(user.totp.iter().map(|token| token.info.id.clone()));
    results.extend(user.u2f.iter().map(|token| token.info.id.clone()));
    results.extend(user.webauthn.iter().map(|token| token.info.id.clone()));
    results.extend(user.yubico.iter().map(|token| token.info.id.clone()));
    if user.recovery.is_some() {
        results.push("recovery".to_string());
    };

    results
}
