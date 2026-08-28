//! The benchmark registry. Each subsystem is one [`Benchmark`] implementation.

mod cpu;
mod disk;
mod memory;
mod network;

use crate::engine::Benchmark;

pub use network::{link_probe, serve as net_serve};

/// Every benchmark, in display order.
pub fn all() -> Vec<Box<dyn Benchmark>> {
    vec![
        Box::new(cpu::CpuBenchmark),
        Box::new(memory::MemoryBenchmark::new()),
        Box::new(disk::DiskBenchmark::new()),
        Box::new(network::NetworkBenchmark),
    ]
}

/// The set of known benchmark ids, for validating `--only`.
pub fn known_ids() -> Vec<&'static str> {
    all().iter().map(|b| b.id()).collect()
}
