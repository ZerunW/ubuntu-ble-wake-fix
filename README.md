# ubuntu-ble-wake-fix

English | [简体中文](README.zh-CN.md)

Fix for Ubuntu: Bluetooth LE (BLE) keyboards and mice can't wake the computer from
suspend. Install it and a key press wakes the machine again.

On Ubuntu 24.04 with a newer kernel (HWE 7.0) a Bluetooth LE keyboard can't
wake the machine from suspend, although `bluetoothctl info` shows
`WakeAllowed: yes` (or no `WakeAllowed` at all). The kernel decides wake per
device with one flag, *Remote Wakeup*, and BlueZ 5.72 either never sets it or
clears it again right after setting it. `ubuntu-ble-wake-fix` is a small root service
that keeps that flag set for your paired input devices and enables USB wakeup
on the Bluetooth controller.

- **Does not modify BlueZ.** It talks to the kernel's Bluetooth management
  interface, the same stable interface bluetoothd uses. BlueZ updates don't
  affect it; once BlueZ sets the flag correctly it simply has nothing to do.
- **Only input devices.** Bonded devices that offer the HID service (HID over
  GATT or classic HID) or have a keyboard/mouse appearance. Headphones and
  the like are left alone. Single devices can be excluded.
- **Reacts at once.** The kernel reports every flag change; if something
  clears Remote Wakeup the service sets it again within milliseconds. It also
  re-checks everything once a minute.

## Install

Download the `.deb` from the [latest release](https://github.com/ZerunW/ubuntu-ble-wake-fix/releases/latest), then

```bash
sudo apt install ./ubuntu-ble-wake-fix_*_amd64.deb
```

The service starts at once and on every boot (`systemctl status ubuntu-ble-wake-fix`).

Check what it sees (changes nothing):

```bash
sudo ubuntu-ble-wake-fix --check
```

```
hci0  00:00:5E:00:53:01
  USB wakeup: 1-14=enabled  usb1=enabled
  C0:FF:EE:00:00:01  LE random  MX MCHNCL             wake on   (flags 0x3, supported 0x7)
  C0:FF:EE:00:00:02  LE random  MX Master 3S M        wake on   (flags 0x3, supported 0x7)
```

Then suspend, wait a moment and press a key.

## What it changes, and the trade-offs

- Every paired Bluetooth keyboard and mouse (and other input devices) gets the kernel's
  Remote Wakeup flag. The service keeps it on: if something else clears it, it is set
  again. A device you don't want to wake the machine goes in the exclude list below.
- USB wakeup is enabled for the Bluetooth controller and the USB hubs above it
  (`--no-usb` turns this off, but then nothing wakes the machine over Bluetooth).
- A mouse that gets bumped, or a keyboard in a bag, can therefore wake the laptop, and
  a controller that is allowed to wake the system may use a little more power in suspend.
  Exclude the devices you don't need.

## Configuration

`/etc/ubuntu-ble-wake-fix/exclude`: addresses that must not wake the system, one per
line (`#` starts a comment). Restart the service after editing:
`sudo systemctl restart ubuntu-ble-wake-fix`. Already set flags stay until bluetoothd
or the controller restarts.

Command line (for the service, see `systemctl cat ubuntu-ble-wake-fix`):

| Option | Meaning |
|---|---|
| `--check` | print devices, flags and USB wakeup, change nothing |
| `--once` | fix everything once and exit |
| `--interval SECS` | full re-check interval (default 60) |
| `--no-usb` | don't touch USB `power/wakeup` |
| `--storage DIR`, `--exclude FILE`, `--sys DIR` | paths (for testing) |

## Requirements

- Linux with the Bluetooth management interface (any current kernel) and
  systemd. Suspend-to-idle (`s2idle`) or S3 with USB wake allowed by the
  firmware (`XHCI ... enabled` in `/proc/acpi/wakeup`).
- BlueZ storage in `/var/lib/bluetooth` (standard).
- Tested: HP OMEN 16 (Intel AX211, USB 8087:0033), Ubuntu 24.04, kernel
  7.0.0-42, BlueZ 5.72-0ubuntu5.8, Logitech MX Mechanical and MX Master 3S.

## Uninstall

```bash
sudo apt remove ubuntu-ble-wake-fix   # or: apt purge, which also deletes the exclude list
```

Removing the package stops the service but does not clear what it already set: the
Remote Wakeup flags stay until bluetoothd or the controller restarts
(`sudo systemctl restart bluetooth`), and USB wakeup stays until the next reboot.

## Sharing logs

`journalctl -u ubuntu-ble-wake-fix` and `--check` print the Bluetooth addresses and
names of your devices. Replace the addresses (for example with `C0:FF:EE:00:00:01`)
before posting them in an issue.

## Why this is needed

Details, btmon evidence and the two BlueZ bugs are in
[docs/how-it-works.md](docs/how-it-works.md). In short:

1. BlueZ 5.72 refuses wake for devices that use privacy addresses unless the
   kernel experimental feature "RPA resolution" is on; kernel 7.0 no longer
   has that feature (it uses LL Privacy automatically). Log:
   `Unable to set wake_support without RPA resolution`.
2. Where it does set the flag, BlueZ 5.72 then sets Device Privacy Mode with
   a stale value (0x2), and the kernel replaces the whole value, so Remote
   Wakeup (0x1) is lost. Upstream BlueZ has since fixed this.
3. USB wakeup of the Bluetooth controller is off by default; without it the
   kernel doesn't configure Bluetooth wake at all.

## Build from source

```bash
cargo test
packaging/build-deb.sh        # -> out/ubuntu-ble-wake-fix_<version>_<arch>.deb
```

## License

GPL-2.0-or-later. See [LICENSE](LICENSE).
