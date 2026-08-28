//! Disk benchmark: sequential write/read throughput and random 4 KiB IOPS, at
//! queue depth 1.
//!
//! ## Methodology
//!
//! A scratch file (`.loadbearer-scratch.<pid>` in `--target-dir`, 1 GiB at the
//! `normal` preset) is created once, filled with pseudo-random bytes to defeat
//! filesystem-level compression, and reused by every subtest. It is deleted when
//! the run ends; a killed run may leave it behind.
//!
//! Reads and random I/O go through unbuffered I/O — `O_DIRECT` on Linux,
//! `FILE_FLAG_NO_BUFFERING` on Windows — so the figures reflect the device, not
//! the page cache. When the platform or filesystem refuses unbuffered access the
//! benchmark falls back to buffered I/O and records a note; read numbers on such
//! a machine may be cache-influenced. Sequential writes always end with an
//! `fsync`, so `Sequential write` is durable-write throughput. All subtests are
//! single-threaded / QD1.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::engine::{Benchmark, Direction, RunContext, SubtestSpec};
use crate::util::{MIB, SplitMix64};

mod aligned;
mod platform;

use aligned::AlignedBuf;

/// Sequential transfer size.
const CHUNK: usize = 1024 * 1024;
/// Random-I/O block size.
const BLOCK: usize = 4096;
/// fsync cadence for the random-write subtest.
const WRITE_SYNC_EVERY: u64 = 64;

fn scratch_size(ctx: &RunContext) -> u64 {
    // 512 MiB / 1 GiB / 2 GiB across the presets.
    (1024.0 * MIB * ctx.preset.workload_scale()) as u64 & !(CHUNK as u64 - 1)
}

struct Scratch {
    path: PathBuf,
    size: u64,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct DiskBenchmark {
    scratch: Mutex<Option<Scratch>>,
    /// `Some(true)` unbuffered, `Some(false)` buffered fallback, `None` untested.
    unbuffered: Mutex<Option<bool>>,
    notes: Mutex<Vec<String>>,
}

impl DiskBenchmark {
    pub fn new() -> Self {
        Self {
            scratch: Mutex::new(None),
            unbuffered: Mutex::new(None),
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

    /// Create and fill the scratch file if it is not already present at the
    /// required size. Not measured.
    fn ensure_scratch(&self, ctx: &RunContext) -> Result<(PathBuf, u64)> {
        let mut guard = self.scratch.lock().unwrap();
        let want = scratch_size(ctx);
        if let Some(s) = guard.as_ref()
            && s.size >= want
        {
            return Ok((s.path.clone(), s.size));
        }

        if platform::is_memory_backed(&ctx.target_dir) {
            self.note(format!(
                "{} is a RAM-backed filesystem (tmpfs/ramfs); disk figures reflect memory, \
                 not storage — pass --target-dir on a real disk",
                ctx.target_dir.display()
            ));
        }

        let path = ctx
            .target_dir
            .join(format!(".loadbearer-scratch.{}", std::process::id()));
        fill_scratch(&path, want, ctx.seed)
            .with_context(|| format!("preparing scratch file at {}", path.display()))?;
        *guard = Some(Scratch {
            path: path.clone(),
            size: want,
        });
        Ok((path, want))
    }

    /// Open the scratch file for reading, unbuffered if the platform allows it.
    /// The buffered-vs-unbuffered decision is made once and cached.
    fn open_read(&self, path: &Path, len: u64) -> Result<(File, bool)> {
        let mut decided = self.unbuffered.lock().unwrap();
        if let Some(false) = *decided {
            return Ok((OpenOptions::new().read(true).open(path)?, false));
        }
        match platform::open_unbuffered_read(path) {
            Ok(file) if probe_unbuffered_read(&file).is_ok() => {
                if decided.is_none() {
                    *decided = Some(true);
                }
                Ok((file, true))
            }
            _ => {
                *decided = Some(false);
                self.note(
                    "unbuffered reads unavailable on this platform/filesystem; \
                     read figures may be cache-influenced",
                );
                let file = OpenOptions::new().read(true).open(path)?;
                platform::drop_from_cache(&file, len);
                Ok((file, false))
            }
        }
    }

    fn open_write(&self, path: &Path) -> Result<(File, bool)> {
        let unbuffered = matches!(*self.unbuffered.lock().unwrap(), Some(true) | None);
        if unbuffered && let Ok(file) = platform::open_unbuffered_write(path) {
            return Ok((file, true));
        }
        Ok((OpenOptions::new().write(true).open(path)?, false))
    }
}

impl Benchmark for DiskBenchmark {
    fn id(&self) -> &'static str {
        "disk"
    }

    fn label(&self) -> &'static str {
        "Disk"
    }

