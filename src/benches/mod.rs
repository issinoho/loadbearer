//! The benchmark registry. Each subsystem is one [`Benchmark`] implementation.

mod cpu;
mod disk;
mod gpu;
mod memory;
mod network;

use crate::engine::Benchmark;

pub use gpu::{GpuInfo, disable as gpu_disable, probe as gpu_probe};
pub use network::{link_probe, serve as net_serve};

/// Every benchmark, in display order. `gpu` is included whether or not a GPU is
/// present; callers that don't want a hard failure on a GPU-less box filter it
/// with [`gpu_probe`] (see `run::select_benchmarks`).
pub fn all() -> Vec<Box<dyn Benchmark>> {
    vec![
        Box::new(cpu::CpuBenchmark),
        Box::new(memory::MemoryBenchmark::new()),
        Box::new(disk::DiskBenchmark::new()),
        Box::new(network::NetworkBenchmark),
        Box::new(gpu::GpuBenchmark::new()),
    ]
}

/// The set of known benchmark ids, for validating `--only`.
pub fn known_ids() -> Vec<&'static str> {
    all().iter().map(|b| b.id()).collect()
}
