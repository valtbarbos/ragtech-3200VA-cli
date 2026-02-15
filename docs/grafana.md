# Grafana Integration (CLI Export)

The `nobreakd export` mode writes continuous NDJSON time-series files designed for Grafana ingestion.

## Run exporter

```bash
./target/release/nobreakd export --output-dir ./data/metrics --retention-days 90
```

Containerized stack (export + Loki + Promtail + Grafana):

```bash
docker compose -f docker-compose.nobreak.yml up -d --build
```

Set a custom UPS device path:

```bash
NOBREAK_DEVICE=/dev/ttyACM0 docker compose -f docker-compose.nobreak.yml up -d --build
```

Optional stream mode service:

```bash
docker compose -f docker-compose.nobreak.stream.yml up -d --build
```

Default local endpoints:
- Grafana: `http://localhost:3000` (`admin` / `admin`)
- Loki: `http://localhost:3100`

Preloaded dashboard preset:
- `Nobreak Command Center` (auto-provisioned at startup)
- Includes: connection, battery charge/voltage, temperature, input/output voltage, output frequency, output load, and recent telemetry logs

Generated files:
- `data/metrics/nobreak-YYYY-MM-DD.jsonl` (daily append-only)
- `data/metrics/latest.json` (last sample snapshot)

Retention:
- Files older than `retention-days` are automatically pruned (circular log control).

## Record format

Each line in `nobreak-YYYY-MM-DD.jsonl` includes:
- `ts`, `unix_ms`
- `connected`, `status`, `freshness`
- `metrics.vInput`, `metrics.vOutput`, `metrics.fOutput`, `metrics.pOutput`, `metrics.vBattery`, `metrics.cBattery`, `metrics.temperature`
- `meta.rawFrameHex`, `meta.metricsConfidence`

## Grafana options

### Option A: Loki + Promtail (recommended for logs/time-series from JSON files)
- Configure Promtail to scrape `data/metrics/nobreak-*.jsonl`.
- Parse JSON fields into labels/values.
- Query in Grafana via Loki datasource.

### Option B: Grafana Infinity plugin
- Point Infinity datasource to local HTTP endpoint serving these files (or `latest.json`).
- Use `unix_ms` as time column.

## Notes
- Export mode is read-only on UPS state.
- Sampling cadence follows monitor adaptive interval.
- `docker-compose.nobreak.yml` uses `NOBREAK_DEVICE` (default `/dev/ttyACM0`).
- Do not run `export` and `run` collectors at the same time on the same UPS node.
