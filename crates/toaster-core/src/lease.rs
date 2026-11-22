use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Monotonic milliseconds since process start (never 0 after first call).
pub fn now_ms() -> u64 {
    epoch().elapsed().as_millis() as u64 + 1
}

/// Only accepted audio renews the lease. Lock-free; readable from any thread.
pub struct ConnectionLease {
    timeout_ms: u64,
    last: AtomicU64,
}

impl ConnectionLease {
    pub fn new(timeout: Duration) -> Self {
        ConnectionLease { timeout_ms: timeout.as_millis() as u64, last: AtomicU64::new(0) }
    }
    pub fn observe_at(&self, now_ms: u64) {
        self.last.store(now_ms, Ordering::Release);
    }
    pub fn observe(&self) {
        self.observe_at(now_ms());
    }
    pub fn clear(&self) {
        self.last.store(0, Ordering::Release);
    }
    pub fn is_active_at(&self, now_ms: u64) -> bool {
        let seen = self.last.load(Ordering::Acquire);
        seen != 0 && now_ms >= seen && now_ms - seen < self.timeout_ms
    }
    pub fn is_active(&self) -> bool {
        self.is_active_at(now_ms())
    }
}
