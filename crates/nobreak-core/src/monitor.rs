use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::time::timeout;

use crate::config::MonitorConfig;
use crate::driver::{DeviceInfo, DriverError, UpsDriver};
use crate::snapshot::{Freshness, MonitorStatus, Snapshot, SnapshotDevice, SnapshotQuality, Transport};

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConnectionState {
    Disconnected,
    Connecting,
    Streaming,
    Degraded,
    Reconnecting,
}

pub struct Monitor<D: UpsDriver> {
    driver: D,
    config: MonitorConfig,
    state: ConnectionState,
    target_id: Option<String>,
    current: Option<DeviceInfo>,
    errors_in_row: u32,
    reads_ok: u64,
    reads_err: u64,
    reconnects: u64,
    effective_interval: Duration,
    process_start: Instant,
    last_ok_instant: Option<Instant>,
    last_ok_ts: Option<chrono::DateTime<Utc>>,
}

impl<D: UpsDriver> Monitor<D> {
    pub fn new(driver: D, config: MonitorConfig, target_id: Option<String>) -> Self {
        Self {
            driver,
            config: config.clone(),
            state: ConnectionState::Disconnected,
            target_id,
            current: None,
            errors_in_row: 0,
            reads_ok: 0,
            reads_err: 0,
            reconnects: 0,
            effective_interval: config.sample_interval,
            process_start: Instant::now(),
            last_ok_instant: None,
            last_ok_ts: None,
        }
    }

    pub fn effective_interval(&self) -> Duration {
        self.effective_interval
    }

    pub async fn discover(&mut self) -> Result<Vec<DeviceInfo>, DriverError> {
        self.driver.discover().await
    }

    pub async fn ensure_connected(&mut self) -> Result<DeviceInfo, DriverError> {
        self.state = ConnectionState::Connecting;
        let device = self.driver.connect(self.target_id.as_deref()).await?;
        self.current = Some(device.clone());
        self.state = ConnectionState::Streaming;
        Ok(device)
    }

    pub async fn tick(&mut self) -> Snapshot {
        if !self.driver.is_connected() {
            match self.ensure_connected().await {
                Ok(_) => {}
                Err(err) => {
                    self.reads_err += 1;
                    self.errors_in_row += 1;
                    self.state = ConnectionState::Disconnected;
                    return self.disconnected_snapshot(err.to_string(), 0, BTreeMap::new());
                }
            }
        }

        let started = Instant::now();
        let timed = timeout(self.config.poll_timeout, self.driver.read()).await;

        match timed {
            Ok(Ok(read_result)) => {
                self.reads_ok += 1;
                self.errors_in_row = 0;
                let rtt = started.elapsed();
                self.last_ok_instant = Some(Instant::now());
                self.last_ok_ts = Some(Utc::now());
                self.state = ConnectionState::Streaming;

                if self.config.auto_tune {
                    self.tune_interval(rtt, true);
                }

                self.connected_snapshot(
                    read_result.status_code,
                    read_result.failures,
                    read_result.vars,
                    rtt,
                )
            }
            Ok(Err(err)) => {
                self.reads_err += 1;
                self.errors_in_row += 1;

                if self.config.auto_tune {
                    self.tune_interval(self.config.poll_timeout, false);
                }

                let should_reconnect = self.errors_in_row >= self.config.error_threshold;
                if should_reconnect {
                    self.state = ConnectionState::Reconnecting;
                    let _ = self.driver.disconnect().await;
                    self.reconnects += 1;
                    self.current = None;
                } else {
                    self.state = ConnectionState::Degraded;
                }

                self.disconnected_snapshot(err.to_string(), started.elapsed().as_millis(), BTreeMap::new())
            }
            Err(_) => {
                self.reads_err += 1;
                self.errors_in_row += 1;

                if self.config.auto_tune {
                    self.tune_interval(self.config.poll_timeout, false);
                }

                if self.errors_in_row >= self.config.error_threshold {
                    self.state = ConnectionState::Reconnecting;
                    let _ = self.driver.disconnect().await;
                    self.reconnects += 1;
                    self.current = None;
                } else {
                    self.state = ConnectionState::Degraded;
                }

                self.disconnected_snapshot("timeout".to_string(), self.config.poll_timeout.as_millis(), BTreeMap::new())
            }
        }
    }

    fn tune_interval(&mut self, rtt: Duration, ok: bool) {
        if !ok {
            self.effective_interval = (self.effective_interval + Duration::from_millis(250))
                .min(self.config.sample_interval_max);
            return;
        }

        let threshold = self.effective_interval.mul_f64(0.6);
        if rtt > threshold {
            self.effective_interval = (self.effective_interval + Duration::from_millis(200))
                .min(self.config.sample_interval_max);
            return;
        }

        if self.reads_ok % 30 == 0 {
            self.effective_interval = self
                .effective_interval
                .saturating_sub(Duration::from_millis(100))
                .max(self.config.sample_interval_min);
        }
    }

    fn connected_snapshot(
        &self,
        status_code: String,
        failures: Vec<String>,
        vars: BTreeMap<String, serde_json::Value>,
        rtt: Duration,
    ) -> Snapshot {
        let now = Utc::now();
        let age = 0_u128;

        Snapshot {
            ts: now,
            mono_ms: self.process_start.elapsed().as_millis(),
            device: self.snapshot_device(true),
            freshness: Freshness {
                rtt_ms: rtt.as_millis(),
                age_ms: age,
                stale: false,
                last_ok_ts: self.last_ok_ts,
            },
            status: MonitorStatus {
                code: status_code,
                failures,
            },
            vars,
            quality: SnapshotQuality {
                poll_ms: rtt.as_millis(),
                stale_seconds: 0.0,
                reads_ok: self.reads_ok,
                reads_err: self.reads_err,
                reconnects: self.reconnects,
                effective_interval_ms: self.effective_interval.as_millis(),
            },
        }
    }

    fn disconnected_snapshot(
        &self,
        reason: String,
        rtt_ms: u128,
        vars: BTreeMap<String, serde_json::Value>,
    ) -> Snapshot {
        let now = Utc::now();
        let age_ms = self
            .last_ok_instant
            .map(|t| t.elapsed().as_millis())
            .unwrap_or(self.config.disconnected_after.as_millis());

        let stale = age_ms > self.config.stale_after.as_millis();

        Snapshot {
            ts: now,
            mono_ms: self.process_start.elapsed().as_millis(),
            device: self.snapshot_device(false),
            freshness: Freshness {
                rtt_ms,
                age_ms,
                stale,
                last_ok_ts: self.last_ok_ts,
            },
            status: MonitorStatus {
                code: "DISCONNECTED".to_string(),
                failures: vec![reason],
            },
            vars,
            quality: SnapshotQuality {
                poll_ms: rtt_ms,
                stale_seconds: age_ms as f64 / 1000.0,
                reads_ok: self.reads_ok,
                reads_err: self.reads_err,
                reconnects: self.reconnects,
                effective_interval_ms: self.effective_interval.as_millis(),
            },
        }
    }

    fn snapshot_device(&self, connected: bool) -> SnapshotDevice {
        let dev = self.current.clone().unwrap_or(DeviceInfo {
            id: self
                .target_id
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            model: "RagTech 3200VA".to_string(),
            transport: "unknown".to_string(),
            path: "".to_string(),
            vid: "".to_string(),
            pid: "".to_string(),
        });

        SnapshotDevice {
            id: dev.id,
            model: dev.model,
            transport: Transport {
                kind: dev.transport,
                path: dev.path,
                vid: dev.vid,
                pid: dev.pid,
            },
            connected,
        }
    }
}
