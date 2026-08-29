//! Thermal / sustained-load soak test.
//!
//! The graded benchmarks are all short bursts — a few hundred milliseconds each.
//! That measures a machine at (or near) its boost clocks and tells you nothing
//! about what it does once the thermal mass saturates and the power limit bites.
//! For a thin-and-light that difference is the whole question: two laptops can
//! post identical burst numbers and then diverge hard 30–60 s into a real
//! workload.
//!
//! This module runs a blended integer + floating-point kernel on every logical
//! CPU for a fixed wall-clock stretch (default 90 s), samples aggregate
//! throughput once a second, and reports:
//!
//! - **peak** — the best rolling 3-sample throughput (an unthrottled few seconds)
//! - **steady** — the mean of the final quarter of the run
//! - **retained %** — steady / peak; the headline number
//! - **throttle onset** — when throughput first drops below 95 % of peak and
//!   stays there
//! - **stability** — the coefficient of variation of the steady window, which
//!   catches a machine that is hunting around a power limit rather than holding
//!   a flat clock
//!
//! It also samples CPU frequency (via `sysinfo`) alongside throughput, as a
//! corroborating signal. It is **not scored** — like the `--net-target` link
//! probe, it is measured and shown but kept out of any grade.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sysinfo::System;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::cli::SoakArgs;
use crate::inventory::Inventory;
use crate::output;

/// Default sustained-load duration when `--duration` is not given.
pub const DEFAULT_SOAK_SECS: u64 = 90;
/// Accepted `--duration` range, in seconds.
pub const MIN_SOAK_SECS: u64 = 15;
pub const MAX_SOAK_SECS: u64 = 1800;
const DEFAULT_SEED: u64 = 0x5EED_1234_ABCD_0001;

/// Resolve an optional `--duration` (seconds) into a clamped [`Duration`].
pub fn resolve_duration(secs: Option<u64>) -> Duration {
    Duration::from_secs(
        secs.unwrap_or(DEFAULT_SOAK_SECS)
            .clamp(MIN_SOAK_SECS, MAX_SOAK_SECS),
    )
}

/// Run a soak with the standard stderr progress: an intro line, the live
/// per-sample line, and a closing newline. `quiet` suppresses all three (for
/// `--json`). Returns the analysed result.
pub fn run_with_progress(cfg: &SoakConfig, quiet: bool) -> SoakResult {
    let abort = AtomicBool::new(false);
    if !quiet {
        eprintln!(
            "soak — {} thread(s), {}s sustained all-core load \
             (blended int + float; not scored)",
            cfg.threads,
            cfg.duration.as_secs(),
        );
    }
    let result = run(cfg, &abort, |s| {
        if !quiet {
            live_line(s);
        }
    });
    if !quiet {
        eprintln!();
    }
    result
}

/// Schema tag for a standalone soak JSON document (`loadbearer soak --output`).
pub const SOAK_SCHEMA: &str = "loadbearer.soak/1";

/// Human label for the blended work unit the kernel counts.
pub const SOAK_UNIT: &str = "Mops/s";

const LANES: usize = 8;
/// Inner iterations per batch. Sized so one batch is roughly 100–300 µs on a
/// current core: fine-grained enough for prompt cancellation, coarse enough that
/// the per-batch atomic store and clock check are free.
const BATCH_INNER: u64 = 1 << 16;
/// Blended operations counted per lane per inner iteration: 4 integer + 3 FP.
const OPS_PER_LANE_ITER: u64 = 7;

/// Parameters for one soak run.
#[derive(Debug, Clone)]
pub struct SoakConfig {
    pub duration: Duration,
    pub threads: usize,
    pub seed: u64,
}

/// One per-second observation during a soak run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SoakSample {
    /// Elapsed seconds from the start of the load when this sample was taken.
    pub t_secs: f64,
    /// Aggregate blended throughput over the interval since the previous sample,
    /// in millions of operations per second.
    pub rate: f64,
    /// Mean CPU frequency across logical CPUs in MHz, or 0 if unavailable.
    pub mhz: u64,
}

/// The full outcome of a soak run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoakResult {
    pub duration_secs: f64,
    pub threads: usize,
    pub unit: String,
    pub samples: Vec<SoakSample>,
    /// Best rolling 3-sample throughput seen (an unthrottled few seconds).
    pub peak_rate: f64,
    /// `(from, to)` elapsed-seconds bounds of the peak window.
    pub peak_window: (f64, f64),
    /// Mean throughput over the final quarter of the run.
    pub steady_rate: f64,
    pub steady_window: (f64, f64),
    /// `steady_rate / peak_rate`, as a percentage.
    pub retained_pct: f64,
    /// Elapsed seconds at which throughput first fell below 95 % of peak and
    /// stayed there; `None` if it never did.
    pub onset_secs: Option<f64>,
    /// Coefficient of variation of the steady window, as a percentage.
    pub steady_cv_pct: f64,
    /// Highest per-sample mean frequency seen (MHz), 0 if unavailable.
    pub mhz_peak: u64,
    /// Mean frequency over the steady window (MHz), 0 if unavailable.
    pub mhz_steady: u64,
}

