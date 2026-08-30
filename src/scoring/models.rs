//! Per-CPU/GPU-model reference values: *what a healthy example of this model
//! produces on loadbearer's kernels*. Lets a single `run` say whether a chip is
//! performing to spec, not just where it lands against the baseline.
//!
//! Data, not API — like [`Baseline`](super::Baseline), it can change in any
//! release (see `VERSIONING.md`). **CPU and GPU only**: memory, disk and
//! network depend on the RAM kit, the SSD and the OS, not the model. The
//! references are measured on the released **portable (SSE2)** build, so a
//! `-C target-cpu=native` build is compared with a caveat.

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::cli::ModelsArgs;
use crate::engine::stats::{Confidence, Stats};
use crate::engine::{BenchmarkOutcome, Direction, SubtestOutcome};

/// `(id, unit, label)` for every subtest the model table can hold. All are
/// higher-is-better; none are latency.
const CPU_META: &[(&str, &str, &str)] = &[
    ("int_single", "Mops/s", "Integer, single-core"),
    ("int_multi", "Mops/s", "Integer, all cores"),
    ("float_single", "MFLOP/s", "Float, single-core"),
    ("float_multi", "MFLOP/s", "Float, all cores"),
    ("hash", "MiB/s", "BLAKE3 hash"),
    ("compress", "MiB/s", "DEFLATE compress"),
    ("aes_gcm", "MiB/s", "AES-256-GCM encrypt"),
    ("sha256", "MiB/s", "SHA-256 hash"),
];
const GPU_META: &[(&str, &str, &str)] = &[
    ("compute_fp32", "GFLOP/s", "FP32 compute (FMA)"),
    ("bandwidth", "GiB/s", "VRAM read bandwidth"),
];

const CPU_MODELS: &str = include_str!("../../baseline/models/cpu.toml");
const GPU_MODELS: &str = include_str!("../../baseline/models/gpu.toml");

/// One model's expected raw values plus where they came from.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub model: String,
    pub aliases: Vec<String>,
    pub topology: String,
    pub samples: u32,
    pub measured: String,
    pub tool_version: String,
    pub build: String,
    /// subtest id -> expected raw value
    pub values: HashMap<String, f64>,
}

/// The embedded CPU and GPU model tables, parsed once.
pub struct ModelTable {
    cpu: HashMap<String, ModelEntry>,
    gpu: HashMap<String, ModelEntry>,
}

impl ModelTable {
    pub fn embedded() -> &'static ModelTable {
        static T: OnceLock<ModelTable> = OnceLock::new();
        T.get_or_init(|| ModelTable {
            cpu: index(parse(CPU_MODELS, "cpu")),
            gpu: index(parse(GPU_MODELS, "gpu")),
        })
    }

    pub fn cpu(&self, model: &str) -> Option<&ModelEntry> {
        lookup(&self.cpu, model)
    }
    pub fn gpu(&self, model: &str) -> Option<&ModelEntry> {
        lookup(&self.gpu, model)
    }

    /// Distinct model entries, for `loadbearer models`.
    pub fn cpu_entries(&self) -> Vec<&ModelEntry> {
        distinct(&self.cpu)
    }
    pub fn gpu_entries(&self) -> Vec<&ModelEntry> {
        distinct(&self.gpu)
    }
}

fn distinct(map: &HashMap<String, ModelEntry>) -> Vec<&ModelEntry> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out: Vec<&ModelEntry> = map
        .values()
        .filter(|e| seen.insert(e.model.as_str()))
        .collect();
    out.sort_by(|a, b| a.model.cmp(&b.model));
    out
}

