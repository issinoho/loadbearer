//! `loadbearer run` — resolve settings (CLI switches over config file over
//! defaults), execute the selected benchmarks, score them, and emit a versioned
//! result file. Interactive terminals get the TUI; otherwise plain text or JSON.

use std::io::IsTerminal;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow, bail};
use clap::ValueEnum;
use log::{debug, info, warn};

use crate::benches;
use crate::cli::{DurationArg, RunArgs};
use crate::config::FileConfig;
use crate::engine::progress::PlainProgress;
use crate::engine::{Benchmark, BenchmarkOutcome, DurationPreset, RunContext, run_benchmark};
use crate::inventory::Inventory;
use crate::output;
use crate::scoring::{
    Baseline, Profile, ResultFile, RunConfig, profile_by_name, profile_names, score_run,
};
use crate::tui;

impl From<DurationArg> for DurationPreset {
    fn from(value: DurationArg) -> Self {
        match value {
            DurationArg::Short => DurationPreset::Short,
            DurationArg::Normal => DurationPreset::Normal,
            DurationArg::Thorough => DurationPreset::Thorough,
        }
    }
}

const DEFAULT_SEED: u64 = 0x5EED_1234_ABCD_0001;

/// Settings after merging CLI switches, the config file and defaults.
struct Resolved {
    profile: Profile,
    duration: DurationArg,
    curve_k: f64,
    seed: Option<u64>,
    runs: Option<u32>,
    target_dir: Option<std::path::PathBuf>,
    only: Vec<String>,
}

fn resolve(args: &RunArgs) -> Result<Resolved> {
    let file = match &args.config {
        Some(path) => FileConfig::load(path)?,
        None => FileConfig::default(),
    };

    let profile_name = args
        .profile
        .clone()
        .or(file.profile)
        .unwrap_or_else(|| "general".to_string());
    let profile = profile_by_name(&profile_name).with_context(|| {
        format!(
            "unknown profile {profile_name:?}; known profiles: {}",
            profile_names().join(", ")
        )
    })?;

    let duration = match args.duration {
        Some(d) => d,
        None => match file.duration.as_deref() {
            Some(s) => DurationArg::from_str(s, true)
                .map_err(|e| anyhow!("config: invalid duration {s:?} ({e})"))?,
            None => DurationArg::Normal,
        },
    };

    let curve_k = args.curve_k.or(file.curve_k).unwrap_or(0.5);
    if !(0.05..=3.0).contains(&curve_k) {
        bail!("curve-k must be between 0.05 and 3.0 (got {curve_k})");
    }

    let only = if args.only.is_empty() {
        file.only.unwrap_or_default()
    } else {
        args.only.clone()
    };

    Ok(Resolved {
        profile,
        duration,
        curve_k,
        seed: args.seed.or(file.seed),
        runs: args.runs.or(file.runs),
        target_dir: args.target_dir.clone().or(file.target_dir),
        only,
    })
}

