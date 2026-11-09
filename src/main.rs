//! ubuntu-ble-wake-fix: let paired Bluetooth keyboards and mice wake Linux from suspend.
//!
//! Whether a Bluetooth device can wake the host is decided by one kernel flag
//! per device (Remote Wakeup, set through the Bluetooth management interface).
//! Some BlueZ versions never set it or clear it again (see README). This
//! daemon keeps it set for bonded input devices, and enables USB wakeup on the
//! Bluetooth controller. It does not modify or replace BlueZ: if BlueZ sets the
//! flag correctly, there is simply nothing to do.

mod mgmt;
mod store;
mod usbwake;

use mgmt::{CmdError, FlagsChanged, Mgmt, FLAG_REMOTE_WAKEUP};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use store::{fmt_addr, Device};

const USAGE: &str = "usage: ubuntu-ble-wake-fix [--check | --once] [options]

  (no mode)        run as a service: keep wake enabled, react to changes
  --check          print paired input devices, their wake flags and USB wakeup; change nothing
  --once           fix everything once and exit

options:
  --storage DIR    BlueZ storage (default /var/lib/bluetooth)
  --exclude FILE   addresses that must not wake the host (default /etc/ubuntu-ble-wake-fix/exclude)
  --sys DIR        sysfs root (default /sys)
  --interval SECS  full re-check interval in service mode (default 60)
  --no-usb         don't touch USB power/wakeup";

struct Opts {
    mode: Mode,
    storage: PathBuf,
    exclude: PathBuf,
    sys: PathBuf,
    interval: Duration,
    usb: bool,
}

#[derive(PartialEq)]
enum Mode {
    Service,
    Check,
    Once,
}

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts {
        mode: Mode::Service,
        storage: "/var/lib/bluetooth".into(),
        exclude: "/etc/ubuntu-ble-wake-fix/exclude".into(),
        sys: "/sys".into(),
        interval: Duration::from_secs(60),
        usb: true,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--check" => o.mode = Mode::Check,
            "--once" => o.mode = Mode::Once,
            "--storage" => o.storage = val()?.into(),
            "--exclude" => o.exclude = val()?.into(),
            "--sys" => o.sys = val()?.into(),
            "--interval" => {
                let s: u64 = val()?.parse().map_err(|_| "--interval needs seconds".to_string())?;
                o.interval = Duration::from_secs(s.max(5));
            }
            "--no-usb" => o.usb = false,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument {a}\n{USAGE}")),
        }
    }
    Ok(o)
}

fn type_name(t: u8) -> &'static str {
    match t {
        mgmt::ADDR_BREDR => "BR/EDR",
        mgmt::ADDR_LE_PUBLIC => "LE public",
        _ => "LE random",
    }
}

struct Daemon {
    o: Opts,
    m: Mgmt,
}

impl Daemon {
    /// Paired input devices of one controller.
    fn devices(&mut self, index: u16) -> Vec<Device> {
        let Ok(addr) = self.m.controller_addr(index) else { return vec![] };
        let ex = store::read_exclude(&self.o.exclude);
        store::scan(&self.o.storage, &ex).remove(&addr).unwrap_or_default()
    }

    /// Make sure one device has Remote Wakeup. Returns a log line if it changed.
    fn ensure(&mut self, index: u16, d: &Device) -> Option<String> {
        match self.m.get_flags(index, &d.addr, d.addr_type) {
            Ok((supported, current)) => {
                if supported & FLAG_REMOTE_WAKEUP == 0 || current & FLAG_REMOTE_WAKEUP != 0 {
                    return None;
                }
                let new = current | FLAG_REMOTE_WAKEUP;
                Some(match self.m.set_flags(index, &d.addr, d.addr_type, new) {
                    Ok(()) => format!("hci{index} {} ({}): wake enabled, flags 0x{current:x} -> 0x{new:x}", fmt_addr(&d.addr), d.name),
                    Err(e) => format!("hci{index} {} ({}): could not set flags: {e}", fmt_addr(&d.addr), d.name),
                })
            }
            // the kernel does not know the device yet (bluetoothd not started): later events cover it
            Err(CmdError::Status(_)) => None,
            Err(e) => Some(format!("hci{index} {}: get flags: {e}", fmt_addr(&d.addr))),
        }
    }

