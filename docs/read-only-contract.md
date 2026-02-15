# Read-only Contract

The monitor must never issue write/configuration commands to the device.

## Enforcement in this implementation
- Driver surface exposes only: discover, connect, read, disconnect.
- `probe` only attempts dynamic loading of vendor libraries and reports status.
- Snapshot collection currently reads connection presence and reports freshness/quality metadata.
- Any future vendor-symbol bindings must stay on an explicit allowlist of read paths.

## Forbidden categories
- Device shutdown/restart commands.
- Battery calibration commands.
- Configuration updates and persistent settings changes.
- Any opaque command path without read-only verification.
