use std::ffi::CStr;

use anyhow::{bail, Error};

// from libcrypt1, 'lib/crypt.h.in'
const CRYPT_OUTPUT_SIZE: usize = 384;
const CRYPT_MAX_PASSPHRASE_SIZE: usize = 512;
const CRYPT_DATA_RESERVED_SIZE: usize = 767;
const CRYPT_DATA_INTERNAL_SIZE: usize = 30720;

#[repr(C)]
struct crypt_data {
    output: [libc::c_char; CRYPT_OUTPUT_SIZE],
    setting: [libc::c_char; CRYPT_OUTPUT_SIZE],
    input: [libc::c_char; CRYPT_MAX_PASSPHRASE_SIZE],
    reserved: [libc::c_char; CRYPT_DATA_RESERVED_SIZE],
    initialized: libc::c_char,
    internal: [libc::c_char; CRYPT_DATA_INTERNAL_SIZE],
}

pub fn crypt(password: &[u8], salt: &[u8]) -> Result<String, Error> {
    #[link(name = "crypt")]
    extern "C" {
        #[link_name = "crypt_r"]
        fn __crypt_r(
            key: *const libc::c_char,
            salt: *const libc::c_char,
            data: *mut crypt_data,
        ) -> *mut libc::c_char;
    }

    let mut data: crypt_data = unsafe { std::mem::zeroed() };
    for (i, c) in salt.iter().take(data.setting.len() - 1).enumerate() {
        data.setting[i] = *c as libc::c_char;
    }
    for (i, c) in password.iter().take(data.input.len() - 1).enumerate() {
        data.input[i] = *c as libc::c_char;
    }

    let res = unsafe {
        let status = __crypt_r(
            &data.input as *const _,
            &data.setting as *const _,
            &mut data as *mut _,
        );
        if status.is_null() {
            bail!("internal error: crypt_r returned null pointer");
        }
        CStr::from_ptr(&data.output as *const _)
    };
    Ok(String::from(res.to_str()?))
}

pub fn encrypt_pw(password: &str) -> Result<String, Error> {

    let salt = proxmox::sys::linux::random_data(8)?;
    let salt = format!("$5${}$", base64::encode_config(&salt, base64::CRYPT));

    crypt(password.as_bytes(), salt.as_bytes())
}

pub fn verify_crypt_pw(password: &str, enc_password: &str) -> Result<(), Error> {
    let verify = crypt(password.as_bytes(), enc_password.as_bytes())?;
    if verify != enc_password {
        bail!("invalid credentials");
    }
    Ok(())
}
