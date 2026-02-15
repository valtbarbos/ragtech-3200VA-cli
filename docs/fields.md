# Snapshot Fields

## Top-level
- `ts`: UTC wall-clock timestamp (RFC3339).
- `mono_ms`: monotonic elapsed milliseconds since process start.
- `device`: identity and current transport.
- `freshness`: realtime guarantees (`rtt_ms`, `age_ms`, `stale`, `last_ok_ts`).
- `status`: monitor status code and failure reasons.
- `vars`: read values map (currently empty until vendor snapshot mapping is bound).
- `quality`: poll/reconnect counters and effective interval.

## Planned minimum vars when vendor read binding is completed
- `vInput`
- `vOutput`
- `fOutput`
- `pOutput`
- `vBattery`
- `cBattery`
- `temperature`
