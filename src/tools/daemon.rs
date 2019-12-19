//! Helpers for daemons/services.

use std::ffi::CString;
use std::future::Future;
use std::os::raw::{c_char, c_int};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::panic::UnwindSafe;
use std::pin::Pin;
use std::task::{Context, Poll};

use failure::*;

use proxmox::tools::io::{ReadExt, WriteExt};

use crate::server;
use crate::tools::{fd_change_cloexec, self};

// Unfortunately FnBox is nightly-only and Box<FnOnce> is unusable, so just use Box<Fn>...
pub type BoxedStoreFunc = Box<dyn FnMut() -> Result<String, Error> + UnwindSafe + Send>;

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
}

// Currently we only need environment variables for storage, but in theory we could also add
// variants which need temporary files or pipes...
struct PreExecEntry {
    name: &'static str, // Feel free to change to String if necessary...
    store_fn: BoxedStoreFunc,
}

impl Reloader {
    pub fn new() -> Self {
        Self {
            pre_exec: Vec::new(),
        }
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
        // Get the path to our executable as CString
        let exe = CString::new(
            std::fs::read_link("/proc/self/exe")?
                .into_os_string()
                .as_bytes()
        )?;

        // Get our parameters as Vec<CString>
        let args = std::env::args_os();
        let mut new_args = Vec::with_capacity(args.len());
        for arg in args {
            new_args.push(CString::new(arg.as_bytes())?);
        }

        // Synchronisation pipe:
        let (pin, pout) = super::pipe()?;

        // Start ourselves in the background:
        use nix::unistd::{fork, ForkResult};
        match fork() {
            Ok(ForkResult::Child) => {
                // Double fork so systemd can supervise us without nagging...
                match fork() {
                    Ok(ForkResult::Child) => {
                        std::mem::drop(pin);
                        // At this point we call pre-exec helpers. We must be certain that if they fail for
                        // whatever reason we can still call `_exit()`, so use catch_unwind.
                        match std::panic::catch_unwind(move || {
                            let mut pout = unsafe {
                                std::fs::File::from_raw_fd(pout.into_raw_fd())
                            };
                            let pid = nix::unistd::Pid::this();
                            if let Err(e) = unsafe { pout.write_host_value(pid.as_raw()) } {
                                log::error!("failed to send new server PID to parent: {}", e);
                                unsafe {
                                    libc::_exit(-1);
                                }
                            }
                            std::mem::drop(pout);
                            self.do_exec(exe, new_args)
                        })
                        {
                            Ok(_) => eprintln!("do_exec returned unexpectedly!"),
                            Err(_) => eprintln!("panic in re-exec"),
                        }
                    }
                    Ok(ForkResult::Parent { child }) => {
                        std::mem::drop((pin, pout));
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
                std::mem::drop(pout);
                let mut pin = unsafe {
                    std::fs::File::from_raw_fd(pin.into_raw_fd())
                };
                let child = nix::unistd::Pid::from_raw(match unsafe { pin.read_le_value() } {
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
                Ok(())
            }
            Err(e) => {
                log::error!("fork() failed, restart delayed: {}", e);
                Ok(())
            }
        }
    }

    fn do_exec(self, exe: CString, args: Vec<CString>) -> Result<(), Error> {
        self.pre_exec()?;
        nix::unistd::setsid()?;
        let args: Vec<&std::ffi::CStr> = args.iter().map(|s| s.as_ref()).collect();
        nix::unistd::execvp(&exe, &args)?;
        Ok(())
    }
}

// For now all we need to do is store and reuse a tcp listening socket:
impl Reloadable for tokio::net::TcpListener {
    // NOTE: The socket must not be closed when the store-function is called:
    // FIXME: We could become "independent" of the TcpListener and its reference to the file
    // descriptor by `dup()`ing it (and check if the listener still exists via kcmp()?)
    fn get_store_func(&self) -> Result<BoxedStoreFunc, Error> {
        let mut fd_opt = Some(tools::Fd(
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

pub struct NotifyReady;

impl Future for NotifyReady {
    type Output = Result<(), Error>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context) -> Poll<Result<(), Error>> {
        systemd_notify(SystemdNotify::Ready)?;
        Poll::Ready(Ok(()))
    }
}

/// This creates a future representing a daemon which reloads itself when receiving a SIGHUP.
/// If this is started regularly, a listening socket is created. In this case, the file descriptor
/// number will be remembered in `PROXMOX_BACKUP_LISTEN_FD`.
/// If the variable already exists, its contents will instead be used to restore the listening
/// socket.  The finished listening socket is then passed to the `create_service` function which
/// can be used to setup the TLS and the HTTP daemon.
pub async fn create_daemon<F, S>(
    address: std::net::SocketAddr,
    create_service: F,
) -> Result<(), Error>
where
    F: FnOnce(tokio::net::TcpListener, NotifyReady) -> Result<S, Error>,
    S: Future<Output = ()>,
{
    let mut reloader = Reloader::new();

    let listener: tokio::net::TcpListener = reloader.restore(
        "PROXMOX_BACKUP_LISTEN_FD",
        move || async move { Ok(tokio::net::TcpListener::bind(&address).await?) },
    ).await?;

    create_service(listener, NotifyReady)?.await;

    let mut reloader = Some(reloader);

    crate::tools::request_shutdown(); // make sure we are in shutdown mode
    if server::is_reload_request() {
        log::info!("daemon reload...");
        if let Err(e) = systemd_notify(SystemdNotify::Reloading) {
            log::error!("failed to notify systemd about the state change: {}", e);
        }
        if let Err(e) = reloader.take().unwrap().fork_restart() {
            log::error!("error during reload: {}", e);
            let _ = systemd_notify(SystemdNotify::Status("error during reload".to_string()));
        }
    } else {
        log::info!("daemon shutting down...");
    }
    Ok(())
}

#[link(name = "systemd")]
extern "C" {
    fn sd_notify(unset_environment: c_int, state: *const c_char) -> c_int;
}

pub enum SystemdNotify {
    Ready,
    Reloading,
    Stopping,
    Status(String),
    MainPid(nix::unistd::Pid),
}

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
