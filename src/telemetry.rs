//! Best-effort CPU clock (and, on Linux, package power) sampling for the
//! duration of a run, so a thermally limited machine can be told apart from one
//! that is simply slow. Metadata only — never scored.
//!
//! Clocks come from `sysinfo` (cross-platform); package power from Intel RAPL
//! (`/sys/class/powercap`) on Linux. macOS exposes neither without elevated
//! privileges, so there the telemetry is reported as `unavailable`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::engine::{BenchmarkOutcome, Representative};

/// How often the background thread samples.
const POLL: Duration = Duration::from_millis(250);
/// A late-run clock mean at or below this fraction of the early-run mean is
/// read as throttling.
const THROTTLE_RATIO: f64 = 0.92;
/// Below this many samples the throttle check is skipped — too little signal.
const MIN_SAMPLES_FOR_THROTTLE: usize = 8;

/// What the sampler observed over a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTelemetry {
    /// `"sysinfo"`, `"sysinfo+rapl"`, or `"unavailable"`.
    pub source: String,
    pub sample_count: usize,
    /// Mean clock across logical CPUs (MHz) at the first and last usable sample.
    pub mhz_start: f64,
    pub mhz_end: f64,
    pub mhz_min: f64,
    pub mhz_max: f64,
    pub mhz_mean: f64,
    /// Set when the late-run clock mean sits well below the early-run mean.
    pub thermal_limited: bool,
    /// Mean package power over the run — Linux + Intel RAPL only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_watts_mean: Option<f64>,
}

impl RunTelemetry {
    fn unavailable() -> Self {
        Self {
            source: "unavailable".into(),
            sample_count: 0,
            mhz_start: 0.0,
            mhz_end: 0.0,
            mhz_min: 0.0,
            mhz_max: 0.0,
            mhz_mean: 0.0,
            thermal_limited: false,
            package_watts_mean: None,
        }
    }

    /// Classify a run from its per-sample clocks plus optional mean package
    /// power. Split out from the sampler so it can be tested directly.
    ///
    /// `mhz` is the mean across logical CPUs, which is what gets reported.
    /// `busiest` is the *highest* core in each sample, which is what decides
    /// throttling: on a 20-thread machine running a single-threaded subtest,
    /// nineteen idle cores drag the mean down to near-idle and hold it there,
    /// so a head-versus-tail comparison of the mean moves by almost nothing.
    /// Measured across twenty real runs — several of which lost 40 % of their
    /// all-core throughput mid-subtest — the mean-based check fired exactly
    /// zero times. The busiest core actually tracks boost.
    fn from_samples(mhz: &[f64], busiest: &[f64], watts: Option<f64>) -> Self {
        if mhz.is_empty() {
            return Self::unavailable();
        }
        let n = mhz.len();
        let window = n.div_ceil(4);
        let throttle_series = if busiest.len() == n { busiest } else { mhz };
        let head = mean(&throttle_series[..window]);
        let tail = mean(&throttle_series[n - window..]);
        let thermal_limited =
            n >= MIN_SAMPLES_FOR_THROTTLE && head > 0.0 && tail <= head * THROTTLE_RATIO;
        Self {
            source: if watts.is_some() {
                "sysinfo+rapl".into()
            } else {
                "sysinfo".into()
            },
            sample_count: n,
            mhz_start: mhz[0],
            mhz_end: mhz[n - 1],
            mhz_min: mhz.iter().copied().fold(f64::INFINITY, f64::min),
            mhz_max: mhz.iter().copied().fold(0.0, f64::max),
            mhz_mean: mean(mhz),
            thermal_limited,
            package_watts_mean: watts,
        }
    }

    /// One-line summary for a report header, or `None` when nothing usable was
    /// captured.
    pub fn summary(&self) -> Option<String> {
        if self.sample_count == 0 || self.mhz_mean <= 0.0 {
            return None;
        }
        let ghz = |m: f64| m / 1000.0;
        let mut s = format!(
            "Clocks {:.1}\u{2192}{:.1} GHz (min {:.1}, max {:.1})",
            ghz(self.mhz_start),
            ghz(self.mhz_end),
            ghz(self.mhz_min),
            ghz(self.mhz_max),
        );
        if let Some(w) = self.package_watts_mean {
            s.push_str(&format!(" \u{b7} {w:.0} W package"));
        }
        s.push_str(if self.thermal_limited {
            " \u{b7} thermally limited"
        } else {
            " \u{b7} steady"
        });
        Some(s)
    }
}

