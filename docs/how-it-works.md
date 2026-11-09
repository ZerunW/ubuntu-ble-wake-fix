# How it works

## What decides whether Bluetooth can wake the host

On suspend the kernel (`hci_suspend_sync()` in `net/bluetooth/hci_sync.c`):

1. disconnects all devices and sets the event mask so only wake events remain;
2. stops here unless the controller's transport says it may wake the system:
   for USB controllers `btusb_wakeup()` returns `device_may_wakeup()` of the
   USB device, i.e. its `power/wakeup` must be `enabled`;
3. keeps only devices whose connection parameters carry
   `HCI_CONN_FLAG_REMOTE_WAKEUP` in the controller's LE accept list
   (`hci_le_add_accept_list_sync()`), programs devices with an IRK into the
   resolving list (LL Privacy), and starts low duty cycle passive scanning.

When a keyboard key is pressed the keyboard advertises, the controller sees
an accepted device, raises an event, and USB remote wakeup wakes the host.

The per-device flag is set through the management interface
(`MGMT_OP_SET_DEVICE_FLAGS`, 0x0050). The kernel **replaces** the whole flag
value with what is sent (`set_device_flags()` in `net/bluetooth/mgmt.c`).

## What goes wrong with BlueZ 5.72 on kernel 7.0

### 1. Wake support refused for privacy devices

`device_set_wake_support()` in BlueZ 5.72:

```c
if (device->rpa && !btd_adapter_has_exp_feature(device->adapter,
					EXP_FEAT_RPA_RESOLUTION)) {
	warn("Unable to set wake_support without RPA resolution");
	return;
}
```

`EXP_FEAT_RPA_RESOLUTION` is the kernel experimental feature
`15c0a148-c273-11ea-b3de-0242ac130004`. Kernel 7.0 doesn't have it any more:
LL Privacy is used automatically when the controller supports it
(`ll_privacy_capable()` in `include/net/bluetooth/hci_core.h`) and reported
as `MGMT_SETTING_LL_PRIVACY`. So the check can never pass, `WakeAllowed`
disappears from D-Bus and the flag is never set. BlueZ master no longer has
this check.

### 2. Remote Wakeup overwritten by Device Privacy Mode

With the check above out of the way, btmon still shows (5.72):

```
@ MGMT Command: Set Device Flags (0x0050)
        LE Address: C0:FF:EE:00:00:01 (Static)
        Current Flags: 0x00000001
          Remote Wakeup
@ MGMT Event: Command Complete ... Status: Success
@ MGMT Command: Set Device Flags (0x0050)
        LE Address: C0:FF:EE:00:00:01 (Static)
        Current Flags: 0x00000002
          Device Privacy Mode
@ MGMT Event: Command Complete ... Status: Success
```

After Add Device, bluetoothd sets Remote Wakeup, then `add_device_complete()`
sets Device Privacy Mode from `current_flags`, which doesn't contain the wake
bit yet (the kernel doesn't send Device Flags Changed back to the socket that
set the flags, and the completion handler doesn't record it). The second
request carries 0x2 only and the kernel drops Remote Wakeup. D-Bus still says
`WakeAllowed: true`. Upstream BlueZ fixed this with `pending_flags`
("Include the pending flags, or they may get overwritten").

`bluez-patches/` contains the two patches that fix both problems in BlueZ 5.72
itself; they were used to confirm the analysis. `ubuntu-ble-wake-fix` makes them
unnecessary.

### 3. USB wakeup off

`/sys/bus/usb/devices/<controller>/power/wakeup` and its parent hubs are
`disabled` by default, so step 2 above returns early.

## What ubuntu-ble-wake-fix does

- Opens a trusted management socket (`AF_BLUETOOTH`, `BTPROTO_HCI`,
  `HCI_CHANNEL_CONTROL`; needs `CAP_NET_ADMIN`).
- Reads bonded devices from `/var/lib/bluetooth/<adapter>/<device>/info` and
  picks input devices (HID service `0x1812` / `0x1124`, or HID appearance).
- For each of them: `Get Device Flags`; if Remote Wakeup is supported but not
  set, `Set Device Flags` with the current value plus Remote Wakeup. Other
  bits (privacy, address resolution) are kept.
- Listens for `Device Flags Changed` events: when bluetoothd (or anything)
  clears the bit, it is set again. bluetoothd receives the kernel's event for
  that change and updates its own state, so the two don't fight: BlueZ only
  rewrites flags on Add Device (start, pairing) or when the user changes
  `WakeAllowed`.
- Enables `power/wakeup` on the controller's USB device and every hub above
  it (also when a controller appears later).
- Re-checks everything once a minute.

It never removes a flag, never touches devices that aren't paired input
devices, and doesn't change BlueZ's files.

## Debugging

```bash
sudo ubuntu-ble-wake-fix --check                    # devices, flags, USB wakeup
journalctl -u ubuntu-ble-wake-fix -u bluetooth -b   # what was changed and when
journalctl -b | grep "Controller resume"   # wake event 0x0 = not Bluetooth
```

btmon from BlueZ 5.72 crashes on terminals wider than 255 columns (fixed
upstream in 2908491c7e); send its output elsewhere:
`sudo btmon -w capture.btsnoop > /dev/null`, read with `btmon -r`.
