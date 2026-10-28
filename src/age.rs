//! Placing a machine in time. Nothing on a PC records when it was built, so
//! this reasons from the things that do carry a date or a count:
//!
//! - **The CPU model.** Every CPU has a launch date and a machine can't
//!   predate its CPU, so the launch is a floor on the build date — a ceiling
//!   on age. Resolved to the *generation* from the model number, taking the
//!   earliest launch in it, so the floor stays true at the cost of a few
//!   months' precision.
//! - **The firmware date** ([`crate::firmware`]). Not a bound either way — an
//!   update moves it forward — but when it sits close to the CPU's launch it's
//!   probably the firmware the machine shipped with, which narrows things.
//! - **Use**: the boot NVMe drive's power-on hours ([`crate::nvme`]) and the
//!   battery's charge cycles. These measure how hard the machine has been
//!   used rather than when it was made, and either part may have been
//!   replaced.
//!
//! The estimate is stored in the result file as `machine.age`, in
//! machine-readable form, so a fleet collector gets it without re-deriving
//! it; the prose is generated from those fields when rendering. A result file
//! from before it existed has it derived on the fly. Never graded, like
//! battery and soak. The launch table is data, not API (see `VERSIONING.md`);
//! a CPU it doesn't know simply has no platform line.

use serde::{Deserialize, Serialize};

use crate::inventory::Inventory;

/// When a CPU generation first shipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// "12th-gen Intel Core (mobile)", "Apple M3 Pro", …
    pub family: String,
    pub year: u16,
    pub month: u8,
}

/// Within this many months of the CPU's launch, the firmware is probably what
/// the machine shipped with.
const AS_SHIPPED_MONTHS: u32 = 12;

/// The age evidence for one machine — `machine.age` in a result file. Dates
/// are `YYYY-MM`, spans are whole months, and every field but `as_of` is
/// absent when its source couldn't be read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Age {
    /// The month this estimate is as of: the run's.
    pub as_of: String,
    /// The CPU generation, as the launch table names it. Descriptive — the
    /// wording may change between releases; key on `cpu_launched`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_family: Option<String>,
    /// When that generation first shipped. The machine can't be older.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_launched: Option<String>,
    /// Months from `cpu_launched` to `as_of`: an upper bound on the machine's
    /// age.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_months: Option<u32>,
    /// The firmware's release month.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_dated: Option<String>,
    /// Months from `cpu_launched` to `firmware_dated`; absent if the firmware
    /// predates the launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_after_launch_months: Option<u32>,
    /// True when the firmware is within a year of the CPU's launch, and so is
    /// probably what the machine shipped with — which dates the machine to
    /// about `firmware_dated`. False means it has likely been updated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware_likely_original: Option<bool>,
    /// Power-on hours of the first NVMe drive (`nvme0`, or the lowest-numbered
    /// physical drive) — usually the boot drive. Absent without root / admin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_power_on_hours: Option<u64>,
    /// The battery's charge cycles, on a machine that reports them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_cycles: Option<u32>,
}

impl Age {
    /// Assess `inv` as of `(year, month)`. `None` when there's nothing to go
    /// on at all.
    pub fn assess(inv: &Inventory, as_of: (u16, u8)) -> Option<Self> {
        let cpu = cpu_launch(&inv.cpu_model);
        let launched = cpu.as_ref().map(|l| (l.year, l.month));
        let fw = inv.firmware.as_ref().and_then(|f| f.bios_year_month());
        let gap = launched.zip(fw).and_then(|(l, f)| months_between(l, f));
        let a = Age {
            as_of: ym(as_of),
            cpu_family: cpu.as_ref().map(|l| l.family.clone()),
            cpu_launched: launched.map(ym),
            max_age_months: launched.and_then(|l| months_between(l, as_of)),
            firmware_dated: fw.map(ym),
            firmware_after_launch_months: gap,
            firmware_likely_original: gap.map(|g| g <= AS_SHIPPED_MONTHS),
            drive_power_on_hours: inv.nvme.first().map(|d| d.power_on_hours),
            battery_cycles: inv.battery.as_ref().and_then(|b| b.cycle_count),
        };
        (a != Age {
            as_of: a.as_of.clone(),
            ..Default::default()
        })
        .then_some(a)
    }

