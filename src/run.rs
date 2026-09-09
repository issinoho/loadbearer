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

impl From<crate::cli::GradeArg> for crate::scoring::Grade {
    fn from(value: crate::cli::GradeArg) -> Self {
        use crate::cli::GradeArg as G;
        use crate::scoring::Grade;
        match value {
            G::S => Grade::S,
            G::A => Grade::A,
            G::B => Grade::B,
            G::C => Grade::C,
            G::D => Grade::D,
            G::F => Grade::F,
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
    tags: crate::tags::Tags,
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
        tags: crate::tags::resolve(file.tags.as_ref(), &args.tags)?,
    })
}

pub fn execute(args: RunArgs) -> Result<u8> {
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

    // Before the baseline, the inventory, the scratch directory — anything a
    // skipped run shouldn't pay for. A gate declining to run is not a failure,
    // so this leaves with 0 and writes no result file.
    let gate_report = match gates(&args)?.evaluate() {
        crate::gates::Decision::Proceed(report) => *report,
        crate::gates::Decision::Skip(why) => {
            info!(target: "loadbearer::run", "skipped: {why}");
            eprintln!("loadbearer run skipped — {why}");
            return Ok(crate::exit::OK);
        }
    };

    let baseline = Baseline::reference_v1();
    let machine = crate::inventory::collect();

    let target_dir = match r.target_dir {
        Some(dir) => dir,
        None => std::env::current_dir()?,
    };
    std::fs::create_dir_all(&target_dir)
        .with_context(|| format!("creating disk scratch directory {}", target_dir.display()))?;
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
            &args,
            selected,
            ctx,
            baseline,
            r.profile,
            r.curve_k,
            machine,
            config,
            r.tags,
            gate_report,
        )
    } else {
        run_plain(
            &args,
            &selected,
            &ctx,
            &baseline,
            r.profile,
            r.curve_k,
            machine,
            config,
            r.tags,
            gate_report,
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
    tags: crate::tags::Tags,
    gate_report: crate::gates::GateReport,
) -> Result<u8> {
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
        no_telemetry: args.no_telemetry,
    };

    let code = match tui::run(init)? {
        Some(mut result) => {
            let (link, link_notes) = probe_link(args);
            result.link = link;
            result.notes.extend(link_notes);
            result.tags = tags;
            result.gates = (!gate_report.is_empty()).then_some(gate_report);
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
            threshold_code(args, &result)
        }
        None => {
            println!("run cancelled.");
            crate::exit::OK
        }
    };
    Ok(code)
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
    tags: crate::tags::Tags,
    gate_report: crate::gates::GateReport,
) -> Result<u8> {
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
    let sampler = crate::telemetry::Sampler::start(!args.no_telemetry);
    let (mut outcomes, mut notes) = run_benchmarks(selected, ctx, &mut progress)?;
    let telemetry = sampler.finish();

    // Before scoring, so the raw and scored views agree on confidence.
    if let Some(note) =
        crate::telemetry::downgrade_thermally_limited(&mut outcomes, telemetry.as_ref())
    {
        warn!(target: "loadbearer::run", "{note}");
        notes.push(note);
    }

    let scored = score_run(&outcomes, baseline, profile, curve_k)?;
    let (link, link_notes) = probe_link(args);
    notes.extend(link_notes);
    let model_ref = model_ref(
        args,
        &machine,
        &outcomes,
        &config.build_isa,
        &config.duration_preset,
    );
    let mut result = ResultFile::assemble(machine, config, outcomes, scored, link);
    result.model_ref = model_ref;
    result.telemetry = telemetry;
    result.tags = tags;
    result.notes = notes;
    result.gates = (!gate_report.is_empty()).then_some(gate_report);

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
    Ok(threshold_code(args, &result))
}

/// Build the unattended-run gates from the command line. The hostname comes
/// straight from the OS rather than the inventory, because the gates run before
/// anything as expensive as a full inventory collection.
fn gates(args: &RunArgs) -> Result<crate::gates::Gates<'_>> {
    let skip_if_newer_than = match &args.skip_if_newer_than {
        Some(s) => Some(
            crate::gates::parse_age(s).with_context(|| format!("--skip-if-newer-than {s:?}"))?,
        ),
        None => None,
    };
    Ok(crate::gates::Gates {
        not_on_battery: args.not_on_battery,
        if_idle: args.if_idle,
        jitter: args.jitter.map(std::time::Duration::from_secs),
        skip_if_newer_than,
        output: args.output.as_deref(),
        hostname: sysinfo::System::host_name(),
    })
}

