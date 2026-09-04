//! CPU benchmark: integer and floating-point throughput (single- and all-core),
//! plus two representative real-world kernels — BLAKE3 hashing and DEFLATE
//! compression — and an informational integer thread-scaling curve
//! (`int_scale_2` … up to just under the logical CPU count).
//!
//! ## Methodology
//!
//! Each subtest runs for a fixed wall-clock budget and counts the work it
//! completed, yielding a rate. The integer and float kernels use eight
//! independent accumulator lanes so the measurement reflects pipeline
//! *throughput* rather than the latency of a single dependency chain.
//!
//! Operation counts (used only to label the rate) assume a portable SSE2
//! baseline. Building with `RUSTFLAGS="-C target-cpu=native"` lets the kernels
//! use wider vectors and will raise the absolute numbers; the reference baseline
//! is captured the same way, so relative grades stay meaningful either way.

use std::hint::black_box;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::engine::{Benchmark, Direction, RunContext, SubtestSpec, parallel_sum, throughput};
use crate::util::{MIB, SplitMix64};

pub struct CpuBenchmark {
    /// Logical CPU count, used to decide which thread-scaling points to emit.
    logical: usize,
}

/// Thread counts for the informational integer-scaling curve. A point is
/// emitted only when it sits strictly between the `int_single` (1) and
/// `int_multi` (all) endpoints.
const SCALE_POINTS: &[(usize, &str, &str)] = &[
    (2, "int_scale_2", "Integer, 2 threads"),
    (4, "int_scale_4", "Integer, 4 threads"),
    (8, "int_scale_8", "Integer, 8 threads"),
    (16, "int_scale_16", "Integer, 16 threads"),
    (32, "int_scale_32", "Integer, 32 threads"),
];

impl CpuBenchmark {
    pub fn new() -> Self {
        Self {
            logical: std::thread::available_parallelism().map_or(1, |n| n.get()),
        }
    }
}

impl Benchmark for CpuBenchmark {
    fn id(&self) -> &'static str {
        "cpu"
    }

    fn label(&self) -> &'static str {
        "CPU"
    }

    fn subtests(&self) -> Vec<SubtestSpec> {
        const HI: Direction = Direction::HigherIsBetter;
        let mut v = vec![
            SubtestSpec::scored("int_single", "Integer, single-core", "Mops/s", HI),
            SubtestSpec::scored("int_multi", "Integer, all cores", "Mops/s", HI),
            SubtestSpec::scored("float_single", "Float, single-core", "MFLOP/s", HI),
            SubtestSpec::scored("float_multi", "Float, all cores", "MFLOP/s", HI),
            SubtestSpec::scored("hash", "BLAKE3 hash", "MiB/s", HI),
            SubtestSpec::scored("compress", "DEFLATE compress", "MiB/s", HI),
            SubtestSpec::scored("aes_gcm", "AES-256-GCM encrypt", "MiB/s", HI),
            SubtestSpec::scored("sha256", "SHA-256 hash", "MiB/s", HI),
        ];
        for &(n, id, label) in SCALE_POINTS {
            if n < self.logical {
                v.push(SubtestSpec::info(id, label, "Mops/s", HI));
            }
        }
        v
    }

    fn run_subtest(&self, subtest_id: &str, ctx: &RunContext) -> Result<f64> {
        let budget = ctx.time_budget();
        Ok(match subtest_id {
            "int_single" => integer_rate(budget),
            "int_multi" => parallel_sum(ctx.threads, || integer_rate(budget)),
            "float_single" => float_rate(budget),
            "float_multi" => parallel_sum(ctx.threads, || float_rate(budget)),
            "hash" => hash_rate(budget, ctx.seed),
            "compress" => compress_rate(budget, ctx.seed),
            "aes_gcm" => aes_gcm_rate(budget, ctx.seed),
            "sha256" => sha256_rate(budget, ctx.seed),
            other => match other
                .strip_prefix("int_scale_")
                .and_then(|s| s.parse::<usize>().ok())
            {
                Some(n) => parallel_sum(n, || integer_rate(budget)),
                None => bail!("unknown cpu subtest: {other}"),
            },
        })
    }

    fn single_threaded(&self, subtest_id: &str) -> bool {
        matches!(
            subtest_id,
            "int_single" | "float_single" | "hash" | "compress" | "aes_gcm" | "sha256"
        )
    }
}