    /// The stored estimate, or — for a result file written before there was
    /// one — one derived now, as of the run's `timestamp`.
    pub fn for_machine(inv: &Inventory, timestamp: &str) -> Option<Self> {
        inv.age
            .clone()
            .or_else(|| Age::assess(inv, year_month(timestamp)?))
    }

    /// One-line headline: "built Jan 2022 or later".
    pub fn summary(&self) -> String {
        if let Some(l) = self.cpu_launched.as_deref().and_then(year_month) {
            format!("built {} or later", month_year(l))
        } else if let Some(f) = self.firmware_dated.as_deref().and_then(year_month) {
            format!("firmware from {}", month_year(f))
        } else {
            "build date unknown".to_string()
        }
    }

    /// "12th-gen Intel Core (mobile), launched Jan 2022".
    pub fn platform(&self) -> Option<String> {
        let l = self.cpu_launched.as_deref().and_then(year_month)?;
        let family = self.cpu_family.as_deref().unwrap_or("CPU");
        Some(format!("{family}, launched {}", month_year(l)))
    }

    /// The age ceiling the CPU puts on the machine: "at most 4.8 years old".
    pub fn ceiling(&self) -> Option<String> {
        self.max_age_months
            .map(|m| format!("at most {} old", span(m)))
    }

    /// What the firmware date adds, read against the CPU's launch.
    pub fn firmware_note(&self) -> Option<String> {
        let fw = month_year(self.firmware_dated.as_deref().and_then(year_month)?);
        let gap = self.firmware_after_launch_months?;
        Some(if gap <= AS_SHIPPED_MONTHS {
            format!(
                "firmware is from {fw}, within a year of the CPU's launch — probably as shipped, \
                 so the machine likely dates from about then",
            )
        } else {
            format!(
                "firmware is from {fw}, {} after the CPU's launch — likely updated since it \
                 shipped, so it doesn't narrow the date",
                span(gap),
            )
        })
    }

    /// "drive powered 11,800 h (1.3 years continuous) · battery 49 cycles".
    pub fn usage(&self) -> Option<String> {
        let drive = self.drive_power_on_hours.map(|h| {
            format!(
                "drive powered {} h ({} continuous)",
                thousands(h),
                span(hours_to_months(h)),
            )
        });
        let battery = self.battery_cycles.map(|c| format!("battery {c} cycles"));
        let parts: Vec<String> = [drive, battery].into_iter().flatten().collect();
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// Set when the drive's hours don't fit the CPU's age — it has been
    /// powered for longer than the CPU has existed.
    pub fn usage_note(&self) -> Option<&'static str> {
        let hours = self.drive_power_on_hours?;
        let ceiling = self.max_age_months?;
        // A month's grace for the launch table's month-level precision.
        (hours_to_months(hours) > ceiling + 1).then_some(
            "the drive has been powered for longer than this CPU has existed — it came from \
             an older machine, or its counter isn't reliable",
        )
    }
}

/// `"2026-10-28T…"` (a result file's timestamp) or `"2026-10"` as
/// `(year, month)`.
pub fn year_month(timestamp: &str) -> Option<(u16, u8)> {
    let year = timestamp.get(0..4)?.parse().ok()?;
    let month = timestamp.get(5..7)?.parse().ok()?;
    (1..=12).contains(&month).then_some((year, month))
}

/// This month, for a freshly collected inventory.
pub fn this_month() -> (u16, u8) {
    let now = time::OffsetDateTime::now_utc();
    (now.year() as u16, u8::from(now.month()))
}

fn ym((year, month): (u16, u8)) -> String {
    format!("{year:04}-{month:02}")
}

/// Months from `a` to `b`, or `None` if `b` is earlier (a wrong clock, or
/// pre-release firmware) — neither says anything useful about age.
fn months_between(a: (u16, u8), b: (u16, u8)) -> Option<u32> {
    let a = a.0 as i32 * 12 + a.1 as i32;
    let b = b.0 as i32 * 12 + b.1 as i32;
    u32::try_from(b - a).ok()
}

/// Whole months of continuous power: a month is 730 hours on average.
fn hours_to_months(hours: u64) -> u32 {
    u32::try_from(hours / 730).unwrap_or(u32::MAX)
}

fn span(months: u32) -> String {
    match months {
        1 => "1 month".to_string(),
        0..12 => format!("{months} months"),
        _ => format!("{:.1} years", months as f64 / 12.0),
    }
}

pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn month_year((year, month): (u16, u8)) -> String {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    match NAMES.get(month.wrapping_sub(1) as usize) {
        Some(n) => format!("{n} {year}"),
        None => year.to_string(),
    }
}

// ---------------------------------------------------------------------------
// CPU launch table
// ---------------------------------------------------------------------------

/// Intel Core i3/i5/i7/i9 by generation: `(desktop, mobile)` first launch.
/// The two differ by up to half a year, and the model suffix says which.
const INTEL_CORE_GENS: [((u16, u8), (u16, u8)); 14] = [
    ((2008, 11), (2009, 9)), // 1st: Nehalem / Westmere
    ((2011, 1), (2011, 1)),  // 2nd: Sandy Bridge
    ((2012, 4), (2012, 4)),  // 3rd: Ivy Bridge
    ((2013, 6), (2013, 6)),  // 4th: Haswell
    ((2015, 6), (2015, 1)),  // 5th: Broadwell
    ((2015, 8), (2015, 9)),  // 6th: Skylake
    ((2017, 1), (2016, 8)),  // 7th: Kaby Lake
    ((2017, 10), (2017, 8)), // 8th: Coffee Lake / Kaby Lake R
    ((2018, 10), (2019, 4)), // 9th: Coffee Lake R
    ((2020, 4), (2019, 8)),  // 10th: Comet Lake / Ice Lake
    ((2021, 3), (2020, 9)),  // 11th: Rocket Lake / Tiger Lake
    ((2021, 11), (2022, 1)), // 12th: Alder Lake
    ((2022, 10), (2023, 1)), // 13th: Raptor Lake
    ((2023, 10), (2024, 1)), // 14th: Raptor Lake Refresh
];

/// Map a CPU brand string — as the OS reports it, or as `models` writes it —
/// to when its generation launched. `None` for anything not in the table:
/// Xeons, older Atoms and most server parts aren't, because their numbering
/// doesn't encode the generation reliably enough to give a true floor.
pub fn cpu_launch(model: &str) -> Option<Launch> {
    let t = tokens(model);
    apple(&t)
        .or_else(|| intel_core_ultra(&t))
        .or_else(|| intel_core_i(&t))
        .or_else(|| intel_core_series(&t))
        .or_else(|| intel_n(&t))
        .or_else(|| intel_atom_class(&t))
        .or_else(|| amd_ryzen_ai(&t))
        .or_else(|| amd_threadripper(&t))
        .or_else(|| amd_ryzen(&t))
        .or_else(|| amd_epyc(&t))
        .or_else(|| snapdragon_x(&t))
}

fn launch(family: impl Into<String>, (year, month): (u16, u8)) -> Option<Launch> {
    Some(Launch {
        family: family.into(),
        year,
        month,
    })
}

/// Lower-cased words with the ®/™ marks gone, so "Intel(R) Core(TM) i7-1265U"
/// and "Intel Core i7-1265U" read the same.
fn tokens(model: &str) -> Vec<String> {
    let mut s = model.to_lowercase();
    for mark in ["(r)", "(tm)", "®", "™"] {
        s = s.replace(mark, " ");
    }
    s.split_whitespace().map(str::to_string).collect()
}

/// The words after the first `word`.
fn after<'a>(t: &'a [String], word: &str) -> Option<&'a [String]> {
    let i = t.iter().position(|x| x == word)?;
    Some(&t[i + 1..])
}

/// "1265u" → ("1265", "u").
fn split_digits(tok: &str) -> (&str, &str) {
    let n = tok.bytes().take_while(u8::is_ascii_digit).count();
    tok.split_at(n)
}

fn ordinal(n: usize) -> String {
    match n {
        1 => "1st".to_string(),
        2 => "2nd".to_string(),
        3 => "3rd".to_string(),
        _ => format!("{n}th"),
    }
}

