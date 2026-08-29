//! GPU compute benchmark, via OpenCL loaded at runtime.
//!
//! Two figures: FP32 fused-multiply-add throughput (GFLOP/s) and VRAM read
//! bandwidth (GiB/s). The OpenCL ICD loader is `dlopen`ed — a machine with no
//! OpenCL, or no GPU device, simply has no `gpu` component (the run does not
//! fail). Like the network component, GPU is **measured and shown but kept out
//! of the overall grade**: it is optional hardware and folding it in would swamp
//! the "which machine is faster for my work" question.

mod cl;

use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::engine::{Benchmark, Direction, RunContext, SubtestSpec};
use crate::util::{GIB, SplitMix64};

/// Work-items launched by the compute kernel.
const COMPUTE_GLOBAL: usize = 1 << 20;
/// Work-items launched by the bandwidth kernel (grid-stride over the buffer).
const BW_GLOBAL: usize = 1 << 18;
/// FLOP per loop iteration in `fma32`: 4 `fma` on `float4` = 4 × 4 × 2.
const FLOP_PER_ITER: u64 = 32;

const KERNELS: &str = r#"
__kernel void fma32(__global float *out, const uint iters) {
    const uint gid = get_global_id(0);
    const float4 b = (float4)(0.99999994f);
    const float4 c = (float4)(1.00000006f);
    float4 x0 = (float4)((float)(gid & 255) * 0.01f + 1.0f);
    float4 x1 = x0 + 1.0f, x2 = x0 + 2.0f, x3 = x0 + 3.0f;
    for (uint i = 0; i < iters; i++) {
        x0 = fma(x0, b, c);
        x1 = fma(x1, b, c);
        x2 = fma(x2, b, c);
        x3 = fma(x3, b, c);
    }
    const float4 s = x0 + x1 + x2 + x3;
    out[gid] = s.x + s.y + s.z + s.w;
}

__kernel void bw_read(__global const float4 *in, __global float *out, const uint n4) {
    const uint gid = get_global_id(0);
    const uint stride = get_global_size(0);
    float4 acc = (float4)(0.0f);
    for (uint i = gid; i < n4; i += stride) acc += in[i];
    out[gid] = acc.x + acc.y + acc.z + acc.w;
}
"#;

/// What `loadbearer` found out about the GPU it will (or would) test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub opencl_version: String,
    pub driver: String,
    /// `CL_DEVICE_HOST_UNIFIED_MEMORY` — a good proxy for "shares system RAM".
    pub integrated: bool,
    pub vram_bytes: u64,
    pub max_alloc_bytes: u64,
    pub compute_units: u32,
    pub clock_mhz: u32,
}

impl GpuInfo {
    /// A one-line summary, e.g.
    /// `NVIDIA GeForce RTX 4070 · discrete · 12288 MiB · OpenCL 3.0`.
    pub fn summary(&self) -> String {
        let kind = if self.integrated {
            "integrated"
        } else {
            "discrete"
        };
        let vram = self.vram_bytes / (1024 * 1024);
        let ocl = self
            .opencl_version
            .strip_prefix("OpenCL ")
            .unwrap_or(&self.opencl_version);
        format!("{} · {kind} · {vram} MiB · OpenCL {ocl}", self.name)
    }
}

/// Set by `--no-gpu`: turns the GPU component off for the whole process,
/// including the [`probe`] that `loadbearer info` runs — so OpenCL is never
/// loaded at all.
static DISABLED: AtomicBool = AtomicBool::new(false);

/// Disable all GPU support for this process. Call once, before anything probes.
pub fn disable() {
    DISABLED.store(true, Ordering::Relaxed);
}

/// The GPU that would be tested, or `None` if `--no-gpu` was given, or OpenCL /
/// a GPU device is absent. Cheap after the first call.
pub fn probe() -> Option<&'static GpuInfo> {
    if DISABLED.load(Ordering::Relaxed) {
        return None;
    }
    static CACHE: OnceLock<Option<GpuInfo>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let cl = cl::Cl::load().ok()?;
            select(&cl).ok().map(|(_, info)| info)
        })
        .as_ref()
}

fn read_info(cl: &cl::Cl, d: cl::Device) -> Result<GpuInfo> {
    Ok(GpuInfo {
        name: cl.device_string(d, cl::CL_DEVICE_NAME)?,
        vendor: cl
            .device_string(d, cl::CL_DEVICE_VENDOR)
            .unwrap_or_default(),
        opencl_version: cl
            .device_string(d, cl::CL_DEVICE_VERSION)
            .unwrap_or_default(),
        driver: cl
            .device_string(d, cl::CL_DRIVER_VERSION)
            .unwrap_or_default(),
        integrated: cl
            .device_u32(d, cl::CL_DEVICE_HOST_UNIFIED_MEMORY)
            .unwrap_or(0)
            != 0,
        vram_bytes: cl.device_u64(d, cl::CL_DEVICE_GLOBAL_MEM_SIZE).unwrap_or(0),
        max_alloc_bytes: cl
            .device_u64(d, cl::CL_DEVICE_MAX_MEM_ALLOC_SIZE)
            .unwrap_or(0),
        compute_units: cl
            .device_u32(d, cl::CL_DEVICE_MAX_COMPUTE_UNITS)
            .unwrap_or(0),
        clock_mhz: cl
            .device_u32(d, cl::CL_DEVICE_MAX_CLOCK_FREQUENCY)
            .unwrap_or(0),
    })
}

