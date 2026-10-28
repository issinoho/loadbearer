//! Drive health: the counters a drive keeps about itself. Its power-on hours
//! are the closest thing a machine has to an odometer, which is why
//! [`crate::age`] reads them; the wear and error counters ride along because
//! a fleet wants them anyway.
//!
//! Two sources, one shape:
//!
//! - **NVMe**: the SMART / Health Information log page (log 02h), which every
//!   NVMe controller keeps and which fills every field.
//! - **SATA**: the ATA Device Statistics log (log 04h) — General Statistics
//!   for hours, power cycles and sectors written, Solid State Statistics for
//!   wear. Not the older SMART attribute table: its attribute 9 counts in
//!   whatever unit the vendor chose (hours, minutes, half-minutes), where the
//!   statistics log is defined in hours. A drive that predates the log
//!   (ACS-2, around 2011) reports nothing rather than a guess.
//!
//! Fields one source can't fill are `None`. Reading either means sending the
//! drive a command, which both platforms gate on privilege: Linux needs root
//! to open the device node, and Windows may refuse the query to a
//! non-elevated process. A refusal is normal, not an error — the list just
//! comes back shorter.

use serde::{Deserialize, Serialize};

/// One drive's health counters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriveHealth {
    /// `nvme0` / `sda` on Linux, `PhysicalDrive0` on Windows.
    pub device: String,
    /// `"nvme"` or `"sata"`.
    pub interface: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Hours the drive has been powered. Many drives keep counting in
    /// low-power idle, so on a laptop that sleeps rather than shuts down this
    /// runs closer to wall-clock time than to hours of use.
    pub power_on_hours: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_cycles: Option<u64>,
    /// NVMe only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsafe_shutdowns: Option<u64>,
    /// The vendor's estimate of rated endurance used, in percent. Can pass
    /// 100. SSDs only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percentage_used: Option<u8>,
    /// NVMe only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_spare_pct: Option<u8>,
    /// Total host writes, in decimal terabytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_written_tb: Option<f64>,
    /// NVMe only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_errors: Option<u64>,
    /// NVMe's critical-warning bitfield; non-zero means the drive is reporting
    /// a problem (spare low, temperature, reliability degraded, read-only, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_warning: Option<u8>,
}

/// Every drive whose health this process is allowed to read. NVMe drives come
/// first on Linux; on Windows the order is the physical drive number. Either
/// way the first is usually the boot drive, which is the one `age` reads.
pub fn collect() -> Vec<DriveHealth> {
    let drives = platform_collect();
    log::debug!(
        target: "loadbearer::drives",
        "read health from {} drive(s){}",
        drives.len(),
        drives
            .iter()
            .map(|d| format!(" · {} ({}) {} h", d.device, d.interface, d.power_on_hours))
            .collect::<String>(),
    );
    drives
}

/// To 10 GB: more digits are noise in a result file.
fn terabytes(bytes: f64) -> f64 {
    (bytes / 1e10).round() / 100.0
}

// ---------------------------------------------------------------------------
// Decoding (platform-independent, tested everywhere)
// ---------------------------------------------------------------------------

/// NVMe Get Log Page: the health log's id and size.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const NVME_LOG_HEALTH: u32 = 0x02;
/// Every log read here is one 512-byte page.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const LOG_LEN: usize = 512;
/// ATA Device Statistics: the log address and the two pages read from it.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const ATA_LOG_DEVICE_STATISTICS: u8 = 0x04;
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const ATA_STATS_GENERAL: u8 = 0x01;
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
const ATA_STATS_SOLID_STATE: u8 = 0x07;

/// Decode a 512-byte NVMe SMART / Health Information log page. Counters are
/// 128-bit little-endian; nothing real overflows the low 64 bits.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn parse_nvme_health(device: String, model: Option<String>, log: &[u8]) -> Option<DriveHealth> {
    if log.len() < LOG_LEN {
        return None;
    }
    let u64_at = |at: usize| u64::from_le_bytes(log[at..at + 8].try_into().unwrap());
    // A data unit is 1000 512-byte sectors.
    let written_bytes = u64_at(48) as f64 * 512_000.0;
    Some(DriveHealth {
        device,
        interface: "nvme".to_string(),
        model,
        power_on_hours: u64_at(128),
        power_cycles: Some(u64_at(112)),
        unsafe_shutdowns: Some(u64_at(144)),
        percentage_used: Some(log[5]),
        available_spare_pct: Some(log[3]),
        data_written_tb: Some(terabytes(written_bytes)),
        media_errors: Some(u64_at(160)),
        critical_warning: Some(log[0]),
    })
}

