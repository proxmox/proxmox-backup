//! Proxmox Backup Server Authentication
//!
//! This library contains helper to authenticate users.

use std::process::{Command, Stdio};
use std::io::Write;
use std::ffi::{CString, CStr};

use base64;
use failure::*;
use serde_json::json;

pub trait ProxmoxAuthenticator {
    fn authenticate_user(&self, username: &str, password: &str) -> Result<(), Error>;
    fn store_password(&self, username: &str, password: &str) -> Result<(), Error>;
}

pub struct PAM();

impl ProxmoxAuthenticator for PAM {

    fn authenticate_user(&self, username: &str, password: &str) -> Result<(), Error> {
        let mut auth = pam::Authenticator::with_password("proxmox-backup-auth").unwrap();
        auth.get_handler().set_credentials(username, password);
        auth.authenticate()?;
        return Ok(());

    }

    fn store_password(&self, username: &str, password: &str) -> Result<(), Error> {
        let mut child = Command::new("passwd")
            .arg(username)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .or_else(|err| Err(format_err!("unable to set password for '{}' - execute passwd failed: {}", username, err)))?;

        // Note: passwd reads password twice from stdin (for verify)
        writeln!(child.stdin.as_mut().unwrap(), "{}\n{}", password, password)?;

        let output = child.wait_with_output()
            .or_else(|err| Err(format_err!("unable to set password for '{}' - wait failed: {}", username, err)))?;

        if !output.status.success() {
            bail!("unable to set password for '{}' - {}", username, String::from_utf8_lossy(&output.stderr));
        }

        Ok(())
    }
}

pub struct PBS();

pub fn crypt(password: &[u8], salt: &str) -> Result<String, Error> {

    #[link(name="crypt")]
    extern "C" {
        #[link_name = "crypt"]
        fn __crypt(key: *const libc::c_char, salt:  *const libc::c_char) -> * mut libc::c_char;
    }

    let salt = CString::new(salt)?;
    let password = CString::new(password)?;

    let res = unsafe {
        CStr::from_ptr(
            __crypt(
                password.as_c_str().as_ptr(),
                salt.as_c_str().as_ptr()
            )
        )
    };
    Ok(String::from(res.to_str()?))
}


pub fn encrypt_pw(password: &str) -> Result<String, Error> {

    let salt = proxmox::sys::linux::random_data(8)?;
    let salt = format!("$5${}$", base64::encode_config(&salt, base64::CRYPT));

    crypt(password.as_bytes(), &salt)
}

pub fn verify_crypt_pw(password: &str, enc_password: &str) -> Result<(), Error> {
    let verify = crypt(password.as_bytes(), enc_password)?;
    if &verify != enc_password {
        bail!("invalid credentials");
    }
    Ok(())
}

const SHADOW_CONFIG_FILENAME: &str = "/etc/proxmox-backup/shadow.json";

impl ProxmoxAuthenticator for PBS {

    fn authenticate_user(&self, username: &str, password: &str) -> Result<(), Error> {
        let data = proxmox::tools::fs::file_get_json(SHADOW_CONFIG_FILENAME, Some(json!({})))?;
        match data[username].as_str() {
            None => bail!("no password set"),
            Some(enc_password) => verify_crypt_pw(password, enc_password)?,
        }
        Ok(())
    }

    fn store_password(&self, username: &str, password: &str) -> Result<(), Error> {
        let enc_password = encrypt_pw(password)?;
        let mut data = proxmox::tools::fs::file_get_json(SHADOW_CONFIG_FILENAME, Some(json!({})))?;
        data[username] = enc_password.into();

        let mode = nix::sys::stat::Mode::from_bits_truncate(0o0600);
        let options =  proxmox::tools::fs::CreateOptions::new()
            .perm(mode)
            .owner(nix::unistd::ROOT)
            .group(nix::unistd::Gid::from_raw(0));

        let data = serde_json::to_vec_pretty(&data)?;
        proxmox::tools::fs::replace_file(SHADOW_CONFIG_FILENAME, &data, options)?;

        Ok(())
    }
}

pub fn parse_userid(userid: &str) -> Result<(String, String), Error> {
    let data: Vec<&str> = userid.rsplitn(2, '@').collect();

    if data.len() != 2 {
        bail!("userid '{}' has no realm", userid);
    }
    Ok((data[1].to_owned(), data[0].to_owned()))
}

pub fn lookup_authenticator(realm: &str) -> Result<Box<dyn ProxmoxAuthenticator>, Error> {
    match realm {
        "pam" => Ok(Box::new(PAM())),
        "pbs" => Ok(Box::new(PBS())),
        _ => bail!("unknown realm '{}'", realm),
    }
}

pub fn authenticate_user(userid: &str, password: &str) -> Result<(), Error> {
    let (username, realm) = parse_userid(userid)?;

    let (user_config, _digest) = crate::config::user::config()?;
    let user: Result<crate::config::user::User, Error> = user_config.lookup("user", userid);
    match user {
        Ok(user) => {
            if let Some(false) = user.enable {
                bail!("account disabled");
            }
            if let Some(expire) = user.expire {
                if expire > 0 {
                    let now = unsafe { libc::time(std::ptr::null_mut()) };
                    if expire <= now {
                        bail!("account expired");
                    }
                }
            }
        },
        Err(_) => bail!("no such user"),
    }

    lookup_authenticator(&realm)?
    .authenticate_user(&username, password)
}