/// The machine's CPU / GPU measured against their model references, unless
/// `--no-model-ref` was given or no ISA was recorded as non-portable.
fn model_ref(
    args: &RunArgs,
    machine: &Inventory,
    outcomes: &[BenchmarkOutcome],
    build_isa: &str,
    run_preset: &str,
) -> Vec<crate::scoring::models::ModelRef> {
    if args.no_model_ref {
        return Vec::new();
    }
    crate::scoring::models::for_run(
        &machine.cpu_model,
        machine.gpu.as_ref().map(|g| g.name.as_str()),
        outcomes,
        build_isa,
        run_preset,
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
///
/// A probe that can't reach its target is reported and noted, not fatal: the
/// link result is never graded, and throwing away a completed CPU/memory/disk
/// assessment because a firewall or a stopped `net-server` got in the way is a
/// poor trade. Returns the result and a note for the result file.
fn probe_link(args: &RunArgs) -> (Option<crate::scoring::LinkResult>, Vec<String>) {
    let Some(target) = &args.net_target else {
        return (None, Vec::new());
    };
    info!(target: "loadbearer::run", "--net-target: probing link to {target}");
    eprintln!("probing link to {target} …");
    match benches::link_probe(target, std::time::Duration::from_secs(1)) {
        Ok(link) => {
            info!(
                target: "loadbearer::run",
                "link probe ok: {:.2} GiB/s up, {:.0} us rtt, {:.0} Kpps",
                link.tcp_upload_gibps, link.tcp_rtt_us, link.udp_send_kpps,
            );
            (Some(link), Vec::new())
        }
        Err(e) => {
            warn!(target: "loadbearer::run", "link probe to {target} failed: {e:#}");
            let note = format!(
                "link probe to {target} was skipped: {e:#} \
                 (is `loadbearer net-server` running there?)"
            );
            eprintln!("warning: {note}");
            (None, vec![note])
        }
    }
}

/// Run the selected benchmarks, tolerating a failure in a component that
/// doesn't feed the grade.
///
/// The case that matters is a locked-down corporate desktop, where endpoint
/// protection refuses the loopback socket the `network` component needs.
/// Losing the CPU, memory and disk grades over an ungraded extra is the wrong
/// trade. A *graded* component failing still stops the run: a grade with a
/// component missing isn't the thing that was asked for.
pub(crate) fn run_benchmarks(
    selected: &[Box<dyn Benchmark>],
    ctx: &RunContext,
    progress: &mut dyn crate::engine::progress::Progress,
) -> Result<(Vec<BenchmarkOutcome>, Vec<String>)> {
    let mut outcomes = Vec::with_capacity(selected.len());
    let mut notes = Vec::new();
    for bench in selected {
        match run_benchmark(bench.as_ref(), ctx, progress) {
            Ok(outcome) => outcomes.push(outcome),
            Err(e) if crate::scoring::UNGRADED_COMPONENTS.contains(&bench.id()) => {
                warn!(
                    target: "loadbearer::run",
                    "ungraded component {} failed, carrying on without it: {e:#}", bench.id(),
                );
                notes.push(format!("{} was skipped: {e:#}", bench.id()));
            }
            Err(e) => return Err(e),
        }
    }
    // Every component skipped means nothing was measured, which is a failure
    // however tolerant we are about the individual pieces.
    if outcomes.is_empty() {
        bail!(
            "no benchmark produced a result{}",
            if notes.is_empty() {
                String::new()
            } else {
                format!(" — {}", notes.join("; "))
            }
        );
    }
    Ok((outcomes, notes))
}

/// The exit code `--fail-under` implies, and why, so the caller can say so.
///
/// `graded` is the overall grade, or `None` when the run graded nothing at
/// all: `run --only network` reports 0/F by construction, and failing on that
/// would announce a policy breach that didn't happen.
fn threshold_verdict(
    floor: Option<crate::scoring::Grade>,
    graded: Option<crate::scoring::Grade>,
) -> (u8, Option<String>) {
    let Some(floor) = floor else {
        return (crate::exit::OK, None);
    };
    let Some(grade) = graded else {
        return (
            crate::exit::OK,
            Some(format!(
                "--fail-under {} ignored — no graded component in this run",
                floor.as_str()
            )),
        );
    };
    if grade.is_at_least(floor) {
        (crate::exit::OK, None)
    } else {
        (
            crate::exit::BELOW_THRESHOLD,
            Some(format!(
                "grade {} is below the --fail-under threshold of {}",
                grade.as_str(),
                floor.as_str()
            )),
        )
    }
}

/// Apply `--fail-under` to a finished run.
fn threshold_code(args: &RunArgs, result: &ResultFile) -> u8 {
    let graded = result
        .components
        .iter()
        .any(|c| c.graded)
        .then_some(result.overall.grade);
    let (code, message) = threshold_verdict(args.fail_under.map(Into::into), graded);
    if let Some(message) = message {
        warn!(target: "loadbearer::run", "{message}");
        eprintln!("\n{message}");
    }
    code
}

fn write_output(result: &ResultFile, path: Option<&Path>) -> Result<()> {
    if let Some(path) = path {
        crate::util::ensure_parent_dir(path)
            .with_context(|| format!("creating directory for {}", path.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring::Grade;

    #[test]
    fn no_threshold_never_fails_a_run() {
        assert_eq!(
            threshold_verdict(None, Some(Grade::F)),
            (crate::exit::OK, None)
        );
    }

    #[test]
    fn a_grade_at_or_above_the_floor_passes() {
        for grade in [Grade::S, Grade::A, Grade::B, Grade::C] {
            let (code, msg) = threshold_verdict(Some(Grade::C), Some(grade));
            assert_eq!(
                code,
                crate::exit::OK,
                "{} should pass a C floor",
                grade.as_str()
            );
            assert!(msg.is_none());
        }
    }

    #[test]
    fn a_grade_below_the_floor_exits_three_and_says_so() {
        for grade in [Grade::D, Grade::F] {
            let (code, msg) = threshold_verdict(Some(Grade::C), Some(grade));
            assert_eq!(code, crate::exit::BELOW_THRESHOLD);
            assert!(msg.unwrap().contains("below the --fail-under threshold"));
        }
    }

    /// `--only network` grades nothing, so its 0/F is an artefact rather than
    /// a verdict on the machine.
    #[test]
    fn an_ungraded_run_is_not_judged_but_is_explained() {
        let (code, msg) = threshold_verdict(Some(Grade::B), None);
        assert_eq!(code, crate::exit::OK);
        assert!(msg.unwrap().contains("no graded component"));
    }

    #[test]
    fn the_threshold_grade_itself_is_a_pass_not_a_failure() {
        assert_eq!(
            threshold_verdict(Some(Grade::B), Some(Grade::B)).0,
            crate::exit::OK
        );
    }
}
