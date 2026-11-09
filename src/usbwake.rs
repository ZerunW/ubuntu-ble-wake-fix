//! USB remote wakeup for a USB Bluetooth controller: enable power/wakeup on
//! the controller and on every hub above it up to the root hub. Without this
//! the kernel does not even configure Bluetooth wake (btusb's wakeup callback
//! returns device_may_wakeup() of the USB device).

use std::path::{Path, PathBuf};

/// USB devices from the controller up to its root hub (closest first).
/// Empty for non-USB controllers.
pub fn chain(sys: &Path, index: u16) -> Vec<PathBuf> {
    let Ok(mut p) = std::fs::canonicalize(sys.join(format!("class/bluetooth/hci{index}/device"))) else {
        return vec![];
    };
    let mut out = Vec::new();
    loop {
        // a USB device (not an interface) has idVendor; root hubs are named usbN
        if p.join("idVendor").exists() && p.join("power/wakeup").exists() {
            out.push(p.clone());
            if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("usb")) {
                break;
            }
        }
        if !p.pop() || p == sys || p.as_os_str().len() <= 1 {
            break;
        }
    }
    out
}

/// Current power/wakeup values, closest device first.
pub fn status(sys: &Path, index: u16) -> Vec<(String, String)> {
    chain(sys, index)
        .into_iter()
        .map(|p| {
            let v = std::fs::read_to_string(p.join("power/wakeup")).unwrap_or_default().trim().to_string();
            (p.file_name().unwrap_or_default().to_string_lossy().into_owned(), v)
        })
        .collect()
}

/// Enable wakeup along the chain. Returns the devices that were changed.
pub fn enable(sys: &Path, index: u16) -> Result<Vec<String>, String> {
    let mut changed = Vec::new();
    for p in chain(sys, index) {
        let f = p.join("power/wakeup");
        let cur = std::fs::read_to_string(&f).unwrap_or_default();
        if cur.trim() == "enabled" {
            continue;
        }
        std::fs::write(&f, "enabled").map_err(|e| format!("{}: {e}", f.display()))?;
        changed.push(p.file_name().unwrap_or_default().to_string_lossy().into_owned());
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_up_to_root_hub() {
        let d = std::env::temp_dir().join(format!("ubuntu-ble-wake-fix-usb-{}", std::process::id()));
        let sys = d.join("sys");
        let root_hub = sys.join("devices/pci0000:00/0000:00:14.0/usb1");
        let ctrl = root_hub.join("1-14");
        let intf = ctrl.join("1-14:1.0");
        let hci = intf.join("bluetooth/hci0");
        std::fs::create_dir_all(&hci).unwrap();
        for dev in [&root_hub, &ctrl] {
            std::fs::create_dir_all(dev.join("power")).unwrap();
            std::fs::write(dev.join("idVendor"), "8087\n").unwrap();
            std::fs::write(dev.join("power/wakeup"), "disabled\n").unwrap();
        }
        // the PCI controller also has power/wakeup but no idVendor: not touched
        std::fs::create_dir_all(sys.join("devices/pci0000:00/0000:00:14.0/power")).unwrap();
        std::fs::write(sys.join("devices/pci0000:00/0000:00:14.0/power/wakeup"), "enabled\n").unwrap();
        std::fs::create_dir_all(sys.join("class/bluetooth")).unwrap();
        std::os::unix::fs::symlink(&hci, sys.join("class/bluetooth/hci0")).unwrap();
        std::os::unix::fs::symlink(&intf, hci.join("device")).unwrap();

        let names: Vec<_> = chain(&sys, 0).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["1-14", "usb1"]);
        assert_eq!(enable(&sys, 0).unwrap(), ["1-14", "usb1"]);
        assert!(status(&sys, 0).iter().all(|(_, v)| v == "enabled"));
        assert!(enable(&sys, 0).unwrap().is_empty(), "nothing to do the second time");
        assert!(chain(&sys, 5).is_empty());
        std::fs::remove_dir_all(d).ok();
    }
}