/// Pick the strongest GPU: a discrete device beats an integrated one, then it's
/// compute-units × clock.
fn select(cl: &cl::Cl) -> Result<(cl::Device, GpuInfo)> {
    let mut best: Option<(cl::Device, GpuInfo, u64)> = None;
    for p in cl.platform_ids()? {
        for d in cl.device_ids(p, cl::CL_DEVICE_TYPE_GPU)? {
            let Ok(info) = read_info(cl, d) else { continue };
            let score = u64::from(info.compute_units.max(1))
                * u64::from(info.clock_mhz.max(1))
                * if info.integrated { 1 } else { 4 };
            if best.as_ref().is_none_or(|(_, _, s)| score > *s) {
                best = Some((d, info, score));
            }
        }
    }
    best.map(|(d, i, _)| (d, i))
        .ok_or_else(|| anyhow!("no OpenCL GPU device found"))
}

pub struct GpuBenchmark {
    notes: Mutex<Vec<String>>,
}

impl GpuBenchmark {
    pub fn new() -> Self {
        Self {
            notes: Mutex::new(Vec::new()),
        }
    }

    fn note(&self, msg: impl Into<String>) {
        let msg = msg.into();
        let mut g = self.notes.lock().unwrap();
        if !g.contains(&msg) {
            g.push(msg);
        }
    }
}

impl Benchmark for GpuBenchmark {
    fn id(&self) -> &'static str {
        "gpu"
    }

    fn label(&self) -> &'static str {
        "GPU"
    }

    fn subtests(&self) -> Vec<SubtestSpec> {
        use Direction::HigherIsBetter as Hi;
        vec![
            SubtestSpec {
                id: "compute_fp32",
                label: "FP32 compute (FMA)",
                unit: "GFLOP/s",
                direction: Hi,
            },
            SubtestSpec {
                id: "bandwidth",
                label: "VRAM read bandwidth",
                unit: "GiB/s",
                direction: Hi,
            },
        ]
    }

    fn run_subtest(&self, subtest_id: &str, ctx: &RunContext) -> Result<f64> {
        if DISABLED.load(Ordering::Relaxed) {
            bail!("GPU support is disabled (--no-gpu)");
        }
        let cl = cl::Cl::load()?;
        let (device, info) = select(&cl)?;
        self.note(format!("device: {}", info.summary()));

        let context = cl.context(device)?;
        let queue = context.queue()?;
        let program = context.program(KERNELS)?;
        let budget = ctx.time_budget();

        match subtest_id {
            "compute_fp32" => compute_fp32(&context, &queue, &program, budget, &ctx.abort),
            "bandwidth" => self.bandwidth(&context, &queue, &program, &info, ctx, budget),
            other => bail!("unknown gpu subtest: {other}"),
        }
    }

    fn notes(&self) -> Vec<String> {
        self.notes.lock().unwrap().clone()
    }
}

fn launch(q: &cl::Queue, k: &cl::Kernel, global: usize) -> Result<()> {
    q.run_1d(k, global, 0)?;
    q.finish()
}

/// Time a single kernel launch (one discarded warmup launch first).
fn time_launch(q: &cl::Queue, k: &cl::Kernel, global: usize) -> Result<f64> {
    launch(q, k, global)?;
    let t = Instant::now();
    launch(q, k, global)?;
    Ok(t.elapsed().as_secs_f64())
}

/// FP32 FMA throughput in GFLOP/s.
fn compute_fp32(
    ctx: &cl::Context,
    q: &cl::Queue,
    prog: &cl::Program,
    budget: Duration,
    abort: &AtomicBool,
) -> Result<f64> {
    let kernel = prog.kernel("fma32")?;
    let out = ctx.buffer(
        cl::CL_MEM_WRITE_ONLY | cl::CL_MEM_HOST_NO_ACCESS,
        COMPUTE_GLOBAL * 4,
    )?;
    kernel.set_mem(0, &out)?;

    // Calibrate `iters` so a launch runs ~8 ms — long enough to amortise launch
    // + sync overhead, short enough to sample the time budget finely.
    let mut iters: u32 = 128;
    for _ in 0..2 {
        kernel.set_u32(1, iters)?;
        let secs = time_launch(q, &kernel, COMPUTE_GLOBAL)?;
        iters = ((f64::from(iters) * 0.008 / secs.max(1e-6)).clamp(64.0, 4.0e6)) as u32;
    }
    kernel.set_u32(1, iters)?;
    for _ in 0..2 {
        launch(q, &kernel, COMPUTE_GLOBAL)?;
    }

    let flop_per_launch = FLOP_PER_ITER * u64::from(iters) * COMPUTE_GLOBAL as u64;
    let start = Instant::now();
    let mut launches: u64 = 0;
    while start.elapsed() < budget {
        launch(q, &kernel, COMPUTE_GLOBAL)?;
        launches += 1;
        if abort.load(Ordering::Relaxed) {
            break;
        }
    }
    let secs = start.elapsed().as_secs_f64();

    let mut sink = [0u8; 4];
    let _ = q.read(&out, &mut sink);
    std::hint::black_box(sink);

    ensure_positive(launches)?;
    Ok(flop_per_launch as f64 * launches as f64 / secs / 1e9)
}