impl SoakResult {
    /// A short verdict line for the report footer.
    pub fn verdict(&self) -> String {
        match self.onset_secs {
            _ if self.retained_pct >= 97.0 => format!(
                "holds {:.0}% of peak — no meaningful throttling in {:.0}s",
                self.retained_pct, self.duration_secs
            ),
            Some(t) => format!(
                "throttles from ~{t:.0}s; settles at {:.0}% of peak",
                self.retained_pct
            ),
            None => format!(
                "drifts down to {:.0}% of peak without a clear onset",
                self.retained_pct
            ),
        }
    }
}

/// A standalone soak document, written by `loadbearer soak --output`. A full
/// `loadbearer run --soak` embeds the same [`SoakResult`] in its result file
/// instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoakDocument {
    pub schema: String,
    pub tool_version: String,
    pub timestamp: String,
    pub machine: Inventory,
    pub soak: SoakResult,
}

/// `loadbearer soak` — run the sustained-load test on its own.
pub fn execute(args: SoakArgs) -> Result<()> {
    let threads = args
        .threads
        .filter(|&t| t > 0)
        .unwrap_or_else(|| thread::available_parallelism().map_or(1, |n| n.get()));
    let cfg = SoakConfig {
        duration: resolve_duration(args.duration),
        threads,
        seed: args.seed.unwrap_or(DEFAULT_SEED),
    };

    let machine = crate::inventory::collect();
    let result = run_with_progress(&cfg, args.json);

    let doc = SoakDocument {
        schema: SOAK_SCHEMA.to_string(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default(),
        machine,
        soak: result,
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&doc)?);
    } else {
        output::print_soak_report(&doc.machine, &doc.soak);
    }
    if let Some(path) = &args.output {
        let json = serde_json::to_string_pretty(&doc)?;
        std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
        if !args.json {
            eprintln!("\nsoak result written to {}", path.display());
        }
    }
    Ok(())
}

/// Print a carriage-return-updated progress line to stderr. A no-op when stderr
/// is not a terminal, so piped/redirected output stays clean — the final report
/// carries the result.
pub fn live_line(s: &SoakSample) {
    use std::io::IsTerminal;
    if !std::io::stderr().is_terminal() {
        return;
    }
    let clock = if s.mhz > 0 {
        format!("   {:.2} GHz", s.mhz as f64 / 1000.0)
    } else {
        String::new()
    };
    eprint!(
        "\r  {:>4.0}s   {:>11.0} {SOAK_UNIT}{clock}        ",
        s.t_secs, s.rate
    );
    let _ = std::io::stderr().flush();
}

