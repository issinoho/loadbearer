//! `machine.security`, collected only with `--collect security`: how the
//! machine booted, whether Secure Boot is on, whether it has a TPM, and
//! whether its system volume is encrypted.
//!
//! Each part is present only if it could be read, by an unprivileged process,
//! without WMI, COM or a child process (see PRIVACY.md) — with **one stated
//! exception**, BitLocker on an elevated Windows run, below. A part that can't
//! be read on a platform is simply absent there:
//!
//! | Part              | Windows                                   | Linux |
//! |-------------------|-------------------------------------------|-------|
//! | `firmware_mode`   | `GetFirmwareType`                         | whether `/sys/firmware/efi` exists (x86 only for `"bios"`) |
//! | `secure_boot`     | `UEFISecureBootEnabled` under `HKLM\SYSTEM\CurrentControlSet\Control\SecureBoot\State` | the `SecureBoot` EFI variable in efivarfs |
//! | `tpm`             | `Tbsi_GetDeviceInfo` (tbs.dll, loaded from System32 only when asked) | `/sys/class/tpm/tpm0/tpm_version_major`, `/sys/class/tpmrm/tpmrm0` |
//! | `disk_encryption` | **elevated runs only**: WMI's `Win32_EncryptableVolume`, the system volume — see below | the root filesystem's device-mapper chain in sysfs |
//!
//! **BitLocker is read through WMI, and only when the run is elevated.** It is
//! the one place loadbearer uses WMI or COM. Every documented way to learn
//! BitLocker's state — `Win32_EncryptableVolume` (WMI), `manage-bde` and
//! `Get-BitLockerVolume` (child processes) — needs administrator rights, and
//! what a standard user *can* read (the `BitLockerStatus\BootStatus` registry
//! value, `fveapi.dll`'s `FveGetStatus`) is undocumented, with no stated
//! meaning to hold it to. So of the documented sources, the one that needs no
//! child process is used, and only where it can answer:
//!
//! - only with `--collect security`, and only when this process is elevated
//!   ([`crate::elevation`]); otherwise WMI is never touched and
//!   `disk_encryption` is absent;
//! - only the class `Win32_EncryptableVolume` in
//!   `ROOT\CIMV2\Security\MicrosoftVolumeEncryption`, and only its
//!   `DriveLetter`, `ConversionStatus`, `ProtectionStatus` and
//!   `EncryptionMethod`, of only the volume holding the Windows directory
//!   (`GetSystemWindowsDirectoryW`'s drive letter, named in the query's
//!   `WHERE`). `DeviceID`, `PersistentVolumeID`, key protectors, recovery
//!   information and every other volume are never asked for, and no volume
//!   identifier is recorded or logged;
//! - on a thread of its own that initialises COM and uninitialises it again,
//!   without `CoInitializeSecurity` (which would set process-wide state): the
//!   proxy blanket is set on the WMI proxies instead. The whole read is given
//!   ten seconds; past that it is abandoned and the part is absent.
//!
//! Anything that goes wrong — not elevated, no such namespace or class, a
//! failed or timed-out query, no row for the system volume, a value the
//! documentation doesn't list — is an absent `disk_encryption`, never a guess
//! and never a fall-back to an undocumented source. "Encrypted" and
//! "protection on" are kept apart (`system_volume` and `protection`), since
//! protection can be suspended on an encrypted volume.
//!
//! On Linux only the **root filesystem** is looked at, not every volume, and
//! only for **dm-crypt** set up by cryptsetup (LUKS1, LUKS2, plain, …),
//! recognised by the `CRYPT-<type>-` prefix cryptsetup gives the mapping's
//! device-mapper UUID. ZFS native encryption, fscrypt, eCryptfs and
//! self-encrypting drives are not looked for, so a root that isn't on
//! dm-crypt is `"none_detected"`, never "not encrypted". The UUID itself is
//! never recorded or logged; only its type prefix is used.
//!
//! Nothing that identifies the TPM — its endorsement key, manufacturer or
//! firmware revision — is read or recorded; only whether there is one and
//! which major version.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Security {
    /// `"uefi"` or `"bios"` (booted without UEFI: legacy BIOS/CSM, or a VM
    /// booted straight into its kernel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_mode: Option<String>,
    /// Whether UEFI Secure Boot is enforcing. Absent on a machine that didn't
    /// boot through UEFI, where there is nothing to read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secure_boot: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tpm: Option<Tpm>,
    /// Linux, and Windows on an elevated run; see the module docs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_encryption: Option<DiskEncryption>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tpm {
    /// Whether the OS exposes a TPM. `false` covers none fitted, one turned
    /// off in firmware, and (Linux) no driver for it.
    pub present: bool,
    /// `"2.0"` or `"1.2"`. Absent when present but the OS doesn't say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskEncryption {
    /// Linux — `"encrypted"`: the root filesystem sits on a dm-crypt mapping,
    /// directly or through LVM / RAID. `"none_detected"`: neither it nor any
    /// device beneath it is dm-crypt — other kinds of encryption are not
    /// looked for.
    ///
    /// Windows — the system volume's BitLocker `ConversionStatus`:
    /// `"encrypted"` (fully encrypted), `"not_encrypted"` (fully decrypted),
    /// or `"partly_encrypted"` (encryption or decryption in progress or
    /// paused).
    pub system_volume: String,
    /// Linux, with `"encrypted"`: `luks1`, `luks2`, `plain`, `loopaes`,
    /// `tcrypt`, `bitlk` or `fvault2`, the cryptsetup type. Windows, with
    /// `"encrypted"` or `"partly_encrypted"`: `bitlocker`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Windows only, with `"encrypted"` or `"partly_encrypted"`: BitLocker's
    /// `EncryptionMethod` — `xts-aes-128`, `xts-aes-256`, `aes-128`,
    /// `aes-256`, `aes-128-diffuser`, `aes-256-diffuser`, or `hardware` (the
    /// drive's own encryption). Absent when BitLocker reports none or an
    /// unknown method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cipher: Option<String>,
    /// Windows only: BitLocker's `ProtectionStatus`, `"on"` or `"off"`. Off
    /// on an encrypted volume means protection is suspended (the key is in
    /// the clear on the disk). Absent when BitLocker can't tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protection: Option<String>,
}

impl Security {
    pub fn is_empty(&self) -> bool {
        *self == Security::default()
    }

