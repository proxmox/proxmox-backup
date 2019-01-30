use std::{io, mem};
use std::time::Duration;

use libc::{pid_t, clockid_t, c_int};

/// Timers can use various clocks. See `timer_create(2)`.
pub enum Clock {
    /// Use `CLOCK_REALTIME` for the timer.
    Realtime,
    /// Use `CLOCK_MONOTONIC` for the timer.
    Monotonic,
}

/// Strong thread-id type to prevent accidental conversion of pid_t.
pub struct Tid(pid_t);

/// Convenience helper to get the current thread ID suitable to pass to a
/// `TimerEvent::ThreadSignal` entry.
pub fn gettid() -> Tid {
    Tid(unsafe { libc::syscall(libc::SYS_gettid) } as pid_t)
}

/// Strong signal type which is more advanced than nix::sys::signal::Signal as
/// it doesn't prevent you from using signals that the nix crate is unaware
/// of...!
pub struct Signal(c_int);

impl Into<c_int> for Signal {
    fn into(self) -> c_int {
        self.0
    }
}

impl From<c_int> for Signal {
    fn from(v: c_int) -> Signal {
        Signal(v)
    }
}

/// When instantiating a Timer, it needs to have an event type associated with
/// it to be fired whenever the timer expires. Most of the time this will be a
/// `Signal`. Sometimes we need to be able to send signals to specific threads.
pub enum TimerEvent {
    /// This will act like passing `NULL` to `timer_create()`, which maps to
    /// using the same as `Signal(SIGALRM)`.
    None,

    /// When the timer expires, send a specific signal to the current process.
    Signal(Signal),

    /// When the timer expires, send a specific signal to a specific thread.
    ThreadSignal(Tid, Signal),

    /// Convenience value to send a signal to the current thread. This is
    /// equivalent to using `ThreadSignal(gettid(), signal)`.
    ThisThreadSignal(Signal),
}

// timer_t is a pointer type, so we create a strongly typed internal handle
// type for it
#[repr(C)]
struct InternalTimerT(u32);
type TimerT = *mut InternalTimerT;

// These wrappers are defined in -lrt.
#[link(name="rt")]
extern "C" {
    fn timer_create(
        clockid: clockid_t,
        evp: *mut libc::sigevent,
        timer: *mut TimerT,
        ) -> c_int;
    fn timer_delete(timer: TimerT) -> c_int;
    fn timer_settime(
        timerid: TimerT,
        flags: c_int,
        new_value: *const libc::itimerspec,
        old_value: *mut libc::itimerspec,
        ) -> c_int;
}

/// Represents a POSIX per-process timer as created via `timer_create(2)`.
pub struct Timer {
    timer: TimerT,
}

/// Timer specification used to arm a `Timer`.
pub struct TimerSpec {
    /// The timeout to the next timer event.
    pub value: Option<Duration>,

    /// When a timer expires, it may be automatically rearmed with another
    /// timeout. This will keep happening until this is explicitly disabled
    /// or the timer deleted.
    pub interval: Option<Duration>,
}

// Helpers to convert between libc::timespec and Option<Duration>
fn opt_duration_to_timespec(v: Option<Duration>) -> libc::timespec {
    match v {
        None => libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        },
        Some(value) => libc::timespec {
            tv_sec: value.as_secs() as i64,
            tv_nsec: value.subsec_nanos() as i64,
        },
    }
}

fn timespec_to_opt_duration(v: libc::timespec) -> Option<Duration> {
    if v.tv_sec == 0 && v.tv_nsec == 0 {
        None
    } else {
        Some(Duration::new(v.tv_sec as u64, v.tv_nsec as u32))
    }
}

impl TimerSpec {
    // Helpers to convert between TimerSpec and libc::itimerspec
    fn to_itimerspec(&self) -> libc::itimerspec {
        libc::itimerspec {
            it_value: opt_duration_to_timespec(self.value),
            it_interval: opt_duration_to_timespec(self.interval),
        }
    }

