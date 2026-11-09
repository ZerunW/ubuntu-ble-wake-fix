//! Minimal client for the kernel Bluetooth management interface (mgmt):
//! an AF_BLUETOOTH / BTPROTO_HCI socket bound to HCI_CHANNEL_CONTROL.
//! This is the same stable kernel interface bluetoothd uses. Packet layout
//! and opcodes are from include/net/bluetooth/mgmt.h (Linux v7.0).
//! Opening a trusted socket (needed to change flags) requires CAP_NET_ADMIN.

use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

pub const INDEX_NONE: u16 = 0xFFFF;

pub const OP_READ_INDEX_LIST: u16 = 0x0003;
pub const OP_READ_INFO: u16 = 0x0004;
pub const OP_GET_DEVICE_FLAGS: u16 = 0x004F;
pub const OP_SET_DEVICE_FLAGS: u16 = 0x0050;

pub const EV_CMD_COMPLETE: u16 = 0x0001;
pub const EV_CMD_STATUS: u16 = 0x0002;
pub const EV_INDEX_ADDED: u16 = 0x0004;
pub const EV_DEVICE_FLAGS_CHANGED: u16 = 0x002A;

/// Device flag bits (mgmt.h: DEVICE_FLAG_*).
pub const FLAG_REMOTE_WAKEUP: u32 = 1 << 0;

/// Address types used by mgmt (BDADDR_BREDR, BDADDR_LE_PUBLIC, BDADDR_LE_RANDOM).
pub const ADDR_BREDR: u8 = 0;
pub const ADDR_LE_PUBLIC: u8 = 1;
pub const ADDR_LE_RANDOM: u8 = 2;

const AF_BLUETOOTH: libc::c_int = 31;
const BTPROTO_HCI: libc::c_int = 1;
const HCI_CHANNEL_CONTROL: u16 = 3;
const HCI_DEV_NONE: u16 = 0xFFFF;

#[repr(C)]
struct SockaddrHci {
    family: libc::sa_family_t,
    dev: u16,
    channel: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub code: u16,
    pub index: u16,
    pub data: Vec<u8>,
}

impl Packet {
    pub fn parse(buf: &[u8]) -> Option<Packet> {
        if buf.len() < 6 {
            return None;
        }
        let code = u16::from_le_bytes([buf[0], buf[1]]);
        let index = u16::from_le_bytes([buf[2], buf[3]]);
        let len = u16::from_le_bytes([buf[4], buf[5]]) as usize;
        let data = buf.get(6..6 + len)?.to_vec();
        Some(Packet { code, index, data })
    }

    pub fn encode(code: u16, index: u16, params: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(6 + params.len());
        v.extend_from_slice(&code.to_le_bytes());
        v.extend_from_slice(&index.to_le_bytes());
        v.extend_from_slice(&(params.len() as u16).to_le_bytes());
        v.extend_from_slice(params);
        v
    }
}

/// "Device Flags Changed" event payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlagsChanged {
    pub addr: [u8; 6],
    pub addr_type: u8,
    pub supported: u32,
    pub current: u32,
}

impl FlagsChanged {
    pub fn parse(d: &[u8]) -> Option<FlagsChanged> {
        if d.len() < 15 {
            return None;
        }
        let mut addr = [0u8; 6];
        addr.copy_from_slice(&d[0..6]);
        Some(FlagsChanged {
            addr,
            addr_type: d[6],
            supported: u32::from_le_bytes(d[7..11].try_into().ok()?),
            current: u32::from_le_bytes(d[11..15].try_into().ok()?),
        })
    }
}

pub struct Mgmt {
    fd: OwnedFd,
    /// Events read while waiting for a command reply.
    pending: VecDeque<Packet>,
}

/// Command failed with this mgmt status (e.g. 0x0d Invalid Parameters when the
/// kernel does not know the device yet).
#[derive(Debug)]
pub enum CmdError {
    Io(io::Error),
    Status(u8),
}

impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CmdError::Io(e) => write!(f, "{e}"),
            CmdError::Status(s) => write!(f, "mgmt status 0x{s:02x}"),
        }
    }
}

impl From<io::Error> for CmdError {
    fn from(e: io::Error) -> Self {
        CmdError::Io(e)
    }
}

