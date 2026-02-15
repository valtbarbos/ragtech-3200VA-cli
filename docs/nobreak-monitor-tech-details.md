# Nobreak Monitor — Technical Design Details (Phase 1 CLI)

This is a **low-level** technical design (no implementation yet): exact tech stack, versions, libraries, packaging, and the runtime model to achieve **continuous connection + near-realtime sampling** from a RagTech UPS connected via USB.

Scope for now:
- Support only **Nobreak RagTech 3200VA**
- **Read-only** monitoring (we do *not* send commands that change device state)
- Start with a CLI that can later evolve into a web service without redesign

---

## 1) What we keep vs what we delete from the old system

### Keep (because it contains the “truth”)
1) **Transport identifiers + protocol structure**
- `devices.xml` defines the USB transports (HID + CDC) and serial presets:

```xml
<ports>
  <usb class="hid" vid="0425" pid="0301"/>
  <usb class="cdc" vid="04D8" pid="000A"/>
  <serial baudrate="2560" databits="8" stopbits="1" parity="0" timeout="100"/>
  ...
</ports>
```

2) **The vendor runtime (`.so`) as the quickest low-level driver**
- The extracted `.so` exports show clean entry points we can wrap (example: `supapi.so` exposes `Start`, `GetStatus`, `GetDevice`, `GetSuperviseLastError`).

### Delete (because it prevents realtime + reliability)
1) **The polling UI loop**
The extracted JS shows a 3-second refresh timer:

```js
$(document).ready(function() {
  getDetail();
  getShutdown();
  var refreshId = setInterval(function() {
    getDetail();
    getShutdown();
  }, 3000);
});
```

2) **The multi-agent sprawl**
The old stack runs multiple daemons (supsvc/cloudsvc/notifysvc/shutsvc), with agents configured around a 3-second messaging interval:

```ini
[notifysvc]
web.host=localhost
web.port=4470
message.interval=3000

[notifygui]
web.host=localhost
web.port=4470
message.interval=3000
```

This architecture is fine for a desktop UI, but it’s the opposite of “single-purpose, always-connected, always-sampling”.

---

## 2) Tech stack choice (simple + durable)

### 2.1 Language/runtime: **Rust**
Why Rust here:
- Long-running daemon safety (no accidental leaks in the wrapper layer)
- Strong `dlopen` story to reuse the vendor `.so`
- Clean concurrency model (poll loop + reconnect loop + state store)
- Easy path to add a minimal HTTP API later (optional, future phase)

**Pinned toolchain**
- Rust: **1.93.1**  
  Reference: Rust release feed / official site:  
  - https://blog.rust-lang.org/releases/latest/  
  - https://www.rust-lang.org/

### 2.2 OS baseline
We standardize on **Debian 12 (bookworm)** for server deployments.

Why: Ragtech’s own Supervise manual lists Debian 12 and Ubuntu 22.04 among supported Linux targets, so Debian 12 is a safe ABI choice for reusing vendor binaries.

Manual reference (PDF):  
- https://ragtech.com.br/Softwares_download/Manual_instala%C3%A7%C3%A3o_Supervise_8_Rev.1.pdf

---

## 3) Phase 1 deliverable shape (single binary, multiple modes)

We build **one binary**: `nobreakd`

Commands:
- `nobreakd run`  
  Runs forever: detects USB, connects, polls, reconnects, prints logs/telemetry.
- `nobreakd watch`  
  Streams the current state to the terminal (human table or NDJSON).
- `nobreakd once`  
  One snapshot read + print, then exit (for debugging USB access).

Output formats:
- Human table (default)
- JSON (single object)
- NDJSON stream (one JSON object per tick) for piping into anything later

---

## 4) Rust dependencies (libraries)

We keep dependencies minimal and “infrastructure-grade”.

Core crates (version pins):
- `clap = "4.5.58"` — CLI interface
- `tokio = "1.49.0"` — timers + async tasks
- `serde = "1.0.228"` — state representation
- `serde_json = "1.0.149"` — JSON output
- `tracing = "0.1.43"` — structured logs
- `tracing-subscriber = "0.3.22"` — log formatting + filtering
- `thiserror = "2.0.18"` *or* `anyhow = "1.0.101"` — error model
- `libloading = "0.9.0"` — dynamic linking to vendor `.so`
- `quick-xml = "0.39.0"` — parse `devices.xml`
- `udev = "0.9.3"` — subscribe to USB add/remove events
- `nix = "0.31.1"` — signals / process plumbing

Rule: we pin majors/minors in `Cargo.toml`, and **lock exact** versions via `Cargo.lock`.

---

## 5) Native OS dependencies (Debian packages)

At runtime, we expect at minimum:
- `libusb-1.0-0`
- `libudev1`
- `ca-certificates`
- `libstdc++6`, `libgcc-s1` (commonly needed by vendor `.so`)

If the vendor runtime uses HID helpers:
- `libhidapi-libusb0` (or distro equivalent)

Packaging validation step (mandatory): run `ldd vendor/*.so` and document required shared libs explicitly.

---

## 6) Connection guarantees: the runtime state machine

We implement **two loops**:

### Loop A — Poller (1 Hz target)
- attempts to read state once per tick
- updates a shared `StateStore`
- records timing (`poll_duration_ms`) + errors

### Loop B — Connection supervisor
- subscribes to udev events for device add/remove
- monitors poll errors and triggers reconnect
- owns the driver lifecycle (open/close)