/// Run the sustained load and return the analysed result. `on_sample` is called
/// once per completed sampling interval (roughly once a second). `abort` stops
/// the run early at the next batch boundary.
pub fn run(
    cfg: &SoakConfig,
    abort: &AtomicBool,
    mut on_sample: impl FnMut(&SoakSample),
) -> SoakResult {
    let threads = cfg.threads.max(1);
    log::info!(
        target: "loadbearer::soak",
        "sustained-load start: {}s, {} thread(s), seed {:#018x}",
        cfg.duration.as_secs(), threads, cfg.seed,
    );
    let counters: Vec<AtomicU64> = (0..threads).map(|_| AtomicU64::new(0)).collect();
    let start = Instant::now();
    let deadline = start + cfg.duration;

    let samples = thread::scope(|scope| {
        for (t, counter) in counters.iter().enumerate() {
            let seed = cfg.seed ^ (t as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            scope.spawn(move || worker(counter, seed, abort, deadline));
        }

        let mut sys = System::new();
        sys.refresh_cpu_all();
        let mut samples: Vec<SoakSample> = Vec::new();
        let mut last_total = 0u64;
        let mut last_t = 0.0f64;
        loop {
            sleep_until_sample(Duration::from_secs(1), abort, deadline);
            let now = start.elapsed().as_secs_f64();
            let total: u64 = counters.iter().map(|c| c.load(Ordering::Relaxed)).sum();
            let dt = now - last_t;
            if dt >= 0.2 {
                let rate = (total.saturating_sub(last_total) as f64 / dt) / 1e6;
                let sample = SoakSample {
                    t_secs: now,
                    rate,
                    mhz: spot_mhz(&mut sys),
                };
                on_sample(&sample);
                samples.push(sample);
                last_total = total;
                last_t = now;
            }
            if abort.load(Ordering::Relaxed) || Instant::now() >= deadline {
                break;
            }
        }
        samples
    });

    derive(cfg, threads, samples)
}

fn worker(counter: &AtomicU64, seed: u64, abort: &AtomicBool, deadline: Instant) {
    let mut istate = int_lanes(seed);
    let mut fstate = float_lanes(seed);
    let mut local: u64 = 0;
    loop {
        local = local.wrapping_add(soak_batch(&mut istate, &mut fstate));
        counter.store(local, Ordering::Relaxed);
        if abort.load(Ordering::Relaxed) || Instant::now() >= deadline {
            break;
        }
    }
}

/// One batch of blended work. Each lane threads an LCG/xorshift integer chain
/// and feeds a slice of its state into a multiply-add FP chain, so neither the
/// integer nor the float work can be optimised away independently. Returns the
/// blended operation count for the batch (bookkeeping only).
fn soak_batch(istate: &mut [u64; LANES], fstate: &mut [f64; LANES]) -> u64 {
    let c1 = 1.000_000_191_8f64;
    let c2 = 0.999_999_937_3f64;
    for _ in 0..BATCH_INNER {
        for (x_slot, f_slot) in istate.iter_mut().zip(fstate.iter_mut()) {
            let mut x = *x_slot;
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            x ^= x >> 27;
            x = x.wrapping_mul(0x2545F4914F6CDD1D);
            *x_slot = x;
            *f_slot = *f_slot * c1 + c2 + ((x >> 40) as f64) * 1e-18;
        }
    }
    std::hint::black_box(*istate);
    std::hint::black_box(*fstate);
    BATCH_INNER * LANES as u64 * OPS_PER_LANE_ITER
}

fn int_lanes(seed: u64) -> [u64; LANES] {
    let mut lanes = [0u64; LANES];
    for (i, lane) in lanes.iter_mut().enumerate() {
        *lane = seed
            .wrapping_add((i as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
            .wrapping_add(0x9E37_79B9_7F4A_7C15);
    }
    lanes
}

fn float_lanes(seed: u64) -> [f64; LANES] {
    let mut lanes = [0.0f64; LANES];
    for (i, lane) in lanes.iter_mut().enumerate() {
        *lane = 1.0 + ((seed >> (i * 3)) & 0xff) as f64 * 1e-3;
    }
    lanes
}

fn spot_mhz(sys: &mut System) -> u64 {
    sys.refresh_cpu_frequency();
    let cpus = sys.cpus();
    if cpus.is_empty() {
        return 0;
    }
    let sum: u64 = cpus.iter().map(|c| c.frequency()).sum();
    sum / cpus.len() as u64
}

/// Sleep up to `step`, waking early if `abort` is set or `deadline` passes.
fn sleep_until_sample(step: Duration, abort: &AtomicBool, deadline: Instant) {
    let target = Instant::now() + step;
    while Instant::now() < target {
        if abort.load(Ordering::Relaxed) || Instant::now() >= deadline {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn derive(cfg: &SoakConfig, threads: usize, samples: Vec<SoakSample>) -> SoakResult {
    let n = samples.len();
    let (peak_rate, peak_window) = rolling_peak(&samples);

    let sw = (n / 4).max(3).min(n.max(1));
    let steady = &samples[n.saturating_sub(sw)..];
    let steady_rates: Vec<f64> = steady.iter().map(|s| s.rate).collect();
    let steady_rate = mean(&steady_rates);
    let steady_window = match (steady.first(), steady.last()) {
        (Some(a), Some(b)) => (a.t_secs, b.t_secs),
        _ => (0.0, 0.0),
    };
    let steady_cv_pct = cv(&steady_rates) * 100.0;
    let retained_pct = if peak_rate > 0.0 {
        100.0 * steady_rate / peak_rate
    } else {
        100.0
    };
    let onset_secs = throttle_onset(&samples, peak_rate, steady_rate);

    let mhz_peak = samples
        .iter()
        .map(|s| s.mhz)
        .filter(|&m| m > 0)
        .max()
        .unwrap_or(0);
    let steady_mhz: Vec<u64> = steady.iter().map(|s| s.mhz).filter(|&m| m > 0).collect();
    let mhz_steady = if steady_mhz.is_empty() {
        0
    } else {
        steady_mhz.iter().sum::<u64>() / steady_mhz.len() as u64
    };

    log::info!(
        target: "loadbearer::soak",
        "sustained-load done: {} sample(s), peak {:.0}, steady {:.0} {}, {:.1}% retained, onset {}",
        n, peak_rate, steady_rate, SOAK_UNIT, retained_pct,
        onset_secs.map_or_else(|| "none".to_string(), |t| format!("~{t:.0}s")),
    );
    SoakResult {
        duration_secs: cfg.duration.as_secs_f64(),
        threads,
        unit: SOAK_UNIT.to_string(),
        samples,
        peak_rate,
        peak_window,
        steady_rate,
        steady_window,
        retained_pct,
        onset_secs,
        steady_cv_pct,
        mhz_peak,
        mhz_steady,
    }
}

/// Best mean over any window of up to three consecutive samples.
fn rolling_peak(samples: &[SoakSample]) -> (f64, (f64, f64)) {
    if samples.is_empty() {
        return (0.0, (0.0, 0.0));
    }
    let w = 3.min(samples.len());
    let mut best = f64::MIN;
    let mut window = (samples[0].t_secs, samples[samples.len() - 1].t_secs);
    for i in 0..=samples.len() - w {
        let slice = &samples[i..i + w];
        let m = slice.iter().map(|s| s.rate).sum::<f64>() / w as f64;
        if m > best {
            best = m;
            window = (slice[0].t_secs, slice[w - 1].t_secs);
        }
    }
    (best, window)
}

/// First elapsed time at which throughput drops below 95 % of peak and the
/// remainder of the run stays below that line. `None` if the steady state is
/// itself at or above 95 % of peak (i.e. nothing meaningfully throttled).
fn throttle_onset(samples: &[SoakSample], peak: f64, steady: f64) -> Option<f64> {
    if peak <= 0.0 {
        return None;
    }
    let threshold = 0.95 * peak;
    if steady >= threshold {
        return None;
    }
    for i in 1..samples.len() {
        if samples[i].rate < threshold {
            let tail = &samples[i..];
            let tail_mean = tail.iter().map(|s| s.rate).sum::<f64>() / tail.len() as f64;
            if tail_mean < threshold {
                return Some(samples[i].t_secs);
            }
        }
    }
    None
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn cv(xs: &[f64]) -> f64 {
    let m = mean(xs);
    if m <= 0.0 || xs.len() < 2 {
        return 0.0;
    }
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / xs.len() as f64;
    var.sqrt() / m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(t: f64, rate: f64) -> SoakSample {
        SoakSample {
            t_secs: t,
            rate,
            mhz: 0,
        }
    }

    #[test]
    fn short_run_produces_samples_and_positive_rates() {
        let cfg = SoakConfig {
            duration: Duration::from_millis(2200),
            threads: 2,
            seed: 1,
        };
        let abort = AtomicBool::new(false);
        let mut seen = 0;
        let result = run(&cfg, &abort, |_| seen += 1);
        assert!(seen >= 1, "expected at least one sample callback");
        assert!(!result.samples.is_empty());
        assert!(result.samples.iter().all(|s| s.rate > 0.0));
        assert!(result.peak_rate > 0.0);
        assert!(result.steady_rate > 0.0);
    }

    #[test]
    fn abort_stops_the_run_early() {
        let cfg = SoakConfig {
            duration: Duration::from_secs(60),
            threads: 1,
            seed: 1,
        };
        let abort = AtomicBool::new(true);
        let start = Instant::now();
        let _ = run(&cfg, &abort, |_| {});
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "abort was not honoured"
        );
    }

    #[test]
    fn flat_series_retains_and_has_no_onset() {
        let samples: Vec<SoakSample> = (1..=20).map(|i| sample(i as f64, 1000.0)).collect();
        let r = derive(&cfg(30), 8, samples);
        assert!((r.retained_pct - 100.0).abs() < 0.01);
        assert!(r.onset_secs.is_none());
        assert!(r.steady_cv_pct < 0.01);
    }

    #[test]
    fn decaying_series_detects_throttle_onset() {
        // Flat at 1000 for 10 s, then a step down to 700 for 10 s.
        let mut samples: Vec<SoakSample> = (1..=10).map(|i| sample(i as f64, 1000.0)).collect();
        samples.extend((11..=20).map(|i| sample(i as f64, 700.0)));
        let r = derive(&cfg(20), 8, samples);
        assert!(r.peak_rate > 950.0);
        assert!(r.retained_pct < 75.0);
        let onset = r
            .onset_secs
            .expect("a sustained step down should register an onset");
        assert!((10.0..=12.0).contains(&onset), "onset at {onset}s");
    }

    fn cfg(secs: u64) -> SoakConfig {
        SoakConfig {
            duration: Duration::from_secs(secs),
            threads: 8,
            seed: 1,
        }
    }
}