    fn from_itimerspec(ts: libc::itimerspec) -> Self {
        TimerSpec {
            value: timespec_to_opt_duration(ts.it_value),
            interval: timespec_to_opt_duration(ts.it_interval),
        }
    }

    /// Create an empty timer specification representing a disabled timer.
    pub fn new() -> Self {
        TimerSpec { value: None, interval: None }
    }

    /// Change the specification to have a specific value.
    pub fn value(self, value: Option<Duration>) -> Self {
        TimerSpec { value, interval: self.interval }
    }

    /// Change the specification to have a specific interval.
    pub fn interval(self, interval: Option<Duration>) -> Self {
        TimerSpec { value: self.value, interval }
    }
}

impl Timer {
    /// Create a Timer object governing a POSIX timer.
    pub fn create(clock: Clock, event: TimerEvent) -> io::Result<Timer> {
        // Map from our clock type to the libc id
        let clkid = match clock {
            Clock::Realtime => libc::CLOCK_REALTIME,
            Clock::Monotonic => libc::CLOCK_MONOTONIC,
        } as clockid_t;

        // Map the TimerEvent to libc::sigevent
        let mut ev: libc::sigevent = unsafe { mem::zeroed() };
        match event {
            TimerEvent::None => ev.sigev_notify = libc::SIGEV_NONE,
            TimerEvent::Signal(signo) => {
                ev.sigev_signo = signo.0;
                ev.sigev_notify = libc::SIGEV_SIGNAL;
            }
            TimerEvent::ThreadSignal(tid, signo) => {
                ev.sigev_signo = signo.0;
                ev.sigev_notify = libc::SIGEV_THREAD_ID;
                ev.sigev_notify_thread_id = tid.0;
            }
            TimerEvent::ThisThreadSignal(signo) => {
                ev.sigev_signo = signo.0;
                ev.sigev_notify = libc::SIGEV_THREAD_ID;
                ev.sigev_notify_thread_id = gettid().0;
            }
        }

        // Create the timer
        let mut timer: TimerT = unsafe { mem::zeroed() };
        let rc = unsafe { timer_create(clkid, &mut ev, &mut timer) };
        if rc != 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(Timer { timer })
        }
    }

    /// Arm a timer. This returns the previous timer specification.
    pub fn arm(&mut self, spec: TimerSpec) -> io::Result<TimerSpec>
    {
        let newspec = spec.to_itimerspec();
        let mut oldspec: libc::itimerspec = unsafe { mem::uninitialized() };

        let rc = unsafe {
            timer_settime(self.timer, 0, &newspec, &mut oldspec)
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(TimerSpec::from_itimerspec(oldspec))
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        unsafe {
            timer_delete(self.timer);
        }
    }
}

/// This is the signal number we use in our timeout implementations. We expect
/// the signal handler for this signal to never be replaced by some other
/// library. If this does happen, we need to find another signal. There should
/// be plenty.
/// Currently this is SIGRTMIN+4, the 5th real-time signal. glibc reserves the
/// first two for pthread internals.
pub const SIGTIMEOUT: Signal = Signal(32 + 4);

// Our timeout handler does exactly nothing. We only need it to interrupt
// system calls.
extern "C" fn sig_timeout_handler(_: c_int) {
}

