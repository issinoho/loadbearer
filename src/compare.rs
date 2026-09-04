//! `loadbearer compare` — head-to-head of two or more result files.
//!
//! The verdict is built from the **raw** metrics in each file, so it does not
//! depend on the baseline or curve the files were scored with. For every subtest
//! present in all files, each machine gets a direction-adjusted ratio to the
//! first machine (>1 means faster / lower-latency). Component and overall figures
//! are geometric means of those ratios, each component weighted equally.

use std::collections::BTreeSet;
use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::cli::CompareArgs;
use crate::engine::Direction;
use crate::output;
use crate::scoring::ResultFile;
use crate::soak::SoakResult;
use crate::util::geomean;

/// Schema tag for `loadbearer compare --json`. Additive fields keep `/1`; a
/// removal, rename or type change bumps it (and is a breaking release).
pub const COMPARE_SCHEMA: &str = "loadbearer.compare/1";

pub fn execute(args: CompareArgs) -> Result<()> {
    log::info!(target: "loadbearer::compare", "comparing {} result file(s)", args.files.len());
    let mut machines = load(&args.files)?;

    if let Some(query) = &args.against {
        let table = crate::scoring::models::ModelTable::embedded();
        let (kind, entry) = table
            .cpu(query)
            .map(|e| ("cpu", e))
            .or_else(|| table.gpu(query).map(|e| ("gpu", e)))
            .ok_or_else(|| anyhow::anyhow!("no model reference matches {query:?}"))?;
        let file = crate::scoring::models::synthetic_result(kind, entry)?;
        machines.push(Machine {
            label: format!("{} (ref)", entry.model),
            path: "(model reference)".to_string(),
            file,
        });
    }

    if machines.len() < 2 {
        bail!("compare needs at least two machines — pass another result file or --against MODEL");
    }

    let comparison = compare(&machines)?;
    for w in &comparison.warnings {
        log::warn!(target: "loadbearer::compare", "{w}");
    }
    log::info!(target: "loadbearer::compare", "verdict: {}", comparison.overall.summary);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&comparison)?);
    } else if args.plain || !std::io::stdout().is_terminal() {
        output::print_comparison(&comparison);
    } else {
        crate::tui::run_compare(&comparison)?;
    }
    Ok(())
}

struct Machine {
    label: String,
    path: String,
    file: ResultFile,
}

#[derive(Debug, Serialize)]
pub struct MachineRef {
    pub tag: String,
    pub label: String,
    pub path: String,
    pub cpu_model: String,
    pub baseline: String,
    pub curve_k: f64,
    pub duration_preset: String,
    pub profile: String,
    pub stored_overall_score: f64,
    pub stored_overall_grade: String,
}

#[derive(Debug, Serialize)]
pub struct SubtestComparison {
    pub id: String,
    pub label: String,
    pub unit: String,
    pub direction: Direction,
    /// Raw value per machine, in machine order.
    pub values: Vec<f64>,
    /// Direction-adjusted ratio to machine 0 (`rel[0] == 1.0`).
    pub rel: Vec<f64>,
    /// Index of the machine that did best on this subtest.
    pub best: usize,
    /// `false` for an informational subtest — shown with its per-metric delta
    /// but kept out of the component and overall rollup.
    pub scored: bool,
}

#[derive(Debug, Serialize)]
pub struct ComponentComparison {
    pub id: String,
    pub label: String,
    pub subtests: Vec<SubtestComparison>,
    /// Geometric mean of the subtest `rel` values, per machine.
    pub rel: Vec<f64>,
    pub best: usize,
}

#[derive(Debug, Serialize)]
pub struct OverallComparison {
    /// Geometric mean over components of their `rel`, per machine.
    pub rel: Vec<f64>,
    /// Machine indices, best first.
    pub ranking: Vec<usize>,
    pub summary: String,
}

