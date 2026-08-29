//! Battery inventory and health.
//!
//! Read once, at inventory time, from the OS power-supply interface (sysfs on
//! Linux, the battery IOCTL on Windows) via the `starship-battery` crate. A
//! machine with no battery — a desktop, a server, most VMs — reports `None` and
//! every battery-related line in `info` and in a run report is simply omitted.
//!
//! Battery **health** (present full-charge capacity as a fraction of the pack's
//! design capacity, plus the charge-cycle count) is shown in a `run` report but
//! is **not** folded into any grade — it is a property of the consumable pack,
//! not of the silicon under test, exactly like the `network` and `gpu`
//! components.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use starship_battery::units::{
    electric_potential::volt, energy::watt_hour, ratio::percent,
    thermodynamic_temperature::degree_celsius,
};

/// The primary battery's inventory facts and wear state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// e.g. `"lithium-ion"`; omitted when the controller reports "unknown".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technology: Option<String>,
    /// State of charge, 0–100 %.
    pub charge_pct: f64,
    /// `"charging"`, `"discharging"`, `"full"`, `"empty"` or `"unknown"`.
    pub state: String,
    /// State of health: present full-charge capacity as a percentage of the
    /// design capacity. `None` when the platform exposes no design capacity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_pct: Option<f64>,
    /// Present full-charge capacity, watt-hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_full_wh: Option<f64>,
    /// Design (as-new) capacity, watt-hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_full_design_wh: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voltage_v: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_c: Option<f64>,
}

/// Present full-charge capacity as a percentage of design capacity, when both
/// readings are available and sane. Values above 100 % (a fresh pack that
/// slightly exceeds its rated capacity) are kept as-is.
fn health_from_energy(full_wh: Option<f64>, design_wh: Option<f64>) -> Option<f64> {
    match (full_wh, design_wh) {
        (Some(full), Some(design)) if design > 0.0 && full > 0.0 => Some(full / design * 100.0),
        _ => None,
    }
}

impl BatteryInfo {
    fn from_battery(b: &starship_battery::Battery) -> Self {
        let pos = |v: f32| (v.is_finite() && v > 0.0).then_some(f64::from(v));
        let energy_full_wh = pos(b.energy_full().get::<watt_hour>());
        let energy_full_design_wh = pos(b.energy_full_design().get::<watt_hour>());
        let technology = b.technology().to_string();

        Self {
            vendor: clean(b.vendor()),
            model: clean(b.model()),
            technology: (technology != "unknown").then_some(technology),
            charge_pct: f64::from(b.state_of_charge().get::<percent>()).clamp(0.0, 100.0),
            state: b.state().to_string(),
            health_pct: health_from_energy(energy_full_wh, energy_full_design_wh),
            energy_full_wh,
            energy_full_design_wh,
            cycle_count: b.cycle_count().filter(|&c| c > 0),
            voltage_v: pos(b.voltage().get::<volt>()),
            temperature_c: b
                .temperature()
                .map(|t| f64::from(t.get::<degree_celsius>())),
        }
    }

    /// One-line wear summary for a report header, e.g.
    /// `health 87% of design · 342 cycles` — or a charge-only line when the
    /// platform gives no design capacity to compare against.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        match self.health_pct {
            Some(h) => parts.push(format!("health {h:.0}% of design")),
            None => parts.push(format!("charge {:.0}%", self.charge_pct)),
        }
        if let Some(c) = self.cycle_count {
            parts.push(format!("{c} cycles"));
        }
        parts.join(" · ")
    }

    /// A plain-language read on the pack's condition, or `None` when health
    /// can't be determined (no design-capacity reading).
    pub fn health_verdict(&self) -> Option<&'static str> {
        let h = self.health_pct?;
        Some(if h >= 95.0 {
            "as-new — no measurable capacity loss"
        } else if h >= 85.0 {
            "healthy — minor capacity loss"
        } else if h >= 70.0 {
            "worn — noticeable capacity loss"
        } else if h >= 50.0 {
            "degraded — significant capacity loss"
        } else {
            "failing — consider replacement"
        })
    }

    /// Set when the machine is running on battery: benchmark clocks may be
    /// capped by a power profile, so a graded run should be treated with
    /// suspicion. `None` on AC power.
    pub fn power_note(&self) -> Option<&'static str> {
        (self.state == "discharging")
            .then_some("on battery power — clocks may be capped; prefer mains for a clean grade")
    }
}

