//! Tokio-based watchdog that shuts down the VM if not pinged for TIMEOUT
use std::sync::atomic::{AtomicI64, Ordering};
use proxmox::tools::time::epoch_i64;

const TIMEOUT: i64 = 600; // seconds
static TRIGGERED: AtomicI64 = AtomicI64::new(0);

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
    TIMEOUT - (epoch_i64() - TRIGGERED.load(Ordering::Acquire))
}
