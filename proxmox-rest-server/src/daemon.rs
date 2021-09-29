//! Helpers to implement restartable daemons/services.

use std::ffi::CString;
use std::future::Future;
use std::io::{Read, Write};
use std::os::raw::{c_char, c_uchar, c_int};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::panic::UnwindSafe;
use std::path::PathBuf;

use anyhow::{bail, format_err, Error};
use futures::future::{self, Either};

use proxmox::tools::io::{ReadExt, WriteExt};
use proxmox::tools::fd::Fd;

use crate::fd_change_cloexec;

#[link(name = "systemd")]
extern "C" {
    fn sd_journal_stream_fd(identifier: *const c_uchar, priority: c_int, level_prefix: c_int) -> c_int;
}

// Unfortunately FnBox is nightly-only and Box<FnOnce> is unusable, so just use Box<Fn>...
type BoxedStoreFunc = Box<dyn FnMut() -> Result<String, Error> + UnwindSafe + Send>;

/// Helper trait to "store" something in the environment to be re-used after re-executing the
/// service on a reload.
pub trait Reloadable: Sized {
    fn restore(var: &str) -> Result<Self, Error>;
    fn get_store_func(&self) -> Result<BoxedStoreFunc, Error>;
}

/// Manages things to be stored and reloaded upon reexec.
/// Anything which should be restorable should be instantiated via this struct's `restore` method,
#[derive(Default)]
pub struct Reloader {
    pre_exec: Vec<PreExecEntry>,
    self_exe: PathBuf,
}

// Currently we only need environment variables for storage, but in theory we could also add
// variants which need temporary files or pipes...
struct PreExecEntry {
    name: &'static str, // Feel free to change to String if necessary...
    store_fn: BoxedStoreFunc,
}

impl Reloader {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            pre_exec: Vec::new(),

