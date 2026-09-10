//! Stable machine identity: the identifiers that let a collector tell "this is
//! the same machine as last month" from "this is one I haven't seen before".
//!
//! `hostname` can't do that job in a managed fleet — it's renameable, sometimes
//! unset, and reissued between machines. Each identifier here fails
//! differently, so all of them are collected and the consumer picks an order:
//!
//! - `smbios_uuid` and `serial` come from firmware, so they survive a reimage
//!   and are what asset, warranty and lease records key on. On Linux they need
//!   root, and cheap hardware often ships placeholder junk instead of a serial.
//! - `machine_id` is written once by the OS install: always readable, but it
//!   resets when the machine is reimaged and is cloned by a careless VM
//!   template.
//!
//! Everything is best-effort. A machine that reports nothing at all — a
//! container, a locked-down VM, a non-root Linux run — is normal, not an error,
//! so every field is `Option` and the whole block is omitted when empty.

use serde::{Deserialize, Serialize};

/// Firmware and OS identifiers for one machine. Fields are independent; expect
/// some to be absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineId {
    /// The OS install's own identity. Windows: `MachineGuid` under
    /// `HKLM\SOFTWARE\Microsoft\Cryptography`. Linux: `/etc/machine-id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<String>,
    /// SMBIOS system UUID (type 1), formatted the way both Windows and the
    /// Linux kernel present it, so the two agree for the same machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smbios_uuid: Option<String>,
    /// Chassis/system serial number (SMBIOS type 1) — the number on the sticker
    /// and in the purchase record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// Asset tag (SMBIOS type 3), when whoever provisioned the machine set one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_tag: Option<String>,
}

impl MachineId {
    /// True when nothing at all could be read, which is when the block is left
    /// out of the result file entirely.
    pub fn is_empty(&self) -> bool {
        *self == MachineId::default()
    }
}

/// Collect what this platform will tell us, or `None` if that's nothing.
pub fn collect() -> Option<MachineId> {
    let id = platform_collect();
    // Whether each field was readable, never the value. These are the most
    // identifying things loadbearer touches, PRIVACY.md promises the
    // diagnostic log holds no personal data, and a log is the artefact people
    // paste into a bug report — the result file is the one they choose to
    // share. Presence is also what actually diagnoses a collector problem:
    // "the serial is wrong" is a `meaningful` question, and that path logs
    // the placeholder it rejected, which is a fixed OEM string.
    log::debug!(
        target: "loadbearer::identity",
        "machine_id={} smbios_uuid={} serial={} asset_tag={}",
        readable(&id.machine_id),
        readable(&id.smbios_uuid),
        readable(&id.serial),
        readable(&id.asset_tag),
    );
    if id.is_empty() { None } else { Some(id) }
}

/// Whether a field was readable, for the diagnostic log — see `collect`.
fn readable(field: &Option<String>) -> &'static str {
    if field.is_some() { "present" } else { "absent" }
}

/// Reject the placeholder strings OEMs ship in place of a real serial. Without
/// this a fleet quietly collapses every unconfigured machine of a given model
/// into one identity, because they all report the same "serial".
///
/// Only the Windows and Linux collectors call this, so it is dead code on a
/// platform with neither — macOS builds from source, which the README covers.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn meaningful(s: &str) -> Option<String> {
    const JUNK: &[&str] = &[
        "to be filled by o.e.m.",
        "to be filled by oem",
        "default string",
        "system serial number",
        "system uuid",
        "chassis serial number",
        "base board serial number",
        "not specified",
        "not applicable",
        "not available",
        "no asset tag",
        "asset tag",
        "unknown",
        "none",
        "n/a",
        "o.e.m.",
        "oem",
        "invalid",
        "filled by oem",
    ];

    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let lower = t.to_ascii_lowercase();
    if JUNK.contains(&lower.as_str()) {
        // Safe to log this one: it matched a fixed list of OEM placeholders, so
        // it describes the firmware and not the machine. It's also the
        // diagnostic that matters — "the firmware didn't report a serial" and
        // "it reported something useless" look identical in the result file
        // and want different answers.
        log::debug!(target: "loadbearer::identity", "rejected placeholder identifier {t:?}");
        return None;
    }
    // "0000000", "xxxxxxxx", "........" and friends: technically a string, but
    // not an identity.
    let mut chars = t.chars();
    let first = chars.next()?;
    if t.chars().count() > 1 && chars.all(|c| c == first) {
        // One repeated character carries no identity, so this is safe too.
        log::debug!(target: "loadbearer::identity", "rejected filler identifier {t:?}");
        return None;
    }
    Some(t.to_string())
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn platform_collect() -> MachineId {
    let (smbios_uuid, serial, asset_tag) = win::smbios().unwrap_or_default();
    MachineId {
        machine_id: win::machine_guid(),
        smbios_uuid,
        serial,
        asset_tag,
    }
}

