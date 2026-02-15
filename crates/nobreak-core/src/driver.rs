use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use libloading::Library;
use serialport::SerialPort;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use tracing::warn;

const CDC_REQUEST_COMMAND: [u8; 6] = [0xAA, 0x04, 0x00, 0x80, 0x1E, 0x9E];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub model: String,
    pub transport: String,
    pub path: String,
    pub vid: String,
    pub pid: String,
}

#[derive(Debug, Clone)]
pub struct ReadResult {
    pub status_code: String,
    pub failures: Vec<String>,
    pub vars: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Error)]
pub enum DriverError {
    #[error("device not found")]
    DeviceNotFound,
    #[error("device disconnected")]
    Disconnected,
    #[error("timeout")]
    Timeout,
    #[error("io error: {0}")]
    Io(String),
    #[error("driver error: {0}")]
    Other(String),
}

#[async_trait]
pub trait UpsDriver: Send {
    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, DriverError>;
    async fn connect(&mut self, preferred_id: Option<&str>) -> Result<DeviceInfo, DriverError>;
    async fn read(&mut self) -> Result<ReadResult, DriverError>;
    async fn disconnect(&mut self) -> Result<(), DriverError>;
    fn is_connected(&self) -> bool;
    fn current_device(&self) -> Option<DeviceInfo>;
}

pub struct VendorShimDriver {
    vendor_dir: PathBuf,
    connected: Option<DeviceInfo>,
    loaded_libs: Vec<Library>,
    cdc_port: Option<Box<dyn SerialPort>>,
}

impl VendorShimDriver {
    pub fn new(vendor_dir: impl Into<PathBuf>) -> Self {
        Self {
            vendor_dir: vendor_dir.into(),
            connected: None,
            loaded_libs: Vec::new(),
            cdc_port: None,
        }
    }

    pub fn probe_vendor_runtime(&mut self) -> Result<serde_json::Value, DriverError> {
        let candidates = ["device.so", "config.so", "supapi.so"];
        self.loaded_libs.clear();
        let mut loaded = Vec::new();

        for file in candidates {
            let path = self.vendor_dir.join(file);
            if !path.exists() {
                continue;
            }
            let lib = unsafe { Library::new(&path) }
                .map_err(|err| DriverError::Other(format!("failed to load {}: {err}", path.display())))?;
            loaded.push(path.display().to_string());
            self.loaded_libs.push(lib);
        }

        if loaded.is_empty() {
            return Err(DriverError::Other(format!(
                "no vendor libraries found under {}",
                self.vendor_dir.display()
            )));
        }

        Ok(json!({"loaded_libraries": loaded, "read_only": true}))
    }

    fn extract_vid_pid(device: &udev::Device) -> (String, String) {
        let from_properties = || {
            let vid = device
                .property_value("ID_VENDOR_ID")
                .and_then(|v| v.to_str())
                .unwrap_or_default()
                .to_string();
            let pid = device
                .property_value("ID_MODEL_ID")
                .and_then(|v| v.to_str())
                .unwrap_or_default()
                .to_string();
            (vid, pid)
        };

        let mut pair = from_properties();
        if !pair.0.is_empty() && !pair.1.is_empty() {
            return pair;
        }

        let mut parent = device.parent();
        while let Some(dev) = parent {
            let vid = dev
                .attribute_value("idVendor")
                .and_then(|v| v.to_str())
                .unwrap_or_default();
            let pid = dev
                .attribute_value("idProduct")
                .and_then(|v| v.to_str())
                .unwrap_or_default();
            if !vid.is_empty() && !pid.is_empty() {
                pair = (vid.to_string(), pid.to_string());
                break;
            }
            parent = dev.parent();
        }

        pair
    }

    fn is_cdc_ragtech(vid: &str, pid: &str) -> bool {
        vid.eq_ignore_ascii_case("04d8") && pid.eq_ignore_ascii_case("000a")
    }

    fn is_hid_ragtech(vid: &str, pid: &str) -> bool {
        vid.eq_ignore_ascii_case("0425") && pid.eq_ignore_ascii_case("0301")
    }

    fn scan_udev_devices() -> Result<Vec<DeviceInfo>, DriverError> {
        let mut enumerator = udev::Enumerator::new().map_err(|e| DriverError::Io(e.to_string()))?;
        enumerator
            .match_subsystem("tty")
            .map_err(|e| DriverError::Io(e.to_string()))?;

        let mut devices = Vec::new();
        for device in enumerator
            .scan_devices()
            .map_err(|e| DriverError::Io(e.to_string()))?
        {
            let (vid, pid) = Self::extract_vid_pid(&device);

            if !Self::is_cdc_ragtech(&vid, &pid) {
                continue;
            }

            let node = device
                .devnode()
                .and_then(Path::to_str)
                .unwrap_or_default()
                .to_string();

            if node.is_empty() {
                continue;
            }

            devices.push(DeviceInfo {
                id: format!("cdc:{}", node),
                model: "RagTech 3200VA".to_string(),
                transport: "cdc".to_string(),
                path: node,
                vid,
                pid,
            });
        }

        if devices.is_empty() {
            let mut hid_enum = udev::Enumerator::new().map_err(|e| DriverError::Io(e.to_string()))?;
            hid_enum
                .match_subsystem("hidraw")
                .map_err(|e| DriverError::Io(e.to_string()))?;

            for device in hid_enum
                .scan_devices()
                .map_err(|e| DriverError::Io(e.to_string()))?
            {
                let (vid, pid) = Self::extract_vid_pid(&device);

                if !Self::is_hid_ragtech(&vid, &pid) {
                    continue;
                }

                let node = device
                    .devnode()
                    .and_then(Path::to_str)
                    .unwrap_or_default()
                    .to_string();

                if node.is_empty() {
                    continue;
                }

                devices.push(DeviceInfo {
                    id: format!("hid:{}", node),
                    model: "RagTech 3200VA".to_string(),
                    transport: "hid".to_string(),
                    path: node,
                    vid,
                    pid,
                });
            }
        }

        Ok(devices)
    }

