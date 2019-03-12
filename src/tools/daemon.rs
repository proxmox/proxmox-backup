//! Helpers for daemons/services.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::panic::UnwindSafe;

use failure::*;

// Unfortunately FnBox is nightly-only and Box<FnOnce> is unusable, so just use Box<Fn>...
pub type BoxedStoreFunc = Box<dyn Fn() -> Result<String, Error> + UnwindSafe + Send>;

/// Helper trait to "store" something in the environment to be re-used after re-executing the
/// service on a reload.
pub trait ReexecContinue: Sized {
    fn restore(var: &str) -> Result<Self, Error>;
    fn get_store_func(&self) -> BoxedStoreFunc;
}

/// Manages things to be stored and reloaded upon reexec.
/// Anything which should be restorable should be instantiated via this struct's `restore` method,
pub struct ReexecStore {
    pre_exec: Vec<PreExecEntry>,
}

// Currently we only need environment variables for storage, but in theory we could also add
// variants which need temporary files or pipes...
struct PreExecEntry {
    name: &'static str, // Feel free to change to String if necessary...
    store_fn: BoxedStoreFunc,
}

impl ReexecStore {
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
        T: ReexecContinue,
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