const LANES: usize = 8;
const INNER: u64 = 4096;

/// Integer throughput in millions of operations per second.
///
/// Per lane per inner iteration: multiply, add, shift-xor, multiply, rotate,
/// xor-accumulate ≈ 7 integer operations.
fn integer_rate(budget: Duration) -> f64 {
    const OPS_PER_INNER: u64 = 7;
    let rate = throughput(budget, || {
        let mut state = seed_lanes();
        let mut acc = black_box(1u64);
        for _ in 0..INNER {
            for lane in state.iter_mut() {
                let mut x = *lane;
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                x ^= x >> 27;
                x = x.wrapping_mul(0x2545F4914F6CDD1D);
                *lane = x;
                acc ^= x.rotate_left(13);
            }
        }
        black_box(acc);
        INNER * LANES as u64 * OPS_PER_INNER
    });
    rate / 1e6
}

/// Floating-point throughput in millions of FLOP per second.
///
/// Per lane per inner iteration: one multiply and one add = 2 FLOP. Constants
/// are chosen so the lane values drift only slightly over a batch (no overflow,
/// no denormals).
fn float_rate(budget: Duration) -> f64 {
    const FLOPS_PER_INNER: u64 = 2;
    let rate = throughput(budget, || {
        let mut lane = [0.0f64; LANES];
        for (i, v) in lane.iter_mut().enumerate() {
            *v = 1.0 + i as f64 * 1e-3;
        }
        let c1 = black_box(1.000_000_191_8f64);
        let c2 = black_box(0.999_999_937_3f64);
        for _ in 0..INNER {
            for v in lane.iter_mut() {
                *v = *v * c1 + c2;
            }
        }
        let mut acc = 0.0f64;
        for v in &lane {
            acc += *v;
        }
        black_box(acc);
        INNER * LANES as u64 * FLOPS_PER_INNER
    });
    rate / 1e6
}

/// BLAKE3 hashing throughput in MiB/s over a 1 MiB buffer.
fn hash_rate(budget: Duration, seed: u64) -> f64 {
    const BUF: usize = 1 << 20;
    let mut buf = vec![0u8; BUF];
    SplitMix64::new(seed ^ 0xD1B5_4A32_D192_ED03).fill_bytes(&mut buf);
    let bytes = throughput(budget, || {
        let digest = blake3::hash(&buf);
        black_box(digest.as_bytes()[0]);
        BUF as u64
    });
    bytes / MIB
}

/// DEFLATE (level 6) compression throughput in MiB/s of input consumed, over a
/// 256 KiB buffer that is roughly half structured, half random.
fn compress_rate(budget: Duration, seed: u64) -> f64 {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::DeflateEncoder;

    const BUF: usize = 256 * 1024;
    let buf = mixed_compressibility_bytes(BUF, seed ^ 0x2545_F491_4F6C_DD1D);
    let bytes = throughput(budget, || {
        let mut encoder = DeflateEncoder::new(Vec::with_capacity(BUF / 2), Compression::new(6));
        encoder
            .write_all(&buf)
            .expect("in-memory write cannot fail");
        let compressed = encoder.finish().expect("in-memory flush cannot fail");
        black_box(compressed.len());
        BUF as u64
    });
    bytes / MIB
}