// See setup_timeout_handler().
fn do_setup_timeout_handler() -> io::Result<()> {
    // Unfortunately nix::sys::signal::Signal cannot represent real time
    // signals, so we need to use libc instead...
    //
    // This WOULD be a nicer impl though:
    //nix::sys::signal::sigaction(
    //    SIGTIMEOUT,
    //    nix::sys::signal::SigAction::new(
    //        nix::sys::signal::SigHandler::Handler(sig_timeout_handler),
    //        nix::sys::signal::SaFlags::empty(),
    //        nix::sys::signal::SigSet::all()))
    //    .map(|_|())

    unsafe {
        let mut sa_mask: libc::sigset_t = mem::uninitialized();
        if libc::sigemptyset(&mut sa_mask) != 0 ||
           libc::sigaddset(&mut sa_mask, SIGTIMEOUT.0) != 0
        {
            return Err(io::Error::last_os_error());
        }

        let sa = libc::sigaction {
            sa_sigaction:
                // libc::sigaction uses `usize` for the function pointer...
                sig_timeout_handler as *const extern "C" fn(i32) as usize,
            sa_mask,
            sa_flags: 0,
            sa_restorer: None,
        };
        if libc::sigaction(SIGTIMEOUT.0, &sa, std::ptr::null_mut()) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

// The first time we unblock SIGTIMEOUT should cause approprate initialization:
static SETUP_TIMEOUT_HANDLER: std::sync::Once = std::sync::Once::new();

/// Setup our timeout-signal workflow. This establishes the signal handler for
/// our `SIGTIMEOUT` and should be called once during initialization.
#[inline]
pub fn setup_timeout_handler() {
    SETUP_TIMEOUT_HANDLER.call_once(|| {
        // We unwrap here.
        // If setting up this handler fails you have other problems already,
        // plus, if setting up fails you can't *use* it either, so everything
        // goes to die.
        do_setup_timeout_handler().unwrap();
    });
}

/// This guards the state of the timeout signal: We want it blocked usually.
pub struct TimeoutBlockGuard(bool);
impl Drop for TimeoutBlockGuard {
    fn drop(&mut self) {
        if self.0 {
            block_timeout_signal();
        } else {
            unblock_timeout_signal().forget();
        }
    }
}

impl TimeoutBlockGuard {
    /// Convenience helper to "forget" to restore the signal block mask.
    #[inline(always)]
    pub fn forget(self) {
        std::mem::forget(self);
    }

    /// Convenience helper to trigger the guard behavior immediately.
    #[inline(always)]
    pub fn trigger(self) {
        std::mem::drop(self); // be explicit here...
    }
}

/// Unblock the timeout signal for the current thread. By default we block the
/// signal this behavior should be restored when done using timeouts, therefor this
/// returns a guard:
#[inline(always)]
pub fn unblock_timeout_signal() -> TimeoutBlockGuard {
    // This calls std::sync::Once:
    setup_timeout_handler();
    //let mut set = nix::sys::signal::SigSet::empty();
    //set.add(SIGTIMEOUT.0);
    //set.thread_unblock()?;
    //Ok(TimeoutBlockGuard{})
    // Again, nix crate and its signal limitations...

    // NOTE:
    //   sigsetops(3) and pthread_sigmask(3) can only fail if invalid memory is
    //   passed to the kernel, or signal numbers are "invalid", since we know
    //   neither is the case we will panic on error...
    let was_blocked = unsafe {
        let mut mask: libc::sigset_t = mem::uninitialized();
        let mut oldset: libc::sigset_t = mem::uninitialized();
        if libc::sigemptyset(&mut mask) != 0
           || libc::sigaddset(&mut mask, SIGTIMEOUT.0) != 0
           || libc::pthread_sigmask(libc::SIG_UNBLOCK, &mask, &mut oldset) != 0
        {
            panic!("Impossibly failed to unblock SIGTIMEOUT");
            //return Err(io::Error::last_os_error());
        }

        libc::sigismember(&oldset, SIGTIMEOUT.0) == 1
    };
    TimeoutBlockGuard(was_blocked)
}

/// Block the timeout signal for the current thread. This is the default.
#[inline(always)]
pub fn block_timeout_signal() {
    //let mut set = nix::sys::signal::SigSet::empty();
    //set.add(SIGTIMEOUT);
    //set.thread_block()
    unsafe {
        let mut mask: libc::sigset_t = mem::uninitialized();
        if libc::sigemptyset(&mut mask) != 0
           || libc::sigaddset(&mut mask, SIGTIMEOUT.0) != 0
           || libc::pthread_sigmask(libc::SIG_BLOCK, &mask,
                                    std::ptr::null_mut()) != 0
        {
            panic!("Impossibly failed to block SIGTIMEOUT");
            //return Err(io::Error::last_os_error());
        }
    }
}
