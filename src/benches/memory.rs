//! Memory benchmark: sequential bandwidth (read, write, copy) and random-access
//! latency.
//!
//! ## Methodology
//!
//! The working set is sized to comfortably exceed any last-level cache (256 MiB
//! at the `normal` preset), so the bandwidth figures reflect DRAM, not cache. On
//! a machine with little RAM the buffer is capped at RAM/8 and a note is
//! recorded, since a capped buffer may partly fit in cache.
//!
//! Bandwidth kernels use eight independent accumulators / a slice fill that the
//! compiler is free to vectorise — the goal is to saturate the memory path.
//! Latency is a pointer chase around a single random cycle (Sattolo), which
//! serialises loads and exposes load-to-use latency to DRAM.
//!
//! Every subtest is single-threaded except the all-core read (`bw_read_mt`),
//! which runs the read kernel on one thread per logical CPU — each on its own
//! buffer (at least 32 MiB, past any per-core slice of a shared L3) — and sums
//! the per-thread rates. The threads fill their buffers, then meet at a barrier
//! so the timed window starts on every core at once.

use std::hint::black_box;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::engine::{Benchmark, Direction, RunContext, SubtestSpec, parallel_sum, throughput};
use crate::util::{GIB, SplitMix64};

/// Target working-set size before the RAM/8 cap, per preset.
fn target_bytes(ctx: &RunContext) -> usize {
    (256.0 * 1024.0 * 1024.0 * ctx.preset.workload_scale()) as usize
}

/// Actual working set: `target`, capped at RAM/8, floored at 16 MiB, rounded to
/// a multiple of 8 KiB.
fn working_bytes(ctx: &RunContext) -> (usize, bool) {
    let target = target_bytes(ctx);
    let cap = (ctx.total_ram / 8).max(16 * 1024 * 1024) as usize;
    let capped = target > cap;
    let chosen = target.min(cap) & !(8 * 1024 - 1);
    (chosen.max(8 * 1024), capped)
}

/// Per-thread working set for the all-core read: the single-thread size split
/// across the threads (floored at 32 MiB each — comfortably past any per-core
/// slice of a shared L3), with the total held to RAM/4.
fn mt_read_bytes(ctx: &RunContext) -> (usize, bool) {
    let threads = ctx.threads.max(1);
    let (single, _) = working_bytes(ctx);
    let mut per = (single / threads).max(32 * 1024 * 1024);
    let mut capped = false;
    let cap_total = (ctx.total_ram / 4) as usize;
    if per.saturating_mul(threads) > cap_total {
        per = (cap_total / threads).max(8 * 1024);
        capped = true;
    }
    (per & !(8 * 1024 - 1), capped)
}

pub struct MemoryBenchmark {
    notes: Mutex<Vec<String>>,
}

impl MemoryBenchmark {
    pub fn new() -> Self {
        Self {
            notes: Mutex::new(Vec::new()),
        }
    }

    fn note(&self, msg: impl Into<String>) {
        let msg = msg.into();
        let mut guard = self.notes.lock().unwrap();
        if !guard.contains(&msg) {
            guard.push(msg);
        }
    }
}

impl Benchmark for MemoryBenchmark {
    fn id(&self) -> &'static str {
        "memory"
    }

    fn label(&self) -> &'static str {
        "Memory"
    }

    fn subtests(&self) -> Vec<SubtestSpec> {
        use Direction::{HigherIsBetter as Hi, LowerIsBetter as Lo};
        vec![
            SubtestSpec {
                id: "bw_read",
                label: "Sequential read",
                unit: "GiB/s",
                direction: Hi,
            },
            SubtestSpec {
                id: "bw_write",
                label: "Sequential write",
                unit: "GiB/s",
                direction: Hi,
            },
            SubtestSpec {
                id: "bw_copy",
                label: "Copy (memcpy)",
                unit: "GiB/s",
                direction: Hi,
            },
            SubtestSpec {
                id: "bw_read_mt",
                label: "Sequential read, all cores",
                unit: "GiB/s",
                direction: Hi,
            },
            SubtestSpec {
                id: "latency",
                label: "Random access latency",
                unit: "ns",
                direction: Lo,
            },
        ]
    }

    fn run_subtest(&self, subtest_id: &str, ctx: &RunContext) -> Result<f64> {
        let budget = ctx.time_budget();
        let (bytes, capped) = working_bytes(ctx);
        if capped {
            self.note(format!(
                "working set capped at {} MiB (RAM/8); bandwidth may include cache effects",
                bytes / (1024 * 1024)
            ));
        }
        let words = bytes / 8;

        Ok(match subtest_id {
            "bw_read" => {
                let buf = filled_words(words, ctx.seed ^ 0xAA);
                read_bandwidth(&buf, budget)
            }
            "bw_write" => {
                let mut buf = vec![0u64; words];
                write_bandwidth(&mut buf, budget)
            }
            "bw_copy" => {
                let src = filled_words(words, ctx.seed ^ 0xCC);
                let mut dst = vec![0u64; words];
                copy_bandwidth(&src, &mut dst, budget)
            }
            "bw_read_mt" => {
                let (per, mt_capped) = mt_read_bytes(ctx);
                if mt_capped {
                    self.note("all-core read working set reduced to fit within RAM/4");
                }
                let threads = ctx.threads.max(1);
                let mt_words = per / 8;
                let seed = ctx.seed ^ 0xB0;
                // Every thread allocates and fills its own buffer, then waits at
                // the barrier so all cores enter the timed read together. Without
                // it, a thread that finished filling early would measure the
                // first slice of its window against a machine not yet fully
                // loaded, inflating the aggregate — worst at the `short` preset.
                let start = std::sync::Barrier::new(threads);
                parallel_sum(threads, || {
                    let buf = filled_words(mt_words, seed);
                    start.wait();
                    read_bandwidth(&buf, budget)
                })
            }
            "latency" => {
                // 4 bytes per node; size independent of the bandwidth working set.
                let nodes = (bytes / 4).max(1 << 20);
                let cycle = sattolo_cycle(nodes, ctx.seed ^ 0xEE);
                latency_ns(&cycle, budget)
            }
            other => bail!("unknown memory subtest: {other}"),
        })
    }

    fn notes(&self) -> Vec<String> {
        self.notes.lock().unwrap().clone()
    }

    /// Every memory subtest is single-threaded except the all-core read.
    fn single_threaded(&self, subtest_id: &str) -> bool {
        subtest_id != "bw_read_mt"
    }
}

