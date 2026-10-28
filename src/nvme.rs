//! NVMe drive health: the SMART / Health Information log page (log 02h) every
//! NVMe controller keeps. Its power-on-hours counter is the closest thing a
//! machine has to an odometer, which is why [`crate::age`] reads it; the wear
//! and error counters ride along because a fleet wants them anyway.
//!
//! Reading it means sending the controller an admin command, which both
//! platforms gate on privilege: Linux needs root to open `/dev/nvmeN`, and
//! Windows may refuse the query to a non-elevated process. A refusal is
//! normal, not an error — the list just comes back empty. SATA drives aren't
//! read: their SMART attributes are vendor-numbered and need a different
//! pass-through.

use serde::{Deserialize, Serialize};

/// One NVMe controller's health counters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NvmeHealth {
    /// `nvme0` on Linux, `PhysicalDrive0` on Windows.
    pub device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Hours the controller has been powered. Many drives keep counting in
    /// low-power idle, so on a laptop that sleeps rather than shuts down this
    /// runs closer to wall-clock time than to hours of use.
    pub power_on_hours: u64,
    pub power_cycles: u64,
    pub unsafe_shutdowns: u64,
    /// The vendor's estimate of rated endurance used, in percent. Can pass 100.
    pub percentage_used: u8,
    pub available_spare_pct: u8,
    /// Total host writes, in decimal terabytes.
    pub data_written_tb: f64,
    pub media_errors: u64,
    /// The critical-warning bitfield; non-zero means the drive is reporting a
    /// problem (spare low, temperature, reliability degraded, read-only, …).
    pub critical_warning: u8,
}

/// Every NVMe drive whose health log this process is allowed to read.
pub fn collect() -> Vec<NvmeHealth> {
    let drives = platform_collect();
    log::debug!(
        target: "loadbearer::nvme",
        "read health from {} drive(s){}",
        drives.len(),
        drives
            .iter()
            .map(|d| format!(" · {} {} h", d.device, d.power_on_hours))
            .collect::<String>(),
    );
    drives
}

/// Decode a 512-byte SMART / Health Information log page. Counters are
/// 128-bit little-endian; nothing real overflows the low 64 bits.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn parse_health_log(device: String, model: Option<String>, log: &[u8]) -> Option<NvmeHealth> {
    if log.len() < 512 {
        return None;
    }
    let u64_at = |at: usize| u64::from_le_bytes(log[at..at + 8].try_into().unwrap());
    // A data unit is 1000 512-byte sectors.
    let written_bytes = u64_at(48) as f64 * 512_000.0;
    Some(NvmeHealth {
        device,
        model,
        power_on_hours: u64_at(128),
        power_cycles: u64_at(112),
        unsafe_shutdowns: u64_at(144),
        percentage_used: log[5],
        available_spare_pct: log[3],
        // To 10 GB: more digits are noise in a result file.
        data_written_tb: (written_bytes / 1e10).round() / 100.0,
        media_errors: u64_at(160),
        critical_warning: log[0],
    })
}

/// NVMe Get Log Page: admin opcode, the health log's id, and its size.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const LOG_HEALTH: u32 = 0x02;
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const LOG_LEN: usize = 512;

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn platform_collect() -> Vec<NvmeHealth> {
    let Ok(dir) = std::fs::read_dir("/sys/class/nvme") else {
        return Vec::new();
    };
    let mut names: Vec<String> = dir
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("nvme"))
        .collect();
    // nvme0 first: it's the one `age` reads, and usually the boot drive.
    names.sort_by_key(|n| (n.len(), n.clone()));
    names
        .into_iter()
        .filter_map(|name| {
            let model = std::fs::read_to_string(format!("/sys/class/nvme/{name}/model"))
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let log = linux::health_log(&name)?;
            parse_health_log(name, model, &log)
        })
        .collect()
}

#[cfg(target_os = "linux")]
mod linux {
    use std::os::fd::AsRawFd;

    /// `struct nvme_admin_cmd` from `<linux/nvme_ioctl.h>`.
    #[repr(C)]
    #[derive(Default)]
    struct AdminCmd {
        opcode: u8,
        flags: u8,
        rsvd1: u16,
        nsid: u32,
        cdw2: u32,
        cdw3: u32,
        metadata: u64,
        addr: u64,
        metadata_len: u32,
        data_len: u32,
        cdw10: u32,
        cdw11: u32,
        cdw12: u32,
        cdw13: u32,
        cdw14: u32,
        cdw15: u32,
        timeout_ms: u32,
        result: u32,
    }

