//! `loadbearer score` — re-score an existing result file against a different
//! baseline, profile or curve, without re-running any benchmark.
//!
//! A result file keeps every raw measurement (`raw`), so the scored `components`
//! and `overall` are just one view of it. This command recomputes that view:
//! point it at a baseline you built from your own fleet
//! ([`loadbearer baseline`](../scoring/fn.generate_baseline.html)), or try a
//! different profile weighting or curve, and see where the machine lands —
//! seconds instead of another benchmark run.

use anyhow::{Context, Result, ensure};

use crate::cli::ScoreArgs;
use crate::engine::BenchmarkOutcome;
use crate::output;
use crate::scoring::{
    Baseline, Grade, Profile, ResultFile, RunConfig, profile_by_name, profile_names, score_run,
};

pub fn execute(args: ScoreArgs) -> Result<()> {
    let text = std::fs::read_to_string(&args.file)
        .with_context(|| format!("reading {}", args.file.display()))?;
    let original: ResultFile = serde_json::from_str(&text).with_context(|| {
        format!(
            "parsing {} as a loadbearer result file",
            args.file.display()
        )
    })?;
    ensure!(
        original.schema.starts_with("loadbearer.result/"),
        "{}: not a loadbearer result file (schema {:?})",
        args.file.display(),
        original.schema
    );

    let baseline = match &args.baseline {
        Some(path) => Baseline::load(path)?,
        None => Baseline::reference_v1(),
    };

    let profile_name = args
        .profile
        .clone()
        .unwrap_or_else(|| original.config.profile.clone());
    let profile = profile_by_name(&profile_name).with_context(|| {
        format!(
            "unknown profile {profile_name:?}; known profiles: {}",
            profile_names().join(", ")
        )
    })?;

    let curve_k = args.curve_k.unwrap_or(original.config.curve_k);

    let (rescored, skipped) = rescore(&original, &baseline, profile, curve_k)?;
    for s in &skipped {
        eprintln!(
            "note: {s} — no entry in baseline {:?}, left out",
            baseline.name
        );
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&rescored)?);
    } else {
        print_diff(&original, &rescored);
        output::print_scored_report(&rescored);
    }
    if let Some(path) = &args.output {
        let json = serde_json::to_string_pretty(&rescored)?;
        std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
        if !args.json {
            eprintln!("\nre-scored result written to {}", path.display());
        }
    }
    Ok(())
}

/// Recompute `components` / `overall` for `original` against a new baseline,
/// profile and curve. `machine`, `raw`, `link` and `soak` are carried over
/// unchanged; `config` is updated to record what it was scored with.
///
/// Subtests the baseline has no entry for are left out of the score rather than
/// being a hard error — a fleet baseline that omits, say, the network component
/// is a normal thing to re-score against. The names of anything skipped are
/// returned alongside the result.
pub fn rescore(
    original: &ResultFile,
    baseline: &Baseline,
    profile: Profile,
    curve_k: f64,
) -> Result<(ResultFile, Vec<String>)> {
    ensure!(
        !original.raw.is_empty(),
        "result file has no raw metrics to score"
    );
    let (scorable, skipped) = covered_by(&original.raw, baseline);
    ensure!(
        !scorable.is_empty(),
        "baseline {:?} has no entry for any subtest in this file",
        baseline.name
    );
    let scored = score_run(&scorable, baseline, profile, curve_k)?;

    let config = RunConfig {
        profile: profile.name.to_string(),
        duration_preset: original.config.duration_preset.clone(),
        curve_k,
        seed: original.config.seed,
        threads: original.config.threads,
        baseline: baseline.name.clone(),
        only: original.config.only.clone(),
    };

    // The output keeps the file's full `raw`; only the scored view is the
    // covered subset.
    let mut out = ResultFile::assemble(
        original.machine.clone(),
        config,
        original.raw.clone(),
        scored,
        original.link.clone(),
    );
    out.soak = original.soak.clone();
    Ok((out, skipped))
}

/// Split `raw` into the part the baseline can score and a list of skipped
/// `component/subtest` names.
fn covered_by(
    raw: &[BenchmarkOutcome],
    baseline: &Baseline,
) -> (Vec<BenchmarkOutcome>, Vec<String>) {
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    for comp in raw {
        let mut subs = Vec::new();
        let mut missing = Vec::new();
        for st in &comp.subtests {
            if baseline.lookup(&comp.id, &st.id).is_ok() {
                subs.push(st.clone());
            } else {
                missing.push(format!("{}/{}", comp.id, st.id));
            }
        }
        if subs.is_empty() {
            if !comp.subtests.is_empty() {
                skipped.push(format!("{} (whole component)", comp.id));
            }
        } else {
            skipped.extend(missing);
            kept.push(BenchmarkOutcome {
                id: comp.id.clone(),
                label: comp.label.clone(),
                subtests: subs,
                notes: comp.notes.clone(),
            });
        }
    }
    (kept, skipped)
}

/// One-line-per-knob summary of what changed between the stored scoring and the
/// re-score, so the report that follows has context.
fn print_diff(before: &ResultFile, after: &ResultFile) {
    let m = &after.machine;
    println!(
        "\nre-scoring {} ({})",
        m.hostname.as_deref().unwrap_or("unknown"),
        before.timestamp,
    );
    if before.tool_version != after.tool_version {
        knob("tool", &before.tool_version, &after.tool_version);
    }
    knob("baseline", &before.config.baseline, &after.config.baseline);
    knob("profile", &before.config.profile, &after.config.profile);
    knob(
        "curve k",
        &format!("{}", before.config.curve_k),
        &format!("{}", after.config.curve_k),
    );
    knob(
        "overall",
        &grade_str(before.overall.score, before.overall.grade),
        &grade_str(after.overall.score, after.overall.grade),
    );
}

