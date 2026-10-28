//! What the machine says it is, and the firmware it's running: the system
//! vendor and model (SMBIOS type 1) and the BIOS version and release date
//! (type 0).
//!
//! Unlike the identifiers in [`crate::identity`], none of this is per-machine
//! or root-only: every unit of a model reports the same strings, and Linux
//! leaves these `/sys/class/dmi/id/` files world-readable. The BIOS date is
//! one of the inputs [`crate::age`] uses to place the machine in time.
//!
//! Best-effort like the rest of the inventory: a VM or container may report
//! nothing, or placeholder strings, which are dropped.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Firmware {
    /// System manufacturer (type 1), e.g. "Dell Inc.", "LENOVO".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_vendor: Option<String>,
    /// Product name (type 1), e.g. "Latitude 7440". Lenovo puts the
    /// machine-type code here and the marketing name in `system_version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_model: Option<String>,
    /// Product version (type 1). Usually empty or a placeholder; the model's
    /// marketing name on Lenovo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_version: Option<String>,
    /// BIOS / UEFI firmware version (type 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bios_version: Option<String>,
    /// BIOS release date (type 0) as ISO `YYYY-MM-DD`. The date of the
    /// firmware *build*, which a firmware update moves forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bios_date: Option<String>,
}

impl Firmware {
    pub fn is_empty(&self) -> bool {
        *self == Firmware::default()
    }

