//! The OS release in pieces a collector can sort and filter on, alongside the
//! free-text `machine.os` / `machine.kernel` strings, which stay as they are.
//!
//! - **Windows**: four values under
//!   `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion`, read-only —
//!   `CurrentBuildNumber` and `UBR` (together the build, `26100.2314`),
//!   `DisplayVersion` (`24H2`) and `EditionID` (`Enterprise`). Nothing else in
//!   that key is read: not `ProductName`, which still says "Windows 10" on
//!   Windows 11, and not the product ID, registered owner and organisation or
//!   install date that sit beside them. The key is shared between the 32- and
//!   64-bit registry views, so the view doesn't matter.
//! - **Linux**: `ID`, `VERSION_ID` and `VARIANT_ID` from `/etc/os-release`,
//!   or `/usr/lib/os-release` when that's missing, as os-release(5) says.
//!
//! Each field is absent when it couldn't be read, and the block is absent
//! when none could.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OsDetail {
    /// Linux distribution, as os-release `ID`: `ubuntu`, `debian`, `fedora`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distro: Option<String>,
    /// The release. Windows `DisplayVersion` (`24H2`); os-release
    /// `VERSION_ID` (`24.04`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Windows build with its update revision, `26100.2314`. Patch Tuesday
    /// moves the part after the dot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    /// Windows `EditionID` (`Professional`, `Enterprise`, `ServerStandard`);
    /// os-release `VARIANT_ID` (`server`, `workstation`) where the
    /// distribution sets one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edition: Option<String>,
}

impl OsDetail {
    pub fn is_empty(&self) -> bool {
        *self == OsDetail::default()
    }