State machine (conceptual):

```
DISCONNECTED
  -> (device appears) CONNECTING
CONNECTING
  -> (open ok) CONNECTED
  -> (open fail) BACKOFF
CONNECTED
  -> (poll ok) CONNECTED
  -> (poll timeout N times OR udev remove) DISCONNECTING
DISCONNECTING
  -> (close) DISCONNECTED
BACKOFF
  -> (timer) CONNECTING
```

Backoff strategy:
- exponential (e.g., 200ms → 3s cap), jittered
- reset to fast when `udev add` event is received

This is the “no minutes of disconnection” guarantee: we reconnect aggressively and deterministically.

---

## 7) USB detection details

From `devices.xml`, we know the expected USB identities:

- HID: `0425:0301`
- CDC: `04D8:000A`

The vendor strings reference Linux device nodes like `ttyACM` and `/dev/hiddev`, so we support:
- CDC path resolution: `/dev/ttyACM*`
- HID path resolution: `/dev/hidraw*` (and tolerate older `/dev/hiddev`-style systems)

Implementation detail:
- Use `udev` to enumerate devices and map VID/PID → best usable node.
- Persist the chosen path in state: `device_path`, `transport = cdc|hid`.

---

## 8) The driver layer: vendor `.so` first, native protocol later

### 8.1 Primary path (Phase 1): vendor `.so` wrapper
We will load vendor libs with `dlopen` (`libloading` in Rust) and call a minimal read-only surface:
- init/start vendor runtime
- list devices
- read “current state” snapshot
- close on shutdown or on reconnect

Strict constraints:
- **No calls** to write/config endpoints.
- Wrapper fails-fast if required symbols are missing.

### 8.2 Fallback (Phase 2): native CDC/HID protocol reader
Only if the vendor `.so` proves unreliable:
- Implement CDC request/response reading over `/dev/ttyACM*`
- Implement HID report reads over `/dev/hidraw*`

Clues the vendor stack does HID report I/O (from extracted symbol strings):
- `readReport`, `writeReport`
- “Checksum error”
- “Error reading HID report.”

---

## 9) Polling frequency: “1 Hz if possible”, with vendor-safe guardrails

We have strong evidence the vendor ecosystem is built around **~3 seconds**:
- UI refresh uses `setInterval(..., 3000)` (see JS snippet above)
- agents use `message.interval=3000` (see INI snippet above)
- Ragtech’s manual states the data log is recorded **every 3 seconds** (Portuguese: “gravadas a cada 3 segundos”)  
  Manual PDF: https://ragtech.com.br/Softwares_download/Manual_instala%C3%A7%C3%A3o_Supervise_8_Rev.1.pdf

So we design polling as **adaptive**:

Config defaults:
- `poll_interval = 1s` (goal)
- `poll_interval_max = 3s` (vendor baseline)
- `poll_timeout = 700ms`
- `error_threshold = 3 consecutive failures` → reconnect
- `auto_tune = on`:
  - if poll timeouts occur or average poll duration > 60% of interval, increase toward 3s
  - if stable for a window, cautiously decrease toward 1s

This turns “how fast can we read?” into a measured runtime fact, not a guess.

---

## 10) Packaging and running 24/7

### 10.1 Systemd (recommended first)
Simplest 24/7 integration: a single unit, automatic restart.

Unit intent:
- `Restart=always`
- `RestartSec=1`
- user/group: `nobreak` once udev permissions are set (otherwise start as root temporarily)

### 10.2 Docker (supported, optional)
We can run in Docker, but it adds USB permission complexity.

Dockerfile approach:
- builder: `rust:1.93.1-slim-bookworm`
- runtime: `debian:bookworm-slim` + needed libs
- copy `nobreakd` binary + `vendor/*.so`

Compose intent:
- `privileged: true` **or**
- explicit devices:
  - `/dev/bus/usb:/dev/bus/usb`
  - `/dev/ttyACM0:/dev/ttyACM0` (if CDC is used)

Recommendation: start with systemd until the sampling loop is proven stable.

---

## 11) Mise toolchain pinning

We ship a `.mise.toml` that pins the exact dev toolchain.

Example:

```toml
[tools]
rust = "1.93.1"
just = "1.46.0"
jq = "1.8.1"
```

Notes:
- Rust pin matches the project’s `Cargo.lock` expectations.
- `just` is used only for local/CI task shortcuts (build, lint, run, docker).  
- `jq` is used for validating/debugging streamed JSON output.

## 12) Implementation readiness checklist

Before coding “real logic”, we confirm:
1) `vendor/*.so` loads on Debian 12 (`ldd` has no missing deps)
2) udev discovery finds the UPS by VID/PID
3) Poller sustains:
   - 1 Hz for 10 minutes *or* auto-tunes to 3s while remaining connected
4) Unplug/replug recovery:
   - < 2s typical
   - < 10s worst-case (backoff cap)
5) Snapshot values match the legacy `/mon/1.1/device` JSON for the same device (sanity parity)

---

## Appendix — references used

- Extracted reverse-engineering notes + code excerpts (from the project’s extracted sources)
- Ragtech Supervise manual (poll/log cadence, supported OS list):  
  https://ragtech.com.br/Softwares_download/Manual_instala%C3%A7%C3%A3o_Supervise_8_Rev.1.pdf
- Rust toolchain reference:  
  https://blog.rust-lang.org/releases/latest/  
  https://www.rust-lang.org/