/// One Device Statistics page, checked to be the page asked for. A bridge or
/// driver that doesn't really pass the command through tends to hand back
/// zeros, and this is what stops those reading as a drive with no hours.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn ata_stats_page(page: &[u8], number: u8) -> Option<&[u8]> {
    // Header qword: revision in bits 15:0, page number in bits 23:16.
    let revision = u16::from_le_bytes(page.get(0..2)?.try_into().ok()?);
    (page.len() >= LOG_LEN && revision != 0 && page[2] == number).then_some(page)
}

/// One statistic: a qword whose top bits say whether it's supported and valid,
/// over a value in the low 48 bits.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn ata_stat(page: &[u8], at: usize) -> Option<u64> {
    let q = u64::from_le_bytes(page.get(at..at + 8)?.try_into().ok()?);
    let supported = q >> 63 & 1 == 1;
    let valid = q >> 62 & 1 == 1;
    (supported && valid).then_some(q & 0x0000_FFFF_FFFF_FFFF)
}

/// The model string from an ATA IDENTIFY DEVICE response: words 27–46, each
/// word's two bytes swapped, space-padded.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn ata_identify_model(identify: &[u8]) -> Option<String> {
    let raw = identify.get(54..94)?;
    let bytes: Vec<u8> = raw
        .as_chunks::<2>()
        .0
        .iter()
        .flat_map(|&[a, b]| [b, a])
        .collect();
    let model = String::from_utf8_lossy(&bytes).trim().to_string();
    // An all-zero or all-space response is a bridge that didn't pass the
    // command through, not a drive with no name.
    (model.chars().any(|c| c.is_ascii_alphanumeric())).then_some(model)
}

