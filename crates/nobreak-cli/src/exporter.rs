use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use chrono::{DateTime, Days, NaiveDate, Utc};
use nobreak_core::{Monitor, Snapshot, UpsDriver};
use tokio::time::sleep;

pub async fn run_exporter<D: UpsDriver>(
    monitor: &mut Monitor<D>,
    output_dir: &str,
    retention_days: u64,
) -> Result<()> {
    let out_dir = PathBuf::from(output_dir);
    fs::create_dir_all(&out_dir)?;

    let mut state = ExportState::new(out_dir, retention_days)?;

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = sleep(monitor.effective_interval()) => {
                let snapshot = monitor.tick().await;
                state.write_snapshot(&snapshot)?;
                state.maybe_prune()?;
            }
        }
    }

    Ok(())
}

struct ExportState {
    out_dir: PathBuf,
    retention_days: u64,
    current_day: String,
    writer: BufWriter<File>,
    last_prune: Instant,
}

impl ExportState {
    fn new(out_dir: PathBuf, retention_days: u64) -> Result<Self> {
        let now = Utc::now();
        let day = now.format("%Y-%m-%d").to_string();
        let writer = Self::open_writer(&out_dir, &day)?;

        Ok(Self {
            out_dir,
            retention_days,
            current_day: day,
            writer,
            last_prune: Instant::now() - Duration::from_secs(3600),
        })
    }

    fn open_writer(out_dir: &Path, day: &str) -> Result<BufWriter<File>> {
        let path = out_dir.join(format!("nobreak-{day}.jsonl"));
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(BufWriter::new(file))
    }

    fn rotate_if_needed(&mut self, ts: DateTime<Utc>) -> Result<()> {
        let day = ts.format("%Y-%m-%d").to_string();
        if day != self.current_day {
            self.writer.flush()?;
            self.writer = Self::open_writer(&self.out_dir, &day)?;
            self.current_day = day;
        }
        Ok(())
    }

    fn write_snapshot(&mut self, snapshot: &Snapshot) -> Result<()> {
        self.rotate_if_needed(snapshot.ts)?;

        let exported = serde_json::json!({
            "ts": snapshot.ts,
            "unix_ms": snapshot.ts.timestamp_millis(),
            "device_id": snapshot.device.id,
            "model": snapshot.device.model,
            "transport": snapshot.device.transport,
            "connected": snapshot.device.connected,
            "freshness": snapshot.freshness,
            "status": snapshot.status,
            "metrics": {
                "vInput": snapshot.vars.get("vInput").cloned(),
                "vOutput": snapshot.vars.get("vOutput").cloned(),
                "fOutput": snapshot.vars.get("fOutput").cloned(),
                "pOutput": snapshot.vars.get("pOutput").cloned(),
                "vBattery": snapshot.vars.get("vBattery").cloned(),
                "cBattery": snapshot.vars.get("cBattery").cloned(),
                "temperature": snapshot.vars.get("temperature").cloned()
            },
            "meta": {
                "metricsConfidence": snapshot.vars.get("metricsConfidence").cloned(),
                "rawFrameHex": snapshot.vars.get("rawFrameHex").cloned(),
                "rawFrameLen": snapshot.vars.get("rawFrameLen").cloned()
            }
        });

        serde_json::to_writer(&mut self.writer, &exported)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;

        let latest_path = self.out_dir.join("latest.json");
        fs::write(latest_path, serde_json::to_vec_pretty(&exported)?)?;

        Ok(())
    }

    fn maybe_prune(&mut self) -> Result<()> {
        if self.last_prune.elapsed() < Duration::from_secs(1800) {
            return Ok(());
        }
        self.last_prune = Instant::now();

        prune_old_log_files(&self.out_dir, self.retention_days, SystemTime::now())?;

        Ok(())
    }
}

pub(crate) fn prune_old_log_files(out_dir: &Path, retention_days: u64, now: SystemTime) -> Result<()> {
    let today = DateTime::<Utc>::from(now).date_naive();
    let cutoff = today
        .checked_sub_days(Days::new(retention_days))
        .unwrap_or(today);

    for entry in fs::read_dir(out_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|v| v.to_str())
            .map(|n| n.starts_with("nobreak-") && n.ends_with(".jsonl"))
            .unwrap_or(false)
        {
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        let Some(date_part) = file_name
            .strip_prefix("nobreak-")
            .and_then(|v| v.strip_suffix(".jsonl"))
        else {
            continue;
        };

        let Ok(file_date) = NaiveDate::parse_from_str(date_part, "%Y-%m-%d") else {
            continue;
        };

        if file_date < cutoff {
            let _ = fs::remove_file(path);
        }
    }

    Ok(())
}

 