    /// `_IOWR('N', 0x41, struct nvme_admin_cmd)`.
    const NVME_IOCTL_ADMIN_CMD: u32 = 0xC048_4E41;

    pub fn health_log(name: &str) -> Option<Vec<u8>> {
        // The controller's character device is root-only by default.
        let dev = match std::fs::File::open(format!("/dev/{name}")) {
            Ok(f) => f,
            Err(e) => {
                log::debug!(target: "loadbearer::nvme", "open /dev/{name}: {e}");
                return None;
            }
        };
        let mut buf = vec![0u8; super::LOG_LEN];
        let numd = (super::LOG_LEN / 4 - 1) as u32;
        let mut cmd = AdminCmd {
            opcode: 0x02, // Get Log Page
            nsid: 0xFFFF_FFFF,
            addr: buf.as_mut_ptr() as u64,
            data_len: super::LOG_LEN as u32,
            cdw10: (numd << 16) | super::LOG_HEALTH,
            ..Default::default()
        };
        // SAFETY: `cmd` matches the kernel's layout, and `addr` points at
        // `buf`, which is `data_len` bytes and outlives the call.
        let rc = unsafe { libc::ioctl(dev.as_raw_fd(), NVME_IOCTL_ADMIN_CMD as _, &mut cmd) };
        if rc != 0 {
            log::debug!(
                target: "loadbearer::nvme",
                "Get Log Page on /dev/{name} failed: rc={rc} {}",
                std::io::Error::last_os_error(),
            );
            return None;
        }
        Some(buf)
    }
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn platform_collect() -> Vec<NvmeHealth> {
    // Drive numbers can have gaps (a removed USB disk), so try a fixed range
    // rather than stopping at the first miss.
    (0..16)
        .filter_map(|i| {
            let name = format!("PhysicalDrive{i}");
            let drive = win::Drive::open(&name)?;
            let model = drive.nvme_model()?;
            let log = drive.health_log()?;
            parse_health_log(name, model, &log)
        })
        .collect()
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    /// `CTL_CODE(IOCTL_STORAGE_BASE, 0x500, METHOD_BUFFERED, FILE_ANY_ACCESS)`.
    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
    // STORAGE_PROPERTY_ID values.
    const STORAGE_DEVICE_PROPERTY: u32 = 0;
    const STORAGE_DEVICE_PROTOCOL_SPECIFIC_PROPERTY: u32 = 50;
    const BUS_TYPE_NVME: u32 = 17;
    const PROTOCOL_TYPE_NVME: u32 = 3;
    const NVME_DATA_TYPE_LOG_PAGE: u32 = 2;
    /// `sizeof(STORAGE_PROTOCOL_SPECIFIC_DATA)`: ten `u32`s.
    const PROTOCOL_DATA_LEN: usize = 40;

    pub struct Drive(HANDLE);

    impl Drive {
        /// Open with no access rights: property queries don't need any, and
        /// asking for read access is what would require elevation.
        pub fn open(name: &str) -> Option<Drive> {
            let path: Vec<u16> = format!("\\\\.\\{name}\0").encode_utf16().collect();
            // SAFETY: `path` is NUL-terminated UTF-16 that outlives the call;
            // the other pointers are documented as optional.
            let h = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            (h != INVALID_HANDLE_VALUE).then_some(Drive(h))
        }

        /// `IOCTL_STORAGE_QUERY_PROPERTY` with `input` in and up to `out_len`
        /// bytes back.
        fn query(&self, input: &[u8], out_len: usize) -> Option<Vec<u8>> {
            let mut out = vec![0u8; out_len];
            let mut got = 0u32;
            // SAFETY: both buffers are live for the call and their lengths
            // are passed alongside them; `got` receives the byte count.
            let ok = unsafe {
                DeviceIoControl(
                    self.0,
                    IOCTL_STORAGE_QUERY_PROPERTY,
                    input.as_ptr().cast(),
                    input.len() as u32,
                    out.as_mut_ptr().cast(),
                    out.len() as u32,
                    &mut got,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                log::debug!(
                    target: "loadbearer::nvme",
                    "IOCTL_STORAGE_QUERY_PROPERTY failed: {}",
                    std::io::Error::last_os_error(),
                );
                return None;
            }
            out.truncate(got as usize);
            Some(out)
        }

        /// `None` when this isn't an NVMe drive; `Some(model)` when it is,
        /// with the model itself optional.
        pub fn nvme_model(&self) -> Option<Option<String>> {
            // STORAGE_PROPERTY_QUERY { PropertyId, QueryType = standard, … }.
            let mut q = [0u8; 12];
            q[0..4].copy_from_slice(&STORAGE_DEVICE_PROPERTY.to_le_bytes());
            let d = self.query(&q, 1024)?;
            let u32_at = |at: usize| {
                d.get(at..at + 4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            };
            // STORAGE_DEVICE_DESCRIPTOR: BusType at 28, ProductIdOffset at 16.
            if u32_at(28)? != BUS_TYPE_NVME {
                return None;
            }
            let model = u32_at(16)
                .map(|o| o as usize)
                .filter(|&o| o != 0 && o < d.len())
                .and_then(|o| {
                    let s = &d[o..];
                    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
                    let m = String::from_utf8_lossy(&s[..end]).trim().to_string();
                    (!m.is_empty()).then_some(m)
                });
            Some(model)
        }

        pub fn health_log(&self) -> Option<Vec<u8>> {
            // STORAGE_PROPERTY_QUERY header (8 bytes), then a
            // STORAGE_PROTOCOL_SPECIFIC_DATA in AdditionalParameters, then
            // room for the log page itself.
            let mut q = vec![0u8; 8 + PROTOCOL_DATA_LEN + super::LOG_LEN];
            let mut put = |at: usize, v: u32| q[at..at + 4].copy_from_slice(&v.to_le_bytes());
            put(0, STORAGE_DEVICE_PROTOCOL_SPECIFIC_PROPERTY);
            put(8, PROTOCOL_TYPE_NVME);
            put(12, NVME_DATA_TYPE_LOG_PAGE);
            put(16, super::LOG_HEALTH);
            put(24, PROTOCOL_DATA_LEN as u32); // ProtocolDataOffset
            put(28, super::LOG_LEN as u32); // ProtocolDataLength
            let len = q.len();
            let out = self.query(&q, len)?;

            // STORAGE_PROTOCOL_DATA_DESCRIPTOR: Version, Size, then the
            // protocol data block echoed back with its offset and length.
            let u32_at = |at: usize| {
                out.get(at..at + 4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
            };
            let offset = u32_at(8 + 16)?;
            let length = u32_at(8 + 20)?;
            let start = 8 + offset;
            if length < super::LOG_LEN || start + super::LOG_LEN > out.len() {
                return None;
            }
            Some(out[start..start + super::LOG_LEN].to_vec())
        }
    }

    impl Drop for Drive {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful CreateFileW and is
            // closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_collect() -> Vec<NvmeHealth> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_health_log_page() {
        let mut log = [0u8; 512];
        log[0] = 0x04; // reliability degraded
        log[3] = 100; // available spare
        log[5] = 7; // percentage used
        log[48..56].copy_from_slice(&20_000_000u64.to_le_bytes()); // data units written
        log[112..120].copy_from_slice(&1_234u64.to_le_bytes());
        log[128..136].copy_from_slice(&11_800u64.to_le_bytes());
        log[144..152].copy_from_slice(&56u64.to_le_bytes());
        log[160..168].copy_from_slice(&2u64.to_le_bytes());

        let h = parse_health_log("nvme0".into(), Some("ACME SSD".into()), &log).unwrap();
        assert_eq!(h.power_on_hours, 11_800);
        assert_eq!(h.power_cycles, 1_234);
        assert_eq!(h.unsafe_shutdowns, 56);
        assert_eq!(h.media_errors, 2);
        assert_eq!(h.percentage_used, 7);
        assert_eq!(h.available_spare_pct, 100);
        assert_eq!(h.critical_warning, 0x04);
        assert_eq!(h.data_written_tb, 10.24);
    }

    #[test]
    fn a_short_page_is_rejected() {
        assert_eq!(parse_health_log("nvme0".into(), None, &[0u8; 100]), None);
    }
}