    /// "Dell Inc. Latitude 7440", or "LENOVO 20XW004BUK (ThinkPad X1 Carbon
    /// Gen 9)" where the version field carries the name people know it by.
    pub fn system_name(&self) -> Option<String> {
        let mut name = [self.system_vendor.as_deref(), self.system_model.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        // Only Lenovo uses the version field for anything a person would
        // recognise; elsewhere it's "1.0", a board revision or an OEM config
        // code, which would read as part of the model name.
        let lenovo = self
            .system_vendor
            .as_deref()
            .is_some_and(|v| v.eq_ignore_ascii_case("lenovo"));
        if let (true, Some(v)) = (lenovo, &self.system_version) {
            if name.is_empty() {
                name = v.clone();
            } else {
                name = format!("{name} ({v})");
            }
        }
        (!name.is_empty()).then_some(name)
    }

    /// "1.15.0, dated 2023-04-12", or whichever half is known.
    pub fn bios_line(&self) -> Option<String> {
        match (&self.bios_version, &self.bios_date) {
            (Some(v), Some(d)) => Some(format!("{v}, dated {d}")),
            (Some(v), None) => Some(v.clone()),
            (None, Some(d)) => Some(format!("dated {d}")),
            (None, None) => None,
        }
    }

    /// `bios_date` as `(year, month)`.
    pub fn bios_year_month(&self) -> Option<(u16, u8)> {
        let d = self.bios_date.as_deref()?;
        let year = d.get(0..4)?.parse().ok()?;
        let month = d.get(5..7)?.parse().ok()?;
        Some((year, month))
    }
}

/// Collect what this platform will tell us, or `None` if that's nothing.
pub fn collect() -> Option<Firmware> {
    let fw = platform_collect();
    // Model and firmware strings are shared by every unit of a model, so
    // unlike the identity block they're safe to log as values.
    log::debug!(
        target: "loadbearer::firmware",
        "system={:?} bios={:?} dated {:?}",
        fw.system_name(),
        fw.bios_version,
        fw.bios_date,
    );
    if fw.is_empty() { None } else { Some(fw) }
}

#[cfg(windows)]
fn platform_collect() -> Firmware {
    crate::identity::raw_smbios_table()
        .map(|t| parse_smbios(&t))
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn platform_collect() -> Firmware {
    use crate::identity::meaningful;
    let dmi = |name: &str| {
        std::fs::read_to_string(format!("/sys/class/dmi/id/{name}"))
            .ok()
            .and_then(|s| meaningful(&s))
    };
    Firmware {
        system_vendor: dmi("sys_vendor"),
        system_model: dmi("product_name"),
        system_version: dmi("product_version"),
        bios_version: dmi("bios_version"),
        bios_date: dmi("bios_date").and_then(|d| iso_date(&d)),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_collect() -> Firmware {
    Firmware::default()
}

/// Read the type 0 and type 1 fields out of an SMBIOS table.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_smbios(table: &[u8]) -> Firmware {
    let mut fw = Firmware::default();
    for s in crate::identity::structures(table) {
        match s.kind {
            // Type 0, BIOS Information: version at 0x05, release date at 0x08.
            0 => {
                fw.bios_version = s.string(0x05);
                fw.bios_date = s.string(0x08).and_then(|d| iso_date(&d));
            }
            // Type 1, System Information: manufacturer, product, version.
            1 => {
                fw.system_vendor = s.string(0x04);
                fw.system_model = s.string(0x05);
                fw.system_version = s.string(0x06);
            }
            _ => {}
        }
    }
    fw
}

/// SMBIOS's `mm/dd/yyyy` (or pre-2.3 `mm/dd/yy`) release date as ISO
/// `YYYY-MM-DD`. Anything else — and some firmware ships junk here — is
/// dropped rather than guessed at.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn iso_date(s: &str) -> Option<String> {
    let mut parts = s.trim().split('/');
    let month: u8 = parts.next()?.parse().ok()?;
    let day: u8 = parts.next()?.parse().ok()?;
    let y = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let year: u16 = match (y.len(), y.parse::<u16>().ok()?) {
        (4, y) => y,
        // Two-digit years predate SMBIOS 2.3; nothing dated 2080+ ships one.
        (2, y) if y >= 80 => 1900 + y,
        (2, y) => 2000 + y,
        _ => return None,
    };
    let plausible = (1..=12).contains(&month) && (1..=31).contains(&day) && year >= 1980;
    plausible.then(|| format!("{year:04}-{month:02}-{day:02}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smbios_dates_become_iso() {
        assert_eq!(iso_date("05/12/2022\n").as_deref(), Some("2022-05-12"));
        assert_eq!(iso_date("1/2/2023").as_deref(), Some("2023-01-02"));
        assert_eq!(iso_date("08/30/99").as_deref(), Some("1999-08-30"));
        assert_eq!(iso_date("08/30/05").as_deref(), Some("2005-08-30"));
        for junk in ["", "2022-05-12", "13/01/2022", "00/00/0000", "01/02/2022/x"] {
            assert_eq!(iso_date(junk), None, "{junk:?} should be rejected");
        }
    }

    /// Type 0 then type 1, then the end marker — the same layout as the
    /// identity parser's fixture.
    fn sample_table(vendor: &str, version: &str) -> Vec<u8> {
        let mut t = Vec::new();

        // --- type 0, length 0x18 ---
        t.extend_from_slice(&[0, 0x18, 0x00, 0x00]);
        t.extend_from_slice(&[1, 2, 0, 0, 3]); // vendor, version, segment, date
        t.extend_from_slice(&[0; 0x18 - 9]);
        for s in ["ACME BIOS", "1.15.0", "04/12/2023"] {
            t.extend_from_slice(s.as_bytes());
            t.push(0);
        }
        t.push(0);

        // --- type 1, length 0x1B ---
        t.extend_from_slice(&[1, 0x1B, 0x01, 0x00]);
        t.extend_from_slice(&[1, 2, 3, 4]); // manufacturer, product, version, serial
        t.extend_from_slice(&[0; 0x1B - 8]);
        for s in [vendor, "20XW004BUK", version, "SN-1"] {
            t.extend_from_slice(s.as_bytes());
            t.push(0);
        }
        t.push(0);

        t.extend_from_slice(&[127, 4, 0x02, 0x00, 0, 0]);
        t
    }

    #[test]
    fn parses_model_and_firmware_from_a_table() {
        let fw = parse_smbios(&sample_table("LENOVO", "ThinkPad X1 Carbon Gen 9"));
        assert_eq!(fw.bios_version.as_deref(), Some("1.15.0"));
        assert_eq!(fw.bios_date.as_deref(), Some("2023-04-12"));
        assert_eq!(fw.bios_year_month(), Some((2023, 4)));
        assert_eq!(
            fw.system_name().as_deref(),
            Some("LENOVO 20XW004BUK (ThinkPad X1 Carbon Gen 9)")
        );
    }

    #[test]
    fn version_field_is_only_shown_for_lenovo() {
        let fw = parse_smbios(&sample_table("Dell Inc.", "1.0"));
        assert_eq!(fw.system_name().as_deref(), Some("Dell Inc. 20XW004BUK"));
    }

    #[test]
    fn placeholder_model_strings_are_dropped() {
        let fw = parse_smbios(&sample_table("System manufacturer", "System Version"));
        assert_eq!(fw.system_vendor, None);
        assert_eq!(fw.system_version, None);
    }

    #[test]
    fn a_truncated_table_yields_what_it_can_without_panicking() {
        let full = sample_table("LENOVO", "ThinkPad");
        for cut in 0..full.len() {
            let _ = parse_smbios(&full[..cut]);
        }
    }
}