/// A running background sampler. Drop-safe: [`finish`](Self::finish) stops the
/// thread and returns what it gathered.
pub struct Sampler {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<RunTelemetry>>,
}

impl Sampler {
    /// Start sampling on a background thread. `enabled == false` gives a no-op
    /// sampler whose [`finish`](Self::finish) returns `None`.
    pub fn start(enabled: bool) -> Self {
        if !enabled {
            return Self {
                stop: Arc::new(AtomicBool::new(true)),
                handle: None,
            };
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let handle = std::thread::Builder::new()
            .name("loadbearer-telemetry".into())
            .spawn(move || sample_loop(&stop_thread))
            .inspect_err(
                |e| log::warn!(target: "loadbearer::telemetry", "could not start sampler: {e}"),
            )
            .ok();
        Self { stop, handle }
    }

    /// Stop sampling and return the telemetry, or `None` if it was disabled or
    /// the thread could not be started.
    pub fn finish(self) -> Option<RunTelemetry> {
        self.stop.store(true, Ordering::Relaxed);
        let telemetry = self.handle?.join().ok()?;
        log::debug!(
            target: "loadbearer::telemetry",
            "{} sample(s), clocks {:.0}->{:.0} MHz (min {:.0}), thermal_limited={}",
            telemetry.sample_count, telemetry.mhz_start, telemetry.mhz_end,
            telemetry.mhz_min, telemetry.thermal_limited,
        );
        Some(telemetry)
    }
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f64>() / xs.len() as f64
    }
}

fn sample_loop(stop: &AtomicBool) -> RunTelemetry {
    let mut sys = sysinfo::System::new();
    let mut samples: Vec<f64> = Vec::new();
    let mut busiest: Vec<f64> = Vec::new();
    let started = Instant::now();
    let energy_start = rapl_energy_uj();

    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(POLL);
        sys.refresh_cpu_all();
        let per_cpu: Vec<f64> = sys
            .cpus()
            .iter()
            .map(|c| c.frequency() as f64)
            .filter(|&f| f > 0.0)
            .collect();
        if !per_cpu.is_empty() {
            samples.push(mean(&per_cpu));
            busiest.push(per_cpu.iter().copied().fold(0.0, f64::max));
        }
    }

    let elapsed = started.elapsed().as_secs_f64();
    let watts = match (energy_start, rapl_energy_uj()) {
        (Some(a), Some(b)) if b >= a && elapsed > 0.5 => Some((b - a) as f64 / elapsed / 1e6),
        _ => None,
    };
    RunTelemetry::from_samples(&samples, &busiest, watts)
}

#[cfg(target_os = "linux")]
fn rapl_energy_uj() -> Option<u64> {
    // Intel RAPL package domain 0. Newer kernels expose AMD's counter at the
    // same path. Unreadable (permissions, no RAPL, a VM) -> None.
    for path in [
        "/sys/class/powercap/intel-rapl:0/energy_uj",
        "/sys/class/powercap/intel-rapl/intel-rapl:0/energy_uj",
    ] {
        if let Ok(text) = std::fs::read_to_string(path)
            && let Ok(v) = text.trim().parse::<u64>()
        {
            return Some(v);
        }
    }
    None
}

#[cfg(not(target_os = "linux"))]
fn rapl_energy_uj() -> Option<u64> {
    None
}

