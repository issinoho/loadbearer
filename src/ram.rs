//! How the RAM is built: how many slots, how many are filled, and each
//! module's size, type (DDR4, LPDDR5, …), form factor and speed — from the
//! SMBIOS Memory Device structures (type 17).
//!
//! Like [`crate::firmware`], nothing here is per-unit: every machine of a
//! model with the same memory fitted reports the same values. The module
//! serial number and asset tag sit in the same structure and are deliberately
//! not read; nor are the manufacturer and part number, which nobody asked
//! this block to carry.
//!
//! Windows reads the SMBIOS table every user can, through
//! `GetSystemFirmwareTable`. Linux has no world-readable copy: the raw table
//! (`/sys/firmware/dmi/tables/DMI`) and the per-entry files under
//! `/sys/firmware/dmi/entries/` are root-only, and `/sys/class/dmi/id/` carries
//! no memory fields. So, like `identity.serial` and `drives`, an unprivileged
//! Linux run simply has no `memory` block; as root it reads the raw table and
//! parses it with the same code as Windows. EDAC's per-DIMM files are
//! world-readable but exist only with an EDAC driver, report no speed and no
//! empty slots, and name types their own way, so they aren't used.
//!
//! Firmware fills these structures with junk often enough — VMs, cheap boards
//! — that every value is optional: an empty slot counts toward `slots` and not
//! toward `populated`, and anything reported as unknown, other, zero or a
//! sentinel is absent rather than written down.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    /// Memory-device sockets the firmware describes for system RAM, filled
    /// or not. Soldered memory reports its packages or channels here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slots: Option<u32>,
    /// How many of those hold a module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub populated: Option<u32>,
    /// One entry per populated slot, in table order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<Module>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Module {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// The SMBIOS memory type: "DDR4", "DDR5", "LPDDR5", …
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// "DIMM", "SODIMM", "Row Of Chips" (soldered), …
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form_factor: Option<String>,
    /// The module's rated maximum, in MT/s (SMBIOS's "Speed", which older
    /// specs labelled MHz).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mts: Option<u32>,
    /// What the memory controller actually runs it at, in MT/s. Below
    /// `speed_mts` when the platform or a mixed set holds it back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configured_speed_mts: Option<u32>,
}

impl Memory {
    pub fn is_empty(&self) -> bool {
        *self == Memory::default()
    }

