use std::time::Duration;

#[derive(Debug, Clone)]
pub struct MonitorConfig {
    pub sample_interval: Duration,
    pub sample_interval_min: Duration,
    pub sample_interval_max: Duration,
    pub stale_after: Duration,
    pub disconnected_after: Duration,
    pub poll_timeout: Duration,
    pub error_threshold: u32,
    pub auto_tune: bool,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            sample_interval: Duration::from_secs(1),
            sample_interval_min: Duration::from_secs(1),
            sample_interval_max: Duration::from_secs(3),
            stale_after: Duration::from_millis(2500),
            disconnected_after: Duration::from_millis(5000),
            poll_timeout: Duration::from_millis(700),
            error_threshold: 3,
            auto_tune: true,
        }
    }
}