    /// The rows `info` shows, `(label, value)`, with "unknown" for a part
    /// asked for and not read.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let unknown = || "unknown".to_string();
        vec![
            (
                "Firmware",
                match self.firmware_mode.as_deref() {
                    Some("uefi") => "UEFI".to_string(),
                    Some("bios") => "BIOS (not UEFI)".to_string(),
                    Some(other) => other.to_string(),
                    None => unknown(),
                },
            ),
            (
                "Secure Boot",
                match self.secure_boot {
                    Some(true) => "on".to_string(),
                    Some(false) => "off".to_string(),
                    None if self.firmware_mode.as_deref() == Some("bios") => {
                        "n/a (not UEFI)".to_string()
                    }
                    None => unknown(),
                },
            ),
            (
                "TPM",
                match &self.tpm {
                    Some(t) => t.summary(),
                    None => unknown(),
                },
            ),
            (
                "Encryption",
                match &self.disk_encryption {
                    Some(d) => d.summary(),
                    None if cfg!(windows) => {
                        "unknown (BitLocker is read only when elevated)".into()
                    }
                    None => unknown(),
                },
            ),
        ]
    }

    /// "UEFI · Secure Boot on · TPM 2.0 · root on LUKS2", for the results
    /// summary: only what was read.
    pub fn summary(&self) -> Option<String> {
        let parts: Vec<String> = [
            self.firmware_mode.as_deref().map(|m| match m {
                "uefi" => "UEFI".to_string(),
                "bios" => "BIOS".to_string(),
                other => other.to_string(),
            }),
            self.secure_boot
                .map(|s| format!("Secure Boot {}", if s { "on" } else { "off" })),
            self.tpm.as_ref().map(|t| format!("TPM {}", t.summary())),
            self.disk_encryption.as_ref().map(|d| d.summary()),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

impl Tpm {
    fn summary(&self) -> String {
        match (self.present, self.version.as_deref()) {
            (false, _) => "none".to_string(),
            (true, Some(v)) => v.to_string(),
            (true, None) => "present".to_string(),
        }
    }
}

impl DiskEncryption {
    /// Linux: "root on LUKS2". Windows: "encrypted (BitLocker, XTS-AES 256,
    /// protection on)", "partly encrypted (…)", "not encrypted".
    fn summary(&self) -> String {
        let bitlocker = || {
            let details: Vec<String> = [
                Some("BitLocker".to_string()),
                self.cipher.as_deref().map(cipher_name),
                self.protection
                    .as_deref()
                    .map(|p| format!("protection {p}")),
            ]
            .into_iter()
            .flatten()
            .collect();
            details.join(", ")
        };
        match (self.system_volume.as_str(), self.method.as_deref()) {
            ("encrypted", Some(METHOD_BITLOCKER)) => format!("encrypted ({})", bitlocker()),
            ("partly_encrypted", Some(METHOD_BITLOCKER)) => {
                format!("partly encrypted ({})", bitlocker())
            }
            ("partly_encrypted", _) => "partly encrypted".to_string(),
            ("not_encrypted", _) => "not encrypted".to_string(),
            ("encrypted", Some(m)) => format!("root on {}", m.to_uppercase()),
            ("encrypted", None) => "root encrypted".to_string(),
            ("none_detected", _) => "no dm-crypt under root".to_string(),
            (other, _) => other.to_string(),
        }
    }
}

/// Only [`crate::optin::gather`] calls this, and only when `security` was
/// asked for.
pub(crate) fn read() -> Option<Security> {
    let s = platform_read();
    (!s.is_empty()).then_some(s)
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn platform_read() -> Security {
    Security {
        firmware_mode: win::firmware_type().and_then(firmware_mode_from_type),
        secure_boot: secure_boot_from_dword(win::secure_boot_state()),
        tpm: win::tbs_device_info().and_then(|(rc, v)| tpm_from_tbs(rc, v)),
        disk_encryption: bitlocker_state(
            crate::elevation::read(),
            win::system_windows_directory().as_deref(),
            wmi::encryptable_volumes,
        ),
    }
}

/// `method` for a volume BitLocker has encrypted, or is encrypting.
const METHOD_BITLOCKER: &str = "bitlocker";

/// A cipher slug as people write it: "XTS-AES 256", "AES 128 with diffuser",
/// "hardware encryption". A slug a later version added is shown as it is.
fn cipher_name(slug: &str) -> String {
    match slug {
        "xts-aes-128" => "XTS-AES 128",
        "xts-aes-256" => "XTS-AES 256",
        "aes-128" => "AES 128",
        "aes-256" => "AES 256",
        "aes-128-diffuser" => "AES 128 with diffuser",
        "aes-256-diffuser" => "AES 256 with diffuser",
        "hardware" => "hardware encryption",
        other => other,
    }
    .to_string()
}

/// The four `Win32_EncryptableVolume` properties loadbearer asks for, as WMI
/// returned them: `None` for a property that was null or of an unexpected
/// type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) struct EncryptableVolume {
    /// `"C:"`; null for a volume without a drive letter.
    pub drive_letter: Option<String>,
    pub conversion_status: Option<u32>,
    pub protection_status: Option<u32>,
    pub encryption_method: Option<u32>,
}

/// The drive letter of a path such as `C:\WINDOWS`, upper-cased. Anything
/// else — a UNC or `\\?\` path, an empty string — has none.
#[cfg_attr(not(windows), allow(dead_code))]
fn drive_letter_of(path: &str) -> Option<char> {
    let mut chars = path.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    (chars.next() == Some(':')).then(|| letter.to_ascii_uppercase())
}

/// The one volume whose `DriveLetter` is `letter:`, case-insensitively. None,
/// or more than one, is no answer.
#[cfg_attr(not(windows), allow(dead_code))]
fn system_volume(volumes: &[EncryptableVolume], letter: char) -> Option<&EncryptableVolume> {
    let want = format!("{letter}:");
    let mut matching = volumes.iter().filter(|v| {
        v.drive_letter
            .as_deref()
            .is_some_and(|d| d.trim().eq_ignore_ascii_case(&want))
    });
    let first = matching.next()?;
    matching.next().is_none().then_some(first)
}

/// `ConversionStatus`, from Win32_EncryptableVolume's documentation:
/// 0 FullyDecrypted, 1 FullyEncrypted, 2 EncryptionInProgress,
/// 3 DecryptionInProgress, 4 EncryptionPaused, 5 DecryptionPaused (the last
/// four all "partially encrypted"). Any other value is no answer.
#[cfg_attr(not(windows), allow(dead_code))]
fn system_volume_from_conversion(status: u32) -> Option<&'static str> {
    match status {
        0 => Some("not_encrypted"),
        1 => Some("encrypted"),
        2..=5 => Some("partly_encrypted"),
        _ => None,
    }
}

/// `EncryptionMethod`, from GetEncryptionMethod's documentation: 0 None,
/// 1 AES_128_WITH_DIFFUSER, 2 AES_256_WITH_DIFFUSER, 3 AES_128, 4 AES_256,
/// 5 HARDWARE_ENCRYPTION, 6 XTS_AES_128, 7 XTS_AES_256. None (0), UNKNOWN
/// ((uint32)-1) and any value not listed have no cipher to report. (The
/// class page's property table calls 7 "XTS-AES 256 WITH DIFFUSER"; the
/// method page, and XTS itself, have no diffuser, so the method page is
/// followed.)
#[cfg_attr(not(windows), allow(dead_code))]
fn cipher_from_method(method: u32) -> Option<&'static str> {
    match method {
        1 => Some("aes-128-diffuser"),
        2 => Some("aes-256-diffuser"),
        3 => Some("aes-128"),
        4 => Some("aes-256"),
        5 => Some("hardware"),
        6 => Some("xts-aes-128"),
        7 => Some("xts-aes-256"),
        _ => None,
    }
}

/// `ProtectionStatus`: 0 Unprotected, 1 Protected, 2 Unknown. Unknown, or
/// a value not listed, is no answer.
#[cfg_attr(not(windows), allow(dead_code))]
fn protection_from_status(status: u32) -> Option<&'static str> {
    match status {
        0 => Some("off"),
        1 => Some("on"),
        _ => None,
    }
}

/// The system volume's [`DiskEncryption`], or `None` when its
/// `ConversionStatus` is missing or undocumented: `system_volume` is the
/// part's whole point, and is never guessed.
#[cfg_attr(not(windows), allow(dead_code))]
fn bitlocker_from_volume(v: &EncryptableVolume) -> Option<DiskEncryption> {
    let system_volume = system_volume_from_conversion(v.conversion_status?)?;
    let on_disk = system_volume != "not_encrypted";
    Some(DiskEncryption {
        system_volume: system_volume.into(),
        method: on_disk.then(|| METHOD_BITLOCKER.into()),
        cipher: v
            .encryption_method
            .filter(|_| on_disk)
            .and_then(cipher_from_method)
            .map(Into::into),
        protection: v
            .protection_status
            .and_then(protection_from_status)
            .map(Into::into),
    })
}

/// Why the WMI read gave no answer, for a debug log line. Never carries a
/// volume identifier, nor the drive letter.
pub(crate) type WmiFailure = String;

/// BitLocker's state for the system volume, the policy around the WMI read:
/// nothing is asked of WMI (`query` isn't called) unless the run is known to
/// be elevated and the Windows directory has a drive letter. Every way of
/// not getting an answer is `None` and a debug line.
#[cfg_attr(not(windows), allow(dead_code))]
fn bitlocker_state(
    elevated: Option<bool>,
    windows_dir: Option<&str>,
    query: impl FnOnce(char) -> Result<Vec<EncryptableVolume>, WmiFailure>,
) -> Option<DiskEncryption> {
    let skip = |why: &str| {
        log::debug!(target: "loadbearer::security", "BitLocker not read: {why}");
        None
    };
    if elevated != Some(true) {
        return skip("this run is not elevated");
    }
    let Some(letter) = windows_dir.and_then(drive_letter_of) else {
        return skip("the Windows directory has no drive letter");
    };
    let volumes = match query(letter) {
        Ok(v) => v,
        Err(why) => return skip(&why),
    };
    let Some(volume) = system_volume(&volumes, letter) else {
        return skip("no single Win32_EncryptableVolume for the system volume");
    };
    bitlocker_from_volume(volume).or_else(|| skip("ConversionStatus missing or undocumented"))
}