/// AES-256-GCM authenticated-encryption throughput in MiB/s of plaintext, over a
/// 256 KiB buffer. RustCrypto uses AES-NI + CLMUL at runtime where the CPU has
/// them, so this reflects the crypto-instruction generation, not just the clock.
fn aes_gcm_rate(budget: Duration, seed: u64) -> f64 {
    use aes_gcm::aead::AeadInPlace;
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

    const BUF: usize = 256 * 1024;
    let mut key = [0u8; 32];
    SplitMix64::new(seed ^ 0xA35F_1C2D_9B4E_7061).fill_bytes(&mut key);
    let cipher = Aes256Gcm::new_from_slice(&key).expect("32-byte key");
    let nonce = Nonce::from_slice(&[0u8; 12]);
    let mut buf = vec![0u8; BUF];
    SplitMix64::new(seed ^ 0x51E5_A2C3_44D9_0177).fill_bytes(&mut buf);

    let bytes = throughput(budget, || {
        // Encrypt in place, discard the tag. Re-encrypting the previous
        // ciphertext costs the same AES + GHASH work.
        let tag = cipher
            .encrypt_in_place_detached(nonce, b"", &mut buf)
            .expect("in-memory AES-GCM cannot fail");
        black_box(tag[0]);
        BUF as u64
    });
    bytes / MIB
}

/// SHA-256 throughput in MiB/s over a 256 KiB buffer. Uses the SHA extensions
/// where the CPU has them.
fn sha256_rate(budget: Duration, seed: u64) -> f64 {
    use sha2::{Digest, Sha256};

    const BUF: usize = 256 * 1024;
    let mut buf = vec![0u8; BUF];
    SplitMix64::new(seed ^ 0x9D2C_5680_C167_88F3).fill_bytes(&mut buf);

    let bytes = throughput(budget, || {
        let mut hasher = Sha256::new();
        hasher.update(&buf);
        black_box(hasher.finalize()[0]);
        BUF as u64
    });
    bytes / MIB
}

fn seed_lanes() -> [u64; LANES] {
    let mut lanes = [0u64; LANES];
    for (i, lane) in lanes.iter_mut().enumerate() {
        *lane = 0x9E37_79B9_7F4A_7C15u64
            .wrapping_add(i as u64)
            .wrapping_mul(0xBF58_476D_1CE4_E5B9);
    }
    lanes
}

/// A payload that is roughly half repeated words (compressible) and half random
/// (incompressible), so DEFLATE has real work to do.
fn mixed_compressibility_bytes(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = SplitMix64::new(seed);
    let mut out = Vec::with_capacity(len + 32);
    while out.len() < len {
        let word = rng.next_u64().to_le_bytes();
        if rng.next_u64() & 1 == 0 {
            for _ in 0..4 {
                out.extend_from_slice(&word);
            }
        } else {
            out.extend_from_slice(&word);
        }
    }
    out.truncate(len);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> RunContext {
        RunContext {
            preset: crate::engine::DurationPreset::Short,
            seed: 1,
            target_dir: std::env::temp_dir(),
            threads: 2,
            total_ram: 8 << 30,
            runs_override: None,
            abort: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    #[test]
    fn every_subtest_produces_a_positive_rate() {
        let bench = CpuBenchmark::new();
        for spec in bench.subtests() {
            let value = bench.run_subtest(spec.id, &ctx()).unwrap();
            assert!(value.is_finite() && value > 0.0, "{} -> {value}", spec.id);
        }
    }

    #[test]
    fn scaling_points_sit_between_the_endpoints() {
        let ids = |logical: usize| {
            CpuBenchmark { logical }
                .subtests()
                .into_iter()
                .filter_map(|s| s.id.strip_prefix("int_scale_").map(str::to_string))
                .collect::<Vec<_>>()
        };
        assert!(ids(1).is_empty());
        assert!(ids(2).is_empty());
        assert_eq!(ids(4), ["2"]);
        assert_eq!(ids(8), ["2", "4"]);
        assert_eq!(ids(12), ["2", "4", "8"]);
        assert_eq!(ids(64), ["2", "4", "8", "16", "32"]);
    }

    #[test]
    fn dispatch_rejects_unknown_subtest() {
        assert!(CpuBenchmark::new().run_subtest("nope", &ctx()).is_err());
        assert!(
            CpuBenchmark::new()
                .run_subtest("int_scale_x", &ctx())
                .is_err()
        );
    }
}
