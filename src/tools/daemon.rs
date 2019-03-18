//! Helpers for daemons/services.

use std::ffi::CString;
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::panic::UnwindSafe;

use failure::*;
use nix::sys::signalfd::siginfo;
use tokio::prelude::*;

use crate::tools::fd_change_cloexec;
use crate::tools::signalfd::{SigSet, SignalFd};

// Unfortunately FnBox is nightly-only and Box<FnOnce> is unusable, so just use Box<Fn>...
pub type BoxedStoreFunc = Box<dyn Fn() -> Result<String, Error> + UnwindSafe + Send>;

/// Helper trait to "store" something in the environment to be re-used after re-executing the
/// service on a reload.
pub trait Reloadable: Sized {
    fn restore(var: &str) -> Result<Self, Error>;
    fn get_store_func(&self) -> BoxedStoreFunc;
}

/// Manages things to be stored and reloaded upon reexec.
/// Anything which should be restorable should be instantiated via this struct's `restore` method,
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
    pub fn restore<T, F>(&mut self, name: &'static str, or_create: F) -> Result<T, Error>
    where
        T: Reloadable,
        F: FnOnce() -> Result<T, Error>,
    {
        let res = match std::env::var(name) {
            Ok(varstr) => T::restore(&varstr)?,
            Err(std::env::VarError::NotPresent) => or_create()?,
            Err(_) => bail!("variable {} has invalid value", name),
        };

        self.pre_exec.push(PreExecEntry {
            name,
            store_fn: res.get_store_func(),
        });
        Ok(res)
    }

    fn pre_exec(self) -> Result<(), Error> {
        for item in self.pre_exec {
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

        // Start ourselves in the background:
        use nix::unistd::{fork, ForkResult};
        match fork() {
            Ok(ForkResult::Child) => {
                // At this point we call pre-exec helpers. We must be certain that if they fail for
                // whatever reason we can still call `_exit()`, so use catch_unwind.
                match std::panic::catch_unwind(move || self.do_exec(exe, new_args)) {
                    Ok(_) => eprintln!("do_exec returned unexpectedly!"),
                    Err(_) => eprintln!("panic in re-exec"),
                }
                // No matter how we managed to get here, this is the time where we bail out quickly:
                unsafe {
                    libc::_exit(-1)
                }
            }
            Ok(ForkResult::Parent { child }) => {
                eprintln!("forked off a new server (pid: {})", child);
                Ok(())
            }
            Err(e) => {
                eprintln!("fork() failed, restart delayed: {}", e);
                Ok(())
            }
        }
    }

    fn do_exec(self, exe: CString, args: Vec<CString>) -> Result<(), Error> {
        self.pre_exec()?;
        nix::unistd::setsid()?;
        nix::unistd::execvp(&exe, &args)?;
        Ok(())
    }
}

/// Provide a default signal handler for daemons (daemon & proxy).
/// When the first `SIGHUP` is received, the `reloader`'s `fork_restart` method will be
/// triggered. Any further `SIGHUP` is "passed through".
pub fn default_signalfd_stream<F>(
    reloader: Reloader,
    before_reload: F,
) -> Result<impl Stream<Item = siginfo, Error = Error>, Error>
where
    F: FnOnce() -> Result<(), Error>,
{
    use nix::sys::signal::{SigmaskHow, Signal, sigprocmask};

    // Block SIGHUP for *all* threads and use it for a signalfd handler:
    let mut sigs = SigSet::empty();
    sigs.add(Signal::SIGHUP);
    sigprocmask(SigmaskHow::SIG_BLOCK, Some(&sigs), None)?;

    let sigfdstream = SignalFd::new(&sigs)?;
    let mut reloader = Some(reloader);
    let mut before_reload = Some(before_reload);

    Ok(sigfdstream
        .filter_map(move |si| {
            // FIXME: logging should be left to the user of this:
            eprintln!("received signal: {}", si.ssi_signo);

            if si.ssi_signo == Signal::SIGHUP as u32 {
                // The firs time this happens we will try to start a new process which should take
                // over.
                if let Some(reloader) = reloader.take() {
                    if let Err(e) = (before_reload.take().unwrap())() {
                        return Some(Err(e));
                    }

                    match reloader.fork_restart() {
                        Ok(_) => return None,
                        Err(e) => return Some(Err(e)),
                    }
                }
            }

            // pass the rest through:
            Some(Ok(si))
        })
        // filter_map cannot produce errors, so we create Result<> items instead, iow:
        //   before: Stream<Item = siginfo, Error>
        //   after:  Stream<Item = Result<siginfo, Error>, Error>.
        // use and_then to lift out the wrapped result:
        .and_then(|si_res| si_res)
    )
}

// For now all we need to do is store and reuse a tcp listening socket:
impl Reloadable for tokio::net::TcpListener {
    // NOTE: The socket must not be closed when the store-function is called:
    // FIXME: We could become "independent" of the TcpListener and its reference to the file
    // descriptor by `dup()`ing it (and check if the listener still exists via kcmp()?)
    fn get_store_func(&self) -> BoxedStoreFunc {
        let fd = self.as_raw_fd();
        Box::new(move || {
            fd_change_cloexec(fd, false)?;
            Ok(fd.to_string())
        })
    }

    fn restore(var: &str) -> Result<Self, Error> {
        let fd = var.parse::<u32>()
            .map_err(|e| format_err!("invalid file descriptor: {}", e))?
            as RawFd;
        fd_change_cloexec(fd, true)?;
        Ok(Self::from_std(
            unsafe { std::net::TcpListener::from_raw_fd(fd) },
            &tokio::reactor::Handle::default(),
        )?)
    }
}