    fn subtests(&self) -> Vec<SubtestSpec> {
        use Direction::HigherIsBetter as Hi;
        vec![
            SubtestSpec {
                id: "seq_write",
                label: "Sequential write",
                unit: "MiB/s",
                direction: Hi,
            },
            SubtestSpec {
                id: "seq_read",
                label: "Sequential read",
                unit: "MiB/s",
                direction: Hi,
            },
            SubtestSpec {
                id: "rand_read",
                label: "Random 4K read",
                unit: "IOPS",
                direction: Hi,
            },
            SubtestSpec {
                id: "rand_write",
                label: "Random 4K write",
                unit: "IOPS",
                direction: Hi,
            },
        ]
    }

    fn run_subtest(&self, subtest_id: &str, ctx: &RunContext) -> Result<f64> {
        let (path, size) = self.ensure_scratch(ctx)?;
        let budget = ctx.time_budget();

        Ok(match subtest_id {
            "seq_write" => {
                let (file, _direct) = self.open_write(&path)?;
                seq_write(&file, size, budget)?
            }
            "seq_read" => {
                let (file, direct) = self.open_read(&path, size)?;
                if !direct {
                    platform::drop_from_cache(&file, size);
                }
                seq_read(&file, size, budget)?
            }
            "rand_read" => {
                let (file, _direct) = self.open_read(&path, size)?;
                rand_read(&file, size, budget, ctx.seed ^ 0x11)?
            }
            "rand_write" => {
                let (file, _direct) = self.open_write(&path)?;
                rand_write(&file, size, budget, ctx.seed ^ 0x22)?
            }
            other => bail!("unknown disk subtest: {other}"),
        })
    }

    fn notes(&self) -> Vec<String> {
        self.notes.lock().unwrap().clone()
    }
}

/// Distinct sub-seeds so the scratch contents and the write payloads differ.
const SEED_FILL: u64 = 0x5CA7_F111_0000_0001;
const SEED_WRITE_PAYLOAD: u64 = 0x5CA7_9271_0000_0002;

fn fill_scratch(path: &Path, size: u64, seed: u64) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    let mut rng = SplitMix64::new(seed ^ SEED_FILL);
    let mut buf = vec![0u8; CHUNK];
    let mut written = 0u64;
    while written < size {
        rng.fill_bytes(&mut buf);
        let n = CHUNK.min((size - written) as usize);
        file.write_all(&buf[..n])?;
        written += n as u64;
    }
    file.sync_all()?;
    Ok(())
}

fn seq_write(file: &File, size: u64, budget: Duration) -> io::Result<f64> {
    let mut buf = AlignedBuf::new(CHUNK);
    SplitMix64::new(SEED_WRITE_PAYLOAD).fill_bytes(buf.as_mut_slice());
    timed_bytes(budget, || {
        let mut off = 0u64;
        while off < size {
            let n = (CHUNK as u64).min(size - off) as usize;
            platform::pwrite_all(file, &buf.as_slice()[..n], off)?;
            off += n as u64;
        }
        file.sync_all()?;
        Ok(size)
    })
}

fn seq_read(file: &File, size: u64, budget: Duration) -> io::Result<f64> {
    let mut buf = AlignedBuf::new(CHUNK);
    timed_bytes(budget, || {
        let mut off = 0u64;
        while off < size {
            let n = (CHUNK as u64).min(size - off) as usize;
            platform::pread_exact(file, &mut buf.as_mut_slice()[..n], off)?;
            off += n as u64;
        }
        Ok(size)
    })
}

