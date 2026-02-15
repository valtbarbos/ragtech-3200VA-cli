# Nobreak Monitor — Execution Plan (Pareto-first)

This plan is deliberately **Pareto-optimized**: the first tasks create the **smallest vertical slice** that proves (1) we can reliably discover + connect to the UPS over USB, and (2) we can read state continuously with **seconds-level freshness**. Only after that is “stable” do we broaden scope (more fields, nicer output, packaging).

---

## Guiding principle: the 20% that unlocks 80%

The current stack has a structural weakness: it behaves like a *web dashboard* rather than a *device monitor*, with coarse refresh intervals.

Evidence from the extracted app shows:
- The UI refreshes device state on a timer: `setInterval(function(){ getDevState(); }, 3000)` fileciteturn18file0L18-L21  
- Agent config also uses `message.interval=3000` fileciteturn18file3L63-L72  

So we start by building a **device-first monitor loop** (discovery → connect → poll → reconnect) with a target cadence of **1 Hz** and an adaptive fallback to vendor-safe rates.

---

# Phase 0 — Repo sanity + constraints (do once, fast)

### Task 0.1 — Freeze scope + invariants
**Goal:** eliminate ambiguity so the team doesn’t accidentally drift into “control” features.

**Invariants**
- Read-only monitoring: **no writes**, no configuration changes.
- Support only the **RagTech 3200VA** for this version.
- Must run 24/7 on a server with UPS connected via USB.
- Must produce “seconds-grade” updates (aim 1 Hz; adapt if required).

**Acceptance criteria**
- One-page scope note checked into repo (`docs/scope.md`).

---

# Phase 1 — Vertical slice: “I can read *something* every second” (highest ROI)

This phase is the Pareto core. Everything else depends on it.

## Task 1.1 — Device discovery: identify USB path reliably
**Goal:** deterministically locate the UPS when plugged/unplugged.

**Inputs we already have**
- `devices.xml` contains USB VID/PID entries (HID and CDC) that we will treat as authoritative sources of “what to match.” fileciteturn17file0L25-L29  

**Implementation intent**
- Use **udev** enumeration + event subscription:
  - Match by VID/PID
  - Resolve to:
    - `/dev/ttyACM*` if CDC
    - `/dev/hidraw*` if HID
- Persist a stable “device identity” (serial, devpath, or udev symlink).

**Deliverables**
- `nobreakd scan` prints:
  - detected transport type (CDC/HID)
  - path(s) found
  - stable identity key (what we’ll reconnect by)

**Acceptance criteria**
- Plug/unplug triggers correct detection within 1s.
- Device is found even if `/dev/ttyACM0` becomes `/dev/ttyACM1` after reboot.

---

## Task 1.2 — Minimal `.so` shim: call one read method successfully
**Goal:** prove we can reuse vendor `.so` safely without rebuilding their stack.

**Why this is Pareto**
- If `.so` reuse is viable, we skip reinventing the protocol on day 1.
- If it’s not viable, we learn *early* and switch to raw USB/serial.

**Inputs we already have**
- Exported symbols suggest read access exists:
  - `device.so`: `openDeviceManager`, `start`, `getDeviceList`, `getDevice`… fileciteturn14file0L16-L58  
  - `supapi.so`: `Start`, `GetStatus`, `GetDevice`, `GetSuperviseLastError` fileciteturn14file4L32-L52  

**Implementation intent**
- Build a thin Rust “dynamic loader” using `libloading`.
- Validate symbols on startup (fail fast with actionable errors).
- Call the smallest “status” method available and parse a minimal struct/string result.

**Deliverables**
- `nobreakd probe`:
  - loads `.so`
  - prints version + last error
  - prints one meaningful state value (even if just “connected: true”)

**Acceptance criteria**
- Running `probe` repeatedly does not leak memory or crash.
- When UPS is disconnected, error is readable and includes which stage failed.

---

## Task 1.3 — Poll loop + reconnect supervisor (the heart)
**Goal:** keep a live stream of readings, self-heal on disconnect.

**Implementation intent**
- Single responsibility components:
  1. **Watcher** (udev add/remove events)
  2. **Connector** (connects to resolved path, verifies handshake)
  3. **Poller** (ticks every `interval`, produces `Snapshot`)
  4. **Backoff policy** (adaptive interval + reconnect delays)
- Poll target:
  - start at **1s**
  - if timeouts/errors exceed threshold, adapt toward **3s**
  - once stable for N cycles, drift back toward 1s

**Deliverables**
- `nobreakd run --format ndjson` outputs one line per second:
  - timestamp
  - connected/disconnected
  - poll duration ms
  - a minimal state set (even 3–5 fields)

**Acceptance criteria**
- No “minutes-long” silent gaps:
  - if disconnected, output continues with `connected=false` snapshots
- Reconnect occurs automatically after replugging, without restarting process.
- Process survives 24h soak test without memory growth.

---

## Task 1.4 — Define the canonical Snapshot schema (minimal)
**Goal:** establish a stable internal contract that future web UI can consume unchanged.

**Implementation intent**
- `Snapshot` includes:
  - `ts` (monotonic + wall clock)
  - `device_id`
  - `transport` (cdc/hid/unknown)
  - `connected` bool
  - `fields` map (string→number/string/bool)
  - `quality` (poll_ms, errors, stale_seconds)

