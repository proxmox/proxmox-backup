use std::io::{IsTerminal, Read};
use std::os::unix::io::{FromRawFd, RawFd};
use std::path::PathBuf;

use anyhow::{bail, format_err, Error};
use serde_json::Value;

use proxmox_schema::*;
use proxmox_sys::fs::file_get_contents;
use proxmox_sys::linux::tty;

use pbs_api_types::CryptMode;

pub const DEFAULT_ENCRYPTION_KEY_FILE_NAME: &str = "encryption-key.json";
pub const DEFAULT_MASTER_PUBKEY_FILE_NAME: &str = "master-public.pem";

pub const KEYFILE_SCHEMA: Schema =
    StringSchema::new("Path to encryption key. All data will be encrypted using this key.")
        .schema();

pub const KEYFD_SCHEMA: Schema =
    IntegerSchema::new("Pass an encryption key via an already opened file descriptor.")
        .minimum(0)
        .schema();

pub const MASTER_PUBKEY_FILE_SCHEMA: Schema = StringSchema::new(
    "Path to master public key. The encryption key used for a backup will be encrypted using this key and appended to the backup.")
    .schema();

pub const MASTER_PUBKEY_FD_SCHEMA: Schema =
    IntegerSchema::new("Pass a master public key via an already opened file descriptor.")
        .minimum(0)
        .schema();

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeySource {
    DefaultKey,
    Fd,
    Path(String),
}