            // Get the path to our executable as PathBuf
            self_exe: std::fs::read_link("/proc/self/exe")?,
        })
    }

    /// Restore an object from an environment variable of the given name, or, if none exists, uses
    /// the function provided in the `or_create` parameter to instantiate the new "first" instance.
    ///
    /// Values created via this method will be remembered for later re-execution.
    pub async fn restore<T, F, U>(&mut self, name: &'static str, or_create: F) -> Result<T, Error>
    where
        T: Reloadable,
        F: FnOnce() -> U,
        U: Future<Output = Result<T, Error>>,
    {
        let res = match std::env::var(name) {
            Ok(varstr) => T::restore(&varstr)?,
            Err(std::env::VarError::NotPresent) => or_create().await?,
            Err(_) => bail!("variable {} has invalid value", name),
        };

        self.pre_exec.push(PreExecEntry {
            name,
            store_fn: res.get_store_func()?,
        });
        Ok(res)
    }

    fn pre_exec(self) -> Result<(), Error> {
        for mut item in self.pre_exec {
            std::env::set_var(item.name, (item.store_fn)()?);
        }
        Ok(())
    }

    pub fn fork_restart(self) -> Result<(), Error> {
        // Get our parameters as Vec<CString>
        let args = std::env::args_os();
        let mut new_args = Vec::with_capacity(args.len());
        for arg in args {
            new_args.push(CString::new(arg.as_bytes())?);
        }

        // Synchronisation pipe:
        let (pold, pnew) = super::socketpair()?;

        // Start ourselves in the background:
        use nix::unistd::{fork, ForkResult};
        match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                // Double fork so systemd can supervise us without nagging...
                match unsafe { fork() } {
                    Ok(ForkResult::Child) => {
                        std::mem::drop(pold);
                        // At this point we call pre-exec helpers. We must be certain that if they fail for
                        // whatever reason we can still call `_exit()`, so use catch_unwind.
                        match std::panic::catch_unwind(move || {
                            let mut pnew = unsafe {
                                std::fs::File::from_raw_fd(pnew.into_raw_fd())
                            };
                            let pid = nix::unistd::Pid::this();
                            if let Err(e) = unsafe { pnew.write_host_value(pid.as_raw()) } {
                                log::error!("failed to send new server PID to parent: {}", e);
                                unsafe {
                                    libc::_exit(-1);
                                }
                            }

                            let mut ok = [0u8];
                            if let Err(e) = pnew.read_exact(&mut ok) {
                                log::error!("parent vanished before notifying systemd: {}", e);
                                unsafe {
                                    libc::_exit(-1);
                                }
                            }
                            assert_eq!(ok[0], 1, "reload handshake should have sent a 1 byte");

                            std::mem::drop(pnew);

                            // Try to reopen STDOUT/STDERR journald streams to get correct PID in logs
                            let ident = CString::new(self.self_exe.file_name().unwrap().as_bytes()).unwrap();
                            let ident = ident.as_bytes();
                            let fd = unsafe { sd_journal_stream_fd(ident.as_ptr(), libc::LOG_INFO, 1) };
                            if fd >= 0 && fd != 1 {
                                let fd = proxmox::tools::fd::Fd(fd); // add drop handler
                                nix::unistd::dup2(fd.as_raw_fd(), 1)?;
                            } else {
                                log::error!("failed to update STDOUT journal redirection ({})", fd);
                            }
                            let fd = unsafe { sd_journal_stream_fd(ident.as_ptr(), libc::LOG_ERR, 1) };
                            if fd >= 0 && fd != 2 {
                                let fd = proxmox::tools::fd::Fd(fd); // add drop handler
                                nix::unistd::dup2(fd.as_raw_fd(), 2)?;
                            } else {
                                log::error!("failed to update STDERR journal redirection ({})", fd);
                            }

                            self.do_reexec(new_args)
                        })
                        {
                            Ok(Ok(())) => eprintln!("do_reexec returned!"),
                            Ok(Err(err)) => eprintln!("do_reexec failed: {}", err),
                            Err(_) => eprintln!("panic in re-exec"),
                        }
                    }
                    Ok(ForkResult::Parent { child }) => {
                        std::mem::drop((pold, pnew));
                        log::debug!("forked off a new server (second pid: {})", child);
                    }
                    Err(e) => log::error!("fork() failed, restart delayed: {}", e),
                }
                // No matter how we managed to get here, this is the time where we bail out quickly:
                unsafe {
                    libc::_exit(-1)
                }
            }
            Ok(ForkResult::Parent { child }) => {
                log::debug!("forked off a new server (first pid: {}), waiting for 2nd pid", child);
                std::mem::drop(pnew);
                let mut pold = unsafe {
                    std::fs::File::from_raw_fd(pold.into_raw_fd())
                };
                let child = nix::unistd::Pid::from_raw(match unsafe { pold.read_le_value() } {
                    Ok(v) => v,
                    Err(e) => {
                        log::error!("failed to receive pid of double-forked child process: {}", e);
                        // systemd will complain but won't kill the service...
                        return Ok(());
                    }
                });

                if let Err(e) = systemd_notify(SystemdNotify::MainPid(child)) {
                    log::error!("failed to notify systemd about the new main pid: {}", e);
                }

                // notify child that it is now the new main process:
                if let Err(e) = pold.write_all(&[1u8]) {
                    log::error!("child vanished during reload: {}", e);
                }

                Ok(())
            }
            Err(e) => {
                log::error!("fork() failed, restart delayed: {}", e);
                Ok(())
            }
        }
    }

    fn do_reexec(self, args: Vec<CString>) -> Result<(), Error> {
        let exe = CString::new(self.self_exe.as_os_str().as_bytes())?;
        self.pre_exec()?;
        nix::unistd::setsid()?;
        let args: Vec<&std::ffi::CStr> = args.iter().map(|s| s.as_ref()).collect();
        nix::unistd::execvp(&exe, &args)?;
        panic!("exec misbehaved");
    }
}

// For now all we need to do is store and reuse a tcp listening socket:
impl Reloadable for tokio::net::TcpListener {
    // NOTE: The socket must not be closed when the store-function is called:
    // FIXME: We could become "independent" of the TcpListener and its reference to the file
    // descriptor by `dup()`ing it (and check if the listener still exists via kcmp()?)
    fn get_store_func(&self) -> Result<BoxedStoreFunc, Error> {
        let mut fd_opt = Some(Fd(
            nix::fcntl::fcntl(self.as_raw_fd(), nix::fcntl::FcntlArg::F_DUPFD_CLOEXEC(0))?
        ));
        Ok(Box::new(move || {
            let fd = fd_opt.take().unwrap();
            fd_change_cloexec(fd.as_raw_fd(), false)?;
            Ok(fd.into_raw_fd().to_string())
        }))
    }

    fn restore(var: &str) -> Result<Self, Error> {
        let fd = var.parse::<u32>()
            .map_err(|e| format_err!("invalid file descriptor: {}", e))?
            as RawFd;
        fd_change_cloexec(fd, true)?;
        Ok(Self::from_std(
            unsafe { std::net::TcpListener::from_raw_fd(fd) },
        )?)
    }
}