/// Normalise a CPU/GPU model string for matching: drop `(R)`/`(TM)`, an
/// `@ 1.70GHz` clock, a trailing ` w/ Radeon …`, the `CPU`/`Processor` noise
/// words and a leading `13th Gen `; lowercase; collapse whitespace.
pub fn normalize(s: &str) -> String {
    let mut t = s.to_lowercase();
    for p in ["(r)", "(tm)", "(c)", "®", "™"] {
        t = t.replace(p, " ");
    }
    if let Some(i) = t.find(" @ ") {
        t.truncate(i);
    }
    for sep in [" w/ ", " with radeon", " with graphics"] {
        if let Some(i) = t.find(sep) {
            t.truncate(i);
        }
    }
    t.split_whitespace()
        .filter(|w| !matches!(*w, "cpu" | "processor" | "gen"))
        // "13th", "5th" — an ordinal gen prefix
        .filter(|w| !(w.ends_with("th") && w[..w.len() - 2].chars().all(|c| c.is_ascii_digit())))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Conservative lookup: an exact normalised hit, else a distinctive SKU token
/// (≥ 4 chars, contains a digit) from the table that appears in the machine's
/// model string. Longest such token wins. No looser fuzzing — a wrong match is
/// worse than none.
fn lookup<'a>(map: &'a HashMap<String, ModelEntry>, machine_model: &str) -> Option<&'a ModelEntry> {
    let norm = normalize(machine_model);
    if let Some(e) = map.get(&norm) {
        return Some(e);
    }
    map.iter()
        .filter(|(k, _)| {
            k.len() >= 4 && k.chars().any(|c| c.is_ascii_digit()) && norm.contains(k.as_str())
        })
        .max_by_key(|(k, _)| k.len())
        .map(|(_, e)| e)
}

/// Build the normalised-key -> entry map; the `model` and every alias become
/// keys (aliases don't overwrite a real model key).
fn index(entries: Vec<ModelEntry>) -> HashMap<String, ModelEntry> {
    let mut m = HashMap::new();
    for e in entries {
        for alias in &e.aliases {
            m.entry(normalize(alias)).or_insert_with(|| e.clone());
        }
        m.insert(normalize(&e.model), e);
    }
    m
}

/// Parse `[[cpu]]` / `[[gpu]]` array-of-tables. Known keys are pulled out; every
/// other float/int key is a subtest value. Panics only on a malformed embedded
/// file (a build-time asset).
fn parse(src: &str, array_key: &str) -> Vec<ModelEntry> {
    let doc: toml::Table = toml::from_str(src).expect("embedded models TOML parses");
    let Some(toml::Value::Array(items)) = doc.get(array_key) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let t = item.as_table()?;
            let s = |k: &str| {
                t.get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            let model = t.get("model")?.as_str()?.to_string();
            let aliases = t
                .get("aliases")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let mut values = HashMap::new();
            for (k, v) in t {
                if k == "samples" {
                    continue;
                }
                if let Some(f) = v.as_float().or_else(|| v.as_integer().map(|i| i as f64)) {
                    values.insert(k.clone(), f);
                }
            }
            Some(ModelEntry {
                model,
                aliases,
                topology: s("topology"),
                samples: t
                    .get("samples")
                    .and_then(|v| v.as_integer())
                    .unwrap_or(1)
                    .max(1) as u32,
                measured: s("measured"),
                tool_version: s("tool_version"),
                build: s("build"),
                values,
            })
        })
        .collect()
}

// --- comparing a run to a model -------------------------------------------

/// A run's CPU or GPU component measured against its model reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRef {
    /// `"cpu"` or `"gpu"`.
    pub component: String,
    /// The matched entry's canonical name.
    pub model: String,
    pub samples: u32,
    pub measured: String,
    /// ISA of the build that produced this run — `sse2` means the model
    /// reference applies directly; anything else and it's only indicative.
    pub build_isa: String,
    /// Duration preset of this run. The references are measured at `thorough`;
    /// a shorter preset reads high across the board, so the gap is only
    /// approximate. Empty when the preset is unknown or already `thorough`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub run_preset: String,
    pub subtests: Vec<ModelSubtestDelta>,
    /// Geometric mean of the direction-adjusted per-subtest ratios, as a
    /// percentage — `+` means faster than the model.
    pub delta_pct: f64,
    pub verdict: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSubtestDelta {
    pub id: String,
    pub value: f64,
    pub reference: f64,
    /// Direction-adjusted: `+` means this run beat the model on this subtest.
    pub delta_pct: f64,
}