#[cfg(windows)]
mod win {
    use super::meaningful;

    /// Read `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`. Present since
    /// Windows 7 and readable by any user.
    pub fn machine_guid() -> Option<String> {
        use std::ffi::c_void;
        use windows_sys::Win32::System::Registry::{
            HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
        };

        let subkey: Vec<u16> = "SOFTWARE\\Microsoft\\Cryptography\0"
            .encode_utf16()
            .collect();
        let value: Vec<u16> = "MachineGuid\0".encode_utf16().collect();

        // 39 UTF-16 units holds a braceless GUID and its NUL with room spare.
        let mut buf = [0u16; 64];
        let mut size = std::mem::size_of_val(&buf) as u32;

        // SAFETY: both name pointers are NUL-terminated UTF-16 that outlive the
        // call, and `size` is the byte length of `buf`, which the call clamps
        // its write to.
        let rc = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_void,
                &mut size,
            )
        };
        if rc != 0 {
            log::debug!(target: "loadbearer::identity", "RegGetValueW(MachineGuid) failed: {rc}");
            return None;
        }

        let units = (size as usize / 2).min(buf.len());
        let s: String = String::from_utf16_lossy(&buf[..units]);
        meaningful(s.trim_end_matches('\0'))
    }

    /// Pull the raw SMBIOS table from firmware and read what we need out of it.
    /// Returns `(uuid, serial, asset_tag)`.
    ///
    /// `GetSystemFirmwareTable` rather than WMI on purpose: no COM to
    /// initialise, no `wmic` subprocess to spawn (it's been removed from
    /// current Windows anyway), and it works in a Session 0 service context,
    /// which is where a deployment tool runs us.
    pub fn smbios() -> Option<(Option<String>, Option<String>, Option<String>)> {
        use windows_sys::Win32::System::SystemInformation::GetSystemFirmwareTable;

        // 'RSMB', the raw-SMBIOS provider, big-endian in the API's terms.
        const RSMB: u32 = u32::from_be_bytes(*b"RSMB");

        // SAFETY: a null buffer with zero length is the documented way to ask
        // for the size it needs.
        let needed = unsafe { GetSystemFirmwareTable(RSMB, 0, std::ptr::null_mut(), 0) };
        if needed == 0 {
            log::debug!(target: "loadbearer::identity", "GetSystemFirmwareTable(RSMB) reported no table");
            return None;
        }

        let mut buf = vec![0u8; needed as usize];
        // SAFETY: `buf` is `needed` bytes and stays alive for the call.
        let got = unsafe { GetSystemFirmwareTable(RSMB, 0, buf.as_mut_ptr() as *mut _, needed) };
        if got == 0 || got > needed {
            log::debug!(target: "loadbearer::identity", "GetSystemFirmwareTable(RSMB) returned {got} for a {needed}-byte buffer");
            return None;
        }
        buf.truncate(got as usize);

        // RawSMBIOSData: u8 calling method, u8 major, u8 minor, u8 revision,
        // u32 length, then the table itself.
        if buf.len() <= 8 {
            return None;
        }
        Some(super::parse_smbios(&buf[8..]))
    }
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn platform_collect() -> MachineId {
    // product_uuid and product_serial are 0400 root-only; the asset tag
    // usually isn't. Non-root runs simply get fewer fields.
    let dmi = |name: &str| {
        std::fs::read_to_string(format!("/sys/class/dmi/id/{name}"))
            .ok()
            .and_then(|s| meaningful(&s))
    };
    MachineId {
        machine_id: std::fs::read_to_string("/etc/machine-id")
            .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
            .ok()
            .and_then(|s| meaningful(&s)),
        smbios_uuid: dmi("product_uuid").map(|s| s.to_ascii_lowercase()),
        serial: dmi("product_serial"),
        asset_tag: dmi("chassis_asset_tag"),
    }
}

