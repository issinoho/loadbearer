//! Turning raw benchmark metrics into scores and grades.
//!
//! Per subtest: `ratio` = raw / baseline (inverted for lower-is-better metrics),
//! then `score` = 1000 · ratio^k, where `k` is the display curve — lower `k`
//! pulls the extremes back toward 1000.
//!
//! Per component: the geometric mean of its subtest scores. Overall: the
//! profile-weighted geometric mean of the component scores.
//!
//! A machine that matches the baseline everywhere scores 1000, which sits in the
//! middle of grade B.

mod baseline;
mod profiles;

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub use baseline::Baseline;
pub use profiles::Profile;
pub use profiles::{ALL as PROFILES, by_name as profile_by_name, names as profile_names};

use crate::engine::stats::Confidence;
use crate::engine::{BenchmarkOutcome, Direction};
use crate::inventory::Inventory;

pub const SCHEMA: &str = "loadbearer.result/1";

/// Letter grade derived from a score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grade {
    S,
    A,
    B,
    C,
    D,
    F,
}

impl Grade {
    /// Grade bands, centred so that the baseline score of 1000 lands mid-B.
    pub fn from_score(score: f64) -> Grade {
        match score {
            s if s >= 1400.0 => Grade::S,
            s if s >= 1150.0 => Grade::A,
            s if s >= 850.0 => Grade::B,
            s if s >= 600.0 => Grade::C,
            s if s >= 400.0 => Grade::D,
            _ => Grade::F,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Grade::S => "S",
            Grade::A => "A",
            Grade::B => "B",
            Grade::C => "C",
            Grade::D => "D",
            Grade::F => "F",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredSubtest {
    pub id: String,
    pub label: String,
    pub unit: String,
    /// Raw representative value (the run median).
    pub value: f64,
    /// Baseline reference value for this subtest.
    pub baseline: f64,
    /// Direction-adjusted ratio to baseline (>1 means better than baseline).
    pub ratio: f64,
    pub score: f64,
    pub confidence: Confidence,
}

fn yes() -> bool {
    true
}

/// Component ids that are scored and shown but kept out of the overall grade —
/// their result is too dependent on the host OS (and any security tooling) to
/// fold into a hardware grade. `compare` still uses their raw metrics.
pub const UNGRADED_COMPONENTS: &[&str] = &["network", "gpu"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredComponent {
    pub id: String,
    pub label: String,
    pub score: f64,
    pub grade: Grade,
    /// Whether this component counts toward the overall grade.
    #[serde(default = "yes")]
    pub graded: bool,
    /// Weakest confidence among the component's subtests.
    pub confidence: Confidence,
    pub subtests: Vec<ScoredSubtest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Overall {
    pub score: f64,
    pub grade: Grade,
    pub profile: String,
    /// Plain-language explanation of what moved the score.
    pub why: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    pub profile: String,
    pub duration_preset: String,
    pub curve_k: f64,
    pub seed: u64,
    pub threads: usize,
    pub baseline: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
}

/// Result of an optional `--net-target` link probe. Not scored — it measures
/// the path between two machines, not either host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkResult {
    pub target: String,
    pub tcp_upload_gibps: f64,
    pub tcp_rtt_us: f64,
    pub udp_send_kpps: f64,
}

/// The full, versioned result artifact written by `loadbearer run`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultFile {
    pub schema: String,
    pub tool_version: String,
    pub timestamp: String,
    pub machine: Inventory,
    pub config: RunConfig,
    /// Unscored metrics, kept for full fidelity and re-scoring.
    pub raw: Vec<BenchmarkOutcome>,
    pub components: Vec<ScoredComponent>,
    pub overall: Overall,
    /// Present only when `--net-target` was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<LinkResult>,
    /// Present only when `--soak` was given. Sustained-load throughput
    /// retention; measured and shown but not folded into any grade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub soak: Option<crate::soak::SoakResult>,
}

impl ResultFile {
    pub fn assemble(
        machine: Inventory,
        config: RunConfig,
        raw: Vec<BenchmarkOutcome>,
        scored: ScoredRun,
        link: Option<LinkResult>,
    ) -> Self {
        Self {
            schema: SCHEMA.to_string(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default(),
            machine,
            config,
            raw,
            components: scored.components,
            overall: scored.overall,
            link,
            soak: None,
        }
    }
}

pub struct ScoredRun {
    pub components: Vec<ScoredComponent>,
    pub overall: Overall,
}

/// Score a set of benchmark outcomes against a baseline and profile.
pub fn score_run(
    outcomes: &[BenchmarkOutcome],
    baseline: &Baseline,
    profile: Profile,
    curve_k: f64,
) -> Result<ScoredRun> {
    ensure!(
        (0.05..=3.0).contains(&curve_k),
        "curve k must be between 0.05 and 3.0 (got {curve_k})"
    );
    ensure!(!outcomes.is_empty(), "nothing to score");

    let mut components = Vec::with_capacity(outcomes.len());
    for bench in outcomes {
        let mut subtests = Vec::with_capacity(bench.subtests.len());
        for st in &bench.subtests {
            let reference = baseline.lookup(&bench.id, &st.id)?;
            ensure!(
                st.value > 0.0,
                "{}/{} produced a non-positive value ({})",
                bench.id,
                st.id,
                st.value
            );
            let ratio = match st.direction {
                Direction::HigherIsBetter => st.value / reference,
                Direction::LowerIsBetter => reference / st.value,
            };
            subtests.push(ScoredSubtest {
                id: st.id.clone(),
                label: st.label.clone(),
                unit: st.unit.clone(),
                value: st.value,
                baseline: reference,
                ratio,
                score: 1000.0 * ratio.powf(curve_k),
                confidence: st.confidence,
            });
        }

        let subtest_scores: Vec<f64> = subtests.iter().map(|s| s.score).collect();
        let score = crate::util::geomean(&subtest_scores);
        let confidence = subtests
            .iter()
            .map(|s| s.confidence)
            .min_by_key(|c| c.rank())
            .unwrap_or(Confidence::High);
        components.push(ScoredComponent {
            id: bench.id.clone(),
            label: bench.label.clone(),
            score,
            grade: Grade::from_score(score),
            graded: !UNGRADED_COMPONENTS.contains(&bench.id.as_str()),
            confidence,
            subtests,
            notes: bench.notes.clone(),
        });
    }

    let weighted: Vec<(f64, f64)> = components
        .iter()
        .filter(|c| c.graded)
        .map(|c| (c.score, profile.weight(&c.id)))
        .collect();
    let overall_score = weighted_geomean(&weighted);
    let overall = Overall {
        score: overall_score,
        grade: Grade::from_score(overall_score),
        profile: profile.name.to_string(),
        why: explain(&components),
    };

    Ok(ScoredRun {
        components,
        overall,
    })
}

/// Rank the graded components by distance from the baseline and phrase the
/// outliers. Ungraded components (e.g. network) are left out of the "why".
fn explain(components: &[ScoredComponent]) -> Vec<String> {
    let mut by_score: Vec<&ScoredComponent> = components.iter().filter(|c| c.graded).collect();
    by_score.sort_by(|a, b| a.score.total_cmp(&b.score));

    let mut why = Vec::new();
    for c in by_score.iter().take(2) {
        if c.score < 850.0 {
            why.push(format!("held back by {} (score {:.0})", c.label, c.score));
        }
    }
    for c in by_score.iter().rev().take(2) {
        if c.score > 1150.0 {
            why.push(format!("lifted by {} (score {:.0})", c.label, c.score));
        }
    }
    if why.is_empty() {
        why.push("balanced — every component is close to the reference baseline".to_string());
    }

    let low: Vec<&str> = components
        .iter()
        .filter(|c| c.graded && c.confidence == Confidence::Low)
        .map(|c| c.label.as_str())
        .collect();
    if !low.is_empty() {
        why.push(format!("low measurement confidence in: {}", low.join(", ")));
    }
    why
}

/// Weighted geometric mean over `(value, weight)` pairs.
fn weighted_geomean(pairs: &[(f64, f64)]) -> f64 {
    let wsum: f64 = pairs.iter().map(|(_, w)| w).sum();
    if wsum <= 0.0 {
        return 0.0;
    }
    let acc: f64 = pairs.iter().map(|(v, w)| w * v.max(1e-9).ln()).sum();
    (acc / wsum).exp()
}

/// Load result files from `paths` and build a baseline TOML document from them.
pub fn generate_baseline(
    paths: &[PathBuf],
    name: &str,
    description: Option<&str>,
) -> Result<String> {
    ensure!(!paths.is_empty(), "no result files given");
    let mut results = Vec::with_capacity(paths.len());
    for path in paths {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let result: ResultFile = serde_json::from_str(&text)
            .with_context(|| format!("parsing {} as a loadbearer result file", path.display()))?;
        ensure!(
            result.schema.starts_with("loadbearer.result/"),
            "{}: not a loadbearer result file",
            path.display()
        );
        results.push(result);
    }
    Ok(baseline_from_results(&results, name, description))
}

/// Build a baseline TOML document by taking, for each subtest, the geometric
/// mean of its raw value across `results`. Warnings about subtests missing from
/// some files go to stderr.
pub fn baseline_from_results(
    results: &[ResultFile],
    name: &str,
    description: Option<&str>,
) -> String {
    let n = results.len();
    let mut acc: BTreeMap<String, BTreeMap<String, Vec<f64>>> = BTreeMap::new();
    for result in results {
        for component in &result.raw {
            for st in &component.subtests {
                acc.entry(component.id.clone())
                    .or_default()
                    .entry(st.id.clone())
                    .or_default()
                    .push(st.value);
            }
        }
    }

    let mut components: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for (component, subs) in acc {
        let mut out = BTreeMap::new();
        for (subtest, values) in subs {
            if values.len() != n {
                eprintln!(
                    "warning: {component}/{subtest} present in {}/{n} files",
                    values.len()
                );
            }
            out.insert(subtest, crate::util::geomean(&values));
        }
        components.insert(component, out);
    }

    let baseline = Baseline {
        name: name.to_string(),
        description: description
            .map(str::to_string)
            .unwrap_or_else(|| format!("geometric mean of {n} result file(s)")),
        components,
    };

    let body = toml::to_string(&baseline).expect("baseline serialises to TOML");
    let header = format!(
        "# loadbearer baseline — {name}\n\
         # Generated {} from {n} result file(s) by loadbearer {}.\n\
         # Each value is the geometric mean of that subtest's raw measurement.\n\n",
        OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default(),
        env!("CARGO_PKG_VERSION"),
    );
    format!("{header}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::stats::Stats;
    use crate::engine::{BenchmarkOutcome, SubtestOutcome};

    fn subtest(id: &str, dir: Direction, value: f64) -> SubtestOutcome {
        SubtestOutcome {
            id: id.to_string(),
            label: id.to_string(),
            unit: "u".to_string(),
            direction: dir,
            value,
            stats: Stats::from_runs(vec![value]),
            confidence: Confidence::High,
        }
    }

    /// A synthetic outcome set that exactly matches the baseline everywhere.
    fn baseline_matching_outcomes(b: &Baseline) -> Vec<BenchmarkOutcome> {
        let mut out = Vec::new();
        for (comp, subs) in &b.components {
            let subtests = subs
                .iter()
                .map(|(id, &v)| {
                    let d = if id == "latency" || id == "tcp_rtt" {
                        Direction::LowerIsBetter
                    } else {
                        Direction::HigherIsBetter
                    };
                    subtest(id, d, v)
                })
                .collect();
            out.push(BenchmarkOutcome {
                id: comp.clone(),
                label: comp.clone(),
                subtests,
                notes: vec![],
            });
        }
        out
    }

    fn fake_inventory() -> Inventory {
        Inventory {
            hostname: Some("host".into()),
            os: Some("TestOS".into()),
            kernel: None,
            arch: "x86_64".into(),
            cpu_model: "Test CPU".into(),
            cpu_vendor: "TestVendor".into(),
            cpu_physical_cores: Some(4),
            cpu_logical_cores: 8,
            cpu_mhz_spot: 0,
            ram_bytes: 16 << 30,
            swap_bytes: 0,
            disks: vec![],
            gpu: None,
        }
    }

    #[test]
    fn result_file_survives_a_json_round_trip() {
        let b = Baseline::reference_v1();
        let outcomes = baseline_matching_outcomes(&b);
        let scored = score_run(&outcomes, &b, GENERAL_FOR_TEST(), 0.5).unwrap();
        let config = RunConfig {
            profile: "general".into(),
            duration_preset: "short".into(),
            curve_k: 0.5,
            seed: 1,
            threads: 8,
            baseline: b.name.clone(),
            only: vec![],
        };
        let result = ResultFile::assemble(fake_inventory(), config, outcomes, scored, None);

        let json = serde_json::to_string(&result).unwrap();
        let back: ResultFile = serde_json::from_str(&json).unwrap();

        assert_eq!(back.schema, SCHEMA);
        assert_eq!(back.components.len(), result.components.len());
        assert!((back.overall.score - result.overall.score).abs() < 1e-9);
        assert_eq!(back.overall.grade, result.overall.grade);
        assert_eq!(back.raw.len(), result.raw.len());
        assert_eq!(back.machine.cpu_logical_cores, 8);
    }

    fn result_matching_baseline() -> ResultFile {
        let b = Baseline::reference_v1();
        let outcomes = baseline_matching_outcomes(&b);
        let scored = score_run(&outcomes, &b, GENERAL_FOR_TEST(), 0.5).unwrap();
        let config = RunConfig {
            profile: "general".into(),
            duration_preset: "short".into(),
            curve_k: 0.5,
            seed: 1,
            threads: 8,
            baseline: b.name.clone(),
            only: vec![],
        };
        ResultFile::assemble(fake_inventory(), config, outcomes, scored, None)
    }

    #[test]
    fn generated_baseline_reproduces_the_inputs() {
        let reference = Baseline::reference_v1();
        let results = vec![result_matching_baseline(), result_matching_baseline()];

        let toml_doc = baseline_from_results(&results, "regen", Some("round trip"));
        let regen: Baseline = toml::from_str(&toml_doc).expect("generated baseline parses");

        assert_eq!(regen.name, "regen");
        for (component, subs) in &reference.components {
            for (subtest, &want) in subs {
                let got = regen.lookup(component, subtest).unwrap();
                // geometric mean of two identical values is that value
                assert!(
                    (got - want).abs() / want < 1e-9,
                    "{component}/{subtest}: {got} vs {want}"
                );
            }
        }
    }

    #[test]
    fn matching_the_baseline_scores_1000_grade_b() {
        let b = Baseline::reference_v1();
        let outcomes = baseline_matching_outcomes(&b);
        let scored = score_run(&outcomes, &b, GENERAL_FOR_TEST(), 0.5).unwrap();
        for c in &scored.components {
            assert!((c.score - 1000.0).abs() < 1.0, "{} -> {}", c.id, c.score);
            assert_eq!(c.grade, Grade::B);
        }
        assert!((scored.overall.score - 1000.0).abs() < 1.0);
        assert_eq!(scored.overall.grade, Grade::B);
    }

    #[test]
    fn network_is_scored_but_excluded_from_the_overall() {
        let b = Baseline::reference_v1();
        let mut outcomes = baseline_matching_outcomes(&b);
        // Tank the network component only.
        for bench in &mut outcomes {
            if bench.id == "network" {
                for st in &mut bench.subtests {
                    st.value = match st.direction {
                        Direction::HigherIsBetter => st.value / 20.0,
                        Direction::LowerIsBetter => st.value * 20.0,
                    };
                }
            }
        }
        let scored = score_run(&outcomes, &b, GENERAL_FOR_TEST(), 0.5).unwrap();

        let net = scored
            .components
            .iter()
            .find(|c| c.id == "network")
            .unwrap();
        assert!(!net.graded);
        assert!(
            net.score < 400.0,
            "network should still be scored low: {}",
            net.score
        );

        // CPU/memory/disk still match the baseline, so the overall is unmoved.
        assert!(
            (scored.overall.score - 1000.0).abs() < 1.0,
            "overall moved with network: {}",
            scored.overall.score
        );
        assert!(
            !scored
                .overall
                .why
                .iter()
                .any(|w| w.to_lowercase().contains("network"))
        );
    }

    #[test]
    fn doubling_every_metric_moves_score_by_two_pow_k() {
        let b = Baseline::reference_v1();
        let mut outcomes = baseline_matching_outcomes(&b);
        for bench in &mut outcomes {
            for st in &mut bench.subtests {
                // Double the "goodness": raise higher-is-better, lower the latency.
                st.value = match st.direction {
                    Direction::HigherIsBetter => st.value * 2.0,
                    Direction::LowerIsBetter => st.value / 2.0,
                };
            }
        }
        let scored = score_run(&outcomes, &b, GENERAL_FOR_TEST(), 0.5).unwrap();
        let expected = 1000.0 * 2f64.powf(0.5);
        assert!(
            (scored.overall.score - expected).abs() < 1.0,
            "{}",
            scored.overall.score
        );
        assert_eq!(scored.overall.grade, Grade::S); // 1414 -> S
    }

    #[test]
    fn latency_direction_is_inverted() {
        let b = Baseline::reference_v1();
        let outcomes = vec![BenchmarkOutcome {
            id: "memory".to_string(),
            label: "Memory".to_string(),
            subtests: vec![subtest("latency", Direction::LowerIsBetter, 47.5)], // half the 95ns baseline
            notes: vec![],
        }];
        let scored = score_run(&outcomes, &b, GENERAL_FOR_TEST(), 1.0).unwrap();
        // ratio 2.0, k 1.0 -> score 2000
        assert!((scored.components[0].subtests[0].score - 2000.0).abs() < 1.0);
    }

    #[test]
    fn rejects_out_of_range_curve_k() {
        let b = Baseline::reference_v1();
        let outcomes = baseline_matching_outcomes(&b);
        assert!(score_run(&outcomes, &b, GENERAL_FOR_TEST(), 0.0).is_err());
        assert!(score_run(&outcomes, &b, GENERAL_FOR_TEST(), 5.0).is_err());
    }

    // Helper: the GENERAL profile (kept local so the test file needs no re-export).
    #[allow(non_snake_case)]
    fn GENERAL_FOR_TEST() -> Profile {
        profile_by_name("general").unwrap()
    }
}