/// Sustained-load figures, present only when every file carries `--soak` data.
/// Not folded into the verdict — shown as its own block, like the network
/// component is kept out of the overall grade.
#[derive(Debug, Serialize)]
pub struct SoakComparison {
    pub unit: String,
    /// Steady-state throughput per machine.
    pub steady_rate: Vec<f64>,
    /// Unthrottled peak throughput per machine.
    pub peak_rate: Vec<f64>,
    /// Steady-state retained as a percentage of each machine's own peak.
    pub retained_pct: Vec<f64>,
    /// Steady-state throughput relative to machine 0 (`rel_steady[0] == 1.0`).
    pub rel_steady: Vec<f64>,
    /// Machine holding the highest fraction of its own peak.
    pub best_retention: usize,
    /// Machine with the highest absolute sustained throughput.
    pub best_sustained: usize,
}

#[derive(Debug, Serialize)]
pub struct Comparison {
    pub schema: String,
    pub tool_version: String,
    pub machines: Vec<MachineRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    pub components: Vec<ComponentComparison>,
    pub overall: OverallComparison,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soak: Option<SoakComparison>,
}

fn load(paths: &[PathBuf]) -> Result<Vec<Machine>> {
    let mut machines = Vec::with_capacity(paths.len());
    for path in paths {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let file: ResultFile = serde_json::from_str(&text)
            .with_context(|| format!("parsing {} as a loadbearer result file", path.display()))?;
        if !file.schema.starts_with("loadbearer.result/") {
            bail!(
                "{}: not a loadbearer result file (schema {:?})",
                path.display(),
                file.schema
            );
        }
        machines.push(Machine {
            label: String::new(),
            path: path.display().to_string(),
            file,
        });
    }

    for (i, m) in machines.iter_mut().enumerate() {
        let host = m
            .file
            .machine
            .hostname
            .clone()
            .filter(|h| !h.trim().is_empty());
        let stem = paths[i]
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned());
        m.label = host.or(stem).unwrap_or_else(|| format!("machine{}", i + 1));
    }
    // Disambiguate identical labels.
    let labels: Vec<String> = machines.iter().map(|m| m.label.clone()).collect();
    for (i, m) in machines.iter_mut().enumerate() {
        if labels
            .iter()
            .enumerate()
            .any(|(j, l)| j != i && *l == labels[i])
        {
            m.label = format!("{}#{}", m.label, i + 1);
        }
    }
    Ok(machines)
}

fn tag(i: usize) -> String {
    if i < 26 {
        ((b'A' + i as u8) as char).to_string()
    } else {
        format!("M{}", i + 1)
    }
}

fn goodness(value: f64, reference: f64, dir: Direction) -> f64 {
    if value <= 0.0 || reference <= 0.0 {
        return 1.0;
    }
    let r = value / reference;
    match dir {
        Direction::HigherIsBetter => r,
        Direction::LowerIsBetter => 1.0 / r,
    }
}

fn argmax(xs: &[f64]) -> usize {
    xs.iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(i, _)| i)
}