// ---------------------------------------------------------------------------
// Anywhere else
// ---------------------------------------------------------------------------

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_collect() -> MachineId {
    MachineId::default()
}

// ---------------------------------------------------------------------------
// SMBIOS table parsing (used by the Windows path; tested everywhere)
// ---------------------------------------------------------------------------

/// Walk an SMBIOS structure table and pull the system UUID and serial (type 1)
/// and the chassis asset tag (type 3). Returns `(uuid, serial, asset_tag)`.
///
/// Each structure is a 4-byte header (type, length, 2-byte handle), a formatted
/// area of `length` bytes total, then a NUL-separated string table terminated
/// by an empty string. String-valued fields hold a 1-based index into it.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_smbios(table: &[u8]) -> (Option<String>, Option<String>, Option<String>) {
    let mut uuid = None;
    let mut serial = None;
    let mut asset_tag = None;

    let mut pos = 0usize;
    while pos + 4 <= table.len() {
        let kind = table[pos];
        let len = table[pos + 1] as usize;
        // A length inside the header, or past the end, means the table is
        // malformed; stop rather than guess.
        if len < 4 || pos + len > table.len() {
            break;
        }
        let formatted = &table[pos..pos + len];

        // Strings follow the formatted area, ending at a double NUL.
        let strings_at = pos + len;
        let mut end = strings_at;
        while end + 1 < table.len() && !(table[end] == 0 && table[end + 1] == 0) {
            end += 1;
        }
        let strings = &table[strings_at..end.min(table.len())];

        match kind {
            // Type 1, System Information: serial at 0x07, UUID at 0x08.
            1 => {
                if len > 0x07 {
                    serial = smbios_string(strings, formatted[0x07]);
                }
                if len >= 0x18 {
                    uuid = format_smbios_uuid(&formatted[0x08..0x18]);
                }
            }
            // Type 3, System Enclosure: asset tag at 0x08.
            3 => {
                if len > 0x08 {
                    asset_tag = smbios_string(strings, formatted[0x08]);
                }
            }
            // Type 127 is the end-of-table marker.
            127 => break,
            _ => {}
        }

        pos = end + 2;
    }

    (uuid, serial, asset_tag)
}

/// Resolve a 1-based SMBIOS string index against a structure's string table.
#[cfg_attr(not(windows), allow(dead_code))]
fn smbios_string(strings: &[u8], index: u8) -> Option<String> {
    if index == 0 {
        return None;
    }
    strings
        .split(|b| *b == 0)
        .nth(index as usize - 1)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .as_deref()
        .and_then(meaningful)
}