    fn usb(&self, index: u16) {
        if !self.o.usb {
            return;
        }
        match usbwake::enable(&self.o.sys, index) {
            Ok(c) if !c.is_empty() => eprintln!("hci{index}: USB wakeup enabled on {}", c.join(", ")),
            Ok(_) => {}
            Err(e) => eprintln!("hci{index}: USB wakeup: {e}"),
        }
    }

    fn resync(&mut self) {
        let idx = match self.m.index_list() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("read controller list: {e}");
                return;
            }
        };
        for i in idx {
            self.usb(i);
            for d in self.devices(i) {
                if let Some(msg) = self.ensure(i, &d) {
                    eprintln!("{msg}");
                }
            }
        }
    }

    /// Someone (normally bluetoothd) changed a device's flags.
    fn on_flags_changed(&mut self, index: u16, e: FlagsChanged) {
        if e.supported & FLAG_REMOTE_WAKEUP == 0 || e.current & FLAG_REMOTE_WAKEUP != 0 {
            return;
        }
        let d = self.devices(index).into_iter().find(|d| d.addr == e.addr && d.addr_type == e.addr_type);
        if let Some(d) = d {
            if let Some(msg) = self.ensure(index, &d) {
                eprintln!("{msg} (it had been cleared)");
            }
        }
    }

    fn run(&mut self) -> Result<(), String> {
        self.resync();
        let mut next = Instant::now() + self.o.interval;
        loop {
            let wait = next.saturating_duration_since(Instant::now());
            match self.m.next_event(wait).map_err(|e| format!("mgmt: {e}"))? {
                Some(p) if p.code == mgmt::EV_DEVICE_FLAGS_CHANGED => {
                    if let Some(e) = FlagsChanged::parse(&p.data) {
                        self.on_flags_changed(p.index, e);
                    }
                }
                Some(p) if p.code == mgmt::EV_INDEX_ADDED => {
                    eprintln!("hci{} added", p.index);
                    self.usb(p.index);
                }
                Some(_) => {}
                None => {
                    self.resync();
                    next = Instant::now() + self.o.interval;
                }
            }
        }
    }

    fn check(&mut self) -> Result<(), String> {
        let idx = self.m.index_list().map_err(|e| format!("read controller list: {e}"))?;
        if idx.is_empty() {
            println!("no Bluetooth controller");
        }
        for i in idx {
            let addr = self.m.controller_addr(i).map(|a| fmt_addr(&a)).unwrap_or_else(|e| format!("? ({e})"));
            println!("hci{i}  {addr}");
            let usb = usbwake::status(&self.o.sys, i);
            if usb.is_empty() {
                println!("  USB wakeup: not a USB controller");
            } else {
                let s: Vec<String> = usb.iter().map(|(n, v)| format!("{n}={v}")).collect();
                println!("  USB wakeup: {}", s.join("  "));
            }
            let devs = self.devices(i);
            if devs.is_empty() {
                println!("  no paired input devices");
            }
            for d in devs {
                let state = match self.m.get_flags(i, &d.addr, d.addr_type) {
                    Ok((s, c)) if c & FLAG_REMOTE_WAKEUP != 0 => format!("wake on   (flags 0x{c:x}, supported 0x{s:x})"),
                    Ok((s, c)) if s & FLAG_REMOTE_WAKEUP == 0 => format!("wake not supported by this controller (flags 0x{c:x})"),
                    Ok((s, c)) => format!("wake OFF  (flags 0x{c:x}, supported 0x{s:x})"),
                    Err(CmdError::Status(st)) => format!("not known to the kernel yet (status 0x{st:02x})"),
                    Err(e) => format!("error: {e}"),
                };
                println!("  {}  {:9}  {:20}  {state}", fmt_addr(&d.addr), type_name(d.addr_type), d.name);
            }
        }
        Ok(())
    }
}

fn main() {
    let o = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let m = match Mgmt::open() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("ubuntu-ble-wake-fix: cannot open the Bluetooth management socket: {e} (run as root)");
            std::process::exit(1);
        }
    };
    let check = o.mode == Mode::Check;
    let once = o.mode == Mode::Once;
    let mut d = Daemon { o, m };
    let r = if check {
        d.check()
    } else if once {
        d.resync();
        Ok(())
    } else {
        eprintln!("ubuntu-ble-wake-fix {} running", env!("CARGO_PKG_VERSION"));
        d.run()
    };
    if let Err(e) = r {
        eprintln!("ubuntu-ble-wake-fix: {e}");
        std::process::exit(1);
    }
}
