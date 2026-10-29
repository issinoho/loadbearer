//! The power settings in force when the machine was measured: whether it was
//! on mains or battery, and the plan / mode / profile that decides how hard
//! the CPU is allowed to run.
//!
//! Windows keeps a separate value for every setting when plugged in and on
//! battery, so both are reported — the plan's processor limits and boost
//! policy, and the Windows 11 *power mode* (Best power efficiency / Balanced /
//! Best performance). Linux has no such split: what's reported is the state
//! in force now — the ACPI platform profile (which power-profiles-daemon and
//! tuned-ppd set), the cpufreq governor and energy-performance preference, and
//! whether turbo is enabled.
//!
//! None of it needs elevation. Vendor tools that apply their own limits (Dell
//! Optimizer's thermal modes, Lenovo's Intelligent Cooling) don't show up in
//! either OS's settings, so a machine can be capped without anything here
//! saying so. Not graded.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Power {
    /// `"mains"` or `"battery"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Windows' energy saver (battery saver) is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_saver: Option<bool>,
    /// The active power plan's name, e.g. "Balanced" (Windows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// The settings that apply when plugged in (Windows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugged_in: Option<ModeSettings>,
    /// The settings that apply on battery (Windows, on a machine with one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_battery: Option<ModeSettings>,
    /// The ACPI platform profile, e.g. `"balanced"`, `"low-power"` (Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_profile: Option<String>,
    /// The first CPU's cpufreq governor, e.g. `"powersave"` (Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governor: Option<String>,
    /// The first CPU's energy-performance preference, e.g.
    /// `"balance_performance"` (Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_preference: Option<String>,
    /// Whether turbo / boost clocks are enabled (Linux).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turbo: Option<bool>,
}

/// One column of a Windows power plan: plugged in, or on battery.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModeSettings {
    /// The Windows 11 power mode: `"best power efficiency"`, `"balanced"` or
    /// `"best performance"`. Only the Balanced plan has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub power_mode: Option<String>,
    /// Minimum processor state, percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_processor_pct: Option<u32>,
    /// Maximum processor state, percent. Below 100 caps the clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_processor_pct: Option<u32>,
    /// Processor performance boost mode, e.g. `"aggressive"`, `"disabled"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost: Option<String>,
}

impl ModeSettings {
    /// "Balanced mode · processor 5–100% · boost aggressive".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(m) = &self.power_mode {
            parts.push(format!("{} mode", capitalise(m)));
        }
        match (self.min_processor_pct, self.max_processor_pct) {
            (Some(lo), Some(hi)) => parts.push(format!("processor {lo}–{hi}%")),
            (None, Some(hi)) => parts.push(format!("processor up to {hi}%")),
            _ => {}
        }
        if let Some(b) = &self.boost {
            parts.push(format!("boost {b}"));
        }
        parts.join(" · ")
    }

    /// Settings in this column that hold the CPU back.
    fn limits(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(hi) = self.max_processor_pct.filter(|&p| p < 100) {
            out.push(format!("maximum processor state {hi}%"));
        }
        if self.boost.as_deref() == Some("disabled") {
            out.push("boost disabled".to_string());
        }
        if self.power_mode.as_deref() == Some("best power efficiency") {
            out.push("Best power efficiency mode".to_string());
        }
        out
    }
}

impl Power {
    pub fn is_empty(&self) -> bool {
        *self == Power::default()
    }

    pub fn on_battery_now(&self) -> bool {
        self.source.as_deref() == Some("battery")
    }

    /// The settings column in force now: on battery if that's what the
    /// machine is running on, plugged in otherwise.
    fn current(&self) -> Option<&ModeSettings> {
        if self.on_battery_now() {
            self.on_battery.as_ref()
        } else {
            self.plugged_in.as_ref()
        }
    }

    /// What, in the settings in force now, would hold a benchmark back —
    /// empty when nothing does.
    pub fn limits(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.on_battery_now() {
            out.push("running on battery".to_string());
        }
        if self.energy_saver == Some(true) {
            out.push("energy saver on".to_string());
        }
        if let Some(m) = self.current() {
            out.extend(m.limits());
        }
        if self.platform_profile.as_deref().is_some_and(saver_profile) {
            out.push(format!(
                "{} platform profile",
                self.platform_profile.as_deref().unwrap_or_default()
            ));
        }
        if self.governor.as_deref() == Some("powersave") && self.energy_preference.is_none() {
            // With intel_pstate / amd-pstate in active mode "powersave" is the
            // normal governor and the preference does the work; it only means
            // "lowest clock" under the generic cpufreq driver, which has none.
            out.push("powersave governor".to_string());
        }
        if self.energy_preference.as_deref() == Some("power") {
            out.push("energy preference \"power\"".to_string());
        }
        if self.turbo == Some(false) {
            out.push("turbo disabled".to_string());
        }
        out
    }
}