impl Mgmt {
    pub fn open() -> io::Result<Mgmt> {
        // SAFETY: plain socket(2) call; the result is checked before use.
        let fd = unsafe { libc::socket(AF_BLUETOOTH, libc::SOCK_RAW | libc::SOCK_CLOEXEC, BTPROTO_HCI) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fd is a fresh, valid descriptor we own.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let sa = SockaddrHci { family: AF_BLUETOOTH as libc::sa_family_t, dev: HCI_DEV_NONE, channel: HCI_CHANNEL_CONTROL };
        // SAFETY: sa is a valid sockaddr_hci and the length matches it.
        let r = unsafe {
            libc::bind(
                fd.as_raw_fd(),
                &sa as *const SockaddrHci as *const libc::sockaddr,
                std::mem::size_of::<SockaddrHci>() as libc::socklen_t,
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Mgmt { fd, pending: VecDeque::new() })
    }

    fn send(&self, code: u16, index: u16, params: &[u8]) -> io::Result<()> {
        let buf = Packet::encode(code, index, params);
        // SAFETY: buf is a valid buffer of buf.len() bytes.
        let n = unsafe { libc::write(self.fd.as_raw_fd(), buf.as_ptr() as *const libc::c_void, buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Read one packet, waiting at most `timeout`. None on timeout.
    fn recv(&self, timeout: Duration) -> io::Result<Option<Packet>> {
        let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
        // SAFETY: pfd is a valid pollfd array of length 1.
        let r = unsafe { libc::poll(&mut pfd, 1, ms) };
        if r < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::Interrupted { Ok(None) } else { Err(e) };
        }
        if r == 0 {
            return Ok(None);
        }
        let mut buf = [0u8; 4096];
        // SAFETY: buf is a valid writable buffer of buf.len() bytes.
        let n = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "mgmt socket closed"));
        }
        Ok(Packet::parse(&buf[..n as usize]))
    }

    /// Send a command and wait for its reply. Events that arrive meanwhile are
    /// kept for `next_event`.
    pub fn command(&mut self, code: u16, index: u16, params: &[u8]) -> Result<Vec<u8>, CmdError> {
        self.send(code, index, params)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(CmdError::Io(io::Error::new(io::ErrorKind::TimedOut, format!("no reply to mgmt command 0x{code:04x}"))));
            }
            let Some(p) = self.recv(left)? else { continue };
            let is_reply = (p.code == EV_CMD_COMPLETE || p.code == EV_CMD_STATUS)
                && p.index == index
                && p.data.len() >= 3
                && u16::from_le_bytes([p.data[0], p.data[1]]) == code;
            if !is_reply {
                self.pending.push_back(p);
                continue;
            }
            let status = p.data[2];
            if status != 0 {
                return Err(CmdError::Status(status));
            }
            return Ok(p.data[3..].to_vec());
        }
    }

    /// Next event (queued first), waiting at most `timeout`.
    pub fn next_event(&mut self, timeout: Duration) -> io::Result<Option<Packet>> {
        if let Some(p) = self.pending.pop_front() {
            return Ok(Some(p));
        }
        self.recv(timeout)
    }

    /// Controller indexes. Also subscribes this socket to Index Added events.
    pub fn index_list(&mut self) -> Result<Vec<u16>, CmdError> {
        let d = self.command(OP_READ_INDEX_LIST, INDEX_NONE, &[])?;
        if d.len() < 2 {
            return Ok(vec![]);
        }
        let n = u16::from_le_bytes([d[0], d[1]]) as usize;
        Ok((0..n).filter_map(|i| d.get(2 + 2 * i..4 + 2 * i).map(|b| u16::from_le_bytes([b[0], b[1]]))).collect())
    }

    /// Controller address (little endian, as on the wire).
    pub fn controller_addr(&mut self, index: u16) -> Result<[u8; 6], CmdError> {
        let d = self.command(OP_READ_INFO, index, &[])?;
        let mut a = [0u8; 6];
        a.copy_from_slice(d.get(0..6).ok_or(CmdError::Status(0xff))?);
        Ok(a)
    }

    /// (supported, current) flags of a device.
    pub fn get_flags(&mut self, index: u16, addr: &[u8; 6], addr_type: u8) -> Result<(u32, u32), CmdError> {
        let mut p = addr.to_vec();
        p.push(addr_type);
        let d = self.command(OP_GET_DEVICE_FLAGS, index, &p)?;
        if d.len() < 15 {
            return Err(CmdError::Status(0xff));
        }
        Ok((u32::from_le_bytes(d[7..11].try_into().unwrap()), u32::from_le_bytes(d[11..15].try_into().unwrap())))
    }

    pub fn set_flags(&mut self, index: u16, addr: &[u8; 6], addr_type: u8, flags: u32) -> Result<(), CmdError> {
        let mut p = addr.to_vec();
        p.push(addr_type);
        p.extend_from_slice(&flags.to_le_bytes());
        self.command(OP_SET_DEVICE_FLAGS, index, &p).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_round_trip() {
        let b = Packet::encode(OP_SET_DEVICE_FLAGS, 0, &[1, 2, 3]);
        assert_eq!(b, vec![0x50, 0x00, 0x00, 0x00, 0x03, 0x00, 1, 2, 3]);
        let p = Packet::parse(&b).unwrap();
        assert_eq!(p, Packet { code: 0x50, index: 0, data: vec![1, 2, 3] });
        assert!(Packet::parse(&b[..7]).is_none(), "truncated payload");
    }

    #[test]
    fn flags_changed_event() {
        // as captured with btmon (address replaced): C0:FF:EE:00:00:01 (LE random), supported 0x7, current 0x2
        let d = [0x01, 0x00, 0x00, 0xee, 0xff, 0xc0, 0x02, 0x07, 0, 0, 0, 0x02, 0, 0, 0];
        let e = FlagsChanged::parse(&d).unwrap();
        assert_eq!(e.addr_type, ADDR_LE_RANDOM);
        assert_eq!((e.supported, e.current), (7, 2));
        assert!(FlagsChanged::parse(&d[..14]).is_none());
    }
}
