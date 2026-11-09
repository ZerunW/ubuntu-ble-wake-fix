# Changelog

## 0.1.1 - 2026-11-08

- Documentation uses made-up Bluetooth addresses instead of real ones.
- README: what the service changes and the trade-offs (accidental wake-ups, power in
  suspend), what stays set after uninstalling, and removing addresses from logs
  before sharing them.
- The installer warns clearly if the service did not start.

## 0.1.0 - 2026-11-08

First release.

- Keeps the kernel Remote Wakeup flag set for bonded Bluetooth input devices
  (HID over GATT, classic HID, or keyboard/mouse appearance) through the
  Bluetooth management interface; reacts to Device Flags Changed events and
  re-checks every 60 s.
- Enables USB `power/wakeup` on the Bluetooth controller and its parent hubs.
- `--check` (read only), `--once`, exclude list in `/etc/ubuntu-ble-wake-fix/exclude`.
- systemd service with a restricted sandbox, Debian package.