/// Apple M-series, by Mac launch.
fn apple(t: &[String]) -> Option<Launch> {
    let rest = after(t, "apple")?;
    let n: u8 = rest.first()?.strip_prefix('m')?.parse().ok()?;
    let tier = rest.get(1).map(String::as_str).unwrap_or("");
    let date = match (n, tier) {
        (1, "pro" | "max") => (2021, 10),
        (1, "ultra") => (2022, 3),
        (1, _) => (2020, 11),
        (2, "pro" | "max") => (2023, 1),
        (2, "ultra") => (2023, 6),
        (2, _) => (2022, 6),
        (3, "ultra") => (2025, 3),
        (3, _) => (2023, 10),
        (4, _) => (2024, 10),
        (5, _) => (2025, 10),
        _ => return None,
    };
    let tier = match tier {
        "pro" => " Pro",
        "max" => " Max",
        "ultra" => " Ultra",
        _ => "",
    };
    launch(format!("Apple M{n}{tier}"), date)
}

/// "Core Ultra 7 155H": the series is the model number's first digit.
fn intel_core_ultra(t: &[String]) -> Option<Launch> {
    let rest = after(t, "ultra")?;
    if !t.iter().any(|x| x == "core") {
        return None;
    }
    let (digits, _) = split_digits(rest.get(1)?);
    if digits.len() != 3 {
        return None;
    }
    let (series, date) = match &digits[..1] {
        "1" => (1, (2023, 12)), // Meteor Lake
        "2" => (2, (2024, 9)),  // Lunar Lake, then Arrow Lake
        "3" => (3, (2026, 1)),  // Panther Lake
        _ => return None,
    };
    launch(format!("Intel Core Ultra Series {series}"), date)
}

/// "i7-1265U", "i5-8350U", "i9-13900K", "i7-920", "i3-N305".
fn intel_core_i(t: &[String]) -> Option<Launch> {
    let tok = t.iter().find(|x| {
        let b = x.as_bytes();
        b.len() > 3 && b[0] == b'i' && b[1].is_ascii_digit() && b[2] == b'-'
    })?;
    let model = &tok[3..];
    if model.starts_with('n') {
        // i3-N300 / N305: Alder Lake-N, same as the plain N-series.
        return launch("Intel Core i3 N-series", (2023, 1));
    }
    let (digits, suffix) = split_digits(model);
    let generation: usize = match digits.len() {
        3 => 1,
        // 10th–13th-gen mobile parts are four digits starting "1x": i7-1265U.
        4 if digits.starts_with('1') => digits[..2].parse().ok()?,
        4 => digits[..1].parse().ok()?,
        5 => digits[..2].parse().ok()?,
        _ => return None,
    };
    let (desktop, mobile) = *INTEL_CORE_GENS.get(generation.checked_sub(1)?)?;
    let is_mobile = suffix.starts_with(['u', 'y', 'h', 'p', 'g', 'm', 'q']);
    let (date, kind) = if is_mobile {
        (mobile, "mobile")
    } else {
        (desktop, "desktop")
    };
    launch(
        format!("{}-gen Intel Core ({kind})", ordinal(generation)),
        date,
    )
}

/// The non-Ultra "Core 7 150U" branding: series is the first digit.
fn intel_core_series(t: &[String]) -> Option<Launch> {
    let rest = after(t, "core")?;
    if !matches!(rest.first()?.as_str(), "3" | "5" | "7" | "9") {
        return None;
    }
    let (digits, _) = split_digits(rest.get(1)?);
    if digits.len() != 3 {
        return None;
    }
    let (series, date) = match &digits[..1] {
        "1" => (1, (2024, 1)),
        "2" => (2, (2025, 1)),
        _ => return None,
    };
    launch(format!("Intel Core Series {series}"), date)
}

/// "Intel N95", "Intel N100": Alder Lake-N, and its Twin Lake refresh.
fn intel_n(t: &[String]) -> Option<Launch> {
    if !t.iter().any(|x| x == "intel") {
        return None;
    }
    t.iter().find_map(|x| {
        let (digits, rest) = split_digits(x.strip_prefix('n')?);
        if !(2..=3).contains(&digits.len()) || !rest.is_empty() {
            return None;
        }
        match digits {
            "150" | "250" | "350" | "355" => launch("Intel N-series (Twin Lake)", (2025, 1)),
            _ => launch("Intel N-series (Alder Lake-N)", (2023, 1)),
        }
    })
}

