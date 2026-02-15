use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub ts: DateTime<Utc>,
    pub mono_ms: u128,
    pub device: SnapshotDevice,
    pub freshness: Freshness,
    pub status: MonitorStatus,
    pub vars: BTreeMap<String, serde_json::Value>,
    pub quality: SnapshotQuality,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDevice {
    pub id: String,
    pub model: String,
    pub transport: Transport,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transport {
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    pub vid: String,
    pub pid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Freshness {
    pub rtt_ms: u128,
    pub age_ms: u128,
    pub stale: bool,
    pub last_ok_ts: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorStatus {
    pub code: String,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotQuality {
    pub poll_ms: u128,
    pub stale_seconds: f64,
    pub reads_ok: u64,
    pub reads_err: u64,
    pub reconnects: u64,
    pub effective_interval_ms: u128,
}