**Deliverables**
- `schemas/snapshot.schema.json`
- `docs/fields.md` listing known fields + units (incrementally filled)

**Acceptance criteria**
- Snapshots are backward compatible across builds (additive fields only).
- Schema validated in CI (jsonschema check).

---

# Phase 2 — Expand coverage: “read *all* the offered data” (still device-first)

Now that the heartbeat is real, widen data.

## Task 2.1 — Field mapping extraction from legacy outputs
**Goal:** learn what “all offered data” means without guessing.

**Inputs we can reuse**
- Old web endpoint shape: `/mon/1.1/device` is what UI consumes. fileciteturn18file1L15-L23  

**Implementation intent**
- Run the legacy stack once (local) and capture a few JSON responses.
- Build a translation table:
  - legacy key → new `fields` key
  - unit normalization

**Deliverables**
- `docs/legacy-parity.md` with mapping table.
- `nobreakd run --format legacy-json` (optional compatibility format).

**Acceptance criteria**
- At least 80% of fields present in legacy JSON appear in new snapshots.
- Remaining fields have documented reasons (not supported by `.so`, unknown units, etc.)

---

## Task 2.2 — Implement “safe read” policy (explicitly read-only)
**Goal:** guarantee we don’t call “write” paths accidentally.

**Implementation intent**
- Maintain a **whitelist** of allowed `.so` functions.
- Hard-block symbol loading for known write/config methods (fail fast if attempted).
- If raw protocol becomes necessary, use only the minimal read queries.

**Deliverables**
- `docs/read-only-contract.md`
- Unit tests that assert “no write symbols referenced.”

**Acceptance criteria**
- Codebase contains no calls to config/set/reboot actions.
- Audit script confirms only whitelisted symbols are linked.

---

# Phase 3 — Make it operational: “runs 24/7 without babysitting”

## Task 3.1 — systemd service + permissions
**Goal:** stable background service with correct USB access.

**Implementation intent**
- Provide:
  - `nobreakd.service`
  - optional udev rule to set permissions on device nodes
- Logging to journald by default.

**Deliverables**
- `packaging/systemd/nobreakd.service`
- `packaging/udev/99-nobreak.rules`
- `docs/install.md` (Debian 12 first)

**Acceptance criteria**
- After reboot, service starts and reconnects automatically if UPS is present.
- USB access works without running as root (preferred), or root-only is explicitly justified.

---

## Task 3.2 — Observability: logs + simple health signal
**Goal:** explain problems quickly, no “mystery disconnects.”

**Implementation intent**
- Structured logs:
  - connect attempt, success, reason for failure
  - poll duration, timeout counts
  - effective interval chosen by adaptive policy
- Health endpoints:
  - `nobreakd status` prints last snapshot + last error
  - optional: write a local status file (`/run/nobreakd/status.json`)

**Deliverables**
- `docs/ops.md` with “what to check when…” playbook.

**Acceptance criteria**
- When unplugged, logs show transition + udev event.
- When replugged, logs show reconnection path.

---

# Phase 4 — Optional packaging paths (only if needed)

## Task 4.1 — Docker image (secondary, not first)
**Goal:** reproducible deployment, but only after the binary is stable.

**Rationale**
- Docker adds permission complexity for USB passthrough.
- The existing container runs privileged with `/dev/bus/usb` mapping, which works, but we keep it optional. fileciteturn13file0L90-L104  

**Deliverables**
- `Dockerfile` (multi-stage build)
- `docker-compose.yml` example using `--privileged` OR device mapping

**Acceptance criteria**
- Containerized daemon reconnects after unplug/replug without restart.
- Same snapshot output format as native service.

---

# Phase 5 — Hardening + regression safety

## Task 5.1 — Soak + fault tests
**Goal:** confidence we won’t regress to “minutes disconnected.”

**Test matrix**
- Reboot host
- Restart daemon
- Unplug/replug USB
- Suspend/resume (if applicable)
- High CPU load

**Deliverables**
- `tests/soak/` scripts (manual + CI-friendly)
- `docs/test-results.md` sample report format

**Acceptance criteria**
- No silent gaps longer than N seconds (defined in docs).
- Memory and CPU remain stable across long runs.

---

# Pareto summary: what to do first (ordered)

1. **Task 1.1** (udev discovery) — without this, reconnect is brittle.  
2. **Task 1.2** (minimal `.so` probe) — decides reuse vs raw protocol early.  
3. **Task 1.3** (poll + reconnect loop) — eliminates “minutes disconnected.”  
4. **Task 1.4** (Snapshot schema) — stabilizes future web UI integration.  
5. **Task 2.1** (legacy parity mapping) — broadens data coverage safely.  
6. **Task 3.1–3.2** (systemd + observability) — makes it production-ready.  
7. Docker + extra hardening only when the above is solid.

---

## Definition of Done for “v1 CLI monitor”
- A single binary (`nobreakd`) runs as a service.
- It continuously emits per-second snapshots when stable, and **continues emitting** `connected=false` snapshots when unplugged.
- It reconnects automatically on replug.
- It reads a meaningful subset of state, with a documented path to “all offered data.”