fn compare(machines: &[Machine]) -> Result<Comparison> {
    let n = machines.len();
    let first = &machines[0].file;
    let mut warnings = config_warnings(machines);

    let mut components = Vec::new();
    for c0 in &first.raw {
        if !machines
            .iter()
            .all(|m| m.file.raw.iter().any(|c| c.id == c0.id))
        {
            warnings.push(format!(
                "component {:?} is not present in every result file — skipped",
                c0.id
            ));
            continue;
        }

        let mut subtests = Vec::new();
        for st0 in &c0.subtests {
            let per_machine: Option<Vec<(f64, Direction)>> = machines
                .iter()
                .map(|m| {
                    m.file
                        .raw
                        .iter()
                        .find(|c| c.id == c0.id)
                        .and_then(|c| c.subtests.iter().find(|s| s.id == st0.id))
                        .map(|s| (s.value, s.direction))
                })
                .collect();
            let Some(per_machine) = per_machine else {
                warnings.push(format!(
                    "subtest {}/{} is not in every result file — skipped",
                    c0.id, st0.id
                ));
                continue;
            };

            let dir = st0.direction;
            if per_machine.iter().any(|(_, d)| *d != dir) {
                warnings.push(format!(
                    "subtest {}/{} has inconsistent direction across files — using the first",
                    c0.id, st0.id
                ));
            }
            let values: Vec<f64> = per_machine.iter().map(|(v, _)| *v).collect();
            let reference = values[0];
            let rel: Vec<f64> = values
                .iter()
                .map(|v| goodness(*v, reference, dir))
                .collect();
            let best = argmax(&rel);
            subtests.push(SubtestComparison {
                id: st0.id.clone(),
                label: st0.label.clone(),
                unit: st0.unit.clone(),
                direction: dir,
                values,
                rel,
                best,
                scored: st0.scored,
            });
        }

        if subtests.is_empty() {
            warnings.push(format!(
                "component {:?} has no subtests shared by every file — skipped",
                c0.id
            ));
            continue;
        }

        let rel: Vec<f64> = (0..n)
            .map(|mi| {
                geomean(
                    &subtests
                        .iter()
                        .filter(|s| s.scored)
                        .map(|s| s.rel[mi])
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let best = argmax(&rel);
        components.push(ComponentComparison {
            id: c0.id.clone(),
            label: c0.label.clone(),
            subtests,
            rel,
            best,
        });
    }

    if components.is_empty() {
        bail!("result files share no common components to compare");
    }

    let overall_rel: Vec<f64> = (0..n)
        .map(|mi| geomean(&components.iter().map(|c| c.rel[mi]).collect::<Vec<_>>()))
        .collect();
    let mut ranking: Vec<usize> = (0..n).collect();
    ranking.sort_by(|&a, &b| overall_rel[b].total_cmp(&overall_rel[a]));

    let summary = summarize(machines, &components, &overall_rel, &ranking);

    let machine_refs = machines
        .iter()
        .enumerate()
        .map(|(i, m)| MachineRef {
            tag: tag(i),
            label: m.label.clone(),
            path: m.path.clone(),
            cpu_model: m.file.machine.cpu_model.clone(),
            baseline: m.file.config.baseline.clone(),
            curve_k: m.file.config.curve_k,
            duration_preset: m.file.config.duration_preset.clone(),
            profile: m.file.config.profile.clone(),
            stored_overall_score: m.file.overall.score,
            stored_overall_grade: m.file.overall.grade.as_str().to_string(),
        })
        .collect();

    Ok(Comparison {
        schema: COMPARE_SCHEMA.to_string(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        machines: machine_refs,
        warnings,
        components,
        overall: OverallComparison {
            rel: overall_rel,
            ranking,
            summary,
        },
        soak: soak_comparison(machines),
    })
}

/// Build the sustained-load comparison, but only if *every* file carries soak
/// data (a `loadbearer run --soak`).
fn soak_comparison(machines: &[Machine]) -> Option<SoakComparison> {
    let soaks: Vec<&SoakResult> = machines
        .iter()
        .map(|m| m.file.soak.as_ref())
        .collect::<Option<Vec<_>>>()?;

    let steady_rate: Vec<f64> = soaks.iter().map(|s| s.steady_rate).collect();
    let peak_rate: Vec<f64> = soaks.iter().map(|s| s.peak_rate).collect();
    let retained_pct: Vec<f64> = soaks.iter().map(|s| s.retained_pct).collect();
    let reference = steady_rate[0].max(1e-9);
    let rel_steady: Vec<f64> = steady_rate.iter().map(|v| v / reference).collect();

    Some(SoakComparison {
        unit: soaks[0].unit.clone(),
        best_retention: argmax(&retained_pct),
        best_sustained: argmax(&steady_rate),
        steady_rate,
        peak_rate,
        retained_pct,
        rel_steady,
    })
}

fn config_warnings(machines: &[Machine]) -> Vec<String> {
    let mut w = Vec::new();
    // A synthetic model-reference machine (--against) has no real run config;
    // don't warn about its "reference" preset / OS etc.
    let real: Vec<&Machine> = machines
        .iter()
        .filter(|m| m.file.config.duration_preset != "reference")
        .collect();
    let distinct = |f: &dyn Fn(&Machine) -> String| -> Vec<String> {
        real.iter()
            .map(|m| f(m))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };

    let baselines = distinct(&|m| m.file.config.baseline.clone());
    if baselines.len() > 1 {
        w.push(format!(
            "files were scored against different baselines ({}); comparison uses raw metrics",
            baselines.join(", ")
        ));
    }
    let presets = distinct(&|m| m.file.config.duration_preset.clone());
    if presets.len() > 1 {
        w.push(format!(
            "files used different duration presets ({}); shorter runs are noisier",
            presets.join(", ")
        ));
    }
    let ks = distinct(&|m| format!("{}", m.file.config.curve_k));
    if ks.len() > 1 {
        w.push(format!(
            "files used different curve k ({}); stored scores are not directly comparable",
            ks.join(", ")
        ));
    }
    let oses: Vec<String> = distinct(&|m| m.file.machine.os.clone().unwrap_or_default())
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    if oses.len() > 1 {
        w.push(format!(
            "files are from different operating systems ({}); the network component in \
             particular is not comparable across OSes",
            oses.join(", ")
        ));
    }
    w
}

fn summarize(
    machines: &[Machine],
    components: &[ComponentComparison],
    overall_rel: &[f64],
    ranking: &[usize],
) -> String {
    let lead = ranking[0];

    if machines.len() == 2 {
        let other = ranking[1];
        let advantage = overall_rel[lead] / overall_rel[other].max(1e-9) - 1.0;
        if advantage < 0.03 {
            return format!(
                "{} and {} are within 3% overall — effectively equal.",
                machines[0].label, machines[1].label
            );
        }
        let mut lead_wins = Vec::new();
        let mut other_wins = Vec::new();
        for c in components {
            let adv = c.rel[lead] / c.rel[other].max(1e-9) - 1.0;
            if adv > 0.02 {
                lead_wins.push(format!("{} +{:.0}%", c.label.to_lowercase(), adv * 100.0));
            } else if adv < -0.02 {
                other_wins.push(format!("{} +{:.0}%", c.label.to_lowercase(), -adv * 100.0));
            }
        }
        let mut s = format!(
            "{} leads by {:.0}% overall",
            machines[lead].label,
            advantage * 100.0
        );
        if !lead_wins.is_empty() {
            s.push_str(&format!(" (ahead on {})", lead_wins.join(", ")));
        }
        if !other_wins.is_empty() {
            s.push_str(&format!(
                "; {} wins {}",
                machines[other].label,
                other_wins.join(", ")
            ));
        }
        s.push('.');
        return s;
    }

    let parts: Vec<String> = ranking
        .iter()
        .enumerate()
        .map(|(rank, &mi)| {
            if rank == 0 {
                format!("1. {}", machines[mi].label)
            } else {
                let behind = overall_rel[mi] / overall_rel[lead].max(1e-9) - 1.0;
                format!(
                    "{}. {} ({:.0}%)",
                    rank + 1,
                    machines[mi].label,
                    behind * 100.0
                )
            }
        })
        .collect();
    format!("Ranking (overall vs leader): {}", parts.join("   "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goodness_respects_direction() {
        // Higher-is-better: twice the value is twice as good.
        assert!((goodness(200.0, 100.0, Direction::HigherIsBetter) - 2.0).abs() < 1e-9);
        // Lower-is-better: half the value is twice as good.
        assert!((goodness(50.0, 100.0, Direction::LowerIsBetter) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn argmax_picks_the_largest() {
        assert_eq!(argmax(&[0.9, 1.4, 1.1]), 1);
    }

    #[test]
    fn json_carries_a_schema_and_version() {
        let cmp = Comparison {
            schema: COMPARE_SCHEMA.to_string(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            machines: vec![],
            warnings: vec![],
            components: vec![],
            overall: OverallComparison {
                rel: vec![],
                ranking: vec![],
                summary: String::new(),
            },
            soak: None,
        };
        let v: serde_json::Value = serde_json::to_value(&cmp).unwrap();
        assert_eq!(v["schema"], "loadbearer.compare/1");
        assert_eq!(v["tool_version"], env!("CARGO_PKG_VERSION"));
    }
}