impl GpuBenchmark {
    /// VRAM read bandwidth in GiB/s.
    fn bandwidth(
        &self,
        ctx: &cl::Context,
        q: &cl::Queue,
        prog: &cl::Program,
        info: &GpuInfo,
        rc: &RunContext,
        budget: Duration,
    ) -> Result<f64> {
        let target = (256.0 * 1024.0 * 1024.0 * rc.preset.workload_scale()) as usize;
        let mut bytes = target;
        let cap_vram = (info.vram_bytes / 4).max(16 * 1024 * 1024) as usize;
        let cap_alloc = if info.max_alloc_bytes > 0 {
            info.max_alloc_bytes as usize
        } else {
            usize::MAX
        };
        let cap = cap_vram.min(cap_alloc);
        if bytes > cap {
            bytes = cap;
            self.note(format!(
                "VRAM read working set capped at {} MiB (device limits)",
                bytes / (1024 * 1024)
            ));
        }
        bytes &= !15;
        bytes = bytes.max(1 << 20);
        let n4 = (bytes / 16) as u32;

        let host = filled(bytes, rc.seed ^ 0x6D2);
        let inb = ctx.buffer_copy(cl::CL_MEM_READ_ONLY, &host)?;
        let outb = ctx.buffer(
            cl::CL_MEM_WRITE_ONLY | cl::CL_MEM_HOST_NO_ACCESS,
            BW_GLOBAL * 4,
        )?;
        let kernel = prog.kernel("bw_read")?;
        kernel.set_mem(0, &inb)?;
        kernel.set_mem(1, &outb)?;
        kernel.set_u32(2, n4)?;

        for _ in 0..3 {
            launch(q, &kernel, BW_GLOBAL)?;
        }
        let start = Instant::now();
        let mut launches: u64 = 0;
        while start.elapsed() < budget {
            launch(q, &kernel, BW_GLOBAL)?;
            launches += 1;
            if rc.abort.load(Ordering::Relaxed) {
                break;
            }
        }
        let secs = start.elapsed().as_secs_f64();

        let mut sink = [0u8; 4];
        let _ = q.read(&outb, &mut sink);
        std::hint::black_box(sink);

        ensure_positive(launches)?;
        Ok(bytes as f64 * launches as f64 / secs / GIB)
    }
}

fn ensure_positive(launches: u64) -> Result<()> {
    if launches == 0 {
        bail!("GPU subtest completed no launches (aborted or budget too short)");
    }
    Ok(())
}

fn filled(bytes: usize, seed: u64) -> Vec<u8> {
    let mut v = vec![0u8; bytes];
    SplitMix64::new(seed).fill_bytes(&mut v);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_never_panics_and_is_stable() {
        let a = probe().map(|i| i.name.clone());
        let b = probe().map(|i| i.name.clone());
        assert_eq!(a, b);
    }

    #[test]
    fn gpu_subtest_errors_cleanly_when_no_gpu() {
        // On CI (no GPU / no ICD) this exercises the graceful-failure path; on a
        // machine with a GPU it actually runs a tiny measurement.
        let bench = GpuBenchmark::new();
        let ctx = RunContext {
            preset: crate::engine::DurationPreset::Short,
            seed: 1,
            target_dir: std::env::temp_dir(),
            threads: 1,
            total_ram: 8 << 30,
            runs_override: None,
            abort: std::sync::Arc::new(AtomicBool::new(false)),
        };
        match bench.run_subtest("compute_fp32", &ctx) {
            Ok(v) => assert!(v.is_finite() && v > 0.0, "got {v}"),
            Err(e) => {
                let s = e.to_string().to_lowercase();
                assert!(
                    s.contains("opencl") || s.contains("gpu") || s.contains("device"),
                    "unexpected error: {e}"
                );
            }
        }
    }

    #[test]
    fn summary_reads_well() {
        let info = GpuInfo {
            name: "Test GPU".into(),
            vendor: "TestCorp".into(),
            opencl_version: "OpenCL 3.0 CUDA".into(),
            driver: "1.2".into(),
            integrated: false,
            vram_bytes: 8 * 1024 * 1024 * 1024,
            max_alloc_bytes: 2 * 1024 * 1024 * 1024,
            compute_units: 40,
            clock_mhz: 2400,
        };
        assert_eq!(
            info.summary(),
            "Test GPU · discrete · 8192 MiB · OpenCL 3.0 CUDA"
        );
    }
}
