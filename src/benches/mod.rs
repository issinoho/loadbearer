//! The benchmark registry. Each subsystem is one [`Benchmark`] implementation.

mod cpu;
mod disk;
mod memory;

use crate::engine::Benchmark;

/// Every benchmark, in display order.
pub fn all() -> Vec<Box<dyn Benchmark>> {
    vec![
        Box::new(cpu::CpuBenchmark),
        Box::new(memory::MemoryBenchmark::new()),
        Box::new(disk::DiskBenchmark::new()),
    ]
}

/// The set of known benchmark ids, for validating `--only`.
pub fn known_ids() -> Vec<&'static str> {
    all().iter().map(|b| b.id()).collect()
}