pub fn format_key_source(source: &KeySource, key_type: &str) -> String {
    match source {
        KeySource::DefaultKey => format!("Using default {} key..", key_type),
        KeySource::Fd => format!("Using {} key from file descriptor..", key_type),
        KeySource::Path(path) => format!("Using {} key from '{}'..", key_type, path),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyWithSource {
    pub source: KeySource,
    pub key: Vec<u8>,
}

impl KeyWithSource {
    pub fn from_fd(key: Vec<u8>) -> Self {
        Self {
            source: KeySource::Fd,
            key,
        }
    }

    pub fn from_default(key: Vec<u8>) -> Self {
        Self {
            source: KeySource::DefaultKey,
            key,
        }
    }

    pub fn from_path(path: String, key: Vec<u8>) -> Self {
        Self {
            source: KeySource::Path(path),
            key,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct CryptoParams {
    pub mode: CryptMode,
    pub enc_key: Option<KeyWithSource>,
    // FIXME switch to openssl::rsa::rsa<openssl::pkey::Public> once that is Eq?
    pub master_pubkey: Option<KeyWithSource>,
}

pub fn crypto_parameters(param: &Value) -> Result<CryptoParams, Error> {
    do_crypto_parameters(param, false)
}

pub fn crypto_parameters_keep_fd(param: &Value) -> Result<CryptoParams, Error> {
    do_crypto_parameters(param, true)
}

fn do_crypto_parameters(param: &Value, keep_keyfd_open: bool) -> Result<CryptoParams, Error> {
    let keyfile = match param.get("keyfile") {
        Some(Value::String(keyfile)) => Some(keyfile),
        Some(_) => bail!("bad --keyfile parameter type"),
        None => None,
    };

    let key_fd = match param.get("keyfd") {
        Some(Value::Number(key_fd)) => Some(
            RawFd::try_from(
                key_fd
                    .as_i64()
                    .ok_or_else(|| format_err!("bad key fd: {:?}", key_fd))?,
            )
            .map_err(|err| format_err!("bad key fd: {:?}: {}", key_fd, err))?,
        ),
        Some(_) => bail!("bad --keyfd parameter type"),
        None => None,
    };

    let master_pubkey_file = match param.get("master-pubkey-file") {
        Some(Value::String(keyfile)) => Some(keyfile),
        Some(_) => bail!("bad --master-pubkey-file parameter type"),
        None => None,
    };

    let master_pubkey_fd = match param.get("master-pubkey-fd") {
        Some(Value::Number(key_fd)) => Some(
            RawFd::try_from(
                key_fd
                    .as_i64()
                    .ok_or_else(|| format_err!("bad master public key fd: {:?}", key_fd))?,
            )
            .map_err(|err| format_err!("bad public master key fd: {:?}: {}", key_fd, err))?,
        ),
        Some(_) => bail!("bad --master-pubkey-fd parameter type"),
        None => None,
    };

    let mode: Option<CryptMode> = match param.get("crypt-mode") {
        Some(mode) => Some(serde::Deserialize::deserialize(mode)?),
        None => None,
    };

    let key = match (keyfile, key_fd) {
        (None, None) => None,
        (Some(_), Some(_)) => bail!("--keyfile and --keyfd are mutually exclusive"),
        (Some(keyfile), None) => Some(KeyWithSource::from_path(
            keyfile.clone(),
            file_get_contents(keyfile)?,
        )),
        (None, Some(fd)) => {
            let mut input = unsafe { std::fs::File::from_raw_fd(fd) };
            let mut data = Vec::new();
            let _len: usize = input.read_to_end(&mut data).map_err(|err| {
                format_err!("error reading encryption key from fd {}: {}", fd, err)
            })?;
            if keep_keyfd_open {
                // don't close fd if requested, and try to reset seek position
                std::mem::forget(input);
                unsafe {
                    libc::lseek(fd, 0, libc::SEEK_SET);
                }
            }
            Some(KeyWithSource::from_fd(data))
        }
    };

    let master_pubkey = match (master_pubkey_file, master_pubkey_fd) {
        (None, None) => None,
        (Some(_), Some(_)) => bail!("--keyfile and --keyfd are mutually exclusive"),
        (Some(keyfile), None) => Some(KeyWithSource::from_path(
            keyfile.clone(),
            file_get_contents(keyfile)?,
        )),
        (None, Some(fd)) => {
            let input = unsafe { std::fs::File::from_raw_fd(fd) };
            let mut data = Vec::new();
            let _len: usize = { input }
                .read_to_end(&mut data)
                .map_err(|err| format_err!("error reading master key from fd {}: {}", fd, err))?;
            Some(KeyWithSource::from_fd(data))
        }
    };

    let res = match mode {
        // no crypt mode, enable encryption if keys are available
        None => match (key, master_pubkey) {
            // only default keys if available
            (None, None) => match read_optional_default_encryption_key()? {
                None => CryptoParams { mode: CryptMode::None, enc_key: None, master_pubkey: None },
                enc_key => {
                    let master_pubkey = read_optional_default_master_pubkey()?;
                    CryptoParams {
                        mode: CryptMode::Encrypt,
                        enc_key,
                        master_pubkey,
                    }
                },
            },

            // explicit master key, default enc key needed
            (None, master_pubkey) => match read_optional_default_encryption_key()? {
                None => bail!("--master-pubkey-file/--master-pubkey-fd specified, but no key available"),
                enc_key => {
                    CryptoParams {
                        mode: CryptMode::Encrypt,
                        enc_key,
                        master_pubkey,
                    }
                },
            },

            // explicit keyfile, maybe default master key
            (enc_key, None) => CryptoParams { mode: CryptMode::Encrypt, enc_key, master_pubkey: read_optional_default_master_pubkey()? },

            // explicit keyfile and master key
            (enc_key, master_pubkey) => CryptoParams { mode: CryptMode::Encrypt, enc_key, master_pubkey },
        },

        // explicitly disabled encryption
        Some(CryptMode::None) => match (key, master_pubkey) {
            // no keys => OK, no encryption
            (None, None) => CryptoParams { mode: CryptMode::None, enc_key: None, master_pubkey: None },

            // --keyfile and --crypt-mode=none
            (Some(_), _) => bail!("--keyfile/--keyfd and --crypt-mode=none are mutually exclusive"),

            // --master-pubkey-file and --crypt-mode=none
            (_, Some(_)) => bail!("--master-pubkey-file/--master-pubkey-fd and --crypt-mode=none are mutually exclusive"),
        },

        // explicitly enabled encryption
        Some(mode) => match (key, master_pubkey) {
            // no key, maybe master key
            (None, master_pubkey) => match read_optional_default_encryption_key()? {
                None => bail!("--crypt-mode without --keyfile and no default key file available"),
                enc_key => {
                    log::info!("Encrypting with default encryption key!");
                    let master_pubkey = match master_pubkey {
                        None => read_optional_default_master_pubkey()?,
                        master_pubkey => master_pubkey,
                    };

                    CryptoParams {
                        mode,
                        enc_key,
                        master_pubkey,
                    }
                },
            },

            // --keyfile and --crypt-mode other than none
            (enc_key, master_pubkey) => {
                let master_pubkey = match master_pubkey {
                    None => read_optional_default_master_pubkey()?,
                    master_pubkey => master_pubkey,
                };

                CryptoParams { mode, enc_key, master_pubkey }
            },
        },
    };

    Ok(res)
}

pub fn find_default_master_pubkey() -> Result<Option<PathBuf>, Error> {
    super::find_xdg_file(
        DEFAULT_MASTER_PUBKEY_FILE_NAME,
        "default master public key file",
    )
}

pub fn place_default_master_pubkey() -> Result<PathBuf, Error> {
    super::place_xdg_file(
        DEFAULT_MASTER_PUBKEY_FILE_NAME,
        "default master public key file",
    )
}

pub fn find_default_encryption_key() -> Result<Option<PathBuf>, Error> {
    super::find_xdg_file(
        DEFAULT_ENCRYPTION_KEY_FILE_NAME,
        "default encryption key file",
    )
}

pub fn place_default_encryption_key() -> Result<PathBuf, Error> {
    super::place_xdg_file(
        DEFAULT_ENCRYPTION_KEY_FILE_NAME,
        "default encryption key file",
    )
}

#[cfg(not(test))]
pub(crate) fn read_optional_default_encryption_key() -> Result<Option<KeyWithSource>, Error> {
    find_default_encryption_key()?
        .map(|path| file_get_contents(path).map(KeyWithSource::from_default))
        .transpose()
}

#[cfg(not(test))]
pub(crate) fn read_optional_default_master_pubkey() -> Result<Option<KeyWithSource>, Error> {
    find_default_master_pubkey()?
        .map(|path| file_get_contents(path).map(KeyWithSource::from_default))
        .transpose()
}

#[cfg(test)]
static mut TEST_DEFAULT_ENCRYPTION_KEY: Result<Option<Vec<u8>>, Error> = Ok(None);

#[cfg(test)]
pub(crate) fn read_optional_default_encryption_key() -> Result<Option<KeyWithSource>, Error> {
    // not safe when multiple concurrent test cases end up here!
    unsafe {
        match &TEST_DEFAULT_ENCRYPTION_KEY {
            Ok(Some(key)) => Ok(Some(KeyWithSource::from_default(key.clone()))),
            Ok(None) => Ok(None),
            Err(_) => bail!("test error"),
        }
    }
}

#[cfg(test)]
// not safe when multiple concurrent test cases end up here!
pub(crate) unsafe fn set_test_encryption_key(value: Result<Option<Vec<u8>>, Error>) {
    TEST_DEFAULT_ENCRYPTION_KEY = value;
}

#[cfg(test)]
static mut TEST_DEFAULT_MASTER_PUBKEY: Result<Option<Vec<u8>>, Error> = Ok(None);

#[cfg(test)]
pub(crate) fn read_optional_default_master_pubkey() -> Result<Option<KeyWithSource>, Error> {
    // not safe when multiple concurrent test cases end up here!
    unsafe {
        match &TEST_DEFAULT_MASTER_PUBKEY {
            Ok(Some(key)) => Ok(Some(KeyWithSource::from_default(key.clone()))),
            Ok(None) => Ok(None),
            Err(_) => bail!("test error"),
        }
    }
}

#[cfg(test)]
// not safe when multiple concurrent test cases end up here!
pub(crate) unsafe fn set_test_default_master_pubkey(value: Result<Option<Vec<u8>>, Error>) {
    TEST_DEFAULT_MASTER_PUBKEY = value;
}

pub fn get_encryption_key_password() -> Result<Vec<u8>, Error> {
    // fixme: implement other input methods

    if let Some(password) = super::get_secret_from_env("PBS_ENCRYPTION_PASSWORD")? {
        return Ok(password.as_bytes().to_vec());
    }

    // If we're on a TTY, query the user for a password
    if std::io::stdin().is_terminal() {
        return tty::read_password("Encryption Key Password: ");
    }

    bail!("no password input mechanism available");
}

#[cfg(test)]
fn create_testdir(name: &str) -> Result<String, Error> {
    // FIXME:
    //let mut testdir: PathBuf = format!("{}/testout", env!("CARGO_TARGET_TMPDIR")).into();
    let mut testdir: PathBuf = "./target/testout".to_string().into();
    testdir.push(std::module_path!());
    testdir.push(name);

    let _ = std::fs::remove_dir_all(&testdir);
    let _ = std::fs::create_dir_all(&testdir);

    Ok(testdir.to_str().unwrap().to_string())
}

#[test]
// WARNING: there must only be one test for crypto_parameters as the default key handling is not
// safe w.r.t. concurrency
fn test_crypto_parameters_handling() -> Result<(), Error> {
    use proxmox_sys::fs::{replace_file, CreateOptions};
    use serde_json::json;

    let some_key = vec![1; 1];
    let default_key = vec![2; 1];

    let some_master_key = vec![3; 1];
    let default_master_key = vec![4; 1];

    let testdir = create_testdir("key_source")?;

    let keypath = format!("{}/keyfile.test", testdir);
    let master_keypath = format!("{}/masterkeyfile.test", testdir);
    let invalid_keypath = format!("{}/invalid_keyfile.test", testdir);

    let no_key_res = CryptoParams {
        enc_key: None,
        master_pubkey: None,
        mode: CryptMode::None,
    };
    let some_key_res = CryptoParams {
        enc_key: Some(KeyWithSource::from_path(
            keypath.to_string(),
            some_key.clone(),
        )),
        master_pubkey: None,
        mode: CryptMode::Encrypt,
    };
    let some_key_some_master_res = CryptoParams {
        enc_key: Some(KeyWithSource::from_path(
            keypath.to_string(),
            some_key.clone(),
        )),
        master_pubkey: Some(KeyWithSource::from_path(
            master_keypath.to_string(),
            some_master_key.clone(),
        )),
        mode: CryptMode::Encrypt,
    };
    let some_key_default_master_res = CryptoParams {
        enc_key: Some(KeyWithSource::from_path(
            keypath.to_string(),
            some_key.clone(),
        )),
        master_pubkey: Some(KeyWithSource::from_default(default_master_key.clone())),
        mode: CryptMode::Encrypt,
    };

    let some_key_sign_res = CryptoParams {
        enc_key: Some(KeyWithSource::from_path(
            keypath.to_string(),
            some_key.clone(),
        )),
        master_pubkey: None,
        mode: CryptMode::SignOnly,
    };
    let default_key_res = CryptoParams {
        enc_key: Some(KeyWithSource::from_default(default_key.clone())),
        master_pubkey: None,
        mode: CryptMode::Encrypt,
    };
    let default_key_sign_res = CryptoParams {
        enc_key: Some(KeyWithSource::from_default(default_key.clone())),
        master_pubkey: None,
        mode: CryptMode::SignOnly,
    };

    replace_file(&keypath, &some_key, CreateOptions::default(), false)?;
    replace_file(
        &master_keypath,
        &some_master_key,
        CreateOptions::default(),
        false,
    )?;

    // no params, no default key == no key
    let res = crypto_parameters(&json!({}));
    assert_eq!(res.unwrap(), no_key_res);

    // keyfile param == key from keyfile
    let res = crypto_parameters(&json!({ "keyfile": keypath }));
    assert_eq!(res.unwrap(), some_key_res);

    // crypt mode none == no key
    let res = crypto_parameters(&json!({"crypt-mode": "none"}));
    assert_eq!(res.unwrap(), no_key_res);

    // crypt mode encrypt/sign-only, no keyfile, no default key == Error
    assert!(crypto_parameters(&json!({"crypt-mode": "sign-only"})).is_err());
    assert!(crypto_parameters(&json!({"crypt-mode": "encrypt"})).is_err());

    // crypt mode none with explicit key == Error
    assert!(crypto_parameters(&json!({"crypt-mode": "none", "keyfile": keypath})).is_err());

    // crypt mode sign-only/encrypt with keyfile == key from keyfile with correct mode
    let res = crypto_parameters(&json!({"crypt-mode": "sign-only", "keyfile": keypath}));
    assert_eq!(res.unwrap(), some_key_sign_res);
    let res = crypto_parameters(&json!({"crypt-mode": "encrypt", "keyfile": keypath}));
    assert_eq!(res.unwrap(), some_key_res);

    // invalid keyfile parameter always errors
    assert!(crypto_parameters(&json!({ "keyfile": invalid_keypath })).is_err());
    assert!(crypto_parameters(&json!({"keyfile": invalid_keypath, "crypt-mode": "none"})).is_err());
    assert!(
        crypto_parameters(&json!({"keyfile": invalid_keypath, "crypt-mode": "sign-only"})).is_err()
    );
    assert!(
        crypto_parameters(&json!({"keyfile": invalid_keypath, "crypt-mode": "encrypt"})).is_err()
    );

    // now set a default key
    unsafe {
        set_test_encryption_key(Ok(Some(default_key)));
    }

    // and repeat

    // no params but default key == default key
    let res = crypto_parameters(&json!({}));
    assert_eq!(res.unwrap(), default_key_res);

    // keyfile param == key from keyfile
    let res = crypto_parameters(&json!({ "keyfile": keypath }));
    assert_eq!(res.unwrap(), some_key_res);

    // crypt mode none == no key
    let res = crypto_parameters(&json!({"crypt-mode": "none"}));
    assert_eq!(res.unwrap(), no_key_res);

    // crypt mode encrypt/sign-only, no keyfile, default key == default key with correct mode
    let res = crypto_parameters(&json!({"crypt-mode": "sign-only"}));
    assert_eq!(res.unwrap(), default_key_sign_res);
    let res = crypto_parameters(&json!({"crypt-mode": "encrypt"}));
    assert_eq!(res.unwrap(), default_key_res);

    // crypt mode none with explicit key == Error
    assert!(crypto_parameters(&json!({"crypt-mode": "none", "keyfile": keypath})).is_err());

    // crypt mode sign-only/encrypt with keyfile == key from keyfile with correct mode
    let res = crypto_parameters(&json!({"crypt-mode": "sign-only", "keyfile": keypath}));
    assert_eq!(res.unwrap(), some_key_sign_res);
    let res = crypto_parameters(&json!({"crypt-mode": "encrypt", "keyfile": keypath}));
    assert_eq!(res.unwrap(), some_key_res);

    // invalid keyfile parameter always errors
    assert!(crypto_parameters(&json!({ "keyfile": invalid_keypath })).is_err());
    assert!(crypto_parameters(&json!({"keyfile": invalid_keypath, "crypt-mode": "none"})).is_err());
    assert!(
        crypto_parameters(&json!({"keyfile": invalid_keypath, "crypt-mode": "sign-only"})).is_err()
    );
    assert!(
        crypto_parameters(&json!({"keyfile": invalid_keypath, "crypt-mode": "encrypt"})).is_err()
    );

    // now make default key retrieval error
    unsafe {
        set_test_encryption_key(Err(format_err!("test error")));
    }

    // and repeat

    // no params, default key retrieval errors == Error
    assert!(crypto_parameters(&json!({})).is_err());

    // keyfile param == key from keyfile
    let res = crypto_parameters(&json!({ "keyfile": keypath }));
    assert_eq!(res.unwrap(), some_key_res);

    // crypt mode none == no key
    let res = crypto_parameters(&json!({"crypt-mode": "none"}));
    assert_eq!(res.unwrap(), no_key_res);

    // crypt mode encrypt/sign-only, no keyfile, default key error == Error
    assert!(crypto_parameters(&json!({"crypt-mode": "sign-only"})).is_err());
    assert!(crypto_parameters(&json!({"crypt-mode": "encrypt"})).is_err());

    // crypt mode none with explicit key == Error
    assert!(crypto_parameters(&json!({"crypt-mode": "none", "keyfile": keypath})).is_err());

    // crypt mode sign-only/encrypt with keyfile == key from keyfile with correct mode
    let res = crypto_parameters(&json!({"crypt-mode": "sign-only", "keyfile": keypath}));
    assert_eq!(res.unwrap(), some_key_sign_res);
    let res = crypto_parameters(&json!({"crypt-mode": "encrypt", "keyfile": keypath}));
    assert_eq!(res.unwrap(), some_key_res);

    // invalid keyfile parameter always errors
    assert!(crypto_parameters(&json!({ "keyfile": invalid_keypath })).is_err());
    assert!(crypto_parameters(&json!({"keyfile": invalid_keypath, "crypt-mode": "none"})).is_err());
    assert!(
        crypto_parameters(&json!({"keyfile": invalid_keypath, "crypt-mode": "sign-only"})).is_err()
    );
    assert!(
        crypto_parameters(&json!({"keyfile": invalid_keypath, "crypt-mode": "encrypt"})).is_err()
    );

    // now remove default key again
    unsafe {
        set_test_encryption_key(Ok(None));
    }
    // set a default master key
    unsafe {
        set_test_default_master_pubkey(Ok(Some(default_master_key)));
    }

    // and use an explicit master key
    assert!(crypto_parameters(&json!({ "master-pubkey-file": master_keypath })).is_err());
    // just a default == no key
    let res = crypto_parameters(&json!({}));
    assert_eq!(res.unwrap(), no_key_res);

    // keyfile param == key from keyfile
    let res = crypto_parameters(&json!({"keyfile": keypath, "master-pubkey-file": master_keypath}));
    assert_eq!(res.unwrap(), some_key_some_master_res);
    // same with fallback to default master key
    let res = crypto_parameters(&json!({ "keyfile": keypath }));
    assert_eq!(res.unwrap(), some_key_default_master_res);

    // crypt mode none == error
    assert!(crypto_parameters(
        &json!({"crypt-mode": "none", "master-pubkey-file": master_keypath})
    )
    .is_err());
    // with just default master key == no key
    let res = crypto_parameters(&json!({"crypt-mode": "none"}));
    assert_eq!(res.unwrap(), no_key_res);

    // crypt mode encrypt without enc key == error
    assert!(crypto_parameters(
        &json!({"crypt-mode": "encrypt", "master-pubkey-file": master_keypath})
    )
    .is_err());
    assert!(crypto_parameters(&json!({"crypt-mode": "encrypt"})).is_err());

    // crypt mode none with explicit key == Error
    assert!(crypto_parameters(
        &json!({"crypt-mode": "none", "keyfile": keypath, "master-pubkey-file": master_keypath})
    )
    .is_err());
    assert!(crypto_parameters(&json!({"crypt-mode": "none", "keyfile": keypath})).is_err());

    // crypt mode encrypt with keyfile == key from keyfile with correct mode
    let res = crypto_parameters(
        &json!({"crypt-mode": "encrypt", "keyfile": keypath, "master-pubkey-file": master_keypath}),
    );
    assert_eq!(res.unwrap(), some_key_some_master_res);
    let res = crypto_parameters(&json!({"crypt-mode": "encrypt", "keyfile": keypath}));
    assert_eq!(res.unwrap(), some_key_default_master_res);

    // invalid master keyfile parameter always errors when a key is passed, even with a valid
    // default master key
    assert!(
        crypto_parameters(&json!({"keyfile": keypath, "master-pubkey-file": invalid_keypath}))
            .is_err()
    );
    assert!(crypto_parameters(
        &json!({"keyfile": keypath, "master-pubkey-file": invalid_keypath,"crypt-mode": "none"})
    )
    .is_err());
    assert!(crypto_parameters(&json!({"keyfile": keypath, "master-pubkey-file": invalid_keypath,"crypt-mode": "sign-only"})).is_err());
    assert!(crypto_parameters(
        &json!({"keyfile": keypath, "master-pubkey-file": invalid_keypath,"crypt-mode": "encrypt"})
    )
    .is_err());

    Ok(())
}
