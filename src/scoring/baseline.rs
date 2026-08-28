//! The reference baseline: raw measurements a reference machine is expected to
//! produce, one per subtest. A run's raw value divided by the matching baseline
//! value gives the ratio the scoring curve is applied to.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const REFERENCE_V1: &str = include_str!("../../baseline/reference-v1.toml");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    pub name: String,
    pub description: String,
    /// component id -> (subtest id -> reference raw value). `BTreeMap` so the
    /// serialised form is deterministically ordered.
    pub components: BTreeMap<String, BTreeMap<String, f64>>,
}

impl Baseline {
    /// The built-in `reference-v1` baseline, embedded at compile time.
    pub fn reference_v1() -> Self {
        toml::from_str(REFERENCE_V1).expect("embedded reference-v1.toml is valid")
    }

    /// The raw TOML source of the built-in baseline, comments and all.
    pub fn embedded_toml() -> &'static str {
        REFERENCE_V1
    }

    /// Reference raw value for `component/subtest`, or an error naming the gap.
    pub fn lookup(&self, component: &str, subtest: &str) -> Result<f64> {
        let value = self
            .components
            .get(component)
            .and_then(|m| m.get(subtest))
            .copied()
            .with_context(|| {
                format!(
                    "baseline {:?} has no reference value for {component}/{subtest}",
                    self.name
                )
            })?;
        anyhow::ensure!(
            value > 0.0,
            "baseline {:?} value for {component}/{subtest} must be positive, got {value}",
            self.name
        );
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_baseline_parses_and_covers_the_known_subtests() {
        let b = Baseline::reference_v1();
        assert_eq!(b.name, "reference-v1");
        for (component, subtests) in [
            (
                "cpu",
                [
                    "int_single",
                    "int_multi",
                    "float_single",
                    "float_multi",
                    "hash",
                    "compress",
                    "aes_gcm",
                    "sha256",
                ]
                .as_slice(),
            ),
            (
                "memory",
                ["bw_read", "bw_write", "bw_copy", "bw_read_mt", "latency"].as_slice(),
            ),
            (
                "disk",
                ["seq_write", "seq_read", "rand_read", "rand_write"].as_slice(),
            ),
            (
                "network",
                ["tcp_stream", "tcp_parallel", "tcp_rtt", "udp_pps"].as_slice(),
            ),
        ] {
            for st in subtests {
                assert!(b.lookup(component, st).unwrap() > 0.0, "{component}/{st}");
            }
        }
    }

    #[test]
    fn missing_entry_is_an_error() {
        assert!(Baseline::reference_v1().lookup("cpu", "nonesuch").is_err());
    }
}
