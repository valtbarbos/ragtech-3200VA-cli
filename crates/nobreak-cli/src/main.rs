use std::time::Duration;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use nobreak_core::{Monitor, MonitorConfig, VendorShimDriver};
use tokio::time::{interval_at, Instant};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

mod viewer;
mod exporter;
#[cfg(test)]
mod exporter_tests;

#[derive(Debug, Parser)]
#[command(name = "nobreakd")]
#[command(about = "RagTech Nobreak realtime monitor (read-only)")]
struct Cli {
    #[command(subcommand)]
    command: Command,

    #[arg(long, default_value = "./vendor")]
    vendor_dir: String,

    #[arg(long, default_value_t = 1000)]
    interval_ms: u64,

    #[arg(long, default_value_t = 2500)]
    stale_after_ms: u64,

    #[arg(long, default_value_t = 5000)]
    disconnected_after_ms: u64,

    #[arg(long, default_value_t = 700)]
    poll_timeout_ms: u64,

    #[arg(long, default_value_t = 3)]
    error_threshold: u32,

    #[arg(long)]
    device_id: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Scan,
    Probe,
    Once {
        #[arg(long, value_enum, default_value = "json")]
        format: OutputFormat,
    },
    Run {
        #[arg(long, value_enum, default_value = "human")]
        format: OutputFormat,
    },
    Watch {
        #[arg(long, value_enum, default_value = "human")]
        format: OutputFormat,
    },
    View {
        #[arg(long, default_value_t = 180.0)]
        window_sec: f64,
    },
    Export {
        #[arg(long, default_value = "./data/metrics")]
        output_dir: String,
        #[arg(long, default_value_t = 90)]
        retention_days: u64,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
    Ndjson,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .init();

    let cli = Cli::parse();

    let config = MonitorConfig {
        sample_interval: Duration::from_millis(cli.interval_ms),
        sample_interval_min: Duration::from_secs(1),
        sample_interval_max: Duration::from_secs(3),
        stale_after: Duration::from_millis(cli.stale_after_ms),
        disconnected_after: Duration::from_millis(cli.disconnected_after_ms),
        poll_timeout: Duration::from_millis(cli.poll_timeout_ms),
        error_threshold: cli.error_threshold,
        auto_tune: true,
    };

    let mut driver = VendorShimDriver::new(cli.vendor_dir.clone());

    match cli.command {
        Command::Scan => {
            let devices = nobreak_core::UpsDriver::discover(&mut driver).await?;
            println!("{}", serde_json::to_string_pretty(&devices)?);
        }
        Command::Probe => {
            let probe = driver.probe_vendor_runtime();
            let devices = nobreak_core::UpsDriver::discover(&mut driver).await;
            let out = serde_json::json!({
                "probe": probe.map_err(|e| e.to_string()),
                "devices": devices.map_err(|e| e.to_string()),
                "read_only": true
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        Command::Once { format } => {
            let mut monitor = Monitor::new(driver, config, cli.device_id);
            let snapshot = monitor.tick().await;
            print_snapshot(&snapshot, format)?;
        }
        Command::Run { format } | Command::Watch { format } => {
            let mut monitor = Monitor::new(driver, config, cli.device_id);
            stream_loop(&mut monitor, format).await?;
        }
        Command::View { window_sec } => {
            let mut monitor = Monitor::new(driver, config, cli.device_id);
            viewer::run_viewer(&mut monitor, window_sec).await?;
        }
        Command::Export {
            output_dir,
            retention_days,
        } => {
            let mut monitor = Monitor::new(driver, config, cli.device_id);
            exporter::run_exporter(&mut monitor, &output_dir, retention_days).await?;
        }
    }

    Ok(())
}

async fn stream_loop<D: nobreak_core::UpsDriver>(
    monitor: &mut Monitor<D>,
    format: OutputFormat,
) -> Result<()> {
    let start = Instant::now() + Duration::from_millis(50);
    let mut ticker = interval_at(start, monitor.effective_interval());

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                warn!("received ctrl-c, stopping");
                break;
            }
            _ = ticker.tick() => {
                let snapshot = monitor.tick().await;
                print_snapshot(&snapshot, format)?;
                let next = monitor.effective_interval();
                ticker = interval_at(Instant::now() + next, next);
                info!(effective_interval_ms=%next.as_millis(), connected=%snapshot.device.connected, stale=%snapshot.freshness.stale, "tick");
            }
        }
    }

    Ok(())
}

fn print_snapshot(snapshot: &nobreak_core::Snapshot, format: OutputFormat) -> Result<()> {
    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(snapshot)?);
        }
        OutputFormat::Ndjson => {
            println!("{}", serde_json::to_string(snapshot)?);
        }
        OutputFormat::Human => {
            println!("=== Nobreak Snapshot ===");
            println!("Time:       {}", snapshot.ts.to_rfc3339());
            println!("Device:     {} ({})", snapshot.device.id, snapshot.device.model);
            println!(
                "Transport:  {} {} [{}:{}]",
                snapshot.device.transport.kind,
                snapshot.device.transport.path,
                snapshot.device.transport.vid,
                snapshot.device.transport.pid
            );
            println!(
                "State:      connected={} status={} stale={} age_ms={} rtt_ms={}",
                snapshot.device.connected,
                snapshot.status.code,
                snapshot.freshness.stale,
                snapshot.freshness.age_ms,
                snapshot.freshness.rtt_ms,
            );

            if !snapshot.status.failures.is_empty() {
                println!("Failures:   {}", snapshot.status.failures.join(", "));
            }

            if let Some(raw_hex) = snapshot.vars.get("rawFrameHex").and_then(|v| v.as_str()) {
                println!("Raw Frame:  {}", raw_hex);
            }

            if let Some(decoded) = snapshot.vars.get("frameDecoded") {
                if let Some(header) = decoded.get("header") {
                    println!("Frame Info:");
                    println!(
                        "  start={} code={} declared_len={} actual_len={} checksum={} length_match={}",
                        header
                            .get("start_byte_hex")
                            .and_then(|v| v.as_str())
                            .unwrap_or("n/a"),
                        header
                            .get("frame_code_hex")
                            .and_then(|v| v.as_str())
                            .unwrap_or("n/a"),
                        header
                            .get("declared_len")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        header
                            .get("actual_len")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        header
                            .get("checksum_hex")
                            .and_then(|v| v.as_str())
                            .unwrap_or("n/a"),
                        header
                            .get("length_match")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    );
                }

                if let Some(payload_hex) = decoded.get("payload_hex").and_then(|v| v.as_str()) {
                    println!("Payload:    {}", payload_hex);
                }

                if let Some(words) = decoded.get("words_le").and_then(|v| v.as_array()) {
                    let label = words
                        .iter()
                        .take(8)
                        .filter_map(|v| v.as_u64())
                        .map(|n| n.to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("Words LE:   [{}{}]", label, if words.len() > 8 {", ..."} else {""});
                }

                if let Some(metrics) = decoded.get("likely_metrics") {
                    println!("Likely Metrics (experimental):");
                    if let Some(conf) = metrics.get("mapping_confidence").and_then(|v| v.as_str()) {
                        println!("  mapping_confidence: {conf}");
                    }
                    print_est_metric(metrics, "vInput_est", "VInput (V)");
                    print_est_metric(metrics, "vOutput_est", "VOutput (V)");
                    print_est_metric(metrics, "fOutput_est", "FOutput (Hz)");
                    print_est_metric(metrics, "pOutput_est", "POutput (%)");
                    print_est_metric(metrics, "vBattery_est", "VBattery (V)");
                    print_est_metric(metrics, "cBattery_est", "CBattery (%)");
                    print_est_metric(metrics, "temperature_est", "Temperature (C)");
                }
            }
        }
    }

    Ok(())
}

fn print_est_metric(metrics: &serde_json::Value, key: &str, label: &str) {
    if let Some(value) = metrics.get(key).and_then(|v| v.as_f64()) {
        println!("  {label:<16} ~ {:.2}", value);
    }
}