/// Format the 16 raw UUID bytes from SMBIOS type 1.
///
/// The first three fields are little-endian here — RFC 4122 says network order,
/// but the PC industry settled on the other one, and the Linux kernel already
/// swaps them when it exposes `product_uuid`. Swapping to match means the same
/// machine reports the same UUID whichever OS it booted.
#[cfg_attr(not(windows), allow(dead_code))]
fn format_smbios_uuid(b: &[u8]) -> Option<String> {
    if b.len() != 16 {
        return None;
    }
    // All-zero or all-0xFF means "not set" rather than an identity.
    if b.iter().all(|x| *x == 0) || b.iter().all(|x| *x == 0xFF) {
        return None;
    }
    Some(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-\
         {:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[3],
        b[2],
        b[1],
        b[0],
        b[5],
        b[4],
        b[7],
        b[6],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oem_placeholders_are_not_identities() {
        for s in [
            "",
            "  ",
            "To Be Filled By O.E.M.",
            "Default string",
            "System Serial Number",
            "none",
            "N/A",
            "0000000000",
            "xxxxxxxx",
        ] {
            assert_eq!(meaningful(s), None, "{s:?} should be rejected");
        }
    }

    #[test]
    fn a_real_serial_survives_trimming() {
        assert_eq!(meaningful("  D47TDY3 \n"), Some("D47TDY3".to_string()));
        // A single character is thin, but it isn't a repeated-filler string.
        assert_eq!(meaningful("7"), Some("7".to_string()));
    }

    #[test]
    fn uuid_first_three_fields_are_byte_swapped() {
        let bytes: Vec<u8> = (0..16).collect();
        assert_eq!(
            format_smbios_uuid(&bytes).unwrap(),
            "03020100-0504-0706-0809-0a0b0c0d0e0f"
        );
    }

    #[test]
    fn an_unset_uuid_is_none() {
        assert_eq!(format_smbios_uuid(&[0u8; 16]), None);
        assert_eq!(format_smbios_uuid(&[0xFFu8; 16]), None);
        assert_eq!(format_smbios_uuid(&[0u8; 15]), None);
    }

    /// A minimal two-structure table: type 1 with a serial and UUID, then type
    /// 3 with an asset tag, then the end marker.
    fn sample_table() -> Vec<u8> {
        let mut t = Vec::new();

        // --- type 1, length 0x1B ---
        t.extend_from_slice(&[1, 0x1B, 0x01, 0x00]); // type, len, handle
        t.extend_from_slice(&[1, 2, 3, 4]); // manufacturer, product, version, serial
        t.extend_from_slice(&(0..16).collect::<Vec<u8>>()); // UUID
        t.extend_from_slice(&[6, 0, 0]); // wake-up type, sku, family
        for s in ["ACME", "Widget", "1.0", "SN-12345"] {
            t.extend_from_slice(s.as_bytes());
            t.push(0);
        }
        t.push(0); // end of this structure's strings

        // --- type 3, length 0x15 ---
        t.extend_from_slice(&[3, 0x15, 0x02, 0x00]);
        t.extend_from_slice(&[1, 0x03, 0, 1, 2]); // mfr, type, version, serial, asset tag
        t.extend_from_slice(&[0; 12]); // remainder of the formatted area
        for s in ["ACME", "ASSET-99"] {
            t.extend_from_slice(s.as_bytes());
            t.push(0);
        }
        t.push(0);

        // --- type 127, end of table ---
        t.extend_from_slice(&[127, 4, 0x03, 0x00, 0, 0]);
        t
    }

    #[test]
    fn parses_serial_uuid_and_asset_tag_from_a_table() {
        let (uuid, serial, asset) = parse_smbios(&sample_table());
        assert_eq!(serial, Some("SN-12345".to_string()));
        assert_eq!(
            uuid,
            Some("03020100-0504-0706-0809-0a0b0c0d0e0f".to_string())
        );
        assert_eq!(asset, Some("ASSET-99".to_string()));
    }

    #[test]
    fn a_truncated_table_yields_what_it_can_without_panicking() {
        let full = sample_table();
        for cut in 0..full.len() {
            let _ = parse_smbios(&full[..cut]);
        }
    }

    #[test]
    fn an_empty_id_is_omitted_rather_than_reported() {
        assert!(MachineId::default().is_empty());
        assert!(
            !MachineId {
                serial: Some("SN-1".into()),
                ..Default::default()
            }
            .is_empty()
        );
    }
}