pub fn execute(args: RunArgs) -> Result<()> {
    let r = resolve(&args)?;
    info!(
        target: "loadbearer::run",
        "resolved settings: profile={}, preset={:?}, curve_k={}, seed={:?}, runs={:?}, only={:?}",
        r.profile.name, r.duration, r.curve_k, r.seed, r.runs, r.only,
    );

    let selected = select_benchmarks(&r.only)?;
    info!(
        target: "loadbearer::run",
        "selected benchmarks: [{}]",
        selected.iter().map(|b| b.id()).collect::<Vec<_>>().join(", "),
    );
    let baseline = Baseline::reference_v1();
    let machine = crate::inventory::collect();

    let target_dir = match r.target_dir {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    debug!(target: "loadbearer::run", "disk scratch target dir: {}", target_dir.display());
    let ctx = RunContext {
        preset: r.duration.into(),
        seed: r.seed.unwrap_or(DEFAULT_SEED),
        target_dir,
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        total_ram: machine.ram_bytes,
        runs_override: r.runs,
        abort: Arc::new(AtomicBool::new(false)),
    };

    let config = RunConfig {
        profile: r.profile.name.to_string(),
        duration_preset: ctx.preset.name().to_string(),
        curve_k: r.curve_k,
        seed: ctx.seed,
        threads: ctx.threads,
        baseline: baseline.name.clone(),
        only: r.only.iter().map(|s| s.trim().to_lowercase()).collect(),
        build_isa: env!("LOADBEARER_BUILD_ISA").to_string(),
    };

    let interactive = std::io::stdout().is_terminal() && !args.plain && !args.json;
    info!(
        target: "loadbearer::run",
        "output path: {}", if interactive { "interactive TUI" } else { "plain/json" },
    );
    if interactive {
        run_interactive(
            &args, selected, ctx, baseline, r.profile, r.curve_k, machine, config,
        )
    } else {
        run_plain(
            &args, &selected, &ctx, &baseline, r.profile, r.curve_k, machine, config,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn run_interactive(
    args: &RunArgs,
    selected: Vec<Box<dyn Benchmark>>,
    ctx: RunContext,
    baseline: Baseline,
    profile: Profile,
    curve_k: f64,
    machine: Inventory,
    config: RunConfig,
) -> Result<()> {
    let header = format!(
        "{} · {} · {} threads · {} preset · {} profile",
        machine.hostname.as_deref().unwrap_or("unknown"),
        machine.cpu_model,
        ctx.threads,
        ctx.preset.name(),
        profile.name,
    );
    let init = tui::RunInit {
        header,
        selected,
        curve_k,
        ctx,
        baseline,
        profile,
        machine,
        config,
        soak: soak_config(args),
        no_model_ref: args.no_model_ref,
    };

    match tui::run(init)? {
        Some(mut result) => {
            result.link = probe_link(args)?;
            write_output(&result, args.output.as_deref())?;
            let written = match &args.output {
                Some(p) => format!(" · written to {}", p.display()),
                None => String::new(),
            };
            if result.components.iter().any(|c| c.graded) {
                println!(
                    "Overall {:.0} [{}] · {} profile{written}",
                    result.overall.score,
                    result.overall.grade.as_str(),
                    result.overall.profile,
                );
            } else {
                println!(
                    "No graded components in this run · {} profile{written}",
                    result.overall.profile
                );
            }
        }
        None => println!("run cancelled."),
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_plain(
    args: &RunArgs,
    selected: &[Box<dyn Benchmark>],
    ctx: &RunContext,
    baseline: &Baseline,
    profile: Profile,
    curve_k: f64,
    machine: Inventory,
    config: RunConfig,
) -> Result<()> {
    if !args.json {
        eprintln!(
            "loadbearer run — {} preset, {} timed run(s), {} thread(s), profile {}, seed {:#018x}",
            ctx.preset.name(),
            ctx.runs_override.unwrap_or_else(|| ctx.preset.timed_runs()),
            ctx.threads,
            profile.name,
            ctx.seed,
        );
    }

    let mut progress = PlainProgress::new();
    let mut outcomes: Vec<BenchmarkOutcome> = Vec::new();
    for bench in selected {
        outcomes.push(run_benchmark(bench.as_ref(), ctx, &mut progress)?);
    }

    let scored = score_run(&outcomes, baseline, profile, curve_k)?;
    let link = probe_link(args)?;
    let model_ref = model_ref(args, &machine, &outcomes, &config.build_isa);
    let mut result = ResultFile::assemble(machine, config, outcomes, scored, link);
    result.model_ref = model_ref;

    if args.json {
        result.soak = run_soak(args);
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        output::print_scored_report(&result);
        result.soak = run_soak(args);
        if let Some(soak) = &result.soak {
            output::print_soak_block(soak);
        }
    }
    if let Some(path) = &args.output {
        write_output(&result, Some(path))?;
        if !args.json {
            eprintln!("\nresult written to {}", path.display());
        }
    }
    Ok(())
}

/// The machine's CPU / GPU measured against their model references, unless
/// `--no-model-ref` was given or no ISA was recorded as non-portable.
fn model_ref(
    args: &RunArgs,
    machine: &Inventory,
    outcomes: &[BenchmarkOutcome],
    build_isa: &str,
) -> Vec<crate::scoring::models::ModelRef> {
    if args.no_model_ref {
        return Vec::new();
    }
    crate::scoring::models::for_run(
        &machine.cpu_model,
        machine.gpu.as_ref().map(|g| g.name.as_str()),
        outcomes,
        build_isa,
    )
}

/// The soak configuration implied by `--soak` / `--soak-duration`, or `None`.
fn soak_config(args: &RunArgs) -> Option<crate::soak::SoakConfig> {
    args.soak.then(|| crate::soak::SoakConfig {
        duration: crate::soak::resolve_duration(args.soak_duration),
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        seed: DEFAULT_SEED,
    })
}

/// Run the optional `--soak` phase for the non-interactive paths, with a live
/// stderr progress line (suppressed under `--json`). The interactive path runs
/// the soak inside the TUI instead.
fn run_soak(args: &RunArgs) -> Option<crate::soak::SoakResult> {
    let cfg = soak_config(args)?;
    info!(target: "loadbearer::run", "--soak: starting sustained-load phase");
    if !args.json {
        eprintln!();
    }
    Some(crate::soak::run_with_progress(&cfg, args.json))
}

/// Run the optional `--net-target` link probe, if one was requested.
fn probe_link(args: &RunArgs) -> Result<Option<crate::scoring::LinkResult>> {
    let Some(target) = &args.net_target else {
        return Ok(None);
    };
    info!(target: "loadbearer::run", "--net-target: probing link to {target}");
    eprintln!("probing link to {target} …");
    let link = benches::link_probe(target, std::time::Duration::from_secs(1))
        .inspect_err(|e| warn!(target: "loadbearer::run", "link probe to {target} failed: {e}"))
        .with_context(|| {
            format!("probing link to {target} (is `loadbearer net-server` running there?)")
        })?;
    info!(
        target: "loadbearer::run",
        "link probe ok: {:.2} GiB/s up, {:.0} us rtt, {:.0} Kpps",
        link.tcp_upload_gibps, link.tcp_rtt_us, link.udp_send_kpps,
    );
    Ok(Some(link))
}

fn write_output(result: &ResultFile, path: Option<&Path>) -> Result<()> {
    if let Some(path) = path {
        let json = serde_json::to_string_pretty(result)?;
        std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
        info!(target: "loadbearer::run", "result written to {}", path.display());
    }
    Ok(())
}

fn select_benchmarks(only: &[String]) -> Result<Vec<Box<dyn Benchmark>>> {
    let all = benches::all();
    let gpu_ok = benches::gpu_probe().is_some();

    // Default set: everything, minus `gpu` when there's no GPU to test.
    if only.is_empty() {
        if !gpu_ok {
            debug!(target: "loadbearer::run", "no GPU/OpenCL — gpu component excluded from the default set");
        }
        return Ok(all
            .into_iter()
            .filter(|b| b.id() != "gpu" || gpu_ok)
            .collect());
    }

    let wanted: Vec<String> = only.iter().map(|s| s.trim().to_lowercase()).collect();
    let known = benches::known_ids();
    for id in &wanted {
        if !known.contains(&id.as_str()) {
            bail!("unknown benchmark {id:?} (known: {})", known.join(", "));
        }
    }
    if wanted.iter().any(|w| w == "gpu") && !gpu_ok {
        bail!(
            "the GPU component is unavailable — either `--no-gpu` was given, or no \
             OpenCL loader / GPU device was found. Drop `gpu` from --only."
        );
    }
    Ok(all
        .into_iter()
        .filter(|b| wanted.iter().any(|w| w == b.id()))
        .collect())
}
