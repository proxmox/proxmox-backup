//! Tokio-based watchdog that shuts down the VM if not pinged for TIMEOUT
use std::sync::atomic::{AtomicI64, Ordering};

use proxmox_time::epoch_i64;

const TIMEOUT: i64 = 600; // seconds
static TRIGGERED: AtomicI64 = AtomicI64::new(0);
static INHIBITORS: AtomicI64 = AtomicI64::new(0);

pub struct WatchdogInhibitor {}

fn handle_expired() -> ! {
    use nix::sys::reboot;
    println!("watchdog expired, shutting down");
    let err = reboot::reboot(reboot::RebootMode::RB_POWER_OFF).unwrap_err();
    println!("'reboot' syscall failed: {}", err);
    std::process::exit(1);
}

async fn watchdog_loop() {
    use tokio::time::{sleep, Duration};
    loop {
        let remaining = watchdog_remaining();
        if remaining <= 0 {
            handle_expired();
        }
        sleep(Duration::from_secs(remaining as u64)).await;
    }
}

/// Initialize watchdog
pub fn watchdog_init() {
    watchdog_ping();
    tokio::spawn(watchdog_loop());
}

/// Trigger watchdog keepalive
pub fn watchdog_ping() {
    TRIGGERED.fetch_max(epoch_i64(), Ordering::AcqRel);
}

/// Returns the remaining time before watchdog expiry in seconds
pub fn watchdog_remaining() -> i64 {
    if INHIBITORS.load(Ordering::Acquire) > 0 {
        TIMEOUT
    } else {
        TIMEOUT - (epoch_i64() - TRIGGERED.load(Ordering::Acquire))
    }
}

/// Returns an object that inhibts watchdog expiry for its lifetime, it will issue a ping on Drop
pub fn watchdog_inhibit() -> WatchdogInhibitor {
    let prev = INHIBITORS.fetch_add(1, Ordering::AcqRel);
    log::info!("Inhibit added: {}", prev + 1);
    WatchdogInhibitor {}
}

impl Drop for WatchdogInhibitor {
    fn drop(&mut self) {
        watchdog_ping();
        let prev = INHIBITORS.fetch_sub(1, Ordering::AcqRel);
        log::info!("Inhibit dropped: {}", prev - 1);
    }
}