fn knob(label: &str, before: &str, after: &str) {
    if before == after {
        println!("  {label:<9} {before}");
    } else {
        println!("  {label:<9} {before}  \u{2192}  {after}");
    }
}

fn grade_str(score: f64, grade: Grade) -> String {
    format!("{score:.0} [{}]", grade.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::stats::Stats;
    use crate::engine::{BenchmarkOutcome, Direction, SubtestOutcome};
    use crate::inventory::Inventory;
    use crate::scoring::RunConfig;

    fn sub(id: &str, value: f64) -> SubtestOutcome {
        SubtestOutcome {
            id: id.to_string(),
            label: id.to_string(),
            unit: "u".to_string(),
            direction: Direction::HigherIsBetter,
            value,
            stats: Stats::from_runs(vec![value]),
            confidence: crate::engine::stats::Confidence::High,
        }
    }

    /// A result whose only component is `cpu`, matching `reference-v1` exactly
    /// on the two integer subtests, scored `general` / k=0.5.
    fn baseline_matching_cpu_result() -> ResultFile {
        let b = Baseline::reference_v1();
        let int_single = b.lookup("cpu", "int_single").unwrap();
        let int_multi = b.lookup("cpu", "int_multi").unwrap();
        let raw = vec![BenchmarkOutcome {
            id: "cpu".to_string(),
            label: "CPU".to_string(),
            subtests: vec![sub("int_single", int_single), sub("int_multi", int_multi)],
            notes: vec![],
        }];
        let scored = score_run(&raw, &b, profile_by_name("general").unwrap(), 0.5).unwrap();
        let config = RunConfig {
            profile: "general".into(),
            duration_preset: "short".into(),
            curve_k: 0.5,
            seed: 1,
            threads: 4,
            baseline: b.name.clone(),
            only: vec!["cpu".into()],
        };
        ResultFile::assemble(fake_inventory(), config, raw, scored, None)
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
            battery: None,
        }
    }

    fn softer_baseline() -> Baseline {
        let mut b = Baseline::reference_v1();
        for subs in b.components.values_mut() {
            for v in subs.values_mut() {
                *v /= 2.0;
            }
        }
        b.name = "soft".into();
        b
    }

    #[test]
    fn original_matches_its_baseline_at_1000() {
        let r = baseline_matching_cpu_result();
        assert!((r.overall.score - 1000.0).abs() < 1.0);
    }

    #[test]
    fn rescoring_against_a_softer_baseline_raises_the_score() {
        let original = baseline_matching_cpu_result();
        let soft = softer_baseline();
        let (out, skipped) =
            rescore(&original, &soft, profile_by_name("general").unwrap(), 0.5).unwrap();

        assert!(skipped.is_empty());
        // Every value is now 2x its baseline; at k=0.5 that is 1000 * sqrt(2).
        let expected = 1000.0 * 2f64.powf(0.5);
        assert!(
            (out.overall.score - expected).abs() < 2.0,
            "got {}",
            out.overall.score
        );
        assert_eq!(out.config.baseline, "soft");
        assert_eq!(out.config.curve_k, 0.5);
        // raw is carried through untouched.
        assert_eq!(out.raw.len(), original.raw.len());
        assert_eq!(out.raw[0].subtests.len(), original.raw[0].subtests.len());
    }

    #[test]
    fn rescoring_honours_a_new_curve() {
        let original = baseline_matching_cpu_result();
        let soft = softer_baseline();
        let (out, _) = rescore(&original, &soft, profile_by_name("general").unwrap(), 1.0).unwrap();
        // ratio 2.0 at k=1.0 -> 2000.
        assert!(
            (out.overall.score - 2000.0).abs() < 2.0,
            "got {}",
            out.overall.score
        );
    }

    #[test]
    fn a_subtest_the_baseline_lacks_is_skipped_not_fatal() {
        let original = baseline_matching_cpu_result();
        let mut b = Baseline::reference_v1();
        b.components.get_mut("cpu").unwrap().remove("int_multi");
        let (out, skipped) =
            rescore(&original, &b, profile_by_name("general").unwrap(), 0.5).unwrap();
        assert_eq!(skipped, vec!["cpu/int_multi".to_string()]);
        // Still scored on int_single, which matches -> ~1000.
        assert!(
            (out.overall.score - 1000.0).abs() < 1.0,
            "got {}",
            out.overall.score
        );
        // Scored view has one subtest; the file's raw is left whole.
        assert_eq!(out.components[0].subtests.len(), 1);
        assert_eq!(out.raw[0].subtests.len(), 2);
    }

    #[test]
    fn a_baseline_that_covers_nothing_is_an_error() {
        let original = baseline_matching_cpu_result();
        let mut b = Baseline::reference_v1();
        b.components.remove("cpu");
        assert!(rescore(&original, &b, profile_by_name("general").unwrap(), 0.5).is_err());
    }

    #[test]
    fn empty_raw_is_rejected() {
        let mut original = baseline_matching_cpu_result();
        original.raw.clear();
        assert!(
            rescore(
                &original,
                &Baseline::reference_v1(),
                profile_by_name("general").unwrap(),
                0.5
            )
            .is_err()
        );
    }
}