fn filled_words(words: usize, seed: u64) -> Vec<u64> {
    let mut rng = SplitMix64::new(seed);
    let mut v = vec![0u64; words];
    for w in v.iter_mut() {
        *w = rng.next_u64();
    }
    v
}

/// Sum the buffer with eight independent lanes; report GiB/s of data read.
fn read_bandwidth(buf: &[u64], budget: Duration) -> f64 {
    let bytes_per_pass = (buf.len() * 8) as u64;
    let rate = throughput(budget, || {
        let mut acc = [0u64; 8];
        let (lanes, remainder) = buf.as_chunks::<8>();
        for c in lanes {
            for i in 0..8 {
                acc[i] = acc[i].wrapping_add(c[i]);
            }
        }
        let mut tail = 0u64;
        for &x in remainder {
            tail = tail.wrapping_add(x);
        }
        black_box((acc, tail));
        bytes_per_pass
    });
    rate / GIB
}

/// Fill the buffer with a loop-varying value; report GiB/s of data written.
fn write_bandwidth(buf: &mut [u64], budget: Duration) -> f64 {
    let bytes_per_pass = (buf.len() * 8) as u64;
    let mut tick = 0u64;
    let rate = throughput(budget, || {
        tick = tick.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let v = tick;
        for x in buf.iter_mut() {
            *x = v;
        }
        black_box(buf.as_ptr());
        bytes_per_pass
    });
    rate / GIB
}

/// `dst.copy_from_slice(src)`; report GiB/s of payload copied (memcpy convention,
/// i.e. not counting the read and write halves separately).
fn copy_bandwidth(src: &[u64], dst: &mut [u64], budget: Duration) -> f64 {
    let bytes_per_pass = (src.len() * 8) as u64;
    let rate = throughput(budget, || {
        dst.copy_from_slice(src);
        black_box(dst.as_ptr());
        bytes_per_pass
    });
    rate / GIB
}

/// Build a single random cycle over `nodes` slots using Sattolo's algorithm:
/// starting at 0 and repeatedly following `cycle[p]` visits every slot exactly
/// once before returning to 0.
fn sattolo_cycle(nodes: usize, seed: u64) -> Vec<u32> {
    assert!(nodes >= 2 && nodes <= u32::MAX as usize);
    let mut cycle: Vec<u32> = (0..nodes as u32).collect();
    let mut rng = SplitMix64::new(seed);
    for i in (1..nodes).rev() {
        let j = rng.below(i as u64) as usize; // strictly in [0, i)
        cycle.swap(i, j);
    }
    cycle
}

/// Chase the cycle; report nanoseconds per dependent load. `p` is kept across
/// batches so the walk keeps moving through the whole array instead of
/// re-treading a cache-resident prefix.
fn latency_ns(cycle: &[u32], budget: Duration) -> f64 {
    const BATCH: u64 = 8192;
    let mut p = 0usize;
    let steps_per_sec = throughput(budget, || {
        for _ in 0..BATCH {
            p = cycle[p] as usize;
        }
        black_box(p);
        BATCH
    });
    1e9 / steps_per_sec
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::DurationPreset;

    fn ctx() -> RunContext {
        RunContext {
            preset: DurationPreset::Short,
            seed: 5,
            target_dir: std::env::temp_dir(),
            threads: 1,
            total_ram: 8 * 1024 * 1024 * 1024,
            runs_override: None,
            abort: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[test]
    fn subtests_produce_positive_values() {
        let bench = MemoryBenchmark::new();
        // Keep it fast: tiny buffer, tiny budget.
        let words = 64 * 1024;
        let short = Duration::from_millis(15);
        let buf = filled_words(words, 1);
        assert!(read_bandwidth(&buf, short) > 0.0);
        let mut w = vec![0u64; words];
        assert!(write_bandwidth(&mut w, short) > 0.0);
        let src = filled_words(words, 2);
        let mut dst = vec![0u64; words];
        assert!(copy_bandwidth(&src, &mut dst, short) > 0.0);
        let cyc = sattolo_cycle(1 << 16, 3);
        assert!(latency_ns(&cyc, short) > 0.0);
        assert!(bench.run_subtest("bw_read", &ctx()).unwrap() > 0.0);
        assert!(bench.run_subtest("bw_read_mt", &ctx()).unwrap() > 0.0);
    }

    #[test]
    fn sattolo_visits_every_node_once() {
        let n = 4096;
        let cycle = sattolo_cycle(n, 99);
        let mut seen = vec![false; n];
        let mut p = 0usize;
        for _ in 0..n {
            assert!(!seen[p], "revisited {p} before completing the cycle");
            seen[p] = true;
            p = cycle[p] as usize;
        }
        assert_eq!(p, 0, "cycle did not return to start");
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn rejects_unknown_subtest() {
        assert!(MemoryBenchmark::new().run_subtest("nope", &ctx()).is_err());
    }
}
