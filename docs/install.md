# Install (Debian 12)

## 1. Build

```bash
cargo build --release -p nobreak-cli
```

Binary output:
- `target/release/nobreakd`

## 2. Runtime prerequisites

Install baseline packages:

```bash
sudo apt-get update
sudo apt-get install -y libusb-1.0-0 libudev1 ca-certificates libstdc++6 libgcc-s1
```

If vendor runtime needs HID helpers, install distro equivalent for hidapi.

## 3. Vendor runtime files

Place vendor libraries in `./vendor` or pass `--vendor-dir`.
Expected names:
- `device.so`
- `config.so`
- `supapi.so`

## 4. Run commands

```bash
./target/release/nobreakd scan
./target/release/nobreakd probe --vendor-dir ./vendor
./target/release/nobreakd run --format ndjson
```

## 5. Serial permissions (host)

If `once`/`run` reports `Permission denied` on `/dev/ttyACM0`, grant user access:

```bash
sudo usermod -aG dialout $USER
newgrp dialout
```

Or apply the bundled udev rules:

```bash
sudo cp packaging/udev/99-nobreak.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```
