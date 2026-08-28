//! Summary statistics over the repeated timed runs of a single subtest.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    /// The individual timed-run values, in run order.
    pub runs: Vec<f64>,
    pub median: f64,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    /// Sample standard deviation (Bessel-corrected); 0.0 for a single run.
    pub stddev: f64,
    /// Coefficient of variation, `stddev / mean`; 0.0 when the mean is ~0.
    pub cv: f64,
}

impl Stats {
    /// Summarise a non-empty set of timed-run values.
    pub fn from_runs(runs: Vec<f64>) -> Self {
        assert!(
            !runs.is_empty(),
            "Stats::from_runs requires at least one run"
        );
        let n = runs.len() as f64;

        let mut sorted = runs.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let mid = sorted.len() / 2;
        let median = if sorted.len() % 2 == 1 {
            sorted[mid]
        } else {
            (sorted[mid - 1] + sorted[mid]) / 2.0
        };

        let mean = runs.iter().sum::<f64>() / n;
        let variance = if runs.len() > 1 {
            runs.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)
        } else {
            0.0
        };
        let stddev = variance.sqrt();
        let cv = if mean.abs() > f64::EPSILON {
            stddev / mean
        } else {
            0.0
        };

        Self {
            runs,
            median,
            mean,
            min: sorted[0],
            max: sorted[sorted.len() - 1],
            stddev,
            cv,
        }
    }
}

/// How much to trust a subtest's representative value, derived from run spread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

impl Confidence {
    /// Map a coefficient of variation to a confidence band.
    pub fn from_cv(cv: f64) -> Self {
        if cv < 0.03 {
            Confidence::High
        } else if cv < 0.08 {
            Confidence::Medium
        } else {
            Confidence::Low
        }
    }

    /// Higher rank = more trustworthy. Use with `min_by_key` to find the weakest.
    pub fn rank(self) -> u8 {
        match self {
            Confidence::High => 2,
            Confidence::Medium => 1,
            Confidence::Low => 0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_run_has_zero_spread() {
        let s = Stats::from_runs(vec![100.0]);
        assert_eq!(s.median, 100.0);
        assert_eq!(s.stddev, 0.0);
        assert_eq!(s.cv, 0.0);
    }

    #[test]
    fn median_of_even_count_averages_middle_pair() {
        let s = Stats::from_runs(vec![10.0, 40.0, 20.0, 30.0]);
        assert_eq!(s.median, 25.0);
        assert_eq!(s.min, 10.0);
        assert_eq!(s.max, 40.0);
    }

    #[test]
    fn confidence_bands() {
        assert_eq!(Confidence::from_cv(0.01), Confidence::High);
        assert_eq!(Confidence::from_cv(0.05), Confidence::Medium);
        assert_eq!(Confidence::from_cv(0.20), Confidence::Low);
    }
}