fn rand_read(file: &File, size: u64, budget: Duration, seed: u64) -> io::Result<f64> {
    let blocks = size / BLOCK as u64;
    let mut buf = AlignedBuf::new(BLOCK);
    let mut rng = SplitMix64::new(seed);
    timed_ops(budget, 64, || {
        let off = rng.below(blocks) * BLOCK as u64;
        platform::pread_exact(file, buf.as_mut_slice(), off)?;
        Ok(())
    })
}

fn rand_write(file: &File, size: u64, budget: Duration, seed: u64) -> io::Result<f64> {
    let blocks = size / BLOCK as u64;
    let mut buf = AlignedBuf::new(BLOCK);
    SplitMix64::new(seed ^ 0xDEAD).fill_bytes(buf.as_mut_slice());
    let mut rng = SplitMix64::new(seed);
    let mut since_sync = 0u64;
    let rate = timed_ops(budget, 64, || {
        let off = rng.below(blocks) * BLOCK as u64;
        platform::pwrite_all(file, buf.as_slice(), off)?;
        since_sync += 1;
        if since_sync >= WRITE_SYNC_EVERY {
            file.sync_all()?;
            since_sync = 0;
        }
        Ok(())
    })?;
    file.sync_all()?;
    Ok(rate)
}

/// Run `pass` repeatedly until `budget` elapses; return bytes/sec as MiB/s.
fn timed_bytes<F>(budget: Duration, mut pass: F) -> io::Result<f64>
where
    F: FnMut() -> io::Result<u64>,
{
    let start = Instant::now();
    let mut total = 0u64;
    loop {
        total += pass()?;
        if start.elapsed() >= budget {
            break;
        }
    }
    Ok(total as f64 / start.elapsed().as_secs_f64() / MIB)
}

/// Run `op` in batches of `batch` until `budget` elapses; return ops/sec.
fn timed_ops<F>(budget: Duration, batch: u64, mut op: F) -> io::Result<f64>
where
    F: FnMut() -> io::Result<()>,
{
    let start = Instant::now();
    let mut ops = 0u64;
    loop {
        for _ in 0..batch {
            op()?;
            ops += 1;
        }
        if start.elapsed() >= budget {
            break;
        }
    }
    Ok(ops as f64 / start.elapsed().as_secs_f64())
}

/// One aligned probe read to confirm the FS actually honours unbuffered I/O.
fn probe_unbuffered_read(file: &File) -> io::Result<()> {
    let mut buf = AlignedBuf::new(BLOCK);
    platform::pread_exact(file, buf.as_mut_slice(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::DurationPreset;

    fn ctx(dir: &Path) -> RunContext {
        RunContext {
            preset: DurationPreset::Short,
            seed: 1,
            target_dir: dir.to_path_buf(),
            threads: 1,
            total_ram: 8 << 30,
            runs_override: None,
            abort: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[test]
    fn all_subtests_run_and_scratch_is_cleaned_up() {
        let dir = std::env::temp_dir().join(format!("lb-disk-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bench = DiskBenchmark::new();
        // Shrink the scratch file for the test.
        {
            let mut g = bench.scratch.lock().unwrap();
            let path = dir.join("mini");
            fill_scratch(&path, 8 * 1024 * 1024, 1).unwrap();
            *g = Some(Scratch {
                path,
                size: 8 * 1024 * 1024,
            });
        }
        for id in ["seq_write", "seq_read", "rand_read", "rand_write"] {
            let v = bench.run_subtest(id, &ctx(&dir)).unwrap();
            assert!(v.is_finite() && v > 0.0, "{id} -> {v}");
        }
        let scratch_path = bench.scratch.lock().unwrap().as_ref().unwrap().path.clone();
        drop(bench);
        assert!(!scratch_path.exists(), "scratch file was not removed");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_unknown_subtest() {
        let dir = std::env::temp_dir();
        assert!(
            DiskBenchmark::new()
                .run_subtest("nope", &ctx(&dir))
                .is_err()
        );
    }
}