/// Compare one raw benchmark component against a model entry. `None` if no
/// subtest overlaps.
pub fn compare(
    component_id: &str,
    outcome: &BenchmarkOutcome,
    entry: &ModelEntry,
    build_isa: &str,
    run_preset: &str,
) -> Option<ModelRef> {
    let mut subtests = Vec::new();
    let mut ratios = Vec::new();
    for st in &outcome.subtests {
        let Some(&refv) = entry.values.get(&st.id) else {
            continue;
        };
        if refv <= 0.0 || st.value <= 0.0 {
            continue;
        }
        let ratio = match st.direction {
            Direction::HigherIsBetter => st.value / refv,
            Direction::LowerIsBetter => refv / st.value,
        };
        ratios.push(ratio);
        subtests.push(ModelSubtestDelta {
            id: st.id.clone(),
            value: st.value,
            reference: refv,
            delta_pct: (ratio - 1.0) * 100.0,
        });
    }
    if ratios.is_empty() {
        return None;
    }
    let delta_pct = (crate::util::geomean(&ratios) - 1.0) * 100.0;
    Some(ModelRef {
        component: component_id.to_string(),
        model: entry.model.clone(),
        samples: entry.samples,
        measured: entry.measured.clone(),
        build_isa: build_isa.to_string(),
        run_preset: if run_preset == "thorough" {
            String::new()
        } else {
            run_preset.to_string()
        },
        verdict: verdict(component_id, &entry.model, delta_pct, build_isa),
        delta_pct,
        subtests,
    })
}

/// Look up and compare the CPU and GPU components of a run. Returns 0–2
/// `ModelRef`s. Non-`sse2` builds still get a comparison, flagged in the
/// verdict.
pub fn for_run(
    cpu_model: &str,
    gpu_model: Option<&str>,
    raw: &[BenchmarkOutcome],
    build_isa: &str,
    run_preset: &str,
) -> Vec<ModelRef> {
    let table = ModelTable::embedded();
    let mut out = Vec::new();
    if let Some(o) = raw.iter().find(|o| o.id == "cpu")
        && let Some(e) = table.cpu(cpu_model)
        && let Some(r) = compare("cpu", o, e, build_isa, run_preset)
    {
        out.push(r);
    }
    if let (Some(o), Some(gm)) = (raw.iter().find(|o| o.id == "gpu"), gpu_model)
        && let Some(e) = table.gpu(gm)
        && let Some(r) = compare("gpu", o, e, build_isa, run_preset)
    {
        out.push(r);
    }
    out
}

fn verdict(component: &str, model: &str, delta_pct: f64, build_isa: &str) -> String {
    let c = component.to_uppercase();
    // The references are portable builds (SSE2 on x86, NEON on Arm). Only a
    // *wider-than-portable* x86 build — AVX and up, from `target-cpu=native` —
    // runs vector kernels the reference didn't, so only that gets the caveat.
    if matches!(build_isa, "avx" | "avx2" | "avx512") {
        return format!(
            "{c} is {delta_pct:+.0}% vs a typical {model} — but this is a {build_isa} build and \
             the reference is a portable one, so expect higher. Indicative only.",
        );
    }
    if delta_pct.abs() <= 5.0 {
        format!("{c} matches a typical {model}")
    } else if delta_pct < 0.0 {
        format!(
            "{c} is ~{:.0}% below par for this {model} — check thermals, the power profile, \
             and background load; re-run --duration thorough on mains",
            -delta_pct,
        )
    } else {
        format!(
            "{c} is ~{delta_pct:.0}% above a typical {model} (newer stepping, better cooling, \
             or a native build)",
        )
    }
}

// --- `loadbearer models` subcommand -------------------------------------

pub fn execute(args: ModelsArgs) -> Result<()> {
    if !args.add.is_empty() {
        return regenerate(&args.add);
    }
    let table = ModelTable::embedded();

    if let Some(query) = &args.model {
        let (kind, entry) = table
            .cpu(query)
            .map(|e| ("cpu", e))
            .or_else(|| table.gpu(query).map(|e| ("gpu", e)))
            .ok_or_else(|| anyhow::anyhow!("no model reference matches {query:?}"))?;
        if args.as_result {
            let synth = synthetic_result(kind, entry)?;
            println!("{}", serde_json::to_string_pretty(&synth)?);
        } else if args.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&entry_json(kind, entry))?
            );
        } else {
            print_entry(kind, entry);
        }
        return Ok(());
    }

    if args.json {
        let all: Vec<_> = table
            .cpu_entries()
            .into_iter()
            .map(|e| entry_json("cpu", e))
            .chain(
                table
                    .gpu_entries()
                    .into_iter()
                    .map(|e| entry_json("gpu", e)),
            )
            .collect();
        println!("{}", serde_json::to_string_pretty(&all)?);
    } else {
        println!(
            "loadbearer {} — model reference table\n",
            env!("CARGO_PKG_VERSION")
        );
        println!("CPU ({} models)", table.cpu_entries().len());
        for e in table.cpu_entries() {
            println!(
                "  {:<26} {:>2} sample(s)  {}  {}",
                e.model, e.samples, e.measured, e.topology
            );
        }
        println!("\nGPU ({} models)", table.gpu_entries().len());
        for e in table.gpu_entries() {
            println!(
                "  {:<26} {:>2} sample(s)  {}",
                e.model, e.samples, e.measured
            );
        }
        println!(
            "\nGrow it:  loadbearer models --add run1.json run2.json > baseline/models/cpu.toml"
        );
    }
    Ok(())
}

