# Nobreak Monitor (CLI-first)

Read-only realtime monitor for RagTech 3200VA, built from the ultraspec in this repository.

## What this repository contains

- Binary: `nobreakd`
- Rust workspace: `crates/nobreak-core`, `crates/nobreak-cli`
- Modes: `scan`, `probe`, `once`, `run`, `watch`, `export`
- Docker stack: `Dockerfile.nobreak`, `docker-compose.nobreak.yml`, `docker-compose.nobreak.stream.yml`
- Ops/docs: `docs/*`, `schemas/snapshot.schema.json`, `packaging/systemd/nobreakd.service`, `packaging/udev/99-nobreak.rules`

## Quick start

```bash
cargo build --release -p nobreak-cli
./target/release/nobreakd scan
./target/release/nobreakd probe --vendor-dir ./vendor
./target/release/nobreakd run --format ndjson
```

## Docker

### Prebuilt image (GHCR)

This repository publishes a container image to GitHub Container Registry:

- `ghcr.io/valtbarbos/ragtech-3200VA-cli`

Common tags:

- `:latest` (pushes to `main`)
- `:vX.Y.Z` (when you push a tag like `v0.1.0`)

Pull and run:

```bash
docker pull ghcr.io/valtbarbos/ragtech-3200VA-cli:latest

# default CMD is `scan`
docker run --rm ghcr.io/valtbarbos/ragtech-3200VA-cli:latest

# run other subcommands
docker run --rm ghcr.io/valtbarbos/ragtech-3200VA-cli:latest --help
docker run --rm ghcr.io/valtbarbos/ragtech-3200VA-cli:latest watch
```

If you want to talk to a USB/serial device from inside the container, you will likely need to pass a device through:

```bash
# example: typical CDC-ACM device
docker run --rm \
	--device=/dev/ttyACM0 \
	-e NOBREAK_DEVICE=/dev/ttyACM0 \
	ghcr.io/valtbarbos/ragtech-3200VA-cli:latest probe
```

```bash
# exporter + loki + promtail + grafana
docker compose -f docker-compose.nobreak.yml up -d --build

# isolated ndjson stream mode
docker compose -f docker-compose.nobreak.stream.yml up -d --build
```

Optional device override:

```bash
NOBREAK_DEVICE=/dev/ttyACM0 docker compose -f docker-compose.nobreak.yml up -d --build
```

## Makefile shortcuts

```bash
make up
make stream
make export
make status
make logs
make logs-stream
make down
```

## Endpoints

- Grafana: http://localhost:3000 (`admin` / `admin`)
- Loki: http://localhost:3100

Dashboard is auto-provisioned from:
`observability/grafana/provisioning/dashboards/json/nobreak-command-center.json`

## Documentation

- `nobreak-monitor-ultraspec.md`
- `nobreak-monitor-tech-details.md`
- `nobreak-monitor-execution-plan.md`
- `docs/scope.md`
- `docs/read-only-contract.md`
- `docs/install.md`
- `docs/ops.md`
- `docs/grafana.md`

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

## Disclaimer

This project is an independent, community implementation and is not affiliated with Ragtech.
Ragtech's proprietary software (e.g. "Supervise") is not distributed with this repository.