    fn open_cdc_port(path: &str) -> Result<Box<dyn SerialPort>, DriverError> {
        serialport::new(path, 2560)
            .timeout(Duration::from_millis(350))
            .open()
            .map_err(|err| DriverError::Io(format!("failed to open serial port {path}: {err}")))
    }

    fn read_cdc_snapshot(port: &mut dyn SerialPort) -> Result<Vec<u8>, DriverError> {
        let mut flush_buf = [0_u8; 256];
        while let Ok(read) = port.read(&mut flush_buf) {
            if read == 0 {
                break;
            }
        }

        port.write_all(&CDC_REQUEST_COMMAND)
            .map_err(|err| DriverError::Io(format!("failed to write request command: {err}")))?;
        port.flush()
            .map_err(|err| DriverError::Io(format!("failed to flush request command: {err}")))?;

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut buf = Vec::with_capacity(128);
        let mut chunk = [0_u8; 128];

        loop {
            match port.read(&mut chunk) {
                Ok(0) => {}
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    if n >= 64 || buf.len() >= 64 {
                        break;
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {
                    if !buf.is_empty() {
                        break;
                    }
                }
                Err(err) => return Err(DriverError::Io(format!("serial read failed: {err}"))),
            }

            if Instant::now() >= deadline {
                break;
            }
        }

        if buf.is_empty() {
            return Err(DriverError::Timeout);
        }

        Ok(buf)
    }

    fn decode_raw_frame(frame: &[u8]) -> serde_json::Value {
        let u16_be = |idx: usize| -> Option<u16> {
            if idx + 1 >= frame.len() {
                return None;
            }
            Some(u16::from_be_bytes([frame[idx], frame[idx + 1]]))
        };

        let mut words_le = Vec::new();
        let mut words_be = Vec::new();
        if frame.len() >= 4 {
            for idx in (2..frame.len().saturating_sub(1)).step_by(2) {
                if idx + 1 >= frame.len().saturating_sub(1) {
                    break;
                }
                let lo = frame[idx];
                let hi = frame[idx + 1];
                words_le.push(u16::from_le_bytes([lo, hi]));
                words_be.push(u16::from_be_bytes([lo, hi]));
            }
        }

        let start_byte = frame.first().copied().unwrap_or_default();
        let frame_code = frame.get(1).copied().unwrap_or_default();
        let declared_len = frame.get(1).copied().unwrap_or_default() as usize;
        let checksum = frame.last().copied().unwrap_or_default();

        let payload_hex = if frame.len() > 3 {
            frame[2..frame.len() - 1]
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join("")
        } else {
            String::new()
        };

        let byte_map = frame
            .iter()
            .enumerate()
            .map(|(idx, byte)| {
                json!({
                    "idx": idx,
                    "hex": format!("{byte:02X}"),
                    "dec": byte
                })
            })
            .collect::<Vec<_>>();

        let frame_aligned = frame.len() >= 31
            && frame.first().copied() == Some(0xAA)
            && frame.get(1).copied() == Some(0x21)
            && frame.get(2).copied() == Some(0x00)
            && frame.get(3).copied() == Some(0x0C);

        let v_input_est = if frame_aligned {
            u16_be(11).map(|v| v as f64 / 504.0)
        } else {
            None
        };
        let v_output_est = if frame_aligned {
            u16_be(23).map(|v| v as f64 / 366.0)
        } else {
            None
        };
        let v_battery_est = if frame_aligned {
            u16_be(20).map(|v| v as f64 / 1249.0)
        } else {
            None
        };
        let f_output_est = if frame_aligned {
            u16_be(27).map(|v| v as f64 / 77.4)
        } else {
            None
        };
        let c_battery_est = if frame_aligned {
            frame.get(26).map(|v| *v as f64)
        } else {
            None
        };
        let p_output_est = if frame_aligned {
            frame.get(27).map(|v| *v as f64)
        } else {
            None
        };
        let temperature_est = if frame_aligned {
            frame.get(15).map(|v| *v as f64)
        } else {
            None
        };

        let likely_metrics = json!({
            "vInput_est": v_input_est,
            "vOutput_est": v_output_est,
            "vBattery_est": v_battery_est,
            "fOutput_est": f_output_est,
            "cBattery_est": c_battery_est,
            "pOutput_est": p_output_est,
            "temperature_est": temperature_est,
            "frame_aligned": frame_aligned,
            "mapping_confidence": if frame_aligned { "experimental" } else { "insufficient_frame_alignment" },
            "mapping_note": "Offsets/scales inferred from observed frames; keep raw bytes for verification"
        });

        json!({
            "header": {
                "start_byte_hex": format!("0x{start_byte:02X}"),
                "frame_code_hex": format!("0x{frame_code:02X}"),
                "declared_len": declared_len,
                "actual_len": frame.len(),
                "checksum_hex": format!("0x{checksum:02X}"),
                "length_match": declared_len == frame.len()
            },
            "payload_hex": payload_hex,
            "byte_map": byte_map,
            "words_le": words_le,
            "words_be": words_be,
            "likely_metrics": likely_metrics,
            "notes": [
                "Decoded from raw CDC frame without write/control commands",
                "Likely metrics are marked experimental and should be cross-validated"
            ]
        })
    }
}

#[async_trait]
impl UpsDriver for VendorShimDriver {
    async fn discover(&mut self) -> Result<Vec<DeviceInfo>, DriverError> {
        Self::scan_udev_devices()
    }