/// Atom-class Celeron / Pentium (J4005, N5105, N2840). The numbering only
/// separates the eras coarsely, so the floor is the earliest of each.
fn intel_atom_class(t: &[String]) -> Option<Launch> {
    if !t.iter().any(|x| x == "celeron" || x == "pentium") {
        return None;
    }
    t.iter().find_map(|x| {
        let (digits, rest) = split_digits(x.strip_prefix(['j', 'n'])?);
        if digits.len() != 4 || !rest.is_empty() {
            return None;
        }
        match &digits[..1] {
            "1" | "2" | "3" => launch("Intel Celeron/Pentium (Bay Trail or later)", (2013, 9)),
            "4" | "5" | "6" => launch("Intel Celeron/Pentium (Gemini Lake or later)", (2017, 12)),
            _ => None,
        }
    })
}

/// "Ryzen AI 9 HX 370", "Ryzen AI Max+ 395".
fn amd_ryzen_ai(t: &[String]) -> Option<Launch> {
    let rest = after(t, "ryzen")?;
    if rest.first()? != "ai" {
        return None;
    }
    if rest.get(1).is_some_and(|x| x.starts_with("max")) {
        launch("AMD Ryzen AI Max", (2025, 1))
    } else {
        launch("AMD Ryzen AI", (2024, 7))
    }
}

/// "Threadripper 3970X", "Threadripper PRO 5995WX".
fn amd_threadripper(t: &[String]) -> Option<Launch> {
    let rest = after(t, "threadripper")?;
    let tok = rest.iter().find(|x| *x != "pro")?;
    let (digits, _) = split_digits(tok);
    if digits.len() != 4 {
        return None;
    }
    let date = match &digits[..1] {
        "1" => (2017, 8),
        "2" => (2018, 8),
        "3" => (2019, 11),
        "5" => (2022, 3),
        "7" => (2023, 10),
        "9" => (2024, 7),
        _ => return None,
    };
    launch(format!("AMD Ryzen Threadripper {}000", &digits[..1]), date)
}

/// "Ryzen 7 5800X", "Ryzen 7 PRO 5850U": the series is the first digit,
/// earliest of desktop and mobile.
fn amd_ryzen(t: &[String]) -> Option<Launch> {
    let rest = after(t, "ryzen")?;
    if !matches!(rest.first()?.as_str(), "3" | "5" | "7" | "9") {
        return None;
    }
    let tok = rest[1..].iter().find(|x| *x != "pro")?;
    let (digits, _) = split_digits(tok);
    if digits.len() != 4 {
        return None;
    }
    let date = match &digits[..1] {
        "1" => (2017, 3),
        "2" => (2017, 10),
        "3" => (2019, 1),
        "4" => (2020, 3),
        "5" => (2020, 11),
        "6" => (2022, 1),
        "7" => (2022, 9),
        "8" => (2023, 12),
        "9" => (2024, 8),
        _ => return None,
    };
    launch(format!("AMD Ryzen {}000 series", &digits[..1]), date)
}

/// "EPYC 7763": the generation is the *last* digit.
fn amd_epyc(t: &[String]) -> Option<Launch> {
    let (digits, _) = split_digits(after(t, "epyc")?.first()?);
    if digits.len() != 4 {
        return None;
    }
    let (name, date) = match &digits[3..] {
        "1" => ("Naples", (2017, 6)),
        "2" => ("Rome", (2019, 8)),
        "3" => ("Milan", (2021, 3)),
        "4" => ("Genoa", (2022, 11)),
        "5" => ("Turin", (2024, 10)),
        _ => return None,
    };
    launch(format!("AMD EPYC ({name})"), date)
}