    /// "Enterprise · 24H2 · build 26100.2314", "ubuntu 24.04 · server".
    pub fn summary(&self) -> Option<String> {
        let release = [self.distro.as_deref(), self.version.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        let parts: Vec<String> = [
            self.edition.clone().filter(|_| self.distro.is_none()),
            (!release.is_empty()).then_some(release),
            self.edition.clone().filter(|_| self.distro.is_some()),
            self.build.as_ref().map(|b| format!("build {b}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// Collect what this platform will tell us, or `None` if that's nothing.
pub fn collect() -> Option<OsDetail> {
    let os = platform_collect();
    log::debug!(
        target: "loadbearer::os_detail",
        "distro={} version={} build={} edition={}",
        readable(&os.distro),
        readable(&os.version),
        readable(&os.build),
        readable(&os.edition),
    );
    if os.is_empty() { None } else { Some(os) }
}

fn readable(field: &Option<String>) -> &'static str {
    if field.is_some() { "present" } else { "absent" }
}

/// Trimmed, and `None` when that leaves nothing.
fn text(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn platform_collect() -> OsDetail {
    from_windows(
        win::string("CurrentBuildNumber"),
        win::dword("UBR"),
        win::string("DisplayVersion"),
        win::string("EditionID"),
    )
}

/// Assemble the block from the four registry values, each absent if unread.
#[cfg_attr(not(windows), allow(dead_code))]
fn from_windows(
    build: Option<String>,
    ubr: Option<u32>,
    display_version: Option<String>,
    edition: Option<String>,
) -> OsDetail {
    // A build without its revision is still the build; a revision without
    // the build it revises is meaningless.
    let build = build.as_deref().and_then(text).map(|b| match ubr {
        Some(r) => format!("{b}.{r}"),
        None => b,
    });
    OsDetail {
        distro: None,
        version: display_version.as_deref().and_then(text),
        build,
        edition: edition.as_deref().and_then(text),
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
    };

    const KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\0";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    /// A `REG_SZ` value from the key, read-only.
    pub fn string(name: &str) -> Option<String> {
        let key = wide(KEY);
        let value = wide(&format!("{name}\0"));
        // Every value read here is a few characters long.
        let mut buf = [0u16; 128];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: both names are NUL-terminated UTF-16 that outlive the call,
        // and `size` is the byte length of `buf`, which the call won't exceed.
        let rc = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_void,
                &mut size,
            )
        };
        if rc != 0 {
            log::debug!(target: "loadbearer::os_detail", "RegGetValueW({name}) failed: {rc}");
            return None;
        }
        let units = (size as usize / 2).min(buf.len());
        Some(
            String::from_utf16_lossy(&buf[..units])
                .trim_end_matches('\0')
                .to_string(),
        )
    }

    /// A `REG_DWORD` value from the key, read-only.
    pub fn dword(name: &str) -> Option<u32> {
        let key = wide(KEY);
        let value = wide(&format!("{name}\0"));
        let mut out = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: as in `string`; `out` is exactly the four bytes `size` says.
        let rc = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut out as *mut u32 as *mut c_void,
                &mut size,
            )
        };
        if rc != 0 {
            log::debug!(target: "loadbearer::os_detail", "RegGetValueW({name}) failed: {rc}");
            return None;
        }
        Some(out)
    }
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn platform_collect() -> OsDetail {
    std::fs::read_to_string("/etc/os-release")
        .or_else(|_| std::fs::read_to_string("/usr/lib/os-release"))
        .map(|t| parse_os_release(&t))
        .unwrap_or_default()
}

/// The three fields this block wants from an os-release file: newline-
/// separated `KEY=value` assignments, values optionally shell-quoted, `#`
/// comments.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_os_release(text_: &str) -> OsDetail {
    let mut os = OsDetail::default();
    for line in text_.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let slot = match key.trim() {
            "ID" => &mut os.distro,
            "VERSION_ID" => &mut os.version,
            "VARIANT_ID" => &mut os.edition,
            _ => continue,
        };
        *slot = text(&unquote(value));
    }
    os
}

/// Undo os-release's shell-style quoting: one pair of matching `"` or `'`,
/// and inside double quotes the backslash escapes for `"`, `\`, `$` and `` ` ``.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn unquote(v: &str) -> String {
    let v = v.trim();
    let quoted = |q: char| v.len() >= 2 && v.starts_with(q) && v.ends_with(q);
    if quoted('\'') {
        return v[1..v.len() - 1].to_string();
    }
    if !quoted('"') {
        return v.to_string();
    }
    let mut out = String::new();
    let mut chars = v[1..v.len() - 1].chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(e @ ('"' | '\\' | '$' | '`')) => out.push(e),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Anywhere else
// ---------------------------------------------------------------------------

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_collect() -> OsDetail {
    OsDetail::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_values_become_the_block() {
        let os = from_windows(
            Some("26100".into()),
            Some(2314),
            Some("24H2".into()),
            Some("Professional".into()),
        );
        assert_eq!(os.build.as_deref(), Some("26100.2314"));
        assert_eq!(os.version.as_deref(), Some("24H2"));
        assert_eq!(os.edition.as_deref(), Some("Professional"));
        assert_eq!(os.distro, None);
        assert_eq!(
            os.summary().as_deref(),
            Some("Professional · 24H2 · build 26100.2314")
        );
    }

    #[test]
    fn windows_partial_reads() {
        // Before 20H2 there is no DisplayVersion; a revision alone is dropped.
        let os = from_windows(Some("17763".into()), None, None, Some(" ".into()));
        assert_eq!(os.build.as_deref(), Some("17763"));
        assert_eq!((os.version, os.edition), (None, None));
        assert!(from_windows(None, Some(5), None, None).is_empty());
    }

    #[test]
    fn ubuntu_os_release() {
        let os = parse_os_release(
            "PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nNAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\n\
             VERSION=\"24.04.1 LTS (Noble Numbat)\"\nID=ubuntu\nID_LIKE=debian\n",
        );
        assert_eq!(os.distro.as_deref(), Some("ubuntu"));
        assert_eq!(os.version.as_deref(), Some("24.04"));
        assert_eq!(os.edition, None);
        assert_eq!(os.build, None);
        assert_eq!(os.summary().as_deref(), Some("ubuntu 24.04"));
    }

    #[test]
    fn fedora_variant_and_quoting() {
        let os = parse_os_release(
            "# comment\nNAME='Fedora Linux'\nID=fedora\nVERSION_ID=41\n\
             VARIANT=\"Server Edition\"\nVARIANT_ID=server\n\nnot an assignment\n",
        );
        assert_eq!(os.distro.as_deref(), Some("fedora"));
        assert_eq!(os.version.as_deref(), Some("41"));
        assert_eq!(os.edition.as_deref(), Some("server"));
        assert_eq!(os.summary().as_deref(), Some("fedora 41 · server"));
    }

    #[test]
    fn rolling_release_without_a_version() {
        let os = parse_os_release("NAME=\"Arch Linux\"\nID=arch\nBUILD_ID=rolling\n");
        assert_eq!(os.distro.as_deref(), Some("arch"));
        assert_eq!(os.version, None);
    }

    #[test]
    fn escapes_and_empty_values() {
        assert_eq!(unquote(r#""a \"b\" \$c \\d""#), r#"a "b" $c \d"#);
        assert_eq!(unquote("'x'"), "x");
        assert_eq!(unquote("\""), "\"");
        let os = parse_os_release("ID=\"\"\nVERSION_ID=\n");
        assert!(os.is_empty());
        assert!(parse_os_release("").is_empty());
    }
}