/// `GetFirmwareType`'s `FIRMWARE_TYPE`: 1 is BIOS, 2 is UEFI; anything else
/// (unknown, or a value added later) is no answer.
#[cfg_attr(not(windows), allow(dead_code))]
fn firmware_mode_from_type(t: i32) -> Option<String> {
    match t {
        1 => Some("bios".into()),
        2 => Some("uefi".into()),
        _ => None,
    }
}

/// `UEFISecureBootEnabled` is a DWORD, 1 when Secure Boot is on, 0 when off.
#[cfg_attr(not(windows), allow(dead_code))]
fn secure_boot_from_dword(v: Option<u32>) -> Option<bool> {
    match v? {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

/// `TBS_SUCCESS`.
#[cfg_attr(not(windows), allow(dead_code))]
const TBS_SUCCESS: u32 = 0;
/// `TBS_E_TPM_NOT_FOUND`: no TPM, or one disabled in firmware.
#[cfg_attr(not(windows), allow(dead_code))]
const TBS_E_TPM_NOT_FOUND: u32 = 0x8028_400F;

/// The result code and `tpmVersion` from `Tbsi_GetDeviceInfo`
/// (`TPM_VERSION_12` = 1, `TPM_VERSION_20` = 2). Any other failure — the
/// TBS service stopped, an access error — is no answer, not "no TPM".
#[cfg_attr(not(windows), allow(dead_code))]
fn tpm_from_tbs(rc: u32, version: u32) -> Option<Tpm> {
    match rc {
        TBS_SUCCESS => Some(Tpm {
            present: true,
            version: match version {
                1 => Some("1.2".into()),
                2 => Some("2.0".into()),
                _ => None,
            },
        }),
        TBS_E_TPM_NOT_FOUND => Some(Tpm {
            present: false,
            version: None,
        }),
        _ => None,
    }
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;
    use windows_sys::Win32::System::Registry::{
        HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW,
    };
    use windows_sys::Win32::System::SystemInformation::GetFirmwareType;

    /// The Windows directory, `C:\WINDOWS` say: its drive letter names the
    /// system volume. `GetSystemWindowsDirectoryW` rather than
    /// `GetWindowsDirectoryW`, which a Terminal Server session redirects.
    pub fn system_windows_directory() -> Option<String> {
        use windows_sys::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;
        let mut buf = [0u16; 261];
        // SAFETY: the buffer is valid for writes of the length passed.
        let n = unsafe { GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        // 0 is failure; a length that doesn't fit is the size it would need.
        let Some(path) = buf.get(..n).filter(|_| n != 0 && n < buf.len()) else {
            log::debug!(target: "loadbearer::security", "GetSystemWindowsDirectoryW failed");
            return None;
        };
        Some(String::from_utf16_lossy(path))
    }

    pub fn firmware_type() -> Option<i32> {
        let mut t = 0i32;
        // SAFETY: `t` is a valid out-pointer for a FIRMWARE_TYPE (an i32).
        if unsafe { GetFirmwareType(&mut t) } == 0 {
            log::debug!(target: "loadbearer::security", "GetFirmwareType failed");
            return None;
        }
        Some(t)
    }

    /// `UEFISecureBootEnabled`, read-only. Absent on a BIOS-booted machine.
    pub fn secure_boot_state() -> Option<u32> {
        let key: Vec<u16> = "SYSTEM\\CurrentControlSet\\Control\\SecureBoot\\State\0"
            .encode_utf16()
            .collect();
        let value: Vec<u16> = "UEFISecureBootEnabled\0".encode_utf16().collect();
        let mut out = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: both names are NUL-terminated UTF-16 that outlive the call;
        // `out` is exactly the four bytes `size` says.
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
            // No error code: "not found" would say the machine booted
            // without UEFI, and the log carries readability, not values.
            log::debug!(target: "loadbearer::security", "UEFISecureBootEnabled not readable");
            return None;
        }
        Some(out)
    }

    /// `TPM_DEVICE_INFO` from tbs.h. The call fills all four; only
    /// `tpm_version` is kept.
    #[repr(C)]
    #[derive(Default)]
    #[allow(dead_code)]
    struct TpmDeviceInfo {
        struct_version: u32,
        tpm_version: u32,
        tpm_interface_type: u32,
        tpm_imp_revision: u32,
    }

    type GetDeviceInfo = unsafe extern "system" fn(u32, *mut c_void) -> u32;

    /// `Tbsi_GetDeviceInfo`'s result code and TPM version. tbs.dll is loaded
    /// here, from System32 only, rather than imported: a run that didn't ask
    /// for `security` never maps it. The interface type and implementation
    /// revision it also returns are not kept.
    pub fn tbs_device_info() -> Option<(u32, u32)> {
        use libloading::os::windows::{LOAD_LIBRARY_SEARCH_SYSTEM32, Library};
        // SAFETY: tbs.dll is a system DLL whose initialisation has no
        // preconditions on us.
        let lib = match unsafe { Library::load_with_flags("tbs.dll", LOAD_LIBRARY_SEARCH_SYSTEM32) }
        {
            Ok(lib) => lib,
            Err(_) => {
                log::debug!(target: "loadbearer::security", "tbs.dll not found");
                return None;
            }
        };
        // SAFETY: the signature is Tbsi_GetDeviceInfo's from tbs.h:
        // `TBS_RESULT WINAPI Tbsi_GetDeviceInfo(UINT32 Size, PVOID Info)`.
        let f = unsafe { lib.get::<GetDeviceInfo>(b"Tbsi_GetDeviceInfo\0") }.ok()?;
        let mut info = TpmDeviceInfo {
            // TPM_VERSION_20's structure version; the call fills it in.
            struct_version: 1,
            ..Default::default()
        };
        // SAFETY: `info` is a TPM_DEVICE_INFO and its exact size is passed.
        let rc = unsafe {
            f(
                std::mem::size_of::<TpmDeviceInfo>() as u32,
                &mut info as *mut TpmDeviceInfo as *mut c_void,
            )
        };
        // Not logged: TBS_E_TPM_NOT_FOUND would say there is no TPM.
        Some((rc, info.tpm_version))
    }
}

/// The one WMI read loadbearer makes (see the module docs): the system
/// volume's row of `Win32_EncryptableVolume`, four properties of it, on a
/// thread of its own, given [`TIMEOUT`] in all.
#[cfg(windows)]
mod wmi {
    use super::{EncryptableVolume, WmiFailure};
    use std::sync::mpsc;
    use std::time::Duration;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoSetProxyBlanket, CoUninitialize, EOAC_NONE, RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
        RPC_C_IMP_LEVEL_IMPERSONATE,
    };
    use windows::Win32::System::Variant::{VARIANT, VT_BSTR, VT_I4, VT_UI4, VariantClear};
    use windows::Win32::System::Wmi::{
        IEnumWbemClassObject, IWbemClassObject, IWbemLocator, WBEM_FLAG_CONNECT_USE_MAX_WAIT,
        WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE,
        WBEM_S_TIMEDOUT, WbemLocator,
    };
    use windows::core::{BSTR, HRESULT, IUnknown, PCWSTR, Param, w};

    /// How long the whole read may take before it is given up as absent.
    const TIMEOUT: Duration = Duration::from_secs(10);
    /// How long one `IEnumWbemClassObject::Next` may wait, in milliseconds.
    const NEXT_TIMEOUT_MS: i32 = 5_000;
    /// The query names one drive letter, so one row is expected; more than
    /// this is not the answer to the question asked.
    const MAX_ROWS: usize = 4;

    const NAMESPACE: &str = r"ROOT\CIMV2\Security\MicrosoftVolumeEncryption";
    /// `RPC_C_AUTHN_WINNT` and `RPC_C_AUTHZ_NONE` from rpcdce.h, as in
    /// Microsoft's WMI client examples.
    const RPC_C_AUTHN_WINNT: u32 = 10;
    const RPC_C_AUTHZ_NONE: u32 = 0;

    /// The rows for `letter:`, or why there are none. Returns within
    /// [`TIMEOUT`] whatever WMI does: a thread that hasn't answered by then is
    /// left to finish on its own (each of its calls is bounded, see
    /// [`read`]) and its answer, if it ever comes, is dropped.
    pub fn encryptable_volumes(letter: char) -> Result<Vec<EncryptableVolume>, WmiFailure> {
        let (tx, rx) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("loadbearer-wmi".into())
            .spawn(move || {
                // The receiver may be gone, after a timeout: nothing to do.
                let _ = tx.send(on_own_thread(letter));
            })
            .map_err(|_| "could not start the WMI thread".to_string())?;
        match rx.recv_timeout(TIMEOUT) {
            Ok(answer) => {
                // It has sent, so all that's left is returning: no wait.
                let _ = thread.join();
                answer
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Err("WMI did not answer in time".into()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("the WMI thread ended without an answer".into())
            }
        }
    }

    /// COM, initialised on this thread for as long as it lives.
    struct Com;

    impl Com {
        fn init() -> Result<Com, WmiFailure> {
            // SAFETY: a fresh thread, initialised once; S_FALSE (already
            // initialised) is success too and is balanced the same way.
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if hr.is_err() {
                return Err(format!("CoInitializeEx failed ({})", hex(hr)));
            }
            Ok(Com)
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            // SAFETY: balances the successful CoInitializeEx in `init`, on the
            // same thread; every interface pointer was released before this.
            unsafe { CoUninitialize() };
        }
    }

    fn on_own_thread(letter: char) -> Result<Vec<EncryptableVolume>, WmiFailure> {
        let _com = Com::init()?;
        // Every interface `read` obtains is a local of it, so all are
        // released before `_com` uninitialises COM.
        read(letter)
    }

    fn hex(hr: HRESULT) -> String {
        format!("{:#010x}", hr.0 as u32)
    }

    fn failed(what: &str, e: windows::core::Error) -> WmiFailure {
        format!("{what} failed ({})", hex(e.code()))
    }

    /// Ask for the blanket the BitLocker provider needs — an encrypted
    /// connection (`PKT_PRIVACY`), impersonation — on this proxy alone,
    /// rather than process-wide through `CoInitializeSecurity`.
    fn set_blanket(proxy: impl Param<IUnknown>, what: &str) -> Result<(), WmiFailure> {
        // SAFETY: `proxy` is a live interface pointer from this apartment;
        // no server principal name or auth identity is passed.
        unsafe {
            CoSetProxyBlanket(
                proxy,
                RPC_C_AUTHN_WINNT,
                RPC_C_AUTHZ_NONE,
                PCWSTR::null(),
                RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
                RPC_C_IMP_LEVEL_IMPERSONATE,
                None,
                EOAC_NONE,
            )
        }
        .map_err(|e| failed(what, e))
    }

    /// Connect, query, and read the four properties of each row. Every call
    /// is bounded: `ConnectServer` by `WBEM_FLAG_CONNECT_USE_MAX_WAIT`, the
    /// query returns at once (`WBEM_FLAG_RETURN_IMMEDIATELY`), and each
    /// `Next` waits at most [`NEXT_TIMEOUT_MS`].
    fn read(letter: char) -> Result<Vec<EncryptableVolume>, WmiFailure> {
        if !letter.is_ascii_alphabetic() {
            return Err("not a drive letter".into());
        }
        // SAFETY: COM is initialised on this thread (see `on_own_thread`).
        let locator: IWbemLocator =
            unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| failed("CoCreateInstance(WbemLocator)", e))?;
        let none = BSTR::new();
        // SAFETY: every BSTR outlives the call; no context object is passed.
        let services = unsafe {
            locator.ConnectServer(
                &BSTR::from(NAMESPACE),
                &none,
                &none,
                &none,
                WBEM_FLAG_CONNECT_USE_MAX_WAIT.0,
                &none,
                None,
            )
        }
        .map_err(|e| failed("ConnectServer(MicrosoftVolumeEncryption)", e))?;
        set_blanket(&services, "CoSetProxyBlanket(services)")?;

        // Only the four properties, and only the system volume's row: the
        // letter is a single ASCII letter, checked above, so it can't change
        // the query's shape.
        let query = format!(
            "SELECT DriveLetter, ConversionStatus, ProtectionStatus, EncryptionMethod \
             FROM Win32_EncryptableVolume WHERE DriveLetter = '{letter}:'"
        );
        let flags =
            WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0);
        // SAFETY: both BSTRs outlive the call; no context object is passed.
        let rows: IEnumWbemClassObject =
            unsafe { services.ExecQuery(&BSTR::from("WQL"), &BSTR::from(query), flags, None) }
                .map_err(|e| failed("ExecQuery(Win32_EncryptableVolume)", e))?;
        set_blanket(&rows, "CoSetProxyBlanket(enumerator)")?;

        let mut volumes = Vec::new();
        loop {
            let mut row: [Option<IWbemClassObject>; 1] = [None];
            let mut returned = 0u32;
            // SAFETY: `row` has room for the one object `Next` may return,
            // and `returned` is a valid out-pointer.
            let hr = unsafe { rows.Next(NEXT_TIMEOUT_MS, &mut row, &mut returned) };
            if hr == HRESULT(WBEM_S_TIMEDOUT.0) {
                return Err("Win32_EncryptableVolume enumeration timed out".into());
            }
            if hr.is_err() {
                return Err(format!(
                    "Win32_EncryptableVolume enumeration failed ({})",
                    hex(hr)
                ));
            }
            let [object] = row;
            match object {
                Some(object) if returned == 1 => volumes.push(volume(&object)),
                // WBEM_S_FALSE with nothing returned: the end.
                _ => break,
            }
            if volumes.len() > MAX_ROWS {
                return Err("more Win32_EncryptableVolume rows than asked for".into());
            }
        }
        Ok(volumes)
    }

    fn volume(object: &IWbemClassObject) -> EncryptableVolume {
        EncryptableVolume {
            drive_letter: property(object, w!("DriveLetter"), string_of),
            conversion_status: property(object, w!("ConversionStatus"), u32_of),
            protection_status: property(object, w!("ProtectionStatus"), u32_of),
            encryption_method: property(object, w!("EncryptionMethod"), u32_of),
        }
    }

    /// One property's value, through `decode`; `None` if `Get` fails.
    fn property<T>(
        object: &IWbemClassObject,
        name: PCWSTR,
        decode: fn(&VARIANT) -> Option<T>,
    ) -> Option<T> {
        let mut value = VARIANT::default();
        // SAFETY: `name` is a NUL-terminated literal; `value` is an empty
        // VARIANT for `Get` to fill; type and flavour aren't asked for.
        if unsafe { object.Get(name, 0, &mut value, None, None) }.is_err() {
            return None;
        }
        let decoded = decode(&value);
        // SAFETY: `value` was filled by `Get` and is cleared exactly once,
        // which frees a BSTR it holds.
        let _ = unsafe { VariantClear(&mut value) };
        decoded
    }

    /// A CIM `uint32` arrives as `VT_I4` (WMI's automation mapping), its bits
    /// unchanged — so `(uint32)-1` is `0xFFFFFFFF`. `VT_UI4` is taken too.
    fn u32_of(v: &VARIANT) -> Option<u32> {
        // SAFETY: `vt` says which union member is live; only that one is read.
        unsafe {
            let inner = &v.Anonymous.Anonymous;
            match inner.vt {
                VT_I4 => Some(inner.Anonymous.lVal as u32),
                VT_UI4 => Some(inner.Anonymous.ulVal),
                _ => None,
            }
        }
    }

    /// A string, or `None` for `VT_NULL` (a volume without a drive letter).
    fn string_of(v: &VARIANT) -> Option<String> {
        // SAFETY: as in `u32_of`; the BSTR is borrowed, not taken.
        unsafe {
            let inner = &v.Anonymous.Anonymous;
            (inner.vt == VT_BSTR).then(|| inner.Anonymous.bstrVal.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn platform_read() -> Security {
    sysfs::read(
        &sysfs::Real,
        cfg!(any(target_arch = "x86", target_arch = "x86_64")),
    )
}

/// The Linux reads, written against [`sysfs::Fs`] so the tests can hand them
/// a synthetic tree: no test reads the host.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod sysfs {
    use super::{DiskEncryption, Security, Tpm};

    pub const EFI: &str = "/sys/firmware/efi";
    pub const SECURE_BOOT_VAR: &str =
        "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";

    pub trait Fs {
        /// The file's bytes, or `None` if it's missing or unreadable.
        fn read(&self, path: &str) -> Option<Vec<u8>>;
        fn exists(&self, path: &str) -> bool;
        /// A directory's entry names, or `None` if it isn't one.
        fn list(&self, path: &str) -> Option<Vec<String>>;
        /// A device node's major and minor numbers.
        fn rdev(&self, path: &str) -> Option<(u32, u32)>;
    }

    #[cfg(target_os = "linux")]
    pub struct Real;

    #[cfg(target_os = "linux")]
    impl Fs for Real {
        fn read(&self, path: &str) -> Option<Vec<u8>> {
            std::fs::read(path).ok()
        }
        fn exists(&self, path: &str) -> bool {
            std::path::Path::new(path).exists()
        }
        fn list(&self, path: &str) -> Option<Vec<String>> {
            let entries = std::fs::read_dir(path).ok()?;
            Some(
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect(),
            )
        }
        fn rdev(&self, path: &str) -> Option<(u32, u32)> {
            use std::os::unix::fs::MetadataExt;
            Some(split_dev(std::fs::metadata(path).ok()?.rdev()))
        }
    }

    /// glibc's `major()` / `minor()` encoding of a `dev_t`.
    pub fn split_dev(dev: u64) -> (u32, u32) {
        let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
        let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
        (major as u32, minor as u32)
    }

    pub fn read(fs: &impl Fs, x86: bool) -> Security {
        let firmware_mode = firmware_mode(fs, x86);
        let secure_boot = if firmware_mode.as_deref() == Some("uefi") {
            fs.read(SECURE_BOOT_VAR).and_then(|b| parse_secure_boot(&b))
        } else {
            None
        };
        Security {
            firmware_mode,
            secure_boot,
            tpm: tpm(fs),
            disk_encryption: disk_encryption(fs),
        }
    }

    /// `/sys/firmware/efi` exists exactly when the kernel was booted through
    /// UEFI. Its absence means BIOS only on x86, where that's the
    /// alternative; elsewhere (U-Boot, device tree) it's left unsaid. Without
    /// `/sys/firmware` at all — a container, sysfs not mounted — nothing is.
    pub fn firmware_mode(fs: &impl Fs, x86: bool) -> Option<String> {
        if fs.exists(EFI) {
            Some("uefi".into())
        } else if x86 && fs.exists("/sys/firmware") {
            Some("bios".into())
        } else {
            None
        }
    }

    /// An efivarfs file is the variable's 4 attribute bytes, then its data;
    /// `SecureBoot`'s data is one byte, 1 when enforcing.
    pub fn parse_secure_boot(bytes: &[u8]) -> Option<bool> {
        match bytes.get(4)? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }

    /// `tpm_version_major` (kernel 5.6+, "1" or "2"); before that a
    /// `tpmrm0` resource manager exists only for a TPM 2.0. A `tpm0` with
    /// neither is present with no version.
    pub fn tpm(fs: &impl Fs) -> Option<Tpm> {
        if fs.exists("/sys/class/tpm/tpm0") {
            let major = fs
                .read("/sys/class/tpm/tpm0/tpm_version_major")
                .map(|b| String::from_utf8_lossy(&b).trim().to_string());
            let version = match major.as_deref() {
                Some("2") => Some("2.0".to_string()),
                Some("1") => Some("1.2".to_string()),
                _ if fs.exists("/sys/class/tpmrm/tpmrm0") => Some("2.0".to_string()),
                _ => None,
            };
            Some(Tpm {
                present: true,
                version,
            })
        } else if fs.exists("/sys/class") {
            Some(Tpm {
                present: false,
                version: None,
            })
        } else {
            None
        }
    }

    /// The root filesystem's device: from the last `/` mount in mountinfo
    /// (a later mount hides an earlier one), its `major:minor`, or for a
    /// filesystem on an anonymous device (btrfs: major 0) its source node.
    pub fn root_device(fs: &impl Fs) -> Option<(u32, u32)> {
        let text = fs.read("/proc/self/mountinfo")?;
        let (dev, source) = parse_root_mount(&String::from_utf8_lossy(&text))?;
        if dev.0 != 0 {
            return Some(dev);
        }
        source
            .filter(|s| s.starts_with("/dev/"))
            .and_then(|s| fs.rdev(&s))
            .filter(|d| d.0 != 0)
    }

    /// From proc(5): `id parent major:minor root mountpoint opts [tags] -
    /// fstype source superopts`. Returns the `major:minor` and source of the
    /// last line mounted at `/`.
    pub fn parse_root_mount(text: &str) -> Option<((u32, u32), Option<String>)> {
        let mut found = None;
        for line in text.lines() {
            let fields: Vec<&str> = line.split(' ').collect();
            if fields.len() < 5 || fields[4] != "/" {
                continue;
            }
            let Some((maj, min)) = fields[2].split_once(':') else {
                continue;
            };
            let (Ok(maj), Ok(min)) = (maj.parse(), min.parse()) else {
                continue;
            };
            let source = fields
                .iter()
                .position(|f| *f == "-")
                .and_then(|i| fields.get(i + 2))
                .map(|s| s.to_string());
            found = Some(((maj, min), source));
        }
        found
    }

    /// The cryptsetup types that mean encryption. VERITY and INTEGRITY are
    /// also `CRYPT-` mappings and are not.
    fn crypt_method(uuid: &str) -> Option<&'static str> {
        let rest = uuid.trim().strip_prefix("CRYPT-")?;
        let kind = rest.split('-').next()?;
        Some(match kind {
            "LUKS1" => "luks1",
            "LUKS2" => "luks2",
            "PLAIN" => "plain",
            "LOOPAES" => "loopaes",
            "TCRYPT" => "tcrypt",
            "BITLK" => "bitlk",
            "FVAULT2" => "fvault2",
            _ => return None,
        })
    }

    #[derive(Debug, PartialEq)]
    enum Walk {
        Crypt(&'static str),
        NoCrypt,
        Unknown,
    }

    /// Down from `dir` (a `/sys/class/block/<dev>`-style directory) through
    /// `slaves/`: LVM on LUKS is an LVM mapping whose slave is the LUKS one;
    /// LUKS on LVM is the LUKS mapping itself. Depth-bounded, since sysfs is
    /// not trusted to be acyclic.
    fn walk(fs: &impl Fs, dir: &str, depth: u32) -> Walk {
        if depth > 8 || !fs.exists(dir) {
            return Walk::Unknown;
        }
        if fs.exists(&format!("{dir}/dm")) {
            let Some(uuid) = fs.read(&format!("{dir}/dm/uuid")) else {
                return Walk::Unknown;
            };
            if let Some(m) = crypt_method(&String::from_utf8_lossy(&uuid)) {
                return Walk::Crypt(m);
            }
        }
        let Some(slaves) = fs.list(&format!("{dir}/slaves")) else {
            return Walk::NoCrypt;
        };
        let mut result = Walk::NoCrypt;
        for name in slaves {
            match walk(fs, &format!("/sys/class/block/{name}"), depth + 1) {
                found @ Walk::Crypt(_) => return found,
                Walk::Unknown => result = Walk::Unknown,
                Walk::NoCrypt => {}
            }
        }
        result
    }

    pub fn disk_encryption(fs: &impl Fs) -> Option<DiskEncryption> {
        let (maj, min) = root_device(fs)?;
        match walk(fs, &format!("/sys/dev/block/{maj}:{min}"), 0) {
            Walk::Crypt(m) => Some(DiskEncryption {
                system_volume: "encrypted".into(),
                method: Some(m.into()),
                ..Default::default()
            }),
            Walk::NoCrypt => Some(DiskEncryption {
                system_volume: "none_detected".into(),
                ..Default::default()
            }),
            Walk::Unknown => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Anywhere else
// ---------------------------------------------------------------------------

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_read() -> Security {
    Security::default()
}

#[cfg(test)]
mod tests {
    use super::sysfs::{self, Fs};
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// A synthetic sysfs: files with contents, files that exist but can't be
    /// read, directories with their listings, and device nodes.
    #[derive(Default)]
    struct Tree {
        files: HashMap<String, Vec<u8>>,
        unreadable: HashSet<String>,
        dirs: HashMap<String, Vec<String>>,
        nodes: HashMap<String, (u32, u32)>,
    }

    impl Tree {
        fn file(mut self, p: &str, b: &[u8]) -> Self {
            self.files.insert(p.into(), b.to_vec());
            self
        }
        fn locked(mut self, p: &str) -> Self {
            self.unreadable.insert(p.into());
            self
        }
        fn dir(mut self, p: &str, entries: &[&str]) -> Self {
            self.dirs
                .insert(p.into(), entries.iter().map(|s| s.to_string()).collect());
            self
        }
        fn node(mut self, p: &str, dev: (u32, u32)) -> Self {
            self.nodes.insert(p.into(), dev);
            self
        }
        /// A device-mapper device: its `dm/uuid` and its slaves.
        fn dm(self, dir: &str, uuid: &str, slaves: &[&str]) -> Self {
            self.dir(dir, &["dm", "slaves"])
                .dir(&format!("{dir}/dm"), &["uuid"])
                .file(&format!("{dir}/dm/uuid"), uuid.as_bytes())
                .dir(&format!("{dir}/slaves"), slaves)
        }
        fn mountinfo(self, root_line: &str) -> Self {
            let text = format!(
                "22 1 0:21 / /proc rw - proc proc rw\n{root_line}\n\
                 30 29 0:25 / /run rw - tmpfs tmpfs rw\n"
            );
            self.file("/proc/self/mountinfo", text.as_bytes())
        }
    }

    impl Fs for Tree {
        fn read(&self, p: &str) -> Option<Vec<u8>> {
            if self.unreadable.contains(p) {
                return None;
            }
            self.files.get(p).cloned()
        }
        fn exists(&self, p: &str) -> bool {
            self.files.contains_key(p)
                || self.unreadable.contains(p)
                || self.dirs.contains_key(p)
                || self.nodes.contains_key(p)
        }
        fn list(&self, p: &str) -> Option<Vec<String>> {
            self.dirs.get(p).cloned()
        }
        fn rdev(&self, p: &str) -> Option<(u32, u32)> {
            self.nodes.get(p).copied()
        }
    }

    const ROOT_EXT4_DM1: &str = "29 1 253:1 / / rw,relatime - ext4 /dev/mapper/vg-root rw";

    fn uefi() -> Tree {
        Tree::default()
            .dir("/sys/firmware", &["efi"])
            .dir(sysfs::EFI, &["efivars"])
            .dir("/sys/class", &[])
    }

    // --- Windows decoders (pure, tested everywhere) ---

    #[test]
    fn windows_values_decode() {
        assert_eq!(firmware_mode_from_type(2).as_deref(), Some("uefi"));
        assert_eq!(firmware_mode_from_type(1).as_deref(), Some("bios"));
        assert_eq!(firmware_mode_from_type(0), None);
        assert_eq!(secure_boot_from_dword(Some(1)), Some(true));
        assert_eq!(secure_boot_from_dword(Some(0)), Some(false));
        assert_eq!(secure_boot_from_dword(Some(7)), None);
        // A BIOS-booted machine has no such value: absent, not false.
        assert_eq!(secure_boot_from_dword(None), None);
    }

    #[test]
    fn tbs_results_decode() {
        let v2 = tpm_from_tbs(TBS_SUCCESS, 2).unwrap();
        assert!(v2.present);
        assert_eq!(v2.version.as_deref(), Some("2.0"));
        assert_eq!(
            tpm_from_tbs(TBS_SUCCESS, 1).unwrap().version.as_deref(),
            Some("1.2")
        );
        assert_eq!(tpm_from_tbs(TBS_SUCCESS, 9).unwrap().version, None);
        assert_eq!(
            tpm_from_tbs(TBS_E_TPM_NOT_FOUND, 0),
            Some(Tpm {
                present: false,
                version: None
            })
        );
        // TBS_E_SERVICE_NOT_RUNNING: can't tell, so no part at all.
        assert_eq!(tpm_from_tbs(0x8028_4008, 0), None);
    }

    // --- Linux, on synthetic trees ---

    #[test]
    fn firmware_mode_and_secure_boot() {
        let on = uefi().file(sysfs::SECURE_BOOT_VAR, &[0x06, 0, 0, 0, 1]);
        let s = sysfs::read(&on, true);
        assert_eq!(s.firmware_mode.as_deref(), Some("uefi"));
        assert_eq!(s.secure_boot, Some(true));

        let off = uefi().file(sysfs::SECURE_BOOT_VAR, &[0x06, 0, 0, 0, 0]);
        assert_eq!(sysfs::read(&off, true).secure_boot, Some(false));

        // UEFI, but the variable is missing, unreadable or short.
        assert_eq!(sysfs::read(&uefi(), true).secure_boot, None);
        let locked = uefi().locked(sysfs::SECURE_BOOT_VAR);
        assert_eq!(sysfs::read(&locked, true).secure_boot, None);
        let short = uefi().file(sysfs::SECURE_BOOT_VAR, &[0x06, 0, 0, 0]);
        assert_eq!(sysfs::read(&short, true).secure_boot, None);
        assert_eq!(sysfs::parse_secure_boot(&[6, 0, 0, 0, 2]), None);
    }

    #[test]
    fn no_efi_directory_is_bios_on_x86_only() {
        let bios = Tree::default()
            .dir("/sys/firmware", &["acpi"])
            .dir("/sys/class", &[]);
        let s = sysfs::read(&bios, true);
        assert_eq!(s.firmware_mode.as_deref(), Some("bios"));
        assert_eq!(s.secure_boot, None);
        assert_eq!(sysfs::read(&bios, false).firmware_mode, None);
        // No sysfs at all: nothing is said.
        assert!(sysfs::read(&Tree::default(), true).is_empty());
    }

    #[test]
    fn tpm_versions() {
        let tpm2 = Tree::default()
            .dir("/sys/class/tpm/tpm0", &["tpm_version_major"])
            .file("/sys/class/tpm/tpm0/tpm_version_major", b"2\n");
        assert_eq!(
            sysfs::tpm(&tpm2),
            Some(Tpm {
                present: true,
                version: Some("2.0".into())
            })
        );
        let tpm12 = Tree::default()
            .dir("/sys/class/tpm/tpm0", &["tpm_version_major"])
            .file("/sys/class/tpm/tpm0/tpm_version_major", b"1\n");
        assert_eq!(sysfs::tpm(&tpm12).unwrap().version.as_deref(), Some("1.2"));
        // An older kernel: no version file, but a resource manager means 2.0.
        let old2 = Tree::default()
            .dir("/sys/class/tpm/tpm0", &[])
            .dir("/sys/class/tpmrm/tpmrm0", &[]);
        assert_eq!(sysfs::tpm(&old2).unwrap().version.as_deref(), Some("2.0"));
        // An older kernel with a 1.2: present, version unsaid.
        let old12 = Tree::default().dir("/sys/class/tpm/tpm0", &[]);
        assert_eq!(
            sysfs::tpm(&old12),
            Some(Tpm {
                present: true,
                version: None
            })
        );
        // An unreadable version file is the same.
        let locked = Tree::default()
            .dir("/sys/class/tpm/tpm0", &[])
            .locked("/sys/class/tpm/tpm0/tpm_version_major");
        assert_eq!(sysfs::tpm(&locked).unwrap().version, None);
        // No TPM device; and no sysfs at all.
        let none = Tree::default().dir("/sys/class", &["block"]);
        assert_eq!(sysfs::tpm(&none).map(|t| t.present), Some(false));
        assert_eq!(sysfs::tpm(&Tree::default()), None);
    }

    #[test]
    fn lvm_on_luks() {
        // / on an LVM LV (dm-1), whose physical volume is a LUKS2 mapping
        // (dm-0) on a partition.
        let t = Tree::default()
            .mountinfo(ROOT_EXT4_DM1)
            .dm("/sys/dev/block/253:1", "LVM-abcdef0123", &["dm-0"])
            .dm(
                "/sys/class/block/dm-0",
                "CRYPT-LUKS2-0123456789abcdef0123456789abcdef-luks-root",
                &["nvme0n1p3"],
            )
            .dir("/sys/class/block/nvme0n1p3", &["partition"]);
        assert_eq!(
            sysfs::disk_encryption(&t),
            Some(DiskEncryption {
                system_volume: "encrypted".into(),
                method: Some("luks2".into()),
                ..Default::default()
            })
        );
    }

    #[test]
    fn luks_on_lvm() {
        // / directly on a LUKS1 mapping whose backing device is an LV.
        let t = Tree::default()
            .mountinfo(ROOT_EXT4_DM1)
            .dm(
                "/sys/dev/block/253:1",
                "CRYPT-LUKS1-0123456789abcdef0123456789abcdef-root_crypt",
                &["dm-0"],
            )
            .dm("/sys/class/block/dm-0", "LVM-abcdef", &["sda2"])
            .dir("/sys/class/block/sda2", &[]);
        assert_eq!(
            sysfs::disk_encryption(&t).unwrap().method.as_deref(),
            Some("luks1")
        );
    }

    #[test]
    fn plain_partition_root_is_none_detected() {
        let t = Tree::default()
            .mountinfo("29 1 259:2 / / rw - ext4 /dev/nvme0n1p2 rw")
            .dir("/sys/dev/block/259:2", &["partition"]);
        assert_eq!(
            sysfs::disk_encryption(&t),
            Some(DiskEncryption {
                system_volume: "none_detected".into(),
                ..Default::default()
            })
        );
    }

    #[test]
    fn integrity_and_verity_are_not_encryption() {
        let t = Tree::default()
            .mountinfo(ROOT_EXT4_DM1)
            .dm("/sys/dev/block/253:1", "CRYPT-VERITY-abc-root", &["sda1"])
            .dir("/sys/class/block/sda1", &[]);
        assert_eq!(
            sysfs::disk_encryption(&t).unwrap().system_volume,
            "none_detected"
        );
    }

    #[test]
    fn btrfs_root_is_followed_through_its_source_node() {
        // btrfs reports an anonymous 0:N device; the source names the node.
        let t = Tree::default()
            .mountinfo("29 1 0:30 /@ / rw - btrfs /dev/mapper/luks-abc rw,subvol=/@")
            .node("/dev/mapper/luks-abc", (253, 0))
            .dm("/sys/dev/block/253:0", "CRYPT-PLAIN-luks-abc", &["sda2"])
            .dir("/sys/class/block/sda2", &[]);
        assert_eq!(
            sysfs::disk_encryption(&t).unwrap().method.as_deref(),
            Some("plain")
        );
        // overlay in a container: anonymous, no /dev source — can't tell.
        let overlay =
            Tree::default().mountinfo("29 1 0:30 / / rw - overlay overlay rw,lowerdir=/x");
        assert_eq!(sysfs::disk_encryption(&overlay), None);
    }

    #[test]
    fn what_cannot_be_read_is_absent_not_none_detected() {
        // No mountinfo.
        assert_eq!(sysfs::disk_encryption(&Tree::default()), None);
        // mountinfo unreadable.
        let locked = Tree::default().locked("/proc/self/mountinfo");
        assert_eq!(sysfs::disk_encryption(&locked), None);
        // The device's sysfs directory is missing.
        let missing = Tree::default().mountinfo(ROOT_EXT4_DM1);
        assert_eq!(sysfs::disk_encryption(&missing), None);
        // A dm device whose uuid can't be read.
        let t = Tree::default()
            .mountinfo(ROOT_EXT4_DM1)
            .dir("/sys/dev/block/253:1", &["dm"])
            .dir("/sys/dev/block/253:1/dm", &["uuid"])
            .locked("/sys/dev/block/253:1/dm/uuid");
        assert_eq!(sysfs::disk_encryption(&t), None);
        // A slave chain that breaks part-way.
        let broken =
            Tree::default()
                .mountinfo(ROOT_EXT4_DM1)
                .dm("/sys/dev/block/253:1", "LVM-x", &["dm-9"]);
        assert_eq!(sysfs::disk_encryption(&broken), None);
        // No line for / at all.
        let no_root = Tree::default().file(
            "/proc/self/mountinfo",
            b"22 1 0:21 / /proc rw - proc proc rw\n",
        );
        assert_eq!(sysfs::disk_encryption(&no_root), None);
    }

    #[test]
    fn a_later_root_mount_wins() {
        let text = "1 0 8:1 / / rw - ext4 /dev/sda1 rw\n2 1 253:3 / / rw - ext4 /dev/dm-3 rw\n";
        assert_eq!(
            sysfs::parse_root_mount(text),
            Some(((253, 3), Some("/dev/dm-3".into())))
        );
        assert_eq!(sysfs::parse_root_mount("garbage\n"), None);
    }

    #[test]
    fn dev_t_split_matches_glibc() {
        // makedev(253, 1) and makedev(259, 70000).
        assert_eq!(sysfs::split_dev((253 << 8) | 1), (253, 1));
        let dev = ((70000u64 & !0xff) << 12) | (259 << 8) | (70000 & 0xff);
        assert_eq!(sysfs::split_dev(dev), (259, 70000));
    }

    #[test]
    fn json_shape_and_summaries() {
        let s = Security {
            firmware_mode: Some("uefi".into()),
            secure_boot: Some(true),
            tpm: Some(Tpm {
                present: true,
                version: Some("2.0".into()),
            }),
            disk_encryption: Some(DiskEncryption {
                system_volume: "encrypted".into(),
                method: Some("luks2".into()),
                ..Default::default()
            }),
        };
        // A Linux document is exactly what it was before `cipher` and
        // `protection` existed.
        assert_eq!(
            serde_json::to_string(&s.disk_encryption).unwrap(),
            r#"{"system_volume":"encrypted","method":"luks2"}"#
        );
        let none = DiskEncryption {
            system_volume: "none_detected".into(),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&none).unwrap(),
            r#"{"system_volume":"none_detected"}"#
        );
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["firmware_mode"], "uefi");
        assert_eq!(v["secure_boot"], true);
        assert_eq!(v["tpm"]["present"], true);
        assert_eq!(v["tpm"]["version"], "2.0");
        assert_eq!(v["disk_encryption"]["system_volume"], "encrypted");
        assert_eq!(v["disk_encryption"]["method"], "luks2");
        assert_eq!(
            s.summary().as_deref(),
            Some("UEFI · Secure Boot on · TPM 2.0 · root on LUKS2")
        );
        let partial = Security {
            tpm: Some(Tpm::default()),
            ..Default::default()
        };
        let text = serde_json::to_string(&partial).unwrap();
        assert_eq!(text, r#"{"tpm":{"present":false}}"#);
        assert_eq!(partial.summary().as_deref(), Some("TPM none"));
        assert!(Security::default().summary().is_none());
    }

    // --- BitLocker (pure, tested everywhere) ---

    fn vol(letter: Option<&str>, cs: u32, ps: u32, em: u32) -> EncryptableVolume {
        EncryptableVolume {
            drive_letter: letter.map(Into::into),
            conversion_status: Some(cs),
            protection_status: Some(ps),
            encryption_method: Some(em),
        }
    }

    #[test]
    fn every_documented_conversion_status() {
        assert_eq!(system_volume_from_conversion(0), Some("not_encrypted"));
        assert_eq!(system_volume_from_conversion(1), Some("encrypted"));
        // In progress or paused, either way: partly encrypted.
        for s in 2..=5 {
            assert_eq!(system_volume_from_conversion(s), Some("partly_encrypted"));
        }
        for s in [6, 100, u32::MAX] {
            assert_eq!(system_volume_from_conversion(s), None);
        }
    }

    #[test]
    fn every_documented_encryption_method() {
        let table = [
            (1, "aes-128-diffuser"),
            (2, "aes-256-diffuser"),
            (3, "aes-128"),
            (4, "aes-256"),
            (5, "hardware"),
            (6, "xts-aes-128"),
            (7, "xts-aes-256"),
        ];
        for (n, slug) in table {
            assert_eq!(cipher_from_method(n), Some(slug));
        }
        // None (0), UNKNOWN ((uint32)-1) and anything unlisted.
        for n in [0, 8, 99, u32::MAX] {
            assert_eq!(cipher_from_method(n), None);
        }
        assert_eq!(cipher_name("xts-aes-256"), "XTS-AES 256");
        assert_eq!(cipher_name("aes-128-diffuser"), "AES 128 with diffuser");
        assert_eq!(cipher_name("hardware"), "hardware encryption");
        // From a later version: shown as it is.
        assert_eq!(cipher_name("aes-512"), "aes-512");
    }

    #[test]
    fn every_documented_protection_status() {
        assert_eq!(protection_from_status(0), Some("off"));
        assert_eq!(protection_from_status(1), Some("on"));
        // 2 is "unknown": no answer, like anything unlisted.
        assert_eq!(protection_from_status(2), None);
        assert_eq!(protection_from_status(3), None);
    }

    #[test]
    fn a_volume_decodes() {
        // As observed on a fully encrypted XTS-AES 256 OS volume.
        assert_eq!(
            bitlocker_from_volume(&vol(Some("C:"), 1, 1, 7)),
            Some(DiskEncryption {
                system_volume: "encrypted".into(),
                method: Some("bitlocker".into()),
                cipher: Some("xts-aes-256".into()),
                protection: Some("on".into()),
            })
        );
        // Encrypted with protection suspended.
        let suspended = bitlocker_from_volume(&vol(Some("C:"), 1, 0, 6)).unwrap();
        assert_eq!(suspended.protection.as_deref(), Some("off"));
        assert_eq!(suspended.cipher.as_deref(), Some("xts-aes-128"));
        // Encryption paused part-way.
        let paused = bitlocker_from_volume(&vol(Some("C:"), 4, 0, 4)).unwrap();
        assert_eq!(paused.system_volume, "partly_encrypted");
        assert_eq!(paused.method.as_deref(), Some("bitlocker"));
        // Fully decrypted: no method, no cipher, protection off.
        assert_eq!(
            bitlocker_from_volume(&vol(Some("C:"), 0, 0, 0)),
            Some(DiskEncryption {
                system_volume: "not_encrypted".into(),
                protection: Some("off".into()),
                ..Default::default()
            })
        );
        // An unknown method or protection drops just that field.
        let unknown = bitlocker_from_volume(&vol(Some("C:"), 1, 2, u32::MAX)).unwrap();
        assert_eq!((unknown.cipher, unknown.protection), (None, None));
        assert_eq!(unknown.system_volume, "encrypted");
        // An undocumented or missing ConversionStatus drops the whole part.
        assert_eq!(bitlocker_from_volume(&vol(Some("C:"), 9, 1, 7)), None);
        let missing = EncryptableVolume {
            conversion_status: None,
            ..vol(Some("C:"), 1, 1, 7)
        };
        assert_eq!(bitlocker_from_volume(&missing), None);
    }

    #[test]
    fn the_system_volume_is_found_by_drive_letter() {
        assert_eq!(drive_letter_of(r"C:\WINDOWS"), Some('C'));
        assert_eq!(drive_letter_of(r"d:\Windows"), Some('D'));
        assert_eq!(drive_letter_of(r"\\?\C:\Windows"), None);
        assert_eq!(drive_letter_of(r"\\server\share"), None);
        assert_eq!(drive_letter_of(""), None);
        assert_eq!(drive_letter_of("1:"), None);

        let vols = [
            vol(None, 0, 0, 0),
            vol(Some("D:"), 0, 0, 0),
            vol(Some("c:"), 1, 1, 7),
        ];
        assert!(system_volume(&vols, 'C').is_some_and(|v| v.encryption_method == Some(7)));
        assert!(system_volume(&vols, 'E').is_none());
        // Two rows for one letter: no answer rather than a pick.
        let twice = [vol(Some("C:"), 1, 1, 7), vol(Some("C:"), 0, 0, 0)];
        assert!(system_volume(&twice, 'C').is_none());
        assert!(system_volume(&[], 'C').is_none());
    }

    #[test]
    fn bitlocker_is_asked_for_only_when_elevated() {
        let asked = std::cell::Cell::new(false);
        let query = |_: char| {
            asked.set(true);
            Ok(vec![vol(Some("C:"), 1, 1, 7)])
        };
        for elevated in [Some(false), None] {
            assert_eq!(bitlocker_state(elevated, Some(r"C:\Windows"), query), None);
            assert!(!asked.get(), "WMI was queried without elevation");
        }
        // No drive letter to ask about: not asked either.
        assert_eq!(bitlocker_state(Some(true), None, query), None);
        assert!(!asked.get());

        let got = bitlocker_state(Some(true), Some(r"C:\Windows"), query);
        assert!(asked.get());
        assert_eq!(got.unwrap().system_volume, "encrypted");
        // The letter asked about is the Windows directory's.
        let letter = std::cell::Cell::new(' ');
        let _ = bitlocker_state(Some(true), Some(r"e:\Windows"), |l| {
            letter.set(l);
            Ok(vec![])
        });
        assert_eq!(letter.get(), 'E');
    }

    #[test]
    fn a_failed_or_empty_query_is_absent() {
        let win = Some(r"C:\Windows");
        assert_eq!(
            bitlocker_state(Some(true), win, |_| Err("timed out".into())),
            None
        );
        assert_eq!(bitlocker_state(Some(true), win, |_| Ok(vec![])), None);
        let other = |_: char| Ok(vec![vol(Some("D:"), 1, 1, 7)]);
        assert_eq!(bitlocker_state(Some(true), win, other), None);
    }

    #[test]
    fn bitlocker_json_shape_and_summaries() {
        let enc = bitlocker_from_volume(&vol(Some("C:"), 1, 1, 7)).unwrap();
        assert_eq!(
            serde_json::to_string(&enc).unwrap(),
            r#"{"system_volume":"encrypted","method":"bitlocker","cipher":"xts-aes-256","protection":"on"}"#
        );
        assert_eq!(
            enc.summary(),
            "encrypted (BitLocker, XTS-AES 256, protection on)"
        );
        let suspended = bitlocker_from_volume(&vol(Some("C:"), 1, 0, 5)).unwrap();
        assert_eq!(
            suspended.summary(),
            "encrypted (BitLocker, hardware encryption, protection off)"
        );
        let paused = bitlocker_from_volume(&vol(Some("C:"), 5, 2, u32::MAX)).unwrap();
        assert_eq!(paused.summary(), "partly encrypted (BitLocker)");
        let off = bitlocker_from_volume(&vol(Some("C:"), 0, 0, 0)).unwrap();
        assert_eq!(
            serde_json::to_string(&off).unwrap(),
            r#"{"system_volume":"not_encrypted","protection":"off"}"#
        );
        assert_eq!(off.summary(), "not encrypted");

        let s = Security {
            firmware_mode: Some("uefi".into()),
            disk_encryption: Some(enc.clone()),
            ..Default::default()
        };
        assert_eq!(
            s.summary().as_deref(),
            Some("UEFI · encrypted (BitLocker, XTS-AES 256, protection on)")
        );
        let row = s.rows().into_iter().find(|(k, _)| *k == "Encryption");
        assert_eq!(
            row.map(|(_, v)| v).as_deref(),
            Some("encrypted (BitLocker, XTS-AES 256, protection on)")
        );
        // Round trip, and a document from before the new fields still parses.
        let back: DiskEncryption =
            serde_json::from_value(serde_json::to_value(&enc).unwrap()).unwrap();
        assert_eq!(back, enc);
        let old: DiskEncryption =
            serde_json::from_str(r#"{"system_volume":"encrypted","method":"luks2"}"#).unwrap();
        assert_eq!((old.cipher, old.protection), (None, None));
    }

    #[test]
    fn unread_encryption_wording() {
        let row = Security::default()
            .rows()
            .into_iter()
            .find(|(k, _)| *k == "Encryption")
            .map(|(_, v)| v);
        let want = if cfg!(windows) {
            "unknown (BitLocker is read only when elevated)"
        } else {
            "unknown"
        };
        assert_eq!(row.as_deref(), Some(want));
    }
}