    /// "2 of 2 slots · 2 × 16.0 GiB DDR5 SODIMM, 5600 MT/s (running 5200)",
    /// with identical modules grouped.
    pub fn summary(&self) -> Option<String> {
        let mut parts = Vec::new();
        match (self.populated, self.slots) {
            (Some(p), Some(s)) => parts.push(format!("{p} of {s} slots")),
            (None, Some(s)) => parts.push(format!("{s} slots")),
            (Some(p), None) => parts.push(format!("{p} populated")),
            (None, None) => {}
        }
        let mut groups: Vec<(&Module, usize)> = Vec::new();
        for m in &self.modules {
            match groups.iter_mut().find(|(g, _)| *g == m) {
                Some((_, n)) => *n += 1,
                None => groups.push((m, 1)),
            }
        }
        for (m, n) in groups {
            let desc = m.describe();
            parts.push(match (n, desc.is_empty()) {
                (1, false) => desc,
                (_, false) => format!("{n} × {desc}"),
                (_, true) => continue,
            });
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

impl Module {
    fn describe(&self) -> String {
        let mut words: Vec<String> = Vec::new();
        if let Some(b) = self.size_bytes {
            words.push(crate::output::human_bytes(b));
        }
        words.extend(self.kind.clone());
        words.extend(self.form_factor.clone());
        let mut s = words.join(" ");
        let speed = match (self.speed_mts, self.configured_speed_mts) {
            (Some(r), Some(c)) if c != r => Some(format!("{r} MT/s (running {c})")),
            (Some(r), _) => Some(format!("{r} MT/s")),
            (None, Some(c)) => Some(format!("running {c} MT/s")),
            (None, None) => None,
        };
        if let Some(sp) = speed {
            if !s.is_empty() {
                s.push_str(", ");
            }
            s.push_str(&sp);
        }
        s
    }
}

/// Collect what this platform will tell us, or `None` if that's nothing.
pub fn collect() -> Option<Memory> {
    let mem = platform_collect();
    // Whether it was readable, not what it said — the diagnostic log's rule.
    log::debug!(
        target: "loadbearer::ram",
        "memory devices {}",
        if mem.is_empty() { "unreadable or absent" } else { "read" },
    );
    if mem.is_empty() { None } else { Some(mem) }
}

#[cfg(windows)]
fn platform_collect() -> Memory {
    crate::identity::raw_smbios_table()
        .map(|t| parse_smbios(&t))
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn platform_collect() -> Memory {
    // 0400 root:root. Unprivileged, this is a permission error, which is the
    // normal case and not worth more than the debug line in `collect`.
    std::fs::read("/sys/firmware/dmi/tables/DMI")
        .map(|t| parse_smbios(&t))
        .unwrap_or_default()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_collect() -> Memory {
    Memory::default()
}

/// Physical Memory Array (type 16) "Use" values that aren't system RAM: video,
/// flash, non-volatile RAM, cache. Devices in those arrays aren't RAM slots.
const NOT_SYSTEM_MEMORY: &[u8] = &[0x04, 0x05, 0x06, 0x07];

/// Read the memory devices out of an SMBIOS structure table.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn parse_smbios(table: &[u8]) -> Memory {
    let structures: Vec<_> = crate::identity::structures(table).collect();

    // Arrays that hold something other than system RAM, by handle. An array
    // whose use is unknown or "other" is kept: most firmware has one array
    // and it is the RAM.
    let excluded: Vec<u16> = structures
        .iter()
        .filter(|s| s.kind == 16 && s.formatted.len() >= 0x0F)
        .filter(|s| NOT_SYSTEM_MEMORY.contains(&s.formatted[0x05]))
        .map(|s| word(s.formatted, 0x02).unwrap_or(0))
        .collect();

    let mut slots = 0u32;
    let mut modules = Vec::new();
    for s in structures.iter().filter(|s| s.kind == 17) {
        let f = s.formatted;
        // 0x15 is the SMBIOS 2.1 length, the shortest a Memory Device has
        // ever been; anything shorter is malformed.
        if f.len() < 0x15 {
            continue;
        }
        if word(f, 0x04).is_some_and(|h| excluded.contains(&h)) {
            continue;
        }
        slots += 1;
        if let Some(m) = module(f) {
            modules.push(m);
        }
    }

    if slots == 0 {
        return Memory::default();
    }
    Memory {
        slots: Some(slots),
        populated: Some(modules.len() as u32),
        modules,
    }
}

/// One type 17 structure's module, or `None` for an empty slot. The field
/// offsets and the length each one needs are from DMTF DSP0134 §7.18.
fn module(f: &[u8]) -> Option<Module> {
    let size = word(f, 0x0C)?;
    // 0 means no module in the socket.
    if size == 0 {
        return None;
    }
    let size_bytes = match size {
        // Installed, size unknown.
        0xFFFF => None,
        // From 2.7: the size is in the Extended Size dword, in MiB (bits
        // 30:0). In an older, shorter structure 0x7FFF is a literal 32767 MiB.
        0x7FFF if f.len() >= 0x20 => dword(f, 0x1C)
            .map(|e| u64::from(e & 0x7FFF_FFFF) << 20)
            .filter(|&b| b > 0),
        // Bit 15 picks the unit: clear is MiB, set is KiB.
        s if s & 0x8000 == 0 => Some(u64::from(s) << 20),
        s => Some(u64::from(s & 0x7FFF) << 10),
    };
    Some(Module {
        size_bytes,
        kind: f
            .get(0x12)
            .and_then(|&t| memory_type(t))
            .map(str::to_string),
        form_factor: f
            .get(0x0E)
            .and_then(|&t| form_factor(t))
            .map(str::to_string),
        // Speed from 2.3 (length 0x17), its dword extension from 3.3 (0x5C).
        speed_mts: speed(f, 0x15, 0x54),
        // Configured speed from 2.7 (0x22), extension from 3.3 (0x5C).
        configured_speed_mts: speed(f, 0x20, 0x58),
    })
}

/// A speed word, where 0 is unknown and 0xFFFF defers to a dword further on
/// (bits 30:0) that only a 3.3+ structure is long enough to carry.
fn speed(f: &[u8], at: usize, extended_at: usize) -> Option<u32> {
    let v = match word(f, at)? {
        0xFFFF if f.len() >= 0x5C => dword(f, extended_at)? & 0x7FFF_FFFF,
        0xFFFF => 0,
        w => u32::from(w),
    };
    (v > 0).then_some(v)
}

fn word(f: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(f.get(at..at + 2)?.try_into().ok()?))
}

fn dword(f: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(f.get(at..at + 4)?.try_into().ok()?))
}

/// SMBIOS Memory Device Type (§7.18.2). Other, Unknown, reserved and
/// anything newer than this table are `None`.
fn memory_type(code: u8) -> Option<&'static str> {
    Some(match code {
        0x03 => "DRAM",
        0x04 => "EDRAM",
        0x05 => "VRAM",
        0x06 => "SRAM",
        0x07 => "RAM",
        0x08 => "ROM",
        0x09 => "Flash",
        0x0A => "EEPROM",
        0x0B => "FEPROM",
        0x0C => "EPROM",
        0x0D => "CDRAM",
        0x0E => "3DRAM",
        0x0F => "SDRAM",
        0x10 => "SGRAM",
        0x11 => "RDRAM",
        0x12 => "DDR",
        0x13 => "DDR2",
        0x14 => "DDR2 FB-DIMM",
        0x18 => "DDR3",
        0x19 => "FBD2",
        0x1A => "DDR4",
        0x1B => "LPDDR",
        0x1C => "LPDDR2",
        0x1D => "LPDDR3",
        0x1E => "LPDDR4",
        0x1F => "Logical non-volatile device",
        0x20 => "HBM",
        0x21 => "HBM2",
        0x22 => "DDR5",
        0x23 => "LPDDR5",
        0x24 => "HBM3",
        _ => return None,
    })
}

/// SMBIOS Memory Device Form Factor (§7.18.1). Other and Unknown are `None`.
fn form_factor(code: u8) -> Option<&'static str> {
    Some(match code {
        0x03 => "SIMM",
        0x04 => "SIP",
        0x05 => "Chip",
        0x06 => "DIP",
        0x07 => "ZIP",
        0x08 => "Proprietary Card",
        0x09 => "DIMM",
        0x0A => "TSOP",
        0x0B => "Row Of Chips",
        0x0C => "RIMM",
        0x0D => "SODIMM",
        0x0E => "SRIMM",
        0x0F => "FB-DIMM",
        0x10 => "Die",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    /// A type 17 structure of `len` bytes, with the strings a real one
    /// carries — locator, bank, manufacturer, serial, asset tag, part number —
    /// so the test proves those aren't what's read.
    struct Dev {
        len: usize,
        array: u16,
        size: u16,
        form: u8,
        kind: u8,
        speed: u16,
        ext_size: u32,
        configured: u16,
        ext_speed: u32,
        ext_configured: u32,
    }

    impl Default for Dev {
        fn default() -> Self {
            Dev {
                len: 0x5C,
                array: 0x1000,
                size: 16 * 1024,
                form: 0x0D,
                kind: 0x22,
                speed: 5600,
                ext_size: 0,
                configured: 5200,
                ext_speed: 0,
                ext_configured: 0,
            }
        }
    }

    impl Dev {
        fn bytes(&self, handle: u16) -> Vec<u8> {
            let mut f = vec![0u8; self.len.max(0x5C)];
            f[0] = 17;
            f[1] = self.len as u8;
            f[2..4].copy_from_slice(&handle.to_le_bytes());
            f[0x04..0x06].copy_from_slice(&self.array.to_le_bytes());
            f[0x0C..0x0E].copy_from_slice(&self.size.to_le_bytes());
            f[0x0E] = self.form;
            f[0x10] = 1; // device locator
            f[0x11] = 2; // bank locator
            f[0x12] = self.kind;
            f[0x15..0x17].copy_from_slice(&self.speed.to_le_bytes());
            f[0x17] = 3; // manufacturer
            f[0x18] = 4; // serial number
            f[0x19] = 5; // asset tag
            f[0x1A] = 6; // part number
            f[0x1C..0x20].copy_from_slice(&self.ext_size.to_le_bytes());
            f[0x20..0x22].copy_from_slice(&self.configured.to_le_bytes());
            f[0x54..0x58].copy_from_slice(&self.ext_speed.to_le_bytes());
            f[0x58..0x5C].copy_from_slice(&self.ext_configured.to_le_bytes());
            f.truncate(self.len);
            for s in ["DIMM 0", "BANK 0", "ACME", "SERIAL-1", "ASSET-1", "PART-1"] {
                f.extend_from_slice(s.as_bytes());
                f.push(0);
            }
            f.push(0);
            f
        }
    }

    /// Type 16 (Physical Memory Array) with the given handle and use.
    fn array(handle: u16, usage: u8) -> Vec<u8> {
        let mut f = vec![0u8; 0x17];
        f[0] = 16;
        f[1] = 0x17;
        f[2..4].copy_from_slice(&handle.to_le_bytes());
        f[0x04] = 0x03; // location: system board
        f[0x05] = usage;
        f[0x06] = 0x03; // error correction: none
        f[0x0D..0x0F].copy_from_slice(&2u16.to_le_bytes());
        f.extend_from_slice(&[0, 0]);
        f
    }

    fn table(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut t: Vec<u8> = parts.concat();
        t.extend_from_slice(&[127, 4, 0xFF, 0xFF, 0, 0]);
        t
    }

    #[test]
    fn two_ddr5_sodimms_in_a_modern_table() {
        let t = table(&[
            array(0x1000, 0x03),
            Dev::default().bytes(0x1100),
            Dev::default().bytes(0x1101),
        ]);
        let m = parse_smbios(&t);
        assert_eq!(m.slots, Some(2));
        assert_eq!(m.populated, Some(2));
        let one = Module {
            size_bytes: Some(16 * GIB),
            kind: Some("DDR5".into()),
            form_factor: Some("SODIMM".into()),
            speed_mts: Some(5600),
            configured_speed_mts: Some(5200),
        };
        assert_eq!(m.modules, vec![one.clone(), one]);
        assert_eq!(
            m.summary().as_deref(),
            Some("2 of 2 slots · 2 × 16.0 GiB DDR5 SODIMM, 5600 MT/s (running 5200)")
        );
    }

    #[test]
    fn an_empty_slot_counts_as_a_slot_not_a_module() {
        let empty = Dev {
            size: 0,
            kind: 0x02,
            form: 0x02,
            speed: 0,
            configured: 0,
            ..Default::default()
        };
        let t = table(&[Dev::default().bytes(1), empty.bytes(2)]);
        let m = parse_smbios(&t);
        assert_eq!((m.slots, m.populated), (Some(2), Some(1)));
        assert_eq!(m.modules.len(), 1);
    }

    #[test]
    fn extended_size_and_extended_speed() {
        let big = Dev {
            kind: 0x1A,
            form: 0x09,
            size: 0x7FFF,
            ext_size: 64 * 1024, // MiB
            speed: 0xFFFF,
            ext_speed: 8800,
            configured: 0xFFFF,
            ext_configured: 6400,
            ..Default::default()
        };
        let m = parse_smbios(&table(&[big.bytes(1)]));
        let module = &m.modules[0];
        assert_eq!(module.size_bytes, Some(64 * GIB));
        assert_eq!(module.kind.as_deref(), Some("DDR4"));
        assert_eq!(module.form_factor.as_deref(), Some("DIMM"));
        assert_eq!(module.speed_mts, Some(8800));
        assert_eq!(module.configured_speed_mts, Some(6400));
    }

    #[test]
    fn kib_granularity() {
        let small = Dev {
            size: 0x8000 | 512,
            ..Default::default()
        };
        let m = parse_smbios(&table(&[small.bytes(1)]));
        assert_eq!(m.modules[0].size_bytes, Some(512 * 1024));
    }

    #[test]
    fn a_short_pre_2_7_structure_reads_what_it_has() {
        // SMBIOS 2.3–2.6: 0x1B or 0x1C bytes, no extended size, no
        // configured speed. 0x7FFF here is a real 32767 MiB, not a pointer to
        // an extended size the structure is too short to have.
        let old = Dev {
            len: 0x1B,
            size: 0x7FFF,
            kind: 0x18,
            form: 0x09,
            speed: 1600,
            ..Default::default()
        };
        let m = parse_smbios(&table(&[old.bytes(1)]));
        let module = &m.modules[0];
        assert_eq!(module.size_bytes, Some(32767 << 20));
        assert_eq!(module.kind.as_deref(), Some("DDR3"));
        assert_eq!(module.speed_mts, Some(1600));
        assert_eq!(module.configured_speed_mts, None);

        // 2.1/2.2, 0x15 bytes: no speed at all.
        let older = Dev {
            len: 0x15,
            ..Default::default()
        };
        let m = parse_smbios(&table(&[older.bytes(1)]));
        assert_eq!(m.modules[0].speed_mts, None);
        assert_eq!(m.modules[0].size_bytes, Some(16 * GIB));
    }

    #[test]
    fn junk_values_are_absent_not_reported() {
        let junk = Dev {
            len: 0x22,     // pre-3.3: 0xFFFF speeds have nowhere to point
            size: 0xFFFF,  // installed, size unknown
            kind: 0x02,    // Unknown
            form: 0x01,    // Other
            speed: 0xFFFF, // extended, but no extended field
            configured: 0, // unknown
            ..Default::default()
        };
        let m = parse_smbios(&table(&[junk.bytes(1)]));
        assert_eq!((m.slots, m.populated), (Some(1), Some(1)));
        assert_eq!(m.modules, vec![Module::default()]);
        // Unknown and reserved type codes don't become strings.
        for code in [0x00, 0x01, 0x02, 0x15, 0x16, 0x17, 0x25, 0xFF] {
            assert_eq!(memory_type(code), None, "{code:#x}");
        }
        // The extended dwords' reserved top bit is ignored, and a zero
        // extended size is unknown, not 0 bytes.
        let odd = Dev {
            size: 0x7FFF,
            ext_size: 0,
            speed: 0xFFFF,
            ext_speed: 0x8000_0000 | 4800,
            ..Default::default()
        };
        let m = parse_smbios(&table(&[odd.bytes(1)]));
        assert_eq!(m.modules[0].size_bytes, None);
        assert_eq!(m.modules[0].speed_mts, Some(4800));
    }

    #[test]
    fn devices_in_a_non_ram_array_are_not_slots() {
        let flash = Dev {
            array: 0x2000,
            ..Default::default()
        };
        let t = table(&[
            array(0x1000, 0x03),
            array(0x2000, 0x05), // flash memory
            Dev::default().bytes(1),
            flash.bytes(2),
        ]);
        let m = parse_smbios(&t);
        assert_eq!((m.slots, m.populated), (Some(1), Some(1)));
    }

    #[test]
    fn no_memory_devices_is_no_block() {
        let t = table(&[array(0x1000, 0x03)]);
        assert!(parse_smbios(&t).is_empty());
        assert!(parse_smbios(&[]).is_empty());
    }

    #[test]
    fn identifiers_in_the_structure_never_reach_the_block() {
        let t = table(&[Dev::default().bytes(1)]);
        let json = serde_json::to_string(&parse_smbios(&t)).unwrap();
        for s in ["SERIAL-1", "ASSET-1", "PART-1", "ACME", "DIMM 0"] {
            assert!(!json.contains(s), "{s} leaked into {json}");
        }
        assert!(json.contains("\"type\":\"DDR5\""), "{json}");
    }

    #[test]
    fn a_truncated_table_yields_what_it_can_without_panicking() {
        let full = table(&[
            array(0x1000, 0x03),
            Dev::default().bytes(1),
            Dev {
                len: 0x1B,
                ..Default::default()
            }
            .bytes(2),
        ]);
        for cut in 0..full.len() {
            let _ = parse_smbios(&full[..cut]);
        }
    }
}
