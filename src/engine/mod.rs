//! The benchmark engine: the [`Benchmark`] trait every subsystem implements, the
//! scheduler that runs warmup + timed iterations and summarises them, and the
//! [`Progress`] hook the CLI and (later) the TUI render from.

pub mod progress;
pub mod stats;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub use progress::{Progress, ProgressEvent};
use stats::{Confidence, Stats};

/// Which direction of a raw measurement counts as "better". Used later by the
/// scoring stage to orient the baseline ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    HigherIsBetter,
    LowerIsBetter,
}

/// Static description of one measurement within a benchmark.
#[derive(Debug, Clone)]
pub struct SubtestSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub direction: Direction,
}

/// Thoroughness preset. Controls iteration counts and the per-run time budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationPreset {
    Short,
    Normal,
    Thorough,
}

impl DurationPreset {
    pub fn name(self) -> &'static str {
        match self {
            DurationPreset::Short => "short",
            DurationPreset::Normal => "normal",
            DurationPreset::Thorough => "thorough",
        }
    }

    pub fn warmup_runs(self) -> u32 {
        match self {
            DurationPreset::Short => 1,
            DurationPreset::Normal => 1,
            DurationPreset::Thorough => 2,
        }
    }

    pub fn timed_runs(self) -> u32 {
        match self {
            DurationPreset::Short => 3,
            DurationPreset::Normal => 5,
            DurationPreset::Thorough => 9,
        }
    }

    /// Wall-clock budget for a single timed run of a subtest.
    pub fn time_budget(self) -> Duration {
        match self {
            DurationPreset::Short => Duration::from_millis(350),
            DurationPreset::Normal => Duration::from_millis(800),
            DurationPreset::Thorough => Duration::from_millis(1500),
        }
    }

    /// Multiplier applied to per-subtest working-set sizing.
    pub fn workload_scale(self) -> f64 {
        match self {
            DurationPreset::Short => 0.5,
            DurationPreset::Normal => 1.0,
            DurationPreset::Thorough => 2.0,
        }
    }
}

/// Everything a subtest needs to size and run its workload.
pub struct RunContext {
    pub preset: DurationPreset,
    pub seed: u64,
    /// Where the disk benchmark places its scratch file.
    pub target_dir: PathBuf,
    /// Logical CPU count, used by "all cores" subtests.
    pub threads: usize,
    /// Total system RAM in bytes; caps the memory benchmark's working set.
    pub total_ram: u64,
    /// Explicit override for the number of timed runs; falls back to the preset.
    pub runs_override: Option<u32>,
    /// Set by the caller (e.g. the TUI on quit) to stop the run between subtests.
    pub abort: Arc<AtomicBool>,
}

impl RunContext {
    pub fn time_budget(&self) -> Duration {
        self.preset.time_budget()
    }

    fn aborted(&self) -> bool {
        self.abort.load(Ordering::Relaxed)
    }

    fn timed_runs(&self) -> u32 {
        self.runs_override
            .unwrap_or_else(|| self.preset.timed_runs())
            .max(1)
    }
}