/// First-generation Snapdragon X Elite / Plus (X1E…, X1P…).
fn snapdragon_x(t: &[String]) -> Option<Launch> {
    if t.iter()
        .any(|x| x.starts_with("x1e") || x.starts_with("x1p"))
    {
        launch("Qualcomm Snapdragon X", (2024, 6))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(model: &str) -> Option<(String, u16, u8)> {
        cpu_launch(model).map(|l| (l.family, l.year, l.month))
    }

    #[test]
    fn os_brand_strings_resolve() {
        for (brand, family, y, m) in [
            (
                "12th Gen Intel(R) Core(TM) i7-1265U",
                "12th-gen Intel Core (mobile)",
                2022,
                1,
            ),
            (
                "Intel(R) Core(TM) i5-8350U CPU @ 1.70GHz",
                "8th-gen Intel Core (mobile)",
                2017,
                8,
            ),
            (
                "Intel(R) Core(TM) i7-10510U CPU @ 1.80GHz",
                "10th-gen Intel Core (mobile)",
                2019,
                8,
            ),
            (
                "13th Gen Intel(R) Core(TM) i9-13900K",
                "13th-gen Intel Core (desktop)",
                2022,
                10,
            ),
            (
                "Intel(R) Core(TM) i7-1065G7 CPU @ 1.30GHz",
                "10th-gen Intel Core (mobile)",
                2019,
                8,
            ),
            (
                "Intel(R) Core(TM) i7-8700 CPU @ 3.20GHz",
                "8th-gen Intel Core (desktop)",
                2017,
                10,
            ),
            ("Intel(R) Core(TM) i7 CPU 920 @ 2.67GHz", "", 0, 0),
            (
                "Intel(R) Core(TM) Ultra 7 155H",
                "Intel Core Ultra Series 1",
                2023,
                12,
            ),
            ("Intel(R) Core(TM) 7 150U", "Intel Core Series 1", 2024, 1),
            (
                "Intel(R) Core(TM) i3-N305",
                "Intel Core i3 N-series",
                2023,
                1,
            ),
            ("Intel(R) N100", "Intel N-series (Alder Lake-N)", 2023, 1),
            (
                "Intel(R) Celeron(R) J4005 CPU @ 2.00GHz",
                "Intel Celeron/Pentium (Gemini Lake or later)",
                2017,
                12,
            ),
            (
                "AMD Ryzen 7 5800X 8-Core Processor",
                "AMD Ryzen 5000 series",
                2020,
                11,
            ),
            (
                "AMD Ryzen 7 PRO 5850U with Radeon Graphics",
                "AMD Ryzen 5000 series",
                2020,
                11,
            ),
            (
                "AMD Ryzen AI 9 HX 370 w/ Radeon 890M",
                "AMD Ryzen AI",
                2024,
                7,
            ),
            (
                "AMD RYZEN AI MAX+ 395 w/ Radeon 8060S",
                "AMD Ryzen AI Max",
                2025,
                1,
            ),
            (
                "AMD Ryzen Threadripper PRO 5995WX 64-Cores",
                "AMD Ryzen Threadripper 5000",
                2022,
                3,
            ),
            (
                "AMD EPYC 7763 64-Core Processor",
                "AMD EPYC (Milan)",
                2021,
                3,
            ),
            (
                "Snapdragon(R) X Elite - X1E78100 - Qualcomm(R) Oryon(TM) CPU",
                "Qualcomm Snapdragon X",
                2024,
                6,
            ),
            ("Apple M1 Max", "Apple M1 Max", 2021, 10),
        ] {
            let want = (!family.is_empty()).then(|| (family.to_string(), y, m));
            assert_eq!(at(brand), want, "{brand}");
        }
    }

    /// Every CPU in the model-reference table should place, since those are
    /// the machines loadbearer has actually been run on.
    #[test]
    fn every_reference_model_resolves() {
        let table: toml::Value =
            toml::from_str(include_str!("../baseline/models/cpu.toml")).unwrap();
        for cpu in table["cpu"].as_array().unwrap() {
            let model = cpu["model"].as_str().unwrap();
            assert!(cpu_launch(model).is_some(), "{model} has no launch date");
        }
    }

    #[test]
    fn unknown_parts_get_no_date_rather_than_a_guess() {
        for brand in [
            "Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz",
            "Intel(R) Core(TM)2 Duo CPU E8400 @ 3.00GHz",
            "Intel(R) Celeron(R) G5900",
            "QEMU Virtual CPU version 2.5+",
            "unknown",
            "",
        ] {
            assert_eq!(cpu_launch(brand), None, "{brand}");
        }
    }

    fn machine(cpu: &str, firmware: Option<&str>, drive_hours: Option<u64>) -> Inventory {
        let mut inv = Inventory::blank();
        inv.cpu_model = cpu.to_string();
        inv.firmware = firmware.map(|d| crate::firmware::Firmware {
            bios_date: Some(d.to_string()),
            ..Default::default()
        });
        inv.nvme = drive_hours
            .map(|h| crate::nvme::NvmeHealth {
                device: "nvme0".into(),
                model: None,
                power_on_hours: h,
                power_cycles: 0,
                unsafe_shutdowns: 0,
                percentage_used: 0,
                available_spare_pct: 100,
                data_written_tb: 0.0,
                media_errors: 0,
                critical_warning: 0,
            })
            .into_iter()
            .collect();
        inv
    }

    fn assessment(cpu: &str, firmware: Option<&str>, as_of: (u16, u8)) -> Age {
        Age::assess(&machine(cpu, firmware, None), as_of).unwrap()
    }

    #[test]
    fn the_cpu_launch_caps_the_age() {
        let a = assessment("Intel Core i7-1265U", None, (2026, 10));
        assert_eq!(a.summary(), "built Jan 2022 or later");
        assert_eq!(a.cpu_launched.as_deref(), Some("2022-01"));
        assert_eq!(a.max_age_months, Some(57));
        assert_eq!(a.ceiling().as_deref(), Some("at most 4.8 years old"));
        assert_eq!(a.firmware_note(), None);

        let a = assessment("Intel Core Ultra 7 366H", None, (2026, 9));
        assert_eq!(a.ceiling().as_deref(), Some("at most 8 months old"));
    }

    #[test]
    fn firmware_near_launch_narrows_and_far_from_it_does_not() {
        let near = assessment("Intel Core i7-1265U", Some("2022-05-12"), (2026, 10));
        assert_eq!(near.firmware_likely_original, Some(true));
        assert!(
            near.firmware_note()
                .unwrap()
                .contains("probably as shipped")
        );
        let far = assessment("Intel Core i7-1265U", Some("2024-03-01"), (2026, 10));
        assert_eq!(far.firmware_likely_original, Some(false));
        assert!(far.firmware_note().unwrap().contains("2.2 years after"));
        // Firmware predating the CPU says nothing either way.
        let early = assessment("Intel Core i7-1265U", Some("2021-06-01"), (2026, 10));
        assert_eq!(early.firmware_likely_original, None);
        assert_eq!(early.firmware_note(), None);
    }

    #[test]
    fn a_clock_behind_the_launch_gives_no_ceiling() {
        let a = assessment("Intel Core i7-1265U", None, (2020, 1));
        assert_eq!(a.ceiling(), None);
    }

    #[test]
    fn drive_hours_are_reported_and_checked_against_the_ceiling() {
        let inv = machine("Intel Core i7-1265U", None, Some(11_800));
        let a = Age::assess(&inv, (2026, 10)).unwrap();
        assert_eq!(a.drive_power_on_hours, Some(11_800));
        assert_eq!(
            a.usage().as_deref(),
            Some("drive powered 11,800 h (1.3 years continuous)")
        );
        assert_eq!(a.usage_note(), None);

        // Six years of power on a CPU at most 4.8 years old.
        let inv = machine("Intel Core i7-1265U", None, Some(52_560));
        let a = Age::assess(&inv, (2026, 10)).unwrap();
        assert!(a.usage_note().is_some());
    }

    #[test]
    fn nothing_to_go_on_is_no_estimate() {
        assert_eq!(
            Age::assess(&machine("QEMU Virtual CPU", None, None), (2026, 10)),
            None
        );
        // Drive hours alone are still worth reporting.
        assert!(Age::assess(&machine("QEMU Virtual CPU", None, Some(10)), (2026, 10)).is_some());
    }

    #[test]
    fn an_old_result_file_gets_an_estimate_derived_from_its_timestamp() {
        let inv = machine("Intel Core i7-1265U", None, None);
        assert!(inv.age.is_none());
        let a = Age::for_machine(&inv, "2026-10-28T09:15:00Z").unwrap();
        assert_eq!(a.as_of, "2026-10");
        assert_eq!(a.max_age_months, Some(57));
    }

    #[test]
    fn the_stored_block_round_trips_through_json() {
        let a = Age::assess(
            &machine("Intel Core i7-1265U", Some("2022-05-12"), Some(9)),
            (2026, 10),
        )
        .unwrap();
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(json["cpu_launched"], "2022-01");
        assert_eq!(json["firmware_likely_original"], true);
        assert!(json.get("battery_cycles").is_none());
        assert_eq!(serde_json::from_value::<Age>(json).unwrap(), a);
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn result_timestamps_parse() {
        assert_eq!(year_month("2026-10-28T09:15:00Z"), Some((2026, 10)));
        assert_eq!(year_month("2026-10"), Some((2026, 10)));
        assert_eq!(year_month("garbage"), None);
        assert_eq!(year_month(""), None);
    }
}