    async fn connect(&mut self, preferred_id: Option<&str>) -> Result<DeviceInfo, DriverError> {
        let devices = Self::scan_udev_devices()?;
        if devices.is_empty() {
            self.connected = None;
            self.cdc_port = None;
            return Err(DriverError::DeviceNotFound);
        }

        let chosen = preferred_id
            .and_then(|id| devices.iter().find(|d| d.id == id).cloned())
            .unwrap_or_else(|| devices[0].clone());

        self.cdc_port = None;
        if chosen.transport == "cdc" {
            let port = Self::open_cdc_port(&chosen.path)?;
            self.cdc_port = Some(port);
        }

        self.connected = Some(chosen.clone());
        Ok(chosen)
    }

    async fn read(&mut self) -> Result<ReadResult, DriverError> {
        let Some(current) = self.connected.clone() else {
            return Err(DriverError::Disconnected);
        };

        let devices = Self::scan_udev_devices()?;
        let still_present = devices.iter().any(|dev| dev.id == current.id);
        if !still_present {
            self.connected = None;
            self.cdc_port = None;
            return Err(DriverError::Disconnected);
        }

        if current.transport == "cdc" {
            if self.cdc_port.is_none() {
                self.cdc_port = Some(Self::open_cdc_port(&current.path)?);
            }
            let Some(port) = self.cdc_port.as_mut() else {
                return Err(DriverError::Disconnected);
            };

            let frame = Self::read_cdc_snapshot(port.as_mut())?;
            let hex = frame
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join("");

            let mut vars = BTreeMap::new();
            vars.insert("rawFrameHex".to_string(), serde_json::Value::String(hex));
            vars.insert(
                "rawFrameLen".to_string(),
                serde_json::Value::from(frame.len() as u64),
            );
            vars.insert(
                "requestCommand".to_string(),
                serde_json::Value::String("AA0400801E9E".to_string()),
            );
            let decoded = Self::decode_raw_frame(&frame);

            if let Some(metrics) = decoded.get("likely_metrics") {
                let map_metric = |src: &str, dst: &str, vars: &mut BTreeMap<String, serde_json::Value>| {
                    if let Some(v) = metrics.get(src).and_then(|v| v.as_f64()) {
                        vars.insert(dst.to_string(), serde_json::Value::from(v));
                    }
                };

                map_metric("vInput_est", "vInput", &mut vars);
                map_metric("vOutput_est", "vOutput", &mut vars);
                map_metric("fOutput_est", "fOutput", &mut vars);
                map_metric("pOutput_est", "pOutput", &mut vars);
                map_metric("vBattery_est", "vBattery", &mut vars);
                map_metric("cBattery_est", "cBattery", &mut vars);
                map_metric("temperature_est", "temperature", &mut vars);

                if let Some(conf) = metrics.get("mapping_confidence").and_then(|v| v.as_str()) {
                    vars.insert(
                        "metricsConfidence".to_string(),
                        serde_json::Value::String(conf.to_string()),
                    );
                }
            }

            vars.insert("frameDecoded".to_string(), decoded);

            return Ok(ReadResult {
                status_code: "ONLINE_RAW".to_string(),
                failures: Vec::new(),
                vars,
            });
        }

        Ok(ReadResult {
            status_code: "UNKNOWN".to_string(),
            failures: vec!["vendor_snapshot_unimplemented".to_string()],
            vars: BTreeMap::new(),
        })
    }

    async fn disconnect(&mut self) -> Result<(), DriverError> {
        if self.connected.is_some() {
            warn!("disconnecting driver session");
        }
        self.connected = None;
        self.cdc_port = None;
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.is_some()
    }

    fn current_device(&self) -> Option<DeviceInfo> {
        self.connected.clone()
    }
}