/// One measurable subsystem (CPU, memory, disk). Implementations own their
/// workloads; the engine owns scheduling and statistics. `Send + Sync` so the
/// TUI can drive a run from a worker thread.
pub trait Benchmark: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn subtests(&self) -> Vec<SubtestSpec>;
    /// Execute one timed run of `subtest_id` and return its measured value in the
    /// subtest's natural unit (a rate for throughput tests, a time for latency).
    fn run_subtest(&self, subtest_id: &str, ctx: &RunContext) -> Result<f64>;

    /// Caveats gathered while running (e.g. "reads were buffered"). Collected by
    /// the engine after all subtests complete. Default: none.
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }

    /// True for subtests that run on one thread and should therefore be pinned
    /// to a single core, so the scheduler can't bounce the measurement between
    /// core types (P/E cores, big.LITTLE) mid-run. Default: false.
    fn single_threaded(&self, _subtest_id: &str) -> bool {
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubtestOutcome {
    pub id: String,
    pub label: String,
    pub unit: String,
    pub direction: Direction,
    /// Representative value for scoring (the run median).
    pub value: f64,
    pub stats: Stats,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkOutcome {
    pub id: String,
    pub label: String,
    pub subtests: Vec<SubtestOutcome>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Run every subtest of `bench`: warmup iterations (discarded) then timed
/// iterations, summarised into a [`BenchmarkOutcome`].
pub fn run_benchmark(
    bench: &dyn Benchmark,
    ctx: &RunContext,
    progress: &mut dyn Progress,
) -> Result<BenchmarkOutcome> {
    let specs = bench.subtests();
    let timed = ctx.timed_runs();
    log::info!(
        target: "loadbearer::engine",
        "benchmark {} start: {} subtest(s), {} warmup + {} timed run(s)",
        bench.id(),
        specs.len(),
        ctx.preset.warmup_runs(),
        timed,
    );
    let started = std::time::Instant::now();
    progress.on_event(ProgressEvent::BenchStart {
        id: bench.id(),
        label: bench.label(),
        subtests: specs.len(),
    });

    let mut subtests = Vec::with_capacity(specs.len());
    for spec in &specs {
        if ctx.aborted() {
            log::warn!(target: "loadbearer::engine", "{} aborted before subtest {}", bench.id(), spec.id);
            bail!("run aborted");
        }
        progress.on_event(ProgressEvent::SubtestStart {
            id: spec.id,
            label: spec.label,
            timed_runs: timed,
        });

        let pinned = bench.single_threaded(spec.id);
        log::debug!(
            target: "loadbearer::engine",
            "subtest {}/{} start ({})",
            bench.id(), spec.id,
            if pinned { "pinned, single-thread" } else { "unpinned" },
        );
        for _ in 0..ctx.preset.warmup_runs() {
            progress.on_event(ProgressEvent::Warmup { id: spec.id });
            run_one(bench, spec.id, ctx, pinned)?;
        }

        let mut runs = Vec::with_capacity(timed as usize);
        for r in 1..=timed {
            let value = run_one(bench, spec.id, ctx, pinned)?;
            runs.push(value);
            log::trace!(
                target: "loadbearer::engine",
                "{}/{} run {}/{}: {:.3} {}", bench.id(), spec.id, r, timed, value, spec.unit,
            );
            progress.on_event(ProgressEvent::Run {
                id: spec.id,
                run: r,
                of: timed,
                value,
                unit: spec.unit,
            });
        }

        let stats = Stats::from_runs(runs);
        let confidence = Confidence::from_cv(stats.cv);
        log::debug!(
            target: "loadbearer::engine",
            "subtest {}/{} done: median {:.3} {} (cv {:.1}%, {})",
            bench.id(), spec.id, stats.median, spec.unit, stats.cv * 100.0, confidence.as_str(),
        );
        let outcome = SubtestOutcome {
            id: spec.id.to_string(),
            label: spec.label.to_string(),
            unit: spec.unit.to_string(),
            direction: spec.direction,
            value: stats.median,
            stats,
            confidence,
        };
        progress.on_event(ProgressEvent::SubtestDone {
            id: spec.id,
            outcome: &outcome,
        });
        subtests.push(outcome);
    }

    log::info!(
        target: "loadbearer::engine",
        "benchmark {} done in {:.1}s",
        bench.id(),
        started.elapsed().as_secs_f64(),
    );
    progress.on_event(ProgressEvent::BenchDone { id: bench.id() });
    Ok(BenchmarkOutcome {
        id: bench.id().to_string(),
        label: bench.label().to_string(),
        subtests,
        notes: bench.notes(),
    })
}

/// Run one iteration of a subtest. A `pinned` (single-threaded) subtest runs on
/// a throwaway thread pinned to one consistent core, so the OS scheduler can't
/// move the measurement between core types (P/E cores, big.LITTLE) part-way
/// through and wreck its run-to-run stability. The thread is disposable, so
/// there is nothing to un-pin afterwards.
fn run_one(bench: &dyn Benchmark, id: &str, ctx: &RunContext, pinned: bool) -> Result<f64> {
    if !pinned {
        return bench.run_subtest(id, ctx);
    }
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| {
            if let Some(core) = measurement_core() {
                core_affinity::set_for_current(core);
            }
            bench.run_subtest(id, ctx)
        });
        match handle.join() {
            Ok(result) => result,
            Err(_) => {
                log::error!(target: "loadbearer::engine", "measurement thread for {id} panicked");
                bail!("measurement thread for {id} panicked")
            }
        }
    })
}

