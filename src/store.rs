//! Paired devices from bluetoothd's storage (read only):
//! /var/lib/bluetooth/<adapter address>/<device address>/info
//! A device is a wake candidate if it is bonded (has a key) and is an input
//! device: it offers the HID service (HID over GATT 0x1812 or classic HID
//! 0x1124) or its appearance is in the HID category (0x03C0-0x03FF).

use crate::mgmt::{ADDR_BREDR, ADDR_LE_PUBLIC, ADDR_LE_RANDOM};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// Little endian, as used by mgmt.
    pub addr: [u8; 6],
    pub addr_type: u8,
    pub name: String,
}

/// "C0:FF:EE:00:00:01" -> little-endian bytes.
pub fn parse_addr(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut a = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        if p.len() != 2 {
            return None;
        }
        a[5 - i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(a)
}

pub fn fmt_addr(a: &[u8; 6]) -> String {
    format!("{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}", a[5], a[4], a[3], a[2], a[1], a[0])
}

/// Decide from an `info` file whether the device should be able to wake the
/// host. Returns (mgmt address type, name).
pub fn wake_candidate(info: &str) -> Option<(u8, String)> {
    let mut section = String::new();
    let mut general: HashMap<String, String> = HashMap::new();
    let mut bonded = false;
    for line in info.lines() {
        let line = line.trim();
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_string();
            if matches!(s, "LinkKey" | "LongTermKey" | "PeripheralLongTermKey" | "SlaveLongTermKey") {
                bonded = true;
            }
            continue;
        }
        if section == "General" {
            if let Some((k, v)) = line.split_once('=') {
                general.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    if !bonded {
        return None;
    }
    let services = general.get("Services").map(String::as_str).unwrap_or("").to_ascii_lowercase();
    let hid_service = services.contains("00001812-0000-1000-8000-00805f9b34fb") || services.contains("00001124-0000-1000-8000-00805f9b34fb");
    let hid_appearance = general
        .get("Appearance")
        .and_then(|a| u16::from_str_radix(a.trim_start_matches("0x"), 16).ok())
        .is_some_and(|a| (0x03C0..=0x03FF).contains(&a));
    if !(hid_service || hid_appearance) {
        return None;
    }
    let tech = general.get("SupportedTechnologies").map(String::as_str).unwrap_or("");
    let addr_type = if tech.split(';').any(|t| t == "LE") {
        match general.get("AddressType").map(String::as_str) {
            Some("static") => ADDR_LE_RANDOM,
            _ => ADDR_LE_PUBLIC,
        }
    } else {
        ADDR_BREDR
    };
    let name = general.get("Name").cloned().unwrap_or_default();
    Some((addr_type, name))
}

/// Wake candidates per adapter address.
pub fn scan(dir: &Path, exclude: &[[u8; 6]]) -> HashMap<[u8; 6], Vec<Device>> {
    let mut out: HashMap<[u8; 6], Vec<Device>> = HashMap::new();
    let Ok(adapters) = std::fs::read_dir(dir) else { return out };
    for a in adapters.flatten() {
        let Some(adapter) = parse_addr(&a.file_name().to_string_lossy()) else { continue };
        let Ok(devices) = std::fs::read_dir(a.path()) else { continue };
        for d in devices.flatten() {
            let Some(addr) = parse_addr(&d.file_name().to_string_lossy()) else { continue };
            if exclude.contains(&addr) {
                continue;
            }
            let Ok(info) = std::fs::read_to_string(d.path().join("info")) else { continue };
            if let Some((addr_type, name)) = wake_candidate(&info) {
                out.entry(adapter).or_default().push(Device { addr, addr_type, name });
            }
        }
    }
    for v in out.values_mut() {
        v.sort_by(|a, b| a.addr.cmp(&b.addr));
    }
    out
}

/// Addresses (one per line, '#' comments) that must never be given wake.
pub fn read_exclude(path: &Path) -> Vec<[u8; 6]> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().map(|l| l.split('#').next().unwrap_or("")).filter_map(parse_addr).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYBOARD: &str = "[General]\nName=MX MCHNCL\nAppearance=0x03c1\nAddressType=static\nSupportedTechnologies=LE;\nTrusted=true\nBlocked=false\nServices=00001800-0000-1000-8000-00805f9b34fb;0000180a-0000-1000-8000-00805f9b34fb;00001812-0000-1000-8000-00805f9b34fb;\n\n[IdentityResolvingKey]\nKey=00\n\n[LongTermKey]\nKey=00\nAuthenticated=0\n";

    #[test]
    fn address_byte_order() {
        let a = parse_addr("C0:FF:EE:00:00:01").unwrap();
        assert_eq!(a, [0x01, 0x00, 0x00, 0xee, 0xff, 0xc0]);
        assert_eq!(fmt_addr(&a), "C0:FF:EE:00:00:01");
        assert!(parse_addr("C0:FF:EE").is_none());
        assert!(parse_addr("cache").is_none());
    }

    #[test]
    fn keyboard_is_candidate() {
        assert_eq!(wake_candidate(KEYBOARD), Some((ADDR_LE_RANDOM, "MX MCHNCL".into())));
    }

    #[test]
    fn not_bonded_or_not_input() {
        let unbonded = KEYBOARD.split("[IdentityResolvingKey]").next().unwrap();
        assert_eq!(wake_candidate(unbonded), None);
        let headphones = "[General]\nName=Buds\nSupportedTechnologies=BR/EDR;\nServices=0000110b-0000-1000-8000-00805f9b34fb;\n[LinkKey]\nKey=00\n";
        assert_eq!(wake_candidate(headphones), None);
    }

    #[test]
    fn classic_hid_and_appearance_only() {
        let classic = "[General]\nName=Old Kbd\nSupportedTechnologies=BR/EDR;\nServices=00001124-0000-1000-8000-00805f9b34fb;\n[LinkKey]\nKey=00\n";
        assert_eq!(wake_candidate(classic), Some((ADDR_BREDR, "Old Kbd".into())));
        let mouse = "[General]\nName=M\nAppearance=0x03c2\nAddressType=public\nSupportedTechnologies=LE;\n[LongTermKey]\nKey=00\n";
        assert_eq!(wake_candidate(mouse), Some((ADDR_LE_PUBLIC, "M".into())));
    }

    #[test]
    fn scan_tree_and_exclude() {
        let d = std::env::temp_dir().join(format!("ubuntu-ble-wake-fix-store-{}", std::process::id()));
        let dev = d.join("00:00:5E:00:53:01/C0:FF:EE:00:00:01");
        std::fs::create_dir_all(&dev).unwrap();
        std::fs::write(dev.join("info"), KEYBOARD).unwrap();
        std::fs::create_dir_all(d.join("00:00:5E:00:53:01/cache")).unwrap();
        let m = scan(&d, &[]);
        let adapter = parse_addr("00:00:5E:00:53:01").unwrap();
        assert_eq!(m[&adapter].len(), 1);
        assert_eq!(m[&adapter][0].name, "MX MCHNCL");
        let ex = d.join("exclude");
        std::fs::write(&ex, "# no wake\nc0:ff:ee:00:00:01  # keyboard\n").unwrap();
        assert!(scan(&d, &read_exclude(&ex)).is_empty());
        std::fs::remove_dir_all(d).ok();
    }
}
