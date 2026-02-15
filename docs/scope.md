# Nobreak Monitor Scope (v1)

## In scope
- Read-only monitoring of one device model: RagTech 3200VA.
- Linux host runtime with USB detection (CDC `04D8:000A`, HID `0425:0301`).
- CLI-first execution with commands: `scan`, `probe`, `once`, `run`, `watch`.
- Snapshot output with explicit freshness (`age_ms`, `stale`, `last_ok_ts`).
- Auto reconnect and adaptive interval from 1s toward 3s.

## Out of scope
- Any command that changes UPS state (shutdown, configuration writes, LED/control actions).
- Multi-device orchestration.
- Historical storage/database.
- Web UI for this phase.