fn entry_json(kind: &str, e: &ModelEntry) -> serde_json::Value {
    serde_json::json!({
        "kind": kind, "model": e.model, "aliases": e.aliases, "topology": e.topology,
        "samples": e.samples, "measured": e.measured, "tool_version": e.tool_version,
        "build": e.build, "values": e.values,
    })
}

fn print_entry(kind: &str, e: &ModelEntry) {
    println!("{} · {}", e.model, kind.to_uppercase());
    if !e.topology.is_empty() {
        println!("  topology     {}", e.topology);
    }
    println!("  samples      {}", e.samples);
    println!(
        "  measured     {}  (loadbearer {})",
        e.measured, e.tool_version
    );
    println!("  build        {}", e.build);
    let meta = if kind == "cpu" { CPU_META } else { GPU_META };
    for (id, unit, label) in meta {
        if let Some(v) = e.values.get(*id) {
            println!("  {label:<22} {v:>11.1} {unit}");
        }
    }
}

/// A synthetic `loadbearer.result/1` carrying a model entry's values as the
/// `<kind>` component's raw metrics, scored against `reference-v1`. For piping
/// into `compare`.
pub fn synthetic_result(kind: &str, entry: &ModelEntry) -> Result<crate::scoring::ResultFile> {
    let meta = if kind == "cpu" { CPU_META } else { GPU_META };
    let subtests: Vec<SubtestOutcome> = meta
        .iter()
        .filter_map(|(id, unit, label)| {
            let v = *entry.values.get(*id)?;
            Some(SubtestOutcome {
                id: (*id).to_string(),
                label: (*label).to_string(),
                unit: (*unit).to_string(),
                direction: Direction::HigherIsBetter,
                value: v,
                stats: Stats::from_runs(vec![v]),
                confidence: Confidence::High,
            })
        })
        .collect();
    ensure!(
        !subtests.is_empty(),
        "model {:?} has no {kind} values",
        entry.model
    );

    let outcome = BenchmarkOutcome {
        id: kind.to_string(),
        label: if kind == "cpu" { "CPU" } else { "GPU" }.to_string(),
        subtests,
        notes: vec![format!(
            "synthetic — model reference for {} ({} sample(s), {})",
            entry.model, entry.samples, entry.measured
        )],
    };

    let baseline = super::Baseline::reference_v1();
    let outcomes = vec![outcome];
    let scored = super::score_run(
        &outcomes,
        &baseline,
        super::profile_by_name("general").expect("general profile"),
        0.5,
    )?;

    let mut machine = crate::inventory::Inventory::blank();
    machine.hostname = Some(entry.model.clone());
    machine.cpu_model = entry.model.clone();

    let config = super::RunConfig {
        profile: "general".into(),
        duration_preset: "reference".into(),
        curve_k: 0.5,
        seed: 0,
        threads: 0,
        baseline: baseline.name.clone(),
        only: vec![kind.to_string()],
        build_isa: entry.build.clone(),
    };
    Ok(super::ResultFile::assemble(
        machine, config, outcomes, scored, None,
    ))
}

