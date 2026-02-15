use crate::exporter::prune_old_log_files;
use chrono::{TimeZone, Utc};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

fn make_temp_dir(name: &str) -> PathBuf {
    let mut path = env::temp_dir();
    let uniq = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("unix epoch")
        .as_nanos();
    path.push(format!("nobreak-tests-{name}-{uniq}"));
    fs::create_dir_all(&path).expect("create temp dir");
    path
}

#[test]
fn prune_removes_only_old_log_files() {
    // Arrange
    let dir = make_temp_dir("old-vs-fresh");
    let old_log = dir.join("nobreak-2025-11-16.jsonl");
    let fresh_log = dir.join("nobreak-2026-02-15.jsonl");
    let unrelated = dir.join("notes.txt");
    fs::write(&old_log, "old").expect("write old log");
    fs::write(&fresh_log, "fresh").expect("write fresh log");
    fs::write(&unrelated, "keep").expect("write unrelated");

    let now: SystemTime = Utc
        .with_ymd_and_hms(2026, 2, 15, 0, 0, 0)
        .single()
        .expect("valid date")
        .into();

    // Act
    prune_old_log_files(&dir, 90, now).expect("prune");

    // Assert
    assert!(!old_log.exists(), "old log should be pruned");
    assert!(fresh_log.exists(), "fresh log should be kept");
    assert!(unrelated.exists(), "non-log file should never be pruned");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn prune_keeps_boundary_age_log_file() {
    // Arrange
    let dir = make_temp_dir("boundary");
    let boundary_log = dir.join("nobreak-2025-11-17.jsonl");
    fs::write(&boundary_log, "boundary").expect("write boundary log");

    let now: SystemTime = Utc
        .with_ymd_and_hms(2026, 2, 15, 0, 0, 0)
        .single()
        .expect("valid date")
        .into();

    // Act
    prune_old_log_files(&dir, 90, now).expect("prune");

    // Assert
    assert!(
        boundary_log.exists(),
        "log exactly on retention boundary should be kept"
    );

    let _ = fs::remove_dir_all(dir);
}