/// Lower the confidence of peak-reported subtests when the run was thermally
/// limited, and describe why.
///
/// A peak-reported subtest claims to measure what the silicon can do with
/// every core (see [`Representative::Peak`]). That only holds if boost was
/// available: on a machine already heat-soaked when the run started, boost
/// never happens and the "peak" is the sustained figure wearing a peak's
/// label. Measured on a 13th-gen mobile part after six back-to-back runs,
/// `int_multi` peaked at 68567 Mops/s against ~104000 from the same binary on
/// the same machine with thermal headroom — a ~35 % under-read that the
/// run-to-run spread can't see, because every iteration was equally throttled.
/// `Confidence::from_cv` would call that `high`.
///
/// The value can't be corrected after the fact, but it can stop looking
/// authoritative. Returns a note for the result file when anything changed.
///
/// # What this does not catch
///
/// It fires on throttling that *happens during* a run, because that is what
/// `thermal_limited` can see. Two gaps, both measured rather than assumed:
///
/// * **A machine that was already saturated before the run started** never
///   declines — it begins at its capped clock and stays there. Deliberately
///   heat-soaking this machine produced `int_multi` 35 % low with a flat clock
///   trace, and no head-versus-tail test can see that. Telling "a 74k machine"
///   from "a 104k machine that is hot" needs a reference outside the run.
/// * **Platforms that don't report real clocks.** `sysinfo` on Windows gave a
///   1638–1900 MHz range on a part that boosts past 5 GHz, and WSL2 reports a
///   single static figure whatever the load. Where the signal is that flat,
///   nothing here can fire, and the `Clocks` line in the report is similarly
///   uninformative. Bare-metal Linux via cpufreq is the case where it should
///   work.
pub fn downgrade_thermally_limited(
    outcomes: &mut [BenchmarkOutcome],
    telemetry: Option<&RunTelemetry>,
) -> Option<String> {
    let tel = telemetry?;
    if !tel.thermal_limited {
        return None;
    }

    let mut affected = Vec::new();
    for comp in outcomes.iter_mut() {
        for st in comp.subtests.iter_mut() {
            if st.representative == Representative::Peak {
                st.confidence = st.confidence.downgraded();
                affected.push(format!("{}/{}", comp.id, st.id));
            }
        }
    }
    if affected.is_empty() {
        return None;
    }

    Some(format!(
        "clocks fell over the run ({:.0} -> {:.0} MHz mean), so peak-reported          subtests may never have reached boost — confidence lowered for {}",
        tel.mhz_start,
        tel.mhz_end,
        affected.join(", "),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression that mattered: a busy core decaying from boost while the
    /// all-CPU mean barely budges, because the idle cores dominate it. Real
    /// shape from a 20-thread machine running mostly single-threaded subtests.
    #[test]
    fn throttling_is_seen_on_the_busiest_core_not_the_mean() {
        let mean_series: Vec<f64> = (0..20).map(|i| 1780.0 - i as f64).collect();
        let busiest: Vec<f64> = (0..20)
            .map(|i| if i < 6 { 3900.0 } else { 2500.0 })
            .collect();

        let old_way = RunTelemetry::from_samples(&mean_series, &mean_series, None);
        assert!(
            !old_way.thermal_limited,
            "the all-CPU mean moves too little to notice — this is the bug"
        );

        let fixed = RunTelemetry::from_samples(&mean_series, &busiest, None);
        assert!(
            fixed.thermal_limited,
            "the busiest core fell from 3900 to 2500 MHz and should be flagged"
        );
        // Reported clocks keep their mean-across-CPUs meaning.
        assert_eq!(fixed.mhz_start, 1780.0);
    }

    #[test]
    fn a_busiest_series_that_holds_its_clock_is_not_throttling() {
        let mean_series: Vec<f64> = vec![1780.0; 20];
        let busiest: Vec<f64> = vec![3900.0; 20];
        assert!(!RunTelemetry::from_samples(&mean_series, &busiest, None).thermal_limited);
    }

    use crate::engine::stats::{Confidence, Stats};
    use crate::engine::{Direction, SubtestOutcome};

    fn outcome(id: &str, rep: Representative, conf: Confidence) -> BenchmarkOutcome {
        BenchmarkOutcome {
            id: "cpu".into(),
            label: "CPU".into(),
            subtests: vec![SubtestOutcome {
                id: id.into(),
                label: id.into(),
                unit: "Mops/s".into(),
                direction: Direction::HigherIsBetter,
                value: 100.0,
                representative: rep,
                stats: Stats::from_runs(vec![100.0, 100.0, 100.0]),
                confidence: conf,
                scored: true,
            }],
            notes: vec![],
        }
    }

    fn throttled(limited: bool) -> RunTelemetry {
        RunTelemetry {
            source: "sysinfo".into(),
            sample_count: 40,
            mhz_start: 3800.0,
            mhz_end: 2400.0,
            mhz_min: 2300.0,
            mhz_max: 3900.0,
            mhz_mean: 3000.0,
            thermal_limited: limited,
            package_watts_mean: None,
        }
    }

    #[test]
    fn a_throttled_run_lowers_confidence_on_peak_subtests() {
        let mut o = vec![outcome("int_multi", Representative::Peak, Confidence::High)];
        let note = downgrade_thermally_limited(&mut o, Some(&throttled(true)));
        assert_eq!(o[0].subtests[0].confidence, Confidence::Medium);
        let note = note.expect("a downgrade should explain itself");
        assert!(note.contains("int_multi"), "{note}");
        assert!(note.contains("boost"), "{note}");
    }

    #[test]
    fn median_subtests_are_left_alone() {
        // Single-core work doesn't reach the package limit, so its median is
        // still the honest answer on a throttled machine.
        let mut o = vec![outcome(
            "int_single",
            Representative::Median,
            Confidence::High,
        )];
        assert!(downgrade_thermally_limited(&mut o, Some(&throttled(true))).is_none());
        assert_eq!(o[0].subtests[0].confidence, Confidence::High);
    }

    #[test]
    fn an_unthrottled_run_changes_nothing() {
        let mut o = vec![outcome("int_multi", Representative::Peak, Confidence::High)];
        assert!(downgrade_thermally_limited(&mut o, Some(&throttled(false))).is_none());
        assert_eq!(o[0].subtests[0].confidence, Confidence::High);
    }

    #[test]
    fn no_telemetry_means_no_opinion() {
        let mut o = vec![outcome("int_multi", Representative::Peak, Confidence::High)];
        assert!(downgrade_thermally_limited(&mut o, None).is_none());
        assert_eq!(o[0].subtests[0].confidence, Confidence::High);
    }

    #[test]
    fn low_confidence_is_the_floor() {
        let mut o = vec![outcome("int_multi", Representative::Peak, Confidence::Low)];
        assert!(downgrade_thermally_limited(&mut o, Some(&throttled(true))).is_some());
        assert_eq!(o[0].subtests[0].confidence, Confidence::Low);
    }

    #[test]
    fn empty_series_is_unavailable() {
        let t = RunTelemetry::from_samples(&[], &[], None);
        assert_eq!(t.source, "unavailable");
        assert_eq!(t.sample_count, 0);
        assert!(t.summary().is_none());
    }

    #[test]
    fn a_steady_series_is_not_flagged() {
        let mhz: Vec<f64> = vec![3800.0; 20];
        let t = RunTelemetry::from_samples(&mhz, &mhz, None);
        assert!(!t.thermal_limited);
        assert_eq!(t.source, "sysinfo");
        assert_eq!(t.mhz_min, 3800.0);
        assert!(t.summary().unwrap().contains("steady"));
    }

    #[test]
    fn a_sustained_decline_trips_thermal_limited() {
        let mut mhz: Vec<f64> = vec![4200.0; 8];
        mhz.extend(std::iter::repeat_n(3100.0, 8));
        let t = RunTelemetry::from_samples(&mhz, &mhz, Some(28.0));
        assert!(t.thermal_limited);
        assert_eq!(t.source, "sysinfo+rapl");
        assert_eq!(t.package_watts_mean, Some(28.0));
        assert!(t.summary().unwrap().contains("thermally limited"));
    }

    #[test]
    fn a_brief_dip_below_the_sample_floor_is_not_flagged() {
        // Fewer than MIN_SAMPLES_FOR_THROTTLE: not enough signal.
        let mhz = vec![4000.0, 4000.0, 2000.0, 2000.0];
        assert!(!RunTelemetry::from_samples(&mhz, &mhz, None).thermal_limited);
    }

    #[test]
    fn disabled_sampler_yields_nothing() {
        assert!(Sampler::start(false).finish().is_none());
    }

    #[test]
    fn live_sampler_returns_without_panicking() {
        let s = Sampler::start(true);
        std::thread::sleep(Duration::from_millis(700));
        let t = s.finish().expect("enabled sampler returns telemetry");
        assert!(t.sample_count > 0 || t.source == "unavailable");
    }
}