/// The core to pin single-threaded measurements to — chosen once. On Linux this
/// is the CPU with the highest rated frequency (a performance core on hybrid
/// parts); elsewhere it's simply the first core the OS reports. The point is
/// consistency: every iteration of a subtest runs on the *same* core.
fn measurement_core() -> Option<core_affinity::CoreId> {
    use std::sync::OnceLock;
    static CORE: OnceLock<Option<core_affinity::CoreId>> = OnceLock::new();
    *CORE.get_or_init(|| {
        let Some(ids) = core_affinity::get_core_ids() else {
            log::warn!(
                target: "loadbearer::engine",
                "core affinity unavailable — single-threaded subtests are not pinned",
            );
            return None;
        };
        #[cfg(target_os = "linux")]
        if let Some(best) = linux_fastest_cpu()
            && let Some(id) = ids.iter().copied().find(|c| c.id == best)
        {
            log::debug!(target: "loadbearer::engine", "pinning single-threaded subtests to CPU {}", id.id);
            return Some(id);
        }
        let chosen = ids.into_iter().next();
        log::debug!(target: "loadbearer::engine", "pinning single-threaded subtests to CPU {:?}", chosen.map(|c| c.id));
        chosen
    })
}

#[cfg(target_os = "linux")]
fn linux_fastest_cpu() -> Option<usize> {
    let mut best: Option<(usize, u64)> = None;
    for entry in std::fs::read_dir("/sys/devices/system/cpu").ok()?.flatten() {
        let name = entry.file_name();
        let Some(idx) = name
            .to_string_lossy()
            .strip_prefix("cpu")
            .and_then(|s| s.parse::<usize>().ok())
        else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(entry.path().join("cpufreq/cpuinfo_max_freq"))
        else {
            continue;
        };
        let Ok(khz) = text.trim().parse::<u64>() else {
            continue;
        };
        if best.is_none_or(|(_, b)| khz > b) {
            best = Some((idx, khz));
        }
    }
    best.map(|(idx, _)| idx)
}

/// Repeatedly invoke `unit_work` until `budget` elapses, then return the number
/// of work units completed per second. `unit_work` returns the count of work
/// units performed by that call (usually a fixed batch size).
///
/// This "fixed time, count work" shape keeps a subtest well-scaled across
/// machines that differ in speed by an order of magnitude.
pub fn throughput<F>(budget: Duration, mut unit_work: F) -> f64
where
    F: FnMut() -> u64,
{
    let start = Instant::now();
    let mut units: u64 = 0;
    loop {
        units += unit_work();
        if start.elapsed() >= budget {
            break;
        }
    }
    let secs = start.elapsed().as_secs_f64();
    units as f64 / secs
}

/// Run `f` on `threads` OS threads concurrently and sum their results. Used by
/// "all cores" subtests to produce an aggregate throughput figure.
pub fn parallel_sum<F>(threads: usize, f: F) -> f64
where
    F: Fn() -> f64 + Sync,
{
    let threads = threads.max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads).map(|_| scope.spawn(&f)).collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    })
}