/// This creates a future representing a daemon which reloads itself when receiving a SIGHUP.
/// If this is started regularly, a listening socket is created. In this case, the file descriptor
/// number will be remembered in `PROXMOX_BACKUP_LISTEN_FD`.
/// If the variable already exists, its contents will instead be used to restore the listening
/// socket.  The finished listening socket is then passed to the `create_service` function which
/// can be used to setup the TLS and the HTTP daemon. The returned future has to call
/// [systemd_notify] with [SystemdNotify::Ready] when the service is ready.
pub async fn create_daemon<F, S>(
    address: std::net::SocketAddr,
    create_service: F,
    service_name: &str,
) -> Result<(), Error>
where
    F: FnOnce(tokio::net::TcpListener) -> Result<S, Error>,
    S: Future<Output = Result<(), Error>>,
{
    let mut reloader = Reloader::new()?;

    let listener: tokio::net::TcpListener = reloader.restore(
        "PROXMOX_BACKUP_LISTEN_FD",
        move || async move { Ok(tokio::net::TcpListener::bind(&address).await?) },
    ).await?;

    let service = create_service(listener)?;

    let service = async move {
        if let Err(err) = service.await {
            log::error!("server error: {}", err);
        }
    };

    let server_future = Box::pin(service);
    let shutdown_future = crate::shutdown_future();

    let finish_future = match future::select(server_future, shutdown_future).await {
        Either::Left((_, _)) => {
            if !crate::shutdown_requested() {
                crate::request_shutdown(); // make sure we are in shutdown mode
            }
            None
        }
        Either::Right((_, server_future)) => Some(server_future),
    };

    let mut reloader = Some(reloader);

    if crate::is_reload_request() {
        log::info!("daemon reload...");
        if let Err(e) = systemd_notify(SystemdNotify::Reloading) {
            log::error!("failed to notify systemd about the state change: {}", e);
        }
        wait_service_is_state(service_name, "reloading").await?;
        if let Err(e) = reloader.take().unwrap().fork_restart() {
            log::error!("error during reload: {}", e);
            let _ = systemd_notify(SystemdNotify::Status("error during reload".to_string()));
        }
    } else {
        log::info!("daemon shutting down...");
    }

    if let Some(future) = finish_future {
        future.await;
    }

    // FIXME: this is a hack, replace with sd_notify_barrier when available
    if crate::is_reload_request() {
        wait_service_is_not_state(service_name, "reloading").await?;
    }

    log::info!("daemon shut down...");
    Ok(())
}

// hack, do not use if unsure!
async fn get_service_state(service: &str) -> Result<String, Error> {
    let text = match tokio::process::Command::new("systemctl")
        .args(&["is-active", service])
        .output()
        .await
    {
        Ok(output) => match String::from_utf8(output.stdout) {
            Ok(text) => text,
            Err(err) => bail!("output of 'systemctl is-active' not valid UTF-8 - {}", err),
        },
        Err(err) => bail!("executing 'systemctl is-active' failed - {}", err),
    };

    Ok(text.trim().trim_start().to_string())
}

async fn wait_service_is_state(service: &str, state: &str) -> Result<(), Error> {
    tokio::time::sleep(std::time::Duration::new(1, 0)).await;
    while get_service_state(service).await? != state {
        tokio::time::sleep(std::time::Duration::new(5, 0)).await;
    }
    Ok(())
}

async fn wait_service_is_not_state(service: &str, state: &str) -> Result<(), Error> {
    tokio::time::sleep(std::time::Duration::new(1, 0)).await;
    while get_service_state(service).await? == state {
        tokio::time::sleep(std::time::Duration::new(5, 0)).await;
    }
    Ok(())
}

#[link(name = "systemd")]
extern "C" {
    fn sd_notify(unset_environment: c_int, state: *const c_char) -> c_int;
}

/// Systemd sercice startup states (see: ``man sd_notify``)
pub enum SystemdNotify {
    Ready,
    Reloading,
    Stopping,
    Status(String),
    MainPid(nix::unistd::Pid),
}

/// Tells systemd the startup state of the service (see: ``man sd_notify``)
pub fn systemd_notify(state: SystemdNotify) -> Result<(), Error> {
    let message = match state {
        SystemdNotify::Ready => CString::new("READY=1"),
        SystemdNotify::Reloading => CString::new("RELOADING=1"),
        SystemdNotify::Stopping => CString::new("STOPPING=1"),
        SystemdNotify::Status(msg) => CString::new(format!("STATUS={}", msg)),
        SystemdNotify::MainPid(pid) => CString::new(format!("MAINPID={}", pid)),
    }?;
    let rc = unsafe { sd_notify(0, message.as_ptr()) };
    if rc < 0 {
        bail!(
            "systemd_notify failed: {}",
            std::io::Error::from_raw_os_error(-rc),
        );
    }
    Ok(())
}
