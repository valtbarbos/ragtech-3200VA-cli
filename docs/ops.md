# Operations Playbook

## Health checks
- `nobreakd once --format json` for one sample.
- `nobreakd run --format ndjson` for continuous stream.
- `nobreakd export --output-dir ./data/metrics --retention-days 90` for Grafana-ready retention logs.

## Expected transitions
- Unplug: snapshots continue with `device.connected=false` and `status.code=DISCONNECTED`.
- Replug: state returns to connected without process restart.

## Key fields for alerting
- `freshness.stale`
- `freshness.age_ms`
- `quality.reconnects`
- `quality.reads_err`

## Logging
Set log level with env var:

```bash
RUST_LOG=info ./target/release/nobreakd run
```