/// `--add`: geomean each model's CPU / GPU subtests across the given result
/// files and print `cpu.toml` + `gpu.toml`. Does **not** merge with the
/// embedded tables (like `loadbearer baseline`) — review the diff and commit.
fn regenerate(paths: &[std::path::PathBuf]) -> Result<()> {
    use std::collections::BTreeMap;

    // model -> (subtest -> values), + topology for CPUs
    let mut cpu: BTreeMap<String, BTreeMap<String, Vec<f64>>> = BTreeMap::new();
    let mut gpu: BTreeMap<String, BTreeMap<String, Vec<f64>>> = BTreeMap::new();
    let mut topo: BTreeMap<String, String> = BTreeMap::new();

    for path in paths {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
        let r: super::ResultFile = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("parsing {}: {e}", path.display()))?;
        for comp in &r.raw {
            match comp.id.as_str() {
                "cpu" => {
                    let cm = canon(&r.machine.cpu_model);
                    let e = cpu.entry(cm.clone()).or_default();
                    for st in &comp.subtests {
                        e.entry(st.id.clone()).or_default().push(st.value);
                    }
                    if let Some(p) = r.machine.cpu_physical_cores {
                        topo.insert(cm, format!("{p}C / {}T", r.machine.cpu_logical_cores));
                    }
                }
                "gpu" => {
                    if let Some(g) = &r.machine.gpu {
                        let e = gpu.entry(canon(&g.name)).or_default();
                        for st in &comp.subtests {
                            e.entry(st.id.clone()).or_default().push(st.value);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    println!("{}", to_toml("cpu", &cpu, &topo, CPU_META));
    println!("# ----- baseline/models/gpu.toml -----");
    println!("{}", to_toml("gpu", &gpu, &BTreeMap::new(), GPU_META));
    Ok(())
}

/// Human-facing canonical model name (keeps case): drop `(R)`/`(TM)`, a clock,
/// a trailing GPU blurb, the `CPU`/`Processor`/`Gen` noise words and an
/// ordinal gen prefix.
fn canon(s: &str) -> String {
    let mut t = s.to_string();
    for p in ["(R)", "(TM)", "(C)", "\u{ae}", "\u{2122}"] {
        t = t.replace(p, "");
    }
    if let Some(i) = t.find(" @ ") {
        t.truncate(i);
    }
    for sep in [" w/ ", " with Radeon", " with Graphics"] {
        if let Some(i) = t.find(sep) {
            t.truncate(i);
        }
    }
    t.split_whitespace()
        .filter(|w| !matches!(*w, "CPU" | "Processor" | "Gen"))
        .filter(|w| !(w.ends_with("th") && w[..w.len() - 2].chars().all(|c| c.is_ascii_digit())))
        .collect::<Vec<_>>()
        .join(" ")
}

fn to_toml(
    kind: &str,
    models: &std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<f64>>>,
    topo: &std::collections::BTreeMap<String, String>,
    meta: &[(&str, &str, &str)],
) -> String {
    let mut s = format!(
        "# loadbearer {} model reference — regenerated {} by loadbearer {}.\n\
         # Data, not API (see VERSIONING.md). Review before committing.\n\n",
        kind.to_uppercase(),
        today(),
        env!("CARGO_PKG_VERSION"),
    );
    for (model, subs) in models {
        let n = subs.values().map(Vec::len).max().unwrap_or(0);
        let sku = model.split_whitespace().last().unwrap_or(model);
        s.push_str(&format!("[[{kind}]]\n"));
        s.push_str(&format!("model = {model:?}\n"));
        let alias = if sku.chars().any(|c| c.is_ascii_digit()) {
            format!("[{sku:?}]")
        } else {
            "[]".to_string()
        };
        s.push_str(&format!("aliases = {alias}\n"));
        if let Some(t) = topo.get(model) {
            s.push_str(&format!("topology = {t:?}\n"));
        }
        s.push_str(&format!("samples = {n}\n"));
        s.push_str(&format!("measured = {:?}\n", today()));
        s.push_str(&format!("tool_version = {:?}\n", env!("CARGO_PKG_VERSION")));
        s.push_str("build = \"portable\"\n");
        for (id, _, _) in meta {
            if let Some(vals) = subs.get(*id) {
                s.push_str(&format!("{id} = {}\n", sig4(crate::util::geomean(vals))));
            }
        }
        s.push('\n');
    }
    s
}

fn today() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
        .split('T')
        .next()
        .unwrap_or("")
        .to_string()
}

fn sig4(x: f64) -> String {
    if x == 0.0 {
        return "0".into();
    }
    let digits = (3 - x.abs().log10().floor() as i32).max(0);
    let f = 10f64.powi(digits);
    let r = (x * f).round() / f;
    if r.fract() == 0.0 {
        format!("{r:.0}")
    } else {
        format!("{r}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_tables_parse_and_have_entries() {
        let t = ModelTable::embedded();
        assert!(
            t.cpu_entries().len() >= 5,
            "cpu models: {}",
            t.cpu_entries().len()
        );
        assert!(!t.gpu_entries().is_empty());
        let e = t.cpu("Intel Core i7-1370P").expect("i7-1370P present");
        assert!(e.values.contains_key("int_multi"));
        assert_eq!(e.build, "portable");
    }

    #[test]
    fn normalize_strips_the_usual_noise() {
        assert_eq!(
            normalize("13th Gen Intel(R) Core(TM) i7-1370P"),
            "intel core i7-1370p"
        );
        assert_eq!(
            normalize("Intel(R) Core(TM) i5-8350U CPU @ 1.70GHz"),
            "intel core i5-8350u"
        );
        assert_eq!(
            normalize("AMD Ryzen 7 7840U w/ Radeon 780M Graphics"),
            "amd ryzen 7 7840u"
        );
    }

    #[test]
    fn lookup_is_exact_or_sku_token_only() {
        let t = ModelTable::embedded();
        // exact-ish (after normalise)
        assert!(t.cpu("Intel(R) Core(TM) i5-8350U CPU @ 1.70GHz").is_some());
        // SKU token embedded in a messier string
        assert!(
            t.cpu("12th Gen genuine intel core i5-8350u special")
                .is_some()
        );
        // no false positive on a different SKU
        assert!(t.cpu("Intel Core i5-9999Z").is_none());
        // generic string, no token → no match
        assert!(t.cpu("Some Unknown CPU").is_none());
    }

    #[test]
    fn verdict_bands_and_native_caveat() {
        assert!(verdict("cpu", "X", 2.0, "sse2").contains("matches a typical"));
        assert!(verdict("cpu", "X", -20.0, "sse2").contains("below par"));
        assert!(verdict("cpu", "X", 15.0, "sse2").contains("above a typical"));
        assert!(verdict("cpu", "X", -20.0, "avx2").contains("Indicative only"));
        assert!(verdict("cpu", "X", -20.0, "avx512").contains("Indicative only"));
        // a portable Arm (NEON) build compares straight against a NEON
        // reference — no x86 "expect higher" caveat
        assert!(!verdict("cpu", "X", 2.0, "neon").contains("Indicative only"));
        assert!(verdict("cpu", "X", 2.0, "neon").contains("matches a typical"));
    }

    /// Build a `cpu` outcome whose subtests equal `entry`'s reference values.
    fn outcome_matching(entry: &ModelEntry) -> BenchmarkOutcome {
        let subtests = CPU_META
            .iter()
            .filter_map(|(id, unit, label)| {
                let v = *entry.values.get(*id)?;
                Some(SubtestOutcome {
                    id: (*id).to_string(),
                    label: (*label).to_string(),
                    unit: (*unit).to_string(),
                    direction: Direction::HigherIsBetter,
                    value: v,
                    stats: Stats::from_runs(vec![v]),
                    confidence: Confidence::High,
                })
            })
            .collect();
        BenchmarkOutcome {
            id: "cpu".to_string(),
            label: "CPU".to_string(),
            subtests,
            notes: vec![],
        }
    }

    #[test]
    fn for_run_matches_its_own_reference() {
        let t = ModelTable::embedded();
        let entry = t.cpu("Intel Core i7-1370P").expect("i7-1370P present");
        let refs = for_run(
            "13th Gen Intel(R) Core(TM) i7-1370P",
            None,
            &[outcome_matching(entry)],
            "sse2",
            "thorough",
        );
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].component, "cpu");
        assert!(
            refs[0].delta_pct.abs() < 1.0,
            "delta {} should be ~0",
            refs[0].delta_pct
        );
        assert!(refs[0].verdict.contains("matches a typical"));
        // a thorough run matches the reference preset — no caveat recorded
        assert_eq!(refs[0].run_preset, "");
    }

    #[test]
    fn a_shorter_preset_is_recorded_for_the_caveat() {
        let t = ModelTable::embedded();
        let entry = t.cpu("Intel Core i7-1370P").expect("i7-1370P present");
        let refs = for_run(
            "13th Gen Intel(R) Core(TM) i7-1370P",
            None,
            &[outcome_matching(entry)],
            "sse2",
            "short",
        );
        assert_eq!(refs[0].run_preset, "short");
    }

    #[test]
    fn synthetic_result_scores_and_carries_the_model() {
        let t = ModelTable::embedded();
        let entry = t.cpu("Intel Core i7-1370P").expect("i7-1370P present");
        let r = synthetic_result("cpu", entry).expect("synthetic result builds");
        assert_eq!(r.machine.cpu_model, "Intel Core i7-1370P");
        assert_eq!(r.config.duration_preset, "reference");
        assert!(r.raw.iter().any(|o| o.id == "cpu"));
        assert!(r.components.iter().any(|c| c.id == "cpu"));
    }
}
