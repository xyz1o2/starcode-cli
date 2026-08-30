use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Watchdog for ANR (Application Not Responding) detection.
///
/// This component monitors the responsiveness of the main UI thread.
/// It runs a separate thread that periodically checks the last heartbeat timestamp.
/// If the timestamp hasn't been updated within the threshold, it logs a warning.
///
/// Architecture notes:
/// - Uses atomic operations for lock-free cross-thread communication
/// - The `pet()` call should be made from the main UI loop at least once per iteration
/// - ANR detection is based on wall-clock time difference between checks
/// - The watchdog thread is a daemon thread (exits when main thread exits)
pub struct Watchdog {
    last_heartbeat: Arc<AtomicU64>,
    check_interval: Duration,
    timeout_threshold: Duration,
    running: Arc<AtomicU64>, // 0: stopped, 1: running
    anr_count: Arc<AtomicU64>,
}

impl Watchdog {
    /// Create a new Watchdog instance.
    ///
    /// # Arguments
    /// * `check_interval` - How often to check the heartbeat.
    /// * `timeout_threshold` - How long without a heartbeat before considering it ANR.
    pub fn new(check_interval: Duration, timeout_threshold: Duration) -> Self {
        Self {
            last_heartbeat: Arc::new(AtomicU64::new(Self::now_ms())),
            check_interval,
            timeout_threshold,
            running: Arc::new(AtomicU64::new(0)),
            anr_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Update the heartbeat timestamp to "now".
    /// Should be called frequently from the main UI thread.
    pub fn pet(&self) {
        self.last_heartbeat.store(Self::now_ms(), Ordering::Relaxed);
    }

    /// Get the number of ANR events detected since start.
    pub fn anr_count(&self) -> u64 {
        self.anr_count.load(Ordering::Relaxed)
    }

    /// Start the monitoring thread.
    pub fn spawn(&self) {
        if self.running.swap(1, Ordering::SeqCst) == 1 {
            return; // Already running
        }

        let last_heartbeat = self.last_heartbeat.clone();
        let check_interval = self.check_interval;
        let timeout_threshold_ms = self.timeout_threshold.as_millis() as u64;
        let running = self.running.clone();
        let anr_count = self.anr_count.clone();

        thread::Builder::new()
            .name("watchdog".into())
            .spawn(move || {
                while running.load(Ordering::Relaxed) == 1 {
                    thread::sleep(check_interval);

                    let last = last_heartbeat.load(Ordering::Relaxed);
                    let now = Self::now_ms();

                    if now > last && (now - last) > timeout_threshold_ms {
                        let delay = now - last;
                        anr_count.fetch_add(1, Ordering::Relaxed);
                        crate::utils::logging::append_debug_log_line(&format!(
                            "⚠️ [ANR Detected] Main thread unresponsive for {} ms (Threshold: {} ms)",
                            delay, timeout_threshold_ms
                        ));
                    }
                }
            })
            .ok(); // Ignore spawn errors — watchdog is non-critical
    }

    /// Stop the monitoring thread.
    pub fn stop(&self) {
        self.running.store(0, Ordering::SeqCst);
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

impl Default for Watchdog {
    fn default() -> Self {
        Self::new(Duration::from_secs(1), Duration::from_secs(5))
    }
}
