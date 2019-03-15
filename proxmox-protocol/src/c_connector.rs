//! C API for the `Connector`.

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};

use crate::Connector;

#[inline(always)]
fn with_errno<T>(err: c_int, value: T) -> T {
    errno::set_errno(errno::Errno(err));
    value
}

#[inline]
fn checkstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }

    let cstr = unsafe { CStr::from_ptr(ptr) };
    match cstr.to_str() {
        Ok(s) => Some(s.to_string()),
        Err(_) => None,
    }
}

#[inline(always)]
fn wrap_buildcall<F>(me: *mut Connector, func: F)
where
    F: FnOnce(Connector) -> Connector,
{
    let me = unsafe { &mut *me };
    let moved_me = std::mem::replace(me, unsafe { std::mem::uninitialized() });
    std::mem::forget(std::mem::replace(me, func(moved_me)));
}

/// Create a connector object.
///
/// Returns a valid pointer or `NULL` on error, with `errno` set.
///
/// Errors:
///   * `EINVAL`: a required parameter was `NULL` or contained invalid bytes.
#[no_mangle]
pub extern "C" fn proxmox_connector_new(
    user: *const c_char,
    server: *const c_char,
    store: *const c_char,
) -> *mut Connector {
    let (user, server, store) = match (checkstr(user), checkstr(server), checkstr(store)) {
        (Some(user), Some(server), Some(store)) => (user, server, store),
        _ => return with_errno(libc::EINVAL, std::ptr::null_mut()),
    };

    Box::leak(Box::new(Connector::new(user, server, store)))
}

/// If a connector is not required anymore and has not been used up via a call to
/// `proxmox_connector_connect`, this can be used to free the associated resources.
#[no_mangle]
pub extern "C" fn proxmox_connector_drop(me: *mut Connector) {
    unsafe { Box::from_raw(me) };
}

/// Use a password
///
/// Returns `0` on success, a negative `errno` value on error.
///
/// Errors:
///   * `EINVAL`: a required parameter was `NULL` or contained invalid bytes.
#[no_mangle]
pub extern "C" fn proxmox_connector_set_password(
    me: *mut Connector,
    password: *const c_char,
) -> c_int {
    let password = match checkstr(password) {
        Some(pw) =>  pw,
        _ => return -libc::EINVAL,
    };

    wrap_buildcall(me, move |me| me.password(password));

    0
}

/// Use an existing ticket.
///
/// Returns `0` on success, a negative `errno` value on error.
///
/// Errors:
///   * `EINVAL`: a required parameter was `NULL` or contained invalid bytes.
#[no_mangle]
pub extern "C" fn proxmox_connector_set_ticket(
    me: *mut Connector,
    ticket: *const c_char,
    token: *const c_char,
) -> c_int {
    let (ticket, token) = match (checkstr(ticket), checkstr(token)) {
        (Some(ticket), Some(token)) => (ticket, token),
        _ => return -libc::EINVAL,
    };

    wrap_buildcall(me, move |me| me.ticket(ticket, token));

    0
}

/// Change whether certificate validation should be used on the connector.
#[no_mangle]
pub extern "C" fn proxmox_connector_set_certificate_validation(me: *mut Connector, on: bool) {
    let me = unsafe { &mut *me };

    wrap_buildcall(me, move |me| me.certificate_validation(on));
}

/// Initiate the connection. This consumes the Connector, invalidating the pointer to it!
///
/// Returns a `ProxmoxBackup*`, or `NULL` on error.
#[no_mangle]
pub extern "C" fn proxmox_connector_connect(
    me: *mut Connector,
) -> *mut crate::c_client::CClient {
    let boxed = unsafe { Box::from_raw(me) };
    let me = *boxed;
    match me.do_connect() {
        Ok(stream) => {
            let mut client = crate::c_client::make_c_compatible_client(stream);
            match client.wait_for_handshake() {
                Ok(true) => crate::c_client::make_c_client(client),
                Ok(false) => {
                    // This is a synchronous blocking connection, so this should be impossible:
                    eprintln!("proxmox backup protocol error handshake did not complete?");
                    std::ptr::null_mut()
                }
                Err(err) => {
                    eprintln!("error during handshake with backup server: {}", err);
                    std::ptr::null_mut()
                }
            }
        }
        Err(err) => {
            eprintln!("error connecting to backup server: {}", err);
            std::ptr::null_mut()
        }
    }
}