fn clean(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

static CACHE: OnceLock<Option<BatteryInfo>> = OnceLock::new();

/// The primary battery, or `None` on a machine with no battery. Cached after
/// the first call. Any error talking to the OS power interface is treated as
/// "no battery" — this is inventory decoration, never a hard failure.
pub fn probe() -> Option<&'static BatteryInfo> {
    CACHE.get_or_init(read).as_ref()
}

fn read() -> Option<BatteryInfo> {
    let manager = match starship_battery::Manager::new() {
        Ok(m) => m,
        Err(e) => {
            log::debug!(target: "loadbearer::battery", "no battery manager ({e})");
            return None;
        }
    };
    let Some(Ok(battery)) = manager.batteries().ok()?.next() else {
        log::debug!(target: "loadbearer::battery", "no battery present");
        return None;
    };
    let info = BatteryInfo::from_battery(&battery);
    log::debug!(
        target: "loadbearer::battery",
        "battery: {} · charge {:.0}% ({})", info.summary(), info.charge_pct, info.state,
    );
    Some(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BatteryInfo {
        BatteryInfo {
            vendor: Some("LGC".to_string()),
            model: Some("01AV445".to_string()),
            technology: Some("lithium-ion".to_string()),
            charge_pct: 76.0,
            state: "discharging".to_string(),
            health_pct: health_from_energy(Some(41.2), Some(47.5)),
            energy_full_wh: Some(41.2),
            energy_full_design_wh: Some(47.5),
            cycle_count: Some(342),
            voltage_v: Some(11.4),
            temperature_c: None,
        }
    }

    #[test]
    fn health_is_full_over_design() {
        let h = health_from_energy(Some(41.2), Some(47.5)).unwrap();
        assert!((h - 86.7).abs() < 0.1, "got {h}");
    }

    #[test]
    fn health_needs_both_readings() {
        assert!(health_from_energy(Some(40.0), None).is_none());
        assert!(health_from_energy(None, Some(50.0)).is_none());
        assert!(health_from_energy(Some(40.0), Some(0.0)).is_none());
    }

    #[test]
    fn verdict_tracks_the_bands() {
        let mut b = sample();
        b.health_pct = Some(97.0);
        assert!(b.health_verdict().unwrap().starts_with("as-new"));
        b.health_pct = Some(87.0);
        assert!(b.health_verdict().unwrap().starts_with("healthy"));
        b.health_pct = Some(72.0);
        assert!(b.health_verdict().unwrap().starts_with("worn"));
        b.health_pct = Some(55.0);
        assert!(b.health_verdict().unwrap().starts_with("degraded"));
        b.health_pct = Some(40.0);
        assert!(b.health_verdict().unwrap().starts_with("failing"));
        b.health_pct = None;
        assert!(b.health_verdict().is_none());
    }

    #[test]
    fn power_note_only_on_battery() {
        let mut b = sample();
        assert!(b.power_note().is_some());
        b.state = "charging".to_string();
        assert!(b.power_note().is_none());
    }

    #[test]
    fn summary_prefers_health_then_falls_back_to_charge() {
        let mut b = sample();
        assert_eq!(b.summary(), "health 87% of design · 342 cycles");
        b.health_pct = None;
        b.cycle_count = None;
        assert_eq!(b.summary(), "charge 76%");
    }
}