/// Decode the General Statistics page and, if the drive has one, the Solid
/// State page. `None` unless the drive reports its power-on hours.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn parse_ata_stats(
    device: String,
    model: Option<String>,
    general: &[u8],
    solid_state: Option<&[u8]>,
) -> Option<DriveHealth> {
    let general = ata_stats_page(general, ATA_STATS_GENERAL)?;
    let power_on_hours = ata_stat(general, 0x10)?;
    // Logical sectors, taken as 512 bytes: what nearly every SATA drive
    // reports, 4Kn ones aside.
    let written = ata_stat(general, 0x18).map(|s| terabytes(s as f64 * 512.0));
    let wear = solid_state
        .and_then(|p| ata_stats_page(p, ATA_STATS_SOLID_STATE))
        .and_then(|p| ata_stat(p, 0x08))
        .map(|v| v.min(255) as u8);
    Some(DriveHealth {
        device,
        interface: "sata".to_string(),
        model,
        power_on_hours,
        power_cycles: ata_stat(general, 0x08),
        unsafe_shutdowns: None,
        percentage_used: wear,
        available_spare_pct: None,
        data_written_tb: written,
        media_errors: None,
        critical_warning: None,
    })
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn platform_collect() -> Vec<DriveHealth> {
    let mut drives = linux::nvme();
    drives.extend(linux::sata());
    drives
}

#[cfg(target_os = "linux")]
mod linux {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    use super::DriveHealth;

    /// Block or controller names under a sysfs class directory, in natural
    /// order (`nvme2` before `nvme10`).
    fn names(dir: &str, prefix: &str) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(prefix))
            .collect();
        names.sort_by_key(|n| (n.len(), n.clone()));
        names
    }

    fn sysfs(path: String) -> Option<String> {
        std::fs::read_to_string(path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// The device node, opened the way a pass-through wants: read-only and
    /// non-blocking. Root-only by default.
    fn open(name: &str) -> Option<std::fs::File> {
        match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(format!("/dev/{name}"))
        {
            Ok(f) => Some(f),
            Err(e) => {
                log::debug!(target: "loadbearer::drives", "open /dev/{name}: {e}");
                None
            }
        }
    }

    pub fn nvme() -> Vec<DriveHealth> {
        names("/sys/class/nvme", "nvme")
            .into_iter()
            .filter_map(|name| {
                let model = sysfs(format!("/sys/class/nvme/{name}/model"));
                let log = nvme_health_log(&name)?;
                super::parse_nvme_health(name, model, &log)
            })
            .collect()
    }

    pub fn sata() -> Vec<DriveHealth> {
        names("/sys/block", "sd")
            .into_iter()
            // A card reader or USB stick: no statistics, and asking can
            // upset a cheap bridge.
            .filter(|name| sysfs(format!("/sys/block/{name}/removable")).as_deref() != Some("1"))
            .filter_map(|name| {
                let dev = open(&name)?;
                // The drive's own name for itself. sysfs only has the SCSI
                // INQUIRY product field, which is cut at 16 characters —
                // "SanDisk SD8TN8U2" for an SD8TN8U256G1001.
                let model = ata_identify(&dev, &name)
                    .and_then(|id| super::ata_identify_model(&id))
                    .or_else(|| sysfs(format!("/sys/block/{name}/device/model")));
                let general = ata_read_log(&dev, &name, super::ATA_STATS_GENERAL)?;
                let solid_state = ata_read_log(&dev, &name, super::ATA_STATS_SOLID_STATE);
                let parsed =
                    super::parse_ata_stats(name.clone(), model, &general, solid_state.as_deref());
                if parsed.is_none() {
                    log::debug!(target: "loadbearer::drives", "{name}: no device statistics");
                }
                parsed
            })
            .collect()
    }

    /// `struct nvme_admin_cmd` from `<linux/nvme_ioctl.h>`.
    #[repr(C)]
    #[derive(Default)]
    struct NvmeAdminCmd {
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

    fn nvme_health_log(name: &str) -> Option<Vec<u8>> {
        let dev = open(name)?;
        let mut buf = vec![0u8; super::LOG_LEN];
        let numd = (super::LOG_LEN / 4 - 1) as u32;
        let mut cmd = NvmeAdminCmd {
            opcode: 0x02, // Get Log Page
            nsid: 0xFFFF_FFFF,
            addr: buf.as_mut_ptr() as u64,
            data_len: super::LOG_LEN as u32,
            cdw10: (numd << 16) | super::NVME_LOG_HEALTH,
            ..Default::default()
        };
        // SAFETY: `cmd` matches the kernel's layout, and `addr` points at
        // `buf`, which is `data_len` bytes and outlives the call.
        let rc = unsafe { libc::ioctl(dev.as_raw_fd(), NVME_IOCTL_ADMIN_CMD as _, &mut cmd) };
        if rc != 0 {
            log::debug!(
                target: "loadbearer::drives",
                "Get Log Page on /dev/{name} failed: rc={rc} {}",
                std::io::Error::last_os_error(),
            );
            return None;
        }
        Some(buf)
    }

    /// `struct sg_io_hdr` from `<scsi/sg.h>`.
    #[repr(C)]
    struct SgIoHdr {
        interface_id: i32,
        dxfer_direction: i32,
        cmd_len: u8,
        mx_sb_len: u8,
        iovec_count: u16,
        dxfer_len: u32,
        dxferp: *mut libc::c_void,
        cmdp: *const u8,
        sbp: *mut u8,
        timeout: u32,
        flags: u32,
        pack_id: i32,
        usr_ptr: *mut libc::c_void,
        status: u8,
        masked_status: u8,
        msg_status: u8,
        sb_len_wr: u8,
        host_status: u16,
        driver_status: u16,
        resid: i32,
        duration: u32,
        info: u32,
    }

    const SG_IO: u32 = 0x2285;
    const SG_DXFER_FROM_DEV: i32 = -3;

    /// READ LOG EXT for one page of the Device Statistics log.
    fn ata_read_log(dev: &std::fs::File, name: &str, page: u8) -> Option<Vec<u8>> {
        let cdb: [u8; 16] = [
            0x85,                             // ATA PASS-THROUGH (16)
            (4 << 1) | 1,                     // PIO data-in, 48-bit command
            0x0E,                             // to host, in blocks, length in COUNT
            0,                                // FEATURES 15:8
            0,                                // FEATURES 7:0
            0,                                // COUNT 15:8
            1,                                // COUNT 7:0: one page
            0,                                // LBA 31:24
            super::ATA_LOG_DEVICE_STATISTICS, // LBA 7:0: log address
            0,                                // LBA 39:32
            page,                             // LBA 15:8: page number
            0,                                // LBA 47:40
            0,                                // LBA 23:16
            0,                                // DEVICE
            0x2F,                             // READ LOG EXT
            0,                                // CONTROL
        ];
        ata_pio_in(
            dev,
            &format!("READ LOG EXT 04h page {page} on /dev/{name}"),
            &cdb,
        )
    }

    /// IDENTIFY DEVICE: 512 bytes describing the drive, its model among them.
    fn ata_identify(dev: &std::fs::File, name: &str) -> Option<Vec<u8>> {
        let cdb: [u8; 16] = [
            0x85,   // ATA PASS-THROUGH (16)
            4 << 1, // PIO data-in, 28-bit command
            0x0E,   // to host, in blocks, length in COUNT
            0,      // FEATURES 15:8
            0,      // FEATURES 7:0
            0,      // COUNT 15:8
            1,      // COUNT 7:0: one block
            0,
            0,
            0,
            0,
            0,
            0,    // LBA
            0,    // DEVICE
            0xEC, // IDENTIFY DEVICE
            0,    // CONTROL
        ];
        ata_pio_in(dev, &format!("IDENTIFY DEVICE on /dev/{name}"), &cdb)
    }

    /// Send one 512-byte PIO data-in ATA command through the kernel's SCSI
    /// layer as an ATA PASS-THROUGH (16) — the same route `smartctl -d sat`
    /// takes. `what` names the command for the log.
    fn ata_pio_in(dev: &std::fs::File, what: &str, cdb: &[u8; 16]) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; super::LOG_LEN];
        let mut sense = [0u8; 32];
        let mut hdr = SgIoHdr {
            interface_id: b'S' as i32,
            dxfer_direction: SG_DXFER_FROM_DEV,
            cmd_len: cdb.len() as u8,
            mx_sb_len: sense.len() as u8,
            iovec_count: 0,
            dxfer_len: buf.len() as u32,
            dxferp: buf.as_mut_ptr().cast(),
            cmdp: cdb.as_ptr(),
            sbp: sense.as_mut_ptr(),
            timeout: 5_000,
            flags: 0,
            pack_id: 0,
            usr_ptr: std::ptr::null_mut(),
            status: 0,
            masked_status: 0,
            msg_status: 0,
            sb_len_wr: 0,
            host_status: 0,
            driver_status: 0,
            resid: 0,
            duration: 0,
            info: 0,
        };
        // SAFETY: `hdr` matches the kernel's layout; the command, data and
        // sense pointers address live buffers of the lengths given.
        let rc = unsafe { libc::ioctl(dev.as_raw_fd(), SG_IO as _, &mut hdr) };
        // SG_INFO_OK_MASK: bit 0 set means the command didn't complete
        // cleanly (a SCSI status, a host or driver error).
        if rc != 0 || hdr.info & 1 != 0 || hdr.status != 0 || hdr.host_status != 0 {
            log::debug!(
                target: "loadbearer::drives",
                "{what} failed: rc={rc} status={} host={} driver={} {}",
                hdr.status,
                hdr.host_status,
                hdr.driver_status,
                if rc != 0 {
                    std::io::Error::last_os_error().to_string()
                } else {
                    String::new()
                },
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
fn platform_collect() -> Vec<DriveHealth> {
    // Drive numbers can have gaps (a removed USB disk), so try a fixed range
    // rather than stopping at the first miss.
    (0..16)
        .filter_map(|i| {
            let name = format!("PhysicalDrive{i}");
            let drive = win::Drive::open(&name)?;
            let (bus, model) = drive.describe()?;
            match bus {
                win::Bus::Nvme => {
                    let log = drive.nvme_health_log()?;
                    parse_nvme_health(name, model, &log)
                }
                win::Bus::Ata => {
                    let general = drive.ata_log_page(ATA_STATS_GENERAL)?;
                    let solid_state = drive.ata_log_page(ATA_STATS_SOLID_STATE);
                    parse_ata_stats(name, model, &general, solid_state.as_deref())
                }
            }
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
    // STORAGE_BUS_TYPE values.
    const BUS_TYPE_ATA: u32 = 3;
    const BUS_TYPE_SATA: u32 = 11;
    const BUS_TYPE_NVME: u32 = 17;
    // STORAGE_PROTOCOL_TYPE and the data types under each.
    const PROTOCOL_TYPE_ATA: u32 = 2;
    const PROTOCOL_TYPE_NVME: u32 = 3;
    const ATA_DATA_TYPE_LOG_PAGE: u32 = 2;
    const NVME_DATA_TYPE_LOG_PAGE: u32 = 2;
    /// `sizeof(STORAGE_PROTOCOL_SPECIFIC_DATA)`: ten `u32`s.
    const PROTOCOL_DATA_LEN: usize = 40;

    pub enum Bus {
        Nvme,
        Ata,
    }

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
                    target: "loadbearer::drives",
                    "IOCTL_STORAGE_QUERY_PROPERTY failed: {}",
                    std::io::Error::last_os_error(),
                );
                return None;
            }
            out.truncate(got as usize);
            Some(out)
        }

        /// The bus, if it's one whose health this can read, and the model.
        pub fn describe(&self) -> Option<(Bus, Option<String>)> {
            // STORAGE_PROPERTY_QUERY { PropertyId, QueryType = standard, … }.
            let mut q = [0u8; 12];
            q[0..4].copy_from_slice(&STORAGE_DEVICE_PROPERTY.to_le_bytes());
            let d = self.query(&q, 1024)?;
            let u32_at = |at: usize| {
                d.get(at..at + 4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            };
            // STORAGE_DEVICE_DESCRIPTOR: BusType at 28, ProductIdOffset at 16.
            let bus = match u32_at(28)? {
                BUS_TYPE_NVME => Bus::Nvme,
                BUS_TYPE_ATA | BUS_TYPE_SATA => Bus::Ata,
                _ => return None,
            };
            let model = u32_at(16)
                .map(|o| o as usize)
                .filter(|&o| o != 0 && o < d.len())
                .and_then(|o| {
                    let s = &d[o..];
                    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
                    let m = String::from_utf8_lossy(&s[..end]).trim().to_string();
                    (!m.is_empty()).then_some(m)
                });
            Some((bus, model))
        }

        /// A protocol-specific log page: STORAGE_PROPERTY_QUERY header (8
        /// bytes), then a STORAGE_PROTOCOL_SPECIFIC_DATA in
        /// AdditionalParameters, then room for the page itself.
        fn log_page(&self, protocol: u32, data_type: u32, value: u32, sub: u32) -> Option<Vec<u8>> {
            let mut q = vec![0u8; 8 + PROTOCOL_DATA_LEN + super::LOG_LEN];
            let mut put = |at: usize, v: u32| q[at..at + 4].copy_from_slice(&v.to_le_bytes());
            put(0, STORAGE_DEVICE_PROTOCOL_SPECIFIC_PROPERTY);
            put(8, protocol);
            put(12, data_type);
            put(16, value); // ProtocolDataRequestValue
            put(20, sub); // ProtocolDataRequestSubValue
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

        pub fn nvme_health_log(&self) -> Option<Vec<u8>> {
            self.log_page(
                PROTOCOL_TYPE_NVME,
                NVME_DATA_TYPE_LOG_PAGE,
                super::NVME_LOG_HEALTH,
                0,
            )
        }

        /// One page of the ATA Device Statistics log: the log address goes in
        /// the request value and the page number in the sub-value.
        pub fn ata_log_page(&self, page: u8) -> Option<Vec<u8>> {
            self.log_page(
                PROTOCOL_TYPE_ATA,
                ATA_DATA_TYPE_LOG_PAGE,
                super::ATA_LOG_DEVICE_STATISTICS as u32,
                page as u32,
            )
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
fn platform_collect() -> Vec<DriveHealth> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_an_nvme_health_log_page() {
        let mut log = [0u8; 512];
        log[0] = 0x04; // reliability degraded
        log[3] = 100; // available spare
        log[5] = 7; // percentage used
        log[48..56].copy_from_slice(&20_000_000u64.to_le_bytes()); // data units written
        log[112..120].copy_from_slice(&1_234u64.to_le_bytes());
        log[128..136].copy_from_slice(&11_800u64.to_le_bytes());
        log[144..152].copy_from_slice(&56u64.to_le_bytes());
        log[160..168].copy_from_slice(&2u64.to_le_bytes());

        let h = parse_nvme_health("nvme0".into(), Some("ACME SSD".into()), &log).unwrap();
        assert_eq!(h.interface, "nvme");
        assert_eq!(h.power_on_hours, 11_800);
        assert_eq!(h.power_cycles, Some(1_234));
        assert_eq!(h.unsafe_shutdowns, Some(56));
        assert_eq!(h.media_errors, Some(2));
        assert_eq!(h.percentage_used, Some(7));
        assert_eq!(h.available_spare_pct, Some(100));
        assert_eq!(h.critical_warning, Some(0x04));
        assert_eq!(h.data_written_tb, Some(10.24));
    }

    #[test]
    fn a_short_nvme_page_is_rejected() {
        assert_eq!(parse_nvme_health("nvme0".into(), None, &[0u8; 100]), None);
    }

    /// A Device Statistics page with the given statistics, each flagged
    /// supported and valid.
    fn ata_page(number: u8, stats: &[(usize, u64)]) -> [u8; 512] {
        let mut p = [0u8; 512];
        p[0] = 1; // revision
        p[2] = number;
        for &(at, v) in stats {
            let q = (0b11u64 << 62) | v;
            p[at..at + 8].copy_from_slice(&q.to_le_bytes());
        }
        p
    }

    #[test]
    fn decodes_ata_device_statistics() {
        let general = ata_page(1, &[(0x08, 1_502), (0x10, 21_344), (0x18, 39_062_500_000)]);
        let ssd = ata_page(7, &[(0x08, 12)]);
        let h =
            parse_ata_stats("sda".into(), Some("SanDisk".into()), &general, Some(&ssd)).unwrap();
        assert_eq!(h.interface, "sata");
        assert_eq!(h.power_on_hours, 21_344);
        assert_eq!(h.power_cycles, Some(1_502));
        assert_eq!(h.data_written_tb, Some(20.0));
        assert_eq!(h.percentage_used, Some(12));
        // What only NVMe reports stays empty rather than zero.
        assert_eq!(h.unsafe_shutdowns, None);
        assert_eq!(h.available_spare_pct, None);
        assert_eq!(h.critical_warning, None);
    }

    #[test]
    fn the_identify_model_is_byte_swapped_and_trimmed() {
        let mut id = [0u8; 512];
        // "SanDisk SD8TN8U256G1001" as ATA stores it: each pair swapped,
        // padded with spaces to 40 characters.
        let mut model = *b"SanDisk SD8TN8U256G1001                 ";
        for pair in model.as_chunks_mut::<2>().0 {
            pair.swap(0, 1);
        }
        id[54..94].copy_from_slice(&model);
        assert_eq!(
            ata_identify_model(&id).as_deref(),
            Some("SanDisk SD8TN8U256G1001")
        );
        assert_eq!(ata_identify_model(&[0u8; 512]), None);
        assert_eq!(ata_identify_model(&[b' '; 512]), None);
        assert_eq!(ata_identify_model(&[0u8; 60]), None);
    }

    #[test]
    fn a_hard_disk_has_hours_and_no_wear() {
        let general = ata_page(1, &[(0x10, 40_000)]);
        let h = parse_ata_stats("sda".into(), None, &general, None).unwrap();
        assert_eq!(h.power_on_hours, 40_000);
        assert_eq!(h.percentage_used, None);
        assert_eq!(h.data_written_tb, None);
    }

    #[test]
    fn an_unflagged_statistic_is_not_read_as_zero() {
        // Hours present but not marked valid: no reading, so no drive.
        let mut general = ata_page(1, &[]);
        general[0x10..0x18].copy_from_slice(&(1u64 << 63 | 500).to_le_bytes());
        assert_eq!(parse_ata_stats("sda".into(), None, &general, None), None);
    }

    #[test]
    fn a_page_of_zeros_or_the_wrong_page_is_rejected() {
        // What a bridge that doesn't pass the command through tends to return.
        assert_eq!(parse_ata_stats("sda".into(), None, &[0u8; 512], None), None);
        let wrong = ata_page(3, &[(0x10, 100)]);
        assert_eq!(parse_ata_stats("sda".into(), None, &wrong, None), None);
        // A Solid State page that isn't one just leaves wear empty.
        let general = ata_page(1, &[(0x10, 100)]);
        let h = parse_ata_stats("sda".into(), None, &general, Some(&[0u8; 512])).unwrap();
        assert_eq!(h.percentage_used, None);
    }
}