fn saver_profile(p: &str) -> bool {
    matches!(p, "low-power" | "quiet" | "cool")
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Collect what this platform will tell us, or `None` if that's nothing.
pub fn collect() -> Option<Power> {
    let p = platform_collect();
    log::debug!(target: "loadbearer::power", "{p:?}");
    if p.is_empty() { None } else { Some(p) }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_collect() -> Power {
    Power::default()
}

/// Processor performance boost mode, by the value index Windows stores.
#[cfg_attr(not(windows), allow(dead_code))]
fn boost_mode(index: u32) -> String {
    match index {
        0 => "disabled",
        1 => "enabled",
        2 => "aggressive",
        3 => "efficient enabled",
        4 => "efficient aggressive",
        5 => "aggressive at guaranteed",
        6 => "efficient aggressive at guaranteed",
        _ => return format!("mode {index}"),
    }
    .to_string()
}

/// The Windows 11 power mode, from the overlay scheme GUID stored for it.
/// An empty value or the nil GUID is Balanced: the mode that applies no
/// overlay at all.
#[cfg_attr(not(windows), allow(dead_code))]
fn power_mode(overlay: &str) -> Option<String> {
    let g = overlay
        .trim()
        .trim_matches(|c| c == '{' || c == '}')
        .to_ascii_lowercase();
    Some(
        match g.as_str() {
            "" | "00000000-0000-0000-0000-000000000000" => "balanced",
            "961cc777-2547-4f9d-8174-7d86181b8a7a" => "best power efficiency",
            // Windows 10's "Better performance", which 11 folded into Balanced.
            "3af9b8d9-7c97-431d-ad78-34a8bfea439f" => "balanced",
            "ded574b5-45a0-4f42-8737-46345c09c238" => "best performance",
            _ => return None,
        }
        .to_string(),
    )
}

#[cfg(target_os = "linux")]
fn platform_collect() -> Power {
    use std::fs;

    let read = |path: &str| {
        fs::read_to_string(path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    // Mains if any "Mains" supply is online; battery if there's a battery and
    // no mains online; unknown (a desktop with no power_supply class entries,
    // most VMs) otherwise.
    let mut mains_online = false;
    let mut has_mains = false;
    let mut has_battery = false;
    if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
        for e in entries.flatten() {
            let dir = e.path();
            let kind = fs::read_to_string(dir.join("type")).unwrap_or_default();
            match kind.trim() {
                "Mains" => {
                    has_mains = true;
                    if fs::read_to_string(dir.join("online")).is_ok_and(|s| s.trim() == "1") {
                        mains_online = true;
                    }
                }
                "Battery" => {
                    // A wireless mouse's battery is a "Battery" too; only a
                    // system battery has scope "System" or no scope at all.
                    let scope = fs::read_to_string(dir.join("scope")).unwrap_or_default();
                    if scope.trim().is_empty() || scope.trim() == "System" {
                        has_battery = true;
                    }
                }
                _ => {}
            }
        }
    }
    let source = if mains_online {
        Some("mains")
    } else if has_battery {
        Some("battery")
    } else if has_mains {
        Some("mains")
    } else {
        None
    };

    // intel_pstate says "no_turbo"; every other driver says "boost".
    let turbo = read("/sys/devices/system/cpu/intel_pstate/no_turbo")
        .map(|v| v == "0")
        .or_else(|| read("/sys/devices/system/cpu/cpufreq/boost").map(|v| v == "1"));

    let cpu0 = "/sys/devices/system/cpu/cpu0/cpufreq";
    Power {
        source: source.map(str::to_string),
        platform_profile: read("/sys/firmware/acpi/platform_profile"),
        governor: read(&format!("{cpu0}/scaling_governor")),
        energy_preference: read(&format!("{cpu0}/energy_performance_preference")),
        turbo,
        ..Power::default()
    }
}

#[cfg(windows)]
fn platform_collect() -> Power {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

    let mut p = Power::default();

    let mut status: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: `status` is a valid, writable SYSTEM_POWER_STATUS.
    let have_status = unsafe { GetSystemPowerStatus(&mut status) } != 0;
    // BatteryFlag 128: no system battery, so there's no "on battery" column
    // worth reporting even though every plan stores one.
    let has_battery = have_status && status.BatteryFlag != 128 && status.BatteryFlag != 255;
    if have_status {
        p.source = match status.ACLineStatus {
            0 => Some("battery".to_string()),
            1 => Some("mains".to_string()),
            _ => None,
        };
        p.energy_saver = Some(status.SystemStatusFlag & 1 != 0);
    }

    if let Some(scheme) = win::active_scheme() {
        p.plan = win::friendly_name(&scheme);
        // The power mode only exists under Balanced: Settings greys it out
        // under any other plan, and the overlay values left behind from when
        // Balanced was active don't apply.
        let balanced = win::same(&scheme, &win::BALANCED);
        let column = |ac: bool| {
            let mode = balanced
                .then(|| {
                    win::overlay(if ac {
                        "ActiveOverlayAcPowerScheme"
                    } else {
                        "ActiveOverlayDcPowerScheme"
                    })
                })
                .and_then(|o| power_mode(&o.unwrap_or_default()));
            let m = ModeSettings {
                power_mode: mode,
                min_processor_pct: win::processor_value(&scheme, &win::THROTTLE_MINIMUM, ac),
                max_processor_pct: win::processor_value(&scheme, &win::THROTTLE_MAXIMUM, ac),
                boost: win::processor_value(&scheme, &win::PERF_BOOST_MODE, ac).map(boost_mode),
            };
            (m != ModeSettings::default()).then_some(m)
        };
        p.plugged_in = column(true);
        if has_battery {
            p.on_battery = column(false);
        }
    }
    p
}

#[cfg(windows)]
mod win {
    use windows_sys::core::GUID;

    pub const BALANCED: GUID = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);
    const PROCESSOR_SUBGROUP: GUID = GUID::from_u128(0x54533251_82be_4824_96c1_47b60b740d00);
    pub const THROTTLE_MINIMUM: GUID = GUID::from_u128(0x893dee8e_2bef_41e0_89c6_b55d0929964c);
    pub const THROTTLE_MAXIMUM: GUID = GUID::from_u128(0xbc5038f7_23e0_4960_96da_33abaf5935ec);
    pub const PERF_BOOST_MODE: GUID = GUID::from_u128(0xbe337238_0d82_4146_a960_4f3749d470c7);

    /// `windows_sys`'s GUID has no `PartialEq`.
    pub fn same(a: &GUID, b: &GUID) -> bool {
        (a.data1, a.data2, a.data3, a.data4) == (b.data1, b.data2, b.data3, b.data4)
    }

    /// The active power plan's GUID.
    pub fn active_scheme() -> Option<GUID> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::System::Power::PowerGetActiveScheme;

        let mut guid: *mut GUID = std::ptr::null_mut();
        // SAFETY: on success the call allocates the GUID with LocalAlloc and
        // stores its address in `guid`; it's copied out and freed below.
        let rc = unsafe { PowerGetActiveScheme(std::ptr::null_mut(), &mut guid) };
        if rc != 0 || guid.is_null() {
            log::debug!(target: "loadbearer::power", "PowerGetActiveScheme failed: {rc}");
            return None;
        }
        // SAFETY: `guid` is non-null and points at the GUID the call wrote.
        let out = unsafe { *guid };
        // SAFETY: allocated by PowerGetActiveScheme with LocalAlloc.
        unsafe { LocalFree(guid as _) };
        Some(out)
    }

    /// A plan's display name, e.g. "Balanced" or an IT department's custom
    /// plan name.
    pub fn friendly_name(scheme: &GUID) -> Option<String> {
        use windows_sys::Win32::System::Power::PowerReadFriendlyName;

        let mut size = 0u32;
        // SAFETY: a null buffer asks for the required size in bytes.
        unsafe {
            PowerReadFriendlyName(
                std::ptr::null_mut(),
                scheme,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        let mut bytes = (buf.len() * 2) as u32;
        // SAFETY: `buf` holds `bytes` bytes, the size the call asked for.
        let rc = unsafe {
            PowerReadFriendlyName(
                std::ptr::null_mut(),
                scheme,
                std::ptr::null(),
                std::ptr::null(),
                buf.as_mut_ptr() as *mut u8,
                &mut bytes,
            )
        };
        if rc != 0 {
            return None;
        }
        let s = String::from_utf16_lossy(&buf);
        let s = s.trim_end_matches('\0').trim();
        (!s.is_empty()).then(|| s.to_string())
    }

    /// One processor-subgroup setting's value for plugged in (`ac`) or on
    /// battery.
    pub fn processor_value(scheme: &GUID, setting: &GUID, ac: bool) -> Option<u32> {
        use windows_sys::Win32::System::Power::{PowerReadACValueIndex, PowerReadDCValueIndex};

        let mut v = 0u32;
        // SAFETY: every GUID pointer is to a live GUID, `v` is writable.
        let rc = unsafe {
            if ac {
                PowerReadACValueIndex(
                    std::ptr::null_mut(),
                    scheme,
                    &PROCESSOR_SUBGROUP,
                    setting,
                    &mut v,
                )
            } else {
                PowerReadDCValueIndex(
                    std::ptr::null_mut(),
                    scheme,
                    &PROCESSOR_SUBGROUP,
                    setting,
                    &mut v,
                )
            }
        };
        (rc == 0).then_some(v)
    }

    /// The power mode's overlay GUID for plugged in or on battery, from
    /// `HKLM\SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes`. Not a
    /// documented interface — the documented one,
    /// `PowerRegisterForEffectivePowerModeNotifications`, only reports the
    /// mode in effect now — but it is where Settings stores both. `Some("")`
    /// when the value exists but is empty, which is Balanced.
    pub fn overlay(value: &str) -> Option<String> {
        use std::ffi::c_void;
        use windows_sys::Win32::System::Registry::{
            HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
        };

        let subkey: Vec<u16> = "SYSTEM\\CurrentControlSet\\Control\\Power\\User\\PowerSchemes\0"
            .encode_utf16()
            .collect();
        let name: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
        let mut buf = [0u16; 64];
        let mut size = std::mem::size_of_val(&buf) as u32;
        // SAFETY: both name pointers are NUL-terminated UTF-16 that outlive the
        // call, and `size` is the byte length of `buf`, which the call clamps
        // its write to.
        let rc = unsafe {
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                subkey.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut c_void,
                &mut size,
            )
        };
        if rc != 0 {
            log::debug!(target: "loadbearer::power", "RegGetValueW({value}) failed: {rc}");
            return None;
        }
        let units = (size as usize / 2).min(buf.len());
        Some(
            String::from_utf16_lossy(&buf[..units])
                .trim_end_matches('\0')
                .to_string(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_modes_from_overlay_guids() {
        assert_eq!(power_mode("").as_deref(), Some("balanced"));
        assert_eq!(
            power_mode("{961CC777-2547-4F9D-8174-7D86181B8A7A}").as_deref(),
            Some("best power efficiency")
        );
        assert_eq!(
            power_mode("ded574b5-45a0-4f42-8737-46345c09c238").as_deref(),
            Some("best performance")
        );
        assert_eq!(power_mode("12345678-0000-0000-0000-000000000000"), None);
    }

    #[test]
    fn a_column_reads_as_one_line() {
        let m = ModeSettings {
            power_mode: Some("best power efficiency".into()),
            min_processor_pct: Some(5),
            max_processor_pct: Some(100),
            boost: Some(boost_mode(2)),
        };
        assert_eq!(
            m.summary(),
            "Best power efficiency mode · processor 5–100% · boost aggressive"
        );
    }

    #[test]
    fn limits_follow_the_column_in_force() {
        let p = Power {
            source: Some("mains".into()),
            plugged_in: Some(ModeSettings {
                max_processor_pct: Some(100),
                ..ModeSettings::default()
            }),
            on_battery: Some(ModeSettings {
                max_processor_pct: Some(50),
                boost: Some("disabled".into()),
                ..ModeSettings::default()
            }),
            ..Power::default()
        };
        assert!(p.limits().is_empty());

        let p = Power {
            source: Some("battery".into()),
            ..p
        };
        assert_eq!(
            p.limits(),
            [
                "running on battery",
                "maximum processor state 50%",
                "boost disabled"
            ]
        );
    }

    #[test]
    fn linux_limits() {
        let p = Power {
            platform_profile: Some("low-power".into()),
            governor: Some("powersave".into()),
            energy_preference: Some("balance_power".into()),
            turbo: Some(false),
            ..Power::default()
        };
        // "powersave" with a preference is the normal pstate setup, not a cap.
        assert_eq!(p.limits(), ["low-power platform profile", "turbo disabled"]);
    }
}
